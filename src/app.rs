//! Interface (egui) : scènes, sources, aperçu, mixeur, commandes et paramètres.

mod control;

use crate::audio::{DESKTOP, MIC, Mixer};
use crate::capture::{self, Runtime};
use crate::compositor::{Compositor, DrawItem};
use crate::engine::{Engine, Shared, TransitionRun, key};
use crate::model::{Filters, HOTKEY_ACTIONS, Project, Settings, SourceKind, TransitionKind};
use crate::output::{self, Kind, Output};
use crate::remote::Remote;
use egui::{Color32, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Vec2, pos2, vec2};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

const RESOLUTIONS: [(u32, u32); 5] = [(1920, 1080), (2560, 1440), (1600, 900), (1280, 720), (3840, 2160)];
const ACCENT: Color32 = Color32::from_rgb(0x8b, 0x5c, 0xf6);
const LIVE: Color32 = Color32::from_rgb(0xe5, 0x48, 0x4d);
const WARN: Color32 = Color32::from_rgb(0xf5, 0x9e, 0x0b);
const OK: Color32 = Color32::from_rgb(0x22, 0xc5, 0x5e);
const IMAGES: [&str; 6] = ["png", "jpg", "jpeg", "gif", "bmp", "webp"];
const MEDIAS: [&str; 10] = ["mp4", "mkv", "mov", "webm", "avi", "flv", "mp3", "wav", "ogg", "m4a"];

