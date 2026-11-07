//! CLAP from the host's side, inside the sandbox: opening a `.clap` file,
//! listing its plugins, and driving one of them. [`Plugin`] holds the calls
//! CLAP wants on the main thread (parameters, state, the window);
//! [`Processor`] the ones it wants on the audio thread.

use std::ffi::{CStr, CString, c_char, c_void};
use std::path::Path;
use std::ptr;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;

use clap_sys::audio_buffer::clap_audio_buffer;
use clap_sys::entry::clap_plugin_entry;
use clap_sys::events::*;
use clap_sys::ext::audio_ports::*;
use clap_sys::ext::gui::*;
use clap_sys::ext::latency::*;
use clap_sys::ext::log::*;
use clap_sys::ext::note_ports::*;
use clap_sys::ext::params::*;
use clap_sys::ext::state::*;
use clap_sys::ext::thread_check::*;
use clap_sys::ext::timer_support::*;
use clap_sys::factory::plugin_factory::*;
use clap_sys::fixedpoint::{CLAP_BEATTIME_FACTOR, CLAP_SECTIME_FACTOR};
use clap_sys::host::clap_host;
use clap_sys::id::clap_id;
use clap_sys::plugin::{clap_plugin, clap_plugin_descriptor};
use clap_sys::process::{CLAP_PROCESS_ERROR, clap_process};
use clap_sys::stream::{clap_istream, clap_ostream};
use clap_sys::version::{CLAP_VERSION, clap_version_is_compatible};

use crate::host::{HostState, OutEvent};
use crate::protocol::{Descriptor, Gesture, ParamInfo};
use crate::shm::{Block, MAX_EVENTS, MAX_FRAMES, RESET, kind};

fn text(p: *const c_char) -> String {
    if p.is_null() { String::new() } else { unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned() }
}

