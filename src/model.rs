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
    /// Service : "restream" (application Restream de BloopStudio) ou "custom" (serveur RTMP).
    pub service: String,
    /// Préréglage x264 (encodage par le processeur).
    pub preset: String,
}

impl Default for StreamSettings {
    fn default() -> Self {
        Self {
            server: String::new(),
            key: String::new(),
            video_bitrate: 6000,
            audio_bitrate: 160,
            encoder: "auto".into(),
            service: "custom".into(),
            preset: "veryfast".into(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct RecordSettings {
    pub folder: String,
    /// "mkv" (résiste aux coupures), "mp4" ou "webm".
    pub format: String,
    pub video_bitrate: u32,
    pub audio_bitrate: u32,
    /// Préréglage x264 (encodage par le processeur).
    pub preset: String,
}

impl Default for RecordSettings {
    fn default() -> Self {
        let folder = dirs::video_dir().or_else(dirs::home_dir).unwrap_or_else(|| PathBuf::from(".")).to_string_lossy().into_owned();
        Self { folder, format: "mkv".into(), video_bitrate: 40000, audio_bitrate: 320, preset: "veryfast".into() }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioSettings {
    pub mic_volume: f32,
    pub mic_muted: bool,
    pub desktop_volume: f32,
    pub desktop_muted: bool,
    /// Filtres du micro (gain, noise gate, compresseur).
    pub mic_filters: AudioFilters,
    /// Écoute au casque du micro.
    pub mic_monitor: bool,
    /// Micro utilisé : vide = celui par défaut, "off" = aucun.
    pub mic_device: String,
    /// Son du bureau dans le mixage.
    pub desktop_audio: bool,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            mic_volume: 1.0,
            mic_muted: false,
            desktop_volume: 1.0,
            desktop_muted: false,
            mic_filters: AudioFilters::default(),
            mic_monitor: false,
            mic_device: String::new(),
            desktop_audio: true,
        }
    }
}

/// Filtres audio d'une piste, comme ceux de la version Electron.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioFilters {
    /// Gain en dB (-20 à +30).
    pub gain_db: f32,
    /// Noise gate : coupe le son sous le seuil (bruit de fond, clavier).
    pub gate: bool,
    pub gate_threshold_db: f32,
    /// Compresseur : réduit les pics au-dessus du seuil.
    pub compressor: bool,
    pub comp_threshold_db: f32,
    pub comp_ratio: f32,
}

impl Default for AudioFilters {
    fn default() -> Self {
        Self { gain_db: 0.0, gate: false, gate_threshold_db: -45.0, compressor: false, comp_threshold_db: -18.0, comp_ratio: 4.0 }
    }
}

/// Application Restream (BloopStudio) : adresse, mot de passe, RTMP imposé.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RestreamSettings {
    pub url: String,
    pub password: String,
    /// Adresse RTMP imposée (reverse proxy, redirection de port) ; vide = automatique.
    pub rtmp_url: String,
    pub allow_self_signed: bool,
}

/// Chat : via Restream, Twitch en direct, ou désactivé.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ChatSettings {
    /// "restream", "direct" (Twitch et Kick sans compte) ou "off".
    pub mode: String,
    pub twitch_channel: String,
    pub kick_channel: String,
}

impl Default for ChatSettings {
    fn default() -> Self {
        Self { mode: "off".into(), twitch_channel: String::new(), kick_channel: String::new() }
    }
}

/// Contrôle à distance compatible obs-websocket (Stream Deck…).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct RemoteSettings {
    pub enabled: bool,
    pub port: u16,
    /// Mot de passe (généré au premier lancement).
    pub password: String,
    /// Accepter les appareils du réseau local (sinon ce PC uniquement).
    pub lan: bool,
    /// Mot de passe de la version Electron déjà repris (une seule fois).
    #[serde(default)]
    pub electron_checked: bool,
}

impl Default for RemoteSettings {
    fn default() -> Self {
        Self { enabled: true, port: 4455, password: String::new(), lan: false, electron_checked: false }
    }
}

