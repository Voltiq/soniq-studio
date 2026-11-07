//! Two small VST3 plugins in one module, for testing the host end to end:
//!
//! - **Soniq Test Delay**, an effect: a gain, and a fixed delay of 256
//!   samples it reports as its latency; a saved state; and an editor that,
//!   when opened, raises the gain by 1 dB through the host (as a knob turned
//!   in it would).
//! - **Soniq Test VST Synth**, an instrument: sine voices that follow notes.
//!
//! Each is one object that is both its processor and its controller. Built
//! by `cargo test` as `test_vst3.dll`; copied to a `.vst3` name it loads in
//! any VST3 host.

#![allow(non_snake_case, clippy::missing_safety_doc)]

use std::ffi::{CStr, c_char, c_void};
use std::sync::Mutex;
use std::{ptr, slice};

use vst3::Steinberg::Vst::*;
use vst3::Steinberg::*;
use vst3::{Class, ComPtr, ComRef, ComWrapper, uid};

/// Samples the delay holds the sound back.
const LATENCY: usize = 256;

const DELAY_CID: TUID = uid(0x5D1A7E57, 0x0A1E4E0B, 0x9C3F5011, 0x7E570001);
const SYNTH_CID: TUID = uid(0x5D1A7E57, 0x0A1E4E0B, 0x9C3F5011, 0x7E570002);

fn copy_cstring(src: &str, dst: &mut [c_char]) {
    for (d, s) in dst.iter_mut().zip(src.bytes().chain(Some(0))) {
        *d = s as c_char;
    }
    if let Some(last) = dst.last_mut() {
        *last = 0;
    }
}

fn copy_wstring(src: &str, dst: &mut [TChar]) {
    let mut n = 0;
    for (d, s) in dst.iter_mut().zip(src.encode_utf16()) {
        *d = s as TChar;
        n += 1;
    }
    if n < dst.len() {
        dst[n] = 0;
    }
}

/// The gain or level parameter: 0–1 maps to −60…+12 dB (delay) or −60…0 dB (synth).
fn plain_db(synth: bool, normalized: f64) -> f64 {
    if synth { -60.0 + 60.0 * normalized } else { -60.0 + 72.0 * normalized }
}

fn normalized_db(synth: bool, db: f64) -> f64 {
    if synth { (db + 60.0) / 60.0 } else { (db + 60.0) / 72.0 }
}

struct Dsp {
    rate: f64,
    /// The delay line, per channel, and where it writes next.
    line: [Vec<f32>; 2],
    write: usize,
    /// Sine voices: key and phase (key 255: free).
    voices: [(u8, f64); 16],
}

struct TestPlugin {
    synth: bool,
    value: Mutex<f64>,
    dsp: Mutex<Dsp>,
    handler: Mutex<Option<ComPtr<IComponentHandler>>>,
}

impl TestPlugin {
    fn new(synth: bool) -> Self {
        Self {
            synth,
            value: Mutex::new(normalized_db(synth, if synth { -6.0 } else { 0.0 })),
            dsp: Mutex::new(Dsp { rate: 48_000.0, line: [vec![0.0; LATENCY], vec![0.0; LATENCY]], write: 0, voices: [(255, 0.0); 16] }),
            handler: Mutex::new(None),
        }
    }

    fn value(&self) -> f64 {
        *self.value.lock().unwrap()
    }

    fn set(&self, v: f64) {
        *self.value.lock().unwrap() = v.clamp(0.0, 1.0);
    }
}

impl Class for TestPlugin {
    type Interfaces = (IComponent, IAudioProcessor, IEditController);
}

impl IPluginBaseTrait for TestPlugin {
    unsafe fn initialize(&self, _context: *mut FUnknown) -> tresult {
        kResultOk
    }

    unsafe fn terminate(&self) -> tresult {
        kResultOk
    }
}

impl IComponentTrait for TestPlugin {
    unsafe fn getControllerClassId(&self, _class_id: *mut TUID) -> tresult {
        kNotImplemented
    }

    unsafe fn setIoMode(&self, _mode: IoMode) -> tresult {
        kResultOk
    }

    unsafe fn getBusCount(&self, media: MediaType, dir: BusDirection) -> i32 {
        match (media as MediaTypes, dir as BusDirections) {
            (MediaTypes_::kAudio, BusDirections_::kOutput) => 1,
            (MediaTypes_::kAudio, BusDirections_::kInput) => i32::from(!self.synth),
            (MediaTypes_::kEvent, BusDirections_::kInput) => i32::from(self.synth),
            _ => 0,
        }
    }

