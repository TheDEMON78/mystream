//! Moteur de rendu : un fil dédié compose chaque image à l'instant exact
//! (horloge haute résolution, sans dépendre de l'interface) et l'envoie aux
//! sorties. L'interface ne fait que lire la texture du canevas pour l'aperçu.

use crate::capture::{Frame, Slot};
use crate::compositor::{Compositor, DrawItem};
use crate::model::TransitionKind;
use crate::output::{Kind, Output};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

#[derive(Default, Clone)]
pub struct EngineStats {
    pub fps: f32,
    pub render_ms: f32,
    /// Images que le rendu n'a pas pu produire à temps.
    pub lagged: u64,
    /// Images qu'une sortie n'a pas pu prendre (FFmpeg en retard).
    pub dropped: u64,
}

/// État partagé entre l'interface et le moteur.
#[derive(Default)]
pub struct Shared {
    /// Éléments à dessiner (scène à l'antenne), mis à jour par l'interface.
    pub items: Mutex<Vec<DrawItem>>,
    /// Scène en préparation (mode studio), dessinée dans une seconde texture.
    pub studio: Mutex<Option<Vec<DrawItem>>>,
    /// Transition en cours vers la scène à l'antenne.
    pub transition: Mutex<Option<TransitionRun>>,
    /// Dernière image de chaque source.
    pub slots: Mutex<HashMap<u64, Slot>>,
    /// Taille de chaque source (pour placer les éléments dans l'interface).
    pub sizes: Mutex<HashMap<u64, (u32, u32)>>,
    pub outputs: Mutex<HashMap<KindKey, Output>>,
    pub stats: Mutex<EngineStats>,
}

/// Transition : la scène précédente (figée au moment du changement) laisse place à la nouvelle.
pub struct TransitionRun {
    pub from: Vec<DrawItem>,
    pub kind: TransitionKind,
    pub start: Instant,
    pub duration: Duration,
    pub stinger: Option<Stinger>,
    /// Glissement : "left", "right", "up" ou "down".
    pub direction: String,
}

/// Vidéo du stinger en cours de lecture.
pub struct Stinger {
    pub runtime: crate::capture::Runtime,
    pub point: Duration,
    /// Arrivée de la première image (la lecture commence vraiment là).
    pub first: Option<Instant>,
}

/// Identifiant réservé à l'image du stinger dans le compositeur.
pub const STINGER: u64 = u64::MAX - 8;

/// Stinger : scène de départ puis d'arrivée (au point de coupe), vidéo par-dessus.
fn blend_stinger(to: &[DrawItem], run: &mut TransitionRun, width: f32, height: f32, frames: &mut HashMap<u64, Arc<Frame>>) -> Option<Vec<DrawItem>> {
    let s = run.stinger.as_mut()?;
    let frame = s.runtime.slot.get();
    if s.first.is_none() {
        if frame.is_some() {
            s.first = Some(Instant::now());
        } else if run.start.elapsed() < Duration::from_secs(1) {
            return Some(run.from.clone()); // la vidéo démarre : on garde l'ancienne scène
        } else {
            log::warn!("Stinger : aucune image reçue en 1 s, simple coupure");
            return None; // vidéo illisible : simple coupure
        }
    }
    let t = s.first.unwrap().elapsed();
    let ended = s.runtime.status.ended.load(Ordering::Relaxed);
    if (ended && t >= s.point) || t > Duration::from_secs(30) {
        log::debug!("Stinger terminé après {} ms", t.as_millis());
        return None;
    }
    let mut out = Vec::new();
    if let Some(f) = frame {
        frames.insert(STINGER, f);
        out.push(DrawItem { source: STINGER, x: 0.0, y: 0.0, w: width, h: height, filters: Default::default(), alpha: 1.0 });
    }
    out.extend_from_slice(if t < s.point { &run.from } else { to });
    Some(out)
}

/// Liste à dessiner pendant une transition (premier plan en tête de liste).
fn blend(to: &[DrawItem], run: &TransitionRun, width: f32, height: f32) -> Option<Vec<DrawItem>> {
    let p = run.start.elapsed().as_secs_f32() / run.duration.as_secs_f32().max(0.001);
    if p >= 1.0 || run.kind == TransitionKind::Cut {
        return None;
    }
    let e = if p < 0.5 { 4.0 * p * p * p } else { 1.0 - (-2.0 * p + 2.0).powi(3) / 2.0 }; // accélération douce
    let mut out: Vec<DrawItem> = to.to_vec();
    let mut from: Vec<DrawItem> = run.from.clone();
    match run.kind {
        TransitionKind::Fade => out.iter_mut().for_each(|i| i.alpha *= p),
        TransitionKind::Slide => {
            // La nouvelle scène pousse l'ancienne dans le sens choisi.
            let (dx, dy) = match run.direction.as_str() {
                "right" => (width, 0.0),
                "up" => (0.0, -height),
                "down" => (0.0, height),
                _ => (-width, 0.0),
            };
            from.iter_mut().for_each(|i| {
                i.x += e * dx;
                i.y += e * dy;
            });
            out.iter_mut().for_each(|i| {
                i.x -= (1.0 - e) * dx;
                i.y -= (1.0 - e) * dy;
            });
        }
        TransitionKind::Cut | TransitionKind::Stinger => {}
    }
    out.extend(from);
    Some(out)
}

