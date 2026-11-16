//! Interface (egui) : scènes, sources, aperçu, mixeur, commandes et paramètres.

mod control;

use crate::audio::{DESKTOP, MIC, Mixer};
use crate::capture::{self, Runtime};
use crate::compositor::{Compositor, DrawItem};
use crate::engine::{Engine, Shared, TransitionRun, key};
use crate::model::{Filters, HOTKEY_ACTIONS, Project, Settings, SourceKind, TransitionKind};
use crate::output::{self, Kind, Output};
use crate::remote::Remote;
use crate::theme::{self, Part};
use egui::{Color32, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Vec2, pos2, vec2};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

const RESOLUTIONS: [(u32, u32); 5] = [(1920, 1080), (2560, 1440), (1600, 900), (1280, 720), (3840, 2160)];
const LIVE: Color32 = theme::DANGER;
const WARN: Color32 = theme::WARN;
const OK: Color32 = theme::SUCCESS;
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
    preview_hidden: bool,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::apply(&cc.egui_ctx);
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
            preview_hidden: false,
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

    /* Panneaux (même disposition que la version Electron) ------------------- */

    fn import_obs(&mut self) {
        let mut dlg = rfd::FileDialog::new().add_filter("Collection OBS", &["json"]);
        if let Some(dir) = crate::obs_import::obs_folder() {
            dlg = dlg.set_directory(dir);
        }
        let Some(path) = dlg.pick_file() else { return };
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

    fn open_settings(&mut self) {
        self.draft = self.project.settings.clone();
        self.settings_open = true;
    }

    fn toggle_studio(&mut self) {
        self.project.settings.studio = !self.project.settings.studio;
        self.project.preview = self.project.current;
        self.touch();
    }

    /// Action sur l'élément sélectionné (menu Édition, barre sous l'aperçu).
    fn item_action(&mut self, what: &str) {
        let scene_id = self.project.edit_scene();
        let Some(sel) = self.selected else { return };
        if what == "remove" {
            self.project.remove_item(scene_id, sel);
            self.selected = None;
            self.touch();
            return;
        }
        let sizes = self.shared.sizes.lock().clone();
        let (cw, ch) = (self.canvas.0 as f32, self.canvas.1 as f32);
        let crops: HashMap<u64, [u32; 4]> = self.project.sources.iter().map(|s| (s.id, s.filters.crop)).collect();
        let Some(scene) = self.project.scene_mut(scene_id) else { return };
        let Some(item) = scene.items.iter_mut().find(|i| i.id == sel) else { return };
        let Some(&(w, h)) = sizes.get(&item.source) else { return };
        let c = crops.get(&item.source).copied().unwrap_or([0; 4]);
        let (w, h) = (w.saturating_sub(c[0] + c[2]).max(1) as f32, h.saturating_sub(c[1] + c[3]).max(1) as f32);
        match what {
            "fit" => {
                let s = (cw / w).min(ch / h);
                item.scale = Some(s);
                item.x = (cw - w * s) / 2.0;
                item.y = (ch - h * s) / 2.0;
            }
            "original" => item.scale = Some(1.0),
            "center" => {
                let s = item.scale.unwrap_or(1.0);
                item.x = (cw - w * s) / 2.0;
                item.y = (ch - h * s) / 2.0;
            }
            _ => {}
        }
        self.touch();
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_centered(|ui| {
            ui.add_space(8.0);
            let (r, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
            ui.painter().circle_filled(r.center(), 5.5, theme::DANGER);
            ui.label(theme::title("MyStream"));
            ui.label(RichText::new(version_label()).color(theme::MUTED).size(12.0));
            ui.add_space(12.0);
            egui::MenuBar::new().ui(ui, |ui| self.menus(ui));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(10.0);
                ui.label(theme::title(self.project.scene(self.project.current).map(|s| s.name.clone()).unwrap_or_default()));
                ui.label(RichText::new("À l'antenne :").color(theme::MUTED));
            });
        });
    }

    fn menus(&mut self, ui: &mut egui::Ui) {
        {
            ui.menu_button("Fichier", |ui| {
                if ui.button("Importer depuis OBS…").clicked() {
                    ui.close();
                    self.import_obs();
                }
                if ui.button("Ouvrir le dossier des enregistrements").clicked() {
                    ui.close();
                    open_path(&self.project.settings.record.folder);
                }
                if ui.button("Paramètres…").clicked() {
                    ui.close();
                    self.open_settings();
                }
                ui.separator();
                if ui.button("Quitter").clicked() {
                    ui.close();
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button("Édition", |ui| {
                let has = self.selected.is_some();
                for (what, label) in
                    [("fit", "Ajuster à l'écran"), ("original", "Taille d'origine"), ("center", "Centrer à l'écran"), ("remove", "Retirer de la scène")]
                {
                    if ui.add_enabled(has, egui::Button::new(label)).clicked() {
                        ui.close();
                        self.item_action(what);
                    }
                }
            });
            ui.menu_button("Affichage", |ui| {
                if ui.button(if self.project.settings.studio { "✔ Mode studio" } else { "Mode studio" }).clicked() {
                    ui.close();
                    self.toggle_studio();
                }
                if ui.button(if self.preview_hidden { "Afficher l'aperçu" } else { "Masquer l'aperçu" }).clicked() {
                    ui.close();
                    self.preview_hidden = !self.preview_hidden;
                }
            });
            ui.menu_button("Aide", |ui| {
                ui.label(RichText::new(format!("MyStream natif — {}", version_label())).color(theme::MUTED));
                if ui.button("Page du projet").clicked() {
                    ui.close();
                    open_path("https://github.com/TheDEMON78/mystream/tree/rust");
                }
            });
        }
    }

    fn docks(&mut self, ui: &mut egui::Ui) {
        let h = ui.available_height().max(0.0);
        let gap = 6.0;
        let fixed = [230.0, 295.0, 205.0, 228.0];
        let mixer_w = (ui.available_width() - fixed.iter().sum::<f32>() - gap * 4.0).max(220.0);
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (w, which) in [(fixed[0], 0), (fixed[1], 1), (mixer_w, 2), (fixed[2], 3), (fixed[3], 4)] {
                ui.allocate_ui_with_layout(vec2(w, h), egui::Layout::top_down(egui::Align::Min), |ui| match which {
                    0 => self.scenes_dock(ui),
                    1 => self.sources_dock(ui),
                    2 => self.mixer_dock(ui),
                    3 => self.transitions_dock(ui),
                    _ => self.commands_dock(ui),
                });
            }
        });
    }

    fn scenes_dock(&mut self, ui: &mut egui::Ui) {
        let studio = self.project.settings.studio;
        theme::dock(ui, "Scènes", true, |ui, part| match part {
            Part::Body => {
                let mut pick = None;
                let ids: Vec<(u64, String)> = self.project.scenes.iter().map(|s| (s.id, s.name.clone())).collect();
                for (id, name) in ids {
                    if let Some((rid, text)) = &mut self.renaming
                        && *rid == id
                    {
                        let r = ui.add(egui::TextEdit::singleline(text).desired_width(f32::INFINITY));
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
                        text = text.color(theme::DANGER); // à l'antenne
                    }
                    let r = ui.add(egui::Button::selectable(selected, text).min_size(vec2(ui.available_width(), 26.0)));
                    if r.clicked() {
                        pick = Some(id);
                    }
                    if r.double_clicked() {
                        if studio {
                            self.project.current = id; // double-clic en mode studio : à l'antenne
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
            }
            Part::Footer => {
                if theme::tool(ui, "+", "Ajouter une scène") {
                    let id = self.project.add_scene();
                    if studio {
                        self.project.preview = id;
                    } else {
                        self.project.current = id;
                    }
                    self.touch();
                }
                if self.project.scenes.len() > 1 && theme::tool(ui, "−", "Supprimer la scène") {
                    let id = self.project.edit_scene();
                    self.project.remove_scene(id);
                    self.touch();
                }
                if theme::tool(ui, "✏", "Renommer la scène (ou double-clic)") {
                    let id = self.project.edit_scene();
                    let name = self.project.scene(id).map(|s| s.name.clone()).unwrap_or_default();
                    self.renaming = Some((id, name));
                }
                for (icon, tip, delta) in [("⏶", "Monter", -1i32), ("⏷", "Descendre", 1)] {
                    if theme::tool(ui, icon, tip) {
                        let id = self.project.edit_scene();
                        let list = &mut self.project.scenes;
                        if let Some(i) = list.iter().position(|s| s.id == id) {
                            let j = i as i32 + delta;
                            if j >= 0 && (j as usize) < list.len() {
                                list.swap(i, j as usize);
                                self.touch();
                            }
                        }
                    }
                }
            }
        });
    }

    fn sources_dock(&mut self, ui: &mut egui::Ui) {
        let scene_id = self.project.edit_scene();
        theme::dock(ui, "Sources", true, |ui, part| match part {
            Part::Body => {
                let items: Vec<(u64, u64, bool)> =
                    self.project.scene(scene_id).map(|s| s.items.iter().map(|i| (i.id, i.source, i.visible)).collect()).unwrap_or_default();
                if items.is_empty() {
                    ui.label(RichText::new("Cliquez sur + pour ajouter une source.").italics().color(theme::MUTED));
                }
                let mut toggle = None;
                for (item, source, visible) in &items {
                    let Some(src) = self.project.source(*source) else { continue };
                    let rt = self.runtimes.get(source);
                    let status = rt.and_then(|r| r.error());
                    let info = rt.and_then(|r| r.info());
                    let shared = self.project.scenes.iter().filter(|s| s.items.iter().any(|i| i.source == *source)).count() > 1;
                    let mut text = format!("{}  {}", src.kind.glyph(), src.name);
                    if shared {
                        text.push_str("  🔗");
                    }
                    let mut label = RichText::new(text);
                    if status.is_some() {
                        label = label.color(theme::WARN);
                    } else if !visible {
                        label = label.color(theme::MUTED);
                    }
                    ui.horizontal(|ui| {
                        let w = (ui.available_width() - 34.0).max(0.0);
                        let sel = self.selected == Some(*item);
                        let r = ui
                            .allocate_ui_with_layout(vec2(w, 26.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                                ui.add(egui::Button::selectable(sel, label).min_size(vec2(w, 26.0)))
                            })
                            .inner;
                        let hover = [status.clone(), info.clone(), shared.then(|| "Source partagée avec d'autres scènes".to_string())]
                            .into_iter()
                            .flatten()
                            .collect::<Vec<_>>()
                            .join("\n");
                        let r = if hover.is_empty() { r } else { r.on_hover_text(hover) };
                        if r.clicked() {
                            self.selected = Some(*item);
                        }
                        if theme::tool(ui, if *visible { "👁" } else { "⊘" }, if *visible { "Masquer" } else { "Afficher" }) {
                            toggle = Some(*item);
                        }
                    });
                    if let Some(i) = info {
                        ui.label(RichText::new(format!("      {i}")).small().color(theme::MUTED));
                    }
                }
                if let Some(id) = toggle
                    && let Some(scene) = self.project.scene_mut(scene_id)
                    && let Some(it) = scene.items.iter_mut().find(|i| i.id == id)
                {
                    it.visible = !it.visible;
                    self.touch();
                }
            }
            Part::Footer => {
                ui.menu_button(RichText::new("+").size(14.0), |ui| self.add_menu(ui)).response.on_hover_text("Ajouter une source");
                if self.selected.is_some() && theme::tool(ui, "−", "Retirer la source de la scène") {
                    self.item_action("remove");
                }
                for (icon, tip, delta) in [("⏶", "Monter", -1i32), ("⏷", "Descendre", 1)] {
                    if theme::tool(ui, icon, tip)
                        && let Some(sel) = self.selected
                        && let Some(scene) = self.project.scene_mut(scene_id)
                        && let Some(i) = scene.items.iter().position(|it| it.id == sel)
                    {
                        let j = i as i32 + delta;
                        if j >= 0 && (j as usize) < scene.items.len() {
                            scene.items.swap(i, j as usize);
                            self.touch();
                        }
                    }
                }
            }
        });
    }

    fn mixer_row(ui: &mut egui::Ui, name: &str, level: f32, vol: &mut f32, muted: &mut bool, err: Option<String>) -> bool {
        let mut changed = false;
        ui.horizontal(|ui| {
            let mut t = RichText::new(name);
            if err.is_some() {
                t = t.color(theme::WARN);
            }
            let r = ui
                .allocate_ui_with_layout(vec2(120.0, 22.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.set_width(120.0);
                    ui.add(egui::Label::new(t).truncate())
                })
                .inner;
            if let Some(e) = err {
                r.on_hover_text(e);
            }
            if theme::tool(ui, if *muted { "🔇" } else { "🔈" }, "Couper ou rétablir le son") {
                *muted = !*muted;
                changed = true;
            }
            ui.spacing_mut().slider_width = 130.0;
            changed |= ui.add(egui::Slider::new(vol, 0.0..=2.0).show_value(false)).changed();
            ui.add_sized([44.0, 22.0], egui::Label::new(RichText::new(format!("{:.0} %", *vol * 100.0)).color(theme::MUTED)));
            let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width().max(40.0) - 6.0, 8.0), Sense::hover());
            let p = ui.painter();
            p.rect_filled(bar, 3.0, theme::PANEL2);
            let lvl = if *muted { 0.0 } else { level.min(1.0) };
            let color = if lvl > 0.9 {
                theme::DANGER
            } else if lvl > 0.6 {
                theme::WARN
            } else {
                theme::SUCCESS
            };
            p.rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * lvl, bar.height())), 3.0, color);
        });
        changed
    }

    fn mixer_dock(&mut self, ui: &mut egui::Ui) {
        theme::dock(ui, "Mixeur audio", false, |ui, _| {
            let mut changed = false;
            for (id, name) in [(MIC, "Micro"), (DESKTOP, "Son du bureau")] {
                let t = self.mixer.get(id);
                let level = t.as_ref().map(|t| *t.level.lock()).unwrap_or(0.0);
                let err = t.as_ref().and_then(|t| t.error.lock().clone());
                let a = &mut self.project.settings.audio;
                let (vol, muted) = if id == MIC { (&mut a.mic_volume, &mut a.mic_muted) } else { (&mut a.desktop_volume, &mut a.desktop_muted) };
                changed |= Self::mixer_row(ui, name, level, vol, muted, err);
            }
            for s in self.project.sources.iter_mut().filter(|s| s.kind.has_audio()) {
                let t = self.mixer.get(s.id);
                let level = t.as_ref().map(|t| *t.level.lock()).unwrap_or(0.0);
                let err = t.as_ref().and_then(|t| t.error.lock().clone());
                changed |= Self::mixer_row(ui, &s.name, level, &mut s.volume, &mut s.muted, err);
            }
            if changed {
                self.touch();
            }
        });
    }

    fn transitions_dock(&mut self, ui: &mut egui::Ui) {
        theme::dock(ui, "Transitions", false, |ui, _| {
            let t = &mut self.project.settings.transition;
            let mut changed = false;
            ui.label(RichText::new("Type").color(theme::MUTED));
            egui::ComboBox::from_id_salt("transition").width((ui.available_width() - 8.0).max(0.0)).selected_text(t.kind.label()).show_ui(ui, |ui| {
                for k in [TransitionKind::Cut, TransitionKind::Fade, TransitionKind::Slide] {
                    changed |= ui.selectable_value(&mut t.kind, k, k.label()).changed();
                }
            });
            ui.add_space(8.0);
            ui.label(RichText::new("Durée (ms)").color(theme::MUTED));
            ui.add_enabled_ui(t.kind != TransitionKind::Cut, |ui| {
                changed |=
                    ui.add_sized([(ui.available_width() - 8.0).max(0.0), 28.0], egui::DragValue::new(&mut t.duration_ms).range(50..=3000).speed(10)).changed();
            });
            if changed {
                self.touch();
            }
        });
    }

    fn commands_dock(&mut self, ui: &mut egui::Ui) {
        theme::dock(ui, "Commandes", false, |ui, _| {
            ui.spacing_mut().item_spacing.y = 6.0;
            let streaming = self.live(Kind::Stream);
            let recording = self.live(Kind::Record);
            if theme::command(ui, if streaming { "Arrêter le streaming" } else { "Démarrer le streaming" }, streaming.then_some(theme::DANGER)) {
                self.toggle_output(Kind::Stream);
            }
            if theme::command(ui, if recording { "Arrêter l'enregistrement" } else { "Démarrer l'enregistrement" }, recording.then_some(theme::DANGER)) {
                self.toggle_output(Kind::Record);
            }
            if theme::command(ui, "Mode studio", self.project.settings.studio.then_some(theme::ACCENT2)) {
                self.toggle_studio();
            }
            if theme::command(ui, "Dossier des enregistrements", None) {
                open_path(&self.project.settings.record.folder);
            }
            if theme::command(ui, "Paramètres", None) {
                self.open_settings();
            }
            if theme::command(ui, "Quitter", None) {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
        });
    }

    fn properties_dock(&mut self, ui: &mut egui::Ui) {
        theme::dock(ui, "Propriétés et filtres", false, |ui, _| {
            if self.selected.is_none() {
                ui.label(
                    RichText::new("Sélectionnez une source (dans la liste ou en cliquant dans l'aperçu) pour régler ses propriétés et ses filtres.")
                        .italics()
                        .color(theme::MUTED),
                );
            }
            self.properties(ui);
        });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let st = self.shared.stats.lock().clone();
        let fps = self.project.settings.fps;
        ui.horizontal_centered(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
            let healthy = st.fps >= fps as f32 * 0.95;
            ui.painter().circle_filled(r.center(), 4.5, if healthy { theme::SUCCESS } else { theme::WARN });
            let mut text = format!(
                "{:.1} / {} i/s · rendu {:.1} ms · {} image{} manquée{}",
                st.fps,
                fps,
                st.render_ms,
                st.lagged,
                if st.lagged > 1 { "s" } else { "" },
                if st.lagged > 1 { "s" } else { "" }
            );
            if st.dropped > 0 {
                text.push_str(&format!(" · {} perdue{} à l'encodage", st.dropped, if st.dropped > 1 { "s" } else { "" }));
            }
            ui.label(RichText::new(text).size(12.5).color(theme::MUTED));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let outputs = self.shared.outputs.lock();
                let time = |k: Kind| {
                    let t = outputs.get(&key(k)).map(|o| o.started.elapsed().as_secs()).unwrap_or(0);
                    format!("{:02}:{:02}:{:02}", t / 3600, t / 60 % 60, t % 60)
                };
                theme::pill(ui, &format!("REC {}", time(Kind::Record)), outputs.contains_key(&key(Kind::Record)));
                theme::pill(ui, &format!("LIVE {}", time(Kind::Stream)), outputs.contains_key(&key(Kind::Stream)));
                for out in outputs.values() {
                    let s = out.stats.lock().clone();
                    ui.label(RichText::new(format!("{:.0} kb/s · {}", s.bitrate_kbps, output::encoder_label(&out.encoder))).size(12.5).color(theme::MUTED));
                }
                drop(outputs);
                if let Some((msg, err, at)) = &self.notice {
                    if at.elapsed() < Duration::from_secs(12) {
                        ui.label(RichText::new(msg).size(12.5).color(if *err { theme::DANGER } else { theme::SUCCESS }));
                    } else {
                        self.notice = None;
                    }
                }
            });
        });
    }

    /// Aperçu, puis barre d'outils dessous (comme la version Electron).
    fn preview(&mut self, ui: &mut egui::Ui) {
        let Some((program, studio)) = self.preview else { return };
        self.update_banner(ui);
        let avail = (ui.available_size() - vec2(0.0, 42.0)).max(Vec2::ZERO);
        if self.preview_hidden {
            let (r, _) = ui.allocate_exact_size(avail, Sense::hover());
            ui.painter().rect_filled(r, 0.0, Color32::from_rgb(0x0c, 0x0d, 0x10));
            ui.painter().text(
                r.center(),
                egui::Align2::CENTER_CENTER,
                "Aperçu masqué (allège la carte graphique)",
                egui::FontId::proportional(14.0),
                theme::MUTED,
            );
        } else if !self.project.settings.studio {
            self.canvas_view(ui, program, self.project.current, true, avail);
        } else {
            // Mode studio : aperçu (modifiable) à gauche, antenne à droite.
            let w = ((avail.x - 150.0) / 2.0).max(0.0);
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(theme::title("Aperçu"));
                    let preview = self.project.preview;
                    self.canvas_view(ui, studio, preview, true, vec2(w, (avail.y - 24.0).max(0.0)));
                });
                ui.vertical(|ui| {
                    ui.add_space(avail.y / 2.0 - 30.0);
                    let b = egui::Button::new(theme::title("Transition ➡")).fill(theme::ACCENT).min_size(vec2(130.0, 36.0));
                    if ui.add(b).on_hover_text("Passer la scène de l'aperçu à l'antenne").clicked() {
                        self.transition();
                    }
                });
                ui.vertical(|ui| {
                    ui.label(theme::title("À l'antenne").color(theme::DANGER));
                    let current = self.project.current;
                    self.canvas_view(ui, program, current, false, vec2(w, (avail.y - 24.0).max(0.0)));
                });
            });
        }
        // Barre sous l'aperçu.
        let scene_id = self.project.edit_scene();
        let name = self
            .selected
            .and_then(|sel| self.project.scene(scene_id)?.items.iter().find(|i| i.id == sel).map(|i| i.source))
            .and_then(|s| self.project.source(s))
            .map(|s| s.name.clone());
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            ui.label(RichText::new(name.clone().unwrap_or_else(|| "Aucune source sélectionnée".into())).color(theme::MUTED));
            ui.add_space(8.0);
            ui.add_enabled_ui(name.is_some(), |ui| {
                if ui.button("Ajuster à l'écran").clicked() {
                    self.item_action("fit");
                }
                if ui.button("Taille d'origine").clicked() {
                    self.item_action("original");
                }
                if ui.button("Centrer").clicked() {
                    self.item_action("center");
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(8.0);
                if ui.button(if self.preview_hidden { "Afficher l'aperçu" } else { "Masquer l'aperçu" }).clicked() {
                    self.preview_hidden = !self.preview_hidden;
                }
            });
        });
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

        let bar = |fill| egui::Frame::new().fill(fill);
        egui::Panel::top("menu").exact_size(36.0).frame(bar(theme::BG).stroke(egui::Stroke::new(1.0, theme::BORDER))).show(ui, |ui| self.top_bar(ui));
        egui::Panel::bottom("statut").exact_size(28.0).frame(bar(theme::BG).inner_margin(egui::Margin::symmetric(10, 0))).show(ui, |ui| self.status_bar(ui));
        egui::Panel::bottom("docks")
            .exact_size(290.0)
            .frame(bar(theme::BG).inner_margin(egui::Margin { left: 4, right: 4, top: 4, bottom: 2 }))
            .show(ui, |ui| self.docks(ui));
        egui::Panel::right("droite").default_size(340.0).min_size(260.0).frame(bar(theme::BG).inner_margin(4)).show(ui, |ui| self.properties_dock(ui));
        egui::CentralPanel::default().frame(bar(theme::BG).inner_margin(egui::Margin::symmetric(4, 4))).show(ui, |ui| self.preview(ui));
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

/// Version affichée (numéro de la préversion GitHub).
fn version_label() -> String {
    match crate::update::build() {
        0 => "natif (développement)".into(),
        n => format!("natif 0.1.{n}"),
    }
}

/// Ouvre un dossier ou une adresse avec l'application par défaut.
fn open_path(target: &str) {
    let cmd = if cfg!(windows) { "explorer" } else { "xdg-open" };
    let _ = std::process::Command::new(cmd).arg(target).spawn();
}
