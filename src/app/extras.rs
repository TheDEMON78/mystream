//! Compléments de l'interface repris de la version Electron : barre « En
//! direct », confirmation d'arrêt du live, assistant de configuration,
//! réduction dans la zone de notification, journaux et diagnostic.

use super::App;
use crate::output::Kind;
use crate::theme;
use egui::{Color32, RichText, vec2};

/// Plateformes proposées pour le streaming, avec leur serveur RTMP.
pub const SERVICES: [(&str, &str, &str); 6] = [
    ("restream", "Restream (votre application)", ""),
    ("twitch", "Twitch", "rtmp://live.twitch.tv/app"),
    ("youtube", "YouTube", "rtmp://a.rtmp.youtube.com/live2"),
    ("kick", "Kick", "rtmps://fa723fc1b171.global-contribute.live-video.net:443/app"),
    ("facebook", "Facebook Live", "rtmps://live-api-s.facebook.com:443/rtmp"),
    ("custom", "Serveur RTMP personnalisé", ""),
];

pub fn service_label(id: &str) -> &'static str {
    SERVICES.iter().find(|(k, _, _)| *k == id).map(|(_, l, _)| *l).unwrap_or("Serveur RTMP personnalisé")
}

/// Choix de la plateforme : renseigne le serveur des plateformes connues.
pub fn service_combo(ui: &mut egui::Ui, id: &str, s: &mut crate::model::StreamSettings) {
    egui::ComboBox::from_id_salt(id).width(300.0).selected_text(service_label(&s.service)).show_ui(ui, |ui| {
        for (k, l, server) in SERVICES {
            if ui.selectable_label(s.service == k, l).clicked() {
                s.service = k.to_string();
                if !server.is_empty() {
                    s.server = server.to_string();
                }
            }
        }
    });
}

const QUALITIES: [(&str, u32, u32, u32, u32); 4] = [
    ("1080p 60 i/s (PC puissant, encodeur matériel)", 1920, 1080, 60, 6000),
    ("1080p 30 i/s (équilibré)", 1920, 1080, 30, 4500),
    ("720p 60 i/s (jeux rapides, connexion moyenne)", 1280, 720, 60, 4500),
    ("720p 30 i/s (PC modeste ou petite connexion)", 1280, 720, 30, 3000),
];

/// Réponses de l'assistant de configuration.
pub struct Wizard {
    stream: bool,
    service: String,
    key: String,
    restream_url: String,
    quality: usize,
    mic: String,
    add_screen: bool,
}

/// Icône de MyStream dans la zone de notification (fenêtre réduite).
pub struct Tray {
    #[cfg(windows)]
    _icon: tray_icon::TrayIcon,
    #[cfg(windows)]
    show: tray_icon::menu::MenuId,
    #[cfg(windows)]
    stream: tray_icon::menu::MenuId,
    #[cfg(windows)]
    quit: tray_icon::menu::MenuId,
}

#[cfg(windows)]
fn make_tray(live: bool) -> Option<Tray> {
    use tray_icon::menu::{Menu, MenuItem, PredefinedMenuItem};
    let img = image::load_from_memory(include_bytes!("../../assets/icon.png")).ok()?.resize(32, 32, image::imageops::FilterType::Lanczos3).to_rgba8();
    let icon = tray_icon::Icon::from_rgba(img.into_raw(), 32, 32).ok()?;
    let show = MenuItem::new("Afficher MyStream", true, None);
    let stream = MenuItem::new(if live { "Arrêter le streaming" } else { "Démarrer le streaming" }, true, None);
    let quit = MenuItem::new("Quitter", true, None);
    let menu = Menu::new();
    menu.append(&show).ok()?;
    menu.append(&stream).ok()?;
    menu.append(&PredefinedMenuItem::separator()).ok()?;
    menu.append(&quit).ok()?;
    let tray = tray_icon::TrayIconBuilder::new().with_icon(icon).with_tooltip("MyStream").with_menu(Box::new(menu)).with_menu_on_left_click(false).build().ok()?;
    Some(Tray { _icon: tray, show: show.id().clone(), stream: stream.id().clone(), quit: quit.id().clone() })
}

