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

/// Journal : affiché dans la console et écrit dans le dossier des journaux
/// (Aide > Ouvrir le dossier des journaux), remis à zéro à chaque lancement.
fn init_logs() {
    struct Tee(Option<std::fs::File>);
    impl std::io::Write for Tee {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            let _ = std::io::stderr().write_all(buf);
            if let Some(f) = &mut self.0 {
                let _ = f.write_all(buf);
            }
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            self.0.as_mut().map(|f| f.flush()).unwrap_or(Ok(()))
        }
    }
    let dir = dirs::config_dir().unwrap_or_else(|| ".".into()).join("MyStream-natif").join("journaux");
    let _ = std::fs::create_dir_all(&dir);
    // Le journal précédent est gardé (utile après un plantage).
    let _ = std::fs::rename(dir.join("mystream.log"), dir.join("mystream-precedent.log"));
    let file = std::fs::File::create(dir.join("mystream.log")).ok();
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info,wgpu_core=warn,wgpu_hal=warn,naga=warn"))
        .target(env_logger::Target::Pipe(Box::new(Tee(file))))
        .init();
}

/// Logo de MyStream (barre des tâches, barre de titre).
fn icon() -> egui::IconData {
    let img = image::load_from_memory(include_bytes!("../assets/icon.png")).expect("logo").to_rgba8();
    egui::IconData { width: img.width(), height: img.height(), rgba: img.into_raw() }
}

fn main() -> eframe::Result {
    init_logs();
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
