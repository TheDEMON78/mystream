//! Fenêtre « Paramètres », comme celle de la version Electron : onglets à
//! gauche (Général, Stream, Restream, Chat, Vidéo, Enregistrement, Stream
//! Deck, Raccourcis), contenu à droite, et en bas « Valeurs par défaut »,
//! « Annuler » et « OK ».

use super::{App, RESOLUTIONS};
use crate::model::{HOTKEY_ACTIONS, Settings};
use crate::output;
use crate::restream;
use crate::theme;
use egui::{Color32, RichText, vec2};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
    #[default]
    General,
    Stream,
    Restream,
    Chat,
    Video,
    Record,
    StreamDeck,
    Hotkeys,
}

impl Tab {
    const ALL: [Tab; 8] = [Tab::General, Tab::Stream, Tab::Restream, Tab::Chat, Tab::Video, Tab::Record, Tab::StreamDeck, Tab::Hotkeys];

    pub fn from_index(n: usize) -> Self {
        Self::ALL[n.min(Self::ALL.len() - 1)]
    }

    fn label(self) -> &'static str {
        match self {
            Tab::General => "Général",
            Tab::Stream => "Stream",
            Tab::Restream => "Restream",
            Tab::Chat => "Chat",
            Tab::Video => "Vidéo",
            Tab::Record => "Enregistrement",
            Tab::StreamDeck => "Stream Deck",
            Tab::Hotkeys => "Raccourcis",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Tab::General => "⚙",
            Tab::Stream => "📡",
            Tab::Restream => "🔁",
            Tab::Chat => "💬",
            Tab::Video => "🖥",
            Tab::Record => "⏺",
            Tab::StreamDeck => "🎮",
            Tab::Hotkeys => "⌨",
        }
    }
}

/// Texte d'aide sous un champ.
fn hint(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(11.5).color(theme::MUTED));
}

/// Titre de section dans un onglet.
fn section(ui: &mut egui::Ui, text: &str) {
    ui.add_space(6.0);
    ui.label(RichText::new(text).family(theme::bold()).size(13.5).color(theme::TEXT));
    ui.add_space(2.0);
}

/// Grille « libellé : champ » alignée.
fn grid(ui: &mut egui::Ui, id: &str, content: impl FnOnce(&mut egui::Ui)) {
    egui::Grid::new(id).num_columns(2).spacing(vec2(14.0, 8.0)).min_col_width(170.0).show(ui, content);
}

/// Préréglage x264 (utilisé quand l'encodeur est le processeur).
fn preset_combo(ui: &mut egui::Ui, id: &str, preset: &mut String) {
    let label = |p: &str| output::PRESETS.iter().find(|(k, _)| *k == p).map(|(_, l)| *l).unwrap_or("veryfast : recommandé");
    egui::ComboBox::from_id_salt(id).width(300.0).selected_text(label(preset)).show_ui(ui, |ui| {
        for (k, l) in output::PRESETS {
            ui.selectable_value(preset, k.to_string(), l);
        }
    });
}

/// Combinaison enregistrée (« ctrl+shift+KeyS ») en texte lisible (« Ctrl + Maj + S »).
pub fn pretty_hotkey(text: &str) -> String {
    text.split('+')
        .filter(|p| !p.is_empty())
        .map(|p| match p.to_lowercase().as_str() {
            "ctrl" | "control" => "Ctrl".to_string(),
            "shift" => "Maj".to_string(),
            "alt" => "Alt".to_string(),
            "super" | "meta" | "cmd" => "Win".to_string(),
            _ => p.strip_prefix("Key").or_else(|| p.strip_prefix("Digit")).unwrap_or(p).to_string(),
        })
        .collect::<Vec<_>>()
        .join(" + ")
}