fn array_text(a: &[c_char]) -> String {
    let bytes: Vec<u8> = a.iter().take_while(|&&c| c != 0).map(|&c| c as u8).collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// An open `.clap` file. Its plugins must be gone before it closes.
pub struct Library {
    entry: *const clap_plugin_entry,
    _lib: libloading::Library,
}

unsafe impl Send for Library {}
unsafe impl Sync for Library {}

impl Library {
    pub fn open(path: &Path) -> Result<Self, String> {
        let lib = unsafe { libloading::Library::new(path) }.map_err(|e| format!("Could not load {}: {e}", path.display()))?;
        let entry = unsafe { lib.get::<*const clap_plugin_entry>(b"clap_entry\0") }.map(|s| *s).map_err(|_| "Not a CLAP plugin: it has no clap_entry.".to_string())?;
        if entry.is_null() || !clap_version_is_compatible(unsafe { (*entry).clap_version }) {
            return Err("The plugin needs a newer CLAP version.".into());
        }
        let c_path = CString::new(path.to_string_lossy().as_bytes()).map_err(|_| "Bad path.".to_string())?;
        let init = unsafe { (*entry).init }.ok_or("The plugin has no init.")?;
        if !unsafe { init(c_path.as_ptr()) } {
            return Err("The plugin refused to start.".into());
        }
        Ok(Self { entry, _lib: lib })
    }

    fn factory(&self) -> Option<&clap_plugin_factory> {
        let get = unsafe { (*self.entry).get_factory }?;
        let f = unsafe { get(CLAP_PLUGIN_FACTORY_ID.as_ptr()) } as *const clap_plugin_factory;
        unsafe { f.as_ref() }
    }

    /// The plugins in the file.
    pub fn descriptors(&self) -> Vec<Descriptor> {
        let Some(factory) = self.factory() else { return Vec::new() };
        let (Some(count), Some(get)) = (factory.get_plugin_count, factory.get_plugin_descriptor) else { return Vec::new() };
        (0..unsafe { count(factory) })
            .filter_map(|i| unsafe { get(factory, i).as_ref() })
            .map(|d: &clap_plugin_descriptor| {
                let mut features = Vec::new();
                if !d.features.is_null() {
                    let mut f = d.features;
                    while !unsafe { *f }.is_null() {
                        features.push(text(unsafe { *f }));
                        f = unsafe { f.add(1) };
                    }
                }
                Descriptor {
                    id: text(d.id),
                    name: text(d.name),
                    vendor: text(d.vendor),
                    version: text(d.version),
                    description: text(d.description),
                    features,
                }
            })
            .collect()
    }
}

impl Drop for Library {
    fn drop(&mut self) {
        if let Some(deinit) = unsafe { (*self.entry).deinit } {
            unsafe { deinit() };
        }
    }
}

unsafe fn state<'a>(host: *const clap_host) -> &'a HostState {
    unsafe { &*((*host).host_data as *const HostState) }
}

unsafe extern "C" fn host_extension(_: *const clap_host, id: *const c_char) -> *const c_void {
    let id = unsafe { CStr::from_ptr(id) };
    let table: [(&CStr, *const c_void); 9] = [
        (CLAP_EXT_LOG, &HOST_LOG as *const _ as *const c_void),
        (CLAP_EXT_THREAD_CHECK, &HOST_THREAD_CHECK as *const _ as *const c_void),
        (CLAP_EXT_PARAMS, &HOST_PARAMS as *const _ as *const c_void),
        (CLAP_EXT_STATE, &HOST_STATE as *const _ as *const c_void),
        (CLAP_EXT_LATENCY, &HOST_LATENCY as *const _ as *const c_void),
        (CLAP_EXT_GUI, &HOST_GUI as *const _ as *const c_void),
        (CLAP_EXT_TIMER_SUPPORT, &HOST_TIMER as *const _ as *const c_void),
        (CLAP_EXT_AUDIO_PORTS, &HOST_AUDIO_PORTS as *const _ as *const c_void),
        (CLAP_EXT_NOTE_PORTS, &HOST_NOTE_PORTS as *const _ as *const c_void),
    ];
    table.iter().find(|(name, _)| *name == id).map_or(ptr::null(), |(_, ext)| *ext)
}

unsafe extern "C" fn request_restart(host: *const clap_host) {
    let s = unsafe { state(host) };
    s.flag(&s.restart);
}

unsafe extern "C" fn request_process(_: *const clap_host) {
    // The engine runs every plugin all the time.
}

unsafe extern "C" fn request_callback(host: *const clap_host) {
    let s = unsafe { state(host) };
    s.flag(&s.callback);
}

unsafe extern "C" fn log(_: *const clap_host, severity: clap_log_severity, msg: *const c_char) {
    if severity >= CLAP_LOG_WARNING {
        eprintln!("[plugin] {}", text(msg));
    }
}

unsafe extern "C" fn is_main_thread(host: *const clap_host) -> bool {
    unsafe { state(host) }.is_main_thread()
}

unsafe extern "C" fn is_audio_thread(host: *const clap_host) -> bool {
    unsafe { state(host) }.is_audio_thread()
}

unsafe extern "C" fn params_rescan(host: *const clap_host, _: clap_param_rescan_flags) {
    let s = unsafe { state(host) };
    s.flag(&s.rescan);
}

unsafe extern "C" fn params_clear(_: *const clap_host, _: clap_id, _: clap_param_clear_flags) {}

unsafe extern "C" fn params_request_flush(host: *const clap_host) {
    let s = unsafe { state(host) };
    s.flag(&s.flush);
}

unsafe extern "C" fn mark_dirty(host: *const clap_host) {
    let s = unsafe { state(host) };
    s.flag(&s.dirty);
}

unsafe extern "C" fn latency_changed(host: *const clap_host) {
    let s = unsafe { state(host) };
    s.flag(&s.latency);
}

unsafe extern "C" fn gui_hints_changed(_: *const clap_host) {}

unsafe extern "C" fn gui_request_resize(host: *const clap_host, width: u32, height: u32) -> bool {
    unsafe { state(host) }.hooks.resize(width, height)
}

unsafe extern "C" fn gui_request_show(_: *const clap_host) -> bool {
    false
}

unsafe extern "C" fn gui_request_hide(_: *const clap_host) -> bool {
    false
}

unsafe extern "C" fn gui_closed(host: *const clap_host, _: bool) {
    let s = unsafe { state(host) };
    s.flag(&s.gui_closed);
}

unsafe extern "C" fn register_timer(host: *const clap_host, period_ms: u32, id: *mut clap_id) -> bool {
    let s = unsafe { state(host) };
    let next = s.next_timer.fetch_add(1, Relaxed);
    if !s.hooks.timer(next, Some(period_ms.max(1))) {
        return false;
    }
    unsafe { *id = next };
    true
}

unsafe extern "C" fn unregister_timer(host: *const clap_host, id: clap_id) -> bool {
    unsafe { state(host) }.hooks.timer(id, None)
}

unsafe extern "C" fn ports_flag_supported(_: *const clap_host, _: u32) -> bool {
    false
}

unsafe extern "C" fn ports_rescan(host: *const clap_host, _: u32) {
    let s = unsafe { state(host) };
    s.flag(&s.restart);
}

unsafe extern "C" fn note_dialects(_: *const clap_host) -> clap_note_dialect {
    CLAP_NOTE_DIALECT_CLAP | CLAP_NOTE_DIALECT_MIDI
}

unsafe extern "C" fn note_ports_rescan(_: *const clap_host, _: u32) {}

static HOST_LOG: clap_host_log = clap_host_log { log: Some(log) };
static HOST_THREAD_CHECK: clap_host_thread_check = clap_host_thread_check { is_main_thread: Some(is_main_thread), is_audio_thread: Some(is_audio_thread) };
static HOST_PARAMS: clap_host_params = clap_host_params { rescan: Some(params_rescan), clear: Some(params_clear), request_flush: Some(params_request_flush) };
static HOST_STATE: clap_host_state = clap_host_state { mark_dirty: Some(mark_dirty) };
static HOST_LATENCY: clap_host_latency = clap_host_latency { changed: Some(latency_changed) };
static HOST_GUI: clap_host_gui = clap_host_gui {
    resize_hints_changed: Some(gui_hints_changed),
    request_resize: Some(gui_request_resize),
    request_show: Some(gui_request_show),
    request_hide: Some(gui_request_hide),
    closed: Some(gui_closed),
};
static HOST_TIMER: clap_host_timer_support = clap_host_timer_support { register_timer: Some(register_timer), unregister_timer: Some(unregister_timer) };
static HOST_AUDIO_PORTS: clap_host_audio_ports = clap_host_audio_ports { is_rescan_flag_supported: Some(ports_flag_supported), rescan: Some(ports_rescan) };
static HOST_NOTE_PORTS: clap_host_note_ports = clap_host_note_ports { supported_dialects: Some(note_dialects), rescan: Some(note_ports_rescan) };

/// Channels on a plugin's audio ports, and how it takes notes.
#[derive(Debug, Clone, Default)]
pub struct Layout {
    pub inputs: Vec<u32>,
    pub outputs: Vec<u32>,
    pub main_in: Option<usize>,
    pub main_out: Option<usize>,
    /// It takes notes: as CLAP note events, as MIDI, or both.
    pub clap_notes: bool,
    pub midi_notes: bool,
}

/// One plugin, driven from the sandbox's main thread.
pub struct Plugin {
    plugin: *const clap_plugin,
    params: *const clap_plugin_params,
    state: *const clap_plugin_state,
    gui: *const clap_plugin_gui,
    latency: *const clap_plugin_latency,
    timer: *const clap_plugin_timer_support,
    active: bool,
    _host: Box<clap_host>,
    /// What `_host` points at.
    _state: Arc<HostState>,
    // Declared last: the file closes after the plugin is gone.
    _library: Arc<Library>,
}

unsafe impl Send for Plugin {}

impl Plugin {
    pub fn create(library: Arc<Library>, id: &str, host_state: Arc<HostState>) -> Result<Self, String> {
        let factory = library.factory().ok_or("The file has no plugins.")?;
        let create = factory.create_plugin.ok_or("The file cannot make plugins.")?;
        let host = Box::new(clap_host {
            clap_version: CLAP_VERSION,
            host_data: Arc::as_ptr(&host_state) as *mut c_void,
            name: c"Soniq Studio".as_ptr(),
            vendor: c"Soniq".as_ptr(),
            url: c"https://github.com/Voltiq/soniq-studio".as_ptr(),
            version: c"0.1.0".as_ptr(),
            get_extension: Some(host_extension),
            request_restart: Some(request_restart),
            request_process: Some(request_process),
            request_callback: Some(request_callback),
        });
        let c_id = CString::new(id).map_err(|_| "Bad plugin id.".to_string())?;
        let plugin = unsafe { create(factory, &*host, c_id.as_ptr()) };
        if plugin.is_null() {
            return Err(format!("The file has no plugin “{id}”."));
        }
        let init = unsafe { (*plugin).init };
        if !init.is_some_and(|f| unsafe { f(plugin) }) {
            if let Some(destroy) = unsafe { (*plugin).destroy } {
                unsafe { destroy(plugin) };
            }
            return Err("The plugin failed to start.".into());
        }
        let ext = |id: &CStr| unsafe { (*plugin).get_extension.map_or(ptr::null(), |f| f(plugin, id.as_ptr())) };
        Ok(Self {
            plugin,
            params: ext(CLAP_EXT_PARAMS).cast(),
            state: ext(CLAP_EXT_STATE).cast(),
            gui: ext(CLAP_EXT_GUI).cast(),
            latency: ext(CLAP_EXT_LATENCY).cast(),
            timer: ext(CLAP_EXT_TIMER_SUPPORT).cast(),
            active: false,
            _host: host,
            _state: host_state,
            _library: library,
        })
    }

    pub fn name(&self) -> String {
        unsafe { (*self.plugin).desc.as_ref() }.map(|d| text(d.name)).unwrap_or_default()
    }

    fn ext<T>(&self, id: &CStr) -> *const T {
        unsafe { (*self.plugin).get_extension.map_or(ptr::null(), |f| f(self.plugin, id.as_ptr())) }.cast()
    }

    pub fn layout(&self) -> Layout {
        let mut layout = Layout::default();
        if let Some(ports) = unsafe { self.ext::<clap_plugin_audio_ports>(CLAP_EXT_AUDIO_PORTS).as_ref() }
            && let (Some(count), Some(get)) = (ports.count, ports.get)
        {
            for input in [true, false] {
                let mut channels = Vec::new();
                let mut main = None;
                for i in 0..unsafe { count(self.plugin, input) } {
                    let mut info: clap_audio_port_info = unsafe { std::mem::zeroed() };
                    if unsafe { get(self.plugin, i, input, &mut info) } {
                        if main.is_none() && info.flags & CLAP_AUDIO_PORT_IS_MAIN != 0 {
                            main = Some(channels.len());
                        }
                        channels.push(info.channel_count);
                    }
                }
                let main = main.or((!channels.is_empty()).then_some(0));
                if input {
                    (layout.inputs, layout.main_in) = (channels, main);
                } else {
                    (layout.outputs, layout.main_out) = (channels, main);
                }
            }
        }
        if let Some(ports) = unsafe { self.ext::<clap_plugin_note_ports>(CLAP_EXT_NOTE_PORTS).as_ref() }
            && let (Some(count), Some(get)) = (ports.count, ports.get)
            && unsafe { count(self.plugin, true) } > 0
        {
            let mut info: clap_note_port_info = unsafe { std::mem::zeroed() };
            if unsafe { get(self.plugin, 0, true, &mut info) } {
                layout.clap_notes = info.supported_dialects & CLAP_NOTE_DIALECT_CLAP != 0;
                layout.midi_notes = info.supported_dialects & CLAP_NOTE_DIALECT_MIDI != 0;
            }
        }
        layout
    }

    pub fn latency(&self) -> u32 {
        unsafe { self.latency.as_ref() }.and_then(|l| l.get).map_or(0, |get| unsafe { get(self.plugin) })
    }

    /// Starts the plugin at `rate`; the processor goes to the audio thread.
    pub fn activate(&mut self, rate: f64) -> Result<Processor, String> {
        self.deactivate();
        let layout = self.layout();
        let activate = unsafe { (*self.plugin).activate }.ok_or("The plugin cannot start.")?;
        if !unsafe { activate(self.plugin, rate, 1, MAX_FRAMES as u32) } {
            return Err(format!("The plugin refused to run at {rate} Hz."));
        }
        self.active = true;
        Ok(Processor::new(self.plugin, layout))
    }

    /// Stops the plugin. Its processor must have stopped processing first.
    pub fn deactivate(&mut self) {
        if std::mem::take(&mut self.active)
            && let Some(deactivate) = unsafe { (*self.plugin).deactivate }
        {
            unsafe { deactivate(self.plugin) };
        }
    }

    pub fn on_main_thread(&self) {
        if let Some(f) = unsafe { (*self.plugin).on_main_thread } {
            unsafe { f(self.plugin) };
        }
    }

    pub fn on_timer(&self, id: u32) {
        if let Some(on_timer) = unsafe { self.timer.as_ref() }.and_then(|t| t.on_timer) {
            unsafe { on_timer(self.plugin, id) };
        }
    }

    fn value_text(&self, id: clap_id, value: f64) -> String {
        let Some(to_text) = unsafe { self.params.as_ref() }.and_then(|p| p.value_to_text) else { return format!("{value:.2}") };
        let mut buf = [0 as c_char; 256];
        if unsafe { to_text(self.plugin, id, value, buf.as_mut_ptr(), buf.len() as u32) } { array_text(&buf) } else { format!("{value:.2}") }
    }

    fn value(&self, id: clap_id) -> Option<f64> {
        let get = unsafe { self.params.as_ref() }?.get_value?;
        let mut value = 0.0;
        unsafe { get(self.plugin, id, &mut value) }.then_some(value)
    }

    pub fn params(&self) -> Vec<ParamInfo> {
        let Some(params) = (unsafe { self.params.as_ref() }) else { return Vec::new() };
        let (Some(count), Some(get_info)) = (params.count, params.get_info) else { return Vec::new() };
        (0..unsafe { count(self.plugin) })
            .filter_map(|i| {
                let mut info: clap_param_info = unsafe { std::mem::zeroed() };
                unsafe { get_info(self.plugin, i, &mut info) }.then_some(info)
            })
            .map(|info| {
                // Numbers a plugin gets wrong are made finite, so they travel as JSON.
                let finite = |v: f64, or: f64| if v.is_finite() { v } else { or };
                let (min, max) = (finite(info.min_value, 0.0), finite(info.max_value, 1.0));
                let default = finite(info.default_value, min);
                let value = finite(self.value(info.id).unwrap_or(default), default);
                ParamInfo {
                    id: info.id,
                    name: array_text(&info.name),
                    module: array_text(&info.module),
                    min,
                    max,
                    default,
                    value,
                    text: self.value_text(info.id, value),
                    stepped: info.flags & CLAP_PARAM_IS_STEPPED != 0,
                    steps: 0,
                    automatable: info.flags & CLAP_PARAM_IS_AUTOMATABLE != 0,
                    hidden: info.flags & CLAP_PARAM_IS_HIDDEN != 0,
                    readonly: info.flags & CLAP_PARAM_IS_READONLY != 0,
                }
            })
            .collect()
    }

    /// The value and text of each parameter.
    pub fn texts(&self, ids: &[u32]) -> Vec<(u32, f64, String)> {
        ids.iter().filter_map(|&id| self.value(id).filter(|v| v.is_finite()).map(|v| (id, v, self.value_text(id, v)))).collect()
    }

    pub fn save_state(&self) -> Result<Vec<u8>, String> {
        let save = unsafe { self.state.as_ref() }.and_then(|s| s.save).ok_or("The plugin cannot save its state.")?;
        let mut data: Vec<u8> = Vec::new();
        let stream = clap_ostream { ctx: (&mut data as *mut Vec<u8>).cast(), write: Some(write_stream) };
        if unsafe { save(self.plugin, &stream) } { Ok(data) } else { Err("The plugin failed to save its state.".into()) }
    }

    pub fn load_state(&self, data: &[u8]) -> Result<(), String> {
        let load = unsafe { self.state.as_ref() }.and_then(|s| s.load).ok_or("The plugin cannot load a state.")?;
        let mut reader = (data, 0usize);
        let stream = clap_istream { ctx: (&mut reader as *mut (&[u8], usize)).cast(), read: Some(read_stream) };
        if unsafe { load(self.plugin, &stream) } { Ok(()) } else { Err("The plugin could not read its saved state.".into()) }
    }

    pub fn has_gui(&self) -> bool {
        unsafe { self.gui.as_ref() }
            .and_then(|g| g.is_api_supported)
            .is_some_and(|ok| unsafe { ok(self.plugin, CLAP_WINDOW_API_WIN32.as_ptr(), false) || ok(self.plugin, CLAP_WINDOW_API_WIN32.as_ptr(), true) })
    }

    pub fn gui(&self) -> Option<Gui<'_>> {
        unsafe { self.gui.as_ref() }.map(|gui| Gui { plugin: self.plugin, gui })
    }
}

