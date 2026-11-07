//! Keeps the sound going when devices come and go. Once a second it looks at
//! the output: gone (the driver said so, or the callbacks stopped), back (the
//! device you picked is plugged in again while Soniq plays elsewhere), or
//! moved (you follow the system default and it changed). Then it reopens the
//! right device and sends the song over again. Playback stops on any switch,
//! the way a phone stops when headphones come out: nothing blasts from the
//! laptop's speakers by surprise. A lost input closes, and opens again when
//! it comes back while a track still needs it.

use std::time::Duration;

use serde::Serialize;
use soniq_engine::EngineConfig;
use tauri::{AppHandle, Emitter, Manager};

use crate::session::{AppState, lock};

/// What the interface hears about (event "audio-device").
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum DeviceEvent {
    /// The output went away; Soniq now plays on `device`.
    Lost { previous: String, device: String, stopped: bool },
    /// The output moved to `device` (the chosen one is back, or the default changed).
    Switched { previous: String, device: String, stopped: bool },
    /// No output could be opened; Soniq keeps trying.
    NoOutput { error: String, stopped: bool },
    /// The input went away (a recording stops where it got to).
    InputLost { device: String },
    InputBack { device: String },
}

enum Reason {
    Lost,
    Changed,
    Retry,
}

pub fn spawn(app: AppHandle) {
    let _ = std::thread::Builder::new().name("soniq-devices".into()).spawn(move || {
        let (mut last_calls, mut stalled, mut tick) = (u64::MAX, 0u32, 0u64);
        let mut last_error: Option<String> = None;
        let mut input_gone: Option<String> = None;
        loop {
            std::thread::sleep(Duration::from_secs(1));
            tick += 1;
            let state = app.state::<AppState>();
            let status = state.engine.status();
            let config = lock(&state.engine_config).clone();

            let calls = state.engine.callbacks();
            stalled = if status.running && calls == last_calls { stalled + 1 } else { 0 };
            last_calls = calls;

            let reason = if state.engine.take_lost() || stalled >= 3 {
                Some(Reason::Lost)
            } else if !status.running {
                (tick % 3 == 0).then_some(Reason::Retry)
            } else if tick % 2 == 0 {
                soniq_engine::output_device_name(&config).filter(|name| *name != status.device).map(|_| Reason::Changed)
            } else {
                None
            };
            if let Some(reason) = reason {
                stalled = 0;
                if let Some(event) = recover(&state, &config, &status.device, reason, &mut last_error) {
                    let _ = app.emit("audio-device", event);
                }
            }

            // The input: closed when it goes, opened again when it is back and wanted.
            if state.recorder.take_lost() {
                let device = state.recorder.status().device;
                state.close_input();
                let _ = app.emit("audio-device", DeviceEvent::InputLost { device: device.clone() });
                input_gone = Some(device);
            } else if input_gone.is_some() && tick % 2 == 0 && state.input_wanted() && state.open_input().is_ok() {
                input_gone = None;
                let _ = app.emit("audio-device", DeviceEvent::InputBack { device: state.recorder.status().device });
            }
        }
    });
}

fn recover(state: &AppState, config: &EngineConfig, previous: &str, reason: Reason, last_error: &mut Option<String>) -> Option<DeviceEvent> {
    let transport = state.engine.transport();
    let stopped = transport.is_playing();
    if stopped {
        transport.set_playing(false);
    }
    match crate::commands::apply_engine(state, config) {
        Ok(status) => {
            *last_error = None;
            let (previous, device) = (previous.to_string(), status.device);
            Some(match reason {
                Reason::Changed => DeviceEvent::Switched { previous, device, stopped },
                Reason::Lost | Reason::Retry => DeviceEvent::Lost { previous, device, stopped },
            })
        }
        // Said once, not every few seconds while nothing can be opened.
        Err(error) if last_error.as_deref() != Some(error.as_str()) => {
            *last_error = Some(error.clone());
            Some(DeviceEvent::NoOutput { error, stopped })
        }
        Err(_) => None,
    }
}