    unsafe fn getBusInfo(&self, media: MediaType, dir: BusDirection, index: i32, bus: *mut BusInfo) -> tresult {
        if index != 0 || unsafe { self.getBusCount(media, dir) } == 0 {
            return kInvalidArgument;
        }
        let bus = unsafe { &mut *bus };
        bus.mediaType = media;
        bus.direction = dir;
        bus.channelCount = if media == MediaTypes_::kAudio as MediaType { 2 } else { 1 };
        copy_wstring(if dir == BusDirections_::kInput as BusDirection { "Input" } else { "Output" }, &mut bus.name);
        bus.busType = BusTypes_::kMain as BusType;
        bus.flags = BusInfo_::BusFlags_::kDefaultActive as u32;
        kResultOk
    }

    unsafe fn getRoutingInfo(&self, _in: *mut RoutingInfo, _out: *mut RoutingInfo) -> tresult {
        kNotImplemented
    }

    unsafe fn activateBus(&self, _media: MediaType, _dir: BusDirection, _index: i32, _state: TBool) -> tresult {
        kResultOk
    }

    unsafe fn setActive(&self, _state: TBool) -> tresult {
        kResultOk
    }

    unsafe fn setState(&self, state: *mut IBStream) -> tresult {
        let Some(stream) = (unsafe { ComRef::from_raw(state) }) else { return kInvalidArgument };
        let mut bytes = [0u8; 8];
        let mut read = 0;
        unsafe { stream.read(bytes.as_mut_ptr().cast(), 8, &mut read) };
        if read != 8 {
            return kResultFalse;
        }
        self.set(f64::from_le_bytes(bytes));
        kResultOk
    }

    unsafe fn getState(&self, state: *mut IBStream) -> tresult {
        let Some(stream) = (unsafe { ComRef::from_raw(state) }) else { return kInvalidArgument };
        let mut bytes = self.value().to_le_bytes();
        let mut written = 0;
        unsafe { stream.write(bytes.as_mut_ptr().cast(), 8, &mut written) };
        if written == 8 { kResultOk } else { kResultFalse }
    }
}

impl IAudioProcessorTrait for TestPlugin {
    unsafe fn setBusArrangements(&self, inputs: *mut SpeakerArrangement, num_ins: i32, outputs: *mut SpeakerArrangement, num_outs: i32) -> tresult {
        let ins = if num_ins > 0 { unsafe { slice::from_raw_parts(inputs, num_ins as usize) } } else { &[] };
        let outs = unsafe { slice::from_raw_parts(outputs, num_outs as usize) };
        if ins.iter().chain(outs).all(|&a| a == SpeakerArr::kStereo) { kResultTrue } else { kResultFalse }
    }

    unsafe fn getBusArrangement(&self, _dir: BusDirection, index: i32, arr: *mut SpeakerArrangement) -> tresult {
        if index != 0 {
            return kInvalidArgument;
        }
        unsafe { *arr = SpeakerArr::kStereo };
        kResultOk
    }

    unsafe fn canProcessSampleSize(&self, size: i32) -> tresult {
        if size == SymbolicSampleSizes_::kSample32 { kResultOk } else { kNotImplemented }
    }

    unsafe fn getLatencySamples(&self) -> u32 {
        if self.synth { 0 } else { LATENCY as u32 }
    }

    unsafe fn setupProcessing(&self, setup: *mut ProcessSetup) -> tresult {
        self.dsp.lock().unwrap().rate = unsafe { (*setup).sampleRate };
        kResultOk
    }

    unsafe fn setProcessing(&self, state: TBool) -> tresult {
        if state != 0 {
            let mut dsp = self.dsp.lock().unwrap();
            dsp.line.iter_mut().for_each(|l| l.fill(0.0));
            dsp.voices = [(255, 0.0); 16];
        }
        kResultOk
    }

