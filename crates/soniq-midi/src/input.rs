//! Hardware MIDI inputs. A background thread keeps every enabled input port
//! connected, picking up keyboards plugged in later and letting go of ones
//! that disappear.

use std::collections::HashSet;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

use midir::{Ignore, MidiInput, MidiInputConnection};
use serde::Serialize;

use crate::{MidiEvent, parse};

const CLIENT: &str = "Soniq Studio";
const RESCAN: Duration = Duration::from_secs(2);

/// Called on the driver's thread for every message Soniq understands, with
/// the name of the port it came from.
pub type Handler = Arc<dyn Fn(&str, MidiEvent) + Send + Sync>;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MidiPortStatus {
    pub name: String,
    pub enabled: bool,
    pub connected: bool,
}

#[derive(Default)]
struct State {
    connections: Vec<(String, MidiInputConnection<()>)>,
    available: Vec<String>,
    disabled: HashSet<String>,
}

#[derive(Clone)]
pub struct MidiInputs {
    state: Arc<Mutex<State>>,
    handler: Handler,
}

fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl MidiInputs {
    /// Starts watching the input ports; `disabled` names ports to leave alone.
    pub fn start(disabled: Vec<String>, handler: Handler) -> Self {
        let state = State { disabled: disabled.into_iter().collect(), ..State::default() };
        let inputs = Self { state: Arc::new(Mutex::new(state)), handler };
        let watcher = inputs.clone();
        let _ = thread::Builder::new().name("soniq-midi".into()).spawn(move || {
            loop {
                watcher.rescan();
                thread::sleep(RESCAN);
            }
        });
        inputs
    }

    /// Connects ports that appeared or were switched on, and drops ports
    /// that went away or were switched off.
    pub fn rescan(&self) {
        let names: Vec<String> = match MidiInput::new(CLIENT) {
            Ok(probe) => probe.ports().iter().filter_map(|p| probe.port_name(p).ok()).collect(),
            Err(_) => return,
        };
        let mut state = lock(&self.state);
        let wanted: Vec<String> = names.iter().filter(|n| !state.disabled.contains(*n)).cloned().collect();
        state.connections.retain(|(name, _)| wanted.contains(name));
        let missing: Vec<String> = wanted.into_iter().filter(|n| !state.connections.iter().any(|(c, _)| c == n)).collect();
        for name in missing {
            if let Some(connection) = self.connect(&name) {
                state.connections.push((name, connection));
            }
        }
        state.available = names;
    }

    fn connect(&self, name: &str) -> Option<MidiInputConnection<()>> {
        let mut input = MidiInput::new(CLIENT).ok()?;
        // Clock, active sensing and SysEx are of no use here.
        input.ignore(Ignore::All);
        let port = input.ports().into_iter().find(|p| input.port_name(p).ok().as_deref() == Some(name))?;
        let handler = Arc::clone(&self.handler);
        let port_name = name.to_string();
        input
            .connect(
                &port,
                CLIENT,
                move |_, bytes, _| {
                    if let Some(event) = parse(bytes) {
                        handler(&port_name, event);
                    }
                },
                (),
            )
            .ok()
    }

    pub fn status(&self) -> Vec<MidiPortStatus> {
        let state = lock(&self.state);
        state
            .available
            .iter()
            .map(|name| MidiPortStatus {
                name: name.clone(),
                enabled: !state.disabled.contains(name),
                connected: state.connections.iter().any(|(c, _)| c == name),
            })
            .collect()
    }

    pub fn set_enabled(&self, name: &str, enabled: bool) {
        {
            let mut state = lock(&self.state);
            if enabled {
                state.disabled.remove(name);
            } else {
                state.disabled.insert(name.to_string());
            }
        }
        self.rescan();
    }

    /// Ports switched off, to remember for next time.
    pub fn disabled(&self) -> Vec<String> {
        let mut names: Vec<String> = lock(&self.state).disabled.iter().cloned().collect();
        names.sort();
        names
    }
}