/// Raccourcis clavier globaux (fonctionnent même en jeu) : action -> combinaison.
pub const HOTKEY_ACTIONS: [(&str, &str); 7] = [
    ("stream", "Démarrer / arrêter le streaming"),
    ("record", "Démarrer / arrêter l'enregistrement"),
    ("mic", "Couper / rétablir le micro"),
    ("desktop", "Couper / rétablir le son du bureau"),
    ("transition", "Transition (mode studio)"),
    ("next", "Scène suivante"),
    ("prev", "Scène précédente"),
];

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
    pub remote: RemoteSettings,
    pub hotkeys: std::collections::BTreeMap<String, String>,
    pub restream: RestreamSettings,
    pub chat: ChatSettings,
    /// Mises à jour installées automatiquement (jamais pendant un live).
    pub auto_update: bool,
    /// Réglages de la version Electron déjà repris (une seule fois).
    pub electron_imported: bool,
    /// Demander confirmation avant d'arrêter le streaming.
    pub confirm_stop_stream: bool,
    /// Réduire dans la zone de notification.
    pub minimize_to_tray: bool,
    /// Pendant un live, ne plus redessiner l'aperçu quand MyStream n'est pas au premier plan.
    pub auto_hide_preview: bool,
    /// Disposition des docks, barre « En direct », assistant.
    pub ui: UiSettings,
}

/// Disposition des docks : rangée du bas et colonne de droite, comme la version Electron.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Layout {
    pub bottom: Vec<String>,
    pub side: Vec<String>,
    /// Docks masqués.
    pub hidden: Vec<String>,
}

pub const DOCKS: [(&str, &str); 7] = [
    ("scenes", "Scènes"),
    ("sources", "Sources"),
    ("mixer", "Mixeur audio"),
    ("transitions", "Transitions"),
    ("controls", "Commandes"),
    ("chat", "Chat"),
    ("liveinfo", "Titre et catégorie"),
];

impl Default for Layout {
    fn default() -> Self {
        Self {
            bottom: ["scenes", "sources", "mixer", "transitions", "controls"].map(String::from).to_vec(),
            side: ["chat", "liveinfo"].map(String::from).to_vec(),
            hidden: Vec::new(),
        }
    }
}

