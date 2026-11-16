//! MyStream natif (BloopStudio) : logiciel de streaming et d'enregistrement
//! écrit en Rust. Capture (Windows Graphics Capture), composition et
//! conversion des couleurs sur la carte graphique (wgpu), encodage matériel
//! par FFmpeg (NVENC, Quick Sync, AMF).

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod audio;
mod capture;
mod chat;
mod compositor;
mod engine;
mod model;
mod obs_import;
mod output;
mod overlay;
mod remote;
mod restream;
mod theme;
mod update;

/// Logo de MyStream (barre des tâches, barre de titre).
fn icon() -> egui::IconData {
    let img = image::load_from_memory(include_bytes!("../assets/icon.png")).expect("logo").to_rgba8();
    egui::IconData { width: img.width(), height: img.height(), rgba: img.into_raw() }
}

fn main() -> eframe::Result {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let mut options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default()
            .with_title("MyStream")
            .with_icon(icon())
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([1024.0, 680.0]),
        ..Default::default()
    };
    // La carte graphique la plus puissante (la même que le jeu sur un PC portable).
    if let egui_wgpu::WgpuSetup::CreateNew(setup) = &mut options.wgpu_options.wgpu_setup {
        setup.power_preference = wgpu::PowerPreference::HighPerformance;
    }
    eframe::run_native("MyStream", options, Box::new(|cc| Ok(Box::new(app::App::new(cc)))))
}
