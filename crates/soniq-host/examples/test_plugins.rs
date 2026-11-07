//! Two small CLAP plugins in one file, for testing the host end to end:
//!
//! - **Soniq Test Gain**, an effect: a gain knob, a small window, a saved
//!   state, and switches that crash it, hang it, or make it move its own
//!   gain (as a knob in its window would).
//! - **Soniq Test Synth**, an instrument: sine voices that follow notes.
//!
//! Built by `cargo test` as `test_plugins.dll`; copied to a `.clap` name it
//! loads in any CLAP host.

#![allow(clippy::missing_safety_doc)]

use std::ffi::{CStr, c_char, c_void};
use std::ptr;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

use clap_sys::entry::clap_plugin_entry;
use clap_sys::events::*;
use clap_sys::ext::audio_ports::*;
use clap_sys::ext::gui::*;
use clap_sys::ext::note_ports::*;
use clap_sys::ext::params::*;
use clap_sys::ext::state::*;
use clap_sys::factory::plugin_factory::*;
use clap_sys::host::clap_host;
use clap_sys::id::{CLAP_INVALID_ID, clap_id};
use clap_sys::plugin::{clap_plugin, clap_plugin_descriptor};
use clap_sys::process::{CLAP_PROCESS_CONTINUE, clap_process, clap_process_status};
use clap_sys::stream::{clap_istream, clap_ostream};
use clap_sys::version::CLAP_VERSION;

struct Features([*const c_char; 3]);
unsafe impl Sync for Features {}

static GAIN_FEATURES: Features = Features([c"audio-effect".as_ptr(), c"utility".as_ptr(), ptr::null()]);
static SYNTH_FEATURES: Features = Features([c"instrument".as_ptr(), c"synthesizer".as_ptr(), ptr::null()]);

const fn descriptor(id: &'static CStr, name: &'static CStr, features: &'static Features) -> clap_plugin_descriptor {
    clap_plugin_descriptor {
        clap_version: CLAP_VERSION,
        id: id.as_ptr(),
        name: name.as_ptr(),
        vendor: c"Soniq".as_ptr(),
        url: c"".as_ptr(),
        manual_url: c"".as_ptr(),
        support_url: c"".as_ptr(),
        version: c"1.0.0".as_ptr(),
        description: c"A plugin for testing Soniq Studio".as_ptr(),
        features: features.0.as_ptr(),
    }
}

static GAIN: clap_plugin_descriptor = descriptor(c"studio.soniq.test.gain", c"Soniq Test Gain", &GAIN_FEATURES);
static SYNTH: clap_plugin_descriptor = descriptor(c"studio.soniq.test.synth", c"Soniq Test Synth", &SYNTH_FEATURES);

#[allow(non_upper_case_globals)]
#[unsafe(no_mangle)]
pub static clap_entry: clap_plugin_entry = clap_plugin_entry { clap_version: CLAP_VERSION, init: Some(entry_init), deinit: Some(entry_deinit), get_factory: Some(get_factory) };

unsafe extern "C" fn entry_init(_: *const c_char) -> bool {
    true
}

unsafe extern "C" fn entry_deinit() {}

static FACTORY: clap_plugin_factory = clap_plugin_factory { get_plugin_count: Some(plugin_count), get_plugin_descriptor: Some(plugin_descriptor), create_plugin: Some(create_plugin) };

unsafe extern "C" fn get_factory(id: *const c_char) -> *const c_void {
    if unsafe { CStr::from_ptr(id) } == CLAP_PLUGIN_FACTORY_ID { &FACTORY as *const _ as *const c_void } else { ptr::null() }
}

unsafe extern "C" fn plugin_count(_: *const clap_plugin_factory) -> u32 {
    2
}

unsafe extern "C" fn plugin_descriptor(_: *const clap_plugin_factory, index: u32) -> *const clap_plugin_descriptor {
    match index {
        0 => &GAIN,
        1 => &SYNTH,
        _ => ptr::null(),
    }
}

/// Parameter ids.
const GAIN_DB: clap_id = 0;
const CRASH: clap_id = 1;
const HANG: clap_id = 2;
const NUDGE: clap_id = 3;
const LEVEL_DB: clap_id = 10;

struct Param {
    id: clap_id,
    name: &'static str,
    min: f64,
    max: f64,
    default: f64,
    stepped: bool,
}

