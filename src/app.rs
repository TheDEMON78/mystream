//! Interface (egui) : scènes, sources, aperçu, mixeur, commandes et paramètres.

use crate::audio::{Gains, Mixer};
use crate::capture::{self, Runtime};
use crate::compositor::{Compositor, DrawItem};
use crate::engine::{Engine, Shared, key};
use crate::model::{Project, Settings, SourceKind};
use crate::output::{self, Kind, Output};
use egui::{Color32, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Vec2, pos2, vec2};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

const RESOLUTIONS: [(u32, u32); 5] = [
    (1920, 1080),
    (2560, 1440),
    (1600, 900),
    (1280, 720),
    (3840, 2160),
];
const ACCENT: Color32 = Color32::from_rgb(0x8b, 0x5c, 0xf6);
const LIVE: Color32 = Color32::from_rgb(0xe5, 0x48, 0x4d);

pub struct App {
    project: Project,
    runtimes: HashMap<u64, Runtime>,
    shared: Arc<Shared>,
    engine: Option<Engine>,
    render_state: egui_wgpu::RenderState,
    preview: Option<egui::TextureId>,
    canvas: (u32, u32),
    mixer: Mixer,
    selected: Option<u64>,
    settings_open: bool,
    draft: Settings,
    notice: Option<(String, bool, Instant)>,
    dirty: Option<Instant>,
    renaming: Option<(u64, String)>,
    /// Test automatique (MYSTREAM_AUTOTEST=<secondes>) : enregistre puis quitte.
    autotest: Option<(Instant, u64)>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let render_state = cc
            .wgpu_render_state
            .clone()
            .expect("MyStream nécessite le rendu wgpu");
        let project = Project::load();
        let a = &project.settings.audio;
        let mixer = Mixer::new(Gains {
            mic: a.mic_volume,
            mic_muted: a.mic_muted,
            desktop: a.desktop_volume,
            desktop_muted: a.desktop_muted,
        });
        // Détection des encodeurs (quelques lancements de FFmpeg) en arrière-plan.
        std::thread::spawn(|| {
            let _ = output::encoders();
        });
        let mut app = Self {
            draft: project.settings.clone(),
            project,
            runtimes: HashMap::new(),
            shared: Arc::new(Shared::default()),
            engine: None,
            render_state,
            preview: None,
            canvas: (0, 0),
            mixer,
            selected: None,
            settings_open: false,
            notice: None,
            dirty: None,
            renaming: None,
            autotest: std::env::var("MYSTREAM_AUTOTEST")
                .ok()
                .and_then(|v| v.parse().ok())
                .map(|s| (Instant::now(), s)),
        };
        app.start_engine(&cc.egui_ctx);
        app
    }

    /// (Re)crée le compositeur à la taille et la cadence des paramètres.
    fn start_engine(&mut self, ctx: &egui::Context) {
        self.engine = None; // arrête l'ancien fil de rendu
        let s = &self.project.settings;
        let rs = &self.render_state;
        let comp = Compositor::new(rs.device.clone(), rs.queue.clone(), s.width, s.height);
        {
            let mut r = rs.renderer.write();
            match self.preview {
                Some(id) => r.update_egui_texture_from_wgpu_texture(
                    &rs.device,
                    comp.view(),
                    wgpu::FilterMode::Linear,
                    id,
                ),
                None => {
                    self.preview = Some(r.register_native_texture(
                        &rs.device,
                        comp.view(),
                        wgpu::FilterMode::Linear,
                    ))
                }
            }
        }
        self.canvas = (comp.width, comp.height);
        self.engine = Some(Engine::start(comp, s.fps, self.shared.clone(), ctx.clone()));
    }

    fn touch(&mut self) {
        self.dirty.get_or_insert_with(Instant::now);
    }

    fn notify(&mut self, msg: impl Into<String>, error: bool) {
        let msg = msg.into();
        if error {
            log::warn!("{msg}");
        }
        self.notice = Some((msg, error, Instant::now()));
    }

    fn live(&self, kind: Kind) -> bool {
        self.shared.outputs.lock().contains_key(&key(kind))
    }

    /// Démarre ou arrête les captures selon les sources du projet.
    fn sync_runtimes(&mut self) {
        let fps = self.project.settings.fps;
        self.runtimes.retain(|id, rt| {
            self.project
                .sources
                .iter()
                .any(|s| s.id == *id && s.kind == *rt.kind())
        });
        for s in &self.project.sources {
            self.runtimes
                .entry(s.id)
                .or_insert_with(|| Runtime::start(&s.kind, fps));
        }
        let mut slots = self.shared.slots.lock();
        slots.retain(|id, _| self.runtimes.contains_key(id));
        for (id, rt) in &self.runtimes {
            slots.entry(*id).or_insert_with(|| rt.slot.clone());
        }
    }

    /// Liste de dessin de la scène à l'antenne ; place les nouveaux éléments (réduits s'ils dépassent).
    fn update_items(&mut self) {
        let (cw, ch) = (self.canvas.0 as f32, self.canvas.1 as f32);
        let sizes = self.shared.sizes.lock().clone();
        let current = self.project.current;
        let mut changed = false;
        let mut list = Vec::new();
        if let Some(scene) = self.project.scene_mut(current) {
            for item in scene.items.iter_mut() {
                let Some(&(w, h)) = sizes.get(&item.source) else {
                    continue;
                };
                let scale = *item.scale.get_or_insert_with(|| {
                    changed = true;
                    (cw / w as f32).min(ch / h as f32).min(1.0)
                });
                if item.visible {
                    list.push(DrawItem {
                        source: item.source,
                        x: item.x,
                        y: item.y,
                        w: w as f32 * scale,
                        h: h as f32 * scale,
                    });
                }
            }
        }
        *self.shared.items.lock() = list;
        if changed {
            self.touch();
        }
    }

    fn check_outputs(&mut self) {
        let ended: Vec<(u8, Option<String>, Kind)> = self
            .shared
            .outputs
            .lock()
            .iter()
            .filter(|(_, o)| o.ended())
            .map(|(k, o)| (*k, o.stats.lock().error.clone(), o.kind))
            .collect();
        for (k, err, kind) in ended {
            if let Some(out) = self.shared.outputs.lock().remove(&k) {
                out.stop();
            }
            self.notify(
                format!(
                    "{} s'est arrêté : {}",
                    kind.label(),
                    err.unwrap_or_else(|| "fin inattendue".into())
                ),
                true,
            );
        }
    }

    fn toggle_output(&mut self, kind: Kind) {
        let running = self.shared.outputs.lock().remove(&key(kind));
        if let Some(out) = running {
            let file = out.file.clone();
            out.stop();
            if let Some(f) = file {
                self.notify(
                    format!("Enregistrement sauvegardé : {}", f.display()),
                    false,
                );
            }
            return;
        }
        let (tx, rx) = crossbeam_channel::bounded(100);
        self.mixer.subscribe(tx);
        match Output::start(
            kind,
            &self.project.settings,
            self.canvas.0,
            self.canvas.1,
            rx,
        ) {
            Ok(out) => {
                self.shared.outputs.lock().insert(key(kind), out);
            }
            Err(e) => self.notify(
                format!("{} n'a pas pu démarrer : {e:#}", kind.label()),
                true,
            ),
        }
    }

    /* Panneaux ------------------------------------------------------------- */

    fn scenes_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Scènes");
        ui.separator();
        let mut switch = None;
        let ids: Vec<(u64, String)> = self
            .project
            .scenes
            .iter()
            .map(|s| (s.id, s.name.clone()))
            .collect();
        for (id, name) in ids {
            if let Some((rid, text)) = &mut self.renaming
                && *rid == id
            {
                let r = ui.text_edit_singleline(text);
                if r.lost_focus() {
                    let new = text.trim().to_string();
                    if !new.is_empty()
                        && let Some(s) = self.project.scene_mut(id)
                    {
                        s.name = new;
                    }
                    self.renaming = None;
                    self.touch();
                } else {
                    r.request_focus();
                }
                continue;
            }
            let r = ui.selectable_label(self.project.current == id, &name);
            if r.clicked() {
                switch = Some(id);
            }
            if r.double_clicked() {
                self.renaming = Some((id, name.clone()));
            }
        }
        if let Some(id) = switch {
            self.project.current = id;
            self.selected = None;
            self.touch();
        }
        ui.separator();
        ui.horizontal(|ui| {
            if ui.button("➕").on_hover_text("Ajouter une scène").clicked() {
                self.project.current = self.project.add_scene();
                self.touch();
            }
            if ui
                .add_enabled(self.project.scenes.len() > 1, egui::Button::new("➖"))
                .on_hover_text("Supprimer la scène")
                .clicked()
            {
                let id = self.project.current;
                self.project.remove_scene(id);
                self.touch();
            }
            if ui
                .button("✏")
                .on_hover_text("Renommer (ou double-clic)")
                .clicked()
            {
                let id = self.project.current;
                let name = self
                    .project
                    .scene(id)
                    .map(|s| s.name.clone())
                    .unwrap_or_default();
                self.renaming = Some((id, name));
            }
        });
    }

    fn sources_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Sources");
        ui.separator();
        let current = self.project.current;
        let items: Vec<(u64, u64, bool)> = self
            .project
            .scene(current)
            .map(|s| {
                s.items
                    .iter()
                    .map(|i| (i.id, i.source, i.visible))
                    .collect()
            })
            .unwrap_or_default();
        if items.is_empty() {
            ui.label(RichText::new("Cliquez sur « Ajouter » pour ajouter une source.").weak());
        }
        let mut action: Option<(&str, u64)> = None;
        for (item, source, visible) in &items {
            let Some(src) = self.project.source(*source) else {
                continue;
            };
            let status = self.runtimes.get(source).and_then(|r| r.error());
            ui.horizontal(|ui| {
                let eye = if *visible { "👁" } else { "⊘" };
                if ui
                    .small_button(eye)
                    .on_hover_text(if *visible { "Masquer" } else { "Afficher" })
                    .clicked()
                {
                    action = Some(("visible", *item));
                }
                let text = format!("{} {}", src.kind.glyph(), src.name);
                let mut label = RichText::new(text);
                if status.is_some() {
                    label = label.color(Color32::from_rgb(0xf5, 0x9e, 0x0b));
                }
                let r = ui.selectable_label(self.selected == Some(*item), label);
                let r = match &status {
                    Some(e) => r.on_hover_text(e),
                    None => r,
                };
                if r.clicked() {
                    self.selected = Some(*item);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .small_button("🗑")
                        .on_hover_text("Retirer de la scène")
                        .clicked()
                    {
                        action = Some(("remove", *item));
                    }
                    if ui.small_button("⏷").on_hover_text("Descendre").clicked() {
                        action = Some(("down", *item));
                    }
                    if ui.small_button("⏶").on_hover_text("Monter").clicked() {
                        action = Some(("up", *item));
                    }
                });
            });
        }
        if let Some((what, id)) = action {
            if what == "remove" {
                self.project.remove_item(current, id);
                if self.selected == Some(id) {
                    self.selected = None;
                }
            } else if let Some(scene) = self.project.scene_mut(current)
                && let Some(i) = scene.items.iter().position(|it| it.id == id)
            {
                match what {
                    "visible" => scene.items[i].visible = !scene.items[i].visible,
                    "up" if i > 0 => scene.items.swap(i, i - 1),
                    "down" if i + 1 < scene.items.len() => scene.items.swap(i, i + 1),
                    _ => {}
                }
            }
            self.touch();
        }
        ui.separator();
        ui.menu_button("➕ Ajouter", |ui| self.add_menu(ui));
        ui.separator();
        self.properties(ui);
    }

    fn add_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("🖥 Capture d'écran", |ui| {
            for m in capture::monitors() {
                if ui.button(&m.name).clicked() {
                    self.project
                        .add_source("Capture d'écran", SourceKind::Display { monitor: m.index });
                    self.touch();
                    ui.close();
                }
            }
        });
        if cfg!(windows) {
            ui.menu_button("🗔 Capture de fenêtre (jeu…)", |ui| {
                egui::ScrollArea::vertical()
                    .max_height(400.0)
                    .show(ui, |ui| {
                        for title in capture::windows() {
                            if ui.button(&title).clicked() {
                                self.project.add_source(
                                    &title.chars().take(40).collect::<String>(),
                                    SourceKind::Window {
                                        title: title.clone(),
                                    },
                                );
                                self.touch();
                                ui.close();
                            }
                        }
                    });
            });
        }
        if ui.button("🖼 Image…").clicked() {
            ui.close();
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("Images", &["png", "jpg", "jpeg", "gif", "bmp", "webp"])
                .pick_file()
            {
                let name = path
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "Image".into());
                self.project.add_source(
                    &name,
                    SourceKind::Image {
                        path: path.to_string_lossy().into_owned(),
                    },
                );
                self.touch();
            }
        }
        if ui.button("■ Couleur").clicked() {
            ui.close();
            let (w, h) = self.canvas;
            self.project.add_source(
                "Couleur",
                SourceKind::Color {
                    rgba: [0x2a, 0x1f, 0x4d, 255],
                    width: w,
                    height: h,
                },
            );
            self.touch();
        }
    }

    /// Réglages de la source sélectionnée.
    fn properties(&mut self, ui: &mut egui::Ui) {
        let current = self.project.current;
        let Some(item_id) = self.selected else { return };
        let Some(source_id) = self
            .project
            .scene(current)
            .and_then(|s| s.items.iter().find(|i| i.id == item_id))
            .map(|i| i.source)
        else {
            return;
        };
        let fps = self
            .runtimes
            .get(&source_id)
            .map(|r| *r.status.fps.lock())
            .unwrap_or(0.0);
        let Some(src) = self.project.sources.iter_mut().find(|s| s.id == source_id) else {
            return;
        };
        ui.label(RichText::new(src.kind.label()).strong());
        let mut changed = ui
            .horizontal(|ui| {
                ui.label("Nom");
                ui.text_edit_singleline(&mut src.name).changed()
            })
            .inner;
        match &mut src.kind {
            SourceKind::Color { rgba, .. } => {
                let mut c = Color32::from_rgba_unmultiplied(rgba[0], rgba[1], rgba[2], rgba[3]);
                if ui.color_edit_button_srgba(&mut c).changed() {
                    *rgba = c.to_srgba_unmultiplied();
                    changed = true;
                }
            }
            SourceKind::Image { path } => {
                ui.label(RichText::new(path.as_str()).small().weak());
                if ui.button("Changer d'image…").clicked()
                    && let Some(p) = rfd::FileDialog::new()
                        .add_filter("Images", &["png", "jpg", "jpeg", "gif", "bmp", "webp"])
                        .pick_file()
                {
                    *path = p.to_string_lossy().into_owned();
                    changed = true;
                }
            }
            SourceKind::Display { .. } | SourceKind::Window { .. } => {
                ui.label(format!("{fps:.0} images/s capturées"));
            }
        }
        if changed {
            self.touch();
        }
        if let Some(scene) = self.project.scene_mut(current)
            && let Some(item) = scene.items.iter_mut().find(|i| i.id == item_id)
        {
            ui.horizontal(|ui| {
                if ui.button("Ajuster à l'écran").clicked() {
                    item.scale = None;
                    item.x = 0.0;
                    item.y = 0.0;
                }
                if ui.button("Taille d'origine").clicked() {
                    item.scale = Some(1.0);
                }
            });
        }
    }

    fn preview(&mut self, ui: &mut egui::Ui) {
        let Some(tex) = self.preview else { return };
        let (cw, ch) = (self.canvas.0 as f32, self.canvas.1 as f32);
        let (full, resp) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        let k = (full.width() / cw).min(full.height() / ch);
        let rect = Rect::from_center_size(full.center(), vec2(cw * k, ch * k));
        let painter = ui.painter_at(full);
        painter.rect_filled(full, 0.0, Color32::from_gray(12));
        painter.image(
            tex,
            rect,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );

        let to_canvas = |p: Pos2| ((p.x - rect.min.x) / k, (p.y - rect.min.y) / k);
        let sizes = self.shared.sizes.lock().clone();
        let current = self.project.current;
        let item_rect = |it: &crate::model::Item| -> Option<Rect> {
            let (w, h) = *sizes.get(&it.source)?;
            let s = it.scale?;
            Some(Rect::from_min_size(
                pos2(it.x, it.y),
                vec2(w as f32 * s, h as f32 * s),
            ))
        };
        // Clic : sélection de l'élément visible le plus en avant sous le pointeur.
        if (resp.clicked() || resp.drag_started())
            && let Some(p) = resp.interact_pointer_pos()
        {
            let (x, y) = to_canvas(p);
            self.selected = self
                .project
                .scene(current)
                .and_then(|s| {
                    s.items
                        .iter()
                        .filter(|i| i.visible)
                        .find(|i| item_rect(i).is_some_and(|r| r.contains(pos2(x, y))))
                })
                .map(|i| i.id);
        }
        let drag = resp.drag_delta() / k;
        let zoom = if resp.hovered() {
            ui.input(|i| i.smooth_scroll_delta.y)
        } else {
            0.0
        };
        let mut moved = false;
        if let Some(sel) = self.selected
            && let Some(scene) = self.project.scene_mut(current)
            && let Some(item) = scene.items.iter_mut().find(|i| i.id == sel)
        {
            if resp.dragged() && drag != Vec2::ZERO {
                item.x += drag.x;
                item.y += drag.y;
                moved = true;
            }
            // Molette : agrandir ou réduire autour du centre de l'élément.
            if zoom != 0.0
                && let (Some(s), Some(&(w, h))) = (item.scale, sizes.get(&item.source))
            {
                let ns = (s * (1.0 + zoom * 0.0015)).clamp(0.02, 10.0);
                item.x -= w as f32 * (ns - s) / 2.0;
                item.y -= h as f32 * (ns - s) / 2.0;
                item.scale = Some(ns);
                moved = true;
            }
            if let Some(r) = item_rect(item) {
                let screen = Rect::from_min_max(
                    rect.min + r.min.to_vec2() * k,
                    rect.min + r.max.to_vec2() * k,
                );
                painter.rect_stroke(screen, 0.0, Stroke::new(2.0, LIVE), StrokeKind::Outside);
            }
        }
        if moved {
            self.touch();
        }
    }

    fn bottom(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            // Mixeur.
            let a = &mut self.project.settings.audio;
            let mut changed = false;
            for (name, level, vol, muted, err) in [
                (
                    "Micro",
                    *self.mixer.mic.level.lock(),
                    &mut a.mic_volume,
                    &mut a.mic_muted,
                    self.mixer.mic.error.lock().clone(),
                ),
                (
                    "Son du bureau",
                    *self.mixer.desktop.level.lock(),
                    &mut a.desktop_volume,
                    &mut a.desktop_muted,
                    self.mixer.desktop.error.lock().clone(),
                ),
            ] {
                ui.group(|ui| {
                    ui.vertical(|ui| {
                        let mut title = RichText::new(name).strong();
                        if err.is_some() {
                            title = title.color(Color32::from_rgb(0xf5, 0x9e, 0x0b));
                        }
                        let r = ui.label(title);
                        if let Some(e) = err {
                            r.on_hover_text(e);
                        }
                        let (bar, _) = ui.allocate_exact_size(vec2(180.0, 8.0), Sense::hover());
                        let p = ui.painter();
                        p.rect_filled(bar, 2.0, Color32::from_gray(40));
                        let lvl = if *muted { 0.0 } else { level.min(1.0) };
                        let color = if lvl > 0.9 {
                            LIVE
                        } else if lvl > 0.6 {
                            Color32::from_rgb(0xf5, 0x9e, 0x0b)
                        } else {
                            Color32::from_rgb(0x22, 0xc5, 0x5e)
                        };
                        p.rect_filled(
                            Rect::from_min_size(bar.min, vec2(bar.width() * lvl, bar.height())),
                            2.0,
                            color,
                        );
                        ui.horizontal(|ui| {
                            changed |= ui
                                .add(egui::Slider::new(vol, 0.0..=2.0).show_value(false))
                                .changed();
                            ui.label(format!("{:.0} %", *vol * 100.0));
                            if ui
                                .selectable_label(
                                    *muted,
                                    if *muted { "🔇 Coupé" } else { "🔈 Couper" },
                                )
                                .on_hover_text("Couper ou rétablir le son")
                                .clicked()
                            {
                                *muted = !*muted;
                                changed = true;
                            }
                        });
                    });
                });
            }
            if changed {
                *self.mixer.gains.lock() = Gains {
                    mic: a.mic_volume,
                    mic_muted: a.mic_muted,
                    desktop: a.desktop_volume,
                    desktop_muted: a.desktop_muted,
                };
                self.touch();
            }

            ui.separator();
            // Commandes.
            ui.vertical(|ui| {
                let streaming = self.live(Kind::Stream);
                let recording = self.live(Kind::Record);
                let b = egui::Button::new(
                    RichText::new(if streaming {
                        "⏹ Arrêter le streaming"
                    } else {
                        "⏺ Démarrer le streaming"
                    })
                    .strong(),
                )
                .fill(if streaming { LIVE } else { ACCENT })
                .min_size(vec2(220.0, 30.0));
                if ui.add(b).clicked() {
                    self.toggle_output(Kind::Stream);
                }
                let b = egui::Button::new(if recording {
                    "⏹ Arrêter l'enregistrement"
                } else {
                    "⏺ Démarrer l'enregistrement"
                })
                .min_size(vec2(220.0, 26.0));
                if ui.add(b).clicked() {
                    self.toggle_output(Kind::Record);
                }
                if ui
                    .add(egui::Button::new("⚙ Paramètres").min_size(vec2(220.0, 22.0)))
                    .clicked()
                {
                    self.draft = self.project.settings.clone();
                    self.settings_open = true;
                }
            });
        });
        ui.separator();
        // Barre d'état.
        let st = self.shared.stats.lock().clone();
        let mut parts = vec![
            format!("{:.1} / {} i/s", st.fps, self.project.settings.fps),
            format!("rendu {:.1} ms", st.render_ms),
        ];
        parts.push(format!("{} image(s) manquée(s) au rendu", st.lagged));
        if st.dropped > 0 {
            parts.push(format!("{} perdue(s) à l'encodage", st.dropped));
        }
        for (k, out) in self.shared.outputs.lock().iter() {
            let s = out.stats.lock().clone();
            let t = out.started.elapsed().as_secs();
            let name = if *k == key(Kind::Stream) {
                "LIVE"
            } else {
                "REC"
            };
            parts.push(format!(
                "{name} {:02}:{:02}:{:02} · {:.0} kb/s · {} ({} écartée(s))",
                t / 3600,
                t / 60 % 60,
                t % 60,
                s.bitrate_kbps,
                output::encoder_label(&out.encoder),
                s.drops
            ));
        }
        ui.horizontal(|ui| {
            ui.label(RichText::new(parts.join("  ·  ")).small());
            if let Some((msg, err, at)) = &self.notice {
                if at.elapsed() < Duration::from_secs(12) {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(msg).color(if *err {
                            LIVE
                        } else {
                            Color32::from_rgb(0x22, 0xc5, 0x5e)
                        }));
                    });
                } else {
                    self.notice = None;
                }
            }
        });
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        if !self.settings_open {
            return;
        }
        let mut open = true;
        let mut apply = false;
        let mut cancel = false;
        let busy = !self.shared.outputs.lock().is_empty();
        egui::Window::new("Paramètres")
            .open(&mut open)
            .resizable(false)
            .collapsible(false)
            .show(ctx, |ui| {
                let d = &mut self.draft;
                ui.heading("Vidéo");
                ui.add_enabled_ui(!busy, |ui| {
                    egui::Grid::new("video").num_columns(2).show(ui, |ui| {
                        ui.label("Résolution");
                        egui::ComboBox::from_id_salt("res")
                            .selected_text(format!("{} × {}", d.width, d.height))
                            .show_ui(ui, |ui| {
                                for (w, h) in RESOLUTIONS {
                                    if ui
                                        .selectable_label(
                                            d.width == w && d.height == h,
                                            format!("{w} × {h}"),
                                        )
                                        .clicked()
                                    {
                                        d.width = w;
                                        d.height = h;
                                    }
                                }
                            });
                        ui.end_row();
                        ui.label("Images par seconde");
                        egui::ComboBox::from_id_salt("fps")
                            .selected_text(d.fps.to_string())
                            .show_ui(ui, |ui| {
                                for f in [30, 48, 50, 60] {
                                    ui.selectable_value(&mut d.fps, f, f.to_string());
                                }
                            });
                        ui.end_row();
                    });
                });
                if busy {
                    ui.label(
                        RichText::new(
                            "Arrêtez le streaming et l'enregistrement pour changer la vidéo.",
                        )
                        .small()
                        .weak(),
                    );
                }
                ui.separator();
                ui.heading("Stream");
                egui::Grid::new("stream").num_columns(2).show(ui, |ui| {
                    ui.label("Serveur RTMP");
                    ui.add(
                        egui::TextEdit::singleline(&mut d.stream.server)
                            .hint_text("rtmp://live.restream.io/live")
                            .desired_width(320.0),
                    );
                    ui.end_row();
                    ui.label("Clé de stream");
                    ui.add(
                        egui::TextEdit::singleline(&mut d.stream.key)
                            .password(true)
                            .desired_width(320.0),
                    );
                    ui.end_row();
                    ui.label("Débit vidéo (kb/s)");
                    ui.add(
                        egui::DragValue::new(&mut d.stream.video_bitrate)
                            .range(500..=50000)
                            .speed(50),
                    );
                    ui.end_row();
                    ui.label("Débit audio (kb/s)");
                    egui::ComboBox::from_id_salt("ab")
                        .selected_text(d.stream.audio_bitrate.to_string())
                        .show_ui(ui, |ui| {
                            for b in [96, 128, 160, 192, 256, 320] {
                                ui.selectable_value(&mut d.stream.audio_bitrate, b, b.to_string());
                            }
                        });
                    ui.end_row();
                    ui.label("Encodeur");
                    egui::ComboBox::from_id_salt("enc")
                        .selected_text(output::encoder_label(&d.stream.encoder))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut d.stream.encoder,
                                "auto".to_string(),
                                output::encoder_label("auto"),
                            );
                            for e in output::encoders() {
                                ui.selectable_value(
                                    &mut d.stream.encoder,
                                    e.clone(),
                                    output::encoder_label(e),
                                );
                            }
                        });
                    ui.end_row();
                });
                ui.separator();
                ui.heading("Enregistrement");
                egui::Grid::new("rec").num_columns(2).show(ui, |ui| {
                    ui.label("Dossier");
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut d.record.folder).desired_width(260.0),
                        );
                        if ui.button("Parcourir…").clicked()
                            && let Some(p) = rfd::FileDialog::new()
                                .set_directory(&d.record.folder)
                                .pick_folder()
                        {
                            d.record.folder = p.to_string_lossy().into_owned();
                        }
                    });
                    ui.end_row();
                    ui.label("Format");
                    egui::ComboBox::from_id_salt("fmt")
                        .selected_text(d.record.format.to_uppercase())
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut d.record.format,
                                "mkv".to_string(),
                                "MKV (résiste aux coupures, recommandé)",
                            );
                            ui.selectable_value(&mut d.record.format, "mp4".to_string(), "MP4");
                        });
                    ui.end_row();
                    ui.label("Débit vidéo (kb/s)");
                    ui.add(
                        egui::DragValue::new(&mut d.record.video_bitrate)
                            .range(2000..=150000)
                            .speed(100),
                    );
                    ui.end_row();
                });
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button(RichText::new("Appliquer").strong()).clicked() {
                        apply = true;
                    }
                    if ui.button("Annuler").clicked() {
                        cancel = true;
                    }
                });
            });
        if apply {
            let video = (self.draft.width, self.draft.height, self.draft.fps)
                != (
                    self.project.settings.width,
                    self.project.settings.height,
                    self.project.settings.fps,
                );
            let audio = self.project.settings.audio.clone();
            self.project.settings = self.draft.clone();
            self.project.settings.audio = audio; // le mixeur a pu changer pendant que la fenêtre était ouverte
            if video {
                self.runtimes.clear(); // les captures repartent à la nouvelle cadence
                self.shared.sizes.lock().clear();
                self.start_engine(ctx);
            }
            self.touch();
            self.notify("Paramètres enregistrés.", false);
        }
        if !open || apply || cancel {
            self.settings_open = false;
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.sync_runtimes();
        self.check_outputs();
        self.update_items();

        egui::Panel::bottom("bas").show(ui, |ui| self.bottom(ui));
        egui::Panel::left("scenes")
            .default_size(200.0)
            .show(ui, |ui| self.scenes_panel(ui));
        egui::Panel::right("sources")
            .default_size(280.0)
            .show(ui, |ui| self.sources_panel(ui));
        egui::CentralPanel::default().show(ui, |ui| self.preview(ui));
        self.settings_window(&ctx);

        if let Some((start, secs)) = self.autotest {
            let t = start.elapsed();
            if t > Duration::from_secs(2) && !self.live(Kind::Record) && t < Duration::from_secs(3)
            {
                self.toggle_output(Kind::Record);
            }
            if t > Duration::from_secs(2 + secs) {
                self.autotest = None;
                let st = self.shared.stats.lock().clone();
                log::info!(
                    "Test : {:.1} i/s, rendu {:.2} ms, {} manquée(s), {} perdue(s)",
                    st.fps,
                    st.render_ms,
                    st.lagged,
                    st.dropped
                );
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
        if self
            .dirty
            .is_some_and(|t| t.elapsed() > Duration::from_secs(1))
        {
            self.project.save();
            self.dirty = None;
        }
    }

    fn on_exit(&mut self) {
        for (_, out) in self.shared.outputs.lock().drain() {
            out.stop();
        }
        self.engine = None;
        self.project.save();
        // Laisse FFmpeg finaliser les fichiers en cours.
        std::thread::sleep(Duration::from_millis(500));
    }
}
