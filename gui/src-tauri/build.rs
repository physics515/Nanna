#![warn(clippy::pedantic, clippy::nursery, clippy::all)]
fn main() {
    // Daemon sidecar is built by beforeDevCommand/beforeBuildCommand in tauri.conf.json
    // This avoids nested cargo deadlock issues
    tauri_build::build();
}
