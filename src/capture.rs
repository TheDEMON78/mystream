//! Sources vidéo : capture d'écran et de fenêtre, image, couleur.
//!
//! Chaque source tient sa dernière image dans un `Slot`, lu par le
//! compositeur à chaque image du stream. Sous Windows, les captures passent
//! par Windows Graphics Capture (comme OBS), limitées à la cadence du stream.

use crate::model::SourceKind;
use parking_lot::Mutex;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Image RGBA 8 bits, sans marge entre les lignes.
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
    /// Numéro croissant : le compositeur ne renvoie l'image à la carte graphique que si elle a changé.
    pub seq: u64,
}

/// Dernière image d'une source, partagée entre la capture et le compositeur.
#[derive(Clone, Default)]
pub struct Slot(Arc<Mutex<Option<Arc<Frame>>>>);

impl Slot {
    pub fn get(&self) -> Option<Arc<Frame>> {
        self.0.lock().clone()
    }
    pub fn set(&self, f: Frame) {
        *self.0.lock() = Some(Arc::new(f));
    }
}

static SEQ: AtomicU64 = AtomicU64::new(1);
fn next_seq() -> u64 {
    SEQ.fetch_add(1, Ordering::Relaxed)
}

/// État partagé d'une capture : arrêt, erreur, cadence mesurée.
#[derive(Default)]
pub struct Status {
    pub stop: AtomicBool,
    pub error: Mutex<Option<String>>,
    /// Images reçues pendant la dernière seconde.
    pub fps: Mutex<f32>,
    counter: Mutex<(u32, Option<Instant>)>,
}

impl Status {
    fn frame(&self) {
        let mut c = self.counter.lock();
        let now = Instant::now();
        let start = *c.1.get_or_insert(now);
        c.0 += 1;
        let dt = now.duration_since(start).as_secs_f32();
        if dt >= 1.0 {
            *self.fps.lock() = c.0 as f32 / dt;
            *c = (0, Some(now));
        }
    }
    fn fail(&self, msg: String) {
        log::warn!("{msg}");
        *self.error.lock() = Some(msg);
    }
}

/// Source vidéo en cours d'exécution.
pub struct Runtime {
    pub slot: Slot,
    pub status: Arc<Status>,
    kind: SourceKind,
}

impl Runtime {
    pub fn start(kind: &SourceKind, fps: u32) -> Self {
        let slot = Slot::default();
        let status = Arc::new(Status::default());
        match kind {
            SourceKind::Color {
                rgba,
                width,
                height,
            } => {
                let (w, h) = ((*width).max(1), (*height).max(1));
                let data = rgba
                    .iter()
                    .copied()
                    .cycle()
                    .take((w * h * 4) as usize)
                    .collect();
                slot.set(Frame {
                    width: w,
                    height: h,
                    data,
                    seq: next_seq(),
                });
            }
            SourceKind::Image { path } => match image::open(path) {
                Ok(img) => {
                    let img = img.to_rgba8();
                    slot.set(Frame {
                        width: img.width(),
                        height: img.height(),
                        data: img.into_raw(),
                        seq: next_seq(),
                    });
                }
                Err(e) => status.fail(format!("Image illisible ({path}) : {e}")),
            },
            SourceKind::Display { monitor } => {
                platform::start_display(*monitor, fps, slot.clone(), status.clone())
            }
            SourceKind::Window { title } => {
                platform::start_window(title.clone(), fps, slot.clone(), status.clone())
            }
        }
        Self {
            slot,
            status,
            kind: kind.clone(),
        }
    }

    pub fn kind(&self) -> &SourceKind {
        &self.kind
    }

    pub fn error(&self) -> Option<String> {
        self.status.error.lock().clone()
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        self.status.stop.store(true, Ordering::Relaxed);
    }
}

/// Écran disponible pour la capture.
pub struct MonitorInfo {
    pub index: usize,
    pub name: String,
}

#[cfg(windows)]
mod platform {
    use super::*;
    use windows_capture::capture::{Context, GraphicsCaptureApiHandler};
    use windows_capture::frame::Frame as WcFrame;
    use windows_capture::graphics_capture_api::InternalCaptureControl;
    use windows_capture::monitor::Monitor;
    use windows_capture::settings::{
        ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
        GraphicsCaptureItemType, MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
    };
    use windows_capture::window::Window;

    type Flags = (Slot, Arc<Status>);

    struct Handler {
        slot: Slot,
        status: Arc<Status>,
        scratch: Vec<u8>,
    }

    impl GraphicsCaptureApiHandler for Handler {
        type Flags = Flags;
        type Error = Box<dyn std::error::Error + Send + Sync>;

        fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
            Ok(Self {
                slot: ctx.flags.0,
                status: ctx.flags.1,
                scratch: Vec::new(),
            })
        }

        fn on_frame_arrived(
            &mut self,
            frame: &mut WcFrame,
            control: InternalCaptureControl,
        ) -> Result<(), Self::Error> {
            if self.status.stop.load(Ordering::Relaxed) {
                control.stop();
                return Ok(());
            }
            let buffer = frame.buffer()?;
            let (width, height) = (buffer.width(), buffer.height());
            let data = buffer.as_nopadding_buffer(&mut self.scratch).to_vec();
            self.slot.set(Frame {
                width,
                height,
                data,
                seq: next_seq(),
            });
            self.status.frame();
            Ok(())
        }

