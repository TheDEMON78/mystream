//! Son : micro, son du bureau (boucle WASAPI sous Windows) et une piste par
//! source sonore (vidéo, jeu, application), mixés en stéréo 48 kHz par blocs
//! de 10 ms et envoyés aux sorties (FFmpeg).

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::Sender;
use parking_lot::Mutex;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const RATE: u32 = 48000;
const BLOCK: usize = 480; // 10 ms
const MAX_BUFFER: usize = RATE as usize / 5; // au-delà de 200 ms de retard, on rattrape

/// Identifiants des pistes fixes (les autres pistes portent l'identifiant de leur source).
pub const MIC: u64 = u64::MAX;
pub const DESKTOP: u64 = u64::MAX - 1;

/// Une piste du mixeur.
#[derive(Default)]
pub struct Track {
    /// Échantillons stéréo entrelacés, 48 kHz, en attente d'être mixés.
    ring: Mutex<VecDeque<f32>>,
    /// Niveau crête récent (0 à 1), pour le vumètre.
    pub level: Mutex<f32>,
    pub error: Mutex<Option<String>>,
    /// Volume (0 à 2) et coupure.
    pub gain: Mutex<(f32, bool)>,
}

impl Track {
    /// Ajoute des échantillons stéréo 48 kHz.
    pub fn push(&self, samples: &[f32]) {
        self.ring.lock().extend(samples.iter().copied());
    }
    pub fn fail(&self, msg: impl Into<String>) {
        let msg = msg.into();
        log::warn!("{msg}");
        *self.error.lock() = Some(msg);
    }
}

/// Convertit un bloc d'un appareil (n canaux, fréquence quelconque) en stéréo 48 kHz.
pub struct Converter {
    channels: usize,
    ratio: f64, // fréquence de l'appareil / 48 kHz
    pos: f64,
    last: [f32; 2],
}

impl Converter {
    pub fn new(channels: usize, rate: u32) -> Self {
        Self {
            channels: channels.max(1),
            ratio: rate as f64 / RATE as f64,
            pos: 0.0,
            last: [0.0; 2],
        }
    }

    pub fn convert(&mut self, input: &[f32], out: &mut Vec<f32>) {
        let frames = input.len() / self.channels;
        let ch = self.channels;
        let frame = |i: usize| -> [f32; 2] {
            let l = input[i * ch];
            let r = if ch > 1 { input[i * ch + 1] } else { l };
            [l, r]
        };
        if (self.ratio - 1.0).abs() < 1e-6 {
            for i in 0..frames {
                out.extend_from_slice(&frame(i));
            }
            return;
        }
        // Rééchantillonnage linéaire (44,1 kHz -> 48 kHz par exemple).
        while self.pos < frames as f64 {
            let i = self.pos.floor() as usize;
            let t = (self.pos - i as f64) as f32;
            let a = if i == 0 { self.last } else { frame(i - 1) };
            let b = frame(i);
            out.push(a[0] + (b[0] - a[0]) * t);
            out.push(a[1] + (b[1] - a[1]) * t);
            self.pos += self.ratio;
        }
        self.pos -= frames as f64;
        if frames > 0 {
            self.last = frame(frames - 1);
        }
    }
}

/// Ouvre un appareil cpal (entrée, ou sortie en boucle WASAPI) vers une piste.
fn open_device(
    track: Arc<Track>,
    name: &str,
    device: Option<cpal::Device>,
) -> Option<cpal::Stream> {
    let result = (|| -> Result<cpal::Stream, String> {
        let device = device.ok_or("aucun appareil trouvé")?;
        let config = device
            .default_input_config()
            .or_else(|_| device.default_output_config())
            .map_err(|e| e.to_string())?;
        let format = config.sample_format();
        let config: cpal::StreamConfig = config.into();
        let mut conv = Converter::new(config.channels as usize, config.sample_rate);
        let t2 = track.clone();
        let t3 = track.clone();
        let on_error = move |e: cpal::Error| *t3.error.lock() = Some(e.to_string());
        let mut buf = Vec::new();
        let stream = match format {
            cpal::SampleFormat::F32 => device.build_input_stream::<f32, _, _>(
                config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    buf.clear();
                    conv.convert(data, &mut buf);
                    t2.push(&buf);
                },
                on_error,
                None,
            ),
            cpal::SampleFormat::I16 => device.build_input_stream::<i16, _, _>(
                config,
                move |data: &[i16], _: &cpal::InputCallbackInfo| {
                    let f: Vec<f32> = data.iter().map(|s| *s as f32 / 32768.0).collect();
                    buf.clear();
                    conv.convert(&f, &mut buf);
                    t2.push(&buf);
                },
                on_error,
                None,
            ),
            other => return Err(format!("format audio non pris en charge : {other:?}")),
        }
        .map_err(|e| e.to_string())?;
        stream.play().map_err(|e| e.to_string())?;
        Ok(stream)
    })();
    match result {
        Ok(s) => Some(s),
        Err(e) => {
            track.fail(format!("{name} : {e}"));
            None
        }
    }
}

