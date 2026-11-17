//! Sources : captures (écran, fenêtre, jeu), image, couleur, texte, média,
//! caméra et son d'une application.
//!
//! Chaque source tient sa dernière image dans un `Slot`, lu par le
//! compositeur à chaque image du stream, et envoie son son éventuel à sa
//! piste du mixeur. Sous Windows, les captures passent par Windows Graphics
//! Capture (comme OBS), limitées à la cadence du stream, et le son d'une
//! fenêtre, d'un jeu ou d'une application par la capture WASAPI par processus.

use crate::audio::Track;
use crate::model::SourceKind;
use parking_lot::Mutex;
use std::io::Read;
use std::process::{Child, Command, Stdio};
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

impl Frame {
    pub fn new(width: u32, height: u32, data: Vec<u8>) -> Self {
        Self { width, height, data, seq: next_seq() }
    }
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
    #[allow(dead_code)] // utilisée par la capture de jeu (Windows)
    pub fn clear(&self) {
        *self.0.lock() = None;
    }
}

static SEQ: AtomicU64 = AtomicU64::new(1);
fn next_seq() -> u64 {
    SEQ.fetch_add(1, Ordering::Relaxed)
}

/// État partagé d'une source en cours : arrêt, erreur, information, cadence mesurée.
#[derive(Default)]
pub struct Status {
    pub stop: AtomicBool,
    pub error: Mutex<Option<String>>,
    /// Information affichée sous la source (ex. jeu capturé).
    pub info: Mutex<Option<String>>,
    /// Images reçues pendant la dernière seconde.
    pub fps: Mutex<f32>,
    counter: Mutex<(u32, Option<Instant>)>,
    /// Processus FFmpeg de la source (média, caméra), arrêtés avec elle.
    children: Mutex<Vec<Child>>,
    /// La vidéo est arrivée à la fin (média lu une seule fois).
    pub ended: AtomicBool,
    /// Images ignorées (capture de l'écran du jeu pendant un Alt+Tab : la dernière image reste).
    #[cfg_attr(not(windows), allow(dead_code))]
    pub paused: AtomicBool,
    /// Images capturées en alpha prémultiplié (fenêtre transparente) : à convertir.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub premultiplied: AtomicBool,
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
    fn stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }
    fn adopt(&self, child: Child) {
        self.children.lock().push(child);
    }
}

/// Source en cours d'exécution.
pub struct Runtime {
    pub slot: Slot,
    pub status: Arc<Status>,
    kind: SourceKind,
}

impl Runtime {
    pub fn start(kind: &SourceKind, fps: u32, track: Option<Arc<Track>>, chat: &crate::chat::ChatHandle) -> Self {
        let slot = Slot::default();
        let status = Arc::new(Status::default());
        match kind {
            SourceKind::Color { rgba, width, height } => {
                let (w, h) = ((*width).max(1), (*height).max(1));
                let data = rgba.iter().copied().cycle().take((w * h * 4) as usize).collect();
                slot.set(Frame::new(w, h, data));
            }
            SourceKind::Image { path } if path.to_lowercase().ends_with(".gif") => start_gif(path.clone(), slot.clone(), status.clone()),
            SourceKind::Image { path } => match image::open(path) {
                Ok(img) => {
                    let img = img.to_rgba8();
                    slot.set(Frame::new(img.width(), img.height(), img.into_raw()));
                }
                Err(e) => status.fail(format!("Image illisible ({path}) : {e}")),
            },
            SourceKind::Text { text, size, rgba, background, mono, font, bold, italic, align, outline, outline_color, outline_size } => slot.set(render_text(
                text,
                *size,
                *rgba,
                *background,
                *mono,
                font,
                *bold,
                *italic,
                align,
                outline.then_some((*outline_color, *outline_size)),
            )),
            SourceKind::Media { path, looping } => start_media(path.clone(), *looping, slot.clone(), status.clone(), track),
            SourceKind::Camera { device, width, height, fps: cam_fps } => start_camera(device.clone(), *width, *height, *cam_fps, slot.clone(), status.clone()),
            SourceKind::Display { monitor, audio } => {
                platform::start_display(*monitor, fps, slot.clone(), status.clone());
                if *audio && let Some(t) = track {
                    platform::pc_audio(status.clone(), t);
                }
            }
            SourceKind::Window { title, .. } => platform::start_window(title.clone(), fps, slot.clone(), status.clone(), track),
            SourceKind::Game { method, .. } => platform::start_game(fps, method == "screen", slot.clone(), status.clone(), track),
            SourceKind::Chat(style) => crate::overlay::start(chat.clone(), style.clone(), slot.clone(), status.clone()),
            SourceKind::Browser { url, width, height, css, audio } => platform::start_browser(
                url.clone(),
                (*width).clamp(16, 3840),
                (*height).clamp(16, 2160),
                css.clone(),
                fps,
                slot.clone(),
                status.clone(),
                if *audio { track } else { None },
            ),
            SourceKind::AudioInput { device, noise_suppression } => {
                if let Some(t) = track {
                    let stop = Arc::new(AtomicBool::new(false));
                    crate::audio::start_input(device.clone(), *noise_suppression, t, stop.clone());
                    // Arrêtée avec la source.
                    let st = status.clone();
                    std::thread::spawn(move || {
                        while !st.stopped() {
                            std::thread::sleep(Duration::from_millis(200));
                        }
                        stop.store(true, Ordering::Relaxed);
                    });
                }
            }
            SourceKind::AppAudio { exe } => {
                if let Some(t) = track {
                    platform::start_app_audio(exe.clone(), status.clone(), t);
                }
            }
        }
        Self { slot, status, kind: kind.clone() }
    }