impl Drop for Plugin {
    fn drop(&mut self) {
        self.deactivate();
        if let Some(destroy) = unsafe { (*self.plugin).destroy } {
            unsafe { destroy(self.plugin) };
        }
    }
}

/// A plugin's window calls (main thread).
pub struct Gui<'a> {
    plugin: *const clap_plugin,
    gui: &'a clap_plugin_gui,
}

impl Gui<'_> {
    pub fn supports(&self, floating: bool) -> bool {
        self.gui.is_api_supported.is_some_and(|f| unsafe { f(self.plugin, CLAP_WINDOW_API_WIN32.as_ptr(), floating) })
    }

    pub fn create(&self, floating: bool) -> bool {
        self.gui.create.is_some_and(|f| unsafe { f(self.plugin, CLAP_WINDOW_API_WIN32.as_ptr(), floating) })
    }

    pub fn destroy(&self) {
        if let Some(f) = self.gui.destroy {
            unsafe { f(self.plugin) };
        }
    }

    pub fn set_scale(&self, scale: f64) {
        if let Some(f) = self.gui.set_scale {
            unsafe { f(self.plugin, scale) };
        }
    }

    pub fn size(&self) -> Option<(u32, u32)> {
        let f = self.gui.get_size?;
        let (mut w, mut h) = (0, 0);
        unsafe { f(self.plugin, &mut w, &mut h) }.then_some((w, h))
    }

    pub fn can_resize(&self) -> bool {
        self.gui.can_resize.is_some_and(|f| unsafe { f(self.plugin) })
    }

    /// Asks for a size; the size it will take.
    pub fn set_size(&self, width: u32, height: u32) -> Option<(u32, u32)> {
        let (mut w, mut h) = (width, height);
        if let Some(adjust) = self.gui.adjust_size {
            unsafe { adjust(self.plugin, &mut w, &mut h) };
        }
        let set = self.gui.set_size?;
        unsafe { set(self.plugin, w, h) }.then_some((w, h))
    }

    pub fn set_parent(&self, hwnd: *mut c_void) -> bool {
        let window = clap_window { api: CLAP_WINDOW_API_WIN32.as_ptr(), specific: clap_window_handle { win32: hwnd } };
        self.gui.set_parent.is_some_and(|f| unsafe { f(self.plugin, &window) })
    }

    pub fn suggest_title(&self, title: &str) {
        if let (Some(f), Ok(t)) = (self.gui.suggest_title, CString::new(title)) {
            unsafe { f(self.plugin, t.as_ptr()) };
        }
    }

    pub fn show(&self) -> bool {
        self.gui.show.is_some_and(|f| unsafe { f(self.plugin) })
    }

    pub fn hide(&self) {
        if let Some(f) = self.gui.hide {
            unsafe { f(self.plugin) };
        }
    }
}

