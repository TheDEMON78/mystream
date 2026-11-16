//! Docks « Chat » et « Titre et catégorie » (Restream), réglages associés,
//! et démarrage des sorties sans bloquer l'interface.

use super::App;
use crate::chat::{Message, Part as ChatPart, badge_svg, platform_color, platform_name, platform_svg};
use crate::output::{Kind, Output};
use crate::restream::{self, PlatformInfo};
use crate::theme::{self, Part};
use crossbeam_channel::Receiver;
use egui::{Color32, RichText, vec2};
use parking_lot::Mutex;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Images des emotes et des badges, téléchargées une fois (certains serveurs refusent le chargeur d'images d'egui).
fn emote_bytes(url: &str) -> Option<Arc<[u8]>> {
    use std::collections::HashMap;
    use std::sync::OnceLock;
    static CACHE: OnceLock<Mutex<HashMap<String, Option<Arc<[u8]>>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(v) = cache.lock().get(url) {
        return v.clone();
    }
    cache.lock().insert(url.to_string(), None);
    let url = url.to_string();
    std::thread::spawn(move || {
        let bytes = ureq::get(&url).call().ok().and_then(|mut r| r.body_mut().with_config().limit(4 << 20).read_to_vec().ok());
        cache.lock().insert(url, bytes.map(Arc::from));
    });
    None
}

fn rgb(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

/// Image téléchargée (emote, badge) à la hauteur donnée.
fn web_image(ui: &mut egui::Ui, url: &str, h: f32, hover: &str) -> bool {
    match emote_bytes(url) {
        Some(b) => {
            ui.add(egui::Image::from_bytes(format!("bytes://{url}"), egui::load::Bytes::Shared(b)).fit_to_exact_size(vec2(h * 4.0, h)).max_height(h)).on_hover_text(hover);
            true
        }
        None => false,
    }
}

/// Image SVG intégrée (logo de plateforme, badge dessiné).
fn svg_image(ui: &mut egui::Ui, name: &str, svg: String, size: f32, hover: &str) {
    ui.add(egui::Image::from_bytes(format!("bytes://{name}.svg"), svg.into_bytes()).fit_to_exact_size(vec2(size, size))).on_hover_text(hover);
}

/// Une ligne du chat, comme dans la version Electron : logo, badges, pseudo, message.
fn chat_line(ui: &mut egui::Ui, m: &Message) {
    let text: String = m.plain_text();
    if m.divider {
        ui.add_space(4.0);
        ui.vertical_centered(|ui| ui.label(RichText::new(format!("— {text} —")).size(11.0).color(theme::MUTED)));
        ui.add_space(4.0);
        return;
    }
    if m.system {
        ui.label(RichText::new(format!("[{}] {text}", platform_name(&m.platform))).italics().color(theme::MUTED));
        return;
    }
    let mention = Color32::from_rgb(0xff, 0xb8, 0x4d);
    let frame = if m.mentioned {
        egui::Frame::new().fill(Color32::from_rgba_unmultiplied(255, 184, 77, 31)).inner_margin(egui::Margin { left: 6, right: 2, top: 2, bottom: 2 })
    } else {
        egui::Frame::new().inner_margin(egui::Margin::symmetric(0, 2))
    };
    let r = frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            if let Some(svg) = platform_svg(&m.platform) {
                svg_image(ui, &format!("platform-{}", m.platform), svg, 14.0, platform_name(&m.platform));
            }
            for b in &m.badges {
                let label = b.label();
                if let Some(url) = &b.image
                    && web_image(ui, url, 18.0, &label)
                {
                    continue;
                }
                if let Some(svg) = badge_svg(&b.kind) {
                    svg_image(ui, &format!("badge-{}", b.kind), svg, 18.0, &label);
                } else {
                    egui::Frame::new()
                        .fill(theme::ACCENT2)
                        .corner_radius(3)
                        .inner_margin(egui::Margin::symmetric(4, 0))
                        .show(ui, |ui| ui.label(RichText::new(&label).size(10.0).strong().color(Color32::WHITE)));
                }
            }
            let name = m.color.map(rgb).unwrap_or(theme::ACCENT);
            ui.label(RichText::new(&m.username).family(theme::bold()).color(name));
            for p in &m.parts {
                match p {
                    ChatPart::Text(t) => {
                        ui.label(t.as_str());
                    }
                    ChatPart::Emote { name, url } => {
                        if !web_image(ui, url, 24.0, name) {
                            ui.label(RichText::new(name).color(theme::MUTED));
                        }
                    }
                }
            }
        });
    });
    if m.mentioned {
        let rect = r.response.rect;
        ui.painter().rect_filled(egui::Rect::from_min_size(rect.min, vec2(3.0, rect.height())), 0.0, mention);
    }
}

/// État du dock « Titre et catégorie ».
#[derive(Default)]
pub struct LiveDock {
    info: Arc<Mutex<Option<Vec<(String, PlatformInfo)>>>>,
    fetched: Option<Instant>,
    title: String,
    category: String,
    filled: bool,
    suggestions: Arc<Mutex<Vec<String>>>,
    searched: String,
    message: Arc<Mutex<Option<(String, bool)>>>,
    /// Résultat du test de connexion (paramètres).
    pub test: Arc<Mutex<Option<(String, bool)>>>,
}