const GAIN_PARAMS: [Param; 4] = [
    Param { id: GAIN_DB, name: "Gain", min: -60.0, max: 12.0, default: 0.0, stepped: false },
    Param { id: CRASH, name: "Crash", min: 0.0, max: 1.0, default: 0.0, stepped: true },
    Param { id: HANG, name: "Hang", min: 0.0, max: 1.0, default: 0.0, stepped: true },
    Param { id: NUDGE, name: "Nudge", min: 0.0, max: 1.0, default: 0.0, stepped: true },
];
const SYNTH_PARAMS: [Param; 1] = [Param { id: LEVEL_DB, name: "Level", min: -60.0, max: 0.0, default: -6.0, stepped: false }];

struct Instance {
    plugin: clap_plugin,
    synth: bool,
    values: [AtomicU64; 11],
    rate: f64,
    /// Sine voices: key and phase (key 255: free).
    voices: [(u8, f64); 16],
    window: *mut c_void,
}

impl Instance {
    fn params(&self) -> &'static [Param] {
        if self.synth { &SYNTH_PARAMS } else { &GAIN_PARAMS }
    }

    fn get(&self, id: clap_id) -> f64 {
        f64::from_bits(self.values[id as usize].load(Relaxed))
    }

    fn set(&self, id: clap_id, value: f64) {
        if let Some(p) = self.params().iter().find(|p| p.id == id) {
            self.values[id as usize].store(value.clamp(p.min, p.max).to_bits(), Relaxed);
        }
    }

    fn take(&mut self, event: *const clap_event_header) {
        let header = unsafe { &*event };
        if header.space_id != CLAP_CORE_EVENT_SPACE_ID {
            return;
        }
        match header.type_ {
            CLAP_EVENT_PARAM_VALUE => {
                let e = unsafe { &*(event as *const clap_event_param_value) };
                self.set(e.param_id, e.value);
            }
            CLAP_EVENT_NOTE_ON => self.note_on(unsafe { &*(event as *const clap_event_note) }.key),
            CLAP_EVENT_NOTE_OFF | CLAP_EVENT_NOTE_CHOKE => self.note_off(unsafe { &*(event as *const clap_event_note) }.key),
            CLAP_EVENT_MIDI => {
                let d = unsafe { &*(event as *const clap_event_midi) }.data;
                match d[0] & 0xf0 {
                    0x90 if d[2] > 0 => self.note_on(i16::from(d[1])),
                    0x80 | 0x90 => self.note_off(i16::from(d[1])),
                    0xb0 if d[1] == 123 || d[1] == 120 => self.note_off(-1),
                    _ => {}
                }
            }
            _ => {}
        }
    }

    fn note_on(&mut self, key: i16) {
        if let Some(v) = self.voices.iter_mut().find(|v| v.0 == 255) {
            *v = (key.clamp(0, 127) as u8, 0.0);
        }
    }

    fn note_off(&mut self, key: i16) {
        for v in &mut self.voices {
            if key < 0 || i16::from(v.0) == key {
                v.0 = 255;
            }
        }
    }
}

unsafe fn instance<'a>(plugin: *const clap_plugin) -> &'a mut Instance {
    unsafe { &mut *((*plugin).plugin_data as *mut Instance) }
}

unsafe extern "C" fn create_plugin(_: *const clap_plugin_factory, _: *const clap_host, id: *const c_char) -> *const clap_plugin {
    let id = unsafe { CStr::from_ptr(id) };
    let synth = match id.to_bytes() {
        b"studio.soniq.test.gain" => false,
        b"studio.soniq.test.synth" => true,
        _ => return ptr::null(),
    };
    let instance = Box::new(Instance {
        plugin: clap_plugin {
            desc: if synth { &SYNTH } else { &GAIN },
            plugin_data: ptr::null_mut(),
            init: Some(init),
            destroy: Some(destroy),
            activate: Some(activate),
            deactivate: Some(deactivate),
            start_processing: Some(start_processing),
            stop_processing: Some(stop_processing),
            reset: Some(reset),
            process: Some(process),
            get_extension: Some(get_extension),
            on_main_thread: Some(on_main_thread),
        },
        synth,
        values: Default::default(),
        rate: 48_000.0,
        voices: [(255, 0.0); 16],
        window: ptr::null_mut(),
    });
    for p in instance.params() {
        instance.set(p.id, p.default);
    }
    let instance = Box::into_raw(instance);
    unsafe {
        (*instance).plugin.plugin_data = instance.cast();
        &(*instance).plugin
    }
}

unsafe extern "C" fn init(_: *const clap_plugin) -> bool {
    true
}

unsafe extern "C" fn destroy(plugin: *const clap_plugin) {
    drop(unsafe { Box::from_raw((*plugin).plugin_data as *mut Instance) });
}