unsafe extern "C" fn write_stream(stream: *const clap_ostream, buffer: *const c_void, size: u64) -> i64 {
    let out = unsafe { &mut *((*stream).ctx as *mut Vec<u8>) };
    out.extend_from_slice(unsafe { std::slice::from_raw_parts(buffer as *const u8, size as usize) });
    size as i64
}

unsafe extern "C" fn read_stream(stream: *const clap_istream, buffer: *mut c_void, size: u64) -> i64 {
    let (data, pos) = unsafe { &mut *((*stream).ctx as *mut (&[u8], usize)) };
    let n = (size as usize).min(data.len() - *pos);
    unsafe { ptr::copy_nonoverlapping(data[*pos..].as_ptr(), buffer as *mut u8, n) };
    *pos += n;
    n as i64
}

/// Any event the host sends, as CLAP lays it out.
#[repr(C)]
#[derive(Clone, Copy)]
pub union AnyEvent {
    header: clap_event_header,
    note: clap_event_note,
    param: clap_event_param_value,
    midi: clap_event_midi,
}

unsafe extern "C" fn in_size(list: *const clap_input_events) -> u32 {
    unsafe { (*((*list).ctx as *const Vec<AnyEvent>)).len() as u32 }
}

unsafe extern "C" fn in_get(list: *const clap_input_events, index: u32) -> *const clap_event_header {
    let events = unsafe { &*((*list).ctx as *const Vec<AnyEvent>) };
    events.get(index as usize).map_or(ptr::null(), |e| e as *const AnyEvent as *const clap_event_header)
}