impl Layout {
    /// Chaque dock présent une seule fois, dans une seule zone.
    pub fn normalized(mut self) -> Self {
        let known = |d: &String| DOCKS.iter().any(|(k, _)| k == d);
        self.bottom.retain(known);
        self.side.retain(|d| known(d));
        let bottom = self.bottom.clone();
        self.side.retain(|d| !bottom.contains(d));
        for (k, _) in DOCKS {
            if !self.bottom.iter().any(|d| d == k) && !self.side.iter().any(|d| d == k) {
                if ["chat", "liveinfo"].contains(&k) { self.side.push(k.into()) } else { self.bottom.push(k.into()) }
            }
        }
        self
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct UiSettings {
    pub layout: Layout,
    /// Dispositions enregistrées par nom.
    pub layouts: std::collections::BTreeMap<String, Layout>,
    /// Barre « En direct » au-dessus de l'aperçu pendant le live.
    pub live_bar: bool,
    /// Assistant de configuration déjà proposé.
    #[serde(default = "yes")]
    pub wizard_done: bool,
}

impl Default for UiSettings {
    fn default() -> Self {
        Self { layout: Layout::default(), layouts: Default::default(), live_bar: true, wizard_done: false }
    }
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
            remote: RemoteSettings::default(),
            hotkeys: Default::default(),
            restream: RestreamSettings::default(),
            chat: ChatSettings::default(),
            auto_update: true,
            electron_imported: false,
            confirm_stop_stream: true,
            minimize_to_tray: false,
            auto_hide_preview: true,
            ui: UiSettings::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum SourceKind {
    /// Écran entier (index de l'écran) ; avec `audio`, le son du PC (hors MyStream).
    Display {
        monitor: usize,
        #[serde(default)]
        audio: bool,
    },
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
        /// "screen" : l'écran du jeu (le plus fluide) ; "window" : la fenêtre du jeu.
        #[serde(default = "screen")]
        method: String,
    },
    /// Image fixe (PNG, JPEG…).
    Image { path: String },
    /// Aplat de couleur.
    Color { rgba: [u8; 4], width: u32, height: u32 },
    /// Texte.
    Text {
        text: String,
        size: f32,
        rgba: [u8; 4],
        #[serde(default)]
        background: [u8; 4],
        #[serde(default)]
        mono: bool,
        #[serde(default = "arial")]
        font: String,
        #[serde(default)]
        bold: bool,
        #[serde(default)]
        italic: bool,
        /// "left", "center" ou "right".
        #[serde(default = "left")]
        align: String,
        #[serde(default)]
        outline: bool,
        #[serde(default = "black")]
        outline_color: [u8; 4],
        #[serde(default = "three")]
        outline_size: f32,
    },
    /// Fichier vidéo ou audio, lu en boucle.
    Media {
        path: String,
        #[serde(default = "yes")]
        looping: bool,
    },
    /// Caméra (webcam, carte d'acquisition).
    Camera { device: String, width: u32, height: u32, fps: u32 },
    /// Son d'une application seule (Discord, navigateur…), sans image.
    AppAudio { exe: String },
    /// Micro ou autre entrée audio (vide : celle par défaut), avec suppression du bruit.
    AudioInput {
        device: String,
        #[serde(default)]
        noise_suppression: bool,
    },
    /// Chat du live incrusté dans l'image.
    Chat(ChatStyle),
    /// Page web (alertes, widgets) avec sa transparence et son son.
    Browser {
        url: String,
        width: u32,
        height: u32,
        #[serde(default = "browser_css")]
        css: String,
        #[serde(default = "yes")]
        audio: bool,
    },
}

/// Apparence de la source « Chat » (les mêmes réglages que la version Electron).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ChatStyle {
    pub width: u32,
    pub height: u32,
    pub size: f32,
    pub background: [u8; 4],
    pub max_messages: u32,
    pub font: String,
    pub text_color: [u8; 4],
    /// Couleur des pseudos quand la plateforme n'en fournit pas.
    pub name_color: [u8; 4],
    /// Messages masqués après ce nombre de secondes (0 : jamais).
    pub lifetime: u32,
    pub show_icons: bool,
    pub show_badges: bool,
    pub show_system: bool,
    /// Contour noir autour du texte.
    pub shadow: bool,
}

impl Default for ChatStyle {
    fn default() -> Self {
        Self {
            width: 450,
            height: 600,
            size: 24.0,
            background: [0, 0, 0, 0],
            max_messages: 30,
            font: "Segoe UI".into(),
            text_color: [255, 255, 255, 255],
            name_color: [0x4c, 0x7d, 0xff, 255],
            lifetime: 0,
            show_icons: true,
            show_badges: true,
            show_system: false,
            shadow: true,
        }
    }
}

/// Polices proposées (celles de la version Electron) et leurs fichiers Windows.
pub const CHAT_FONTS: [(&str, &str, &str); 12] = [
    ("Arial", "arial.ttf", "arialbd.ttf"),
    ("Segoe UI", "segoeui.ttf", "seguisb.ttf"),
    ("Roboto", "Roboto-Regular.ttf", "Roboto-Bold.ttf"),
    ("Helvetica", "arial.ttf", "arialbd.ttf"),
    ("Verdana", "verdana.ttf", "verdanab.ttf"),
    ("Tahoma", "tahoma.ttf", "tahomabd.ttf"),
    ("Trebuchet MS", "trebuc.ttf", "trebucbd.ttf"),
    ("Georgia", "georgia.ttf", "georgiab.ttf"),
    ("Times New Roman", "times.ttf", "timesbd.ttf"),
    ("Courier New", "cour.ttf", "courbd.ttf"),
    ("Impact", "impact.ttf", "impact.ttf"),
    ("Comic Sans MS", "comic.ttf", "comicbd.ttf"),
];

/// CSS ajouté aux pages, comme celui d'OBS : fond transparent, sans barres de défilement.
pub fn browser_css() -> String {
    "body { background-color: rgba(0, 0, 0, 0); margin: 0px auto; overflow: hidden; }".into()
}

fn yes() -> bool {
    true
}
fn screen() -> String {
    "screen".into()
}
fn arial() -> String {
    "Arial".into()
}
fn left() -> String {
    "left".into()
}
fn black() -> [u8; 4] {
    [0, 0, 0, 255]
}
fn three() -> f32 {
    3.0
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
            SourceKind::AudioInput { .. } => "Entrée audio",
            SourceKind::Chat { .. } => "Chat",
            SourceKind::Browser { .. } => "Navigateur",
        }
    }
    pub fn glyph(&self) -> &'static str {
        match self {
            SourceKind::Display { .. } => "🖥",
            SourceKind::Window { .. } => "🗗",
            SourceKind::Game { .. } => "🎮",
            SourceKind::Image { .. } => "🖼",
            SourceKind::Color { .. } => "■",
            SourceKind::Text { .. } => "T",
            SourceKind::Media { .. } => "🎞",
            SourceKind::Camera { .. } => "📷",
            SourceKind::AppAudio { .. } => "🎵",
            SourceKind::AudioInput { .. } => "🎤",
            SourceKind::Chat { .. } => "💬",
            SourceKind::Browser { .. } => "🌐",
        }
    }
    /// Sources qui produisent du son (piste dans le mixeur).
    pub fn has_audio(&self) -> bool {
        match self {
            SourceKind::Window { audio, .. } | SourceKind::Game { audio, .. } | SourceKind::Browser { audio, .. } | SourceKind::Display { audio, .. } => *audio,
            SourceKind::Media { .. } | SourceKind::AppAudio { .. } | SourceKind::AudioInput { .. } => true,
            _ => false,
        }
    }
    /// Texte avec les réglages par défaut (police Arial, contour noir désactivé).
    pub fn text(text: &str, size: f32, rgba: [u8; 4]) -> Self {
        SourceKind::Text {
            text: text.into(),
            size,
            rgba,
            background: [0, 0, 0, 0],
            mono: false,
            font: arial(),
            bold: false,
            italic: false,
            align: left(),
            outline: false,
            outline_color: black(),
            outline_size: 3.0,
        }
    }
    /// Sources qui occupent un appareil ou du calcul : arrêtées quand elles ne sont plus affichées.
    pub fn costly(&self) -> bool {
        matches!(
            self,
            SourceKind::Camera { .. }
                | SourceKind::Display { .. }
                | SourceKind::Window { .. }
                | SourceKind::Game { .. }
                | SourceKind::Media { .. }
                | SourceKind::Browser { .. }
                | SourceKind::AppAudio { .. }
                | SourceKind::AudioInput { .. }
        )
    }
    /// Sources sans image.
    pub fn audio_only(&self) -> bool {
        matches!(self, SourceKind::AppAudio { .. } | SourceKind::AudioInput { .. })
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
    /// Teinte : rotation des couleurs en degrés (-180 à 180).
    pub hue: f32,
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
            hue: 0.0,
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
    #[serde(default)]
    pub audio_filters: AudioFilters,
    /// Écoute au casque.
    #[serde(default)]
    pub monitor: bool,
}

