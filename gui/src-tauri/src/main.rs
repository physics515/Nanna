#![warn(clippy::pedantic, clippy::nursery, clippy::all)]
// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    nanna_gui_lib::run();
}