unsafe extern "C" fn out_push(list: *const clap_output_events, event: *const clap_event_header) -> bool {
    let out = unsafe { &mut *((*list).ctx as *mut Vec<OutEvent>) };
    let header = unsafe { &*event };
    if header.space_id != CLAP_CORE_EVENT_SPACE_ID || out.len() >= out.capacity() {
        return true;
    }
    match header.type_ {
        CLAP_EVENT_PARAM_VALUE => {
            let e = unsafe { &*(event as *const clap_event_param_value) };
            if e.value.is_finite() {
                out.push(OutEvent { id: e.param_id, value: Some(e.value), gesture: Gesture::None });
            }
        }
        CLAP_EVENT_PARAM_GESTURE_BEGIN | CLAP_EVENT_PARAM_GESTURE_END => {
            let e = unsafe { &*(event as *const clap_event_param_gesture) };
            let gesture = if header.type_ == CLAP_EVENT_PARAM_GESTURE_BEGIN { Gesture::Begin } else { Gesture::End };
            out.push(OutEvent { id: e.param_id, value: None, gesture });
        }
        _ => {}
    }
    true
}

fn input_list(events: &mut Vec<AnyEvent>) -> clap_input_events {
    clap_input_events { ctx: (events as *mut Vec<AnyEvent>).cast(), size: Some(in_size), get: Some(in_get) }
}

