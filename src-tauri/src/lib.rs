mod build;
mod commands;
mod crash;
mod demo;
mod files;
mod freeze;
mod midi;
mod plugins;
mod podcast;
mod processing;
mod scenes;
mod session;
mod settings;
mod templates;
mod warping;

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use soniq_engine::{AudioEngine, EngineConfig, Recorder};
use soniq_midi::MidiInputs;
use tauri::Manager;

use session::{AppState, Dirs, MidiRouting};

pub use plugins::SANDBOX_FLAG;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            let dirs = Dirs { config: app.path().app_config_dir()?, data: app.path().app_data_dir()? };
            std::fs::create_dir_all(&dirs.data)?;
            // Crash reports, kept only if the user switched them on.
            crash::install(dirs.crashes(), crash::load(&dirs.config).enabled);

            // Open the saved device; if it is gone, fall back to the system default.
            let saved = settings::load(&dirs.config);
            let engine = AudioEngine::new();
            let mut config = saved.clone();
            if engine.start(&saved).is_err() && saved != EngineConfig::default() {
                config = EngineConfig::default();
                if let Err(err) = engine.start(&config) {
                    eprintln!("no audio output: {err}");
                }
            }

            let recovery = files::stash_autosave(&dirs.autosave(), &dirs.recovered());
            let session = demo::session(engine.sample_rate());
            let midi_settings = settings::load_midi(&dirs.config);
            let (controls, control_moves) = std::sync::mpsc::channel();
            let state = AppState {
                session: Mutex::new(session),
                engine,
                recorder: Recorder::new(),
                record_tracks: Mutex::new(Vec::new()),
                midi_take: Mutex::new(None),
                midi_routing: Mutex::new(MidiRouting::default()),
                midi: Mutex::new(None),
                midi_activity: AtomicU64::new(0),
                midi_history: Mutex::new(VecDeque::with_capacity(session::HISTORY)),
                started: Instant::now(),
                punch: Mutex::new(None),
                pass: Mutex::new(None),
                noise_profile: Mutex::new(None),
                engine_config: Mutex::new(config),
                dirs,
                recovery_available: AtomicBool::new(recovery),
                meter_generation: AtomicU64::new(0),
                learning: Mutex::new(None),
                controls,
                revision: AtomicU64::new(0),
                session_take: Mutex::new(None),
                plugins: plugins::Plugins::new(),
                analyzer: Mutex::new(None),
            };
            app.manage(state);
            let state = app.state::<AppState>();
            state.plugins.set_app(app.handle().clone());
            state.sync_engine(&session::lock(&state.session));

            // Every MIDI keyboard plays the armed (or selected) MIDI tracks;
            // its knobs and faders move what they are mapped to.
            midi::spawn_control_worker(app.handle().clone(), control_moves);
            let handle = app.handle().clone();
            let inputs = MidiInputs::start(midi_settings.disabled_inputs, Arc::new(move |_port, event| handle.state::<AppState>().midi_in(event)));
            *session::lock(&app.state::<AppState>().midi) = Some(inputs);
            commands::spawn_autosave(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::project_get,
            commands::project_apply,
            commands::project_undo,
            commands::project_redo,
            commands::project_new,
            crash::crash_status,
            crash::crash_enable,
            crash::crash_seen,
            crash::crash_read,
            crash::crash_delete,
            crash::crash_ui,
            processing::clip_fix_analyse,
            podcast::podcast_measure,
            podcast::podcast_level,
            podcast::podcast_duck,
            podcast::chapters_export,
            commands::templates_list,
            commands::template_save,
            commands::template_delete,
            commands::recent_projects,
            commands::recent_forget,
            commands::history_list,
            commands::history_jump,
            commands::assets_find,
            commands::asset_locate,
            commands::project_open_demo,
            commands::project_open,
            commands::project_save,
            commands::project_import,
            commands::clip_edit,
            commands::clip_clipboard,
            commands::asset_peaks,
            processing::clip_effect,
            processing::clip_effect_preview,
            processing::preview_stop,
            processing::noise_profile_capture,
            processing::noise_profile_status,
            processing::clip_spectral,
            processing::asset_spectrogram,
            processing::asset_window,
            processing::clip_draw,
            processing::generate_audio,
            processing::macros_get,
            processing::macros_set,
            processing::batch_process,
            freeze::track_freeze,
            freeze::clip_render,
            midi::note_play,
            midi::note_sustain,
            midi::note_panic,
            midi::midi_select,
            midi::midi_activity,
            midi::midi_inputs,
            midi::midi_input_enable,
            midi::midi_import,
            midi::midi_export,
            midi::device_load_sample,
            midi::midi_capture,
            midi::sample_onsets,
            midi::midi_learn,
            midi::midi_simulate,
            commands::input_status,
            commands::record_start,
            commands::record_stop,
            commands::recordings_orphaned,
            commands::recordings_recover,
            commands::recordings_discard,
            commands::export_mix,
            commands::recovery_available,
            commands::recovery_restore,
            commands::recovery_discard,
            commands::app_closing,
            commands::engine_status,
            commands::engine_config,
            commands::audio_devices,
            commands::audio_device_options,
            commands::engine_configure,
            commands::transport_state,
            commands::transport_play,
            commands::transport_seek,
            commands::transport_set_metronome,
            commands::transport_set_loop,
            commands::meters_subscribe,
            commands::analysis_enable,
            commands::analysis_reset,
            scenes::session_launch,
            scenes::session_launch_scene,
            scenes::session_stop_clips,
            scenes::session_back_to_arrangement,
            warping::clip_warp,
            warping::slice_to_midi,
            plugins::plugins_list,
            plugins::plugins_set_folders,
            plugins::plugin_add,
            plugins::plugin_window,
            plugins::plugin_params,
            plugins::plugin_texts,
            plugins::plugin_reload,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Soniq Studio");
}