/// Touche egui -> nom compris par les raccourcis globaux.
fn key_code(k: egui::Key) -> Option<String> {
    let name = k.name();
    if name.len() == 1 {
        let c = name.chars().next()?;
        if c.is_ascii_alphabetic() {
            return Some(format!("Key{}", c.to_ascii_uppercase()));
        }
        if c.is_ascii_digit() {
            return Some(format!("Digit{c}"));
        }
        return None;
    }
    let ok = name.starts_with('F') && name[1..].parse::<u8>().is_ok()
        || matches!(name, "Space" | "Enter" | "Tab" | "Backspace" | "Insert" | "Delete" | "Home" | "End" | "PageUp" | "PageDown" | "ArrowUp" | "ArrowDown" | "ArrowLeft" | "ArrowRight");
    ok.then(|| name.to_string())
}

impl App {
    pub(super) fn open_settings_tab(&mut self, tab: Tab) {
        self.draft = self.project.settings.clone();
        self.settings_tab = tab;
        self.settings_open = true;
        self.capturing_hotkey = None;
    }

    /// Remet l'onglet affiché à ses valeurs par défaut (sans toucher aux autres).
    fn reset_tab(&mut self) {
        let def = Settings::default();
        let d = &mut self.draft;
        match self.settings_tab {
            Tab::General => d.auto_update = def.auto_update,
            Tab::Stream => {
                let (server, key) = (d.stream.server.clone(), d.stream.key.clone());
                d.stream = def.stream;
                (d.stream.server, d.stream.key) = (server, key);
            }
            Tab::Restream => d.restream = def.restream,
            Tab::Chat => d.chat = def.chat,
            Tab::Video => (d.width, d.height, d.fps) = (def.width, def.height, def.fps),
            Tab::Record => d.record = def.record,
            Tab::StreamDeck => {
                let password = d.remote.password.clone();
                d.remote = def.remote;
                d.remote.password = password;
                d.remote.electron_checked = true;
            }
            Tab::Hotkeys => d.hotkeys = def.hotkeys,
        }
    }

