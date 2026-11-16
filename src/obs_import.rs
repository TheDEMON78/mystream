//! Import d'une collection de scènes OBS (fichier JSON de
//! %APPDATA%\obs-studio\basic\scenes) : scènes, sources, positions,
//! tailles, recadrages, visibilité et volumes.

use crate::model::{Filters, Item, Project, Scene, Source, SourceKind};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub struct Report {
    pub scenes: usize,
    pub sources: usize,
    /// Sources non prises en charge (type OBS -> nombre).
    pub skipped: Vec<(String, usize)>,
}

/// Dossier des collections OBS.
pub fn obs_folder() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("obs-studio").join("basic").join("scenes")).filter(|p| p.exists())
}

fn color(abgr: u64) -> [u8; 4] {
    // OBS stocke les couleurs en 0xAABBGGRR.
    [(abgr & 0xff) as u8, (abgr >> 8 & 0xff) as u8, (abgr >> 16 & 0xff) as u8, (abgr >> 24 & 0xff) as u8]
}

/// Correspondance d'une source OBS ; None si le type n'est pas pris en charge.
fn kind_of(src: &Value, project: &Project) -> Option<SourceKind> {
    let s = &src["settings"];
    let id = src["id"].as_str().unwrap_or("");
    let (w, h) = (project.settings.width, project.settings.height);
    Some(match id {
        "monitor_capture" => SourceKind::Display { monitor: s["monitor"].as_u64().unwrap_or(0) as usize },
        // « Titre:Classe:programme.exe »
        "window_capture" => SourceKind::Window {
            title: s["window"].as_str().unwrap_or("").split(':').next().unwrap_or("").replace("#3A", ":"),
            audio: s["capture_audio"].as_bool().unwrap_or(false),
        },
        "game_capture" => SourceKind::Game { audio: s["capture_audio"].as_bool().unwrap_or(true) },
        "image_source" => SourceKind::Image { path: s["file"].as_str()?.to_string() },
        "color_source" | "color_source_v2" | "color_source_v3" => SourceKind::Color {
            rgba: color(s["color"].as_u64().unwrap_or(0xffff_ffff)),
            width: s["width"].as_u64().map(|v| v as u32).unwrap_or(w),
            height: s["height"].as_u64().map(|v| v as u32).unwrap_or(h),
        },
        "text_gdiplus" | "text_gdiplus_v2" | "text_gdiplus_v3" | "text_ft2_source" | "text_ft2_source_v2" => {
            let mut c = color(s["color"].as_u64().or(s["color1"].as_u64()).unwrap_or(0xffff_ffff));
            if id.starts_with("text_gdiplus") && s["opacity"].is_u64() {
                c[3] = (s["opacity"].as_u64().unwrap_or(100) * 255 / 100) as u8;
            }
            SourceKind::Text {
                text: s["text"].as_str().unwrap_or("").to_string(),
                size: s["font"]["size"].as_f64().unwrap_or(36.0) as f32,
                rgba: c,
                background: [0, 0, 0, 0],
                mono: false,
            }
        }
        "ffmpeg_source" => SourceKind::Media { path: s["local_file"].as_str()?.to_string(), looping: s["looping"].as_bool().unwrap_or(false) },
        "vlc_source" => SourceKind::Media { path: s["playlist"][0]["value"].as_str()?.to_string(), looping: s["loop"].as_bool().unwrap_or(true) },
        "dshow_input" | "av_capture_input" | "v4l2_input" => SourceKind::Camera {
            // « Nom de la caméra:\\?\usb#… »
            device: s["video_device_id"].as_str().unwrap_or("").split(':').next().unwrap_or("").replace("#3A", ":"),
            width: 1920,
            height: 1080,
            fps: 30,
        },
        "wasapi_process_output_capture" => SourceKind::AppAudio { exe: s["window"].as_str().unwrap_or("").rsplit(':').next().unwrap_or("").to_string() },
        _ => return None,
    })
}