    unsafe fn process(&self, data: *mut ProcessData) -> tresult {
        let data = unsafe { &*data };
        if let Some(changes) = unsafe { ComRef::from_raw(data.inputParameterChanges) } {
            for i in 0..unsafe { changes.getParameterCount() } {
                if let Some(queue) = unsafe { ComRef::from_raw(changes.getParameterData(i)) } {
                    let (mut at, mut value) = (0, 0.0);
                    let last = unsafe { queue.getPointCount() } - 1;
                    if unsafe { queue.getParameterId() } == 0 && last >= 0 && unsafe { queue.getPoint(last, &mut at, &mut value) } == kResultOk {
                        self.set(value);
                    }
                }
            }
        }
        let n = data.numSamples as usize;
        if data.numOutputs < 1 || n == 0 {
            return kResultOk;
        }
        let gain = 10f64.powf(plain_db(self.synth, self.value()) / 20.0) as f32;
        let outputs = unsafe { slice::from_raw_parts(data.outputs, data.numOutputs as usize) };
        let out = unsafe { slice::from_raw_parts(outputs[0].__field0.channelBuffers32, 2) };
        let (ol, or) = unsafe { (slice::from_raw_parts_mut(out[0], n), slice::from_raw_parts_mut(out[1], n)) };
        let mut dsp = self.dsp.lock().unwrap();
        if self.synth {
            let mut notes: Vec<Event> = Vec::new();
            if let Some(events) = unsafe { ComRef::from_raw(data.inputEvents) } {
                for i in 0..unsafe { events.getEventCount() } {
                    let mut e: Event = unsafe { std::mem::zeroed() };
                    if unsafe { events.getEvent(i, &mut e) } == kResultOk {
                        notes.push(e);
                    }
                }
            }
            let mut next = 0;
            for i in 0..n {
                while let Some(e) = notes.get(next).filter(|e| e.sampleOffset as usize <= i) {
                    if e.r#type == Event_::EventTypes_::kNoteOnEvent as u16 {
                        let key = unsafe { e.__field0.noteOn.pitch } as u8;
                        if let Some(v) = dsp.voices.iter_mut().find(|v| v.0 == 255) {
                            *v = (key, 0.0);
                        }
                    } else if e.r#type == Event_::EventTypes_::kNoteOffEvent as u16 {
                        let key = unsafe { e.__field0.noteOff.pitch } as u8;
                        dsp.voices.iter_mut().filter(|v| v.0 == key).for_each(|v| v.0 = 255);
                    }
                    next += 1;
                }
                let rate = dsp.rate;
                let mut s = 0.0;
                for v in dsp.voices.iter_mut().filter(|v| v.0 != 255) {
                    s += (v.1 * std::f64::consts::TAU).sin() as f32;
                    v.1 = (v.1 + 440.0 * 2f64.powf((f64::from(v.0) - 69.0) / 12.0) / rate).fract();
                }
                (ol[i], or[i]) = (s * gain, s * gain);
            }
            return kResultOk;
        }
        let inputs = unsafe { slice::from_raw_parts(data.inputs, data.numInputs.max(0) as usize) };
        if inputs.is_empty() {
            return kResultOk;
        }
        let input = unsafe { slice::from_raw_parts(inputs[0].__field0.channelBuffers32, 2) };
        let (il, ir) = unsafe { (slice::from_raw_parts(input[0], n), slice::from_raw_parts(input[1], n)) };
        for i in 0..n {
            let w = dsp.write;
            let (dl, dr) = (dsp.line[0][w], dsp.line[1][w]);
            dsp.line[0][w] = il[i];
            dsp.line[1][w] = ir[i];
            dsp.write = (w + 1) % LATENCY;
            (ol[i], or[i]) = (dl * gain, dr * gain);
        }
        kResultOk
    }

    unsafe fn getTailSamples(&self) -> u32 {
        LATENCY as u32
    }
}

impl IEditControllerTrait for TestPlugin {
    unsafe fn setComponentState(&self, _state: *mut IBStream) -> tresult {
        kResultOk
    }

    unsafe fn setState(&self, _state: *mut IBStream) -> tresult {
        kResultOk
    }

    unsafe fn getState(&self, _state: *mut IBStream) -> tresult {
        kResultOk
    }

    unsafe fn getParameterCount(&self) -> i32 {
        1
    }

    unsafe fn getParameterInfo(&self, index: i32, info: *mut ParameterInfo) -> tresult {
        if index != 0 {
            return kInvalidArgument;
        }
        let info = unsafe { &mut *info };
        info.id = 0;
        copy_wstring(if self.synth { "Level" } else { "Gain" }, &mut info.title);
        copy_wstring(if self.synth { "Level" } else { "Gain" }, &mut info.shortTitle);
        copy_wstring("dB", &mut info.units);
        info.stepCount = 0;
        info.defaultNormalizedValue = normalized_db(self.synth, if self.synth { -6.0 } else { 0.0 });
        info.unitId = 0;
        info.flags = ParameterInfo_::ParameterFlags_::kCanAutomate;
        kResultOk
    }