impl App {
    /// Zone de notification : la fenêtre réduite disparaît de la barre des tâches.
    pub(super) fn tray_tick(&mut self, ctx: &egui::Context, minimized: bool) {
        #[cfg(windows)]
        {
            if self.tray.is_none() && minimized && self.project.settings.minimize_to_tray {
                self.tray = make_tray(self.live(Kind::Stream));
                if self.tray.is_some() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                }
            }
            let Some(tray) = &self.tray else { return };
            let mut show = false;
            while let Ok(e) = tray_icon::TrayIconEvent::receiver().try_recv() {
                show |= matches!(e, tray_icon::TrayIconEvent::DoubleClick { .. } | tray_icon::TrayIconEvent::Click { button: tray_icon::MouseButton::Left, .. });
            }
            let (show_id, stream_id, quit_id) = (tray.show.clone(), tray.stream.clone(), tray.quit.clone());
            while let Ok(e) = tray_icon::menu::MenuEvent::receiver().try_recv() {
                if e.id == show_id {
                    show = true;
                } else if e.id == stream_id {
                    self.toggle_output(Kind::Stream);
                } else if e.id == quit_id {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
            if show {
                self.tray = None;
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
        }
        #[cfg(not(windows))]
        let _ = (ctx, minimized, &self.tray);
    }

    /// Barre « En direct » au-dessus de l'aperçu : durée, spectateurs, titre, débit, pertes.
    pub(super) fn live_bar(&mut self, ui: &mut egui::Ui) {
        let Some(elapsed) = self.live_elapsed() else { return };
        if !self.project.settings.ui.live_bar {
            return;
        }
        let rate = self.shared.outputs.lock().get(&crate::engine::key(Kind::Stream)).map(|o| o.stats.lock().bitrate_kbps).unwrap_or(0.0);
        let lost = self.live_lost();
        let frames = (elapsed.as_secs_f32() * self.project.settings.fps as f32).max(1.0);
        let platforms = self.live_platforms();
        egui::Frame::new().fill(theme::PANEL).stroke(egui::Stroke::new(1.0, theme::BORDER)).corner_radius(6).inner_margin(egui::Margin::symmetric(10, 5)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                theme::pill(ui, "LIVE", true);
                let s = elapsed.as_secs();
                ui.label(RichText::new(format!("{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)).family(theme::bold()).size(15.0));
                for (name, viewers, color) in &platforms {
                    ui.add_space(6.0);
                    let (r, _) = ui.allocate_exact_size(vec2(8.0, 8.0), egui::Sense::hover());
                    ui.painter().circle_filled(r.center(), 4.0, *color);
                    ui.label(RichText::new(viewers.map(|v| format!("{v}")).unwrap_or_else(|| "–".into())).strong()).on_hover_text(format!("Spectateurs {name}"));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let pct = lost as f32 * 100.0 / frames;
                    let color = if pct > 1.0 { theme::DANGER } else if lost > 0 { theme::WARN } else { theme::MUTED };
                    ui.label(RichText::new(format!("{lost} image(s) perdue(s) · {pct:.1} %")).color(color));
                    ui.label(RichText::new(format!("{:.0} kb/s", rate)).color(theme::MUTED));
                });
            });
        });
        ui.add_space(4.0);
    }

    /// « Arrêter le live ? »
    pub(super) fn confirm_stop_window(&mut self, ctx: &egui::Context) {
        if !self.confirm_stop {
            return;
        }
        let (mut stop, mut cancel) = (false, false);
        egui::Window::new("Arrêter le live ?").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, vec2(0.0, 0.0)).show(ctx, |ui| {
            ui.label("Le live s'arrêtera pour tous vos spectateurs.");
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                stop = ui.add(egui::Button::new(RichText::new("Arrêter le live").color(Color32::WHITE)).fill(theme::DANGER)).clicked();
                cancel = ui.button("Continuer le live").clicked();
            });
        });
        if stop {
            self.toggle_output(Kind::Stream);
        }
        if stop || cancel {
            self.confirm_stop = false;
        }
    }

    pub(super) fn open_wizard(&mut self) {
        let s = &self.project.settings;
        self.wizard = Some(Wizard {
            stream: true,
            service: s.stream.service.clone(),
            key: s.stream.key.clone(),
            restream_url: if s.restream.url.is_empty() { "http://localhost:3050".into() } else { s.restream.url.clone() },
            quality: if crate::output::encoders().first().is_some_and(|e| e != "libx264") { 0 } else { 2 },
            mic: s.audio.mic_device.clone(),
            add_screen: true,
        });
    }

    /// Assistant de configuration (premier lancement, ou menu Aide).
    pub(super) fn wizard_window(&mut self, ctx: &egui::Context) {
        if !self.project.settings.ui.wizard_done {
            self.project.settings.ui.wizard_done = true;
            self.open_wizard();
            self.touch();
        }
        let Some(w) = &mut self.wizard else { return };
        let (mut done, mut later) = (false, false);
        let mics = crate::audio::input_devices();
        egui::Window::new(RichText::new("Bienvenue dans MyStream").family(theme::bold()))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.set_width(460.0);
                ui.label(RichText::new("Quelques questions pour tout régler en une minute. Tout reste modifiable ensuite dans les Paramètres.").color(theme::MUTED));
                ui.add_space(8.0);
                egui::Grid::new("assistant").num_columns(2).spacing(vec2(12.0, 8.0)).show(ui, |ui| {
                    ui.label("Que souhaitez-vous faire ?");
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut w.stream, true, "Streamer (et enregistrer)");
                        ui.selectable_value(&mut w.stream, false, "Enregistrer uniquement");
                    });
                    ui.end_row();
                    if w.stream {
                        ui.label("Plateforme");
                        egui::ComboBox::from_id_salt("assistant-service").width(260.0).selected_text(service_label(&w.service)).show_ui(ui, |ui| {
                            for (k, l, _) in SERVICES {
                                ui.selectable_value(&mut w.service, k.to_string(), l);
                            }
                        });
                        ui.end_row();
                        if w.service == "restream" {
                            ui.label("Adresse de Restream");
                            ui.add(egui::TextEdit::singleline(&mut w.restream_url).desired_width(260.0));
                        } else {
                            ui.label("Clé de stream");
                            ui.add(egui::TextEdit::singleline(&mut w.key).password(true).desired_width(260.0));
                        }
                        ui.end_row();
                    }
                    ui.label("Qualité");
                    egui::ComboBox::from_id_salt("assistant-qualite").width(260.0).selected_text(QUALITIES[w.quality].0).show_ui(ui, |ui| {
                        for (i, q) in QUALITIES.iter().enumerate() {
                            ui.selectable_value(&mut w.quality, i, q.0);
                        }
                    });
                    ui.end_row();
                    ui.label("Micro");
                    let label = |m: &str| match m {
                        "" => "Micro par défaut".to_string(),
                        "off" => "Aucun micro".to_string(),
                        m => m.to_string(),
                    };
                    egui::ComboBox::from_id_salt("assistant-micro").width(260.0).selected_text(label(&w.mic)).show_ui(ui, |ui| {
                        ui.selectable_value(&mut w.mic, String::new(), label(""));
                        ui.selectable_value(&mut w.mic, "off".to_string(), label("off"));
                        for m in &mics {
                            ui.selectable_value(&mut w.mic, m.clone(), m);
                        }
                    });
                    ui.end_row();
                    ui.label("");
                    ui.checkbox(&mut w.add_screen, "Ajouter une capture de l'écran principal à la scène");
                    ui.end_row();
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    done = ui.add(egui::Button::new(RichText::new("Terminer").color(Color32::WHITE)).fill(theme::ACCENT)).clicked();
                    later = ui.button("Plus tard").clicked();
                });
            });
        if later {
            self.wizard = None;
        }
        if !done {
            return;
        }
        let Some(w) = self.wizard.take() else { return };
        let busy = !self.shared.outputs.lock().is_empty();
        let s = &mut self.project.settings;
        let (_, width, height, fps, rate) = QUALITIES[w.quality];
        if w.stream {
            s.stream.service = w.service.clone();
            if w.service == "restream" {
                s.restream.url = w.restream_url.trim().to_string();
            } else {
                s.stream.key = w.key.trim().to_string();
                if let Some((_, _, server)) = SERVICES.iter().find(|(k, _, _)| *k == w.service)
                    && !server.is_empty()
                {
                    s.stream.server = server.to_string();
                }
            }
        }
        s.stream.video_bitrate = rate;
        s.stream.encoder = "auto".into();
        s.audio.mic_device = w.mic;
        let video = !busy && (s.width, s.height, s.fps) != (width, height, fps);
        if video {
            (s.width, s.height, s.fps) = (width, height, fps);
            self.runtimes.clear();
            self.shared.sizes.lock().clear();
            self.start_engine(ctx);
        }
        if w.add_screen {
            self.add("Écran", crate::model::SourceKind::Display { monitor: 0, audio: false });
        }
        self.touch();
        self.notify("Configuration terminée. Bon stream !", false);
    }

    /// Texte du diagnostic de performance (Aide), à coller dans un message.
    pub(super) fn perf_diagnostic(&self) -> String {
        let st = self.shared.stats.lock().clone();
        let s = &self.project.settings;
        let mut out = format!(
            "MyStream {} — diagnostic de performance\nSystème : {} {}\nCarte graphique : {}\nVidéo : {} × {} à {} i/s, encodeur {}\nRendu : {:.1} i/s, {:.2} ms par image, {} image(s) manquée(s), {} perdue(s)\n",
            super::version_label(),
            std::env::consts::OS,
            std::env::consts::ARCH,
            self.render_state.adapter.get_info().name,
            s.width,
            s.height,
            s.fps,
            crate::output::encoder_label(&s.stream.encoder),
            st.fps,
            st.render_ms,
            st.lagged,
            st.dropped,
        );
        for (k, o) in self.shared.outputs.lock().iter() {
            let o = o.stats.lock();
            out += &format!("Sortie {}: {:.0} kb/s\n", if *k == 0 { "streaming" } else { "enregistrement" }, o.bitrate_kbps);
        }
        out += "Sources :\n";
        for src in &self.project.sources {
            let rt = self.runtimes.get(&src.id);
            let fps = rt.map(|r| *r.status.fps.lock()).unwrap_or(0.0);
            let state = match rt {
                None => "arrêtée (pas affichée)".to_string(),
                Some(r) => r.error().unwrap_or_else(|| format!("{fps:.0} i/s")),
            };
            out += &format!("  - {} ({}) : {state}\n", src.name, src.kind.label());
        }
        out
    }
}

/// Dossier des journaux de MyStream.
pub fn logs_dir() -> std::path::PathBuf {
    dirs::config_dir().unwrap_or_else(|| ".".into()).join("MyStream-natif").join("journaux")
}
