//! What the sandbox offers a plugin of either format: a way to wake its
//! main thread, timers, a window that resizes, and a place to leave what
//! the plugin asked for until the main thread gets to it.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering::Relaxed};

use crate::protocol::Gesture;

/// What the sandbox does for a plugin when it asks (on its main thread).
pub trait Hooks: Send + Sync {
    /// Something was flagged: look at the [`HostState`] on the main thread.
    fn wake(&self);
    /// Starts (`Some` period) or stops a timer that calls the plugin back.
    fn timer(&self, id: u32, period_ms: Option<u32>) -> bool;
    /// The plugin's window wants this size.
    fn resize(&self, width: u32, height: u32) -> bool;
}

/// A parameter change the plugin made itself (no value: a gesture's start or end).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OutEvent {
    pub id: u32,
    pub value: Option<f64>,
    pub gesture: Gesture,
}

/// What a plugin asked of the host, from any thread. It lives at a fixed
/// address for as long as the plugin does.
pub struct HostState {
    pub hooks: Box<dyn Hooks>,
    pub main_thread: AtomicU32,
    pub audio_thread: AtomicU32,
    pub callback: AtomicBool,
    pub restart: AtomicBool,
    pub rescan: AtomicBool,
    pub dirty: AtomicBool,
    pub latency: AtomicBool,
    pub gui_closed: AtomicBool,
    pub flush: AtomicBool,
    /// Changes made in the plugin's own window (VST3 reports them on the
    /// main thread), for the app.
    pub edits: Mutex<Vec<OutEvent>>,
    /// The same changes, for the processor to hear in its next block.
    pub to_processor: Mutex<Vec<(u32, f64)>>,
    /// Values the processor got (from the app, or made itself) that a VST3
    /// controller must be told on the main thread; `true`: the plugin made it.
    pub to_controller: Mutex<Vec<(u32, f64, bool)>>,
    pub(crate) next_timer: AtomicU32,
}

impl HostState {
    pub fn new(hooks: Box<dyn Hooks>) -> Self {
        Self {
            hooks,
            main_thread: AtomicU32::new(current_thread()),
            audio_thread: AtomicU32::new(0),
            callback: AtomicBool::new(false),
            restart: AtomicBool::new(false),
            rescan: AtomicBool::new(false),
            dirty: AtomicBool::new(false),
            latency: AtomicBool::new(false),
            gui_closed: AtomicBool::new(false),
            flush: AtomicBool::new(false),
            edits: Mutex::new(Vec::new()),
            to_processor: Mutex::new(Vec::new()),
            to_controller: Mutex::new(Vec::new()),
            next_timer: AtomicU32::new(1),
        }
    }

    pub(crate) fn flag(&self, flag: &AtomicBool) {
        flag.store(true, Relaxed);
        self.hooks.wake();
    }

    pub fn is_main_thread(&self) -> bool {
        self.main_thread.load(Relaxed) == current_thread()
    }

    pub fn is_audio_thread(&self) -> bool {
        self.audio_thread.load(Relaxed) == current_thread()
    }
}

#[cfg(windows)]
fn current_thread() -> u32 {
    unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() }
}

#[cfg(not(windows))]
fn current_thread() -> u32 {
    0
}

/// Marks the calling thread as the one that processes audio.
pub fn mark_audio_thread(state: &HostState) {
    state.audio_thread.store(current_thread(), Relaxed);
}
