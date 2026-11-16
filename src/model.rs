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
    pub transition: Transition,
    /// Mode studio : on prépare une scène dans l'aperçu avant de la passer à l'antenne.
    pub studio: bool,
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
            transition: Transition::default(),
            studio: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum SourceKind {
    /// Écran entier (index de l'écran).
    Display { monitor: usize },
    /// Fenêtre d'une application, retrouvée par son titre ; son son peut être capturé.
    Window {
        title: String,
        #[serde(default)]
        audio: bool,
    },
    /// Capture de jeu automatique : le jeu en plein écran (ou fenêtré sans bordure), avec son son.
    Game {
        #[serde(default = "yes")]
        audio: bool,
    },
    /// Image fixe (PNG, JPEG…).
    Image { path: String },
    /// Aplat de couleur.
    Color {
        rgba: [u8; 4],
        width: u32,
        height: u32,
    },
    /// Texte.
    Text {
        text: String,
        size: f32,
        rgba: [u8; 4],
        #[serde(default)]
        background: [u8; 4],
        #[serde(default)]
        mono: bool,
    },
    /// Fichier vidéo ou audio, lu en boucle.
    Media {
        path: String,
        #[serde(default = "yes")]
        looping: bool,
    },
    /// Caméra (webcam, carte d'acquisition).
    Camera {
        device: String,
        width: u32,
        height: u32,
        fps: u32,
    },
    /// Son d'une application seule (Discord, navigateur…), sans image.
    AppAudio { exe: String },
}

fn yes() -> bool {
    true
}

impl SourceKind {
    pub fn label(&self) -> &'static str {
        match self {
            SourceKind::Display { .. } => "Capture d'écran",
            SourceKind::Window { .. } => "Capture de fenêtre",
            SourceKind::Game { .. } => "Capture de jeu",
            SourceKind::Image { .. } => "Image",
            SourceKind::Color { .. } => "Couleur",
            SourceKind::Text { .. } => "Texte",
            SourceKind::Media { .. } => "Média",
            SourceKind::Camera { .. } => "Caméra",
            SourceKind::AppAudio { .. } => "Son d'une application",
        }
    }
    pub fn glyph(&self) -> &'static str {
        match self {
            SourceKind::Display { .. } => "🖥",
            SourceKind::Window { .. } => "🗔",
            SourceKind::Game { .. } => "🎮",
            SourceKind::Image { .. } => "🖼",
            SourceKind::Color { .. } => "■",
            SourceKind::Text { .. } => "T",
            SourceKind::Media { .. } => "🎞",
            SourceKind::Camera { .. } => "📷",
            SourceKind::AppAudio { .. } => "🎵",
        }
    }
    /// Sources qui produisent du son (piste dans le mixeur).
    pub fn has_audio(&self) -> bool {
        match self {
            SourceKind::Window { audio, .. } | SourceKind::Game { audio } => *audio,
            SourceKind::Media { .. } | SourceKind::AppAudio { .. } => true,
            _ => false,
        }
    }
    /// Sources sans image.
    pub fn audio_only(&self) -> bool {
        matches!(self, SourceKind::AppAudio { .. })
    }
}

/// Filtres d'une source, comme ceux d'OBS.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Filters {
    /// Opacité (0 à 1).
    pub opacity: f32,
    /// Luminosité (-1 à 1), contraste et saturation (0 à 2, 1 = inchangé).
    pub brightness: f32,
    pub contrast: f32,
    pub saturation: f32,
    /// Recadrage en pixels de la source : gauche, haut, droite, bas.
    pub crop: [u32; 4],
    pub flip_h: bool,
    pub flip_v: bool,
    /// Incrustation (fond vert).
    pub chroma: bool,
    pub chroma_color: [u8; 3],
    pub similarity: f32,
    pub smoothness: f32,
}

impl Default for Filters {
    fn default() -> Self {
        Self {
            opacity: 1.0,
            brightness: 0.0,
            contrast: 1.0,
            saturation: 1.0,
            crop: [0; 4],
            flip_h: false,
            flip_v: false,
            chroma: false,
            chroma_color: [0, 255, 0],
            similarity: 0.4,
            smoothness: 0.08,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Source {
    pub id: u64,
    pub name: String,
    pub kind: SourceKind,
    #[serde(default)]
    pub filters: Filters,
    #[serde(default = "one")]
    pub volume: f32,
    #[serde(default)]
    pub muted: bool,
}

fn one() -> f32 {
    1.0
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum TransitionKind {
    Cut,
    Fade,
    Slide,
}

impl TransitionKind {
    pub fn label(self) -> &'static str {
        match self {
            TransitionKind::Cut => "Coupure",
            TransitionKind::Fade => "Fondu",
            TransitionKind::Slide => "Glissement",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Transition {
    pub kind: TransitionKind,
    pub duration_ms: u32,
}

impl Default for Transition {
    fn default() -> Self {
        Self {
            kind: TransitionKind::Fade,
            duration_ms: 300,
        }
    }
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
    /// Scène de l'aperçu en mode studio.
    pub preview: u64,
    pub next_id: u64,
}

impl Default for Project {
    fn default() -> Self {
        let mut p = Self {
            settings: Settings::default(),
            sources: vec![],
            scenes: vec![],
            current: 0,
            preview: 0,
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
        let path = Self::path();
        let mut p: Project = match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str(&text) {
                Ok(p) => p,
                Err(e) => {
                    // Projet illisible : on le met de côté au lieu de l'écraser.
                    let backup = path.with_extension("json.illisible");
                    log::error!(
                        "Projet illisible ({e}) : copie conservée dans {}",
                        backup.display()
                    );
                    let _ = std::fs::copy(&path, &backup);
                    Project::default()
                }
            },
            Err(_) => Project::default(),
        };
        if p.scenes.is_empty() {
            p = Project {
                settings: p.settings,
                ..Project::default()
            };
        }
        if p.scene(p.current).is_none() {
            p.current = p.scenes[0].id;
        }
        if p.scene(p.preview).is_none() {
            p.preview = p.current;
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
        self.sources.push(Source {
            id,
            name,
            kind,
            filters: Filters::default(),
            volume: 1.0,
            muted: false,
        });
        let item = Item {
            id: self.new_id(),
            source: id,
            x: 0.0,
            y: 0.0,
            scale: None,
            visible: true,
        };
        let edit = self.edit_scene();
        if let Some(scene) = self.scene_mut(edit) {
            scene.items.insert(0, item);
        }
        id
    }

    /// Scène modifiée par l'interface : l'aperçu en mode studio, sinon celle à l'antenne.
    pub fn edit_scene(&self) -> u64 {
        if self.settings.studio {
            self.preview
        } else {
            self.current
        }
    }

    /// Ajoute une source existante (partagée) à une scène.
    pub fn add_existing(&mut self, scene: u64, source: u64) {
        let item = Item {
            id: self.new_id(),
            source,
            x: 0.0,
            y: 0.0,
            scale: None,
            visible: true,
        };
        if let Some(s) = self.scene_mut(scene) {
            s.items.insert(0, item);
        }
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
        if self.preview == id {
            self.preview = self.current;
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn lit_le_projet_de_test() {
        let Ok(path) = std::env::var("PROJET") else {
            return;
        };
        let text = std::fs::read_to_string(path).unwrap();
        let p: super::Project = serde_json::from_str(&text).unwrap();
        assert_eq!(p.settings.width, 1280);
    }
}
