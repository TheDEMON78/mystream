//! Scènes, sources et réglages, enregistrés en JSON dans le dossier de configuration.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct StreamSettings {
    /// Adresse RTMP du service (ex. rtmp://live.restream.io/live).
    pub server: String,
    /// Clé de stream (jamais affichée en clair).
    pub key: String,
    /// Débit vidéo en kb/s.
    pub video_bitrate: u32,
    /// Débit audio en kb/s.
    pub audio_bitrate: u32,
    /// Encodeur FFmpeg : "auto", "h264_nvenc", "h264_qsv", "h264_amf" ou "libx264".
    pub encoder: String,
}

impl Default for StreamSettings {
    fn default() -> Self {
        Self {
            server: String::new(),
            key: String::new(),
            video_bitrate: 6000,
            audio_bitrate: 160,
            encoder: "auto".into(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct RecordSettings {
    pub folder: String,
    /// "mkv" (résiste aux coupures) ou "mp4".
    pub format: String,
    pub video_bitrate: u32,
}

impl Default for RecordSettings {
    fn default() -> Self {
        let folder = dirs::video_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("."))
            .to_string_lossy()
            .into_owned();
        Self {
            folder,
            format: "mkv".into(),
            video_bitrate: 40000,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioSettings {
    pub mic_volume: f32,
    pub mic_muted: bool,
    pub desktop_volume: f32,
    pub desktop_muted: bool,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            mic_volume: 1.0,
            mic_muted: false,
            desktop_volume: 1.0,
            desktop_muted: false,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub stream: StreamSettings,
    pub record: RecordSettings,
    pub audio: AudioSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            fps: 60,
            stream: StreamSettings::default(),
            record: RecordSettings::default(),
            audio: AudioSettings::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum SourceKind {
    /// Écran entier (index de l'écran).
    Display { monitor: usize },
    /// Fenêtre d'une application, retrouvée par son titre.
    Window { title: String },
    /// Image fixe (PNG, JPEG…).
    Image { path: String },
    /// Aplat de couleur.
    Color {
        rgba: [u8; 4],
        width: u32,
        height: u32,
    },
}

impl SourceKind {
    pub fn label(&self) -> &'static str {
        match self {
            SourceKind::Display { .. } => "Capture d'écran",
            SourceKind::Window { .. } => "Capture de fenêtre",
            SourceKind::Image { .. } => "Image",
            SourceKind::Color { .. } => "Couleur",
        }
    }
    pub fn glyph(&self) -> &'static str {
        match self {
            SourceKind::Display { .. } => "🖥",
            SourceKind::Window { .. } => "🗔",
            SourceKind::Image { .. } => "🖼",
            SourceKind::Color { .. } => "■",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Source {
    pub id: u64,
    pub name: String,
    pub kind: SourceKind,
}

/// Élément d'une scène : une source placée sur le canevas.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Item {
    pub id: u64,
    pub source: u64,
    pub x: f32,
    pub y: f32,
    /// Échelle (1 = taille d'origine) ; None tant que la taille de la source est inconnue.
    pub scale: Option<f32>,
    pub visible: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Scene {
    pub id: u64,
    pub name: String,
    /// Du premier plan à l'arrière-plan, comme dans OBS.
    pub items: Vec<Item>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Project {
    pub settings: Settings,
    pub sources: Vec<Source>,
    pub scenes: Vec<Scene>,
    pub current: u64,
    pub next_id: u64,
}

impl Default for Project {
    fn default() -> Self {
        let mut p = Self {
            settings: Settings::default(),
            sources: vec![],
            scenes: vec![],
            current: 0,
            next_id: 1,
        };
        let id = p.new_id();
        p.scenes.push(Scene {
            id,
            name: "Scène".into(),
            items: vec![],
        });
        p.current = id;
        p
    }
}

impl Project {
    fn path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("MyStream-natif")
            .join("projet.json")
    }

    pub fn load() -> Self {
        let mut p: Project = std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        if p.scenes.is_empty() {
            p = Project {
                settings: p.settings,
                ..Project::default()
            };
        }
        if p.scene(p.current).is_none() {
            p.current = p.scenes[0].id;
        }
        p
    }

    pub fn save(&self) {
        let path = Self::path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        match serde_json::to_string_pretty(self) {
            // Écriture dans un fichier temporaire puis renommage : jamais de projet à moitié écrit.
            Ok(json) => {
                let tmp = path.with_extension("json.tmp");
                if std::fs::write(&tmp, json).is_ok() {
                    let _ = std::fs::rename(&tmp, &path);
                }
            }
            Err(e) => log::error!("Enregistrement du projet impossible : {e}"),
        }
    }

    pub fn new_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn scene(&self, id: u64) -> Option<&Scene> {
        self.scenes.iter().find(|s| s.id == id)
    }

    pub fn scene_mut(&mut self, id: u64) -> Option<&mut Scene> {
        self.scenes.iter_mut().find(|s| s.id == id)
    }

    pub fn source(&self, id: u64) -> Option<&Source> {
        self.sources.iter().find(|s| s.id == id)
    }

    pub fn unique_name(&self, base: &str, names: impl Iterator<Item = String>) -> String {
        let names: Vec<String> = names.collect();
        if !names.iter().any(|n| n == base) {
            return base.to_string();
        }
        (2..)
            .map(|i| format!("{base} {i}"))
            .find(|n| !names.contains(n))
            .unwrap()
    }

    pub fn add_source(&mut self, name: &str, kind: SourceKind) -> u64 {
        let id = self.new_id();
        let name = self.unique_name(name, self.sources.iter().map(|s| s.name.clone()));
        self.sources.push(Source { id, name, kind });
        let item = Item {
            id: self.new_id(),
            source: id,
            x: 0.0,
            y: 0.0,
            scale: None,
            visible: true,
        };
        let current = self.current;
        if let Some(scene) = self.scene_mut(current) {
            scene.items.insert(0, item);
        }
        id
    }

    /// Retire l'élément ; la source disparaît si plus aucune scène ne l'utilise.
    pub fn remove_item(&mut self, scene: u64, item: u64) {
        let Some(s) = self.scene_mut(scene) else {
            return;
        };
        let Some(pos) = s.items.iter().position(|i| i.id == item) else {
            return;
        };
        let source = s.items.remove(pos).source;
        if !self
            .scenes
            .iter()
            .any(|s| s.items.iter().any(|i| i.source == source))
        {
            self.sources.retain(|s| s.id != source);
        }
    }

    pub fn add_scene(&mut self) -> u64 {
        let id = self.new_id();
        let name = self.unique_name("Scène", self.scenes.iter().map(|s| s.name.clone()));
        self.scenes.push(Scene {
            id,
            name,
            items: vec![],
        });
        id
    }

    pub fn remove_scene(&mut self, id: u64) {
        if self.scenes.len() <= 1 {
            return;
        }
        let items: Vec<u64> = self
            .scene(id)
            .map(|s| s.items.iter().map(|i| i.id).collect())
            .unwrap_or_default();
        for item in items {
            self.remove_item(id, item);
        }
        self.scenes.retain(|s| s.id != id);
        if self.current == id {
            self.current = self.scenes[0].id;
        }
    }
}