    pub(super) fn settings_window(&mut self, ctx: &egui::Context) {
        if !self.settings_open {
            return;
        }
        let mut open = true;
        let (mut ok, mut cancel, mut reset) = (false, false, false);
        let size = vec2(800.0f32.min(ctx.content_rect().width() - 40.0), 540.0f32.min(ctx.content_rect().height() - 80.0));
        egui::Window::new(RichText::new("Paramètres").family(theme::bold()))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .fixed_size(size)
            .anchor(egui::Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .show(ctx, |ui| {
                let body_h = size.y - 48.0;
                let tabs_w = 170.0;
                ui.allocate_ui_with_layout(vec2(size.x, body_h), egui::Layout::left_to_right(egui::Align::Min), |ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    // Onglets.
                    ui.allocate_ui_with_layout(vec2(tabs_w, body_h), egui::Layout::top_down(egui::Align::Min), |ui| {
                        egui::Frame::new().fill(theme::BG).corner_radius(6).inner_margin(egui::Margin::same(8)).show(ui, |ui| {
                            ui.set_min_size(vec2(tabs_w - 16.0, body_h - 16.0));
                            ui.spacing_mut().item_spacing.y = 2.0;
                            for tab in Tab::ALL {
                                let active = self.settings_tab == tab;
                                let text = RichText::new(format!("{}  {}", tab.icon(), tab.label())).color(if active { Color32::WHITE } else { theme::TEXT });
                                let b = egui::Button::new(text)
                                    .fill(if active { theme::ACCENT2 } else { Color32::TRANSPARENT })
                                    .stroke(egui::Stroke::NONE)
                                    .corner_radius(5)
                                    .min_size(vec2(tabs_w - 16.0, 30.0));
                                if ui.add(b).clicked() {
                                    self.settings_tab = tab;
                                    self.capturing_hotkey = None;
                                }
                            }
                        });
                    });
                    ui.add_space(16.0);
                    // Contenu de l'onglet.
                    let content_w = size.x - tabs_w - 16.0;
                    ui.allocate_ui_with_layout(vec2(content_w, body_h), egui::Layout::top_down(egui::Align::Min), |ui| {
                        ui.set_max_width(content_w);
                        ui.label(RichText::new(self.settings_tab.label()).family(theme::bold()).size(16.0));
                        ui.add_space(4.0);
                        egui::ScrollArea::vertical().id_salt(("parametres", self.settings_tab as u8)).auto_shrink(false).max_height(body_h - 30.0).show(ui, |ui| {
                            ui.set_max_width(content_w - 12.0);
                            ui.spacing_mut().item_spacing.y = 6.0;
                            match self.settings_tab {
                                Tab::General => self.tab_general(ui),
                                Tab::Stream => self.tab_stream(ui),
                                Tab::Restream => self.tab_restream(ui),
                                Tab::Chat => self.tab_chat(ui),
                                Tab::Video => self.tab_video(ui),
                                Tab::Record => self.tab_record(ui),
                                Tab::StreamDeck => self.tab_streamdeck(ui),
                                Tab::Hotkeys => self.tab_hotkeys(ui),
                            }
                        });
                    });
                });
                ui.add_space(6.0);
                theme::line(ui);
                ui.add_space(4.0);
                // Boutons du bas.
                ui.horizontal(|ui| {
                    if ui.button("Valeurs par défaut").on_hover_text("Remet cet onglet à ses réglages d'origine").clicked() {
                        reset = true;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let okb = egui::Button::new(RichText::new("OK").color(Color32::WHITE).strong()).fill(theme::ACCENT).min_size(vec2(84.0, 28.0));
                        if ui.add(okb).clicked() {
                            ok = true;
                        }
                        if ui.add(egui::Button::new("Annuler").min_size(vec2(84.0, 28.0))).clicked() {
                            cancel = true;
                        }
                    });
                });
            });
        if reset {
            self.reset_tab();
        }
        self.capture_hotkey(ctx);
        if ok {
            self.apply_settings(ctx);
        }
        if !open || ok || cancel {
            self.settings_open = false;
            self.capturing_hotkey = None;
        }
    }

    fn apply_settings(&mut self, ctx: &egui::Context) {
        let s = &self.project.settings;
        let video = (self.draft.width, self.draft.height, self.draft.fps) != (s.width, s.height, s.fps);
        // Le mixeur, la transition et le mode studio ont pu changer pendant que la fenêtre était ouverte.
        let (audio, transition, studio) = (s.audio.clone(), s.transition.clone(), s.studio);
        let remote_changed = {
            let (a, b) = (&self.draft.remote, &s.remote);
            (a.enabled, a.port, &a.password, a.lan) != (b.enabled, b.port, &b.password, b.lan)
        };
        let hotkeys_changed = self.draft.hotkeys != s.hotkeys;
        let chat_changed = self.draft.chat != s.chat || self.draft.restream != s.restream;
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
        if chat_changed {
            self.chat.restart(&self.project.settings.chat, &self.project.settings.restream);
            self.live = Default::default();
        }
        if hotkeys_changed {
            let errors = self.apply_hotkeys();
            if !errors.is_empty() {
                self.notify(format!("Raccourcis refusés : {}", errors.join(", ")), true);
            }
        }
    }

    /* Onglets ------------------------------------------------------------- */

    fn tab_general(&mut self, ui: &mut egui::Ui) {
        section(ui, "Mises à jour");
        ui.checkbox(&mut self.draft.auto_update, "Installer automatiquement les mises à jour");
        hint(ui, "MyStream vérifie au démarrage puis toutes les 30 minutes. La mise à jour se télécharge en arrière-plan, jamais pendant un live ou un enregistrement, et s'applique au prochain lancement.");
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("Version installée : {}", super::version_label())).color(theme::MUTED));
            if ui.button("Rechercher maintenant").clicked() {
                self.check_updates(true);
            }
        });
        if let Some(text) = self.update_message() {
            hint(ui, &text);
        }
        section(ui, "Version Electron");
        hint(ui, "Reprend le chat (Twitch et Kick en direct, ou Restream) et les réglages Restream de MyStream (version Electron). Validez ensuite avec OK.");
        if ui.button("Reprendre les réglages de la version Electron").clicked() {
            match crate::model::electron_config() {
                Some(e) => {
                    crate::model::import_electron(&mut self.draft, &e["settings"]);
                    self.notify(format!("Réglages repris : chat « {} ». Cliquez sur OK pour les garder.", self.draft.chat.mode), false);
                }
                None => self.notify("Version Electron introuvable sur ce PC.", true),
            }
        }
    }

    fn tab_stream(&mut self, ui: &mut egui::Ui) {
        let d = &mut self.draft;
        section(ui, "Destination");
        grid(ui, "stream", |ui| {
            ui.label("Service");
            egui::ComboBox::from_id_salt("service")
                .width(300.0)
                .selected_text(if d.stream.service == "restream" { "Restream (votre application)" } else { "Serveur RTMP personnalisé" })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut d.stream.service, "restream".to_string(), "Restream (votre application)");
                    ui.selectable_value(&mut d.stream.service, "custom".to_string(), "Serveur RTMP personnalisé");
                });
            ui.end_row();
            if d.stream.service != "restream" {
                ui.label("Serveur");
                ui.add(egui::TextEdit::singleline(&mut d.stream.server).hint_text("rtmp://live.twitch.tv/app").desired_width(300.0));
                ui.end_row();
                ui.label("Clé de stream");
                ui.add(egui::TextEdit::singleline(&mut d.stream.key).password(true).desired_width(300.0));
                ui.end_row();
            }
        });
        if d.stream.service == "restream" {
            hint(ui, "Le live part vers votre application Restream (onglet Restream), qui le relaie vers Twitch et Kick sans le réencoder.");
        }
        section(ui, "Qualité");
        grid(ui, "stream-q", |ui| {
            ui.label("Encodeur vidéo");
            egui::ComboBox::from_id_salt("enc").width(300.0).selected_text(output::encoder_label(&d.stream.encoder)).show_ui(ui, |ui| {
                ui.selectable_value(&mut d.stream.encoder, "auto".to_string(), output::encoder_label("auto"));
                for e in output::encoders() {
                    ui.selectable_value(&mut d.stream.encoder, e.clone(), output::encoder_label(e));
                }
            });
            ui.end_row();
            ui.label("Débit vidéo");
            ui.add(egui::DragValue::new(&mut d.stream.video_bitrate).range(500..=50000).speed(50).suffix(" kb/s"));
            ui.end_row();
            ui.label("Débit audio");
            egui::ComboBox::from_id_salt("ab").selected_text(format!("{} kb/s", d.stream.audio_bitrate)).show_ui(ui, |ui| {
                for b in [96, 128, 160, 192, 256, 320] {
                    ui.selectable_value(&mut d.stream.audio_bitrate, b, format!("{b} kb/s"));
                }
            });
            ui.end_row();
            ui.label("Préréglage x264");
            preset_combo(ui, "preset-stream", &mut d.stream.preset);
            ui.end_row();
        });
        hint(ui, "Twitch accepte jusqu'à 6 000 kb/s (8 000 pour les partenaires), Kick jusqu'à 8 000 kb/s.");
    }

    fn tab_restream(&mut self, ui: &mut egui::Ui) {
        let d = &mut self.draft;
        hint(ui, "Votre application Restream (BloopStudio) : MyStream lui envoie le live, lit le chat et gère titre, catégorie et spectateurs.");
        grid(ui, "restream", |ui| {
            let r = &mut d.restream;
            ui.label("Adresse de l'interface");
            ui.add(egui::TextEdit::singleline(&mut r.url).hint_text("http://localhost:3050").desired_width(300.0));
            ui.end_row();
            ui.label("Mot de passe (si défini)");
            ui.add(egui::TextEdit::singleline(&mut r.password).password(true).desired_width(300.0));
            ui.end_row();
            ui.label("Adresse RTMP");
            ui.add(egui::TextEdit::singleline(&mut r.rtmp_url).hint_text("seulement si différente de l'interface").desired_width(300.0));
            ui.end_row();
            ui.label("");
            ui.checkbox(&mut r.allow_self_signed, "Accepter un certificat HTTPS auto-signé");
            ui.end_row();
        });
        let test = self.live.test.clone();
        ui.horizontal(|ui| {
            if ui.button("Tester la connexion").clicked() {
                let (cfg, test) = (d.restream.clone(), test.clone());
                *test.lock() = Some(("Test en cours…".into(), false));
                std::thread::spawn(move || {
                    let r = restream::ingest_url(&cfg).and_then(|(url, _)| restream::check_rtmp(&url).map(|_| url));
                    *test.lock() = Some(match r {
                        Ok(url) => (format!("Connexion réussie : le live partira vers {}", url.rsplit_once('/').map(|x| x.0).unwrap_or(&url)), false),
                        Err(e) => (e, true),
                    });
                });
            }
        });
        if let Some((m, err)) = test.lock().clone() {
            ui.label(RichText::new(m).size(12.0).color(if err { theme::DANGER } else { theme::SUCCESS }));
        }
    }

    fn tab_chat(&mut self, ui: &mut egui::Ui) {
        let c = &mut self.draft.chat;
        if c.mode == "twitch" {
            c.mode = "direct".into(); // ancien réglage « Twitch en direct »
        }
        grid(ui, "chat", |ui| {
            ui.label("Source du chat");
            let label = |m: &str| match m {
                "restream" => "Restream (Twitch + Kick, avec badges)",
                "direct" => "Direct (Twitch + Kick, sans compte)",
                _ => "Désactivé",
            };
            egui::ComboBox::from_id_salt("chatmode").width(300.0).selected_text(label(&c.mode)).show_ui(ui, |ui| {
                for m in ["off", "restream", "direct"] {
                    ui.selectable_value(&mut c.mode, m.to_string(), label(m));
                }
            });
            ui.end_row();
            if c.mode == "direct" {
                ui.label("Chaîne Twitch");
                ui.add(egui::TextEdit::singleline(&mut c.twitch_channel).hint_text("nom de la chaîne").desired_width(300.0));
                ui.end_row();
                ui.label("Chaîne Kick");
                ui.add(egui::TextEdit::singleline(&mut c.kick_channel).hint_text("nom de la chaîne").desired_width(300.0));
                ui.end_row();
            }
        });
        match c.mode.as_str() {
            "restream" => hint(ui, "Restream regroupe les chats de Twitch et de Kick via leurs API officielles (badges et emotes inclus). Pour Kick, Restream doit être joignable en HTTPS (voir sa documentation)."),
            "direct" => hint(ui, "Kick : MyStream lit la page de chat officielle de votre chaîne (celle que Kick fournit pour OBS), avec ses badges et emotes. Aucun compte n'est nécessaire."),
            _ => {}
        }
    }

    fn tab_video(&mut self, ui: &mut egui::Ui) {
        let busy = !self.shared.outputs.lock().is_empty();
        let d = &mut self.draft;
        ui.add_enabled_ui(!busy, |ui| {
            grid(ui, "video", |ui| {
                ui.label("Résolution de base (canevas)");
                egui::ComboBox::from_id_salt("res").selected_text(format!("{} × {}", d.width, d.height)).show_ui(ui, |ui| {
                    for (w, h) in RESOLUTIONS {
                        if ui.selectable_label(d.width == w && d.height == h, format!("{w} × {h}")).clicked() {
                            (d.width, d.height) = (w, h);
                        }
                    }
                });
                ui.end_row();
                ui.label("Images par seconde");
                egui::ComboBox::from_id_salt("fps").selected_text(d.fps.to_string()).show_ui(ui, |ui| {
                    for f in [24, 25, 30, 48, 50, 60] {
                        ui.selectable_value(&mut d.fps, f, f.to_string());
                    }
                });
                ui.end_row();
            });
        });
        if busy {
            hint(ui, "Arrêtez le streaming et l'enregistrement pour changer la vidéo.");
        }
        hint(ui, "Les captures sont limitées à cette cadence, même si le jeu tourne plus vite.");
    }

    fn tab_record(&mut self, ui: &mut egui::Ui) {
        let d = &mut self.draft;
        grid(ui, "rec", |ui| {
            ui.label("Dossier");
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut d.record.folder).desired_width(230.0));
                if ui.button("Parcourir…").clicked()
                    && let Some(p) = rfd::FileDialog::new().set_directory(&d.record.folder).pick_folder()
                {
                    d.record.folder = p.to_string_lossy().into_owned();
                }
            });
            ui.end_row();
            ui.label("Format");
            let label = |f: &str| match f {
                "mp4" => "MP4 (H.264 / AAC)",
                "webm" => "WebM (VP9 / Opus), encodé par le processeur",
                _ => "MKV (H.264 / AAC), résiste aux coupures",
            };
            egui::ComboBox::from_id_salt("fmt").width(300.0).selected_text(label(&d.record.format)).show_ui(ui, |ui| {
                for f in ["mkv", "mp4", "webm"] {
                    ui.selectable_value(&mut d.record.format, f.to_string(), label(f));
                }
            });
            ui.end_row();
            ui.label("Débit vidéo");
            ui.add(egui::DragValue::new(&mut d.record.video_bitrate).range(2000..=150000).speed(100).suffix(" kb/s"));
            ui.end_row();
            ui.label("Débit audio");
            egui::ComboBox::from_id_salt("rab").selected_text(format!("{} kb/s", d.record.audio_bitrate)).show_ui(ui, |ui| {
                for b in [128, 160, 192, 256, 320] {
                    ui.selectable_value(&mut d.record.audio_bitrate, b, format!("{b} kb/s"));
                }
            });
            ui.end_row();
            ui.label("Préréglage x264");
            preset_combo(ui, "preset-rec", &mut d.record.preset);
            ui.end_row();
        });
        hint(ui, "MKV est recommandé : le fichier reste lisible même si le PC s'arrête pendant l'enregistrement.");
    }

    fn tab_streamdeck(&mut self, ui: &mut egui::Ui) {
        let clients = self.remote.as_ref().map(|r| r.clients()).unwrap_or(0);
        let running = self.remote.is_some();
        let d = &mut self.draft.remote;
        hint(ui, "Télécommande compatible obs-websocket 5 : plugin Stream Deck MyStream, plugins OBS du Stream Deck, Touch Portal, Companion…");
        ui.checkbox(&mut d.enabled, "Activer la télécommande");
        ui.add_enabled_ui(d.enabled, |ui| {
            grid(ui, "remote", |ui| {
                ui.label("Port");
                ui.add(egui::DragValue::new(&mut d.port).range(1024..=65535));
                ui.end_row();
                ui.label("Mot de passe");
                ui.horizontal(|ui| {
                    let shown = self.show_password;
                    ui.add(egui::TextEdit::singleline(&mut d.password).password(!shown).desired_width(170.0));
                    if ui.button(if shown { "Masquer" } else { "Afficher" }).clicked() {
                        self.show_password = !shown;
                    }
                    if ui.button("Copier").clicked() {
                        ui.ctx().copy_text(d.password.clone());
                    }
                    if ui.button("Nouveau").on_hover_text("Génère un nouveau mot de passe (à recopier dans le Stream Deck)").clicked() {
                        d.password = crate::remote::random_token(16);
                    }
                });
                ui.end_row();
                ui.label("");
                ui.checkbox(&mut d.lan, "Accepter les autres appareils du réseau local (téléphone, autre PC)");
                ui.end_row();
                ui.label("État");
                let (text, color) = match (running, clients) {
                    (false, _) => ("Télécommande arrêtée".to_string(), theme::MUTED),
                    (true, 0) => ("En attente : aucun appareil connecté".to_string(), theme::WARN),
                    (true, n) => (format!("{n} appareil(s) connecté(s)"), theme::SUCCESS),
                };
                ui.label(RichText::new(text).color(color));
                ui.end_row();
            });
        });
        section(ui, "Plugin Stream Deck MyStream");
        hint(ui, "Dans le logiciel Stream Deck, réglages d'une touche MyStream > Connexion : adresse 127.0.0.1, ce port et ce mot de passe.");
        if ui.link(RichText::new("Télécharger le plugin Stream Deck MyStream").color(theme::ACCENT)).clicked() {
            super::open_path("https://github.com/TheDEMON78/mystream/raw/streamdeck-plugin/MyStream.streamDeckPlugin");
        }
    }

    fn tab_hotkeys(&mut self, ui: &mut egui::Ui) {
        hint(ui, "Fonctionnent même en jeu. Cliquez sur une case puis appuyez sur la combinaison voulue (Échap pour annuler).");
        ui.add_space(4.0);
        grid(ui, "hotkeys", |ui| {
            for (action, label) in HOTKEY_ACTIONS {
                ui.label(label);
                ui.horizontal(|ui| {
                    let current = self.draft.hotkeys.get(action).cloned().unwrap_or_default();
                    let capturing = self.capturing_hotkey.as_deref() == Some(action);
                    let text = if capturing {
                        RichText::new("Appuyez sur les touches…").color(theme::WARN)
                    } else if current.trim().is_empty() {
                        RichText::new("Aucun").color(theme::MUTED)
                    } else {
                        RichText::new(pretty_hotkey(&current)).color(theme::TEXT)
                    };
                    let b = egui::Button::new(text).min_size(vec2(200.0, 26.0)).fill(if capturing { theme::PANEL2 } else { theme::BG });
                    if ui.add(b).clicked() {
                        self.capturing_hotkey = Some(action.to_string());
                    }
                    if !current.is_empty() && ui.small_button("✕").on_hover_text("Retirer le raccourci").clicked() {
                        self.draft.hotkeys.insert(action.to_string(), String::new());
                    }
                });
                ui.end_row();
            }
        });
    }

    /// Enregistre la combinaison tapée pour le raccourci en cours de saisie.
    fn capture_hotkey(&mut self, ctx: &egui::Context) {
        let Some(action) = self.capturing_hotkey.clone() else { return };
        let pressed = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Key { key, pressed: true, modifiers, .. } => Some((*key, *modifiers)),
                _ => None,
            })
        });
        let Some((key, m)) = pressed else { return };
        if key == egui::Key::Escape {
            self.capturing_hotkey = None;
            return;
        }
        let Some(code) = key_code(key) else { return };
        let mut parts = Vec::new();
        if m.ctrl {
            parts.push("ctrl".to_string());
        }
        if m.shift {
            parts.push("shift".to_string());
        }
        if m.alt {
            parts.push("alt".to_string());
        }
        parts.push(code);
        self.draft.hotkeys.insert(action, parts.join("+"));
        self.capturing_hotkey = None;
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn raccourci_lisible() {
        assert_eq!(super::pretty_hotkey("ctrl+shift+KeyS"), "Ctrl + Maj + S");
        assert_eq!(super::pretty_hotkey("alt+F9"), "Alt + F9");
        assert_eq!(super::pretty_hotkey("ctrl+Digit1"), "Ctrl + 1");
    }
}