/// Démarrage en cours d'une sortie (connexion à Restream, lancement de FFmpeg).
pub struct Pending {
    pub kind: Kind,
    pub rx: Receiver<Result<Output, String>>,
}

impl App {
    pub(super) fn restream_used(&self) -> bool {
        let s = &self.project.settings;
        !s.restream.url.trim().is_empty() && (s.stream.service == "restream" || s.chat.mode == "restream")
    }

    /// Démarre une sortie dans un fil (la connexion à Restream peut prendre quelques secondes).
    pub(super) fn start_output(&mut self, kind: Kind) {
        if self.pending.iter().any(|p| p.kind == kind) {
            return;
        }
        let (tx, rx) = crossbeam_channel::bounded(100);
        self.mixer.subscribe(tx);
        let settings = self.project.settings.clone();
        let (w, h) = self.canvas;
        let (rtx, rrx) = crossbeam_channel::bounded(1);
        std::thread::spawn(move || {
            let _ = rtx.send(Output::start(kind, &settings, w, h, rx).map_err(|e| format!("{e:#}")));
        });
        self.pending.push(Pending { kind, rx: rrx });
    }

    pub(super) fn poll_outputs(&mut self) {
        let mut done = Vec::new();
        self.pending.retain(|p| match p.rx.try_recv() {
            Ok(r) => {
                done.push((p.kind, r));
                false
            }
            Err(crossbeam_channel::TryRecvError::Empty) => true,
            Err(_) => false,
        });
        for (kind, r) in done {
            match r {
                Ok(out) => {
                    self.shared.outputs.lock().insert(crate::engine::key(kind), out);
                    if kind == Kind::Stream {
                        self.session_started();
                    }
                }
                Err(e) => self.notify(format!("{} n'a pas pu démarrer : {e}", kind.label()), true),
            }
        }
    }

    /// Spectateurs de toutes les plateformes connectées (pour le pic du live).
    pub(super) fn live_viewers(&self) -> Option<u64> {
        let info = self.live.info.lock();
        let sum: u64 = info.as_ref()?.iter().filter(|(_, i)| i.connected).filter_map(|(_, i)| i.viewers).sum();
        (sum > 0).then_some(sum)
    }

    pub(super) fn starting(&self, kind: Kind) -> bool {
        self.pending.iter().any(|p| p.kind == kind)
    }

    /* Dock Chat ---------------------------------------------------------- */