pub struct App {
    project: Project,
    runtimes: HashMap<u64, Runtime>,
    shared: Arc<Shared>,
    engine: Option<Engine>,
    render_state: egui_wgpu::RenderState,
    preview: Option<(egui::TextureId, egui::TextureId)>,
    canvas: (u32, u32),
    mixer: Mixer,
    selected: Option<u64>,
    settings_open: bool,
    draft: Settings,
    notice: Option<(String, bool, Instant)>,
    dirty: Option<Instant>,
    renaming: Option<(u64, String)>,
    /// Scène à l'antenne lors de la dernière image (pour déclencher les transitions).
    shown: u64,
    cameras: Option<Vec<String>>,
    /// Test automatique (MYSTREAM_AUTOTEST=<secondes>) : enregistre puis quitte.
    autotest: Option<(Instant, u64)>,
    /// Serveur obs-websocket (Stream Deck…).
    remote: Option<Remote>,
    last_snapshot: control::Snapshot,
    hotkey_manager: Option<global_hotkey::GlobalHotKeyManager>,
    hotkeys: Vec<(global_hotkey::hotkey::HotKey, &'static str)>,
    update: crate::update::Shared,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let render_state = cc.wgpu_render_state.clone().expect("MyStream nécessite le rendu wgpu");
        let project = Project::load();
        let mixer = Mixer::new();
        // Détection des encodeurs (quelques lancements de FFmpeg) en arrière-plan.
        std::thread::spawn(|| {
            let _ = output::encoders();
        });
        let mut app = Self {
            draft: project.settings.clone(),
            shown: project.current,
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
            cameras: None,
            autotest: std::env::var("MYSTREAM_AUTOTEST").ok().and_then(|v| v.parse().ok()).map(|s| (Instant::now(), s)),
            remote: None,
            last_snapshot: Default::default(),
            hotkey_manager: None,
            hotkeys: Vec::new(),
            update: Default::default(),
        };
        crate::update::check(app.update.clone());
        app.start_engine(&cc.egui_ctx);
        app.apply_remote();
        let errors = app.apply_hotkeys();
        if !errors.is_empty() {
            app.notify(format!("Raccourcis refusés : {}", errors.join(", ")), true);
        }
        app.touch(); // enregistre le mot de passe généré au premier lancement
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
            let filter = wgpu::FilterMode::Linear;
            match self.preview {
                Some((p, st)) => {
                    r.update_egui_texture_from_wgpu_texture(&rs.device, comp.view(), filter, p);
                    r.update_egui_texture_from_wgpu_texture(&rs.device, comp.studio_view(), filter, st);
                }
                None => {
                    let p = r.register_native_texture(&rs.device, comp.view(), filter);
                    let st = r.register_native_texture(&rs.device, comp.studio_view(), filter);
                    self.preview = Some((p, st));
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

    /// Démarre ou arrête les sources selon le projet ; règle les pistes du mixeur.
    fn sync_runtimes(&mut self) {
        let fps = self.project.settings.fps;
        self.runtimes.retain(|id, rt| self.project.sources.iter().any(|s| s.id == *id && s.kind == *rt.kind()));
        for s in &self.project.sources {
            if !self.runtimes.contains_key(&s.id) {
                let track = s.kind.has_audio().then(|| self.mixer.track(s.id));
                self.runtimes.insert(s.id, Runtime::start(&s.kind, fps, track));
            }
            if s.kind.has_audio() {
                *self.mixer.track(s.id).gain.lock() = (s.volume, s.muted);
            }
        }
        let sources = &self.project.sources;
        self.mixer.retain(|id| sources.iter().any(|s| s.id == id && s.kind.has_audio()));
        let a = &self.project.settings.audio;
        if let Some(t) = self.mixer.get(MIC) {
            *t.gain.lock() = (a.mic_volume, a.mic_muted);
        }
        if let Some(t) = self.mixer.get(DESKTOP) {
            *t.gain.lock() = (a.desktop_volume, a.desktop_muted);
        }
        let mut slots = self.shared.slots.lock();
        slots.retain(|id, _| self.runtimes.contains_key(id));
        for (id, rt) in &self.runtimes {
            slots.entry(*id).or_insert_with(|| rt.slot.clone());
        }
    }

    /// Liste de dessin d'une scène ; place les nouveaux éléments (réduits s'ils dépassent).
    fn draw_list(&mut self, scene: u64, sizes: &HashMap<u64, (u32, u32)>) -> Vec<DrawItem> {
        let (cw, ch) = (self.canvas.0 as f32, self.canvas.1 as f32);
        let filters: HashMap<u64, Filters> = self.project.sources.iter().map(|s| (s.id, s.filters.clone())).collect();
        let mut changed = false;
        let mut list = Vec::new();
        if let Some(scene) = self.project.scene_mut(scene) {
            for item in scene.items.iter_mut() {
                let Some(&(w, h)) = sizes.get(&item.source) else {
                    continue;
                };
                let f = filters.get(&item.source).cloned().unwrap_or_default();
                let (vw, vh) = (w.saturating_sub(f.crop[0] + f.crop[2]).max(1) as f32, h.saturating_sub(f.crop[1] + f.crop[3]).max(1) as f32);
                let scale = *item.scale.get_or_insert_with(|| {
                    changed = true;
                    (cw / vw).min(ch / vh).min(1.0)
                });
                if item.visible {
                    list.push(DrawItem { source: item.source, x: item.x, y: item.y, w: vw * scale, h: vh * scale, filters: f, alpha: 1.0 });
                }
            }
        }
        if changed {
            self.touch();
        }
        list
    }

    fn update_items(&mut self) {
        let sizes = self.shared.sizes.lock().clone();
        let program = self.draw_list(self.project.current, &sizes);
        // Changement de scène à l'antenne : transition depuis l'image précédente.
        if self.shown != self.project.current {
            let t = &self.project.settings.transition;
            if t.kind != TransitionKind::Cut && t.duration_ms > 0 {
                let from = self.shared.items.lock().clone();
                *self.shared.transition.lock() =
                    Some(TransitionRun { from, kind: t.kind, start: Instant::now(), duration: Duration::from_millis(t.duration_ms as u64) });
            }
            self.shown = self.project.current;
        }
        *self.shared.items.lock() = program;
        let studio = self.project.settings.studio.then(|| self.draw_list(self.project.preview, &sizes));
        *self.shared.studio.lock() = studio;
    }

    fn check_outputs(&mut self) {
        let ended: Vec<(u8, Option<String>, Kind)> =
            self.shared.outputs.lock().iter().filter(|(_, o)| o.ended()).map(|(k, o)| (*k, o.stats.lock().error.clone(), o.kind)).collect();
        for (k, err, kind) in ended {
            let out = self.shared.outputs.lock().remove(&k);
            if let Some(out) = out {
                out.stop();
            }
            self.notify(format!("{} s'est arrêté : {}", kind.label(), err.unwrap_or_else(|| "fin inattendue".into())), true);
        }
    }

    fn toggle_output(&mut self, kind: Kind) {
        let running = self.shared.outputs.lock().remove(&key(kind));
        if let Some(out) = running {
            let file = out.file.clone();
            out.stop();
            if let Some(f) = file {
                self.notify(format!("Enregistrement sauvegardé : {}", f.display()), false);
            }
            return;
        }
        let (tx, rx) = crossbeam_channel::bounded(100);
        self.mixer.subscribe(tx);
        match Output::start(kind, &self.project.settings, self.canvas.0, self.canvas.1, rx) {
            Ok(out) => {
                self.shared.outputs.lock().insert(key(kind), out);
            }
            Err(e) => self.notify(format!("{} n'a pas pu démarrer : {e:#}", kind.label()), true),
        }
    }

    /// Mode studio : la scène de l'aperçu passe à l'antenne.
    fn transition(&mut self) {
        if self.project.preview != self.project.current {
            self.project.current = self.project.preview;
            self.touch();
        }
    }

    /* Panneaux ------------------------------------------------------------- */

    fn scenes_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Scènes");
        ui.separator();
        let studio = self.project.settings.studio;
        let mut pick = None;
        let ids: Vec<(u64, String)> = self.project.scenes.iter().map(|s| (s.id, s.name.clone())).collect();
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
            let selected = if studio { self.project.preview == id } else { self.project.current == id };
            let mut text = RichText::new(&name);
            if studio && self.project.current == id {
                text = text.color(LIVE); // à l'antenne
            }
            let r = ui.selectable_label(selected, text);
            if r.clicked() {
                pick = Some(id);
            }
            if r.double_clicked() {
                if studio {
                    // Double-clic en mode studio : passe directement à l'antenne.
                    self.project.current = id;
                } else {
                    self.renaming = Some((id, name.clone()));
                }
            }
        }
        if let Some(id) = pick {
            if studio {
                self.project.preview = id;
            } else {
                self.project.current = id;
            }
            self.selected = None;
            self.touch();
        }
        ui.separator();
        ui.horizontal(|ui| {
            if ui.button("➕").on_hover_text("Ajouter une scène").clicked() {
                let id = self.project.add_scene();
                if studio {
                    self.project.preview = id;
                } else {
                    self.project.current = id;
                }
                self.touch();
            }
            if ui.add_enabled(self.project.scenes.len() > 1, egui::Button::new("➖")).on_hover_text("Supprimer la scène").clicked() {
                let id = self.project.edit_scene();
                self.project.remove_scene(id);
                self.touch();
            }
            if ui.button("✏").on_hover_text("Renommer").clicked() {
                let id = self.project.edit_scene();
                let name = self.project.scene(id).map(|s| s.name.clone()).unwrap_or_default();
                self.renaming = Some((id, name));
            }
        });
        ui.separator();
        ui.label(RichText::new("Transition").strong());
        let t = &mut self.project.settings.transition;
        let mut changed = false;
        egui::ComboBox::from_id_salt("transition").selected_text(t.kind.label()).show_ui(ui, |ui| {
            for k in [TransitionKind::Cut, TransitionKind::Fade, TransitionKind::Slide] {
                changed |= ui.selectable_value(&mut t.kind, k, k.label()).changed();
            }
        });
        if t.kind != TransitionKind::Cut {
            changed |= ui.add(egui::Slider::new(&mut t.duration_ms, 100..=2000).suffix(" ms").text("Durée")).changed();
        }
        if changed {
            self.touch();
        }
        ui.separator();
        if ui.button("📥 Importer depuis OBS…").on_hover_text("Ajoute les scènes et sources d'une collection OBS").clicked() {
            let mut dlg = rfd::FileDialog::new().add_filter("Collection OBS", &["json"]);
            if let Some(dir) = crate::obs_import::obs_folder() {
                dlg = dlg.set_directory(dir);
            }
            if let Some(path) = dlg.pick_file() {
                match crate::obs_import::import(&path, &mut self.project) {
                    Ok(r) => {
                        let mut msg = format!("Importé depuis OBS : {} scène(s), {} source(s).", r.scenes, r.sources);
                        if !r.skipped.is_empty() {
                            let list: Vec<String> = r.skipped.iter().map(|(k, n)| format!("{} × {}", n, crate::obs_import::obs_type_label(k))).collect();
                            msg.push_str(&format!(" Non prises en charge : {}.", list.join(", ")));
                        }
                        self.notify(msg, false);
                        self.touch();
                    }
                    Err(e) => self.notify(format!("Import OBS impossible : {e}"), true),
                }
            }
        }
    }

    fn sources_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Sources");
        ui.separator();
        let scene_id = self.project.edit_scene();
        let items: Vec<(u64, u64, bool)> =
            self.project.scene(scene_id).map(|s| s.items.iter().map(|i| (i.id, i.source, i.visible)).collect()).unwrap_or_default();
        if items.is_empty() {
            ui.label(RichText::new("Cliquez sur « Ajouter » pour ajouter une source.").weak());
        }
        let mut action: Option<(&str, u64)> = None;
        egui::ScrollArea::vertical().id_salt("sources").max_height(260.0).show(ui, |ui| {
            for (item, source, visible) in &items {
                let Some(src) = self.project.source(*source) else {
                    continue;
                };
                let rt = self.runtimes.get(source);
                let status = rt.and_then(|r| r.error());
                let info = rt.and_then(|r| r.info());
                let shared = self.project.scenes.iter().filter(|s| s.items.iter().any(|i| i.source == *source)).count() > 1;
                ui.horizontal(|ui| {
                    let eye = if *visible { "👁" } else { "⊘" };
                    if ui.small_button(eye).on_hover_text(if *visible { "Masquer" } else { "Afficher" }).clicked() {
                        action = Some(("visible", *item));
                    }
                    let mut text = format!("{} {}", src.kind.glyph(), src.name);
                    if shared {
                        text.push_str(" 🔗");
                    }
                    let mut label = RichText::new(text);
                    if status.is_some() {
                        label = label.color(WARN);
                    }
                    let r = ui.selectable_label(self.selected == Some(*item), label);
                    let hover = [status.clone(), info.clone(), shared.then(|| "Source partagée avec d'autres scènes".to_string())]
                        .into_iter()
                        .flatten()
                        .collect::<Vec<_>>()
                        .join("\n");
                    let r = if hover.is_empty() { r } else { r.on_hover_text(hover) };
                    if r.clicked() {
                        self.selected = Some(*item);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("🗑").on_hover_text("Retirer de la scène").clicked() {
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
                if let Some(i) = info {
                    ui.label(RichText::new(format!("      {i}")).small().weak());
                }
            }
        });
        if let Some((what, id)) = action {
            if what == "remove" {
                self.project.remove_item(scene_id, id);
                if self.selected == Some(id) {
                    self.selected = None;
                }
            } else if let Some(scene) = self.project.scene_mut(scene_id)
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
        egui::ScrollArea::vertical().id_salt("proprietes").show(ui, |ui| self.properties(ui));
    }

    fn add(&mut self, name: &str, kind: SourceKind) {
        self.project.add_source(name, kind);
        let scene = self.project.edit_scene();
        self.selected = self.project.scene(scene).and_then(|s| s.items.first()).map(|i| i.id);
        self.touch();
    }

    fn add_menu(&mut self, ui: &mut egui::Ui) {
        if cfg!(windows)
            && ui.button("🎮 Capture de jeu (automatique)").on_hover_text("Capture le jeu en plein écran, avec son son, sans rien choisir").clicked()
        {
            ui.close();
            self.add("Capture de jeu", SourceKind::Game { audio: true });
        }
        ui.menu_button("🖥 Capture d'écran", |ui| {
            for m in capture::monitors() {
                if ui.button(&m.name).clicked() {
                    ui.close();
                    self.add("Capture d'écran", SourceKind::Display { monitor: m.index });
                }
            }
        });
        if cfg!(windows) {
            ui.menu_button("🗔 Capture de fenêtre", |ui| {
                egui::ScrollArea::vertical().max_height(400.0).show(ui, |ui| {
                    for title in capture::windows() {
                        if ui.button(&title).clicked() {
                            ui.close();
                            let name: String = title.chars().take(40).collect();
                            self.add(&name, SourceKind::Window { title: title.clone(), audio: true });
                        }
                    }
                });
            });
        }
        ui.menu_button("📷 Caméra", |ui| {
            let cams = self.cameras.get_or_insert_with(capture::cameras).clone();
            if cams.is_empty() {
                ui.label("Aucune caméra trouvée");
            }
            for c in cams {
                if ui.button(&c).clicked() {
                    ui.close();
                    self.add(&c, SourceKind::Camera { device: c.clone(), width: 1920, height: 1080, fps: 30 });
                }
            }
            if ui.button("Actualiser la liste").clicked() {
                self.cameras = None;
            }
        });
        if ui.button("T Texte").clicked() {
            ui.close();
            self.add("Texte", SourceKind::Text { text: "Votre texte".into(), size: 72.0, rgba: [255, 255, 255, 255], background: [0, 0, 0, 0], mono: false });
        }
        if ui.button("🖼 Image…").clicked() {
            ui.close();
            if let Some(path) = rfd::FileDialog::new().add_filter("Images", &IMAGES).pick_file() {
                let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Image".into());
                self.add(&name, SourceKind::Image { path: path.to_string_lossy().into_owned() });
            }
        }
        if ui.button("🎞 Média (vidéo, son)…").clicked() {
            ui.close();
            if let Some(path) = rfd::FileDialog::new().add_filter("Médias", &MEDIAS).pick_file() {
                let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Média".into());
                self.add(&name, SourceKind::Media { path: path.to_string_lossy().into_owned(), looping: true });
            }
        }
        if ui.button("■ Couleur").clicked() {
            ui.close();
            let (w, h) = self.canvas;
            self.add("Couleur", SourceKind::Color { rgba: [0x2a, 0x1f, 0x4d, 255], width: w, height: h });
        }
        if cfg!(windows) {
            ui.menu_button("🎵 Son d'une application", |ui| {
                ui.label(RichText::new("Discord, navigateur, lecteur de musique… même en arrière-plan").small().weak());
                egui::ScrollArea::vertical().max_height(400.0).show(ui, |ui| {
                    for exe in capture::programs() {
                        if ui.button(&exe).clicked() {
                            ui.close();
                            let name = exe.trim_end_matches(".exe").trim_end_matches(".EXE").to_string();
                            self.add(&name, SourceKind::AppAudio { exe: exe.clone() });
                        }
                    }
                });
            });
        }
        // Sources d'autres scènes, à partager (comme « Ajouter une source existante » d'OBS).
        let scene = self.project.edit_scene();
        let others: Vec<(u64, String)> = self
            .project
            .sources
            .iter()
            .filter(|s| !self.project.scene(scene).is_some_and(|sc| sc.items.iter().any(|i| i.source == s.id)))
            .map(|s| (s.id, format!("{} {}", s.kind.glyph(), s.name)))
            .collect();
        if !others.is_empty() {
            ui.separator();
            ui.menu_button("🔗 Source existante (partagée)", |ui| {
                for (id, label) in others {
                    if ui.button(label).clicked() {
                        ui.close();
                        self.project.add_existing(scene, id);
                        self.touch();
                    }
                }
            });
        }
    }

    /// Réglages et filtres de la source sélectionnée.
    fn properties(&mut self, ui: &mut egui::Ui) {
        let scene_id = self.project.edit_scene();
        let Some(item_id) = self.selected else { return };
        let Some(source_id) = self.project.scene(scene_id).and_then(|s| s.items.iter().find(|i| i.id == item_id)).map(|i| i.source) else {
            return;
        };
        let fps = self.runtimes.get(&source_id).map(|r| *r.status.fps.lock()).unwrap_or(0.0);
        let mut cams = None;
        let Some(src) = self.project.sources.iter_mut().find(|s| s.id == source_id) else {
            return;
        };
        ui.label(RichText::new(format!("{} {}", src.kind.glyph(), src.kind.label())).strong());
        let mut changed = ui
            .horizontal(|ui| {
                ui.label("Nom");
                ui.text_edit_singleline(&mut src.name).changed()
            })
            .inner;
        let color = |ui: &mut egui::Ui, rgba: &mut [u8; 4]| -> bool {
            let mut c = Color32::from_rgba_unmultiplied(rgba[0], rgba[1], rgba[2], rgba[3]);
            let r = ui.color_edit_button_srgba(&mut c).changed();
            if r {
                *rgba = c.to_srgba_unmultiplied();
            }
            r
        };
        match &mut src.kind {
            SourceKind::Color { rgba, .. } => {
                changed |= color(ui, rgba);
            }
            SourceKind::Image { path } => {
                ui.label(RichText::new(path.as_str()).small().weak());
                if ui.button("Changer d'image…").clicked()
                    && let Some(p) = rfd::FileDialog::new().add_filter("Images", &IMAGES).pick_file()
                {
                    *path = p.to_string_lossy().into_owned();
                    changed = true;
                }
            }
            SourceKind::Text { text, size, rgba, background, mono } => {
                changed |= ui.add(egui::TextEdit::multiline(text).desired_rows(2)).changed();
                changed |= ui.add(egui::Slider::new(size, 8.0..=300.0).text("Taille")).changed();
                ui.horizontal(|ui| {
                    ui.label("Couleur");
                    changed |= color(ui, rgba);
                    ui.label("Fond");
                    changed |= color(ui, background);
                });
                changed |= ui.checkbox(mono, "Police à chasse fixe").changed();
            }
            SourceKind::Media { path, looping } => {
                ui.label(RichText::new(path.as_str()).small().weak());
                if ui.button("Changer de fichier…").clicked()
                    && let Some(p) = rfd::FileDialog::new().add_filter("Médias", &MEDIAS).pick_file()
                {
                    *path = p.to_string_lossy().into_owned();
                    changed = true;
                }
                changed |= ui.checkbox(looping, "Lire en boucle").changed();
            }
            SourceKind::Camera { device, width, height, fps: cam_fps } => {
                egui::ComboBox::from_label("Appareil").selected_text(device.as_str()).show_ui(ui, |ui| {
                    for c in cams.get_or_insert_with(capture::cameras).iter() {
                        changed |= ui.selectable_value(device, c.clone(), c).changed();
                    }
                });
                egui::ComboBox::from_label("Résolution").selected_text(format!("{width} × {height}")).show_ui(ui, |ui| {
                    for (w, h) in [(1920, 1080), (1280, 720), (640, 480)] {
                        if ui.selectable_label(*width == w && *height == h, format!("{w} × {h}")).clicked() {
                            (*width, *height) = (w, h);
                            changed = true;
                        }
                    }
                });
                egui::ComboBox::from_label("Images par seconde").selected_text(cam_fps.to_string()).show_ui(ui, |ui| {
                    for f in [30, 60] {
                        changed |= ui.selectable_value(cam_fps, f, f.to_string()).changed();
                    }
                });
                ui.label(format!("{fps:.0} images/s reçues"));
            }
            SourceKind::Window { audio, .. } => {
                changed |= ui.checkbox(audio, "Capturer le son de la fenêtre").changed();
                ui.label(format!("{fps:.0} images/s capturées"));
            }
            SourceKind::Game { audio } => {
                changed |= ui.checkbox(audio, "Capturer le son du jeu").changed();
                ui.label(format!("{fps:.0} images/s capturées"));
            }
            SourceKind::Display { .. } => {
                ui.label(format!("{fps:.0} images/s capturées"));
            }
            SourceKind::AppAudio { exe } => {
                ui.label(format!("Programme : {exe}"));
            }
        }
        if !src.kind.audio_only() {
            egui::CollapsingHeader::new("Filtres").show(ui, |ui| {
                let f = &mut src.filters;
                changed |= ui.add(egui::Slider::new(&mut f.opacity, 0.0..=1.0).text("Opacité")).changed();
                changed |= ui.add(egui::Slider::new(&mut f.brightness, -0.5..=0.5).text("Luminosité")).changed();
                changed |= ui.add(egui::Slider::new(&mut f.contrast, 0.0..=2.0).text("Contraste")).changed();
                changed |= ui.add(egui::Slider::new(&mut f.saturation, 0.0..=2.0).text("Saturation")).changed();
                ui.label("Recadrage (pixels) : gauche, haut, droite, bas");
                ui.horizontal(|ui| {
                    for v in f.crop.iter_mut() {
                        changed |= ui.add(egui::DragValue::new(v).range(0..=8000)).changed();
                    }
                });
                ui.horizontal(|ui| {
                    changed |= ui.checkbox(&mut f.flip_h, "Miroir horizontal").changed();
                    changed |= ui.checkbox(&mut f.flip_v, "Retourner").changed();
                });
                changed |= ui.checkbox(&mut f.chroma, "Incrustation (fond vert)").changed();
                if f.chroma {
                    ui.horizontal(|ui| {
                        ui.label("Couleur clé");
                        let mut c = Color32::from_rgb(f.chroma_color[0], f.chroma_color[1], f.chroma_color[2]);
                        if ui.color_edit_button_srgba(&mut c).changed() {
                            f.chroma_color = [c.r(), c.g(), c.b()];
                            changed = true;
                        }
                    });
                    changed |= ui.add(egui::Slider::new(&mut f.similarity, 0.0..=1.0).text("Similarité")).changed();
                    changed |= ui.add(egui::Slider::new(&mut f.smoothness, 0.0..=0.5).text("Lissage")).changed();
                }
                if ui.button("Réinitialiser les filtres").clicked() {
                    *f = Filters::default();
                    changed = true;
                }
            });
        }
        if changed {
            self.touch();
        }
        if let Some(scene) = self.project.scene_mut(scene_id)
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

    /// Aperçu interactif d'une scène (sélection, déplacement, molette pour la taille).
    fn canvas_view(&mut self, ui: &mut egui::Ui, tex: egui::TextureId, scene_id: u64, interactive: bool, size: Vec2) {
        let (cw, ch) = (self.canvas.0 as f32, self.canvas.1 as f32);
        let (full, resp) = ui.allocate_exact_size(size, if interactive { Sense::click_and_drag() } else { Sense::hover() });
        let k = (full.width() / cw).min(full.height() / ch);
        let rect = Rect::from_center_size(full.center(), vec2(cw * k, ch * k));
        let painter = ui.painter_at(full);
        painter.rect_filled(full, 0.0, Color32::from_gray(12));
        painter.image(tex, rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        if !interactive {
            return;
        }
        let to_canvas = |p: Pos2| ((p.x - rect.min.x) / k, (p.y - rect.min.y) / k);
        let sizes = self.shared.sizes.lock().clone();
        let filters: HashMap<u64, [u32; 4]> = self.project.sources.iter().map(|s| (s.id, s.filters.crop)).collect();
        let item_rect = |it: &crate::model::Item| -> Option<Rect> {
            let (w, h) = *sizes.get(&it.source)?;
            let c = filters.get(&it.source).copied().unwrap_or([0; 4]);
            let s = it.scale?;
            Some(Rect::from_min_size(pos2(it.x, it.y), vec2(w.saturating_sub(c[0] + c[2]) as f32 * s, h.saturating_sub(c[1] + c[3]) as f32 * s)))
        };
        // Clic : sélection de l'élément visible le plus en avant sous le pointeur.
        if (resp.clicked() || resp.drag_started())
            && let Some(p) = resp.interact_pointer_pos()
        {
            let (x, y) = to_canvas(p);
            self.selected = self
                .project
                .scene(scene_id)
                .and_then(|s| s.items.iter().filter(|i| i.visible).find(|i| item_rect(i).is_some_and(|r| r.contains(pos2(x, y)))))
                .map(|i| i.id);
        }
        let drag = resp.drag_delta() / k;
        let zoom = if resp.hovered() { ui.input(|i| i.smooth_scroll_delta.y) } else { 0.0 };
        let mut moved = false;
        if let Some(sel) = self.selected
            && let Some(scene) = self.project.scene_mut(scene_id)
            && let Some(item) = scene.items.iter_mut().find(|i| i.id == sel)
        {
            if resp.dragged() && drag != Vec2::ZERO {
                item.x += drag.x;
                item.y += drag.y;
                moved = true;
            }
            // Molette : agrandir ou réduire autour du centre de l'élément.
            if zoom != 0.0
                && let (Some(s), Some(r)) = (item.scale, item_rect(item))
            {
                let ns = (s * (1.0 + zoom * 0.0015)).clamp(0.02, 10.0);
                item.x -= r.width() / s * (ns - s) / 2.0;
                item.y -= r.height() / s * (ns - s) / 2.0;
                item.scale = Some(ns);
                moved = true;
            }
            if let Some(r) = item_rect(item) {
                let screen = Rect::from_min_max(rect.min + r.min.to_vec2() * k, rect.min + r.max.to_vec2() * k);
                painter.rect_stroke(screen, 0.0, Stroke::new(2.0, LIVE), StrokeKind::Outside);
            }
        }
        if moved {
            self.touch();
        }
    }

    fn preview(&mut self, ui: &mut egui::Ui) {
        let Some((program, studio)) = self.preview else {
            return;
        };
        if !self.project.settings.studio {
            let size = ui.available_size();
            self.canvas_view(ui, program, self.project.current, true, size);
            return;
        }
        // Mode studio : aperçu (modifiable) à gauche, antenne à droite.
        let avail = ui.available_size();
        let w = (avail.x - 140.0) / 2.0;
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new("Aperçu").strong());
                let preview = self.project.preview;
                self.canvas_view(ui, studio, preview, true, vec2(w, avail.y - 24.0));
            });
            ui.vertical(|ui| {
                ui.add_space(avail.y / 2.0 - 30.0);
                let b = egui::Button::new(RichText::new("Transition ➡").strong()).fill(ACCENT).min_size(vec2(120.0, 36.0));
                if ui.add(b).on_hover_text("Passer la scène de l'aperçu à l'antenne").clicked() {
                    self.transition();
                }
            });
            ui.vertical(|ui| {
                ui.label(RichText::new("À l'antenne").strong().color(LIVE));
                let current = self.project.current;
                self.canvas_view(ui, program, current, false, vec2(w, avail.y - 24.0));
            });
        });
    }

    fn mixer_strip(ui: &mut egui::Ui, name: &str, level: f32, vol: &mut f32, muted: &mut bool, err: Option<String>) -> bool {
        let mut changed = false;
        ui.group(|ui| {
            ui.set_width(190.0);
            ui.vertical(|ui| {
                let mut title = RichText::new(name).strong();
                if err.is_some() {
                    title = title.color(WARN);
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
                    WARN
                } else {
                    OK
                };
                p.rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * lvl, bar.height())), 2.0, color);
                ui.horizontal(|ui| {
                    changed |= ui.add(egui::Slider::new(vol, 0.0..=2.0).show_value(false)).changed();
                    ui.label(format!("{:.0} %", *vol * 100.0));
                    if ui.selectable_label(*muted, if *muted { "🔇" } else { "🔈" }).on_hover_text("Couper ou rétablir le son").clicked() {
                        *muted = !*muted;
                        changed = true;
                    }
                });
            });
        });
        changed
    }

    fn bottom(&mut self, ui: &mut egui::Ui) {
        self.update_banner(ui);
        ui.horizontal(|ui| {
            // Mixeur : micro, son du bureau, puis une piste par source sonore.
            egui::ScrollArea::horizontal().id_salt("mixeur").max_width(ui.available_width() - 250.0).show(ui, |ui| {
                ui.horizontal(|ui| {
                    let mut changed = false;
                    for (id, name) in [(MIC, "Micro"), (DESKTOP, "Son du bureau")] {
                        let t = self.mixer.get(id);
                        let level = t.as_ref().map(|t| *t.level.lock()).unwrap_or(0.0);
                        let err = t.as_ref().and_then(|t| t.error.lock().clone());
                        let a = &mut self.project.settings.audio;
                        let (vol, muted) = if id == MIC { (&mut a.mic_volume, &mut a.mic_muted) } else { (&mut a.desktop_volume, &mut a.desktop_muted) };
                        changed |= Self::mixer_strip(ui, name, level, vol, muted, err);
                    }
                    for s in self.project.sources.iter_mut().filter(|s| s.kind.has_audio()) {
                        let t = self.mixer.get(s.id);
                        let level = t.as_ref().map(|t| *t.level.lock()).unwrap_or(0.0);
                        let err = t.as_ref().and_then(|t| t.error.lock().clone());
                        changed |= Self::mixer_strip(ui, &s.name, level, &mut s.volume, &mut s.muted, err);
                    }
                    if changed {
                        self.touch();
                    }
                });
            });

            ui.separator();
            // Commandes.
            ui.vertical(|ui| {
                let streaming = self.live(Kind::Stream);
                let recording = self.live(Kind::Record);
                let b = egui::Button::new(RichText::new(if streaming { "⏹ Arrêter le streaming" } else { "⏺ Démarrer le streaming" }).strong())
                    .fill(if streaming { LIVE } else { ACCENT })
                    .min_size(vec2(220.0, 30.0));
                if ui.add(b).clicked() {
                    self.toggle_output(Kind::Stream);
                }
                let b = egui::Button::new(if recording { "⏹ Arrêter l'enregistrement" } else { "⏺ Démarrer l'enregistrement" }).min_size(vec2(220.0, 26.0));
                if ui.add(b).clicked() {
                    self.toggle_output(Kind::Record);
                }
                let studio = self.project.settings.studio;
                if ui
                    .add(egui::Button::new(if studio { "🎬 Quitter le mode studio" } else { "🎬 Mode studio" }).selected(studio).min_size(vec2(220.0, 22.0)))
                    .clicked()
                {
                    self.project.settings.studio = !studio;
                    self.project.preview = self.project.current;
                    self.touch();
                }
                if ui.add(egui::Button::new("⚙ Paramètres").min_size(vec2(220.0, 22.0))).clicked() {
                    self.draft = self.project.settings.clone();
                    self.settings_open = true;
                }
            });
        });
        ui.separator();
        // Barre d'état.
        let st = self.shared.stats.lock().clone();
        let mut parts = vec![format!("{:.1} / {} i/s", st.fps, self.project.settings.fps), format!("rendu {:.1} ms", st.render_ms)];
        parts.push(format!("{} image(s) manquée(s) au rendu", st.lagged));
        if st.dropped > 0 {
            parts.push(format!("{} perdue(s) à l'encodage", st.dropped));
        }
        for (k, out) in self.shared.outputs.lock().iter() {
            let s = out.stats.lock().clone();
            let t = out.started.elapsed().as_secs();
            let name = if *k == key(Kind::Stream) { "LIVE" } else { "REC" };
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
                        ui.label(RichText::new(msg).color(if *err { LIVE } else { OK }));
                    });
                } else {
                    self.notice = None;
                }
            }
        });
    }

    fn update_banner(&mut self, ui: &mut egui::Ui) {
        use crate::update::State;
        let state = self.update.lock().clone();
        let busy = !self.shared.outputs.lock().is_empty();
        match state {
            State::Idle => {}
            State::Available(a) => {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("Une nouvelle version est disponible (natif 0.1.{}).", a.number)).color(OK));
                    let b = ui.add_enabled(!busy, egui::Button::new("Installer et redémarrer"));
                    let b = if busy { b.on_disabled_hover_text("Arrêtez le streaming et l'enregistrement d'abord") } else { b };
                    if b.clicked() {
                        crate::update::install(self.update.clone(), a.clone());
                    }
                });
            }
            State::Downloading => {
                ui.label(RichText::new("Téléchargement de la mise à jour…").color(WARN));
            }
            State::Ready => {
                self.project.save();
                crate::update::restart();
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
            State::Failed(e) => {
                ui.label(RichText::new(format!("La mise à jour a échoué : {e}")).color(LIVE));
            }
        }
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        if !self.settings_open {
            return;
        }
        let mut open = true;
        let mut apply = false;
        let mut cancel = false;
        let busy = !self.shared.outputs.lock().is_empty();
        egui::Window::new("Paramètres").open(&mut open).resizable(false).collapsible(false).show(ctx, |ui| {
            let d = &mut self.draft;
            ui.heading("Vidéo");
            ui.add_enabled_ui(!busy, |ui| {
                egui::Grid::new("video").num_columns(2).show(ui, |ui| {
                    ui.label("Résolution");
                    egui::ComboBox::from_id_salt("res").selected_text(format!("{} × {}", d.width, d.height)).show_ui(ui, |ui| {
                        for (w, h) in RESOLUTIONS {
                            if ui.selectable_label(d.width == w && d.height == h, format!("{w} × {h}")).clicked() {
                                d.width = w;
                                d.height = h;
                            }
                        }
                    });
                    ui.end_row();
                    ui.label("Images par seconde");
                    egui::ComboBox::from_id_salt("fps").selected_text(d.fps.to_string()).show_ui(ui, |ui| {
                        for f in [30, 48, 50, 60] {
                            ui.selectable_value(&mut d.fps, f, f.to_string());
                        }
                    });
                    ui.end_row();
                });
            });
            if busy {
                ui.label(RichText::new("Arrêtez le streaming et l'enregistrement pour changer la vidéo.").small().weak());
            }
            ui.separator();
            ui.heading("Stream");
            egui::Grid::new("stream").num_columns(2).show(ui, |ui| {
                ui.label("Serveur RTMP");
                ui.add(egui::TextEdit::singleline(&mut d.stream.server).hint_text("rtmp://live.restream.io/live").desired_width(320.0));
                ui.end_row();
                ui.label("Clé de stream");
                ui.add(egui::TextEdit::singleline(&mut d.stream.key).password(true).desired_width(320.0));
                ui.end_row();
                ui.label("Débit vidéo (kb/s)");
                ui.add(egui::DragValue::new(&mut d.stream.video_bitrate).range(500..=50000).speed(50));
                ui.end_row();
                ui.label("Débit audio (kb/s)");
                egui::ComboBox::from_id_salt("ab").selected_text(d.stream.audio_bitrate.to_string()).show_ui(ui, |ui| {
                    for b in [96, 128, 160, 192, 256, 320] {
                        ui.selectable_value(&mut d.stream.audio_bitrate, b, b.to_string());
                    }
                });
                ui.end_row();
                ui.label("Encodeur");
                egui::ComboBox::from_id_salt("enc").selected_text(output::encoder_label(&d.stream.encoder)).show_ui(ui, |ui| {
                    ui.selectable_value(&mut d.stream.encoder, "auto".to_string(), output::encoder_label("auto"));
                    for e in output::encoders() {
                        ui.selectable_value(&mut d.stream.encoder, e.clone(), output::encoder_label(e));
                    }
                });
                ui.end_row();
            });
            ui.separator();
            ui.heading("Enregistrement");
            egui::Grid::new("rec").num_columns(2).show(ui, |ui| {
                ui.label("Dossier");
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut d.record.folder).desired_width(260.0));
                    if ui.button("Parcourir…").clicked()
                        && let Some(p) = rfd::FileDialog::new().set_directory(&d.record.folder).pick_folder()
                    {
                        d.record.folder = p.to_string_lossy().into_owned();
                    }
                });
                ui.end_row();
                ui.label("Format");
                egui::ComboBox::from_id_salt("fmt").selected_text(d.record.format.to_uppercase()).show_ui(ui, |ui| {
                    ui.selectable_value(&mut d.record.format, "mkv".to_string(), "MKV (résiste aux coupures, recommandé)");
                    ui.selectable_value(&mut d.record.format, "mp4".to_string(), "MP4");
                });
                ui.end_row();
                ui.label("Débit vidéo (kb/s)");
                ui.add(egui::DragValue::new(&mut d.record.video_bitrate).range(2000..=150000).speed(100));
                ui.end_row();
            });
            ui.separator();
            ui.heading("Contrôle à distance (Stream Deck)");
            ui.label(RichText::new("Compatible obs-websocket 5 : plugin MyStream ou plugins OBS du Stream Deck, Touch Portal, Companion…").small().weak());
            egui::Grid::new("remote").num_columns(2).show(ui, |ui| {
                ui.label("Activer");
                ui.checkbox(&mut d.remote.enabled, "");
                ui.end_row();
                ui.label("Port");
                ui.add(egui::DragValue::new(&mut d.remote.port).range(1024..=65535));
                ui.end_row();
                ui.label("Mot de passe");
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut d.remote.password).desired_width(200.0));
                    if ui.button("Copier").clicked() {
                        ui.ctx().copy_text(d.remote.password.clone());
                    }
                });
                ui.end_row();
                ui.label("Réseau local");
                ui.checkbox(&mut d.remote.lan, "Accepter les autres appareils du réseau (téléphone, tablette)");
                ui.end_row();
            });
            ui.separator();
            ui.heading("Raccourcis clavier (fonctionnent même en jeu)");
            ui.label(RichText::new("Exemples : ctrl+shift+KeyS, alt+F9, ctrl+Digit1. Laissez vide pour aucun raccourci.").small().weak());
            egui::Grid::new("hotkeys").num_columns(2).show(ui, |ui| {
                for (action, label) in HOTKEY_ACTIONS {
                    ui.label(label);
                    let v = d.hotkeys.entry(action.to_string()).or_default();
                    ui.add(egui::TextEdit::singleline(v).desired_width(180.0));
                    ui.end_row();
                }
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
            let s = &self.project.settings;
            let video = (self.draft.width, self.draft.height, self.draft.fps) != (s.width, s.height, s.fps);
            // Le mixeur, la transition et le mode studio ont pu changer pendant que la fenêtre était ouverte.
            let (audio, transition, studio) = (s.audio.clone(), s.transition.clone(), s.studio);
            let remote_changed = {
                let (a, b) = (&self.draft.remote, &s.remote);
                (a.enabled, a.port, &a.password, a.lan) != (b.enabled, b.port, &b.password, b.lan)
            };
            let hotkeys_changed = self.draft.hotkeys != s.hotkeys;
            self.project.settings = self.draft.clone();
            self.project.settings.audio = audio;
            self.project.settings.transition = transition;
            self.project.settings.studio = studio;
            if video {
                self.runtimes.clear(); // les captures repartent à la nouvelle cadence
                self.shared.sizes.lock().clear();
                self.start_engine(ctx);
            }
            self.touch();
            self.notify("Paramètres enregistrés.", false);
            if remote_changed {
                self.apply_remote();
            }
            if hotkeys_changed {
                let errors = self.apply_hotkeys();
                if !errors.is_empty() {
                    self.notify(format!("Raccourcis refusés : {}", errors.join(", ")), true);
                }
            }
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
        self.handle_remote();
        self.handle_hotkeys();
        self.update_items();

        egui::Panel::bottom("bas").show(ui, |ui| self.bottom(ui));
        egui::Panel::left("scenes").default_size(210.0).show(ui, |ui| self.scenes_panel(ui));
        egui::Panel::right("sources").default_size(300.0).show(ui, |ui| self.sources_panel(ui));
        egui::CentralPanel::default().show(ui, |ui| self.preview(ui));
        self.settings_window(&ctx);

        if let Some((start, secs)) = self.autotest {
            let t = start.elapsed();
            if t > Duration::from_secs(2) && !self.live(Kind::Record) && t < Duration::from_secs(3) {
                self.toggle_output(Kind::Record);
            }
            if t > Duration::from_secs(2 + secs) {
                self.autotest = None;
                let st = self.shared.stats.lock().clone();
                log::info!("Test : {:.1} i/s, rendu {:.2} ms, {} manquée(s), {} perdue(s)", st.fps, st.render_ms, st.lagged, st.dropped);
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
        if self.dirty.is_some_and(|t| t.elapsed() > Duration::from_secs(1)) {
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
