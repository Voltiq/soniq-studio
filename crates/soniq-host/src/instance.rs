//! A plugin of either format, as the sandbox drives it. The format comes
//! from the file: `.vst3` is VST3, anything else CLAP.

use std::ffi::c_void;
use std::path::Path;
use std::sync::Arc;

use crate::clap;
use crate::host::{HostState, OutEvent};
use crate::protocol::{Descriptor, Loaded, ParamInfo};
use crate::shm::Block;
use crate::vst3;

pub fn is_vst3(path: &Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case("vst3"))
}

/// The plugins a file offers.
pub fn scan(path: &Path) -> Result<Vec<Descriptor>, String> {
    if is_vst3(path) { Ok(vst3::Module::open(path)?.descriptors()) } else { Ok(clap::Library::open(path)?.descriptors()) }
}

pub enum Instance {
    Clap(clap::Plugin),
    Vst3(vst3::Plugin),
}

/// The audio-thread half of an [`Instance`].
pub enum Processor {
    Clap(clap::Processor),
    Vst3(vst3::Processor),
}

impl Processor {
    /// # Safety
    /// `block` must be the shared block, and this side must hold the turn.
    pub unsafe fn process(&mut self, block: *mut Block) {
        match self {
            Self::Clap(p) => unsafe { p.process(block) },
            Self::Vst3(p) => unsafe { p.process(block) },
        }
    }

    /// Hands over the parameter changes the plugin made during the last block.
    pub fn drain_out(&mut self, mut f: impl FnMut(OutEvent)) {
        match self {
            Self::Clap(p) => p.take_out().for_each(&mut f),
            Self::Vst3(p) => p.take_out().for_each(&mut f),
        }
    }

    pub fn stop(&mut self) {
        match self {
            Self::Clap(p) => p.stop(),
            Self::Vst3(p) => p.stop(),
        }
    }
}

impl Instance {
    pub fn open(path: &Path, id: &str, host: Arc<HostState>) -> Result<Self, String> {
        Ok(if is_vst3(path) {
            Self::Vst3(vst3::Plugin::create(Arc::new(vst3::Module::open(path)?), id, host)?)
        } else {
            Self::Clap(clap::Plugin::create(Arc::new(clap::Library::open(path)?), id, host)?)
        })
    }

    pub fn loaded(&self) -> Loaded {
        match self {
            Self::Clap(p) => {
                let l = p.layout();
                Loaded {
                    name: p.name(),
                    inputs: l.main_in.map_or(0, |i| l.inputs[i]),
                    outputs: l.main_out.map_or(0, |i| l.outputs[i]),
                    notes: l.clap_notes || l.midi_notes,
                    gui: p.has_gui(),
                    latency: p.latency(),
                }
            }
            Self::Vst3(p) => {
                let l = p.layout();
                Loaded {
                    name: p.name(),
                    inputs: l.main_in.map_or(0, |i| l.inputs[i]),
                    outputs: l.main_out.map_or(0, |i| l.outputs[i]),
                    notes: l.notes,
                    gui: p.has_gui(),
                    latency: p.latency(),
                }
            }
        }
    }

    pub fn latency(&self) -> u32 {
        match self {
            Self::Clap(p) => p.latency(),
            Self::Vst3(p) => p.latency(),
        }
    }

    /// Starts the plugin at `rate`; its processor goes to the audio thread.
    pub fn activate(&mut self, rate: f64) -> Result<Processor, String> {
        Ok(match self {
            Self::Clap(p) => Processor::Clap(p.activate(rate)?),
            Self::Vst3(p) => Processor::Vst3(p.activate(rate)?),
        })
    }

    pub fn params(&self) -> Vec<ParamInfo> {
        match self {
            Self::Clap(p) => p.params(),
            Self::Vst3(p) => p.params(),
        }
    }

    pub fn texts(&self, ids: &[u32]) -> Vec<(u32, f64, String)> {
        match self {
            Self::Clap(p) => p.texts(ids),
            Self::Vst3(p) => p.texts(ids),
        }
    }

    pub fn save_state(&self) -> Result<Vec<u8>, String> {
        match self {
            Self::Clap(p) => p.save_state(),
            Self::Vst3(p) => p.save_state(),
        }
    }

    pub fn load_state(&self, state: &[u8]) -> Result<(), String> {
        match self {
            Self::Clap(p) => p.load_state(state),
            Self::Vst3(p) => p.load_state(state),
        }
    }

    /// Main-thread work the plugin asked for; what it changed by itself.
    pub fn main_thread(&self, callback: bool) -> Vec<OutEvent> {
        match self {
            Self::Clap(p) => {
                if callback {
                    p.on_main_thread();
                }
                Vec::new()
            }
            Self::Vst3(p) => p.sync_controller(),
        }
    }

    pub fn on_timer(&self, id: u32) {
        if let Self::Clap(p) = self {
            p.on_timer(id);
        }
    }

    /// It has a view that goes inside a window of the host's.
    pub fn embeds(&self) -> bool {
        match self {
            Self::Clap(p) => p.gui().is_some_and(|g| g.supports(false)),
            Self::Vst3(p) => p.has_gui(),
        }
    }

    /// It has a window of its own (CLAP only).
    pub fn floats(&self) -> bool {
        matches!(self, Self::Clap(p) if p.gui().is_some_and(|g| g.supports(true)))
    }

    /// Puts the plugin's view inside `hwnd` at `scale`; its size.
    pub fn embed(&mut self, hwnd: *mut c_void, scale: f64) -> Result<(u32, u32), String> {
        match self {
            Self::Clap(p) => {
                let gui = p.gui().ok_or("This plugin has no window of its own.")?;
                if !gui.create(false) {
                    return Err("The plugin could not open its window.".into());
                }
                gui.set_scale(scale);
                let size = gui.size().unwrap_or((480, 320));
                if !gui.set_parent(hwnd) {
                    gui.destroy();
                    return Err("The plugin could not open its window.".into());
                }
                gui.show();
                Ok(size)
            }
            Self::Vst3(p) => p.embed(hwnd, scale),
        }
    }

    /// Opens the plugin's own window (CLAP only).
    pub fn float(&mut self, title: &str) -> Result<(), String> {
        let Self::Clap(p) = self else { return Err("This plugin has no window of its own.".into()) };
        let gui = p.gui().ok_or("This plugin has no window of its own.")?;
        if !gui.create(true) {
            return Err("The plugin could not open its window.".into());
        }
        gui.suggest_title(title);
        gui.show();
        Ok(())
    }

    /// Takes the plugin's view (or own window) away.
    pub fn unembed(&mut self) {
        match self {
            Self::Clap(p) => {
                if let Some(gui) = p.gui() {
                    gui.hide();
                    gui.destroy();
                }
            }
            Self::Vst3(p) => p.unembed(),
        }
    }

    pub fn can_resize(&self) -> bool {
        match self {
            Self::Clap(p) => p.gui().is_some_and(|g| g.can_resize()),
            Self::Vst3(p) => p.can_resize(),
        }
    }

    pub fn set_size(&self, width: u32, height: u32) {
        match self {
            Self::Clap(p) => {
                if let Some(gui) = p.gui() {
                    gui.set_size(width, height);
                }
            }
            Self::Vst3(p) => p.set_size(width, height),
        }
    }

    pub fn set_scale(&self, scale: f64) {
        match self {
            Self::Clap(p) => {
                if let Some(gui) = p.gui() {
                    gui.set_scale(scale);
                }
            }
            Self::Vst3(p) => p.set_scale(scale),
        }
    }
}
