#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if let Err(error) = litematica_preview::run() {
        eprintln!("Litematica Preview: {error}");
        std::process::exit(1);
    }
}