    unsafe fn getParamStringByValue(&self, id: u32, value: f64, string: *mut String128) -> tresult {
        if id != 0 {
            return kInvalidArgument;
        }
        copy_wstring(&format!("{:.1}", plain_db(self.synth, value)), unsafe { &mut *string });
        kResultOk
    }

    unsafe fn getParamValueByString(&self, _id: u32, _string: *mut TChar, _value: *mut f64) -> tresult {
        kNotImplemented
    }

    unsafe fn normalizedParamToPlain(&self, _id: u32, value: f64) -> f64 {
        plain_db(self.synth, value)
    }

    unsafe fn plainParamToNormalized(&self, _id: u32, plain: f64) -> f64 {
        normalized_db(self.synth, plain)
    }

    unsafe fn getParamNormalized(&self, id: u32) -> f64 {
        if id == 0 { self.value() } else { 0.0 }
    }

    unsafe fn setParamNormalized(&self, id: u32, value: f64) -> tresult {
        if id != 0 {
            return kInvalidArgument;
        }
        self.set(value);
        kResultOk
    }

    unsafe fn setComponentHandler(&self, handler: *mut IComponentHandler) -> tresult {
        *self.handler.lock().unwrap() = unsafe { ComRef::from_raw(handler) }.map(|h| h.to_com_ptr());
        kResultOk
    }

    unsafe fn createView(&self, name: FIDString) -> *mut IPlugView {
        if self.synth || name.is_null() || unsafe { CStr::from_ptr(name) } != c"editor" {
            return ptr::null_mut();
        }
        let editor = Editor { handler: self.handler.lock().unwrap().clone(), value: self.value(), window: Mutex::new(ptr::null_mut()) };
        ComWrapper::new(editor).to_com_ptr::<IPlugView>().map_or(ptr::null_mut(), |v| v.into_raw())
    }
}

/// The delay's editor: a label, and a nudge of the gain when it opens.
struct Editor {
    handler: Option<ComPtr<IComponentHandler>>,
    value: f64,
    window: Mutex<*mut c_void>,
}

unsafe impl Send for Editor {}
unsafe impl Sync for Editor {}

impl Class for Editor {
    type Interfaces = (IPlugView,);
}

impl IPlugViewTrait for Editor {
    unsafe fn isPlatformTypeSupported(&self, kind: FIDString) -> tresult {
        if !kind.is_null() && unsafe { CStr::from_ptr(kind) } == c"HWND" { kResultTrue } else { kResultFalse }
    }

    unsafe fn attached(&self, parent: *mut c_void, kind: FIDString) -> tresult {
        if unsafe { self.isPlatformTypeSupported(kind) } != kResultTrue {
            return kResultFalse;
        }
        #[cfg(windows)]
        {
            use windows_sys::Win32::UI::WindowsAndMessaging::{CreateWindowExW, WS_CHILD, WS_VISIBLE};
            let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
            let text: Vec<u16> = "Soniq Test Delay\0".encode_utf16().collect();
            let hwnd = unsafe { CreateWindowExW(0, class.as_ptr(), text.as_ptr(), WS_CHILD | WS_VISIBLE, 0, 0, 320, 120, parent, ptr::null_mut(), ptr::null_mut(), ptr::null()) };
            *self.window.lock().unwrap() = hwnd;
        }
        // As if a knob in here were turned up by 1 dB.
        if let Some(h) = &self.handler {
            let raised = normalized_db(false, plain_db(false, self.value) + 1.0).min(1.0);
            unsafe {
                h.beginEdit(0);
                h.performEdit(0, raised);
                h.endEdit(0);
            }
        }
        kResultOk
    }

    unsafe fn removed(&self) -> tresult {
        #[cfg(windows)]
        {
            let hwnd = std::mem::replace(&mut *self.window.lock().unwrap(), ptr::null_mut());
            if !hwnd.is_null() {
                unsafe { windows_sys::Win32::UI::WindowsAndMessaging::DestroyWindow(hwnd) };
            }
        }
        kResultOk
    }

    unsafe fn onWheel(&self, _distance: f32) -> tresult {
        kResultFalse
    }

