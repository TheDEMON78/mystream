//! Apparence : mêmes couleurs, polices et docks que la version Electron de MyStream.

use egui::{Color32, FontData, FontDefinitions, FontFamily, FontId, Frame, Margin, RichText, Stroke, TextStyle, Vec2, vec2};
use std::sync::Arc;

pub const BG: Color32 = Color32::from_rgb(0x17, 0x18, 0x1d);
pub const PANEL: Color32 = Color32::from_rgb(0x1f, 0x20, 0x27);
pub const PANEL2: Color32 = Color32::from_rgb(0x27, 0x29, 0x34);
pub const BORDER: Color32 = Color32::from_rgb(0x33, 0x36, 0x4a);
pub const TEXT: Color32 = Color32::from_rgb(0xe6, 0xe8, 0xef);
pub const MUTED: Color32 = Color32::from_rgb(0x9a, 0xa0, 0xb4);
pub const ACCENT: Color32 = Color32::from_rgb(0x4c, 0x7d, 0xff);
pub const ACCENT2: Color32 = Color32::from_rgb(0x3a, 0x64, 0xd6);
pub const DANGER: Color32 = Color32::from_rgb(0xff, 0x4d, 0x5a);
pub const SUCCESS: Color32 = Color32::from_rgb(0x33, 0xc2, 0x7f);
pub const WARN: Color32 = Color32::from_rgb(0xf5, 0xa5, 0x24);

/// Famille des titres en demi-gras.
pub fn bold() -> FontFamily {
    FontFamily::Name("gras".into())
}

/// Texte en demi-gras.
pub fn title(text: impl Into<String>) -> RichText {
    RichText::new(text).family(bold()).color(TEXT)
}

fn system_font(names: &[&str]) -> Option<Vec<u8>> {
    let dirs: Vec<std::path::PathBuf> = if cfg!(windows) {
        vec![std::env::var("WINDIR").map(|w| std::path::PathBuf::from(w).join("Fonts")).unwrap_or_else(|_| "C:\\Windows\\Fonts".into())]
    } else {
        vec!["/usr/share/fonts/truetype/dejavu".into()]
    };
    dirs.iter().flat_map(|d| names.iter().map(move |n| d.join(n))).find_map(|p| std::fs::read(p).ok())
}

pub fn apply(ctx: &egui::Context) {
    // Polices : Segoe UI sous Windows (celle de la version Electron), sinon la police intégrée.
    let mut fonts = FontDefinitions::default();
    let regular = system_font(&["segoeui.ttf"]);
    let semibold = system_font(&["seguisb.ttf", "segoeuib.ttf"]);
    if let Some(r) = regular {
        fonts.font_data.insert("segoe".into(), Arc::new(FontData::from_owned(r)));
        fonts.families.entry(FontFamily::Proportional).or_default().insert(0, "segoe".into());
    }
    let mut gras = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    if let Some(b) = semibold {
        fonts.font_data.insert("segoe-gras".into(), Arc::new(FontData::from_owned(b)));
        gras.insert(0, "segoe-gras".into());
    }
    fonts.families.insert(bold(), gras);
    ctx.set_fonts(fonts);

    let mut style = (*ctx.global_style()).clone();
    style.text_styles = [
        (TextStyle::Small, FontId::proportional(11.5)),
        (TextStyle::Body, FontId::proportional(13.5)),
        (TextStyle::Button, FontId::proportional(13.5)),
        (TextStyle::Heading, FontId::new(15.0, bold())),
        (TextStyle::Monospace, FontId::monospace(12.5)),
    ]
    .into();
    style.spacing.item_spacing = vec2(6.0, 5.0);
    style.spacing.button_padding = vec2(10.0, 5.0);
    style.spacing.interact_size = vec2(32.0, 26.0);
    style.spacing.combo_height = 300.0;

    let v = &mut style.visuals;
    *v = egui::Visuals::dark();
    v.override_text_color = Some(TEXT);
    v.weak_text_color = Some(MUTED);
    v.panel_fill = BG;
    v.window_fill = PANEL;
    v.window_stroke = Stroke::new(1.0, BORDER);
    v.window_corner_radius = 8.into();
    v.menu_corner_radius = 6.into();
    v.extreme_bg_color = PANEL2; // champs de saisie
    v.faint_bg_color = PANEL2;
    v.code_bg_color = PANEL2;
    v.hyperlink_color = ACCENT;
    v.warn_fg_color = WARN;
    v.error_fg_color = DANGER;
    v.selection.bg_fill = ACCENT2;
    v.selection.stroke = Stroke::new(1.0, TEXT);
    v.slider_trailing_fill = true;
    v.striped = false;
    let w = &mut v.widgets;
    for (wv, fill, stroke) in [
        (&mut w.noninteractive, PANEL, BORDER),
        (&mut w.inactive, PANEL2, BORDER),
        (&mut w.hovered, Color32::from_rgb(0x30, 0x33, 0x42), Color32::from_rgb(0x46, 0x4a, 0x63)),
        (&mut w.active, Color32::from_rgb(0x36, 0x39, 0x4a), ACCENT),
        (&mut w.open, PANEL2, BORDER),
    ] {
        wv.bg_fill = fill;
        wv.weak_bg_fill = fill;
        wv.bg_stroke = Stroke::new(1.0, stroke);
        wv.corner_radius = 6.into();
        wv.fg_stroke = Stroke::new(1.0, TEXT);
        wv.expansion = 0.0;
    }
    w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    w.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    ctx.set_global_style(style);
}