pub fn import(path: &Path, project: &mut Project) -> Result<Report, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let json: Value = serde_json::from_str(&text).map_err(|e| format!("fichier OBS illisible : {e}"))?;
    let sources = json["sources"].as_array().ok_or("aucune source dans ce fichier")?;
    let mut by_name: HashMap<String, u64> = HashMap::new();
    let mut skipped: HashMap<String, usize> = HashMap::new();
    let mut count = 0;

    // Sources (hors scènes et groupes).
    for src in sources {
        let id = src["id"].as_str().unwrap_or("");
        if id == "scene"
            || id == "group"
            || id.starts_with("wasapi_input")
            || id.starts_with("wasapi_output")
            || id.starts_with("pulse_")
            || id.starts_with("coreaudio")
        {
            continue; // micro et son du bureau : déjà dans le mixeur
        }
        let name = src["name"].as_str().unwrap_or("Source").to_string();
        match kind_of(src, project) {
            Some(kind) => {
                let sid = project.new_id();
                let unique = project.unique_name(&name, project.sources.iter().map(|s| s.name.clone()));
                let volume = src["volume"].as_f64().unwrap_or(1.0) as f32;
                project.sources.push(Source {
                    id: sid,
                    name: unique,
                    kind,
                    filters: Filters::default(),
                    volume,
                    muted: src["muted"].as_bool().unwrap_or(false),
                    audio_filters: Default::default(),
                    monitor: false,
                });
                by_name.insert(name, sid);
                count += 1;
            }
            None => *skipped.entry(id.to_string()).or_default() += 1,
        }
    }

    // Scènes, dans l'ordre d'OBS.
    let order: Vec<String> =
        json["scene_order"].as_array().map(|a| a.iter().filter_map(|s| s["name"].as_str().map(String::from)).collect()).unwrap_or_default();
    let mut scenes: Vec<&Value> = sources.iter().filter(|s| s["id"] == "scene").collect();
    scenes.sort_by_key(|s| order.iter().position(|n| s["name"].as_str() == Some(n)).unwrap_or(usize::MAX));
    let mut first = None;
    let mut current = None;
    for sc in &scenes {
        let name = sc["name"].as_str().unwrap_or("Scène").to_string();
        let sid = project.new_id();
        let unique = project.unique_name(&name, project.scenes.iter().map(|s| s.name.clone()));
        let mut items = Vec::new();
        // OBS : du dernier (premier plan) au premier ; MyStream : premier plan en tête.
        for it in sc["settings"]["items"].as_array().cloned().unwrap_or_default().iter().rev() {
            let Some(&source) = it["name"].as_str().and_then(|n| by_name.get(n)) else { continue };
            let crop =
                [it["crop_left"].as_u64(), it["crop_top"].as_u64(), it["crop_right"].as_u64(), it["crop_bottom"].as_u64()].map(|v| v.unwrap_or(0) as u32);
            if crop != [0; 4]
                && let Some(s) = project.sources.iter_mut().find(|s| s.id == source)
            {
                s.filters.crop = crop;
            }
            items.push(Item {
                id: project.new_id(),
                source,
                x: it["pos"]["x"].as_f64().unwrap_or(0.0) as f32,
                y: it["pos"]["y"].as_f64().unwrap_or(0.0) as f32,
                scale: Some(it["scale"]["x"].as_f64().unwrap_or(1.0) as f32),
                visible: it["visible"].as_bool().unwrap_or(true),
                locked: it["locked"].as_bool().unwrap_or(false),
            });
        }
        project.scenes.push(Scene { id: sid, name: unique, items });
        first.get_or_insert(sid);
        if json["current_scene"].as_str() == Some(&name) {
            current = Some(sid);
        }
    }
    if let Some(id) = current.or(first) {
        project.current = id;
        project.preview = id;
    }
    let mut skipped: Vec<(String, usize)> = skipped.into_iter().collect();
    skipped.sort();
    Ok(Report { scenes: scenes.len(), sources: count, skipped })
}

/// Nom lisible d'un type de source OBS non pris en charge.
pub fn obs_type_label(id: &str) -> String {
    match id {
        "browser_source" => "navigateur".into(),
        "slideshow" | "slideshow_v2" => "diaporama".into(),
        "group" => "groupe".into(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn importe_une_collection() {
        let Ok(path) = std::env::var("OBS") else { return };
        let mut p = crate::model::Project::default();
        let r = super::import(std::path::Path::new(&path), &mut p).unwrap();
        for s in &p.sources {
            println!("source {} : {:?} vol {} crop {:?}", s.name, s.kind, s.volume, s.filters.crop);
        }
        for s in &p.scenes {
            println!("scène {} : {:?}", s.name, s.items.iter().map(|i| (i.source, i.x, i.y, i.scale, i.visible)).collect::<Vec<_>>());
        }
        println!("{} scènes, {} sources, ignorées {:?}, courante {}", r.scenes, r.sources, r.skipped, p.scene(p.current).unwrap().name);
    }
}