    unsafe fn onKeyDown(&self, _key: char16, _code: i16, _modifiers: i16) -> tresult {
        kResultFalse
    }

    unsafe fn onKeyUp(&self, _key: char16, _code: i16, _modifiers: i16) -> tresult {
        kResultFalse
    }

    unsafe fn getSize(&self, size: *mut ViewRect) -> tresult {
        unsafe { *size = ViewRect { left: 0, top: 0, right: 320, bottom: 120 } };
        kResultOk
    }

    unsafe fn onSize(&self, _size: *mut ViewRect) -> tresult {
        kResultOk
    }

    unsafe fn onFocus(&self, _state: TBool) -> tresult {
        kResultOk
    }

    unsafe fn setFrame(&self, _frame: *mut IPlugFrame) -> tresult {
        kResultOk
    }

    unsafe fn canResize(&self) -> tresult {
        kResultFalse
    }

    unsafe fn checkSizeConstraint(&self, _rect: *mut ViewRect) -> tresult {
        kResultOk
    }
}

struct Factory;

impl Class for Factory {
    type Interfaces = (IPluginFactory2,);
}

impl IPluginFactoryTrait for Factory {
    unsafe fn getFactoryInfo(&self, info: *mut PFactoryInfo) -> tresult {
        let info = unsafe { &mut *info };
        copy_cstring("Soniq", &mut info.vendor);
        copy_cstring("", &mut info.url);
        copy_cstring("", &mut info.email);
        info.flags = PFactoryInfo_::FactoryFlags_::kUnicode as int32;
        kResultOk
    }

    unsafe fn countClasses(&self) -> i32 {
        2
    }

    unsafe fn getClassInfo(&self, index: i32, info: *mut PClassInfo) -> tresult {
        let (cid, name) = match index {
            0 => (DELAY_CID, "Soniq Test Delay"),
            1 => (SYNTH_CID, "Soniq Test VST Synth"),
            _ => return kInvalidArgument,
        };
        let info = unsafe { &mut *info };
        info.cid = cid;
        info.cardinality = PClassInfo_::ClassCardinality_::kManyInstances as int32;
        copy_cstring("Audio Module Class", &mut info.category);
        copy_cstring(name, &mut info.name);
        kResultOk
    }

    unsafe fn createInstance(&self, cid: FIDString, iid: FIDString, obj: *mut *mut c_void) -> tresult {
        let synth = match unsafe { *(cid as *const TUID) } {
            DELAY_CID => false,
            SYNTH_CID => true,
            _ => return kInvalidArgument,
        };
        let Some(instance) = ComWrapper::new(TestPlugin::new(synth)).to_com_ptr::<FUnknown>() else { return kInternalError };
        let p = instance.as_ptr();
        unsafe { ((*(*p).vtbl).queryInterface)(p, iid as *const TUID, obj) }
    }
}

impl IPluginFactory2Trait for Factory {
    unsafe fn getClassInfo2(&self, index: i32, info: *mut PClassInfo2) -> tresult {
        let (cid, name, sub) = match index {
            0 => (DELAY_CID, "Soniq Test Delay", "Fx|Delay"),
            1 => (SYNTH_CID, "Soniq Test VST Synth", "Instrument|Synth"),
            _ => return kInvalidArgument,
        };
        let info = unsafe { &mut *info };
        info.cid = cid;
        info.cardinality = PClassInfo_::ClassCardinality_::kManyInstances as int32;
        copy_cstring("Audio Module Class", &mut info.category);
        copy_cstring(name, &mut info.name);
        info.classFlags = 0;
        copy_cstring(sub, &mut info.subCategories);
        copy_cstring("Soniq", &mut info.vendor);
        copy_cstring("1.0.0", &mut info.version);
        copy_cstring("VST 3.7", &mut info.sdkVersion);
        kResultOk
    }
}

#[cfg(windows)]
#[unsafe(no_mangle)]
extern "system" fn InitDll() -> bool {
    true
}

#[cfg(windows)]
#[unsafe(no_mangle)]
extern "system" fn ExitDll() -> bool {
    true
}

#[unsafe(no_mangle)]
extern "system" fn GetPluginFactory() -> *mut IPluginFactory {
    ComWrapper::new(Factory).to_com_ptr::<IPluginFactory>().map_or(ptr::null_mut(), |f| f.into_raw())
}