/// Partie d'un dock à dessiner.
#[derive(PartialEq, Eq, Clone, Copy)]
pub enum Part {
    Body,
    Footer,
}

/// Dock comme ceux de la version Electron : cadre arrondi, titre, contenu
/// défilant et, s'il y en a une, barre d'outils en bas. `content` est appelé
/// pour le contenu, puis pour la barre d'outils.
pub fn dock(ui: &mut egui::Ui, name: &str, footer: bool, mut content: impl FnMut(&mut egui::Ui, Part)) {
    let size = (ui.available_size() - Vec2::splat(2.0)).max(Vec2::ZERO);
    Frame::new().fill(PANEL).stroke(Stroke::new(1.0, BORDER)).corner_radius(6).show(ui, |ui| {
        ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| dock_inner(ui, name, footer, size, &mut content));
    });
}

fn dock_inner(ui: &mut egui::Ui, name: &str, footer: bool, size: Vec2, content: &mut impl FnMut(&mut egui::Ui, Part)) {
    {
        ui.set_min_size(size);
        ui.set_max_size(size);
        ui.spacing_mut().item_spacing.y = 0.0;
        Frame::new().inner_margin(Margin::symmetric(10, 8)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(title(name));
        });
        line(ui);
        let footer_h = if footer { 34.0 } else { 0.0 };
        let body_h = (ui.available_height() - footer_h).max(10.0);
        ui.allocate_ui_with_layout(vec2(ui.available_width(), body_h), egui::Layout::top_down(egui::Align::Min), |ui| {
            ui.spacing_mut().item_spacing.y = 3.0;
            egui::ScrollArea::vertical().id_salt(name).auto_shrink(false).show(ui, |ui| {
                Frame::new().inner_margin(Margin::same(6)).show(ui, |ui| content(ui, Part::Body));
            });
        });
        if footer {
            line(ui);
            ui.allocate_ui(vec2(ui.available_width(), footer_h - 2.0), |ui| {
                ui.horizontal_centered(|ui| {
                    ui.add_space(6.0);
                    ui.spacing_mut().item_spacing.x = 2.0;
                    content(ui, Part::Footer);
                });
            });
        }
    }
}

/// Trait horizontal sur toute la largeur.
pub fn line(ui: &mut egui::Ui) {
    let r = ui.available_rect_before_wrap();
    ui.painter().hline(r.x_range(), r.top(), Stroke::new(1.0, BORDER));
    ui.add_space(1.0);
}

/// Bouton d'icône sans cadre (barres d'outils des docks).
pub fn tool(ui: &mut egui::Ui, icon: &str, tip: &str) -> bool {
    ui.add(egui::Button::new(RichText::new(icon).size(14.0)).frame(false).min_size(vec2(28.0, 24.0))).on_hover_text(tip).clicked()
}

/// Bouton pleine largeur du dock « Commandes ».
pub fn command(ui: &mut egui::Ui, text: &str, fill: Option<Color32>) -> bool {
    let mut b = egui::Button::new(RichText::new(text).color(TEXT)).min_size(vec2(ui.available_width(), 30.0));
    if let Some(f) = fill {
        b = b.fill(f);
    }
    let w = ui.available_width();
    ui.allocate_ui_with_layout(vec2(w, 30.0), egui::Layout::top_down(egui::Align::Center), |ui| ui.add(b).clicked()).inner
}

/// Pastille (LIVE / REC de la barre d'état).
pub fn pill(ui: &mut egui::Ui, text: &str, active: bool) {
    Frame::new()
        .fill(if active { DANGER } else { PANEL2 })
        .corner_radius(10)
        .inner_margin(Margin::symmetric(9, 2))
        .show(ui, |ui| ui.label(RichText::new(text).small().color(if active { Color32::WHITE } else { MUTED })));
}
