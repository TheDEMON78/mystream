//! Édition : annuler / rétablir, verrouillage et transformation des sources,
//! menu des scènes, export / import de la collection et sauvegardes.

use super::App;
use crate::model::{Scene, Source};
use crate::theme;
use egui::RichText;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Collection de scènes (export, sauvegardes, annuler).
#[derive(Serialize, Deserialize)]
pub struct Collection {
    pub scenes: Vec<Scene>,
    pub sources: Vec<Source>,
}

fn backups_dir() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| ".".into()).join("MyStream-natif").join("sauvegardes")
}

impl App {
    fn collection_json(&self) -> String {
        serde_json::to_string(&Collection { scenes: self.project.scenes.clone(), sources: self.project.sources.clone() }).unwrap_or_default()
    }

    fn apply_collection(&mut self, c: Collection) {
        if c.scenes.is_empty() {
            return;
        }
        self.project.scenes = c.scenes;
        self.project.sources = c.sources;
        let max = self
            .project
            .scenes
            .iter()
            .flat_map(|s| std::iter::once(s.id).chain(s.items.iter().map(|i| i.id)))
            .chain(self.project.sources.iter().map(|s| s.id))
            .max()
            .unwrap_or(0);
        self.project.next_id = self.project.next_id.max(max + 1);
        if self.project.scene(self.project.current).is_none() {
            self.project.current = self.project.scenes[0].id;
        }
        if self.project.scene(self.project.preview).is_none() {
            self.project.preview = self.project.current;
        }
        self.selected = None;
    }