    pub fn kind(&self) -> &SourceKind {
        &self.kind
    }

    pub fn error(&self) -> Option<String> {
        self.status.error.lock().clone()
    }

    pub fn info(&self) -> Option<String> {
        self.status.info.lock().clone()
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        self.status.stop.store(true, Ordering::Relaxed);
        for mut c in self.status.children.lock().drain(..) {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

/* GIF animé --------------------------------------------------------------- */

fn start_gif(path: String, slot: Slot, status: Arc<Status>) {
    use image::AnimationDecoder;
    std::thread::spawn(move || {
        let frames = std::fs::File::open(&path)
            .map_err(|e| e.to_string())
            .and_then(|f| image::codecs::gif::GifDecoder::new(std::io::BufReader::new(f)).map_err(|e| e.to_string()))
            .and_then(|d| d.into_frames().collect_frames().map_err(|e| e.to_string()));
        let frames = match frames {
            Ok(f) if !f.is_empty() => f,
            Ok(_) => return status.fail(format!("GIF vide : {path}")),
            Err(e) => return status.fail(format!("GIF illisible ({path}) : {e}")),
        };
        // Images décodées une fois, puis rejouées en boucle à leur propre rythme.
        let decoded: Vec<(Arc<Frame>, Duration)> = frames
            .into_iter()
            .map(|f| {
                let (n, d) = f.delay().numer_denom_ms();
                let ms = if d == 0 { 100 } else { (n / d).max(20) };
                let img = f.into_buffer();
                (Arc::new(Frame::new(img.width(), img.height(), img.into_raw())), Duration::from_millis(ms as u64))
            })
            .collect();
        loop {
            for (frame, delay) in &decoded {
                if status.stopped() {
                    return;
                }
                // Nouveau numéro à chaque passage : le compositeur renvoie l'image à la carte graphique.
                slot.set(Frame::new(frame.width, frame.height, frame.data.clone()));
                std::thread::sleep(*delay);
            }
            if decoded.len() == 1 {
                return;
            }
        }
    });
}

/* Texte ------------------------------------------------------------------ */

#[allow(clippy::too_many_arguments)]
fn render_text(
    text: &str,
    size: f32,
    rgba: [u8; 4],
    background: [u8; 4],
    mono: bool,
    font: &str,
    bold: bool,
    italic: bool,
    align: &str,
    outline: Option<([u8; 4], f32)>,
) -> Frame {
    use std::sync::OnceLock;
    static MONO: OnceLock<fontdue::Font> = OnceLock::new();
    let fonts = crate::overlay::fonts(font);
    let monospace = MONO.get_or_init(|| fontdue::Font::from_bytes(epaint_default_fonts::HACK_REGULAR, fontdue::FontSettings::default()).expect("police intégrée"));
    let font = if mono {
        monospace
    } else if bold {
        &fonts.bold
    } else {
        &fonts.regular
    };
    let size = size.clamp(4.0, 400.0);
    let lm = font.horizontal_line_metrics(size);
    let (ascent, line_h) = lm.map(|m| (m.ascent, m.new_line_size)).unwrap_or((size * 0.8, size * 1.2));
    let lines: Vec<&str> = if text.is_empty() { vec![" "] } else { text.lines().collect() };
    let stroke = outline.map(|o| o.1.clamp(1.0, 40.0)).unwrap_or(0.0);
    // Italique : les caractères sont penchés (décalage proportionnel à la hauteur).
    let slant = if italic { 0.2 } else { 0.0 };
    let pad = (size * 0.25 + stroke).ceil() as i32 + (size * slant).ceil() as i32;
    let widths: Vec<f32> = lines.iter().map(|l| l.chars().map(|c| font.metrics(c, size).advance_width).sum()).collect();
    let max_w = widths.iter().cloned().fold(1.0, f32::max);
    let w = (max_w.ceil() as i32 + pad * 2).max(2) as usize;
    let h = ((line_h * lines.len() as f32).ceil() as i32 + pad * 2).max(2) as usize;
    // Couverture du texte (0 à 1) sur toute l'image.
    let mut mask = vec![0f32; w * h];
    for (li, line) in lines.iter().enumerate() {
        let mut x = pad as f32
            + match align {
                "center" => (max_w - widths[li]) / 2.0,
                "right" => max_w - widths[li],
                _ => 0.0,
            };
        let baseline = pad as f32 + ascent + li as f32 * line_h;
        for c in line.chars() {
            let (m, bitmap) = font.rasterize(c, size);
            let gx = x.round() as i32 + m.xmin;
            let gy = baseline.round() as i32 - m.height as i32 - m.ymin;
            for row in 0..m.height {
                let py = gy + row as i32;
                let shift = ((baseline - py as f32) * slant).round() as i32;
                for col in 0..m.width {
                    let px = gx + col as i32 + shift;
                    if px < 0 || py < 0 || px >= w as i32 || py >= h as i32 {
                        continue;
                    }
                    let i = py as usize * w + px as usize;
                    mask[i] = mask[i].max(bitmap[row * m.width + col] as f32 / 255.0);
                }
            }
            x += m.advance_width;
        }
    }
    let mut data: Vec<u8> = background.iter().copied().cycle().take(w * h * 4).collect();
    let mut blend = |i: usize, rgb: &[u8], a: f32| {
        if a <= 0.0 {
            return;
        }
        let p = &mut data[i * 4..i * 4 + 4];
        let da = p[3] as f32 / 255.0;
        let oa = a + da * (1.0 - a);
        for k in 0..3 {
            p[k] = ((rgb[k] as f32 * a + p[k] as f32 * da * (1.0 - a)) / oa.max(1e-6)).round() as u8;
        }
        p[3] = (oa * 255.0).round() as u8;
    };
    // Contour : la couverture du texte élargie d'un disque du rayon voulu.
    if let Some((oc, _)) = outline {
        let r = stroke.ceil() as i32;
        let disk: Vec<(i32, i32, f32)> = (-r..=r)
            .flat_map(|dy| (-r..=r).map(move |dx| (dx, dy)))
            .filter_map(|(dx, dy)| {
                let d = ((dx * dx + dy * dy) as f32).sqrt();
                (d <= stroke + 0.5).then_some((dx, dy, (stroke + 0.5 - d).min(1.0)))
            })
            .collect();
        let mut grown = vec![0f32; w * h];
        for y in 0..h as i32 {
            for x in 0..w as i32 {
                let m = mask[y as usize * w + x as usize];
                if m <= 0.0 {
                    continue;
                }
                for &(dx, dy, k) in &disk {
                    let (nx, ny) = (x + dx, y + dy);
                    if nx >= 0 && ny >= 0 && nx < w as i32 && ny < h as i32 {
                        let g = &mut grown[ny as usize * w + nx as usize];
                        *g = g.max(m * k);
                    }
                }
            }
        }
        for (i, g) in grown.iter().enumerate() {
            blend(i, &oc[..3], g * oc[3] as f32 / 255.0);
        }
    }
    for (i, m) in mask.iter().enumerate() {
        blend(i, &rgba[..3], m * rgba[3] as f32 / 255.0);
    }
    Frame::new(w as u32, h as u32, data)
}

/* FFmpeg : média et caméra ------------------------------------------------ */

fn ffmpeg() -> Command {
    #[allow(unused_mut)]
    let mut c = Command::new(crate::output::ffmpeg_path());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // pas de fenêtre de console
    }
    c.arg("-hide_banner");
    c
}

/// Taille de la vidéo et présence de son d'un fichier, lues dans la sortie de « ffmpeg -i ».
fn probe(path: &str) -> (Option<(u32, u32)>, bool, String, f64) {
    let out = ffmpeg().args(["-i", path]).stdout(Stdio::null()).stderr(Stdio::piped()).output();
    let text = out.map(|o| String::from_utf8_lossy(&o.stderr).into_owned()).unwrap_or_default();
    let size = text
        .lines()
        .filter(|l| l.contains("Video:") && !l.contains("attached pic"))
        .flat_map(|l| l.split([' ', ',']))
        .find_map(|t| {
            let (a, b) = t.split_once('x')?;
            Some((a.parse::<u32>().ok()?, b.parse::<u32>().ok()?))
        })
        .filter(|(w, h)| *w > 0 && *h > 0);
    // Codec vidéo : les WebM avec transparence doivent être lus par libvpx (le décodeur
    // intégré de FFmpeg ignore la couche alpha).
    let codec = text.lines().find(|l| l.contains("Video:")).and_then(|l| l.split("Video:").nth(1)).map(|c| c.trim().split([' ', ',']).next().unwrap_or("").to_string()).unwrap_or_default();
    let alpha = text.contains("alpha_mode") || text.contains("ALPHA_MODE");
    // Fréquence d'images (« 30 fps », « 29.97 fps », sinon « 30 tbr »).
    let rate = |unit: &str| {
        text.lines().filter(|l| l.contains("Video:")).find_map(|l| {
            let words: Vec<&str> = l.split([' ', ',']).filter(|w| !w.is_empty()).collect();
            words.windows(2).find(|w| w[1] == unit).and_then(|w| w[0].parse::<f64>().ok())
        })
    };
    let fps = rate("fps").or_else(|| rate("tbr")).filter(|f| *f > 1.0 && *f <= 240.0).unwrap_or(30.0);
    (size, text.contains("Audio:"), if alpha { codec } else { String::new() }, fps)
}

/// Lit des images RGBA de taille fixe sur la sortie d'un processus FFmpeg.
/// Avec `pace`, les images sont cadencées à cette fréquence (l'option -re de
/// FFmpeg envoie les premières images en rafale).
fn pump_video(mut child: Child, w: u32, h: u32, slot: Slot, status: Arc<Status>, pace: Option<f64>) {
    let mut out = child.stdout.take().unwrap();
    status.adopt(child);
    std::thread::spawn(move || {
        let mut buf = vec![0u8; (w * h * 4) as usize];
        let start = Instant::now();
        let mut n = 0u64;
        while !status.stopped() && out.read_exact(&mut buf).is_ok() {
            if let Some(fps) = pace {
                let due = start + Duration::from_secs_f64(n as f64 / fps);
                let now = Instant::now();
                if due > now {
                    std::thread::sleep(due - now);
                }
                n += 1;
            }
            slot.set(Frame::new(w, h, buf.clone()));
            status.frame();
        }
        status.ended.store(true, Ordering::Relaxed);
    });
}

fn start_media(path: String, looping: bool, slot: Slot, status: Arc<Status>, track: Option<Arc<Track>>) {
    std::thread::spawn(move || {
        let (size, has_audio, alpha_codec, fps) = probe(&path);
        let rate = format!("{fps}");
        let decoder: Vec<&str> = match alpha_codec.as_str() {
            "vp9" => vec!["-c:v", "libvpx-vp9"],
            "vp8" => vec!["-c:v", "libvpx"],
            _ => vec![],
        };
        if size.is_none() && !has_audio {
            return status.fail(format!("Fichier illisible : {path}"));
        }
        let lp: Vec<&str> = if looping { vec!["-stream_loop", "-1"] } else { vec![] };
        // -re : lecture à la vitesse normale.
        if let Some((w, h)) = size {
            let child = ffmpeg()
                .args(["-loglevel", "error", "-re"])
                .args(&lp)
                .args(&decoder)
                .args(["-i", &path, "-an", "-fps_mode", "cfr", "-r", &rate, "-pix_fmt", "rgba", "-f", "rawvideo", "-"])
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn();
            match child {
                Ok(c) => pump_video(c, w, h, slot, status.clone(), Some(fps)),
                Err(e) => return status.fail(format!("FFmpeg introuvable : {e}")),
            }
        }
        if has_audio && let Some(track) = track {
            let child = ffmpeg()
                .args(["-loglevel", "error", "-re"])
                .args(&lp)
                .args(["-i", &path, "-vn", "-f", "f32le", "-ar", "48000", "-ac", "2", "-"])
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn();
            if let Ok(mut c) = child {
                let mut out = c.stdout.take().unwrap();
                status.adopt(c);
                let mut buf = vec![0u8; 480 * 8];
                while !status.stopped() && out.read_exact(&mut buf).is_ok() {
                    track.push(bytemuck::cast_slice(&buf));
                }
            }
        }
    });
}

/// Caméras disponibles (DirectShow sous Windows, V4L2 sous Linux).
pub fn cameras() -> Vec<String> {
    if cfg!(windows) {
        let out = ffmpeg().args(["-list_devices", "true", "-f", "dshow", "-i", "dummy"]).stdout(Stdio::null()).stderr(Stdio::piped()).output();
        let text = out.map(|o| String::from_utf8_lossy(&o.stderr).into_owned()).unwrap_or_default();
        text.lines()
            .filter(|l| l.contains("(video)"))
            .filter_map(|l| {
                let a = l.find('"')?;
                let b = l[a + 1..].find('"')?;
                Some(l[a + 1..a + 1 + b].to_string())
            })
            .collect()
    } else {
        let mut list: Vec<String> = std::fs::read_dir("/dev")
            .map(|d| d.filter_map(|e| e.ok()).map(|e| e.path().to_string_lossy().into_owned()).filter(|p| p.starts_with("/dev/video")).collect())
            .unwrap_or_default();
        list.sort();
        list
    }
}

fn start_camera(device: String, w: u32, h: u32, fps: u32, slot: Slot, status: Arc<Status>) {
    std::thread::spawn(move || {
        // Réglage « Par défaut » (0) : le mode de la caméra, mis en 1920 × 1080 (bandes noires si besoin).
        let default = w == 0;
        let (w, h) = if default { (1920, 1080) } else { (w.max(2), h.max(2)) };
        let size = format!("{w}x{h}");
        let rate = fps.max(1).to_string();
        // L'image est mise à la taille demandée en sortie : la taille des images lues est connue.
        let scale = format!("scale={w}:{h}:force_original_aspect_ratio=decrease,pad={w}:{h}:-1:-1:color=black@0");
        // Toutes les caméras n'offrent pas la taille et la cadence demandées : on essaie
        // ensuite la taille seule, puis le réglage par défaut de la caméra.
        let rate_only = ["-framerate", rate.as_str()];
        let all: [&[&str]; 3] = [&["-video_size", &size, "-framerate", &rate], &["-video_size", &size], &[]];
        let modes: Vec<&[&str]> = match (default, fps) {
            (true, 0) => vec![&[]],
            (true, _) => vec![&rate_only, &[]],
            (false, 0) => vec![all[1], all[2]],
            _ => all.to_vec(),
        };
        let mut last_error = String::new();
        for mode in modes {
            if status.stopped() {
                return;
            }
            let mut cmd = ffmpeg();
            cmd.args(["-loglevel", "error"]);
            if cfg!(windows) {
                cmd.args(["-f", "dshow", "-rtbufsize", "256M"]).args(mode).args(["-i", &format!("video={device}")]);
            } else {
                cmd.args(["-f", "v4l2"]).args(mode).args(["-i", &device]);
            }
            let child = cmd.args(["-vf", &scale, "-pix_fmt", "rgba", "-f", "rawvideo", "-"]).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn();
            let mut c = match child {
                Ok(c) => c,
                Err(e) => return status.fail(format!("FFmpeg introuvable : {e}")),
            };
            let mut out = c.stdout.take().unwrap();
            let mut err = c.stderr.take().unwrap();
            status.adopt(c);
            let mut buf = vec![0u8; (w * h * 4) as usize];
            let mut frames = 0u64;
            while !status.stopped() && out.read_exact(&mut buf).is_ok() {
                if frames == 0 {
                    *status.error.lock() = None;
                    log::info!("Caméra « {device} » ouverte ({})", if mode.is_empty() { "réglage par défaut".to_string() } else { mode.join(" ") });
                }
                frames += 1;
                slot.set(Frame::new(w, h, buf.clone()));
                status.frame();
            }
            if status.stopped() {
                return;
            }
            let mut text = String::new();
            let _ = err.read_to_string(&mut text);
            last_error = text.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("la caméra ne répond pas").to_string();
            if frames > 0 {
                break; // la caméra a fonctionné puis s'est arrêtée (débranchée, prise par un autre logiciel)
            }
            log::info!("Caméra « {device} » refusée avec {:?} : {last_error}", mode);
        }
        status.fail(format!("Caméra « {device} » : {last_error}. Fermez les autres logiciels qui l'utilisent, ou choisissez une autre résolution."));
    });
}

/// Écran disponible pour la capture.
pub struct MonitorInfo {
    pub index: usize,
    pub name: String,
}

#[cfg(windows)]
mod platform {
    use super::*;
    mod browser;
    pub use browser::{start_browser, web_page};
    use windows::Win32::Foundation::{CloseHandle, HWND, RECT};
    use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow};
    use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS};
    use windows::Win32::UI::WindowsAndMessaging::{GetClassNameW, GetForegroundWindow, GetWindowRect, GetWindowThreadProcessId, IsWindow, IsWindowVisible};
    use windows_capture::capture::{Context, GraphicsCaptureApiHandler};
    use windows_capture::frame::Frame as WcFrame;
    use windows_capture::graphics_capture_api::InternalCaptureControl;
    use windows_capture::monitor::Monitor;
    use windows_capture::settings::{
        ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings, GraphicsCaptureItemType, MinimumUpdateIntervalSettings,
        SecondaryWindowSettings, Settings,
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
            Ok(Self { slot: ctx.flags.0, status: ctx.flags.1, scratch: Vec::new() })
        }