pub type KindKey = u8;
pub fn key(kind: Kind) -> KindKey {
    match kind {
        Kind::Stream => 0,
        Kind::Record => 1,
    }
}

pub struct Engine {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

/// Attend l'instant `t` avec une précision d'environ 0,1 ms : sommeil jusqu'à
/// 1 ms avant (minuteur haute résolution de Windows), puis attente active.
fn wait_until(t: Instant) {
    loop {
        let now = Instant::now();
        if now >= t {
            return;
        }
        let left = t - now;
        if left > Duration::from_micros(1500) {
            std::thread::sleep(left - Duration::from_millis(1));
        } else {
            std::hint::spin_loop();
            std::thread::yield_now();
        }
    }
}

impl Engine {
    pub fn start(mut comp: Compositor, fps: u32, shared: Arc<Shared>, ctx: egui::Context) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let thread = std::thread::Builder::new()
            .name("rendu".into())
            .spawn(move || {
                let period = Duration::from_secs_f64(1.0 / fps.max(1) as f64);
                let mut next = Instant::now();
                let mut window = (Instant::now(), 0u32);
                let mut last: Option<Arc<Vec<u8>>> = None;
                let mut render_ms = 0.0f32;
                while !stop2.load(Ordering::Relaxed) {
                    wait_until(next);
                    let t0 = Instant::now();
                    // En retard d'une image ou plus (PC surchargé) : on compte et on se recale.
                    let behind = t0.saturating_duration_since(next);
                    if behind >= period {
                        let missed = (behind.as_secs_f64() / period.as_secs_f64()).floor() as u64;
                        shared.stats.lock().lagged += missed;
                        next += period * missed as u32;
                    }

                    let mut frames: HashMap<u64, Arc<Frame>> = shared.slots.lock().iter().filter_map(|(id, s)| s.get().map(|f| (*id, f))).collect();
                    {
                        let mut sizes = shared.sizes.lock();
                        for (id, f) in &frames {
                            sizes.insert(*id, (f.width, f.height));
                        }
                    }
                    let live = !shared.outputs.lock().is_empty();
                    let nv12 = {
                        let items = shared.items.lock();
                        let mut tr = shared.transition.lock();
                        let (cw, ch) = (comp.width as f32, comp.height as f32);
                        let blended = tr.as_mut().and_then(|run| if run.kind == TransitionKind::Stinger { blend_stinger(&items, run, cw, ch, &mut frames) } else { blend(&items, run, cw, ch) });
                        if blended.is_none() {
                            *tr = None;
                        }
                        drop(tr);
                        let studio = shared.studio.lock();
                        comp.render(blended.as_deref().unwrap_or(&items), studio.as_deref(), &frames, live)
                    };
                    if live {
                        // Une image par période, toujours : si la relecture n'est pas prête,
                        // on renvoie la précédente pour garder la cadence (et la synchro du son).
                        if let Some(f) = nv12 {
                            last = Some(Arc::new(f));
                        }
                        if let Some(f) = &last {
                            let outputs = shared.outputs.lock();
                            let mut dropped = 0;
                            for out in outputs.values() {
                                // Les 2 premières secondes, FFmpeg démarre : les images refusées ne sont pas des pertes.
                                if !out.push(f.clone()) && out.started.elapsed() > Duration::from_secs(2) {
                                    dropped += 1;
                                }
                            }
                            shared.stats.lock().dropped += dropped;
                        }
                    } else {
                        last = None;
                    }
                    comp.retain(|id| frames.contains_key(&id));

                    render_ms = render_ms * 0.9 + t0.elapsed().as_secs_f32() * 1000.0 * 0.1;
                    window.1 += 1;
                    let el = window.0.elapsed().as_secs_f32();
                    if el >= 1.0 {
                        let mut s = shared.stats.lock();
                        s.fps = window.1 as f32 / el;
                        s.render_ms = render_ms;
                        window = (Instant::now(), 0);
                    }
                    ctx.request_repaint();
                    next += period;
                }
            })
            .expect("fil de rendu");
        Self { stop, thread: Some(thread) }
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