pub struct Mixer {
    tracks: Arc<Mutex<HashMap<u64, Arc<Track>>>>,
    sinks: Arc<Mutex<Vec<Sender<Arc<Vec<f32>>>>>>,
    _streams: Vec<cpal::Stream>,
}

impl Mixer {
    pub fn new() -> Self {
        let host = cpal::default_host();
        let tracks: Arc<Mutex<HashMap<u64, Arc<Track>>>> = Arc::default();
        let mic = Arc::new(Track::default());
        let desktop = Arc::new(Track::default());
        *mic.gain.lock() = (1.0, false);
        *desktop.gain.lock() = (1.0, false);
        let mut streams = Vec::new();
        streams.extend(open_device(
            mic.clone(),
            "Micro",
            host.default_input_device(),
        ));
        // Sortie ouverte en entrée : WASAPI capture tout ce que le PC joue (boucle).
        streams.extend(open_device(
            desktop.clone(),
            "Son du bureau",
            host.default_output_device(),
        ));
        tracks.lock().insert(MIC, mic);
        tracks.lock().insert(DESKTOP, desktop);
        let mixer = Self {
            tracks,
            sinks: Arc::default(),
            _streams: streams,
        };
        mixer.spawn();
        mixer
    }

    /// Piste d'une source (créée au besoin).
    pub fn track(&self, id: u64) -> Arc<Track> {
        self.tracks
            .lock()
            .entry(id)
            .or_insert_with(|| {
                let t = Track::default();
                *t.gain.lock() = (1.0, false);
                Arc::new(t)
            })
            .clone()
    }

    pub fn get(&self, id: u64) -> Option<Arc<Track>> {
        self.tracks.lock().get(&id).cloned()
    }

    /// Retire les pistes des sources qui n'existent plus.
    pub fn retain(&self, alive: impl Fn(u64) -> bool) {
        self.tracks
            .lock()
            .retain(|id, _| *id == MIC || *id == DESKTOP || alive(*id));
    }

    /// Reçoit le mixage (blocs de 10 ms, stéréo entrelacée, 48 kHz).
    pub fn subscribe(&self, tx: Sender<Arc<Vec<f32>>>) {
        self.sinks.lock().push(tx);
    }

    fn spawn(&self) {
        let tracks = self.tracks.clone();
        let sinks = self.sinks.clone();
        std::thread::Builder::new()
            .name("mixeur".into())
            .spawn(move || {
                let period = Duration::from_millis(10);
                let mut next = Instant::now() + period;
                loop {
                    let list: Vec<Arc<Track>> = tracks.lock().values().cloned().collect();
                    let mut mix = vec![0.0f32; BLOCK * 2];
                    for track in list {
                        let (vol, muted) = *track.gain.lock();
                        let vol = if muted { 0.0 } else { vol };
                        let mut r = track.ring.lock();
                        // Horloges des appareils légèrement différentes : on rattrape un retard trop grand.
                        let excess = r.len().saturating_sub(MAX_BUFFER * 2);
                        r.drain(..excess);
                        let mut peak = 0.0f32;
                        for s in mix.iter_mut() {
                            let v = r.pop_front().unwrap_or(0.0) * vol;
                            peak = peak.max(v.abs());
                            *s += v;
                        }
                        drop(r);
                        let mut l = track.level.lock();
                        *l = peak.max(*l * 0.85); // retombée douce du vumètre
                    }
                    for s in mix.iter_mut() {
                        *s = s.clamp(-1.0, 1.0);
                    }
                    let block = Arc::new(mix);
                    // Sortie trop lente : bloc perdu pour elle ; sortie fermée : retirée.
                    sinks.lock().retain(|tx| {
                        !matches!(
                            tx.try_send(block.clone()),
                            Err(crossbeam_channel::TrySendError::Disconnected(_))
                        )
                    });
                    // Cadence fixe de 10 ms, sans dérive.
                    let now = Instant::now();
                    if next > now {
                        std::thread::sleep(next - now);
                    } else if now - next > Duration::from_millis(100) {
                        next = now;
                    }
                    next += period;
                }
            })
            .expect("fil du mixeur");
    }
}
