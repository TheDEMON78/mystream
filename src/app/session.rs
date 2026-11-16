//! Lives : récapitulatif à la fin (durée, messages, pic de spectateurs,
//! images perdues, débit moyen) et historique, comme la version Electron.

use super::App;
use crate::theme;
use egui::{RichText, vec2};
use serde::{Deserialize, Serialize};
use std::time::Instant;

/// Live en cours.
pub struct Session {
    start: Instant,
    started_at: u64,
    lagged0: u64,
    dropped0: u64,
    messages0: u64,
    peak: u64,
    bitrate_sum: f64,
    bitrate_n: u64,
    last_sample: Instant,
}

/// Résumé d'un live terminé (gardé dans l'historique).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Summary {
    pub started_at: u64,
    pub duration_s: u64,
    pub scene: String,
    pub messages: u64,
    pub peak_viewers: u64,
    pub lost_frames: u64,
    pub avg_bitrate: u64,
}

fn history_path() -> std::path::PathBuf {
    dirs::config_dir().unwrap_or_else(|| ".".into()).join("MyStream-natif").join("historique.json")
}

pub fn load_history() -> Vec<Summary> {
    std::fs::read_to_string(history_path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn save_history(list: &[Summary]) {
    if let Ok(json) = serde_json::to_string_pretty(list) {
        let _ = std::fs::write(history_path(), json);
    }
}

fn duration(s: u64) -> String {
    if s >= 3600 { format!("{} h {:02} min", s / 3600, s / 60 % 60) } else { format!("{} min {:02} s", s / 60, s % 60) }
}

fn date(ts: u64) -> String {
    // Date locale lisible (même calcul que les noms des enregistrements).
    let d = crate::output::local_date(ts as i64);
    format!("{} à {}", &d[..10], d[11..16].replace('-', ":"))
}

impl App {
    fn chat_count(&self) -> u64 {
        self.chat.total()
    }

    pub(super) fn session_started(&mut self) {
        let st = self.shared.stats.lock().clone();
        self.session = Some(Session {
            start: Instant::now(),
            started_at: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
            lagged0: st.lagged,
            dropped0: st.dropped,
            messages0: self.chat_count(),
            peak: 0,
            bitrate_sum: 0.0,
            bitrate_n: 0,
            last_sample: Instant::now(),
        });
    }

    /// Mesures prises pendant le live (débit, spectateurs).
    pub(super) fn session_tick(&mut self, viewers: Option<u64>) {
        let rate = self.shared.outputs.lock().get(&crate::engine::key(crate::output::Kind::Stream)).map(|o| o.stats.lock().bitrate_kbps);
        let Some(s) = self.session.as_mut() else { return };
        if let Some(v) = viewers {
            s.peak = s.peak.max(v);
        }
        if s.last_sample.elapsed().as_secs() >= 2
            && let Some(r) = rate.filter(|r| *r > 0.0)
        {
            s.bitrate_sum += r as f64;
            s.bitrate_n += 1;
            s.last_sample = Instant::now();
        }
    }

    pub(super) fn session_ended(&mut self) {
        let Some(s) = self.session.take() else { return };
        let st = self.shared.stats.lock().clone();
        let summary = Summary {
            started_at: s.started_at,
            duration_s: s.start.elapsed().as_secs(),
            scene: self.project.scene(self.project.current).map(|x| x.name.clone()).unwrap_or_default(),
            messages: self.chat_count().saturating_sub(s.messages0),
            peak_viewers: s.peak,
            lost_frames: (st.lagged - s.lagged0) + (st.dropped - s.dropped0),
            avg_bitrate: if s.bitrate_n > 0 { (s.bitrate_sum / s.bitrate_n as f64) as u64 } else { 0 },
        };
        let mut history = load_history();
        history.insert(0, summary.clone());
        history.truncate(200);
        save_history(&history);
        self.recap = Some(summary);
    }

    fn tiles(ui: &mut egui::Ui, s: &Summary, fps: u32) {
        let total = s.duration_s * fps as u64;
        let pct = if total > 0 { s.lost_frames as f64 * 100.0 / total as f64 } else { 0.0 };
        egui::Grid::new(("recap", s.started_at)).num_columns(3).spacing(vec2(12.0, 12.0)).show(ui, |ui| {
            let tile = |ui: &mut egui::Ui, value: String, label: &str| {
                egui::Frame::new().fill(theme::PANEL2).corner_radius(6).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
                    ui.set_width(150.0);
                    ui.label(RichText::new(value).size(20.0).family(theme::bold()));
                    ui.label(RichText::new(label).small().color(theme::MUTED));
                });
            };
            tile(ui, duration(s.duration_s), "Durée");
            tile(ui, s.messages.to_string(), "Messages du chat");
            tile(ui, if s.peak_viewers > 0 { s.peak_viewers.to_string() } else { "–".into() }, "Pic de spectateurs");
            ui.end_row();
            tile(ui, format!("{pct:.1} %"), &format!("Images perdues ({})", s.lost_frames));
            tile(ui, if s.avg_bitrate > 0 { s.avg_bitrate.to_string() } else { "–".into() }, "Débit moyen (kb/s)");
            tile(ui, s.scene.clone(), "Dernière scène");
            ui.end_row();
        });
    }

    pub(super) fn session_windows(&mut self, ctx: &egui::Context) {
        let fps = self.project.settings.fps;
        if let Some(s) = self.recap.clone() {
            let mut open = true;
            egui::Window::new("Fin du live").open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
                ui.label(RichText::new(format!("Live du {}", date(s.started_at))).color(theme::MUTED));
                ui.add_space(6.0);
                Self::tiles(ui, &s, fps);
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Historique…").clicked() {
                        self.history_open = true;
                        self.recap = None;
                    }
                    if ui.button("Fermer").clicked() {
                        self.recap = None;
                    }
                });
            });
            if !open {
                self.recap = None;
            }
        }
        if self.history_open {
            let mut open = true;
            egui::Window::new("Historique des lives").open(&mut open).default_size(vec2(560.0, 480.0)).show(ctx, |ui| {
                let list = load_history();
                if list.is_empty() {
                    ui.label(RichText::new("Aucun live pour le moment.").italics().color(theme::MUTED));
                }
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for s in &list {
                        egui::CollapsingHeader::new(format!("{} — {}", date(s.started_at), duration(s.duration_s)))
                            .id_salt(s.started_at)
                            .show(ui, |ui| Self::tiles(ui, s, fps));
                    }
                });
            });
            self.history_open = open;
        }
    }
}