fn one() -> f32 {
    1.0
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum TransitionKind {
    Cut,
    Fade,
    Slide,
    /// Vidéo jouée par-dessus ; la scène change au point de coupe.
    Stinger,
}

impl TransitionKind {
    pub fn label(self) -> &'static str {
        match self {
            TransitionKind::Cut => "Coupure",
            TransitionKind::Fade => "Fondu",
            TransitionKind::Slide => "Glissement",
            TransitionKind::Stinger => "Stinger (vidéo)",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Transition {
    pub kind: TransitionKind,
    pub duration_ms: u32,
    /// Stinger : vidéo (de préférence avec transparence : WebM VP9, MOV ProRes 4444).
    pub stinger_file: String,
    /// Moment où la scène change, depuis le début de la vidéo.
    pub stinger_point_ms: u32,
    /// Glissement : "left", "right", "up" ou "down" (sens du mouvement).
    #[serde(default = "left")]
    pub direction: String,
    /// Son de l'animation du stinger dans le live.
    #[serde(default = "yes")]
    pub stinger_audio: bool,
}

impl Default for Transition {
    fn default() -> Self {
        Self { kind: TransitionKind::Fade, duration_ms: 300, stinger_file: String::new(), stinger_point_ms: 500, direction: left(), stinger_audio: true }
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
    /// Échelle verticale différente (« Étirer à l'écran ») ; None : la même que `scale`.
    #[serde(default)]
    pub scale_y: Option<f32>,
    pub visible: bool,
    /// Verrouillé : ne se déplace pas par erreur dans l'aperçu.
    #[serde(default)]
    pub locked: bool,
}

impl Item {
    /// Échelles horizontale et verticale.
    pub fn scales(&self) -> Option<(f32, f32)> {
        self.scale.map(|s| (s, self.scale_y.unwrap_or(s)))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Scene {
    pub id: u64,
    pub name: String,
    /// Du premier plan à l'arrière-plan, comme dans OBS.
    pub items: Vec<Item>,
    /// Transition pour passer à cette scène (sinon celle du dock Transitions).
    #[serde(default)]
    pub transition: Option<Transition>,
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
        let mut p = Self { settings: Settings::default(), sources: vec![], scenes: vec![], current: 0, preview: 0, next_id: 1 };
        let id = p.new_id();
        p.scenes.push(Scene { id, name: "Scène".into(), items: vec![], transition: None });
        p.current = id;
        p
    }
}

impl Project {
    fn path() -> PathBuf {
        dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("MyStream-natif").join("projet.json")
    }

    pub fn load() -> Self {
        let path = Self::path();
        let existed = path.exists();
        let mut p: Project = match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str(&text) {
                Ok(p) => p,
                Err(e) => {
                    // Projet illisible : on le met de côté au lieu de l'écraser.
                    let backup = path.with_extension("json.illisible");
                    log::error!("Projet illisible ({e}) : copie conservée dans {}", backup.display());
                    let _ = std::fs::copy(&path, &backup);
                    Project::default()
                }
            },
            Err(_) => Project::default(),
        };
        // L'assistant n'est proposé qu'au tout premier lancement.
        p.settings.ui.wizard_done |= existed;
        p.settings.ui.layout = p.settings.ui.layout.clone().normalized();
        if p.scenes.is_empty() {
            p = Project { settings: p.settings, ..Project::default() };
        }
        if p.scene(p.current).is_none() {
            p.current = p.scenes[0].id;
        }
        if p.scene(p.preview).is_none() {
            p.preview = p.current;
        }
        // Réglages de la version Electron (chat, Restream) repris au premier lancement.
        if !p.settings.electron_imported {
            p.settings.electron_imported = true;
            if let Some(e) = electron_config() {
                import_electron(&mut p.settings, &e["settings"]);
                log::info!("Réglages de la version Electron repris (chat : {})", p.settings.chat.mode);
            }
        }
        // Ancien réglage « Twitch en direct » : devenu « Direct (Twitch + Kick) ».
        if p.settings.chat.mode == "twitch" {
            p.settings.chat.mode = "direct".into();
        }
        // Le plugin Stream Deck garde le mot de passe de la version Electron :
        // on le reprend pour que les touches marchent sans rien changer.
        if !p.settings.remote.electron_checked {
            p.settings.remote.electron_checked = true;
            if let Some((password, port)) = electron_remote() {
                p.settings.remote.password = password;
                p.settings.remote.port = port;
            }
        }
        if p.settings.remote.password.is_empty() {
            p.settings.remote.password = crate::remote::random_token(16);
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
        (2..).map(|i| format!("{base} {i}")).find(|n| !names.contains(n)).unwrap()
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
            audio_filters: AudioFilters::default(),
            monitor: false,
        });
        let item = Item { id: self.new_id(), source: id, x: 0.0, y: 0.0, scale: None, scale_y: None, visible: true, locked: false };
        let edit = self.edit_scene();
        if let Some(scene) = self.scene_mut(edit) {
            scene.items.insert(0, item);
        }
        id
    }

    /// Scène modifiée par l'interface : l'aperçu en mode studio, sinon celle à l'antenne.
    pub fn edit_scene(&self) -> u64 {
        if self.settings.studio { self.preview } else { self.current }
    }

    /// Ajoute une source existante (partagée) à une scène.
    pub fn add_existing(&mut self, scene: u64, source: u64) {
        let item = Item { id: self.new_id(), source, x: 0.0, y: 0.0, scale: None, scale_y: None, visible: true, locked: false };
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
        if !self.scenes.iter().any(|s| s.items.iter().any(|i| i.source == source)) {
            self.sources.retain(|s| s.id != source);
        }
    }

    /// Copie d'une scène : sources partagées, ou copiées (indépendantes).
    pub fn duplicate_scene(&mut self, id: u64, independent: bool) -> Option<u64> {
        let scene = self.scene(id)?.clone();
        let name = self.unique_name(&format!("{} (copie)", scene.name), self.scenes.iter().map(|s| s.name.clone()));
        let mut items = Vec::new();
        for it in &scene.items {
            let mut it = it.clone();
            it.id = self.new_id();
            if independent && let Some(src) = self.source(it.source).cloned() {
                let sid = self.new_id();
                let n = self.unique_name(&format!("{} ({name})", src.name), self.sources.iter().map(|s| s.name.clone()));
                self.sources.push(Source { id: sid, name: n, ..src });
                it.source = sid;
            }
            items.push(it);
        }
        let new = self.new_id();
        let pos = self.scenes.iter().position(|s| s.id == id).map(|p| p + 1).unwrap_or(self.scenes.len());
        self.scenes.insert(pos, Scene { id: new, name, items, transition: self.scene(id).and_then(|s| s.transition.clone()) });
        Some(new)
    }

    /// « Rendre indépendante dans cette scène » : l'élément reçoit sa propre copie de la source.
    pub fn unlink_item(&mut self, scene: u64, item: u64) {
        let Some(source) = self.scene(scene).and_then(|s| s.items.iter().find(|i| i.id == item)).map(|i| i.source) else { return };
        let Some(src) = self.source(source).cloned() else { return };
        let scene_name = self.scene(scene).map(|s| s.name.clone()).unwrap_or_default();
        let sid = self.new_id();
        let name = self.unique_name(&format!("{} ({scene_name})", src.name), self.sources.iter().map(|s| s.name.clone()));
        self.sources.push(Source { id: sid, name, ..src });
        if let Some(it) = self.scene_mut(scene).and_then(|s| s.items.iter_mut().find(|i| i.id == item)) {
            it.source = sid;
        }
        self.drop_unused_sources();
    }

    /// « Utiliser une source existante » : l'élément affiche une autre source (partagée).
    pub fn link_item(&mut self, scene: u64, item: u64, source: u64) {
        if let Some(it) = self.scene_mut(scene).and_then(|s| s.items.iter_mut().find(|i| i.id == item)) {
            it.source = source;
        }
        self.drop_unused_sources();
    }

    /// Supprime les sources qui ne sont plus dans aucune scène.
    fn drop_unused_sources(&mut self) {
        let used: std::collections::HashSet<u64> = self.scenes.iter().flat_map(|s| s.items.iter().map(|i| i.source)).collect();
        self.sources.retain(|s| used.contains(&s.id));
    }

    pub fn add_scene(&mut self) -> u64 {
        let id = self.new_id();
        let name = self.unique_name("Scène", self.scenes.iter().map(|s| s.name.clone()));
        self.scenes.push(Scene { id, name, items: vec![], transition: None });
        id
    }

    pub fn remove_scene(&mut self, id: u64) {
        if self.scenes.len() <= 1 {
            return;
        }
        let items: Vec<u64> = self.scene(id).map(|s| s.items.iter().map(|i| i.id).collect()).unwrap_or_default();
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
mod electron_tests {
    #[test]
    fn reprend_le_chat_direct() {
        let mut s = super::Settings::default();
        let e = serde_json::json!({ "chat": { "mode": "direct", "twitchChannel": "bloop", "kickChannel": "bloop_k" }, "restream": { "url": "http://localhost:3050", "allowSelfSigned": true }, "stream": { "service": "restream" } });
        super::import_electron(&mut s, &e);
        assert_eq!((s.chat.mode.as_str(), s.chat.twitch_channel.as_str(), s.chat.kick_channel.as_str()), ("direct", "bloop", "bloop_k"));
        assert_eq!(s.stream.service, "restream");
        assert!(s.restream.allow_self_signed);
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

/// Configuration de la version Electron, si elle est installée.
pub fn electron_config() -> Option<serde_json::Value> {
    let path = dirs::config_dir()?.join("MyStream").join("config.json");
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// Reprend le chat (mode, chaînes) et Restream de la version Electron.
pub fn import_electron(s: &mut Settings, e: &serde_json::Value) {
    let text = |v: &serde_json::Value| v.as_str().unwrap_or("").to_string();
    let c = &e["chat"];
    if let Some(mode) = c["mode"].as_str().filter(|m| ["off", "restream", "direct"].contains(m)) {
        s.chat.mode = mode.to_string();
    }
    s.chat.twitch_channel = text(&c["twitchChannel"]);
    s.chat.kick_channel = text(&c["kickChannel"]);
    let r = &e["restream"];
    if r.is_object() {
        s.restream.url = text(&r["url"]);
        s.restream.password = text(&r["password"]);
        s.restream.rtmp_url = text(&r["rtmpUrl"]);
        s.restream.allow_self_signed = r["allowSelfSigned"].as_bool().unwrap_or(false);
    }
    let st = &e["stream"];
    match st["service"].as_str() {
        Some("restream") => s.stream.service = "restream".into(),
        Some(_) if !text(&st["key"]).is_empty() => {
            s.stream.service = "custom".into();
            s.stream.server = text(&st["server"]);
            s.stream.key = text(&st["key"]);
        }
        _ => {}
    }
}

/// Mot de passe et port du contrôle à distance de la version Electron, s'il est installé.
fn electron_remote() -> Option<(String, u16)> {
    let v = electron_config()?;
    let r = &v["settings"]["remote"];
    let password = r["password"].as_str().filter(|p| !p.is_empty())?.to_string();
    Some((password, r["port"].as_u64().and_then(|p| u16::try_from(p).ok()).unwrap_or(4455)))
}