fn output_list(out: &mut Vec<OutEvent>) -> clap_output_events {
    clap_output_events { ctx: (out as *mut Vec<OutEvent>).cast(), try_push: Some(out_push) }
}

fn header(time: u32, type_: u16, size: usize) -> clap_event_header {
    clap_event_header { size: size as u32, time, space_id: CLAP_CORE_EVENT_SPACE_ID, type_, flags: 0 }
}

/// One plugin, driven from the sandbox's audio thread. Buffers are made
/// when it starts, so processing allocates nothing.
pub struct Processor {
    plugin: *const clap_plugin,
    layout: Layout,
    started: bool,
    steady: i64,
    /// A buffer for every channel the shared block does not hold.
    scratch: Vec<Vec<f32>>,
    events: Vec<AnyEvent>,
    out: Vec<OutEvent>,
    /// Parameters the plugin's own window is moving: the host leaves them alone.
    held: Vec<u32>,
}

unsafe impl Send for Processor {}

impl Processor {
    fn new(plugin: *const clap_plugin, layout: Layout) -> Self {
        let channels: u32 = layout.inputs.iter().chain(&layout.outputs).sum::<u32>() + 2;
        Self {
            plugin,
            scratch: (0..channels).map(|_| vec![0.0; MAX_FRAMES]).collect(),
            layout,
            started: false,
            steady: 0,
            events: Vec::with_capacity(MAX_EVENTS + 8),
            out: Vec::with_capacity(1024),
            held: Vec::with_capacity(32),
        }
    }