        fn on_closed(&mut self) -> Result<(), Self::Error> {
            self.status
                .fail("La fenêtre ou l'écran capturé a disparu".into());
            Ok(())
        }
    }

    fn settings<T: TryInto<GraphicsCaptureItemType>>(
        item: T,
        cursor: bool,
        fps: u32,
        slot: Slot,
        status: Arc<Status>,
    ) -> Settings<Flags, T> {
        Settings::new(
            item,
            if cursor {
                CursorCaptureSettings::WithCursor
            } else {
                CursorCaptureSettings::WithoutCursor
            },
            // Pas de cadre jaune autour de la zone capturée (Windows 11).
            DrawBorderSettings::WithoutBorder,
            SecondaryWindowSettings::Default,
            // Au plus <fps> images par seconde, même si le jeu tourne à 120 i/s ou plus.
            MinimumUpdateIntervalSettings::Custom(Duration::from_secs_f64(1.0 / fps.max(1) as f64)),
            DirtyRegionSettings::Default,
            ColorFormat::Rgba8,
            (slot, status),
        )
    }

    pub fn start_display(index: usize, fps: u32, slot: Slot, status: Arc<Status>) {
        std::thread::spawn(move || {
            let monitor = match Monitor::from_index(index + 1).or_else(|_| Monitor::primary()) {
                Ok(m) => m,
                Err(e) => return status.fail(format!("Écran introuvable : {e}")),
            };
            if let Err(e) = Handler::start(settings(monitor, true, fps, slot, status.clone())) {
                status.fail(format!("Capture d'écran impossible : {e}"));
            }
        });
    }

    pub fn start_window(title: String, fps: u32, slot: Slot, status: Arc<Status>) {
        std::thread::spawn(move || {
            // Comme OBS : la fenêtre (jeu pas encore lancé) est recherchée toutes les 2 s.
            while !status.stop.load(Ordering::Relaxed) {
                match Window::from_contains_name(&title) {
                    Ok(window) => {
                        *status.error.lock() = None;
                        if let Err(e) = Handler::start(settings(
                            window,
                            false,
                            fps,
                            slot.clone(),
                            status.clone(),
                        )) {
                            status.fail(format!("Capture de « {title} » impossible : {e}"));
                        }
                    }
                    Err(_) => {
                        *status.error.lock() = Some(format!(
                            "Fenêtre « {title} » introuvable : nouvelle recherche toutes les 2 s"
                        ))
                    }
                }
                std::thread::sleep(Duration::from_secs(2));
            }
        });
    }

    pub fn monitors() -> Vec<MonitorInfo> {
        Monitor::enumerate()
            .unwrap_or_default()
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let size = format!("{}×{}", m.width().unwrap_or(0), m.height().unwrap_or(0));
                MonitorInfo {
                    index: i,
                    name: format!("Écran {} ({size})", i + 1),
                }
            })
            .collect()
    }

    pub fn windows() -> Vec<String> {
        let mut list: Vec<String> = Window::enumerate()
            .unwrap_or_default()
            .iter()
            .filter_map(|w| w.title().ok())
            .filter(|t| !t.trim().is_empty())
            .collect();
        list.sort();
        list.dedup();
        list
    }
}

#[cfg(not(windows))]
mod platform {
    //! Hors Windows (développement et tests) : capture d'écran simple via xcap.
    use super::*;

    pub fn start_display(index: usize, fps: u32, slot: Slot, status: Arc<Status>) {
        std::thread::spawn(move || {
            let monitors = match xcap::Monitor::all() {
                Ok(m) => m,
                Err(e) => return status.fail(format!("Écrans introuvables : {e}")),
            };
            let Some(monitor) = monitors.into_iter().nth(index) else {
                return status.fail("Écran introuvable".into());
            };
            let period = Duration::from_secs_f64(1.0 / fps.max(1) as f64);
            while !status.stop.load(Ordering::Relaxed) {
                let t = Instant::now();
                match monitor.capture_image() {
                    Ok(img) => {
                        slot.set(Frame {
                            width: img.width(),
                            height: img.height(),
                            data: img.into_raw(),
                            seq: next_seq(),
                        });
                        status.frame();
                    }
                    Err(e) => {
                        status.fail(format!("Capture d'écran impossible : {e}"));
                        std::thread::sleep(Duration::from_secs(1));
                    }
                }
                if let Some(rest) = period.checked_sub(t.elapsed()) {
                    std::thread::sleep(rest);
                }
            }
        });
    }

    pub fn start_window(title: String, _fps: u32, _slot: Slot, status: Arc<Status>) {
        status.fail(format!(
            "Capture de fenêtre (« {title} ») disponible sous Windows uniquement"
        ));
    }

    pub fn monitors() -> Vec<MonitorInfo> {
        xcap::Monitor::all()
            .unwrap_or_default()
            .iter()
            .enumerate()
            .map(|(i, m)| MonitorInfo {
                index: i,
                name: format!(
                    "Écran {} ({}×{})",
                    i + 1,
                    m.width().unwrap_or(0),
                    m.height().unwrap_or(0)
                ),
            })
            .collect()
    }

    pub fn windows() -> Vec<String> {
        Vec::new()
    }
}

pub use platform::{monitors, windows};