        fn on_frame_arrived(&mut self, frame: &mut WcFrame, control: InternalCaptureControl) -> Result<(), Self::Error> {
            if self.status.stopped() {
                control.stop();
                return Ok(());
            }
            if self.status.paused.load(Ordering::Relaxed) {
                return Ok(());
            }
            let buffer = frame.buffer()?;
            let (width, height) = (buffer.width(), buffer.height());
            let mut data = buffer.as_nopadding_buffer(&mut self.scratch).to_vec();
            if self.status.premultiplied.load(Ordering::Relaxed) {
                for p in data.chunks_exact_mut(4) {
                    let a = p[3] as u32;
                    if a > 0 && a < 255 {
                        for c in &mut p[..3] {
                            *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
                        }
                    }
                }
            }
            self.slot.set(Frame::new(width, height, data));
            self.status.frame();
            Ok(())
        }

        fn on_closed(&mut self) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    fn settings<T: TryInto<GraphicsCaptureItemType>>(item: T, cursor: bool, fps: u32, slot: Slot, status: Arc<Status>) -> Settings<Flags, T> {
        Settings::new(
            item,
            if cursor { CursorCaptureSettings::WithCursor } else { CursorCaptureSettings::WithoutCursor },
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

    /// Capture d'une fenêtre jusqu'à ce qu'elle disparaisse ou que `alive` devienne faux.
    fn capture_window(window: Window, fps: u32, slot: &Slot, status: &Arc<Status>, premultiplied: bool, alive: impl Fn() -> bool) -> Result<(), String> {
        let sub = Arc::new(Status::default());
        sub.premultiplied.store(premultiplied, Ordering::Relaxed);
        let hwnd = HWND(window.as_raw_hwnd());
        let control = Handler::start_free_threaded(settings(window, false, fps, slot.clone(), sub.clone())).map_err(|e| e.to_string())?;
        while !status.stopped() && alive() && unsafe { IsWindow(Some(hwnd)) }.as_bool() && !control.is_finished() {
            *status.fps.lock() = *sub.fps.lock();
            std::thread::sleep(Duration::from_millis(250));
        }
        sub.stop.store(true, Ordering::Relaxed);
        let _ = control.stop();
        Ok(())
    }

    pub fn start_window(title: String, fps: u32, slot: Slot, status: Arc<Status>, track: Option<Arc<Track>>) {
        std::thread::spawn(move || {
            // Comme OBS : la fenêtre (jeu pas encore lancé) est recherchée toutes les 2 s.
            while !status.stopped() {
                match Window::from_contains_name(&title) {
                    Ok(window) => {
                        *status.error.lock() = None;
                        let audio = track.as_ref().and_then(|t| window.process_id().ok().map(|pid| process_audio(pid, status.clone(), t.clone())));
                        if let Err(e) = capture_window(window, fps, &slot, &status, false, || true) {
                            status.fail(format!("Capture de « {title} » impossible : {e}"));
                        }
                        if let Some(a) = audio {
                            a.store(true, Ordering::Relaxed);
                        }
                    }
                    Err(_) => *status.error.lock() = Some(format!("Fenêtre « {title} » introuvable : nouvelle recherche toutes les 2 s")),
                }
                std::thread::sleep(Duration::from_secs(2));
            }
        });
    }

    /// Fenêtre au premier plan qui occupe tout son écran (plein écran ou fenêtré sans bordure).
    fn fullscreen_foreground() -> Option<(HWND, u32)> {
        unsafe {
            let hwnd = GetForegroundWindow();
            if hwnd.0.is_null() || !IsWindowVisible(hwnd).as_bool() {
                return None;
            }
            let mut class = [0u16; 64];
            let n = GetClassNameW(hwnd, &mut class) as usize;
            let class = String::from_utf16_lossy(&class[..n]);
            // Bureau et barre des tâches : jamais un jeu.
            if ["Progman", "WorkerW", "Shell_TrayWnd", "Windows.UI.Core.CoreWindow"].contains(&class.as_str()) {
                return None;
            }
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            if pid == std::process::id() {
                return None;
            }
            let mut r = RECT::default();
            GetWindowRect(hwnd, &mut r).ok()?;
            let mon = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
            let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
            if !GetMonitorInfoW(mon, &mut mi).as_bool() {
                return None;
            }
            let m = mi.rcMonitor;
            (r.left <= m.left && r.top <= m.top && r.right >= m.right && r.bottom >= m.bottom).then_some((hwnd, pid))
        }
    }

    /// Capture de jeu automatique : le jeu reste capturé pendant un alt-tab, jusqu'à
    /// ce qu'il se ferme ou qu'une autre application passe en plein écran.
    /// Capture de l'écran où tourne le jeu, tant que la fenêtre du jeu existe. Plus fluide
    /// que la capture de la fenêtre ; pendant un Alt+Tab, la dernière image du jeu reste.
    fn capture_game_screen(hwnd: HWND, fps: u32, slot: &Slot, status: &Arc<Status>) -> Result<(), String> {
        let mon = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
        let monitor = Monitor::from_raw_hmonitor(mon.0);
        let sub = Arc::new(Status::default());
        let control = Handler::start_free_threaded(settings(monitor, false, fps, slot.clone(), sub.clone())).map_err(|e| e.to_string())?;
        while !status.stopped() && unsafe { IsWindow(Some(hwnd)) }.as_bool() && !control.is_finished() {
            let front = fullscreen_foreground();
            // Une autre application passe en plein écran : on la capture à la place.
            if matches!(front, Some((h, _)) if h != hwnd) {
                break;
            }
            sub.paused.store(unsafe { GetForegroundWindow() } != hwnd, Ordering::Relaxed);
            *status.fps.lock() = *sub.fps.lock();
            std::thread::sleep(Duration::from_millis(100));
        }
        sub.stop.store(true, Ordering::Relaxed);
        let _ = control.stop();
        Ok(())
    }

    pub fn start_game(fps: u32, screen: bool, slot: Slot, status: Arc<Status>, track: Option<Arc<Track>>) {
        std::thread::spawn(move || {
            *status.info.lock() = Some("En attente d'un jeu en plein écran".into());
            while !status.stopped() {
                let Some((hwnd, pid)) = fullscreen_foreground() else {
                    std::thread::sleep(Duration::from_secs(1));
                    continue;
                };
                let window = Window::from_raw_hwnd(hwnd.0);
                let title = window.title().unwrap_or_default();
                let exe = window.process_name().unwrap_or_default();
                *status.info.lock() = Some(if title.is_empty() { exe.clone() } else { title });
                *status.error.lock() = None;
                log::info!("Capture de jeu : {exe}");
                let audio = track.as_ref().map(|t| process_audio(pid, status.clone(), t.clone()));
                // Une autre application passe en plein écran : on la capture à la place.
                let other = move || !matches!(fullscreen_foreground(), Some((h, _)) if h != hwnd);
                let r = if screen { capture_game_screen(hwnd, fps, &slot, &status) } else { capture_window(window, fps, &slot, &status, false, other) };
                if let Err(e) = r {
                    status.fail(format!("Capture du jeu impossible : {e}"));
                    std::thread::sleep(Duration::from_secs(2));
                }
                if let Some(a) = audio {
                    a.store(true, Ordering::Relaxed);
                }
                slot.clear();
                *status.info.lock() = Some("En attente d'un jeu en plein écran".into());
            }
        });
    }

    /// Son du PC (hors MyStream) dans la piste d'une source « Capture d'écran ».
    pub fn pc_audio(status: Arc<Status>, track: Arc<Track>) {
        let stop = process_audio_mode(std::process::id(), false, status.clone(), track);
        std::thread::spawn(move || {
            while !status.stopped() {
                std::thread::sleep(Duration::from_millis(200));
            }
            stop.store(true, Ordering::Relaxed);
        });
    }

    /// Son de tout le PC sauf MyStream (et ses processus) : pas d'écho de l'écoute au casque.
    pub fn system_audio(track: Arc<Track>) -> bool {
        let status = Arc::new(Status::default());
        // Le drapeau d'arrêt n'est jamais levé : la capture dure tant que MyStream est ouvert.
        std::mem::forget(process_audio_mode(std::process::id(), false, status, track));
        true
    }

    /// Son d'un processus et de ses enfants (WASAPI). Renvoie le drapeau d'arrêt.
    fn process_audio(pid: u32, status: Arc<Status>, track: Arc<Track>) -> Arc<AtomicBool> {
        process_audio_mode(pid, true, status, track)
    }

    /// `include` faux : tout sauf ce processus et ses enfants.
    fn process_audio_mode(pid: u32, include: bool, status: Arc<Status>, track: Arc<Track>) -> Arc<AtomicBool> {
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        std::thread::spawn(move || {
            let run = || -> Result<(), String> {
                let _ = wasapi::initialize_mta();
                let format = wasapi::WaveFormat::new(32, 32, wasapi::SampleType::Float, 48000, 2, None);
                let mut client = wasapi::AudioClient::new_application_loopback_client(pid, include).map_err(|e| e.to_string())?;
                let mode = wasapi::StreamMode::EventsShared { autoconvert: true, buffer_duration_hns: 0 };
                client.initialize_client(&format, wasapi::Direction::Capture, mode, None).map_err(|e| e.to_string())?;
                let event = client.set_get_eventhandle().map_err(|e| e.to_string())?;
                let capture = client.get_audiocaptureclient().map_err(|e| e.to_string())?;
                client.start_stream().map_err(|e| e.to_string())?;
                let mut queue = std::collections::VecDeque::<u8>::new();
                while !stop2.load(Ordering::Relaxed) && !status.stopped() {
                    if capture.get_next_packet_size().map_err(|e| e.to_string())?.unwrap_or(0) > 0 {
                        capture.read_from_device_to_deque(&mut queue).map_err(|e| e.to_string())?;
                        let n = queue.len() / 4 * 4;
                        let bytes: Vec<u8> = queue.drain(..n).collect();
                        let samples: Vec<f32> = bytes.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
                        track.push(&samples);
                    }
                    let _ = event.wait_for_event(200);
                }
                let _ = client.stop_stream();
                Ok(())
            };
            if let Err(e) = run() {
                track.fail(format!("Son du processus {pid} : {e}"));
            }
        });
        stop
    }

    fn processes() -> Vec<(u32, u32, String)> {
        unsafe {
            let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
                return vec![];
            };
            let mut e = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
            let mut all = Vec::new();
            if Process32FirstW(snap, &mut e).is_ok() {
                loop {
                    let len = e.szExeFile.iter().position(|c| *c == 0).unwrap_or(e.szExeFile.len());
                    all.push((e.th32ProcessID, e.th32ParentProcessID, String::from_utf16_lossy(&e.szExeFile[..len])));
                    if Process32NextW(snap, &mut e).is_err() {
                        break;
                    }
                }
            }
            let _ = CloseHandle(snap);
            all
        }
    }

    /// Processus principal d'un programme (celui dont le parent ne porte pas le même nom).
    fn find_process(exe: &str) -> Option<u32> {
        let all = processes();
        let same: Vec<_> = all.iter().filter(|p| p.2.eq_ignore_ascii_case(exe)).collect();
        same.iter().find(|p| !same.iter().any(|q| q.0 == p.1)).or(same.first()).map(|p| p.0)
    }

    pub fn start_app_audio(exe: String, status: Arc<Status>, track: Arc<Track>) {
        std::thread::spawn(move || {
            // Le programme peut ne pas être lancé, ou redémarrer : on le suit.
            while !status.stopped() {
                match find_process(&exe) {
                    Some(pid) => {
                        *status.info.lock() = Some(format!("{exe} (processus {pid})"));
                        *status.error.lock() = None;
                        let stop = process_audio(pid, status.clone(), track.clone());
                        while !status.stopped() && find_process(&exe) == Some(pid) {
                            std::thread::sleep(Duration::from_secs(2));
                        }
                        stop.store(true, Ordering::Relaxed);
                    }
                    None => {
                        *status.error.lock() = Some(format!("{exe} n'est pas lancé : en attente"));
                        std::thread::sleep(Duration::from_secs(2));
                    }
                }
            }
        });
    }

    /// Programmes lancés, pour « Son d'une application ».
    pub fn programs() -> Vec<String> {
        let skip =
            ["svchost.exe", "system", "conhost.exe", "csrss.exe", "dllhost.exe", "runtimebroker.exe", "smss.exe", "wininit.exe", "services.exe", "lsass.exe"];
        let mut list: Vec<String> =
            processes().into_iter().map(|p| p.2).filter(|n| n.to_lowercase().ends_with(".exe") && !skip.contains(&n.to_lowercase().as_str())).collect();
        list.sort_by_key(|n| n.to_lowercase());
        list.dedup_by_key(|n| n.to_lowercase());
        list
    }

    pub fn monitors() -> Vec<MonitorInfo> {
        Monitor::enumerate()
            .unwrap_or_default()
            .iter()
            .enumerate()
            .map(|(i, m)| MonitorInfo { index: i, name: format!("Écran {} ({}×{})", i + 1, m.width().unwrap_or(0), m.height().unwrap_or(0)) })
            .collect()
    }

    pub fn windows() -> Vec<String> {
        let mut list: Vec<String> = Window::enumerate().unwrap_or_default().iter().filter_map(|w| w.title().ok()).filter(|t| !t.trim().is_empty()).collect();
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
            while !status.stopped() {
                let t = Instant::now();
                match monitor.capture_image() {
                    Ok(img) => {
                        slot.set(Frame::new(img.width(), img.height(), img.into_raw()));
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

    pub fn start_window(title: String, _fps: u32, _slot: Slot, status: Arc<Status>, _track: Option<Arc<Track>>) {
        status.fail(format!("Capture de fenêtre (« {title} ») disponible sous Windows uniquement"));
    }

    pub fn pc_audio(_status: Arc<Status>, track: Arc<Track>) {
        track.fail("Le son du PC n'est capturé que sous Windows.");
    }

    pub fn start_game(_fps: u32, _screen: bool, _slot: Slot, status: Arc<Status>, _track: Option<Arc<Track>>) {
        status.fail("Capture de jeu disponible sous Windows uniquement".into());
    }

    pub fn start_browser(_url: String, _w: u32, _h: u32, _css: String, _fps: u32, _slot: Slot, status: Arc<Status>, _track: Option<Arc<Track>>) {
        status.fail("Les sources navigateur nécessitent Windows (WebView2).".into());
    }

    pub fn web_page(_url: &str, _script: &str, _folder: &str, _on_message: Box<dyn Fn(String) + Send>, _stop: Arc<AtomicBool>) -> Result<(), String> {
        Err("Le chat Kick en direct nécessite Windows (WebView2) ; utilisez Restream.".into())
    }

    pub fn start_app_audio(exe: String, status: Arc<Status>, _track: Arc<Track>) {
        status.fail(format!("Son d'une application ({exe}) disponible sous Windows uniquement"));
    }

    pub fn system_audio(_track: Arc<Track>) -> bool {
        false
    }

    pub fn programs() -> Vec<String> {
        Vec::new()
    }

    pub fn monitors() -> Vec<MonitorInfo> {
        xcap::Monitor::all()
            .unwrap_or_default()
            .iter()
            .enumerate()
            .map(|(i, m)| MonitorInfo { index: i, name: format!("Écran {} ({}×{})", i + 1, m.width().unwrap_or(0), m.height().unwrap_or(0)) })
            .collect()
    }

    pub fn windows() -> Vec<String> {
        Vec::new()
    }
}

pub use platform::{monitors, programs, system_audio, web_page, windows};
