//! Interface (egui) : scènes, sources, aperçu, mixeur, commandes et paramètres.

mod control;
mod edit;
mod extras;
mod layout;
mod live;
mod session;
mod settings;

use crate::audio::{DESKTOP, MIC, Mixer};
use crate::capture::{self, Runtime};
use crate::compositor::{Compositor, DrawItem};
use crate::engine::{Engine, Shared, TransitionRun, key};
use crate::model::{Filters, Project, Settings, SourceKind, TransitionKind};
use crate::output::{self, Kind};
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
    settings_tab: settings::Tab,
    /// Raccourci en cours de saisie (fenêtre Paramètres).
    capturing_hotkey: Option<String>,
    /// Mot de passe de la télécommande affiché en clair.
    show_password: bool,
    /// Vérification des mises à jour : dernière, et si elle a été demandée (Aide).
    update_checked: Option<Instant>,
    update_manual: bool,
    draft: Settings,
    notice: Option<(String, bool, Instant)>,
    dirty: Option<Instant>,
    renaming: Option<(u64, String)>,
    /// Scène à l'antenne lors de la dernière image (pour déclencher les transitions).
    shown: u64,
    /// Scène dont la transition est en cours de réglage (menu des scènes).
    scene_transition: Option<u64>,
    /// Scène quittée par la dernière transition.
    leaving: u64,
    /// Nom saisi pour enregistrer la disposition des docks.
    saving_layout: Option<String>,
    /// Position non calée de l'élément en cours de déplacement.
    drag_raw: Option<(f32, f32)>,
    /// Confirmation d'arrêt du live demandée.
    confirm_stop: bool,
    /// Assistant de configuration ouvert.
    wizard: Option<extras::Wizard>,
    /// Icône dans la zone de notification (fenêtre réduite).
    tray: Option<extras::Tray>,
    /// Dernière fois que chaque source était affichée (arrêt des sources cachées).
    last_needed: HashMap<u64, Instant>,
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
    chat: crate::chat::Chat,
    pending: Vec<live::Pending>,
    live: live::LiveDock,
    session: Option<session::Session>,
    recap: Option<session::Summary>,
    history_open: bool,
    props_window: bool,
    /// Piste dont la fenêtre des filtres audio est ouverte.
    audio_filters_open: Option<u64>,
    undo: Vec<String>,
    redo: Vec<String>,
    undo_base: String,
    last_backup: Option<Instant>,
    transform_open: bool,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::apply(&cc.egui_ctx);
        egui_extras::install_image_loaders(&cc.egui_ctx); // images des emotes du chat
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
            scene_transition: None,
            leaving: 0,
            saving_layout: None,
            drag_raw: None,
            confirm_stop: false,
            wizard: None,
            tray: None,
            last_needed: HashMap::new(),
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
            settings_tab: Default::default(),
            capturing_hotkey: None,
            show_password: false,
            update_checked: None,
            update_manual: false,
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
            chat: Default::default(),
            pending: Vec::new(),
            live: Default::default(),
            session: None,
            recap: None,
            history_open: false,
            props_window: false,
            audio_filters_open: None,
            undo: Vec::new(),
            redo: Vec::new(),
            undo_base: String::new(),
            last_backup: None,
            transform_open: false,
        };
        app.start_engine(&cc.egui_ctx);
        crate::update::fix_shortcuts();
        app.chat.restart(&app.project.settings.chat, &app.project.settings.restream);
        app.apply_remote();
        // Tests automatiques : MYSTREAM_TEST_SETTINGS=<onglet> ouvre les paramètres.
        if let Some(n) = std::env::var("MYSTREAM_TEST_SETTINGS").ok().and_then(|v| v.parse::<usize>().ok()) {
            app.open_settings_tab(settings::Tab::from_index(n));
        }
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
    /// Sources affichées : scène à l'antenne, aperçu du mode studio et scène quittée
    /// pendant une transition (éléments visibles seulement).
    fn needed_sources(&self) -> std::collections::HashSet<u64> {
        let mut scenes = vec![self.project.current, self.shown];
        if self.project.settings.studio {
            scenes.push(self.project.preview);
        }
        self.project
            .scenes
            .iter()
            .filter(|sc| scenes.contains(&sc.id))
            .flat_map(|sc| sc.items.iter().filter(|i| i.visible).map(|i| i.source))
            .collect()
    }

    /// Sources de la scène à l'antenne (et de la scène quittée pendant une transition).
    fn on_air_sources(&self) -> std::collections::HashSet<u64> {
        let transition = self.shared.transition.lock().is_some();
        self.project
            .scenes
            .iter()
            .filter(|sc| sc.id == self.project.current || (transition && sc.id == self.leaving))
            .flat_map(|sc| sc.items.iter().filter(|i| i.visible).map(|i| i.source))
            .collect()
    }

    fn sync_runtimes(&mut self) {
        let fps = self.project.settings.fps;
        let chat = self.chat.handle();
        self.runtimes.retain(|id, rt| self.project.sources.iter().any(|s| s.id == *id && s.kind == *rt.kind()));
        // Comme la version Electron : une source coûteuse (caméra, capture, média,
        // navigateur…) qui n'est plus affichée est arrêtée après quelques secondes
        // (la caméra s'éteint), puis relancée quand elle revient à l'écran.
        let needed = self.needed_sources();
        let now = Instant::now();
        let grace = Duration::from_secs(3) + Duration::from_millis(self.project.settings.transition.duration_ms as u64);
        for id in &needed {
            self.last_needed.insert(*id, now);
        }
        let idle: Vec<u64> = self
            .project
            .sources
            .iter()
            .filter(|s| s.kind.costly() && !needed.contains(&s.id))
            .filter(|s| self.last_needed.get(&s.id).is_none_or(|t| now.duration_since(*t) > grace))
            .map(|s| s.id)
            .collect();
        for id in &idle {
            if self.runtimes.remove(id).is_some() {
                log::info!("Source {id} arrêtée : elle n'est plus affichée");
            }
        }
        for s in &self.project.sources {
            if idle.contains(&s.id) {
                continue;
            }
            if !self.runtimes.contains_key(&s.id) {
                let track = s.kind.has_audio().then(|| self.mixer.track(s.id));
                self.runtimes.insert(s.id, Runtime::start(&s.kind, fps, track, &chat));
            }
            if s.kind.has_audio() {
                let t = self.mixer.track(s.id);
                *t.gain.lock() = (s.volume, s.muted);
                *t.filters.lock() = s.audio_filters.clone();
                t.monitor.store(s.monitor, std::sync::atomic::Ordering::Relaxed);
            }
        }
        // Seules les sources de la scène à l'antenne (et de celle qu'on quitte pendant
        // une transition) s'entendent dans le live, comme dans OBS.
        let on_air = self.on_air_sources();
        for s in self.project.sources.iter().filter(|s| s.kind.has_audio()) {
            if let Some(t) = self.mixer.get(s.id) {
                t.off_air.store(!on_air.contains(&s.id), std::sync::atomic::Ordering::Relaxed);
            }
        }
        let sources = &self.project.sources;
        self.mixer.retain(|id| id == crate::engine::STINGER || sources.iter().any(|s| s.id == id && s.kind.has_audio()));
        let a = &self.project.settings.audio;
        if let Some(t) = self.mixer.get(MIC) {
            *t.gain.lock() = (a.mic_volume, a.mic_muted);
            *t.filters.lock() = a.mic_filters.clone();
            t.monitor.store(a.mic_monitor, std::sync::atomic::Ordering::Relaxed);
        }
        if let Some(t) = self.mixer.get(DESKTOP) {
            *t.gain.lock() = (a.desktop_volume, a.desktop_muted);
            t.off_air.store(!a.desktop_audio, std::sync::atomic::Ordering::Relaxed);
        }
        self.mixer.set_mic(&a.mic_device);
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
                let sy = item.scale_y.unwrap_or(scale);
                if item.visible {
                    list.push(DrawItem { source: item.source, x: item.x, y: item.y, w: vw * scale, h: vh * sy, filters: f, alpha: 1.0 });
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
            // Transition propre à la scène, sinon celle du dock Transitions.
            let t = &self.project.scene(self.project.current).and_then(|s| s.transition.clone()).unwrap_or_else(|| self.project.settings.transition.clone());
            let stinger = t.kind == TransitionKind::Stinger && !t.stinger_file.trim().is_empty();
            if (t.kind != TransitionKind::Cut && t.kind != TransitionKind::Stinger && t.duration_ms > 0) || stinger {
                let from = self.shared.items.lock().clone();
                // Stinger : la vidéo est lue une fois, son son passe dans le mixage du live.
                if stinger {
                    *self.mixer.track(crate::engine::STINGER).gain.lock() = (1.0, !t.stinger_audio);
                }
                let stinger = stinger.then(|| crate::engine::Stinger {
                    runtime: Runtime::start(
                        &SourceKind::Media { path: t.stinger_file.clone(), looping: false },
                        self.project.settings.fps,
                        Some(self.mixer.track(crate::engine::STINGER)),
                        &self.chat.handle(),
                    ),
                    point: Duration::from_millis(t.stinger_point_ms as u64),
                    first: None,
                });
                *self.shared.transition.lock() =
                    Some(TransitionRun {
                        from,
                        kind: t.kind,
                        start: Instant::now(),
                        duration: Duration::from_millis(t.duration_ms as u64),
                        stinger,
                        direction: t.direction.clone(),
                    });
            }
            self.leaving = self.shown;
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
            if kind == Kind::Stream {
                self.session_ended();
            }
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
            if kind == Kind::Stream {
                self.session_ended();
            }
            return;
        }
        self.start_output(kind);
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
        self.open_settings_tab(settings::Tab::General);
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
        if what == "lock" {
            if let Some(it) = self.project.scene_mut(scene_id).and_then(|s| s.items.iter_mut().find(|i| i.id == sel)) {
                it.locked = !it.locked;
            }
            self.touch();
            return;
        }
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
                (item.scale, item.scale_y) = (Some(s), None);
                item.x = (cw - w * s) / 2.0;
                item.y = (ch - h * s) / 2.0;
            }
            // Étirer : remplit tout le canevas, quitte à déformer l'image.
            "stretch" => {
                (item.scale, item.scale_y) = (Some(cw / w), Some(ch / h));
                (item.x, item.y) = (0.0, 0.0);
            }
            "original" => (item.scale, item.scale_y) = (Some(1.0), None),
            "center" => {
                let (sx, sy) = item.scales().unwrap_or((1.0, 1.0));
                item.x = (cw - w * sx) / 2.0;
                item.y = (ch - h * sy) / 2.0;
            }
            _ => {}
        }
        self.touch();
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_centered(|ui| {
            ui.add_space(8.0);
            ui.add(egui::Image::from_bytes("bytes://logo.png", include_bytes!("../assets/icon.png").as_slice()).fit_to_exact_size(vec2(20.0, 20.0)));
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
                if ui.button("Exporter la collection de scènes…").clicked() {
                    ui.close();
                    self.export_collection();
                }
                if ui.button("Importer une collection de scènes…").clicked() {
                    ui.close();
                    self.import_collection(false);
                }
                if ui.button("Importer une collection OBS…").clicked() {
                    ui.close();
                    self.import_obs();
                }
                if ui.button("Restaurer une sauvegarde automatique…").clicked() {
                    ui.close();
                    self.import_collection(true);
                }
                ui.separator();
                if ui.button("Ouvrir le dossier des enregistrements").clicked() {
                    ui.close();
                    open_path(&self.project.settings.record.folder);
                }
                if ui.button("Historique des lives…").clicked() {
                    ui.close();
                    self.history_open = true;
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
                let (u, r) = self.can_undo();
                if ui.add_enabled(u, egui::Button::new("Annuler").shortcut_text("Ctrl+Z")).clicked() {
                    ui.close();
                    self.undo();
                }
                if ui.add_enabled(r, egui::Button::new("Rétablir").shortcut_text("Ctrl+Y")).clicked() {
                    ui.close();
                    self.redo();
                }
                ui.separator();
                let has = self.selected.is_some();
                if ui.add_enabled(has, egui::Button::new("Modifier la transformation…").shortcut_text("Ctrl+E")).clicked() {
                    ui.close();
                    self.transform_open = true;
                }
                if ui.add_enabled(has, egui::Button::new("Verrouiller / déverrouiller")).clicked() {
                    ui.close();
                    self.item_action("lock");
                }
                if ui.add_enabled(has, egui::Button::new("Propriétés et filtres…")).clicked() {
                    ui.close();
                    self.props_window = true;
                }
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
                let bar = self.project.settings.ui.live_bar;
                if ui.button(format!("{}  Barre « En direct » pendant le live", if bar { "✔" } else { "   " })).clicked() {
                    self.project.settings.ui.live_bar = !bar;
                    self.touch();
                    ui.close();
                }
                ui.separator();
                ui.menu_button("Docks", |ui| self.docks_menu(ui));
            });
            ui.menu_button("Aide", |ui| {
                ui.label(RichText::new(format!("MyStream natif — {}", version_label())).color(theme::MUTED));
                if ui.button("Assistant de configuration…").clicked() {
                    ui.close();
                    self.open_wizard();
                }
                if ui.button("Rechercher des mises à jour").clicked() {
                    ui.close();
                    self.check_updates(true);
                }
                if ui.button("Ouvrir le dossier des journaux").clicked() {
                    ui.close();
                    open_path(&extras::logs_dir().to_string_lossy());
                }
                if ui.button("Copier le diagnostic de performance").clicked() {
                    ui.close();
                    let text = self.perf_diagnostic();
                    ui.ctx().copy_text(text);
                    self.notify("Diagnostic copié : collez-le dans votre message.", false);
                }
                if ui.button("Notes de version").clicked() {
                    ui.close();
                    open_path("https://github.com/TheDEMON78/mystream/releases");
                }
                if ui.button("Plugin Stream Deck MyStream…").clicked() {
                    ui.close();
                    self.open_settings_tab(settings::Tab::StreamDeck);
                }
                if ui.button("Page du projet").clicked() {
                    ui.close();
                    open_path("https://github.com/TheDEMON78/mystream/tree/rust");
                }
            });
        }
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
                    r.context_menu(|ui| self.scene_menu(ui, id, &name));
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
                let items: Vec<(u64, u64, bool, bool)> =
                    self.project.scene(scene_id).map(|s| s.items.iter().map(|i| (i.id, i.source, i.visible, i.locked)).collect()).unwrap_or_default();
                if items.is_empty() {
                    ui.label(RichText::new("Cliquez sur + pour ajouter une source.").italics().color(theme::MUTED));
                }
                let mut toggle = None;
                let mut lock = None;
                let mut menu: Option<(u64, String)> = None;
                for (item, source, visible, locked) in &items {
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
                        let w = (ui.available_width() - 76.0).max(0.0);
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
                        if r.double_clicked() {
                            self.selected = Some(*item);
                            self.props_window = true;
                        }
                        // Clic droit : le menu de la version Electron.
                        let kind_label = src.kind.label();
                        let candidates: Vec<(u64, String)> = self
                            .project
                            .sources
                            .iter()
                            .filter(|s| s.id != *source && s.kind.label() == kind_label)
                            .map(|s| (s.id, s.name.clone()))
                            .collect();
                        r.context_menu(|ui| {
                            let chosen = std::cell::RefCell::new(None::<String>);
                            let pick = |ui: &mut egui::Ui, text: &str, action: &str| {
                                if ui.button(text).clicked() {
                                    *chosen.borrow_mut() = Some(action.to_string());
                                    ui.close();
                                }
                            };
                            pick(ui, "Propriétés et filtres…", "props");
                            ui.separator();
                            pick(ui, "Ajuster à l'écran", "fit");
                            pick(ui, "Étirer à l'écran", "stretch");
                            pick(ui, "Centrer à l'écran", "center");
                            pick(ui, "Taille d'origine", "original");
                            pick(ui, "Modifier la transformation…", "transform");
                            ui.separator();
                            pick(ui, "Monter", "up");
                            pick(ui, "Descendre", "down");
                            pick(ui, if *locked { "Déverrouiller" } else { "Verrouiller" }, "lock");
                            ui.separator();
                            if shared {
                                ui.label(RichText::new("🔗 Partagée avec d'autres scènes").small().color(theme::MUTED));
                                pick(ui, "Rendre indépendante dans cette scène", "unlink");
                            }
                            if !candidates.is_empty() {
                                ui.menu_button("Utiliser une source existante", |ui| {
                                    for (id, name) in &candidates {
                                        if ui.button(name).clicked() {
                                            *chosen.borrow_mut() = Some(format!("link:{id}"));
                                            ui.close();
                                        }
                                    }
                                });
                            }
                            ui.separator();
                            pick(ui, "Supprimer", "remove");
                            if let Some(a) = chosen.into_inner() {
                                menu = Some((*item, a));
                            }
                        });
                        if theme::tool(ui, if *visible { "👁" } else { "⊘" }, if *visible { "Masquer" } else { "Afficher" }) {
                            toggle = Some(*item);
                        }
                        if theme::tool(
                            ui,
                            if *locked { "🔒" } else { "🔓" },
                            if *locked { "Déverrouiller" } else { "Verrouiller (empêche de la déplacer par erreur)" },
                        ) {
                            lock = Some(*item);
                        }
                    });
                    if let Some(i) = info {
                        ui.label(RichText::new(format!("      {i}")).small().color(theme::MUTED));
                    }
                }
                if let Some((id, action)) = menu {
                    self.selected = Some(id);
                    match action.as_str() {
                        "props" => self.props_window = true,
                        "transform" => self.transform_open = true,
                        "lock" => lock = Some(id),
                        "unlink" => {
                            self.project.unlink_item(scene_id, id);
                            self.touch();
                        }
                        "up" | "down" => {
                            if let Some(scene) = self.project.scene_mut(scene_id)
                                && let Some(i) = scene.items.iter().position(|it| it.id == id)
                            {
                                let j = if action == "up" { i.checked_sub(1) } else { (i + 1 < scene.items.len()).then_some(i + 1) };
                                if let Some(j) = j {
                                    scene.items.swap(i, j);
                                    self.touch();
                                }
                            }
                        }
                        a if a.starts_with("link:") => {
                            if let Ok(src) = a[5..].parse() {
                                self.project.link_item(scene_id, id, src);
                                self.touch();
                            }
                        }
                        other => self.item_action(other),
                    }
                }
                if let Some(id) = lock
                    && let Some(scene) = self.project.scene_mut(scene_id)
                    && let Some(it) = scene.items.iter_mut().find(|i| i.id == id)
                {
                    it.locked = !it.locked;
                    self.touch();
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
                if self.selected.is_some() && theme::tool(ui, "⚙", "Propriétés et filtres (ou double-clic)") {
                    self.props_window = true;
                }
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

    /// Ligne du mixeur ; renvoie (modifié, demande d'ouverture des filtres audio).
    fn mixer_row(ui: &mut egui::Ui, name: &str, level: f32, vol: &mut f32, muted: &mut bool, monitor: Option<&mut bool>, err: Option<String>) -> (bool, bool) {
        let mut changed = false;
        let mut filters = false;
        ui.horizontal(|ui| {
            let mut t = RichText::new(name);
            if err.is_some() {
                t = t.color(theme::WARN);
            }
            let r = ui
                .allocate_ui_with_layout(vec2(104.0, 22.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.set_width(104.0);
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
            match monitor {
                Some(m) => {
                    let b = egui::Button::new(RichText::new("🎧").size(14.0).color(if *m { theme::ACCENT } else { theme::MUTED }))
                        .frame(false)
                        .min_size(vec2(28.0, 24.0));
                    if ui.add(b).on_hover_text("Écouter au casque (sans l'ajouter au stream)").clicked() {
                        *m = !*m;
                        changed = true;
                    }
                    if theme::tool(ui, "⚙", "Filtres audio (gain, noise gate, compresseur)") {
                        filters = true;
                    }
                }
                None => {
                    ui.add_space(64.0);
                }
            }
            ui.spacing_mut().slider_width = (ui.available_width() - 120.0).clamp(60.0, 130.0);
            changed |= ui.add(egui::Slider::new(vol, 0.0..=2.0).show_value(false)).changed();
            ui.add_sized([44.0, 22.0], egui::Label::new(RichText::new(format!("{:.0} %", *vol * 100.0)).color(theme::MUTED)));
            let (bar, _) = ui.allocate_exact_size(vec2((ui.available_width() - 6.0).max(0.0), 8.0), Sense::hover());
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
        (changed, filters)
    }

    fn mixer_dock(&mut self, ui: &mut egui::Ui) {
        theme::dock(ui, "Mixeur audio", false, |ui, _| {
            let mut changed = false;
            let mut open = None;
            let a = &self.project.settings.audio;
            let fixed: Vec<(u64, &str)> = [(MIC, "Micro", a.mic_device != "off"), (DESKTOP, "Son du bureau", a.desktop_audio)]
                .into_iter()
                .filter(|(_, _, on)| *on)
                .map(|(id, n, _)| (id, n))
                .collect();
            for (id, name) in fixed {
                let t = self.mixer.get(id);
                let level = t.as_ref().map(|t| *t.level.lock()).unwrap_or(0.0);
                let err = t.as_ref().and_then(|t| t.error.lock().clone());
                let a = &mut self.project.settings.audio;
                // Le son du bureau n'a ni écoute (il s'entendrait deux fois) ni filtres.
                let (vol, muted, mon) = if id == MIC {
                    (&mut a.mic_volume, &mut a.mic_muted, Some(&mut a.mic_monitor))
                } else {
                    (&mut a.desktop_volume, &mut a.desktop_muted, None)
                };
                let (c, f) = Self::mixer_row(ui, name, level, vol, muted, mon, err);
                changed |= c;
                if f {
                    open = Some(id);
                }
            }
            // Comme dans OBS : seulement les sources de la scène à l'antenne.
            let on_air = self.on_air_sources();
            for s in self.project.sources.iter_mut().filter(|s| s.kind.has_audio() && on_air.contains(&s.id)) {
                let t = self.mixer.get(s.id);
                let level = t.as_ref().map(|t| *t.level.lock()).unwrap_or(0.0);
                let err = t.as_ref().and_then(|t| t.error.lock().clone());
                let (c, f) = Self::mixer_row(ui, &s.name, level, &mut s.volume, &mut s.muted, Some(&mut s.monitor), err);
                changed |= c;
                if f {
                    open = Some(s.id);
                }
            }
            if open.is_some() {
                self.audio_filters_open = open;
            }
            if changed {
                self.touch();
            }
        });
    }

    fn transitions_dock(&mut self, ui: &mut egui::Ui) {
        theme::dock(ui, "Transitions", false, |ui, _| {
            if transition_editor(ui, &mut self.project.settings.transition, "dock") {
                self.touch();
            }
        });
    }

    /// Fenêtre « Transition vers cette scène » (menu des scènes).
    fn scene_transition_window(&mut self, ctx: &egui::Context) {
        let Some(id) = self.scene_transition else { return };
        let global = self.project.settings.transition.clone();
        let Some(scene) = self.project.scene_mut(id) else {
            self.scene_transition = None;
            return;
        };
        let mut open = true;
        let mut changed = false;
        egui::Window::new(format!("Transition vers « {} »", scene.name)).open(&mut open).resizable(false).collapsible(false).show(ctx, |ui| {
            ui.set_width(300.0);
            let mut own = scene.transition.is_some();
            if ui.checkbox(&mut own, "Transition propre à cette scène").changed() {
                scene.transition = own.then(|| global.clone());
                changed = true;
            }
            match &mut scene.transition {
                Some(t) => changed |= transition_editor(ui, t, "scene"),
                None => {
                    ui.label(RichText::new(format!("Celle du dock Transitions : {}", global.kind.label())).color(theme::MUTED));
                }
            }
        });
        if changed {
            self.touch();
        }
        if !open {
            self.scene_transition = None;
        }
    }

    fn commands_dock(&mut self, ui: &mut egui::Ui) {
        theme::dock(ui, "Commandes", false, |ui, _| {
            ui.spacing_mut().item_spacing.y = 6.0;
            let streaming = self.live(Kind::Stream);
            let recording = self.live(Kind::Record);
            let label = if streaming {
                "Arrêter le streaming"
            } else if self.starting(Kind::Stream) {
                "Connexion…"
            } else {
                "Démarrer le streaming"
            };
            if theme::command(ui, label, streaming.then_some(theme::DANGER)) {
                if streaming && self.project.settings.confirm_stop_stream {
                    self.confirm_stop = true;
                } else {
                    self.toggle_output(Kind::Stream);
                }
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

    /// Colonne de droite : chat, puis titre et catégorie (comme la version Electron).
    /// Filtres audio d'une piste (micro ou source sonore).
    fn audio_filters_window(&mut self, ctx: &egui::Context) {
        let Some(id) = self.audio_filters_open else { return };
        let (name, f) = if id == MIC {
            ("Micro".to_string(), &mut self.project.settings.audio.mic_filters)
        } else {
            match self.project.sources.iter_mut().find(|s| s.id == id) {
                Some(s) => (s.name.clone(), &mut s.audio_filters),
                None => {
                    self.audio_filters_open = None;
                    return;
                }
            }
        };
        let mut open = true;
        let mut changed = false;
        egui::Window::new(format!("Filtres audio : {name}")).open(&mut open).resizable(false).collapsible(false).show(ctx, |ui| {
            changed |= ui.add(egui::Slider::new(&mut f.gain_db, -20.0..=30.0).suffix(" dB").text("Gain")).changed();
            ui.separator();
            changed |= ui.checkbox(&mut f.gate, "Noise gate (coupe le bruit de fond entre les phrases)").changed();
            ui.add_enabled_ui(f.gate, |ui| {
                changed |= ui.add(egui::Slider::new(&mut f.gate_threshold_db, -80.0..=-10.0).suffix(" dB").text("Seuil d'ouverture")).changed();
            });
            ui.separator();
            changed |= ui.checkbox(&mut f.compressor, "Compresseur (égalise le volume, évite la saturation)").changed();
            ui.add_enabled_ui(f.compressor, |ui| {
                changed |= ui.add(egui::Slider::new(&mut f.comp_threshold_db, -50.0..=0.0).suffix(" dB").text("Seuil")).changed();
                changed |= ui.add(egui::Slider::new(&mut f.comp_ratio, 1.0..=20.0).text("Ratio")).changed();
            });
            ui.separator();
            if ui.button("Réinitialiser").clicked() {
                *f = Default::default();
                changed = true;
            }
        });
        if changed {
            self.touch();
        }
        if !open {
            self.audio_filters_open = None;
        }
    }

    /// Fenêtre des propriétés et filtres de la source sélectionnée.
    fn properties_window(&mut self, ctx: &egui::Context) {
        if !self.props_window || self.selected.is_none() {
            self.props_window = false;
            return;
        }
        let mut open = true;
        egui::Window::new("Propriétés et filtres").open(&mut open).default_width(380.0).default_pos(ctx.content_rect().center() - vec2(190.0, 250.0)).show(
            ctx,
            |ui| {
                egui::ScrollArea::vertical().max_height(560.0).show(ui, |ui| self.properties(ui));
            },
        );
        self.props_window = open;
    }

    #[allow(dead_code)]
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
        self.live_bar(ui);
        let avail = (ui.available_size() - vec2(0.0, 42.0)).max(Vec2::ZERO);
        let paused = self.shared.preview_paused.load(std::sync::atomic::Ordering::Relaxed);
        if self.preview_hidden || paused {
            let text = if paused { "Aperçu en pause pendant le live : MyStream n'est pas au premier plan" } else { "Aperçu masqué (allège la carte graphique)" };
            let (r, _) = ui.allocate_exact_size(avail, Sense::hover());
            ui.painter().rect_filled(r, 0.0, Color32::from_rgb(0x0c, 0x0d, 0x10));
            ui.painter().text(
                r.center(),
                egui::Align2::CENTER_CENTER,
                text,
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
            self.add("Capture de jeu", SourceKind::Game { audio: true, method: "screen".into() });
        }
        ui.menu_button("🖥 Capture d'écran", |ui| {
            for m in capture::monitors() {
                if ui.button(&m.name).clicked() {
                    ui.close();
                    self.add("Capture d'écran", SourceKind::Display { monitor: m.index, audio: false });
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
            let mut kind = SourceKind::text("Votre texte", 72.0, [255, 255, 255, 255]);
            if let SourceKind::Text { bold, outline, .. } = &mut kind {
                (*bold, *outline) = (true, true);
            }
            self.add("Texte", kind);
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
        if cfg!(windows) && ui.button("🌐 Navigateur (alertes, widgets)").on_hover_text("Page web transparente : alertes Streamlabs, StreamElements, widgets…").clicked() {
            ui.close();
            self.add("Navigateur", SourceKind::Browser { url: String::new(), width: 800, height: 600, css: crate::model::browser_css(), audio: true });
        }
        if ui.button("💬 Chat (incrusté dans le stream)").clicked() {
            ui.close();
            self.add("Chat", SourceKind::Chat(Default::default()));
        }
        if ui.button("■ Couleur").clicked() {
            ui.close();
            let (w, h) = self.canvas;
            self.add("Couleur", SourceKind::Color { rgba: [0x2a, 0x1f, 0x4d, 255], width: w, height: h });
        }
        ui.menu_button("🎤 Entrée audio (micro…)", |ui| {
            if ui.button("Par défaut").clicked() {
                ui.close();
                self.add("Entrée audio", SourceKind::AudioInput { device: String::new(), noise_suppression: false });
            }
            for d in crate::audio::input_devices() {
                if ui.button(&d).clicked() {
                    ui.close();
                    let name: String = d.chars().take(40).collect();
                    self.add(&name, SourceKind::AudioInput { device: d.clone(), noise_suppression: false });
                }
            }
        });
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
            SourceKind::Text { text, size, rgba, background, mono, font, bold, italic, align, outline, outline_color, outline_size } => {
                changed |= ui.add(egui::TextEdit::multiline(text).desired_rows(2)).changed();
                egui::ComboBox::from_label("Police").selected_text(if *mono { "Chasse fixe" } else { font.as_str() }).show_ui(ui, |ui| {
                    for (f, _, _) in crate::model::CHAT_FONTS {
                        if ui.selectable_label(!*mono && font == f, f).clicked() {
                            (*font, *mono) = (f.to_string(), false);
                            changed = true;
                        }
                    }
                    if ui.selectable_label(*mono, "Chasse fixe").clicked() {
                        *mono = true;
                        changed = true;
                    }
                });
                changed |= ui.add(egui::Slider::new(size, 4.0..=400.0).text("Taille")).changed();
                ui.horizontal(|ui| {
                    ui.label("Couleur");
                    changed |= color(ui, rgba);
                    ui.label("Fond");
                    changed |= color(ui, background);
                });
                ui.horizontal(|ui| {
                    changed |= ui.checkbox(bold, "Gras").changed();
                    changed |= ui.checkbox(italic, "Italique").changed();
                });
                ui.horizontal(|ui| {
                    ui.label("Alignement");
                    for (v, l) in [("left", "Gauche"), ("center", "Centre"), ("right", "Droite")] {
                        changed |= ui.selectable_value(align, v.to_string(), l).changed();
                    }
                });
                ui.horizontal(|ui| {
                    changed |= ui.checkbox(outline, "Contour").changed();
                    if *outline {
                        changed |= color(ui, outline_color);
                        changed |= ui.add(egui::DragValue::new(outline_size).range(1.0..=40.0).suffix(" px")).changed();
                    }
                });
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
                let res = |w: u32, h: u32| if w == 0 { "Par défaut".to_string() } else { format!("{w} × {h}") };
                egui::ComboBox::from_label("Résolution").selected_text(res(*width, *height)).show_ui(ui, |ui| {
                    for (w, h) in [(0, 0), (640, 480), (1280, 720), (1920, 1080), (2560, 1440), (3840, 2160)] {
                        if ui.selectable_label(*width == w && *height == h, res(w, h)).clicked() {
                            (*width, *height) = (w, h);
                            changed = true;
                        }
                    }
                });
                let rate = |f: u32| if f == 0 { "Par défaut".to_string() } else { f.to_string() };
                egui::ComboBox::from_label("Images par seconde").selected_text(rate(*cam_fps)).show_ui(ui, |ui| {
                    for f in [0, 30, 60] {
                        changed |= ui.selectable_value(cam_fps, f, rate(f)).changed();
                    }
                });
                ui.label(format!("{fps:.0} images/s reçues"));
            }
            SourceKind::Window { audio, .. } => {
                changed |= ui.checkbox(audio, "Capturer le son de la fenêtre").changed();
                ui.label(format!("{fps:.0} images/s capturées"));
            }
            SourceKind::Game { audio, method } => {
                ui.label(
                    RichText::new("Aucun réglage nécessaire : le jeu en plein écran ou en fenêtré sans bordure est détecté, avec son son. Pendant un Alt+Tab, la dernière image du jeu reste affichée.")
                        .small()
                        .color(theme::MUTED),
                );
                changed |= ui.checkbox(audio, "Capturer le son du jeu").changed();
                let label = |m: &str| if m == "window" { "Fenêtre du jeu : pour un jeu qui ne couvre pas tout l'écran" } else { "Écran du jeu : la plus fluide (recommandée)" };
                egui::ComboBox::from_label("Méthode").selected_text(label(method)).show_ui(ui, |ui| {
                    for m in ["screen", "window"] {
                        changed |= ui.selectable_value(method, m.to_string(), label(m)).changed();
                    }
                });
                ui.label(format!("{fps:.0} images/s capturées"));
            }
            SourceKind::Display { audio, .. } => {
                changed |= ui.checkbox(audio, "Capturer le son du PC (hors MyStream)").changed();
                ui.label(format!("{fps:.0} images/s capturées"));
            }
            SourceKind::AudioInput { device, noise_suppression } => {
                let label = |d: &str| if d.is_empty() { "Par défaut".to_string() } else { d.to_string() };
                egui::ComboBox::from_label("Périphérique").selected_text(label(device)).show_ui(ui, |ui| {
                    changed |= ui.selectable_value(device, String::new(), "Par défaut").changed();
                    for d in crate::audio::input_devices() {
                        changed |= ui.selectable_value(device, d.clone(), d).changed();
                    }
                });
                changed |= ui.checkbox(noise_suppression, "Suppression du bruit (souffle, ventilateur, clavier)").changed();
            }
            SourceKind::AppAudio { exe } => {
                ui.label(format!("Programme : {exe}"));
            }
            SourceKind::Chat(st) => {
                ui.horizontal(|ui| {
                    ui.label("Taille");
                    changed |= ui.add(egui::DragValue::new(&mut st.width).range(100..=3840).suffix(" px")).changed();
                    ui.label("×");
                    changed |= ui.add(egui::DragValue::new(&mut st.height).range(100..=2160).suffix(" px")).changed();
                });
                egui::ComboBox::from_label("Police").selected_text(st.font.as_str()).show_ui(ui, |ui| {
                    for (f, _, _) in crate::model::CHAT_FONTS {
                        changed |= ui.selectable_value(&mut st.font, f.to_string(), f).changed();
                    }
                });
                changed |= ui.add(egui::Slider::new(&mut st.size, 8.0..=96.0).text("Taille du texte")).changed();
                ui.horizontal(|ui| {
                    ui.label("Texte");
                    changed |= color(ui, &mut st.text_color);
                    ui.label("Pseudos");
                    changed |= color(ui, &mut st.name_color);
                    ui.label("Fond");
                    changed |= color(ui, &mut st.background);
                });
                ui.label(RichText::new("Pseudos : couleur utilisée si la plateforme n'en fournit pas.").small().color(theme::MUTED));
                changed |= ui.add(egui::Slider::new(&mut st.lifetime, 0..=600).text("Masquer après (s, 0 = jamais)")).changed();
                changed |= ui.add(egui::Slider::new(&mut st.max_messages, 1..=100).text("Messages au plus")).changed();
                changed |= ui.checkbox(&mut st.show_icons, "Logo de la plateforme").changed();
                changed |= ui.checkbox(&mut st.show_badges, "Badges (modérateur, abonné…)").changed();
                changed |= ui.checkbox(&mut st.show_system, "Messages système").changed();
                changed |= ui.checkbox(&mut st.shadow, "Contour du texte").changed();
                ui.label(RichText::new("Le chat vient de Restream, ou de Twitch et Kick en direct (Paramètres > Chat).").small().color(theme::MUTED));
            }
            SourceKind::Browser { url, width, height, css, audio } => {
                // La page n'est rechargée qu'une fois la saisie terminée.
                ui.label("Adresse (ou fichier local)");
                ui.horizontal(|ui| {
                    let r = ui.add(egui::TextEdit::singleline(url).hint_text("https://…").desired_width(ui.available_width() - 70.0));
                    changed |= r.lost_focus();
                    if ui.button("Fichier…").clicked()
                        && let Some(p) = rfd::FileDialog::new().add_filter("Pages web", &["html", "htm"]).pick_file()
                    {
                        *url = p.to_string_lossy().into_owned();
                        changed = true;
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Taille");
                    changed |= ui.add(egui::DragValue::new(width).range(16..=3840).suffix(" px")).changed();
                    ui.label("×");
                    changed |= ui.add(egui::DragValue::new(height).range(16..=2160).suffix(" px")).changed();
                });
                ui.label("CSS personnalisé");
                changed |= ui.add(egui::TextEdit::multiline(css).code_editor().desired_rows(3)).lost_focus();
                changed |= ui.checkbox(audio, "Capturer le son de la page").changed();
                ui.horizontal(|ui| {
                    ui.label(format!("{fps:.0} images/s capturées"));
                    if ui.button("⟳ Recharger la page").clicked() {
                        // Une modification relance la source, donc recharge la page.
                        changed = true;
                    }
                });
            }
        }
        if !src.kind.audio_only() {
            egui::CollapsingHeader::new("Filtres").show(ui, |ui| {
                let f = &mut src.filters;
                changed |= ui.add(egui::Slider::new(&mut f.opacity, 0.0..=1.0).text("Opacité")).changed();
                changed |= ui.add(egui::Slider::new(&mut f.brightness, -0.5..=0.5).text("Luminosité")).changed();
                changed |= ui.add(egui::Slider::new(&mut f.contrast, 0.0..=2.0).text("Contraste")).changed();
                changed |= ui.add(egui::Slider::new(&mut f.saturation, 0.0..=2.0).text("Saturation")).changed();
                changed |= ui.add(egui::Slider::new(&mut f.hue, -180.0..=180.0).text("Teinte (°)")).changed();
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
                    (item.scale, item.scale_y) = (None, None);
                    item.x = 0.0;
                    item.y = 0.0;
                }
                if ui.button("Taille d'origine").clicked() {
                    (item.scale, item.scale_y) = (Some(1.0), None);
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
            let (sx, sy) = it.scales()?;
            Some(Rect::from_min_size(pos2(it.x, it.y), vec2(w.saturating_sub(c[0] + c[2]) as f32 * sx, h.saturating_sub(c[1] + c[3]) as f32 * sy)))
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
        if !resp.dragged() {
            self.drag_raw = None;
        }
        // Calage magnétique : bords et centre de l'écran, bords et centres des autres sources.
        let others: Vec<Rect> = self
            .project
            .scene(scene_id)
            .map(|s| s.items.iter().filter(|i| i.visible && Some(i.id) != self.selected).filter_map(&item_rect).collect())
            .unwrap_or_default();
        let free = ui.input(|i| i.modifiers.alt);
        let mut guides: Vec<(bool, f32)> = Vec::new();
        if let Some(sel) = self.selected
            && let Some(scene) = self.project.scene_mut(scene_id)
            && let Some(item) = scene.items.iter_mut().find(|i| i.id == sel)
        {
            if resp.dragged() && drag != Vec2::ZERO && !item.locked {
                // Position « libre » suivie à part : le calage ne bloque pas le déplacement.
                let raw = self.drag_raw.get_or_insert((item.x, item.y));
                raw.0 += drag.x;
                raw.1 += drag.y;
                let (rx, ry) = *raw;
                (item.x, item.y) = (rx, ry);
                if !free && let Some(r) = item_rect(item) {
                    let tol = 10.0 / k; // 10 pixels à l'écran
                    let snap = |pos: f32, len: f32, targets: &[f32]| -> Option<(f32, f32)> {
                        // Bord de début, centre, bord de fin de l'élément contre chaque cible.
                        let mut best: Option<(f32, f32, f32)> = None;
                        for t in targets {
                            for off in [0.0, len / 2.0, len] {
                                let d = (pos + off - t).abs();
                                if d <= tol && best.is_none_or(|b| d < b.2) {
                                    best = Some((t - off, *t, d));
                                }
                            }
                        }
                        best.map(|b| (b.0, b.1))
                    };
                    let mut xs = vec![0.0, cw / 2.0, cw];
                    let mut ys = vec![0.0, ch / 2.0, ch];
                    for o in &others {
                        xs.extend([o.min.x, o.center().x, o.max.x]);
                        ys.extend([o.min.y, o.center().y, o.max.y]);
                    }
                    if let Some((x, line)) = snap(rx, r.width(), &xs) {
                        item.x = x;
                        guides.push((true, line));
                    }
                    if let Some((y, line)) = snap(ry, r.height(), &ys) {
                        item.y = y;
                        guides.push((false, line));
                    }
                }
                moved = true;
            }
            // Molette : agrandir ou réduire autour du centre de l'élément.
            if zoom != 0.0
                && !item.locked
                && let (Some(s), Some(r)) = (item.scale, item_rect(item))
            {
                let ns = (s * (1.0 + zoom * 0.0015)).clamp(0.02, 10.0);
                let k = ns / s;
                item.x -= r.width() * (k - 1.0) / 2.0;
                item.y -= r.height() * (k - 1.0) / 2.0;
                item.scale = Some(ns);
                item.scale_y = item.scale_y.map(|y| y * k);
                moved = true;
            }
            if let Some(r) = item_rect(item) {
                let screen = Rect::from_min_max(rect.min + r.min.to_vec2() * k, rect.min + r.max.to_vec2() * k);
                painter.rect_stroke(screen, 0.0, Stroke::new(2.0, LIVE), StrokeKind::Outside);
            }
            // Lignes de repère du calage.
            let guide = Stroke::new(1.0, Color32::from_rgb(0xff, 0x4d, 0xd2));
            for (vertical, v) in &guides {
                if *vertical {
                    let x = rect.min.x + v * k;
                    painter.vline(x, rect.y_range(), guide);
                } else {
                    let y = rect.min.y + v * k;
                    painter.hline(rect.x_range(), y, guide);
                }
            }
        }
        if moved {
            self.touch();
        }
    }

    /// Cherche une mise à jour ; `manual` : demandé par l'utilisateur (résultat affiché).
    fn check_updates(&mut self, manual: bool) {
        self.update_checked = Some(Instant::now());
        self.update_manual = manual;
        crate::update::check(self.update.clone());
    }

    /// Vérification régulière et installation automatique (appelé même fenêtre réduite).
    fn update_tick(&mut self) {
        use crate::update::State;
        if self.update_checked.is_none_or(|t| t.elapsed() > Duration::from_secs(30 * 60)) {
            self.check_updates(false);
        }
        let busy = !self.shared.outputs.lock().is_empty() || !self.pending.is_empty();
        let state = self.update.lock().clone();
        if let State::Available(a) = state
            && self.project.settings.auto_update
            && !busy
        {
            crate::update::install(self.update.clone(), a);
        }
    }

    /// État des mises à jour en une phrase (onglet Général).
    fn update_message(&self) -> Option<String> {
        use crate::update::State;
        Some(match self.update.lock().clone() {
            State::Idle => return None,
            State::Checking => "Recherche en cours…".into(),
            State::UpToDate => "Vous avez la dernière version.".into(),
            State::Available(a) => format!("Nouvelle version disponible : natif 0.1.{}.", a.number),
            State::Downloading => "Téléchargement de la mise à jour…".into(),
            State::Ready(n) => format!("Version natif 0.1.{n} installée : elle sera utilisée au prochain lancement."),
            State::Failed(e) => format!("Mise à jour : {e}."),
        })
    }

    fn update_banner(&mut self, ui: &mut egui::Ui) {
        use crate::update::State;
        let state = self.update.lock().clone();
        let busy = !self.shared.outputs.lock().is_empty();
        match state {
            State::Idle | State::Checking => {}
            State::UpToDate => {
                if self.update_manual {
                    self.update_manual = false;
                    self.notify("Vous avez la dernière version de MyStream.", false);
                }
            }
            State::Available(a) => {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("Nouvelle version disponible (natif 0.1.{}).", a.number)).color(OK));
                    let b = ui.add_enabled(!busy, egui::Button::new("Installer"));
                    let b = if busy { b.on_disabled_hover_text("Elle s'installera à la fin du live ou de l'enregistrement") } else { b };
                    if b.clicked() {
                        crate::update::install(self.update.clone(), a.clone());
                    }
                });
            }
            State::Downloading => {
                ui.label(RichText::new("Téléchargement de la mise à jour…").color(WARN));
            }
            State::Ready(n) => {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("MyStream natif 0.1.{n} est installé.")).color(OK));
                    let b = ui.add_enabled(!busy, egui::Button::new("Redémarrer maintenant"));
                    let b = if busy { b.on_disabled_hover_text("Arrêtez le streaming et l'enregistrement d'abord") } else { b.on_hover_text("Sinon, la nouvelle version sera utilisée au prochain lancement") };
                    if b.clicked() {
                        self.project.save();
                        crate::update::restart();
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
            }
            State::Failed(e) => {
                if self.update_manual {
                    ui.label(RichText::new(format!("Mise à jour : {e}")).color(LIVE));
                }
            }
        }
    }
}

impl eframe::App for App {
    /// Appelé aussi quand la fenêtre est réduite (en jeu) : Stream Deck,
    /// raccourcis globaux et sorties doivent continuer de fonctionner.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Pendant un live, MyStream au second plan : l'aperçu n'est plus redessiné.
        let (focused, minimized) = ctx.input(|i| (i.viewport().focused.unwrap_or(true), i.viewport().minimized.unwrap_or(false)));
        let paused = minimized || (self.project.settings.auto_hide_preview && self.live(Kind::Stream) && !focused);
        self.shared.preview_paused.store(paused, std::sync::atomic::Ordering::Relaxed);
        self.tray_tick(ctx, minimized);
        self.sync_runtimes();
        self.poll_outputs();
        self.check_outputs();
        let viewers = self.live_viewers();
        self.session_tick(viewers);
        self.handle_remote();
        self.handle_hotkeys();
        self.update_items();
        self.update_tick();
        if self.dirty.is_some_and(|t| t.elapsed() > Duration::from_secs(1)) {
            self.record_undo();
            self.project.save();
            self.dirty = None;
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        let bar = |fill| egui::Frame::new().fill(fill);
        egui::Panel::top("menu").exact_size(36.0).frame(bar(theme::BG).stroke(egui::Stroke::new(1.0, theme::BORDER))).show(ui, |ui| self.top_bar(ui));
        egui::Panel::bottom("statut").exact_size(28.0).frame(bar(theme::BG).inner_margin(egui::Margin::symmetric(10, 0))).show(ui, |ui| self.status_bar(ui));
        if !self.zone(false).is_empty() {
            egui::Panel::bottom("docks")
                .exact_size(290.0)
                .frame(bar(theme::BG).inner_margin(egui::Margin { left: 4, right: 4, top: 4, bottom: 2 }))
                .show(ui, |ui| self.docks(ui));
        }
        if !self.zone(true).is_empty() {
            egui::Panel::right("droite").default_size(340.0).min_size(260.0).frame(bar(theme::BG).inner_margin(4)).show(ui, |ui| self.right_column(ui));
        }
        egui::CentralPanel::default().frame(bar(theme::BG).inner_margin(egui::Margin::symmetric(4, 4))).show(ui, |ui| self.preview(ui));
        self.settings_window(&ctx);
        self.properties_window(&ctx);
        self.audio_filters_window(&ctx);
        self.transform_window(&ctx);
        self.scene_transition_window(&ctx);
        self.save_layout_window(&ctx);
        self.confirm_stop_window(&ctx);
        self.wizard_window(&ctx);
        // Raccourcis de l'interface (hors champs de saisie).
        if !ctx.egui_wants_keyboard_input() {
            let (undo, redo, transform) = ctx.input(|i| {
                (
                    i.modifiers.command && i.key_pressed(egui::Key::Z) && !i.modifiers.shift,
                    i.modifiers.command && (i.key_pressed(egui::Key::Y) || (i.modifiers.shift && i.key_pressed(egui::Key::Z))),
                    i.modifiers.command && i.key_pressed(egui::Key::E),
                )
            });
            if undo {
                self.undo();
            }
            if redo {
                self.redo();
            }
            if transform && self.selected.is_some() {
                self.transform_open = true;
            }
        }
        self.session_windows(&ctx);

        if let Some((start, secs)) = self.autotest {
            let t = start.elapsed();
            if t > Duration::from_secs(2) && !self.live(Kind::Record) && t < Duration::from_secs(3) {
                self.toggle_output(Kind::Record);
            }
            // Changement de scène à mi-parcours (pour tester les transitions).
            if t > Duration::from_secs(2 + secs / 2) && self.project.scenes.len() > 1 && self.shown == self.project.scenes[0].id {
                self.project.current = self.project.scenes[1].id;
            }
            if t > Duration::from_secs(2 + secs) {
                self.autotest = None;
                let st = self.shared.stats.lock().clone();
                log::info!("Test : {:.1} i/s, rendu {:.2} ms, {} manquée(s), {} perdue(s)", st.fps, st.render_ms, st.lagged, st.dropped);
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
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
/// Réglages d'une transition (dock Transitions et transition propre à une scène).
fn transition_editor(ui: &mut egui::Ui, t: &mut crate::model::Transition, id: &str) -> bool {
    let mut changed = false;
    let full = |ui: &egui::Ui| (ui.available_width() - 8.0).max(0.0);
    ui.label(RichText::new("Type").color(theme::MUTED));
    egui::ComboBox::from_id_salt(("transition", id)).width(full(ui)).selected_text(t.kind.label()).show_ui(ui, |ui| {
        for k in [TransitionKind::Cut, TransitionKind::Fade, TransitionKind::Slide, TransitionKind::Stinger] {
            changed |= ui.selectable_value(&mut t.kind, k, k.label()).changed();
        }
    });
    ui.add_space(6.0);
    if t.kind == TransitionKind::Stinger {
        let name = std::path::Path::new(&t.stinger_file).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Aucune vidéo".into());
        ui.label(RichText::new(name).small().color(theme::MUTED));
        if ui.add(egui::Button::new("Choisir la vidéo ou le GIF…").min_size(vec2(full(ui), 26.0))).clicked()
            && let Some(p) = rfd::FileDialog::new().add_filter("Vidéos et GIF", &["webm", "mov", "mp4", "mkv", "avi", "gif"]).pick_file()
        {
            t.stinger_file = p.to_string_lossy().into_owned();
            changed = true;
        }
        ui.label(RichText::new("Point de coupe (ms)").color(theme::MUTED)).on_hover_text("Moment de la vidéo où la scène change (quand l'animation couvre l'écran)");
        changed |= ui.add_sized([full(ui), 26.0], egui::DragValue::new(&mut t.stinger_point_ms).range(0..=20000).speed(10)).changed();
        changed |= ui.checkbox(&mut t.stinger_audio, "Son de l'animation").changed();
        return changed;
    }
    ui.label(RichText::new("Durée (ms)").color(theme::MUTED));
    ui.add_enabled_ui(t.kind != TransitionKind::Cut, |ui| {
        changed |= ui.add_sized([full(ui), 26.0], egui::DragValue::new(&mut t.duration_ms).range(50..=10000).speed(10)).changed();
    });
    if t.kind == TransitionKind::Slide {
        ui.add_space(4.0);
        ui.label(RichText::new("Direction").color(theme::MUTED));
        let label = |d: &str| match d {
            "right" => "Vers la droite",
            "up" => "Vers le haut",
            "down" => "Vers le bas",
            _ => "Vers la gauche",
        };
        egui::ComboBox::from_id_salt(("direction", id)).width(full(ui)).selected_text(label(&t.direction)).show_ui(ui, |ui| {
            for d in ["left", "right", "up", "down"] {
                changed |= ui.selectable_value(&mut t.direction, d.to_string(), label(d)).changed();
            }
        });
    }
    changed
}

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