    /// Stops processing (before the plugin is deactivated).
    pub fn stop(&mut self) {
        if std::mem::take(&mut self.started)
            && let Some(stop) = unsafe { (*self.plugin).stop_processing }
        {
            unsafe { stop(self.plugin) };
        }
    }

    /// The parameter changes the plugin made during the last block.
    pub fn take_out(&mut self) -> impl Iterator<Item = OutEvent> + '_ {
        self.out.drain(..)
    }

    /// Processes the shared block: its input, notes and parameter changes
    /// in, its output out.
    ///
    /// # Safety
    /// `block` must be the shared block, and this side must hold the turn.
    pub unsafe fn process(&mut self, block: *mut Block) {
        let b = unsafe { &mut *block };
        let n = (b.frames as usize).min(MAX_FRAMES);
        if !self.started {
            self.started = unsafe { (*self.plugin).start_processing }.is_none_or(|f| unsafe { f(self.plugin) });
            if !self.started {
                self.bypass(b, n);
                return;
            }
        }
        if b.flags & RESET != 0
            && let Some(reset) = unsafe { (*self.plugin).reset }
        {
            unsafe { reset(self.plugin) };
        }

        self.out.clear();
        self.events.clear();
        let transport = b.transport;
        let mut flags = CLAP_TRANSPORT_HAS_TEMPO | CLAP_TRANSPORT_HAS_BEATS_TIMELINE | CLAP_TRANSPORT_HAS_SECONDS_TIMELINE;
        if transport.playing != 0 {
            flags |= CLAP_TRANSPORT_IS_PLAYING;
        }
        let clock = clap_event_transport {
            header: header(0, CLAP_EVENT_TRANSPORT, std::mem::size_of::<clap_event_transport>()),
            flags,
            song_pos_beats: (transport.beat * CLAP_BEATTIME_FACTOR as f64) as i64,
            song_pos_seconds: (transport.seconds * CLAP_SECTIME_FACTOR as f64) as i64,
            tempo: transport.tempo,
            tempo_inc: 0.0,
            loop_start_beats: 0,
            loop_end_beats: 0,
            loop_start_seconds: 0,
            loop_end_seconds: 0,
            bar_start: 0,
            bar_number: 0,
            tsig_num: 4,
            tsig_denom: 4,
        };
        for e in &b.events[..(b.event_count as usize).min(MAX_EVENTS)] {
            let at = e.at.min(n.saturating_sub(1) as u32);
            if let Some(event) = self.event(at, e) {
                self.events.push(event);
            }
        }

        // Buffers: the main ports use the shared block, everything else scratch.
        let (input, output) = (b.input.as_mut_ptr(), b.output.as_mut_ptr());
        let mut scratch = self.scratch.iter_mut();
        let mut pointers: [[*mut f32; 64]; 2] = [[ptr::null_mut(); 64]; 2];
        let mut used = [0usize; 2];
        let mut buffers: [[clap_audio_buffer; MAX_PORTS]; 2] = unsafe { std::mem::zeroed() };
        let mut ports_used = [0usize; 2];
        let mut mono_in = None;
        let mut mono_out = None;
        for (side, (ports, main)) in [(&self.layout.inputs, self.layout.main_in), (&self.layout.outputs, self.layout.main_out)].into_iter().enumerate() {
            for (p, &channels) in ports.iter().enumerate().take(MAX_PORTS) {
                let first = used[side];
                for c in 0..channels as usize {
                    let at = used[side];
                    if at >= 64 {
                        break;
                    }
                    let shared = if side == 0 { input } else { output };
                    pointers[side][at] = if Some(p) == main && channels >= 2 && c < 2 {
                        unsafe { (*shared.add(c)).as_mut_ptr() }
                    } else if let Some(buf) = scratch.next() {
                        if side == 0 {
                            buf[..n].fill(0.0);
                        }
                        if Some(p) == main && channels == 1 {
                            if side == 0 {
                                mono_in = Some(buf.as_mut_ptr());
                            } else {
                                mono_out = Some(buf.as_mut_ptr());
                            }
                        }
                        buf.as_mut_ptr()
                    } else {
                        ptr::null_mut()
                    };
                    used[side] += 1;
                }
                buffers[side][p] = clap_audio_buffer {
                    data32: unsafe { pointers[side].as_mut_ptr().add(first) },
                    data64: ptr::null_mut(),
                    channel_count: (used[side] - first) as u32,
                    latency: 0,
                    constant_mask: 0,
                };
                ports_used[side] = p + 1;
            }
        }
        if let Some(mono) = mono_in {
            for i in 0..n {
                unsafe { *mono.add(i) = 0.5 * (b.input[0][i] + b.input[1][i]) };
            }
        }

        let in_events = input_list(&mut self.events);
        let out_events = output_list(&mut self.out);
        let process = clap_process {
            steady_time: self.steady,
            frames_count: n as u32,
            transport: &clock,
            audio_inputs: buffers[0].as_ptr(),
            audio_outputs: buffers[1].as_mut_ptr(),
            audio_inputs_count: ports_used[0] as u32,
            audio_outputs_count: ports_used[1] as u32,
            in_events: &in_events,
            out_events: &out_events,
        };
        let status = unsafe { (*self.plugin).process }.map_or(CLAP_PROCESS_ERROR, |f| unsafe { f(self.plugin, &process) });
        self.steady += n as i64;
        // What the plugin moves itself it keeps: the host's changes to those are dropped.
        for e in &self.out {
            match e.gesture {
                Gesture::Begin if !self.held.contains(&e.id) && self.held.len() < self.held.capacity() => self.held.push(e.id),
                Gesture::End => self.held.retain(|&id| id != e.id),
                _ => {}
            }
        }
        if status == CLAP_PROCESS_ERROR {
            self.bypass(b, n);
        } else if let Some(mono) = mono_out {
            for i in 0..n {
                let s = unsafe { *mono.add(i) };
                (b.output[0][i], b.output[1][i]) = (s, s);
            }
        } else if self.layout.main_out.is_none_or(|p| self.layout.outputs[p] == 0) {
            b.output[0][..n].fill(0.0);
            b.output[1][..n].fill(0.0);
        }
    }

    /// No answer from the plugin: an effect passes its input on, an instrument is silent.
    fn bypass(&self, b: &mut Block, n: usize) {
        for c in 0..2 {
            if self.layout.main_in.is_some() {
                let input = b.input[c];
                b.output[c][..n].copy_from_slice(&input[..n]);
            } else {
                b.output[c][..n].fill(0.0);
            }
        }
    }

    fn event(&self, at: u32, e: &crate::shm::Event) -> Option<AnyEvent> {
        let (clap, midi) = (self.layout.clap_notes, self.layout.midi_notes);
        let note = |type_: u16, key: i16, velocity: f64| AnyEvent {
            note: clap_event_note {
                header: header(at, type_, std::mem::size_of::<clap_event_note>()),
                note_id: -1,
                port_index: if key < 0 { -1 } else { 0 },
                channel: if key < 0 { -1 } else { 0 },
                key,
                velocity,
            },
        };
        let midi_event = |data: [u8; 3]| AnyEvent { midi: clap_event_midi { header: header(at, CLAP_EVENT_MIDI, std::mem::size_of::<clap_event_midi>()), port_index: 0, data } };
        let velocity = |v: f64| (v.clamp(0.0, 1.0) * 127.0).round().max(1.0) as u8;
        let key = e.id.min(127) as u8;
        Some(match e.kind {
            kind::NOTE_ON if clap => note(CLAP_EVENT_NOTE_ON, key as i16, e.value.clamp(0.0, 1.0)),
            kind::NOTE_ON if midi => midi_event([0x90, key, velocity(e.value)]),
            kind::NOTE_OFF if clap => note(CLAP_EVENT_NOTE_OFF, key as i16, 0.0),
            kind::NOTE_OFF if midi => midi_event([0x80, key, 0]),
            kind::SUSTAIN if midi => midi_event([0xB0, 64, if e.value > 0.5 { 127 } else { 0 }]),
            kind::ALL_NOTES_OFF if midi => midi_event([0xB0, 123, 0]),
            kind::ALL_NOTES_OFF if clap => note(CLAP_EVENT_NOTE_OFF, -1, 0.0),
            kind::ALL_SOUND_OFF if midi => midi_event([0xB0, 120, 0]),
            kind::ALL_SOUND_OFF if clap => note(CLAP_EVENT_NOTE_CHOKE, -1, 0.0),
            kind::PARAM if !self.held.contains(&e.id) => AnyEvent {
                param: clap_event_param_value {
                    header: header(at, CLAP_EVENT_PARAM_VALUE, std::mem::size_of::<clap_event_param_value>()),
                    param_id: e.id,
                    cookie: ptr::null_mut(),
                    note_id: -1,
                    port_index: -1,
                    channel: -1,
                    key: -1,
                    value: e.value,
                },
            },
            _ => return None,
        })
    }
}


/// Audio ports a plugin can use on each side.
const MAX_PORTS: usize = 16;
