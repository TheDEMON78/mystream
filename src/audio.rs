//! Son : micro et son du bureau (boucle WASAPI sous Windows), mixés en
//! stéréo 48 kHz par blocs de 10 ms et envoyés aux sorties (FFmpeg).

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::Sender;
use parking_lot::Mutex;
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const RATE: u32 = 48000;
const BLOCK: usize = 480; // 10 ms
const MAX_BUFFER: usize = RATE as usize / 5; // au-delà de 200 ms de retard, on rattrape

/// Échantillons stéréo entrelacés en attente d'être mixés.
#[derive(Default)]
struct Ring {
    samples: VecDeque<f32>,
}

/// Une entrée du mixeur (micro ou son du bureau).
pub struct Channel {
    ring: Arc<Mutex<Ring>>,
    /// Niveau crête du dernier bloc (0 à 1), pour le vumètre.
    pub level: Arc<Mutex<f32>>,
    pub error: Arc<Mutex<Option<String>>>,
    _stream: Option<cpal::Stream>,
}

pub struct Mixer {
    pub mic: Channel,
    pub desktop: Channel,
    /// Volume (0 à 2) et coupure, réglés par l'interface.
    pub gains: Arc<Mutex<Gains>>,
    sinks: Arc<Mutex<Vec<Sender<Arc<Vec<f32>>>>>>,
}

#[derive(Clone, Copy)]
pub struct Gains {
    pub mic: f32,
    pub mic_muted: bool,
    pub desktop: f32,
    pub desktop_muted: bool,
}

/// Convertit un bloc de l'appareil (n canaux, fréquence quelconque) en stéréo 48 kHz.
struct Converter {
    channels: usize,
    ratio: f64, // fréquence de l'appareil / 48 kHz
    pos: f64,
    last: [f32; 2],
}

impl Converter {
    fn push(&mut self, input: &[f32], out: &mut VecDeque<f32>) {
        let frames = input.len() / self.channels.max(1);
        let frame = |i: usize| -> [f32; 2] {
            let base = i * self.channels;
            let l = input[base];
            let r = if self.channels > 1 {
                input[base + 1]
            } else {
                l
            };
            [l, r]
        };
        if (self.ratio - 1.0).abs() < 1e-6 {
            for i in 0..frames {
                let [l, r] = frame(i);
                out.push_back(l);
                out.push_back(r);
            }
            return;
        }
        // Rééchantillonnage linéaire (44,1 kHz -> 48 kHz par exemple).
        while self.pos < frames as f64 {
            let i = self.pos.floor() as usize;
            let t = (self.pos - i as f64) as f32;
            let a = if i == 0 { self.last } else { frame(i - 1) };
            let b = frame(i);
            out.push_back(a[0] + (b[0] - a[0]) * t);
            out.push_back(a[1] + (b[1] - a[1]) * t);
            self.pos += self.ratio;
        }
        self.pos -= frames as f64;
        if frames > 0 {
            self.last = frame(frames - 1);
        }
    }
}

fn open(name: &'static str, device: Option<cpal::Device>) -> Channel {
    let ring = Arc::new(Mutex::new(Ring::default()));
    let level = Arc::new(Mutex::new(0.0));
    let error = Arc::new(Mutex::new(None));
    let stream = (|| -> Result<cpal::Stream, String> {
        let device = device.ok_or("aucun appareil trouvé")?;
        // Pour le son du bureau, `device` est une sortie : WASAPI l'ouvre en boucle (loopback).
        let config = device
            .default_input_config()
            .or_else(|_| device.default_output_config())
            .map_err(|e| e.to_string())?;
        let sample_format = config.sample_format();
        let config: cpal::StreamConfig = config.into();
        let mut conv = Converter {
            channels: config.channels as usize,
            ratio: config.sample_rate as f64 / RATE as f64,
            pos: 0.0,
            last: [0.0; 2],
        };
        let ring2 = ring.clone();
        let err2 = error.clone();
        let on_error = move |e: cpal::Error| *err2.lock() = Some(e.to_string());
        let stream = match sample_format {
            cpal::SampleFormat::F32 => device.build_input_stream::<f32, _, _>(
                config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    let mut r = ring2.lock();
                    conv.push(data, &mut r.samples);
                },
                on_error,
                None,
            ),
            cpal::SampleFormat::I16 => device.build_input_stream::<i16, _, _>(
                config,
                move |data: &[i16], _: &cpal::InputCallbackInfo| {
                    let f: Vec<f32> = data.iter().map(|s| *s as f32 / 32768.0).collect();
                    let mut r = ring2.lock();
                    conv.push(&f, &mut r.samples);
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
    let stream = match stream {
        Ok(s) => Some(s),
        Err(e) => {
            log::warn!("{name} : {e}");
            *error.lock() = Some(e);
            None
        }
    };
    Channel {
        ring,
        level,
        error,
        _stream: stream,
    }
}

impl Mixer {
    pub fn new(gains: Gains) -> Self {
        let host = cpal::default_host();
        let mic = open("Micro", host.default_input_device());
        let desktop = open("Son du bureau", host.default_output_device());
        let mixer = Self {
            mic,
            desktop,
            gains: Arc::new(Mutex::new(gains)),
            sinks: Arc::new(Mutex::new(Vec::new())),
        };
        mixer.spawn();
        mixer
    }

    /// Reçoit le mixage (blocs de 10 ms, stéréo entrelacée, 48 kHz).
    pub fn subscribe(&self, tx: Sender<Arc<Vec<f32>>>) {
        self.sinks.lock().push(tx);
    }

    fn spawn(&self) {
        let inputs = [
            (self.mic.ring.clone(), self.mic.level.clone()),
            (self.desktop.ring.clone(), self.desktop.level.clone()),
        ];
        let gains = self.gains.clone();
        let sinks = self.sinks.clone();
        std::thread::Builder::new()
            .name("mixeur".into())
            .spawn(move || {
                let period = Duration::from_millis(10);
                let mut next = Instant::now() + period;
                loop {
                    let g = *gains.lock();
                    let volumes = [
                        if g.mic_muted { 0.0 } else { g.mic },
                        if g.desktop_muted { 0.0 } else { g.desktop },
                    ];
                    let mut mix = vec![0.0f32; BLOCK * 2];
                    for ((ring, level), vol) in inputs.iter().zip(volumes) {
                        let mut r = ring.lock();
                        // Horloges des appareils légèrement différentes : on rattrape un retard trop grand.
                        while r.samples.len() > MAX_BUFFER * 2 {
                            r.samples.pop_front();
                        }
                        let mut peak = 0.0f32;
                        for s in mix.iter_mut() {
                            let v = r.samples.pop_front().unwrap_or(0.0) * vol;
                            peak = peak.max(v.abs());
                            *s += v;
                        }
                        drop(r);
                        let mut l = level.lock();
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
