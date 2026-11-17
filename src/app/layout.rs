//! Disposition des docks comme la version Electron : rangée du bas et colonne
//! de droite, docks masquables et déplaçables, dispositions enregistrées par nom.

use super::App;
use crate::model::{DOCKS, Layout};
use crate::theme;
use egui::{RichText, vec2};

/// Largeur relative de chaque dock (mêmes proportions que la version Electron).
fn weight(dock: &str) -> f32 {
    match dock {
        "scenes" => 1.0,
        "sources" => 1.3,
        "mixer" => 2.0,
        "transitions" => 0.9,
        "chat" => 1.7,
        _ => 1.0,
    }
}

impl App {
    fn layout(&self) -> &Layout {
        &self.project.settings.ui.layout
    }

    /// Docks visibles d'une zone.
    pub(super) fn zone(&self, side: bool) -> Vec<String> {
        let l = self.layout();
        let list = if side { &l.side } else { &l.bottom };
        list.iter().filter(|d| !l.hidden.contains(d)).cloned().collect()
    }

    fn show_dock(&mut self, ui: &mut egui::Ui, dock: &str) {
        match dock {
            "scenes" => self.scenes_dock(ui),
            "sources" => self.sources_dock(ui),
            "mixer" => self.mixer_dock(ui),
            "transitions" => self.transitions_dock(ui),
            "controls" => self.commands_dock(ui),
            "chat" => self.chat_dock(ui),
            "liveinfo" => self.live_dock(ui),
            _ => {}
        }
    }

    /// Rangée du bas.
    pub(super) fn docks(&mut self, ui: &mut egui::Ui) {
        let docks = self.zone(false);
        let h = ui.available_height().max(0.0);
        let gap = 6.0;
        let total: f32 = docks.iter().map(|d| weight(d)).sum::<f32>().max(0.001);
        let free = (ui.available_width() - gap * (docks.len().saturating_sub(1)) as f32).max(0.0);
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for d in &docks {
                let w = (free * weight(d) / total).max(120.0);
                ui.allocate_ui_with_layout(vec2(w, h), egui::Layout::top_down(egui::Align::Min), |ui| self.show_dock(ui, d));
            }
        });
    }

    /// Colonne de droite.
    pub(super) fn right_column(&mut self, ui: &mut egui::Ui) {
        let docks = self.zone(true);
        let w = ui.available_width();
        let gap = 4.0;
        let total: f32 = docks.iter().map(|d| weight(d)).sum::<f32>().max(0.001);
        let free = (ui.available_height() - gap * (docks.len().saturating_sub(1)) as f32).max(0.0);
        for (i, d) in docks.iter().enumerate() {
            if i > 0 {
                ui.add_space(gap);
            }
            let h = (free * weight(d) / total).max(80.0);
            ui.allocate_ui_with_layout(vec2(w, h), egui::Layout::top_down(egui::Align::Min), |ui| self.show_dock(ui, d));
        }
    }

    /// Menu Affichage > Docks.
    pub(super) fn docks_menu(&mut self, ui: &mut egui::Ui) {
        let mut changed = false;
        {
            let l = &mut self.project.settings.ui.layout;
            for (k, name) in DOCKS {
                let shown = !l.hidden.iter().any(|d| d == k);
                let side = l.side.iter().any(|d| d == k);
                ui.menu_button(format!("{}  {name}", if shown { "✔" } else { "   " }), |ui| {
                    if ui.button(if shown { "Masquer" } else { "Afficher" }).clicked() {
                        if shown {
                            l.hidden.push(k.into());
                        } else {
                            l.hidden.retain(|d| d != k);
                        }
                        changed = true;
                    }
                    if ui.button(if side { "Mettre dans la rangée du bas" } else { "Mettre dans la colonne de droite" }).clicked() {
                        l.bottom.retain(|d| d != k);
                        l.side.retain(|d| d != k);
                        if side { l.bottom.push(k.into()) } else { l.side.push(k.into()) }
                        changed = true;
                    }
                    let list = if side { &mut l.side } else { &mut l.bottom };
                    if let Some(i) = list.iter().position(|d| d == k) {
                        let (before, after) = if side { ("Monter", "Descendre") } else { ("Vers la gauche", "Vers la droite") };
                        if ui.add_enabled(i > 0, egui::Button::new(before)).clicked() {
                            list.swap(i, i - 1);
                            changed = true;
                        }
                        if ui.add_enabled(i + 1 < list.len(), egui::Button::new(after)).clicked() {
                            list.swap(i, i + 1);
                            changed = true;
                        }
                    }
                });
            }
        }
        ui.separator();
        let names: Vec<String> = self.project.settings.ui.layouts.keys().cloned().collect();
        for n in &names {
            if ui.button(format!("Disposition « {n} »")).clicked() {
                if let Some(l) = self.project.settings.ui.layouts.get(n).cloned() {
                    self.project.settings.ui.layout = l.normalized();
                    changed = true;
                }
                ui.close();
            }
        }
        if ui.button("Enregistrer la disposition…").clicked() {
            self.saving_layout = Some(String::new());
            ui.close();
        }
        ui.add_enabled_ui(!names.is_empty(), |ui| {
            ui.menu_button("Supprimer une disposition", |ui| {
                for n in &names {
                    if ui.button(n).clicked() {
                        self.project.settings.ui.layouts.remove(n);
                        changed = true;
                        ui.close();
                    }
                }
            });
        });
        if ui.button("Disposition d'origine").clicked() {
            self.project.settings.ui.layout = Layout::default();
            changed = true;
            ui.close();
        }
        if changed {
            self.touch();
        }
    }

    /// Fenêtre « Enregistrer la disposition ».
    pub(super) fn save_layout_window(&mut self, ctx: &egui::Context) {
        let Some(name) = &mut self.saving_layout else { return };
        let mut open = true;
        let (mut ok, mut cancel) = (false, false);
        egui::Window::new("Enregistrer la disposition").open(&mut open).resizable(false).collapsible(false).show(ctx, |ui| {
            ui.label(RichText::new("Nom (ex. Live jeu, Discussion)").color(theme::MUTED));
            let r = ui.add(egui::TextEdit::singleline(name).desired_width(260.0));
            r.request_focus();
            ok = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.horizontal(|ui| {
                ok |= ui.button("Enregistrer").clicked();
                cancel = ui.button("Annuler").clicked();
            });
        });
        if ok {
            let n = name.trim().to_string();
            if !n.is_empty() {
                let l = self.project.settings.ui.layout.clone();
                self.project.settings.ui.layouts.insert(n.clone(), l);
                self.touch();
                self.notify(format!("Disposition « {n} » enregistrée."), false);
            }
        }
        if ok || cancel || !open {
            self.saving_layout = None;
        }
    }
}
