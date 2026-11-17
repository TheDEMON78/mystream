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
    /// Filtres (gain, noise gate, compresseur).
    pub filters: Mutex<crate::model::AudioFilters>,
    /// Écoute au casque.
    pub monitor: std::sync::atomic::AtomicBool,
}

/// État des filtres dynamiques d'une piste (suiveur d'enveloppe).
#[derive(Default)]
struct Dynamics {
    env: f32,
    gate: f32,
}

fn db_to_lin(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// Applique gain, noise gate et compresseur à un bloc stéréo entrelacé.
fn apply_filters(f: &crate::model::AudioFilters, d: &mut Dynamics, block: &mut [f32]) {
    let gain = db_to_lin(f.gain_db);
    // Constantes de temps à 48 kHz : attaque ~2 ms, relâchement ~120 ms.
    let attack = (-1.0f32 / (0.002 * RATE as f32)).exp();
    let release = (-1.0f32 / (0.12 * RATE as f32)).exp();
    let gate_open = (-1.0f32 / (0.003 * RATE as f32)).exp();
    let gate_close = (-1.0f32 / (0.15 * RATE as f32)).exp();
    let gate_thr = db_to_lin(f.gate_threshold_db);
    let comp_thr = db_to_lin(f.comp_threshold_db);
    for frame in block.chunks_exact_mut(2) {
        frame[0] *= gain;
        frame[1] *= gain;
        let x = frame[0].abs().max(frame[1].abs());
        d.env = if x > d.env { attack * d.env + (1.0 - attack) * x } else { release * d.env + (1.0 - release) * x };
        let mut g = 1.0;
        if f.gate {
            let target = if d.env >= gate_thr { 1.0 } else { 0.0 };
            let k = if target > d.gate { gate_open } else { gate_close };
            d.gate = k * d.gate + (1.0 - k) * target;
            g *= d.gate;
        }
        if f.compressor && d.env > comp_thr {
            let over_db = 20.0 * (d.env / comp_thr).log10();
            g *= db_to_lin(-over_db * (1.0 - 1.0 / f.comp_ratio.max(1.0)));
        }
        frame[0] *= g;
        frame[1] *= g;
    }
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
        Self { channels: channels.max(1), ratio: rate as f64 / RATE as f64, pos: 0.0, last: [0.0; 2] }
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
/// Suppression du bruit (RNNoise) d'un son stéréo 48 kHz : traité en mono, par blocs de 10 ms.
pub struct Denoiser {
    state: Box<nnnoiseless::DenoiseState<'static>>,
    input: Vec<f32>,
    out: Vec<f32>,
}

impl Default for Denoiser {
    fn default() -> Self {
        Self { state: nnnoiseless::DenoiseState::new(), input: Vec::new(), out: vec![0.0; nnnoiseless::DenoiseState::FRAME_SIZE] }
    }
}

impl Denoiser {
    /// Ajoute du son et renvoie le son débruité disponible (stéréo entrelacé).
    pub fn process(&mut self, stereo: &[f32], result: &mut Vec<f32>) {
        // RNNoise attend des valeurs à l'échelle des entiers 16 bits.
        self.input.extend(stereo.chunks_exact(2).map(|c| (c[0] + c[1]) * 0.5 * 32768.0));
        let n = nnnoiseless::DenoiseState::FRAME_SIZE;
        let mut done = 0;
        while self.input.len() - done >= n {
            self.state.process_frame(&mut self.out, &self.input[done..done + n]);
            for v in &self.out {
                let v = v / 32768.0;
                result.extend([v, v]);
            }
            done += n;
        }
        self.input.drain(..done);
    }
}

/// Noms des micros et entrées audio.
pub fn input_devices() -> Vec<String> {
    let host = cpal::default_host();
    host.input_devices().map(|d| d.filter_map(|d| d.description().ok().map(|x| x.name().to_string())).collect()).unwrap_or_default()
}

/// Source « Entrée audio » : un micro choisi (vide : celui par défaut), éventuellement débruité.
pub fn start_input(device: String, denoise: bool, track: Arc<Track>, stop: Arc<std::sync::atomic::AtomicBool>) {
    // Le flux cpal reste dans son fil (il n'est pas transférable sous tous les systèmes).
    std::thread::spawn(move || {
        let host = cpal::default_host();
        let dev = if device.is_empty() {
            host.default_input_device()
        } else {
            host.input_devices().ok().and_then(|mut d| d.find(|d| d.description().is_ok_and(|x| x.name() == device)))
        };
        if dev.is_none() && !device.is_empty() {
            track.fail(format!("« {device} » introuvable : est-il branché ?"));
            return;
        }
        let stream = open_device_with(track.clone(), "Entrée audio", dev, denoise);
        while stream.is_some() && !stop.load(std::sync::atomic::Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(200));
        }
        drop(stream);
    });
}

fn open_device(track: Arc<Track>, name: &str, device: Option<cpal::Device>) -> Option<cpal::Stream> {
    open_device_with(track, name, device, false)
}

fn open_device_with(track: Arc<Track>, name: &str, device: Option<cpal::Device>, denoise: bool) -> Option<cpal::Stream> {
    let result = (|| -> Result<cpal::Stream, String> {
        let device = device.ok_or("aucun appareil trouvé")?;
        let config = device.default_input_config().or_else(|_| device.default_output_config()).map_err(|e| e.to_string())?;
        let format = config.sample_format();
        let config: cpal::StreamConfig = config.into();
        let mut conv = Converter::new(config.channels as usize, config.sample_rate);
        let t2 = track.clone();
        let t3 = track.clone();
        let on_error = move |e: cpal::Error| *t3.error.lock() = Some(e.to_string());
        let mut buf = Vec::new();
        let mut clean = Vec::new();
        let mut denoiser = denoise.then(Denoiser::default);
        let mut push = move |buf: &[f32]| match denoiser.as_mut() {
            Some(d) => {
                clean.clear();
                d.process(buf, &mut clean);
                t2.push(&clean);
            }
            None => t2.push(buf),
        };
        let stream = match format {
            cpal::SampleFormat::F32 => device.build_input_stream::<f32, _, _>(
                config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    buf.clear();
                    conv.convert(data, &mut buf);
                    push(&buf);
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
                    push(&buf);
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
    /// Son de l'écoute au casque, en attente d'être joué.
    monitor: Arc<Mutex<VecDeque<f32>>>,
    _streams: Vec<cpal::Stream>,
}

/// Sortie de l'écoute au casque (périphérique de sortie par défaut).
fn open_monitor(ring: Arc<Mutex<VecDeque<f32>>>) -> Option<cpal::Stream> {
    let device = cpal::default_host().default_output_device()?;
    let config: cpal::StreamConfig = device.default_output_config().ok()?.into();
    let ch = config.channels as usize;
    let step = RATE as f64 / config.sample_rate as f64;
    let mut pos = 0.0f64;
    let mut last = [0.0f32; 2];
    let stream = device
        .build_output_stream::<f32, _, _>(
            config,
            move |out: &mut [f32], _: &cpal::OutputCallbackInfo| {
                let mut r = ring.lock();
                for frame in out.chunks_exact_mut(ch) {
                    // Rééchantillonnage au plus proche (suffisant pour l'écoute).
                    pos += step;
                    while pos >= 1.0 {
                        pos -= 1.0;
                        if let (Some(l), Some(rr)) = (r.pop_front(), r.pop_front()) {
                            last = [l, rr];
                        } else {
                            last = [0.0, 0.0];
                        }
                    }
                    for (i, s) in frame.iter_mut().enumerate() {
                        *s = last[i.min(1)];
                    }
                }
            },
            |_| {},
            None,
        )
        .ok()?;
    stream.play().ok()?;
    Some(stream)
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
        streams.extend(open_device(mic.clone(), "Micro", host.default_input_device()));
        // Son du bureau : sous Windows, tout ce que le PC joue sauf MyStream (pas d'écho de
        // l'écoute au casque) ; ailleurs, la sortie ouverte en boucle.
        if !crate::capture::system_audio(desktop.clone()) {
            streams.extend(open_device(desktop.clone(), "Son du bureau", host.default_output_device()));
        }
        tracks.lock().insert(MIC, mic);
        tracks.lock().insert(DESKTOP, desktop);
        let monitor: Arc<Mutex<VecDeque<f32>>> = Arc::default();
        streams.extend(open_monitor(monitor.clone()));
        let mixer = Self { tracks, sinks: Arc::default(), monitor, _streams: streams };
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
        self.tracks.lock().retain(|id, _| *id == MIC || *id == DESKTOP || alive(*id));
    }

    /// Reçoit le mixage (blocs de 10 ms, stéréo entrelacée, 48 kHz).
    pub fn subscribe(&self, tx: Sender<Arc<Vec<f32>>>) {
        self.sinks.lock().push(tx);
    }

    fn spawn(&self) {
        let tracks = self.tracks.clone();
        let sinks = self.sinks.clone();
        let monitor = self.monitor.clone();
        std::thread::Builder::new()
            .name("mixeur".into())
            .spawn(move || {
                let period = Duration::from_millis(10);
                let mut next = Instant::now() + period;
                let mut dynamics: HashMap<u64, Dynamics> = HashMap::new();
                loop {
                    let list: Vec<(u64, Arc<Track>)> = tracks.lock().iter().map(|(k, v)| (*k, v.clone())).collect();
                    let mut mix = vec![0.0f32; BLOCK * 2];
                    let mut listen = vec![0.0f32; BLOCK * 2];
                    let mut listening = false;
                    for (id, track) in list {
                        let (vol, muted) = *track.gain.lock();
                        let vol = if muted { 0.0 } else { vol };
                        let mut block = vec![0.0f32; BLOCK * 2];
                        {
                            let mut r = track.ring.lock();
                            // Horloges des appareils légèrement différentes : on rattrape un retard trop grand.
                            let excess = r.len().saturating_sub(MAX_BUFFER * 2);
                            r.drain(..excess);
                            for s in block.iter_mut() {
                                *s = r.pop_front().unwrap_or(0.0);
                            }
                        }
                        let filters = track.filters.lock().clone();
                        if filters != crate::model::AudioFilters::default() {
                            apply_filters(&filters, dynamics.entry(id).or_default(), &mut block);
                        }
                        let monitored = track.monitor.load(std::sync::atomic::Ordering::Relaxed) && !muted;
                        listening |= monitored;
                        let mut peak = 0.0f32;
                        for (i, s) in mix.iter_mut().enumerate() {
                            let v = block[i] * vol;
                            peak = peak.max(v.abs());
                            *s += v;
                            if monitored {
                                listen[i] += v;
                            }
                        }
                        let mut l = track.level.lock();
                        *l = peak.max(*l * 0.85); // retombée douce du vumètre
                    }
                    for s in mix.iter_mut() {
                        *s = s.clamp(-1.0, 1.0);
                    }
                    if listening {
                        let mut m = monitor.lock();
                        m.extend(listen.iter().map(|s| s.clamp(-1.0, 1.0)));
                        let excess = m.len().saturating_sub(MAX_BUFFER);
                        m.drain(..excess);
                    }
                    let block = Arc::new(mix);
                    // Sortie trop lente : bloc perdu pour elle ; sortie fermée : retirée.
                    sinks.lock().retain(|tx| !matches!(tx.try_send(block.clone()), Err(crossbeam_channel::TrySendError::Disconnected(_))));
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
