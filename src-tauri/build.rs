fn main() {
    // tauri-build only watches tauri.conf.json and capabilities/, so a new
    // icon would never be embedded into the .exe without this.
    println!("cargo:rerun-if-changed=icons");
    tauri_build::build()
}
