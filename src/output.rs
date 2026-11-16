//! Sorties : streaming (RTMP) et enregistrement, encodés par FFmpeg.
//!
//! La vidéo arrive déjà en NV12 (convertie par la carte graphique) : FFmpeg
//! la passe directement à l'encodeur matériel, sans conversion. Le son du
//! mixeur arrive par une connexion locale.

use crate::model::Settings;
use anyhow::{Context, Result, anyhow};
use crossbeam_channel::{Receiver, Sender, bounded};
use parking_lot::Mutex;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Stream,
    Record,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Stream => "Le streaming",
            Kind::Record => "L'enregistrement",
        }
    }
}

/// Mesures envoyées par FFmpeg (-progress).
#[derive(Default, Clone)]
pub struct Stats {
    pub fps: f32,
    pub bitrate_kbps: f32,
    pub speed: f32,
    pub drops: u64,
    pub error: Option<String>,
    pub ended: bool,
}

pub struct Output {
    pub kind: Kind,
    pub encoder: String,
    pub file: Option<PathBuf>,
    pub started: Instant,
    pub stats: Arc<Mutex<Stats>>,
    video_tx: Option<Sender<Arc<Vec<u8>>>>,
    child: Arc<Mutex<Child>>,
    audio_stop: Arc<std::sync::atomic::AtomicBool>,
}

/// FFmpeg : à côté de l'application, sinon celui du système.
pub fn ffmpeg_path() -> PathBuf {
    if let Ok(p) = std::env::var("MYSTREAM_FFMPEG") {
        return p.into();
    }
    let name = if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" };
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let local = dir.join(name);
        if local.exists() {
            return local;
        }
    }
    PathBuf::from(name)
}

fn command() -> Command {
    #[allow(unused_mut)]
    let mut c = Command::new(ffmpeg_path());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW : pas de console qui s'ouvre
    }
    c
}

/// Encodeurs matériels qui fonctionnent réellement sur ce PC (testés une fois), puis x264.
pub fn encoders() -> &'static [String] {
    static LIST: OnceLock<Vec<String>> = OnceLock::new();
    LIST.get_or_init(|| {
        let mut found: Vec<String> = ["h264_nvenc", "h264_qsv", "h264_amf", "h264_videotoolbox"]
            .iter()
            .filter(|enc| {
                command()
                    .args([
                        "-hide_banner",
                        "-loglevel",
                        "error",
                        "-f",
                        "lavfi",
                        "-i",
                        "color=black:s=256x144:r=30",
                        "-frames:v",
                        "5",
                        "-pix_fmt",
                        "nv12",
                        "-c:v",
                        enc,
                        "-f",
                        "null",
                        "-",
                    ])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .map(|s| s.success())
                    .unwrap_or(false)
            })
            .map(|s| s.to_string())
            .collect();
        found.push("libx264".into());
        found
    })
}

pub fn encoder_label(enc: &str) -> &'static str {
    match enc {
        "auto" => "Automatique (le meilleur disponible)",
        "h264_nvenc" => "NVIDIA NVENC",
        "h264_qsv" => "Intel Quick Sync",
        "h264_amf" => "AMD AMF",
        "h264_videotoolbox" => "Apple VideoToolbox",
        _ => "Processeur (x264)",
    }
}

/// Arguments de l'encodeur vidéo, débit constant (réglages proches d'OBS « Qualité »).
fn video_args(enc: &str, kbps: u32, fps: u32) -> Vec<String> {
    let rate = format!("{kbps}k");
    let buf = format!("{}k", kbps * 2);
    let gop = (fps * 2).to_string();
    let mut a: Vec<&str> = match enc {
        "h264_nvenc" => vec!["-c:v", enc, "-preset", "p5", "-tune", "hq", "-rc", "cbr", "-bf", "2"],
        "h264_qsv" => vec!["-c:v", enc, "-preset", "medium"],
        "h264_amf" => vec!["-c:v", enc, "-usage", "transcoding", "-quality", "quality", "-rc", "cbr"],
        "h264_videotoolbox" => vec!["-c:v", enc, "-realtime", "1"],
        _ => vec!["-c:v", "libx264", "-preset", "veryfast", "-tune", "zerolatency"],
    };
    a.extend(["-b:v", &rate, "-maxrate", &rate, "-bufsize", &buf, "-g", &gop]);
    a.into_iter().map(String::from).collect()
}

