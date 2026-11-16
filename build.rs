// Icône et informations de l'exécutable Windows.
fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico").set("ProductName", "MyStream").set("FileDescription", "MyStream").set("CompanyName", "BloopStudio");
        // Sans compilateur de ressources (compilation croisée), l'exécutable reste sans icône.
        if let Err(e) = res.compile() {
            println!("cargo:warning=icône non intégrée : {e}");
        }
    }
}