unsafe extern "C" fn activate(plugin: *const clap_plugin, rate: f64, _: u32, _: u32) -> bool {
    unsafe { instance(plugin) }.rate = rate;
    true
}

unsafe extern "C" fn deactivate(_: *const clap_plugin) {}

unsafe extern "C" fn start_processing(_: *const clap_plugin) -> bool {
    true
}

unsafe extern "C" fn stop_processing(_: *const clap_plugin) {}

unsafe extern "C" fn reset(plugin: *const clap_plugin) {
    unsafe { instance(plugin) }.voices = [(255, 0.0); 16];
}

unsafe extern "C" fn on_main_thread(_: *const clap_plugin) {}

fn db(v: f64) -> f32 {
    10f64.powf(v / 20.0) as f32
}

unsafe extern "C" fn process(plugin: *const clap_plugin, process: *const clap_process) -> clap_process_status {
    let me = unsafe { instance(plugin) };
    let p = unsafe { &*process };
    let n = p.frames_count as usize;
    let events = unsafe { &*p.in_events };
    let count = events.size.map_or(0, |f| unsafe { f(events) });
    let nudged = me.get(NUDGE);
    let out = unsafe { &*p.audio_outputs };
    let (ol, or) = unsafe { (std::slice::from_raw_parts_mut(*out.data32, n), std::slice::from_raw_parts_mut(*out.data32.add(1), n)) };
    if me.synth {
        // Sample-accurate notes: render up to each event, then take it.
        let mut cursor = 0;
        let level = db(me.get(LEVEL_DB));
        let render = |me: &mut Instance, from: usize, to: usize, ol: &mut [f32], or: &mut [f32]| {
            for i in from..to {
                let mut s = 0.0;
                for v in me.voices.iter_mut().filter(|v| v.0 != 255) {
                    let hz = 440.0 * 2f64.powf((f64::from(v.0) - 69.0) / 12.0);
                    s += (v.1 * std::f64::consts::TAU).sin() as f32;
                    v.1 = (v.1 + hz / me.rate).fract();
                }
                (ol[i], or[i]) = (s * level, s * level);
            }
        };
        for i in 0..count {
            let e = unsafe { events.get.unwrap()(events, i) };
            let at = (unsafe { (*e).time } as usize).min(n);
            render(me, cursor, at, ol, or);
            cursor = at;
            me.take(e);
        }
        render(me, cursor, n, ol, or);
        return CLAP_PROCESS_CONTINUE;
    }
    for i in 0..count {
        me.take(unsafe { events.get.unwrap()(events, i) });
    }
    if me.get(CRASH) >= 0.5 {
        std::process::abort();
    }
    if me.get(HANG) >= 0.5 {
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
    // Nudge switched on: the plugin raises its own gain by 1 dB and says so,
    // as a knob turned in its window would.
    if nudged < 0.5 && me.get(NUDGE) >= 0.5 {
        let gain = (me.get(GAIN_DB) + 1.0).min(12.0);
        me.set(GAIN_DB, gain);
        let out = unsafe { &*p.out_events };
        let push = out.try_push.unwrap();
        let gesture = |type_| clap_event_param_gesture {
            header: clap_event_header { size: std::mem::size_of::<clap_event_param_gesture>() as u32, time: 0, space_id: CLAP_CORE_EVENT_SPACE_ID, type_, flags: 0 },
            param_id: GAIN_DB,
        };
        let value = clap_event_param_value {
            header: clap_event_header { size: std::mem::size_of::<clap_event_param_value>() as u32, time: 0, space_id: CLAP_CORE_EVENT_SPACE_ID, type_: CLAP_EVENT_PARAM_VALUE, flags: 0 },
            param_id: GAIN_DB,
            cookie: ptr::null_mut(),
            note_id: -1,
            port_index: -1,
            channel: -1,
            key: -1,
            value: gain,
        };
        unsafe {
            push(out, &gesture(CLAP_EVENT_PARAM_GESTURE_BEGIN).header);
            push(out, &value.header);
            push(out, &gesture(CLAP_EVENT_PARAM_GESTURE_END).header);
        }
    }
    let input = unsafe { &*p.audio_inputs };
    let (il, ir) = unsafe { (std::slice::from_raw_parts(*input.data32, n), std::slice::from_raw_parts(*input.data32.add(1), n)) };
    let g = db(me.get(GAIN_DB));
    for i in 0..n {
        (ol[i], or[i]) = (il[i] * g, ir[i] * g);
    }
    CLAP_PROCESS_CONTINUE
}

static PARAMS: clap_plugin_params = clap_plugin_params {
    count: Some(params_count),
    get_info: Some(params_info),
    get_value: Some(params_value),
    value_to_text: Some(params_text),
    text_to_value: Some(params_parse),
    flush: Some(params_flush),
};

unsafe extern "C" fn params_count(plugin: *const clap_plugin) -> u32 {
    unsafe { instance(plugin) }.params().len() as u32
}

unsafe extern "C" fn params_info(plugin: *const clap_plugin, index: u32, info: *mut clap_param_info) -> bool {
    let Some(p) = unsafe { instance(plugin) }.params().get(index as usize) else { return false };
    let info = unsafe { &mut *info };
    info.id = p.id;
    info.flags = CLAP_PARAM_IS_AUTOMATABLE | if p.stepped { CLAP_PARAM_IS_STEPPED } else { 0 };
    info.cookie = ptr::null_mut();
    info.name = [0; 256];
    for (d, s) in info.name.iter_mut().zip(p.name.bytes()) {
        *d = s as c_char;
    }
    info.module = [0; 1024];
    (info.min_value, info.max_value, info.default_value) = (p.min, p.max, p.default);
    true
}

unsafe extern "C" fn params_value(plugin: *const clap_plugin, id: clap_id, value: *mut f64) -> bool {
    let me = unsafe { instance(plugin) };
    if !me.params().iter().any(|p| p.id == id) {
        return false;
    }
    unsafe { *value = me.get(id) };
    true
}

unsafe extern "C" fn params_text(_: *const clap_plugin, id: clap_id, value: f64, out: *mut c_char, size: u32) -> bool {
    let text = match id {
        GAIN_DB | LEVEL_DB => format!("{value:.1} dB"),
        _ => (if value >= 0.5 { "On" } else { "Off" }).to_string(),
    };
    let n = text.len().min(size as usize - 1);
    unsafe {
        ptr::copy_nonoverlapping(text.as_ptr().cast(), out, n);
        *out.add(n) = 0;
    }
    true
}

unsafe extern "C" fn params_parse(_: *const clap_plugin, _: clap_id, text: *const c_char, value: *mut f64) -> bool {
    let text = unsafe { CStr::from_ptr(text) }.to_string_lossy();
    match text.trim_end_matches(" dB").trim().parse() {
        Ok(v) => {
            unsafe { *value = v };
            true
        }
        Err(_) => false,
    }
}

unsafe extern "C" fn params_flush(plugin: *const clap_plugin, input: *const clap_input_events, _: *const clap_output_events) {
    let me = unsafe { instance(plugin) };
    let events = unsafe { &*input };
    for i in 0..events.size.map_or(0, |f| unsafe { f(events) }) {
        me.take(unsafe { events.get.unwrap()(events, i) });
    }
}

static STATE: clap_plugin_state = clap_plugin_state { save: Some(state_save), load: Some(state_load) };

unsafe extern "C" fn state_save(plugin: *const clap_plugin, stream: *const clap_ostream) -> bool {
    let me = unsafe { instance(plugin) };
    let id = if me.synth { LEVEL_DB } else { GAIN_DB };
    let bytes = me.get(id).to_le_bytes();
    let write = unsafe { (*stream).write.unwrap() };
    unsafe { write(stream, bytes.as_ptr().cast(), 8) == 8 }
}

unsafe extern "C" fn state_load(plugin: *const clap_plugin, stream: *const clap_istream) -> bool {
    let me = unsafe { instance(plugin) };
    let mut bytes = [0u8; 8];
    let read = unsafe { (*stream).read.unwrap() };
    if unsafe { read(stream, bytes.as_mut_ptr().cast(), 8) } != 8 {
        return false;
    }
    me.set(if me.synth { LEVEL_DB } else { GAIN_DB }, f64::from_le_bytes(bytes));
    true
}

static AUDIO_PORTS: clap_plugin_audio_ports = clap_plugin_audio_ports { count: Some(ports_count), get: Some(ports_get) };

unsafe extern "C" fn ports_count(plugin: *const clap_plugin, input: bool) -> u32 {
    if input && unsafe { instance(plugin) }.synth { 0 } else { 1 }
}

unsafe extern "C" fn ports_get(plugin: *const clap_plugin, index: u32, input: bool, info: *mut clap_audio_port_info) -> bool {
    if index != 0 || (input && unsafe { instance(plugin) }.synth) {
        return false;
    }
    let info = unsafe { &mut *info };
    info.id = 0;
    info.name = [0; 256];
    info.flags = CLAP_AUDIO_PORT_IS_MAIN;
    info.channel_count = 2;
    info.port_type = CLAP_PORT_STEREO.as_ptr();
    info.in_place_pair = CLAP_INVALID_ID;
    true
}

static NOTE_PORTS: clap_plugin_note_ports = clap_plugin_note_ports { count: Some(notes_count), get: Some(notes_get) };

unsafe extern "C" fn notes_count(_: *const clap_plugin, input: bool) -> u32 {
    u32::from(input)
}

unsafe extern "C" fn notes_get(_: *const clap_plugin, index: u32, input: bool, info: *mut clap_note_port_info) -> bool {
    if index != 0 || !input {
        return false;
    }
    let info = unsafe { &mut *info };
    info.id = 0;
    info.supported_dialects = CLAP_NOTE_DIALECT_CLAP | CLAP_NOTE_DIALECT_MIDI;
    info.preferred_dialect = CLAP_NOTE_DIALECT_CLAP;
    info.name = [0; 256];
    true
}

#[cfg(windows)]
mod gui {
    use super::*;
    use windows_sys::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, SW_HIDE, SW_SHOW, ShowWindow, WS_CHILD, WS_VISIBLE};

    pub static GUI: clap_plugin_gui = clap_plugin_gui {
        is_api_supported: Some(supported),
        get_preferred_api: None,
        create: Some(create),
        destroy: Some(destroy),
        set_scale: Some(set_scale),
        get_size: Some(get_size),
        can_resize: Some(can_resize),
        get_resize_hints: None,
        adjust_size: None,
        set_size: None,
        set_parent: Some(set_parent),
        set_transient: None,
        suggest_title: None,
        show: Some(show),
        hide: Some(hide),
    };

    unsafe extern "C" fn supported(_: *const clap_plugin, api: *const c_char, floating: bool) -> bool {
        !floating && unsafe { CStr::from_ptr(api) } == CLAP_WINDOW_API_WIN32
    }

    unsafe extern "C" fn create(_: *const clap_plugin, _: *const c_char, floating: bool) -> bool {
        !floating
    }

    unsafe extern "C" fn destroy(plugin: *const clap_plugin) {
        let me = unsafe { instance(plugin) };
        if !me.window.is_null() {
            unsafe { DestroyWindow(me.window) };
            me.window = ptr::null_mut();
        }
    }

    unsafe extern "C" fn set_scale(_: *const clap_plugin, _: f64) -> bool {
        true
    }

    unsafe extern "C" fn get_size(_: *const clap_plugin, w: *mut u32, h: *mut u32) -> bool {
        unsafe { (*w, *h) = (320, 120) };
        true
    }

    unsafe extern "C" fn can_resize(_: *const clap_plugin) -> bool {
        false
    }

    unsafe extern "C" fn set_parent(plugin: *const clap_plugin, window: *const clap_window) -> bool {
        let me = unsafe { instance(plugin) };
        let parent = unsafe { (*window).specific.win32 };
        let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
        let text: Vec<u16> = "Soniq Test Gain\0".encode_utf16().collect();
        me.window = unsafe { CreateWindowExW(0, class.as_ptr(), text.as_ptr(), WS_CHILD | WS_VISIBLE, 0, 0, 320, 120, parent, ptr::null_mut(), ptr::null_mut(), ptr::null()) };
        !me.window.is_null()
    }

    unsafe extern "C" fn show(plugin: *const clap_plugin) -> bool {
        unsafe { ShowWindow(instance(plugin).window, SW_SHOW) };
        true
    }

    unsafe extern "C" fn hide(plugin: *const clap_plugin) -> bool {
        unsafe { ShowWindow(instance(plugin).window, SW_HIDE) };
        true
    }
}

unsafe extern "C" fn get_extension(plugin: *const clap_plugin, id: *const c_char) -> *const c_void {
    let id = unsafe { CStr::from_ptr(id) };
    let synth = unsafe { instance(plugin) }.synth;
    if id == CLAP_EXT_PARAMS {
        &PARAMS as *const _ as *const c_void
    } else if id == CLAP_EXT_STATE {
        &STATE as *const _ as *const c_void
    } else if id == CLAP_EXT_AUDIO_PORTS {
        &AUDIO_PORTS as *const _ as *const c_void
    } else if id == CLAP_EXT_NOTE_PORTS && synth {
        &NOTE_PORTS as *const _ as *const c_void
    } else if id == CLAP_EXT_GUI && !synth && cfg!(windows) {
        #[cfg(windows)]
        return &gui::GUI as *const _ as *const c_void;
        #[cfg(not(windows))]
        ptr::null()
    } else {
        ptr::null()
    }
}