impl Output {
    pub fn start(kind: Kind, s: &Settings, width: u32, height: u32, audio: Receiver<Arc<Vec<f32>>>) -> Result<Self> {
        let list = encoders();
        let wanted = &s.stream.encoder;
        let encoder = if wanted != "auto" && list.contains(wanted) { wanted.clone() } else { list[0].clone() };
        let fps = s.fps;

        // Son : FFmpeg se connecte à ce port local et reçoit le mixage en flottants 32 bits.
        let listener = TcpListener::bind("127.0.0.1:0").context("port local pour le son")?;
        let port = listener.local_addr()?.port();

        let mut args: Vec<String> = vec!["-hide_banner", "-loglevel", "error", "-nostats", "-progress", "pipe:1"].into_iter().map(String::from).collect();
        let size = format!("{width}x{height}");
        let rate = fps.to_string();
        args.extend(
            ["-f", "rawvideo", "-pix_fmt", "nv12", "-video_size", &size, "-framerate", &rate, "-thread_queue_size", "64", "-i", "pipe:0"]
                .into_iter()
                .map(String::from),
        );
        let audio_url = format!("tcp://127.0.0.1:{port}");
        args.extend(["-f", "f32le", "-ar", "48000", "-ac", "2", "-thread_queue_size", "1024", "-i", &audio_url].into_iter().map(String::from));
        args.extend(["-map", "0:v", "-map", "1:a"].into_iter().map(String::from));
        let (target, file) = match kind {
            Kind::Stream => {
                if s.stream.server.trim().is_empty() || s.stream.key.trim().is_empty() {
                    return Err(anyhow!("Serveur ou clé de stream manquant (Paramètres > Stream)."));
                }
                args.extend(video_args(&encoder, s.stream.video_bitrate, fps));
                (format!("{}/{}", s.stream.server.trim_end_matches('/'), s.stream.key.trim()), None)
            }
            Kind::Record => {
                args.extend(video_args(&encoder, s.record.video_bitrate, fps));
                let name = format!("{}.{}", chrono_like_now(), if s.record.format == "mp4" { "mp4" } else { "mkv" });
                let path = PathBuf::from(&s.record.folder).join(name);
                (path.to_string_lossy().into_owned(), Some(path))
            }
        };
        let ab = format!("{}k", s.stream.audio_bitrate.max(64));
        args.extend(
            [
                "-pix_fmt",
                "nv12",
                "-colorspace",
                "bt709",
                "-color_primaries",
                "bt709",
                "-color_trc",
                "bt709",
                "-color_range",
                "tv",
                "-c:a",
                "aac",
                "-b:a",
                &ab,
                "-ar",
                "48000",
            ]
            .into_iter()
            .map(String::from),
        );
        match kind {
            Kind::Stream => args.extend(["-f", "flv"].into_iter().map(String::from)),
            Kind::Record if file.as_ref().is_some_and(|f| f.extension().is_some_and(|e| e == "mp4")) => {
                args.extend(["-movflags", "+faststart"].into_iter().map(String::from))
            }
            Kind::Record => args.extend(["-f", "matroska"].into_iter().map(String::from)),
        }
        args.push(target);

        let mut child = command().args(&args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().context("FFmpeg introuvable")?;
        let stats = Arc::new(Mutex::new(Stats::default()));

        // Vidéo : tampon de 3 images ; si FFmpeg prend du retard, l'image est comptée perdue.
        let (video_tx, video_rx) = bounded::<Arc<Vec<u8>>>(3);
        let mut stdin = child.stdin.take().unwrap();
        std::thread::spawn(move || {
            for frame in video_rx {
                if stdin.write_all(&frame).is_err() {
                    break;
                }
            }
        });

        // Son.
        let audio_stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stop2 = audio_stop.clone();
        std::thread::spawn(move || {
            let _ = listener.set_nonblocking(false);
            let Ok((mut sock, _)) = listener.accept() else {
                return;
            };
            let _ = sock.set_nodelay(true);
            for block in audio {
                if stop2.load(std::sync::atomic::Ordering::Relaxed) {
                    break;
                }
                if sock.write_all(bytemuck::cast_slice(&block)).is_err() {
                    break;
                }
            }
        });

        // Progression (stdout) et erreurs (stderr).
        let out = child.stdout.take().unwrap();
        let st = stats.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                let Some((k, v)) = line.split_once('=') else {
                    continue;
                };
                let mut s = st.lock();
                match k {
                    "fps" => s.fps = v.trim().parse().unwrap_or(0.0),
                    "bitrate" => s.bitrate_kbps = v.trim().trim_end_matches("kbits/s").parse().unwrap_or(0.0),
                    "speed" => s.speed = v.trim().trim_end_matches('x').parse().unwrap_or(0.0),
                    "drop_frames" => s.drops = v.trim().parse().unwrap_or(0),
                    _ => {}
                }
            }
        });
        let mut err = child.stderr.take().unwrap();
        let st = stats.clone();
        let child = Arc::new(Mutex::new(child));
        let child2 = child.clone();
        std::thread::spawn(move || {
            let mut text = String::new();
            let _ = err.read_to_string(&mut text);
            let status = child2.lock().wait().ok();
            let mut s = st.lock();
            s.ended = true;
            if !status.map(|s| s.success()).unwrap_or(false) {
                let last = text.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("FFmpeg s'est arrêté").to_string();
                s.error = Some(last);
            }
        });

        let mut line = args.join(" ");
        if !s.stream.key.trim().is_empty() {
            line = line.replace(s.stream.key.trim(), "***"); // jamais la clé de stream dans le journal
        }
        log::info!("{} démarré ({encoder}) : {line}", kind.label());
        Ok(Self { kind, encoder, file, started: Instant::now(), stats, video_tx: Some(video_tx), child, audio_stop })
    }

    /// Envoie une image NV12 ; renvoie false si FFmpeg est en retard (image perdue).
    pub fn push(&self, frame: Arc<Vec<u8>>) -> bool {
        self.video_tx.as_ref().map(|tx| tx.try_send(frame).is_ok()).unwrap_or(false)
    }

    pub fn ended(&self) -> bool {
        self.stats.lock().ended
    }

    /// Arrêt propre : FFmpeg termine le fichier (en-têtes, index) avant de quitter.
    pub fn stop(mut self) {
        self.video_tx.take();
        self.audio_stop.store(true, std::sync::atomic::Ordering::Relaxed);
        let child = self.child.clone();
        std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(8);
            loop {
                if let Ok(Some(_)) = child.lock().try_wait() {
                    return;
                }
                if Instant::now() > deadline {
                    let _ = child.lock().kill();
                    return;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        });
    }
}

/// Horodatage local pour le nom des enregistrements (AAAA-MM-JJ HH-MM-SS).
fn chrono_like_now() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let offset = local_offset_secs();
    let t = secs + offset;
    let days = t.div_euclid(86400);
    let rem = t.rem_euclid(86400);
    // Conversion jours -> date civile (algorithme de Howard Hinnant).
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02} {:02}-{:02}-{:02}", rem / 3600, rem % 3600 / 60, rem % 60)
}

#[cfg(windows)]
fn local_offset_secs() -> i64 {
    #[repr(C)]
    struct Tzi {
        bias: i32,
        _rest: [u8; 168],
    }
    unsafe extern "system" {
        fn GetTimeZoneInformation(tzi: *mut Tzi) -> u32;
    }
    let mut tzi = Tzi { bias: 0, _rest: [0; 168] };
    let r = unsafe { GetTimeZoneInformation(&mut tzi) };
    // Biais en minutes (UTC = local + biais) ; l'heure d'été ajoute -60 min quand elle est active.
    let mut bias = tzi.bias as i64;
    if r == 2 {
        bias -= 60;
    }
    -bias * 60
}

#[cfg(not(windows))]
fn local_offset_secs() -> i64 {
    0
}
