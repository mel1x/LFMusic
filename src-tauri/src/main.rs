// No console window on Windows in release. Do not remove.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    lf_music_lib::run()
}
