//! Mises à jour automatiques : au démarrage puis toutes les 30 minutes,
//! MyStream cherche sur GitHub une préversion native plus récente. Elle est
//! téléchargée et installée en arrière-plan (jamais pendant un live), et
//! s'applique au prochain lancement ; un clic permet de redémarrer tout de suite.

use parking_lot::Mutex;
use serde_json::Value;
use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;

const RELEASES: &str = "https://api.github.com/repos/TheDEMON78/mystream/releases?per_page=30";
const TAG: &str = "natif-v0.1.";

/// Numéro de compilation de cet exécutable (fourni par la CI), 0 en développement.
pub fn build() -> u64 {
    option_env!("MYSTREAM_BUILD").and_then(|v| v.parse().ok()).unwrap_or(0)
}

#[derive(Clone, Debug)]
pub struct Available {
    pub number: u64,
    pub url: String,
}

#[derive(Clone, Debug, Default)]
pub enum State {
    #[default]
    Idle,
    /// Vérification en cours.
    Checking,
    /// Aucune version plus récente.
    UpToDate,
    Available(Available),
    Downloading,
    /// Installée : utilisée au prochain lancement.
    Ready(u64),
    Failed(String),
}

pub type Shared = Arc<Mutex<State>>;

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().timeout_global(Some(std::time::Duration::from_secs(600))).build().into()
}

/// Cherche une préversion plus récente (en arrière-plan).
pub fn check(state: Shared) {
    // Restes de la mise à jour précédente.
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::fs::remove_file(exe.with_extension("old.exe"));
    }
    if build() == 0 {
        *state.lock() = State::Failed("version de développement : pas de mise à jour automatique".into());
        return;
    }
    {
        let mut st = state.lock();
        if matches!(*st, State::Downloading | State::Ready(_) | State::Checking) {
            return;
        }
        *st = State::Checking;
    }
    std::thread::spawn(move || {
        let result = (|| -> Result<Option<Available>, String> {
            let text = agent()
                .get(RELEASES)
                .header("User-Agent", "MyStream")
                .header("Accept", "application/vnd.github+json")
                .call()
                .map_err(|e| e.to_string())?
                .body_mut()
                .read_to_string()
                .map_err(|e| e.to_string())?;
            let list: Vec<Value> = serde_json::from_str(&text).map_err(|e| e.to_string())?;
            let best = list
                .iter()
                .filter_map(|r| {
                    let n: u64 = r["tag_name"].as_str()?.strip_prefix(TAG)?.parse().ok()?;
                    let url = r["assets"].as_array()?.iter().find(|a| a["name"].as_str().is_some_and(|n| n.ends_with("-win64.zip")))?["browser_download_url"]
                        .as_str()?
                        .to_string();
                    Some(Available { number: n, url })
                })
                .max_by_key(|a| a.number);
            Ok(best.filter(|a| a.number > build()))
        })();
        *state.lock() = match result {
            Ok(Some(a)) => {
                log::info!("Mise à jour disponible : natif 0.1.{}", a.number);
                State::Available(a)
            }
            Ok(None) => State::UpToDate,
            Err(e) => {
                log::info!("Vérification des mises à jour impossible : {e}");
                State::Failed(format!("vérification impossible ({e})"))
            }
        };
    });
}

/// Télécharge et installe la mise à jour ; l'état passe à Ready quand il ne reste qu'à redémarrer.
pub fn install(state: Shared, a: Available) {
    let number = a.number;
    *state.lock() = State::Downloading;
    std::thread::spawn(move || {
        let result = (|| -> Result<(), String> {
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            let dir = exe.parent().ok_or("dossier introuvable")?.to_path_buf();
            let zip_path = std::env::temp_dir().join("MyStream-mise-a-jour.zip");
            {
                let resp = agent().get(&a.url).header("User-Agent", "MyStream").call().map_err(|e| e.to_string())?;
                let mut reader = resp.into_body().into_reader();
                let mut file = std::fs::File::create(&zip_path).map_err(|e| e.to_string())?;
                std::io::copy(&mut reader, &mut file).map_err(|e| e.to_string())?;
            }
            let mut archive = zip::ZipArchive::new(std::fs::File::open(&zip_path).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
            let mut extract = |name: &str| -> Result<Option<PathBuf>, String> {
                let Ok(mut f) = archive.by_name(name) else { return Ok(None) };
                let mut data = Vec::new();
                f.read_to_end(&mut data).map_err(|e| e.to_string())?;
                let tmp = dir.join(format!("{name}.nouveau"));
                std::fs::write(&tmp, data).map_err(|e| format!("écriture dans {} impossible : {e}", dir.display()))?;
                Ok(Some(tmp))
            };
            let new_exe = extract("MyStream.exe")?.ok_or("MyStream.exe absent de l'archive")?;
            let new_ffmpeg = extract("ffmpeg.exe")?;
            // Windows autorise à renommer l'exécutable en cours : l'ancien part de côté.
            std::fs::rename(&exe, exe.with_extension("old.exe")).map_err(|e| e.to_string())?;
            std::fs::rename(&new_exe, &exe).map_err(|e| e.to_string())?;
            if let Some(ff) = new_ffmpeg {
                let target = dir.join("ffmpeg.exe");
                let _ = std::fs::remove_file(target.with_extension("old.exe"));
                let _ = std::fs::rename(&target, target.with_extension("old.exe"));
                let _ = std::fs::rename(&ff, &target);
            }
            let _ = std::fs::remove_file(&zip_path);
            Ok(())
        })();
        *state.lock() = match result {
            Ok(()) => {
                log::info!("Mise à jour natif 0.1.{number} installée");
                State::Ready(number)
            }
            Err(e) => State::Failed(format!("installation impossible ({e})")),
        };
    });
}

/// Relance MyStream (après l'installation).
pub fn restart() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe).spawn();
    }
}
