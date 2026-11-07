// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Started again by itself to host a plugin out of harm's way.
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|a| a == soniq_studio_lib::SANDBOX_FLAG) {
        std::process::exit(soniq_host::sandbox_main(&args[1..]));
    }
    soniq_studio_lib::run()
}