    /// Au moment d'enregistrer : mémorise l'état précédent pour « Annuler ».
    pub(super) fn record_undo(&mut self) {
        let now = self.collection_json();
        if self.undo_base.is_empty() {
            self.undo_base = now;
            return;
        }
        if now != self.undo_base {
            let before = std::mem::replace(&mut self.undo_base, now);
            self.undo.push(before);
            if self.undo.len() > 50 {
                self.undo.remove(0);
            }
            self.redo.clear();
        }
        // Sauvegarde automatique toutes les 10 minutes (les 20 dernières).
        if self.last_backup.is_none_or(|t| t.elapsed() > Duration::from_secs(600)) {
            self.last_backup = Some(Instant::now());
            let dir = backups_dir();
            let _ = std::fs::create_dir_all(&dir);
            let name = format!(
                "{}.json",
                crate::output::local_date(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0))
            );
            let _ = std::fs::write(dir.join(name), &self.undo_base);
            if let Ok(rd) = std::fs::read_dir(&dir) {
                let mut files: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
                files.sort();
                while files.len() > 20 {
                    let _ = std::fs::remove_file(files.remove(0));
                }
            }
        }
    }

    pub(super) fn undo(&mut self) {
        let Some(prev) = self.undo.pop() else { return };
        let now = self.collection_json();
        self.redo.push(now);
        if let Ok(c) = serde_json::from_str(&prev) {
            self.apply_collection(c);
        }
        self.undo_base = prev;
        self.project.save();
    }

    pub(super) fn redo(&mut self) {
        let Some(next) = self.redo.pop() else { return };
        let now = self.collection_json();
        self.undo.push(now);
        if let Ok(c) = serde_json::from_str(&next) {
            self.apply_collection(c);
        }
        self.undo_base = next;
        self.project.save();
    }

    pub(super) fn can_undo(&self) -> (bool, bool) {
        (!self.undo.is_empty(), !self.redo.is_empty())
    }

    pub(super) fn export_collection(&mut self) {
        if let Some(path) = rfd::FileDialog::new().add_filter("Collection MyStream", &["json"]).set_file_name("Collection MyStream.json").save_file() {
            match std::fs::write(&path, self.collection_json()) {
                Ok(()) => self.notify(format!("Collection exportée : {}", path.display()), false),
                Err(e) => self.notify(format!("Export impossible : {e}"), true),
            }
        }
    }

    pub(super) fn import_collection(&mut self, backup: bool) {
        let mut dlg = rfd::FileDialog::new().add_filter("Collection MyStream", &["json"]);
        if backup {
            dlg = dlg.set_directory(backups_dir());
        }
        let Some(path) = dlg.pick_file() else { return };
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        match serde_json::from_str::<Collection>(&text) {
            Ok(c) => {
                self.record_undo();
                self.apply_collection(c);
                self.touch();
                self.notify("Collection chargée (Édition > Annuler pour revenir en arrière).", false);
            }
            // Fichier d'OBS choisi ici par erreur : on passe par l'import OBS.
            Err(_) if text.contains("\"scene_order\"") => {
                match crate::obs_import::import(&path, &mut self.project) {
                    Ok(r) => self.notify(format!("Importé depuis OBS : {} scène(s), {} source(s).", r.scenes, r.sources), false),
                    Err(e) => self.notify(format!("Import impossible : {e}"), true),
                }
                self.touch();
            }
            Err(e) => self.notify(format!("Collection illisible : {e}"), true),
        }
    }

    /// Menu clic droit d'une scène.
    pub(super) fn scene_menu(&mut self, ui: &mut egui::Ui, id: u64, name: &str) {
        if ui.button("Renommer…").clicked() {
            self.renaming = Some((id, name.to_string()));
            ui.close();
        }
        if ui.button("Passer à l'antenne").clicked() {
            self.project.current = id;
            self.touch();
            ui.close();
        }
        if ui.button("Dupliquer (sources partagées)").clicked() {
            if let Some(n) = self.project.duplicate_scene(id, false) {
                self.select_scene(n);
            }
            ui.close();
        }
        if ui.button("Dupliquer avec des sources indépendantes").clicked() {
            if let Some(n) = self.project.duplicate_scene(id, true) {
                self.select_scene(n);
            }
            ui.close();
        }
        ui.separator();
        if ui.add_enabled(self.project.scenes.len() > 1, egui::Button::new("Supprimer")).clicked() {
            self.project.remove_scene(id);
            self.touch();
            ui.close();
        }
    }

    pub(super) fn select_scene(&mut self, id: u64) {
        if self.project.settings.studio {
            self.project.preview = id;
        } else {
            self.project.current = id;
        }
        self.selected = None;
        self.touch();
    }

    /// Fenêtre « Modifier la transformation » : position, taille, rotation à 0.
    pub(super) fn transform_window(&mut self, ctx: &egui::Context) {
        if !self.transform_open {
            return;
        }
        let scene_id = self.project.edit_scene();
        let sizes = self.shared.sizes.lock().clone();
        let Some(sel) = self.selected else {
            self.transform_open = false;
            return;
        };
        let Some(item) = self.project.scene_mut(scene_id).and_then(|s| s.items.iter_mut().find(|i| i.id == sel)) else {
            self.transform_open = false;
            return;
        };
        let (w, h) = sizes.get(&item.source).copied().unwrap_or((0, 0));
        let mut open = true;
        let mut changed = false;
        egui::Window::new("Modifier la transformation").open(&mut open).resizable(false).collapsible(false).show(ctx, |ui| {
            egui::Grid::new("transform").num_columns(2).show(ui, |ui| {
                ui.label("Position X");
                changed |= ui.add(egui::DragValue::new(&mut item.x).speed(1.0).suffix(" px")).changed();
                ui.end_row();
                ui.label("Position Y");
                changed |= ui.add(egui::DragValue::new(&mut item.y).speed(1.0).suffix(" px")).changed();
                ui.end_row();
                ui.label("Échelle");
                let mut pct = item.scale.unwrap_or(1.0) * 100.0;
                if ui.add(egui::DragValue::new(&mut pct).range(1.0..=1000.0).speed(0.5).suffix(" %")).changed() {
                    item.scale = Some(pct / 100.0);
                    changed = true;
                }
                ui.end_row();
                if w > 0 {
                    let s = item.scale.unwrap_or(1.0);
                    ui.label("Taille affichée");
                    ui.label(RichText::new(format!("{:.0} × {:.0} px (source {w} × {h})", w as f32 * s, h as f32 * s)).color(theme::MUTED));
                    ui.end_row();
                }
                ui.label("Verrouillé");
                changed |= ui.checkbox(&mut item.locked, "Ne pas déplacer dans l'aperçu").changed();
                ui.end_row();
            });
        });
        if changed {
            self.touch();
        }
        self.transform_open = open;
    }
}