    pub(super) fn chat_dock(&mut self, ui: &mut egui::Ui) {
        let mode = self.project.settings.chat.mode.clone();
        let mut configure = false;
        theme::dock(ui, "Chat", true, |ui, part| match part {
            Part::Body => {
                if mode == "off" {
                    if ui.link(RichText::new("Configurer le chat…").color(theme::ACCENT)).clicked() {
                        configure = true;
                    }
                    return;
                }
                // Pastilles d'état, comme la version Electron.
                let status = self.chat.status();
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                    let list = if status.is_empty() && mode == "restream" { vec![(mode.clone(), "connecting".to_string(), None)] } else { status.clone() };
                    for (p, st, err) in &list {
                        let (fill, color, word) = match st.as_str() {
                            "connected" => (Color32::from_rgba_unmultiplied(51, 194, 127, 46), theme::SUCCESS, "connecté"),
                            "error" => (Color32::from_rgba_unmultiplied(255, 77, 90, 46), theme::DANGER, "erreur"),
                            _ => (theme::PANEL2, theme::MUTED, "connexion…"),
                        };
                        let r = egui::Frame::new()
                            .fill(fill)
                            .corner_radius(10)
                            .inner_margin(egui::Margin::symmetric(8, 1))
                            .show(ui, |ui| ui.label(RichText::new(format!("{} : {word}", platform_name(p))).size(11.0).color(color)))
                            .response;
                        if let Some(e) = err {
                            r.on_hover_text(e);
                        }
                    }
                });
                for (p, st, err) in &status {
                    if st == "error"
                        && let Some(e) = err
                    {
                        ui.label(RichText::new(format!("{} : {e}", platform_name(p))).size(11.0).color(theme::DANGER));
                    }
                }
                theme::line(ui);
                let messages = self.chat.messages();
                if status.is_empty() && mode == "direct" {
                    ui.label(RichText::new("Indiquez vos chaînes Twitch et Kick (⚙ ci-dessous).").italics().color(theme::MUTED));
                } else if messages.is_empty() {
                    ui.label(RichText::new("Aucun message pour le moment.").italics().color(theme::MUTED));
                }
                egui::ScrollArea::vertical().id_salt("chat-msgs").stick_to_bottom(true).auto_shrink(false).show(ui, |ui| {
                    for m in &messages {
                        chat_line(ui, m);
                    }
                });
            }
            Part::Footer => {
                if theme::tool(ui, "🗑", "Effacer le chat") {
                    self.chat.clear();
                }
                if theme::tool(ui, "⚙", "Paramètres du chat") {
                    configure = true;
                }
            }
        });
        if configure {
            self.open_settings_tab(super::settings::Tab::Chat);
        }
    }

    /* Dock Titre et catégorie ----------------------------------------------- */

    fn refresh_live(&mut self) {
        self.live.fetched = Some(Instant::now());
        let (cfg, info) = (self.project.settings.restream.clone(), self.live.info.clone());
        std::thread::spawn(move || *info.lock() = Some(restream::live_info(&cfg)));
    }

    pub(super) fn live_dock(&mut self, ui: &mut egui::Ui) {
        let used = self.restream_used();
        if used && self.live.fetched.is_none_or(|t| t.elapsed() > Duration::from_secs(30)) {
            self.refresh_live();
        }
        theme::dock(ui, "Titre et catégorie", true, |ui, part| match part {
            Part::Body => {
                if !used {
                    ui.label(RichText::new("Reliez MyStream à votre application Restream (Paramètres > Restream) pour voir et modifier le titre, la catégorie et les spectateurs de vos chaînes.").italics().color(theme::MUTED));
                    return;
                }
                let info = self.live.info.lock().clone();
                let Some(info) = info else {
                    ui.label(RichText::new("Chargement…").color(theme::MUTED));
                    return;
                };
                if let Some(e) = info.iter().find_map(|(_, i)| i.error.clone()).filter(|_| info.iter().all(|(_, i)| i.error.is_some())) {
                    ui.label(RichText::new(e).color(theme::DANGER));
                    return;
                }
                for (p, i) in &info {
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(vec2(10.0, 14.0), egui::Sense::hover());
                        ui.painter().circle_filled(r.center(), 4.0, rgb(platform_color(p)));
                        ui.label(theme::title(platform_name(p)));
                        if i.connected {
                            if let Some(v) = i.viewers {
                                ui.label(RichText::new(format!("👁 {v}")).color(theme::TEXT));
                            }
                            ui.label(RichText::new(&i.category).small().color(theme::MUTED));
                        } else {
                            ui.label(RichText::new("non connecté dans Restream").small().color(theme::MUTED));
                        }
                    });
                }
                if !self.live.filled
                    && let Some((_, i)) = info.iter().find(|(_, i)| i.connected)
                {
                    self.live.title = i.title.clone();
                    self.live.category = i.category.clone();
                    self.live.filled = true;
                }
                ui.add_space(6.0);
                ui.label(RichText::new("Titre").color(theme::MUTED));
                ui.add(egui::TextEdit::singleline(&mut self.live.title).desired_width(f32::INFINITY));
                ui.label(RichText::new("Catégorie").color(theme::MUTED));
                let r = ui.add(egui::TextEdit::singleline(&mut self.live.category).desired_width(f32::INFINITY).hint_text("Tapez au moins 3 lettres"));
                // Suggestions de catégories, demandées à Restream pendant la saisie.
                let q = self.live.category.trim().to_string();
                if r.changed() && q.chars().count() >= 3 && q != self.live.searched {
                    self.live.searched = q.clone();
                    let (cfg, sug) = (self.project.settings.restream.clone(), self.live.suggestions.clone());
                    std::thread::spawn(move || *sug.lock() = restream::categories(&cfg, &q).unwrap_or_default());
                }
                if r.has_focus() || r.lost_focus() {
                    let list = self.live.suggestions.lock().clone();
                    for c in list.iter().take(6) {
                        if c != &self.live.category && ui.add(egui::Button::new(RichText::new(c).small()).frame(false)).clicked() {
                            self.live.category = c.clone();
                            self.live.suggestions.lock().clear();
                        }
                    }
                }
                ui.add_space(4.0);
                if ui
                    .add(egui::Button::new("Mettre à jour sur toutes les plateformes").fill(theme::ACCENT2).min_size(vec2(ui.available_width(), 28.0)))
                    .clicked()
                {
                    let (cfg, title, cat, msg, info) = (
                        self.project.settings.restream.clone(),
                        self.live.title.clone(),
                        self.live.category.clone(),
                        self.live.message.clone(),
                        self.live.info.clone(),
                    );
                    *msg.lock() = Some(("Mise à jour…".into(), false));
                    std::thread::spawn(move || {
                        *msg.lock() = Some(match restream::update_info(&cfg, &title, &cat) {
                            Ok(_) => ("Titre et catégorie mis à jour.".into(), false),
                            Err(e) => (e, true),
                        });
                        *info.lock() = Some(restream::live_info(&cfg));
                    });
                }
                if let Some((m, err)) = self.live.message.lock().clone() {
                    ui.label(RichText::new(m).small().color(if err { theme::DANGER } else { theme::SUCCESS }));
                }
            }
            Part::Footer => {
                if theme::tool(ui, "⟳", "Actualiser") {
                    self.live.filled = false;
                    self.refresh_live();
                }
            }
        });
    }
}
