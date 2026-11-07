//! VST3 from the host's side, inside the sandbox: opening a `.vst3` module,
//! listing its plugins, and driving one. [`Plugin`] holds the calls VST3
//! wants on the main thread (parameters, state, the editor); [`Processor`]
//! the ones it wants on the audio thread. The host's own COM objects (the
//! application, the component handler, streams, parameter queues, event
//! lists, messages) are here too.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::{CStr, CString, c_char, c_void};
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::{Arc, Mutex};

use vst3::Steinberg::Vst::*;
use vst3::Steinberg::*;
use vst3::{Class, ComPtr, ComWrapper, Interface};

use crate::host::{HostState, OutEvent};
use crate::protocol::{Descriptor, Gesture, ParamInfo};
use crate::shm::{Block, MAX_EVENTS, MAX_FRAMES, RESET, kind};

/// Parameter changes a block can carry each way.
const QUEUES: usize = 256;
const POINTS: usize = 16;

fn ok(result: tresult) -> bool {
    result == kResultOk
}

fn wide_text(s: &[TChar]) -> String {
    let len = s.iter().position(|&c| c == 0).unwrap_or(s.len());
    String::from_utf16_lossy(&s[..len])
}

fn char_text(s: &[c_char]) -> String {
    let bytes: Vec<u8> = s.iter().take_while(|&&c| c != 0).map(|&c| c as u8).collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

fn copy_wide(src: &str, dst: &mut [TChar]) {
    let mut n = 0;
    for (d, s) in dst.iter_mut().zip(src.encode_utf16()) {
        *d = s as TChar;
        n += 1;
    }
    if let Some(end) = dst.get_mut(n.min(dst.len().saturating_sub(1))) {
        *end = 0;
    }
}

/// A class id as the 32 hex digits the app keeps.
pub fn id_text(cid: &TUID) -> String {
    cid.iter().map(|b| format!("{:02X}", *b as u8)).collect()
}

fn parse_id(id: &str) -> Option<TUID> {
    if id.len() != 32 {
        return None;
    }
    let mut cid: TUID = [0; 16];
    for (i, b) in cid.iter_mut().enumerate() {
        *b = u8::from_str_radix(&id[i * 2..i * 2 + 2], 16).ok()? as i8;
    }
    Some(cid)
}

fn same_iid(tuid: *const TUID, iid: &[u8; 16]) -> bool {
    !tuid.is_null() && unsafe { std::slice::from_raw_parts(tuid as *const u8, 16) } == iid
}

/// The binary inside a `.vst3` bundle, or the file itself (older plugins).
fn binary(path: &Path) -> PathBuf {
    if !path.is_dir() {
        return path.to_path_buf();
    }
    let arch = if cfg!(target_arch = "aarch64") {
        "arm64-win"
    } else if cfg!(target_arch = "x86") {
        "x86-win"
    } else {
        "x86_64-win"
    };
    path.join("Contents").join(arch).join(path.file_name().unwrap_or_default())
}

/// An open `.vst3` module. Its plugins must be gone before it closes.
pub struct Module {
    factory: Option<ComPtr<IPluginFactory>>,
    exit: Option<unsafe extern "system" fn() -> bool>,
    _lib: libloading::Library,
}

unsafe impl Send for Module {}
unsafe impl Sync for Module {}

impl Module {
    pub fn open(path: &Path) -> Result<Self, String> {
        let file = binary(path);
        let lib = unsafe { libloading::Library::new(&file) }.map_err(|e| format!("Could not load {}: {e}", file.display()))?;
        if let Ok(init) = unsafe { lib.get::<unsafe extern "system" fn() -> bool>(b"InitDll\0") }
            && !unsafe { init() }
        {
            return Err("The plugin refused to start.".into());
        }
        let exit = unsafe { lib.get::<unsafe extern "system" fn() -> bool>(b"ExitDll\0") }.ok().map(|f| *f);
        let get = unsafe { lib.get::<unsafe extern "system" fn() -> *mut IPluginFactory>(b"GetPluginFactory\0") }.map_err(|_| "Not a VST3 plugin: it has no GetPluginFactory.".to_string())?;
        let factory = unsafe { ComPtr::from_raw(get()) }.ok_or("The plugin has no factory.")?;
        Ok(Self { factory: Some(factory), exit, _lib: lib })
    }

    fn factory(&self) -> &ComPtr<IPluginFactory> {
        self.factory.as_ref().expect("an open module has a factory")
    }

    /// The plugins in the module (its audio module classes).
    pub fn descriptors(&self) -> Vec<Descriptor> {
        let factory = self.factory();
        let mut info: PFactoryInfo = unsafe { std::mem::zeroed() };
        let vendor = if ok(unsafe { factory.getFactoryInfo(&mut info) }) { char_text(&info.vendor) } else { String::new() };
        let factory2 = factory.cast::<IPluginFactory2>();
        (0..unsafe { factory.countClasses() })
            .filter_map(|i| {
                if let Some(f2) = &factory2 {
                    let mut c: PClassInfo2 = unsafe { std::mem::zeroed() };
                    if ok(unsafe { f2.getClassInfo2(i, &mut c) }) {
                        let sub = char_text(&c.subCategories);
                        return Some((c.cid, char_text(&c.category), char_text(&c.name), char_text(&c.vendor), char_text(&c.version), sub));
                    }
                }
                let mut c: PClassInfo = unsafe { std::mem::zeroed() };
                ok(unsafe { factory.getClassInfo(i, &mut c) }).then(|| (c.cid, char_text(&c.category), char_text(&c.name), String::new(), String::new(), String::new()))
            })
            .filter(|(_, category, ..)| category == "Audio Module Class")
            .map(|(cid, _, name, class_vendor, version, sub)| {
                let mut features: Vec<String> = sub.split('|').filter(|s| !s.is_empty()).map(str::to_lowercase).collect();
                let instrument = features.iter().any(|f| f == "instrument");
                features.insert(0, if instrument { "instrument".into() } else { "audio-effect".into() });
                Descriptor { id: id_text(&cid), name, vendor: if class_vendor.is_empty() { vendor.clone() } else { class_vendor }, version, description: sub, features }
            })
            .collect()
    }
}

impl Drop for Module {
    fn drop(&mut self) {
        self.factory = None;
        if let Some(exit) = self.exit {
            unsafe { exit() };
        }
    }
}

// ---------- The host's COM objects ----------

struct HostApp;

impl Class for HostApp {
    type Interfaces = (IHostApplication,);
}

impl IHostApplicationTrait for HostApp {
    unsafe fn getName(&self, name: *mut String128) -> tresult {
        copy_wide("Soniq Studio", unsafe { &mut *name });
        kResultOk
    }

    unsafe fn createInstance(&self, cid: *mut TUID, iid: *mut TUID, obj: *mut *mut c_void) -> tresult {
        // Plugins ask the host for messages (and their attributes) to talk between their halves.
        let object = if same_iid(cid, &IMessage::IID) {
            ComWrapper::new(HostMessage::new()).to_com_ptr::<FUnknown>()
        } else if same_iid(cid, &IAttributeList::IID) {
            ComWrapper::new(Attributes::default()).to_com_ptr::<FUnknown>()
        } else {
            None
        };
        match object {
            Some(o) => unsafe { ((*(*o.as_ptr()).vtbl).queryInterface)(o.as_ptr(), iid as *const TUID, obj) },
            None => {
                unsafe { *obj = ptr::null_mut() };
                kNoInterface
            }
        }
    }
}

enum Attr {
    Int(i64),
    Float(f64),
    Text(Vec<TChar>),
    Binary(Vec<u8>),
}

#[derive(Default)]
struct Attributes {
    values: Mutex<HashMap<String, Attr>>,
}

impl Class for Attributes {
    type Interfaces = (IAttributeList,);
}

fn attr_key(id: *const c_char) -> String {
    if id.is_null() { String::new() } else { unsafe { CStr::from_ptr(id) }.to_string_lossy().into_owned() }
}

impl IAttributeListTrait for Attributes {
    unsafe fn setInt(&self, id: *const c_char, value: i64) -> tresult {
        self.values.lock().map(|mut v| v.insert(attr_key(id), Attr::Int(value))).map_or(kInternalError, |_| kResultOk)
    }

    unsafe fn getInt(&self, id: *const c_char, value: *mut i64) -> tresult {
        match self.values.lock().ok().as_ref().and_then(|v| match v.get(&attr_key(id)) {
            Some(Attr::Int(i)) => Some(*i),
            _ => None,
        }) {
            Some(i) => {
                unsafe { *value = i };
                kResultOk
            }
            None => kResultFalse,
        }
    }

    unsafe fn setFloat(&self, id: *const c_char, value: f64) -> tresult {
        self.values.lock().map(|mut v| v.insert(attr_key(id), Attr::Float(value))).map_or(kInternalError, |_| kResultOk)
    }

    unsafe fn getFloat(&self, id: *const c_char, value: *mut f64) -> tresult {
        match self.values.lock().ok().as_ref().and_then(|v| match v.get(&attr_key(id)) {
            Some(Attr::Float(f)) => Some(*f),
            _ => None,
        }) {
            Some(f) => {
                unsafe { *value = f };
                kResultOk
            }
            None => kResultFalse,
        }
    }

    unsafe fn setString(&self, id: *const c_char, string: *const TChar) -> tresult {
        if string.is_null() {
            return kInvalidArgument;
        }
        let mut text = Vec::new();
        let mut i = 0;
        while unsafe { *string.add(i) } != 0 {
            text.push(unsafe { *string.add(i) });
            i += 1;
        }
        text.push(0);
        self.values.lock().map(|mut v| v.insert(attr_key(id), Attr::Text(text))).map_or(kInternalError, |_| kResultOk)
    }

    unsafe fn getString(&self, id: *const c_char, string: *mut TChar, size_in_bytes: u32) -> tresult {
        let Ok(values) = self.values.lock() else { return kInternalError };
        let Some(Attr::Text(text)) = values.get(&attr_key(id)) else { return kResultFalse };
        let room = size_in_bytes as usize / 2;
        if room == 0 {
            return kResultFalse;
        }
        let n = text.len().min(room);
        unsafe {
            ptr::copy_nonoverlapping(text.as_ptr(), string, n);
            *string.add(n - 1) = 0;
        }
        kResultOk
    }

    unsafe fn setBinary(&self, id: *const c_char, data: *const c_void, size_in_bytes: u32) -> tresult {
        let bytes = if data.is_null() { Vec::new() } else { unsafe { std::slice::from_raw_parts(data as *const u8, size_in_bytes as usize) }.to_vec() };
        self.values.lock().map(|mut v| v.insert(attr_key(id), Attr::Binary(bytes))).map_or(kInternalError, |_| kResultOk)
    }

    unsafe fn getBinary(&self, id: *const c_char, data: *mut *const c_void, size_in_bytes: *mut u32) -> tresult {
        let Ok(values) = self.values.lock() else { return kInternalError };
        let Some(Attr::Binary(bytes)) = values.get(&attr_key(id)) else { return kResultFalse };
        // The bytes stay put while the attribute list lives, as VST3 expects.
        unsafe {
            *data = bytes.as_ptr().cast();
            *size_in_bytes = bytes.len() as u32;
        }
        kResultOk
    }
}

struct HostMessage {
    id: Mutex<Option<CString>>,
    attributes: ComPtr<IAttributeList>,
}

impl HostMessage {
    fn new() -> Self {
        let attributes = ComWrapper::new(Attributes::default()).to_com_ptr::<IAttributeList>().expect("attribute lists are attribute lists");
        Self { id: Mutex::new(None), attributes }
    }
}

impl Class for HostMessage {
    type Interfaces = (IMessage,);
}

impl IMessageTrait for HostMessage {
    unsafe fn getMessageID(&self) -> FIDString {
        self.id.lock().ok().and_then(|id| id.as_ref().map(|c| c.as_ptr())).unwrap_or(ptr::null())
    }

    unsafe fn setMessageID(&self, id: FIDString) {
        if let Ok(mut slot) = self.id.lock() {
            *slot = (!id.is_null()).then(|| unsafe { CStr::from_ptr(id) }.to_owned());
        }
    }

    unsafe fn getAttributes(&self) -> *mut IAttributeList {
        self.attributes.as_ptr()
    }
}

/// A plugin's state in memory, read or written like a file.
#[derive(Default)]
struct Stream {
    data: Mutex<(Vec<u8>, usize)>,
}

impl Stream {
    fn with(data: &[u8]) -> Self {
        Self { data: Mutex::new((data.to_vec(), 0)) }
    }
}

impl Class for Stream {
    type Interfaces = (IBStream,);
}

impl IBStreamTrait for Stream {
    unsafe fn read(&self, buffer: *mut c_void, num_bytes: i32, read: *mut i32) -> tresult {
        let Ok(mut d) = self.data.lock() else { return kInternalError };
        let (data, pos) = &mut *d;
        let n = (num_bytes.max(0) as usize).min(data.len().saturating_sub(*pos));
        unsafe { ptr::copy_nonoverlapping(data[*pos..].as_ptr(), buffer as *mut u8, n) };
        *pos += n;
        if !read.is_null() {
            unsafe { *read = n as i32 };
        }
        kResultOk
    }

    unsafe fn write(&self, buffer: *mut c_void, num_bytes: i32, written: *mut i32) -> tresult {
        let Ok(mut d) = self.data.lock() else { return kInternalError };
        let (data, pos) = &mut *d;
        let n = num_bytes.max(0) as usize;
        let bytes = unsafe { std::slice::from_raw_parts(buffer as *const u8, n) };
        if data.len() < *pos + n {
            data.resize(*pos + n, 0);
        }
        data[*pos..*pos + n].copy_from_slice(bytes);
        *pos += n;
        if !written.is_null() {
            unsafe { *written = n as i32 };
        }
        kResultOk
    }

    unsafe fn seek(&self, offset: i64, mode: i32, result: *mut i64) -> tresult {
        let Ok(mut d) = self.data.lock() else { return kInternalError };
        let (data, pos) = &mut *d;
        let base = match mode as IBStream_::IStreamSeekMode {
            IBStream_::IStreamSeekMode_::kIBSeekSet => 0,
            IBStream_::IStreamSeekMode_::kIBSeekCur => *pos as i64,
            IBStream_::IStreamSeekMode_::kIBSeekEnd => data.len() as i64,
            _ => return kInvalidArgument,
        };
        *pos = (base + offset).clamp(0, data.len() as i64) as usize;
        if !result.is_null() {
            unsafe { *result = *pos as i64 };
        }
        kResultOk
    }

    unsafe fn tell(&self, pos: *mut i64) -> tresult {
        let Ok(d) = self.data.lock() else { return kInternalError };
        if !pos.is_null() {
            unsafe { *pos = d.1 as i64 };
        }
        kResultOk
    }
}

/// Where the plugin's editor reports what the user does in it.
struct Handler {
    host: Arc<HostState>,
}

impl Class for Handler {
    type Interfaces = (IComponentHandler,);
}

impl Handler {
    fn edit(&self, event: OutEvent) {
        if let Ok(mut edits) = self.host.edits.lock() {
            edits.push(event);
        }
        self.host.hooks.wake();
    }
}

impl IComponentHandlerTrait for Handler {
    unsafe fn beginEdit(&self, id: ParamID) -> tresult {
        self.edit(OutEvent { id, value: None, gesture: Gesture::Begin });
        kResultOk
    }

    unsafe fn performEdit(&self, id: ParamID, value: ParamValue) -> tresult {
        // The processor hears it in its next block; the app gets it as a change.
        if let Ok(mut pending) = self.host.to_processor.lock() {
            pending.push((id, value));
        }
        self.edit(OutEvent { id, value: Some(value), gesture: Gesture::None });
        kResultOk
    }

    unsafe fn endEdit(&self, id: ParamID) -> tresult {
        self.edit(OutEvent { id, value: None, gesture: Gesture::End });
        kResultOk
    }

    unsafe fn restartComponent(&self, flags: i32) -> tresult {
        let flags = flags as RestartFlags;
        if flags & RestartFlags_::kLatencyChanged != 0 {
            self.host.flag(&self.host.latency);
        }
        if flags & (RestartFlags_::kParamValuesChanged | RestartFlags_::kParamTitlesChanged) != 0 {
            self.host.flag(&self.host.rescan);
        }
        if flags & (RestartFlags_::kReloadComponent | RestartFlags_::kIoChanged) != 0 {
            self.host.flag(&self.host.restart);
        }
        kResultOk
    }
}

/// The window the editor sits in: it asks to be resized through here.
struct Frame {
    host: Arc<HostState>,
}

impl Class for Frame {
    type Interfaces = (IPlugFrame,);
}

impl IPlugFrameTrait for Frame {
    unsafe fn resizeView(&self, view: *mut IPlugView, size: *mut ViewRect) -> tresult {
        if size.is_null() {
            return kInvalidArgument;
        }
        let r = unsafe { *size };
        if !self.host.hooks.resize((r.right - r.left).max(1) as u32, (r.bottom - r.top).max(1) as u32) {
            return kResultFalse;
        }
        if let Some(view) = unsafe { vst3::ComRef::from_raw(view) } {
            unsafe { view.onSize(size) };
        }
        kResultOk
    }
}

/// Notes for one block.
struct EventList {
    events: RefCell<Vec<Event>>,
}

impl Class for EventList {
    type Interfaces = (IEventList,);
}

impl IEventListTrait for EventList {
    unsafe fn getEventCount(&self) -> i32 {
        self.events.borrow().len() as i32
    }

    unsafe fn getEvent(&self, index: i32, e: *mut Event) -> tresult {
        match self.events.borrow().get(index as usize) {
            Some(event) => {
                unsafe { *e = *event };
                kResultOk
            }
            None => kInvalidArgument,
        }
    }

    unsafe fn addEvent(&self, e: *mut Event) -> tresult {
        let mut events = self.events.borrow_mut();
        if events.len() >= events.capacity() || e.is_null() {
            return kResultFalse;
        }
        events.push(unsafe { *e });
        kResultOk
    }
}

/// One parameter's changes in a block.
struct Queue {
    id: Cell<ParamID>,
    points: RefCell<Vec<(i32, f64)>>,
}

impl Class for Queue {
    type Interfaces = (IParamValueQueue,);
}

impl IParamValueQueueTrait for Queue {
    unsafe fn getParameterId(&self) -> ParamID {
        self.id.get()
    }

    unsafe fn getPointCount(&self) -> i32 {
        self.points.borrow().len() as i32
    }

    unsafe fn getPoint(&self, index: i32, offset: *mut i32, value: *mut ParamValue) -> tresult {
        match self.points.borrow().get(index as usize) {
            Some(&(at, v)) => {
                unsafe {
                    *offset = at;
                    *value = v;
                }
                kResultOk
            }
            None => kInvalidArgument,
        }
    }

    unsafe fn addPoint(&self, offset: i32, value: ParamValue, index: *mut i32) -> tresult {
        let mut points = self.points.borrow_mut();
        let i = match points.iter().position(|&(at, _)| at >= offset) {
            Some(i) if points[i].0 == offset => {
                points[i].1 = value;
                i
            }
            Some(i) if points.len() < points.capacity() => {
                points.insert(i, (offset, value));
                i
            }
            None if points.len() < points.capacity() => {
                points.push((offset, value));
                points.len() - 1
            }
            _ => return kResultFalse,
        };
        if !index.is_null() {
            unsafe { *index = i as i32 };
        }
        kResultOk
    }
}

/// Every parameter's changes in a block, from fixed queues so nothing allocates.
struct Changes {
    queues: Vec<ComWrapper<Queue>>,
    pointers: Vec<ComPtr<IParamValueQueue>>,
    used: Cell<usize>,
}

impl Changes {
    fn new() -> Self {
        let queues: Vec<ComWrapper<Queue>> = (0..QUEUES).map(|_| ComWrapper::new(Queue { id: Cell::new(0), points: RefCell::new(Vec::with_capacity(POINTS)) })).collect();
        let pointers = queues.iter().map(|q| q.to_com_ptr::<IParamValueQueue>().expect("queues are queues")).collect();
        Self { queues, pointers, used: Cell::new(0) }
    }

    fn clear(&self) {
        for q in &self.queues[..self.used.get()] {
            q.points.borrow_mut().clear();
        }
        self.used.set(0);
    }

    fn queue(&self, id: ParamID) -> Option<usize> {
        let used = self.used.get();
        if let Some(i) = self.queues[..used].iter().position(|q| q.id.get() == id) {
            return Some(i);
        }
        if used >= self.queues.len() {
            return None;
        }
        self.queues[used].id.set(id);
        self.used.set(used + 1);
        Some(used)
    }

    fn add(&self, id: ParamID, at: i32, value: f64) {
        if let Some(i) = self.queue(id) {
            unsafe { self.queues[i].addPoint(at, value, ptr::null_mut()) };
        }
    }

    /// Each parameter's last value in the block.
    fn last(&self) -> impl Iterator<Item = (ParamID, f64)> + '_ {
        self.queues[..self.used.get()].iter().filter_map(|q| q.points.borrow().last().map(|&(_, v)| (q.id.get(), v)))
    }
}

impl Class for Changes {
    type Interfaces = (IParameterChanges,);
}

impl IParameterChangesTrait for Changes {
    unsafe fn getParameterCount(&self) -> i32 {
        self.used.get() as i32
    }

    unsafe fn getParameterData(&self, index: i32) -> *mut IParamValueQueue {
        if (index as usize) < self.used.get() { self.pointers[index as usize].as_ptr() } else { ptr::null_mut() }
    }

    unsafe fn addParameterData(&self, id: *const ParamID, index: *mut i32) -> *mut IParamValueQueue {
        let Some(i) = (!id.is_null()).then(|| self.queue(unsafe { *id })).flatten() else { return ptr::null_mut() };
        if !index.is_null() {
            unsafe { *index = i as i32 };
        }
        self.pointers[i].as_ptr()
    }
}

// ---------- The plugin ----------

/// Channels on a plugin's audio buses, and whether it takes notes.
#[derive(Debug, Clone, Default)]
pub struct Layout {
    pub inputs: Vec<u32>,
    pub outputs: Vec<u32>,
    pub main_in: Option<usize>,
    pub main_out: Option<usize>,
    pub notes: bool,
}

fn channels(arrangement: SpeakerArrangement) -> u32 {
    arrangement.count_ones()
}

/// One plugin, driven from the sandbox's main thread.
pub struct Plugin {
    component: ComPtr<IComponent>,
    processor: ComPtr<IAudioProcessor>,
    controller: Option<ComPtr<IEditController>>,
    /// The controller is an object of its own (initialized and ended apart).
    separate: bool,
    connection: Option<(ComPtr<IConnectionPoint>, ComPtr<IConnectionPoint>)>,
    view: Option<ComPtr<IPlugView>>,
    frame: ComPtr<IPlugFrame>,
    _handler: ComPtr<IComponentHandler>,
    _app: ComPtr<FUnknown>,
    host: Arc<HostState>,
    layout: Layout,
    /// The parameter the plugin maps the sustain pedal to.
    sustain: Option<ParamID>,
    name: String,
    active: bool,
    // Declared last: the module closes after the plugin is gone.
    _module: Arc<Module>,
}

unsafe impl Send for Plugin {}

impl Plugin {
    pub fn create(module: Arc<Module>, id: &str, host: Arc<HostState>) -> Result<Self, String> {
        let cid = parse_id(id).ok_or("Bad plugin id.")?;
        let name = module.descriptors().into_iter().find(|d| d.id == id).map(|d| d.name).unwrap_or_default();
        let factory = module.factory();
        let create = |cid: &TUID, iid: &[u8; 16]| -> *mut c_void {
            let mut obj = ptr::null_mut();
            let result = unsafe { factory.createInstance(cid.as_ptr(), iid.as_ptr() as FIDString, &mut obj) };
            if ok(result) { obj } else { ptr::null_mut() }
        };
        let component = unsafe { ComPtr::from_raw(create(&cid, &IComponent::IID) as *mut IComponent) }.ok_or_else(|| format!("The module has no plugin “{id}”."))?;
        let app = ComWrapper::new(HostApp).to_com_ptr::<FUnknown>().ok_or("No host context.")?;
        if !ok(unsafe { component.initialize(app.as_ptr()) }) {
            return Err("The plugin failed to start.".into());
        }

        // The editing half: the same object, or a class of its own that is connected to the processing half.
        let (controller, separate) = match component.cast::<IEditController>() {
            Some(c) => (Some(c), false),
            None => {
                let mut ccid: TUID = [0; 16];
                let c = ok(unsafe { component.getControllerClassId(&mut ccid) })
                    .then(|| unsafe { ComPtr::from_raw(create(&ccid, &IEditController::IID) as *mut IEditController) })
                    .flatten();
                match c {
                    Some(c) if ok(unsafe { c.initialize(app.as_ptr()) }) => (Some(c), true),
                    _ => (None, false),
                }
            }
        };
        let connection = match (separate, &controller) {
            (true, Some(c)) => match (component.cast::<IConnectionPoint>(), c.cast::<IConnectionPoint>()) {
                (Some(a), Some(b)) => {
                    unsafe {
                        a.connect(b.as_ptr());
                        b.connect(a.as_ptr());
                    }
                    Some((a, b))
                }
                _ => None,
            },
            _ => None,
        };
        let handler = ComWrapper::new(Handler { host: Arc::clone(&host) }).to_com_ptr::<IComponentHandler>().ok_or("No handler.")?;
        let frame = ComWrapper::new(Frame { host: Arc::clone(&host) }).to_com_ptr::<IPlugFrame>().ok_or("No frame.")?;
        let mut sustain = None;
        if let Some(c) = &controller {
            // The controller starts from the processor's state.
            let stream = ComWrapper::new(Stream::default());
            if let Some(s) = stream.to_com_ptr::<IBStream>()
                && ok(unsafe { component.getState(s.as_ptr()) })
            {
                unsafe {
                    s.seek(0, IBStream_::IStreamSeekMode_::kIBSeekSet as _, ptr::null_mut());
                    c.setComponentState(s.as_ptr());
                }
            }
            unsafe { c.setComponentHandler(handler.as_ptr()) };
            if let Some(map) = c.cast::<IMidiMapping>() {
                let mut param: ParamID = 0;
                if ok(unsafe { map.getMidiControllerAssignment(0, 0, ControllerNumbers_::kCtrlSustainOnOff as i16, &mut param) }) {
                    sustain = Some(param);
                }
            }
        }
        let processor = component.cast::<IAudioProcessor>().ok_or("The plugin does not process audio.")?;
        let mut plugin = Self {
            component,
            processor,
            controller,
            separate,
            connection,
            view: None,
            frame,
            _handler: handler,
            _app: app,
            host,
            layout: Layout::default(),
            sustain,
            name,
            active: false,
            _module: module,
        };
        plugin.layout = plugin.arrange();
        Ok(plugin)
    }

    /// Turns on the main buses (stereo where the plugin agrees) and the first note input.
    fn arrange(&self) -> Layout {
        let c = &self.component;
        let mut layout = Layout::default();
        let mut wanted = [Vec::new(), Vec::new()];
        for (side, dir) in [BusDirections_::kInput, BusDirections_::kOutput].into_iter().enumerate() {
            let count = unsafe { c.getBusCount(MediaTypes_::kAudio as MediaType, dir as BusDirection) };
            for i in 0..count {
                let mut info: BusInfo = unsafe { std::mem::zeroed() };
                unsafe { c.getBusInfo(MediaTypes_::kAudio as MediaType, dir as BusDirection, i, &mut info) };
                let main = info.busType == BusTypes_::kMain as BusType;
                let first_main = main && if side == 0 { layout.main_in.is_none() } else { layout.main_out.is_none() };
                if first_main {
                    if side == 0 {
                        layout.main_in = Some(i as usize);
                    } else {
                        layout.main_out = Some(i as usize);
                    }
                }
                unsafe { c.activateBus(MediaTypes_::kAudio as MediaType, dir as BusDirection, i, u8::from(first_main)) };
                let mut arrangement: SpeakerArrangement = 0;
                unsafe { self.processor.getBusArrangement(dir as BusDirection, i, &mut arrangement) };
                wanted[side].push(if first_main && info.channelCount >= 2 { SpeakerArr::kStereo } else { arrangement });
            }
        }
        let [mut ins, mut outs] = wanted;
        let accepted = ok(unsafe { self.processor.setBusArrangements(ins.as_mut_ptr(), ins.len() as i32, outs.as_mut_ptr(), outs.len() as i32) });
        for (side, dir) in [BusDirections_::kInput, BusDirections_::kOutput].into_iter().enumerate() {
            let list = if side == 0 { &ins } else { &outs };
            let counts: Vec<u32> = (0..list.len())
                .map(|i| {
                    if accepted {
                        return channels(list[i]);
                    }
                    let mut arrangement: SpeakerArrangement = 0;
                    unsafe { self.processor.getBusArrangement(dir as BusDirection, i as i32, &mut arrangement) };
                    channels(arrangement)
                })
                .collect();
            if side == 0 {
                layout.inputs = counts;
            } else {
                layout.outputs = counts;
            }
        }
        if unsafe { c.getBusCount(MediaTypes_::kEvent as MediaType, BusDirections_::kInput as BusDirection) } > 0 {
            unsafe { c.activateBus(MediaTypes_::kEvent as MediaType, BusDirections_::kInput as BusDirection, 0, 1) };
            layout.notes = true;
        }
        layout
    }

    pub fn name(&self) -> String {
        self.name.clone()
    }

    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    pub fn latency(&self) -> u32 {
        unsafe { self.processor.getLatencySamples() }
    }

    /// Starts the plugin at `rate`; the processor goes to the audio thread.
    pub fn activate(&mut self, rate: f64) -> Result<Processor, String> {
        self.deactivate();
        let mut setup = ProcessSetup {
            processMode: ProcessModes_::kRealtime as _,
            symbolicSampleSize: SymbolicSampleSizes_::kSample32 as _,
            maxSamplesPerBlock: MAX_FRAMES as i32,
            sampleRate: rate,
        };
        if !ok(unsafe { self.processor.setupProcessing(&mut setup) }) {
            return Err(format!("The plugin refused to run at {rate} Hz."));
        }
        if !ok(unsafe { self.component.setActive(1) }) {
            return Err("The plugin could not start processing.".into());
        }
        self.active = true;
        Ok(Processor::new(self.processor.clone(), self.layout.clone(), Arc::clone(&self.host), self.sustain, rate))
    }

    /// Stops the plugin. Its processor must have stopped processing first.
    pub fn deactivate(&mut self) {
        if std::mem::take(&mut self.active) {
            unsafe { self.component.setActive(0) };
        }
    }

    /// Tells the controller what the processor heard or did, so its editor follows.
    pub fn sync_controller(&self) -> Vec<OutEvent> {
        let pending = self.host.to_controller.lock().map(|mut p| std::mem::take(&mut *p)).unwrap_or_default();
        let Some(c) = &self.controller else { return Vec::new() };
        pending
            .into_iter()
            .filter_map(|(id, value, own)| {
                unsafe { c.setParamNormalized(id, value) };
                own.then_some(OutEvent { id, value: Some(value), gesture: Gesture::None })
            })
            .collect()
    }

    fn text(&self, c: &ComPtr<IEditController>, id: ParamID, value: f64, units: &str) -> String {
        let mut text: String128 = [0; 128];
        if ok(unsafe { c.getParamStringByValue(id, value, &mut text) }) {
            let t = wide_text(&text);
            if units.is_empty() { t } else { format!("{t} {units}") }
        } else {
            format!("{value:.2}")
        }
    }

    pub fn params(&self) -> Vec<ParamInfo> {
        let Some(c) = &self.controller else { return Vec::new() };
        (0..unsafe { c.getParameterCount() })
            .filter_map(|i| {
                let mut info: ParameterInfo = unsafe { std::mem::zeroed() };
                ok(unsafe { c.getParameterInfo(i, &mut info) }).then_some(info)
            })
            .map(|info| {
                let flags = info.flags as ParameterInfo_::ParameterFlags;
                let value = unsafe { c.getParamNormalized(info.id) };
                let value = if value.is_finite() { value.clamp(0.0, 1.0) } else { 0.0 };
                ParamInfo {
                    id: info.id,
                    name: wide_text(&info.title),
                    module: String::new(),
                    min: 0.0,
                    max: 1.0,
                    default: info.defaultNormalizedValue.clamp(0.0, 1.0),
                    value,
                    text: self.text(c, info.id, value, &wide_text(&info.units)),
                    stepped: info.stepCount > 0,
                    steps: info.stepCount.max(0) as u32,
                    automatable: flags & ParameterInfo_::ParameterFlags_::kCanAutomate != 0,
                    hidden: flags & ParameterInfo_::ParameterFlags_::kIsHidden != 0,
                    readonly: flags & ParameterInfo_::ParameterFlags_::kIsReadOnly != 0,
                }
            })
            .collect()
    }

    pub fn texts(&self, ids: &[u32]) -> Vec<(u32, f64, String)> {
        let Some(c) = &self.controller else { return Vec::new() };
        let units: HashMap<u32, String> = (0..unsafe { c.getParameterCount() })
            .filter_map(|i| {
                let mut info: ParameterInfo = unsafe { std::mem::zeroed() };
                ok(unsafe { c.getParameterInfo(i, &mut info) }).then(|| (info.id, wide_text(&info.units)))
            })
            .collect();
        ids.iter()
            .map(|&id| {
                let value = unsafe { c.getParamNormalized(id) };
                (id, value, self.text(c, id, value, units.get(&id).map_or("", String::as_str)))
            })
            .filter(|(_, v, _)| v.is_finite())
            .collect()
    }

    /// The processor's state and the controller's, one after the other.
    pub fn save_state(&self) -> Result<Vec<u8>, String> {
        let read = |write: &dyn Fn(*mut IBStream) -> tresult| -> Result<Vec<u8>, String> {
            let stream = ComWrapper::new(Stream::default());
            let s = stream.to_com_ptr::<IBStream>().ok_or("No stream.")?;
            if !ok(write(s.as_ptr())) {
                return Err("The plugin failed to save its state.".into());
            }
            Ok(stream.data.lock().map(|d| d.0.clone()).unwrap_or_default())
        };
        let processing = read(&|s| unsafe { self.component.getState(s) })?;
        let editing = match &self.controller {
            Some(c) if self.separate => read(&|s| unsafe { c.getState(s) }).unwrap_or_default(),
            _ => Vec::new(),
        };
        let mut state = b"SNQ3".to_vec();
        state.extend_from_slice(&(processing.len() as u32).to_le_bytes());
        state.extend_from_slice(&processing);
        state.extend_from_slice(&editing);
        Ok(state)
    }

    pub fn load_state(&self, state: &[u8]) -> Result<(), String> {
        if state.len() < 8 || &state[..4] != b"SNQ3" {
            return Err("That is not a saved VST3 state.".into());
        }
        let len = u32::from_le_bytes([state[4], state[5], state[6], state[7]]) as usize;
        let (processing, editing) = state[8..].split_at(len.min(state.len() - 8));
        let stream = |bytes: &[u8]| ComWrapper::new(Stream::with(bytes)).to_com_ptr::<IBStream>();
        let s = stream(processing).ok_or("No stream.")?;
        if !ok(unsafe { self.component.setState(s.as_ptr()) }) {
            return Err("The plugin could not read its saved state.".into());
        }
        if let Some(c) = &self.controller {
            if let Some(s) = stream(processing) {
                unsafe { c.setComponentState(s.as_ptr()) };
            }
            if self.separate
                && !editing.is_empty()
                && let Some(s) = stream(editing)
            {
                unsafe { c.setState(s.as_ptr()) };
            }
        }
        Ok(())
    }

    pub fn has_gui(&self) -> bool {
        let Some(c) = &self.controller else { return false };
        let view = unsafe { ComPtr::from_raw(c.createView(ViewType::kEditor)) };
        view.is_some_and(|v| ok(unsafe { v.isPlatformTypeSupported(kPlatformTypeHWND) }))
    }

    /// Puts the editor inside `hwnd`; its size.
    pub fn embed(&mut self, hwnd: *mut c_void, scale: f64) -> Result<(u32, u32), String> {
        let c = self.controller.as_ref().ok_or("This plugin has no editor.")?;
        let view = unsafe { ComPtr::from_raw(c.createView(ViewType::kEditor)) }.ok_or("This plugin has no editor.")?;
        if !ok(unsafe { view.isPlatformTypeSupported(kPlatformTypeHWND) }) {
            return Err("This plugin's editor does not run on Windows.".into());
        }
        unsafe { view.setFrame(self.frame.as_ptr()) };
        if let Some(scaling) = view.cast::<IPlugViewContentScaleSupport>() {
            unsafe { scaling.setContentScaleFactor(scale as f32) };
        }
        if !ok(unsafe { view.attached(hwnd, kPlatformTypeHWND) }) {
            unsafe { view.setFrame(ptr::null_mut()) };
            return Err("The plugin could not open its editor.".into());
        }
        let mut r = ViewRect { left: 0, top: 0, right: 480, bottom: 320 };
        unsafe { view.getSize(&mut r) };
        self.view = Some(view);
        Ok(((r.right - r.left).max(1) as u32, (r.bottom - r.top).max(1) as u32))
    }

    pub fn unembed(&mut self) {
        if let Some(view) = self.view.take() {
            unsafe {
                view.removed();
                view.setFrame(ptr::null_mut());
            }
        }
    }

    pub fn can_resize(&self) -> bool {
        self.view.as_ref().is_some_and(|v| ok(unsafe { v.canResize() }))
    }

    pub fn set_size(&self, width: u32, height: u32) {
        if let Some(v) = &self.view {
            let mut r = ViewRect { left: 0, top: 0, right: width as i32, bottom: height as i32 };
            unsafe {
                v.checkSizeConstraint(&mut r);
                v.onSize(&mut r);
            }
        }
    }

    pub fn set_scale(&self, scale: f64) {
        if let Some(scaling) = self.view.as_ref().and_then(|v| v.cast::<IPlugViewContentScaleSupport>()) {
            unsafe { scaling.setContentScaleFactor(scale as f32) };
        }
    }
}

impl Drop for Plugin {
    fn drop(&mut self) {
        self.unembed();
        self.deactivate();
        if let Some((a, b)) = self.connection.take() {
            unsafe {
                a.disconnect(b.as_ptr());
                b.disconnect(a.as_ptr());
            }
        }
        if let Some(c) = self.controller.take() {
            unsafe { c.setComponentHandler(ptr::null_mut()) };
            if self.separate {
                unsafe { c.terminate() };
            }
        }
        unsafe { self.component.terminate() };
    }
}

/// One plugin, driven from the sandbox's audio thread. Buffers, queues and
/// event lists are made when it starts, so processing allocates nothing.
pub struct Processor {
    processor: ComPtr<IAudioProcessor>,
    layout: Layout,
    host: Arc<HostState>,
    sustain: Option<ParamID>,
    rate: f64,
    started: bool,
    scratch: Vec<Vec<f32>>,
    events: ComWrapper<EventList>,
    events_ptr: ComPtr<IEventList>,
    changes_in: ComWrapper<Changes>,
    changes_in_ptr: ComPtr<IParameterChanges>,
    changes_out: ComWrapper<Changes>,
    changes_out_ptr: ComPtr<IParameterChanges>,
    out: Vec<OutEvent>,
    /// Keys the processor holds down, to let go of on "all notes off".
    held: [bool; 128],
    /// Parameters the editor is being moved by hand: the host leaves them alone.
    edits: Vec<(u32, f64)>,
}

unsafe impl Send for Processor {}

impl Processor {
    fn new(processor: ComPtr<IAudioProcessor>, layout: Layout, host: Arc<HostState>, sustain: Option<ParamID>, rate: f64) -> Self {
        let channels: u32 = layout.inputs.iter().chain(&layout.outputs).sum::<u32>() + 2;
        let events = ComWrapper::new(EventList { events: RefCell::new(Vec::with_capacity(MAX_EVENTS + 128)) });
        let events_ptr = events.to_com_ptr::<IEventList>().expect("event lists are event lists");
        let changes_in = ComWrapper::new(Changes::new());
        let changes_in_ptr = changes_in.to_com_ptr::<IParameterChanges>().expect("changes are changes");
        let changes_out = ComWrapper::new(Changes::new());
        let changes_out_ptr = changes_out.to_com_ptr::<IParameterChanges>().expect("changes are changes");
        Self {
            processor,
            scratch: (0..channels).map(|_| vec![0.0; MAX_FRAMES]).collect(),
            layout,
            host,
            sustain,
            rate,
            started: false,
            events,
            events_ptr,
            changes_in,
            changes_in_ptr,
            changes_out,
            changes_out_ptr,
            out: Vec::with_capacity(QUEUES),
            held: [false; 128],
            edits: Vec::with_capacity(QUEUES),
        }
    }

    pub fn stop(&mut self) {
        if std::mem::take(&mut self.started) {
            unsafe { self.processor.setProcessing(0) };
        }
    }

    pub fn take_out(&mut self) -> std::vec::Drain<'_, OutEvent> {
        self.out.drain(..)
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

    fn note(&mut self, at: i32, on: bool, key: u8, velocity: f32) {
        let mut e: Event = unsafe { std::mem::zeroed() };
        e.busIndex = 0;
        e.sampleOffset = at;
        if on {
            e.r#type = Event_::EventTypes_::kNoteOnEvent as u16;
            e.__field0.noteOn = NoteOnEvent { channel: 0, pitch: i16::from(key), tuning: 0.0, velocity, length: 0, noteId: -1 };
        } else {
            e.r#type = Event_::EventTypes_::kNoteOffEvent as u16;
            e.__field0.noteOff = NoteOffEvent { channel: 0, pitch: i16::from(key), velocity: 0.0, noteId: -1, tuning: 0.0 };
        }
        self.held[usize::from(key.min(127))] = on;
        unsafe { self.events.addEvent(&mut e) };
    }

    /// Processes the shared block: its input, notes and parameter changes
    /// in, its output out.
    ///
    /// # Safety
    /// `block` must be the shared block, and this side must hold the turn.
    pub unsafe fn process(&mut self, block: *mut Block) {
        let b = unsafe { &mut *block };
        let n = (b.frames as usize).min(MAX_FRAMES);
        if b.flags & RESET != 0 && self.started {
            // VST3 has no reset: stopping and starting processing clears tails in most plugins.
            unsafe { self.processor.setProcessing(0) };
            self.started = false;
        }
        if !self.started {
            unsafe { self.processor.setProcessing(1) };
            self.started = true;
        }

        self.changes_in.clear();
        self.changes_out.clear();
        self.events.events.borrow_mut().clear();
        // What the user does in the editor first, then the app's changes,
        // except to parameters the editor is moving right now.
        self.edits.clear();
        if let Ok(mut pending) = self.host.to_processor.try_lock() {
            self.edits.extend(pending.drain(..));
        }
        for i in 0..self.edits.len() {
            let (id, value) = self.edits[i];
            self.changes_in.add(id, 0, value);
        }
        let mut told = false;
        let host = Arc::clone(&self.host);
        let mut controller = host.to_controller.try_lock().ok();
        for e in &b.events[..(b.event_count as usize).min(MAX_EVENTS)] {
            let at = e.at.min(n.saturating_sub(1) as u32) as i32;
            match e.kind {
                kind::PARAM if !self.edits.iter().any(|&(id, _)| id == e.id) => {
                    self.changes_in.add(e.id, at, e.value.clamp(0.0, 1.0));
                    if let Some(c) = controller.as_mut() {
                        c.push((e.id, e.value.clamp(0.0, 1.0), false));
                        told = true;
                    }
                }
                kind::NOTE_ON if self.layout.notes => self.note(at, true, e.id.min(127) as u8, e.value.clamp(0.0, 1.0) as f32),
                kind::NOTE_OFF if self.layout.notes => self.note(at, false, e.id.min(127) as u8, 0.0),
                kind::SUSTAIN => {
                    if let Some(id) = self.sustain {
                        self.changes_in.add(id, at, if e.value > 0.5 { 1.0 } else { 0.0 });
                    }
                }
                kind::ALL_NOTES_OFF | kind::ALL_SOUND_OFF if self.layout.notes => {
                    for key in 0..128u8 {
                        if self.held[usize::from(key)] {
                            self.note(at, false, key, 0.0);
                        }
                    }
                }
                _ => {}
            }
        }

        // Buffers: the main buses use the shared block, everything else scratch.
        let (input, output) = (b.input.as_mut_ptr(), b.output.as_mut_ptr());
        let mut scratch = self.scratch.iter_mut();
        let mut pointers: [[*mut f32; 64]; 2] = [[ptr::null_mut(); 64]; 2];
        let mut used = [0usize; 2];
        let mut buses: [[AudioBusBuffers; 16]; 2] = unsafe { std::mem::zeroed() };
        let mut bus_count = [0usize; 2];
        let mut mono_in = None;
        let mut mono_out = None;
        for (side, (list, main)) in [(&self.layout.inputs, self.layout.main_in), (&self.layout.outputs, self.layout.main_out)].into_iter().enumerate() {
            for (p, &count) in list.iter().enumerate().take(16) {
                let first = used[side];
                for c in 0..count as usize {
                    let at = used[side];
                    if at >= 64 {
                        break;
                    }
                    let shared = if side == 0 { input } else { output };
                    pointers[side][at] = if Some(p) == main && count >= 2 && c < 2 {
                        unsafe { (*shared.add(c)).as_mut_ptr() }
                    } else if let Some(buf) = scratch.next() {
                        if side == 0 {
                            buf[..n].fill(0.0);
                        }
                        if Some(p) == main && count == 1 {
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
                buses[side][p].numChannels = (used[side] - first) as i32;
                buses[side][p].silenceFlags = 0;
                buses[side][p].__field0.channelBuffers32 = unsafe { pointers[side].as_mut_ptr().add(first) };
                bus_count[side] = p + 1;
            }
        }
        if let Some(mono) = mono_in {
            for i in 0..n {
                unsafe { *mono.add(i) = 0.5 * (b.input[0][i] + b.input[1][i]) };
            }
        }

        let t = b.transport;
        let mut state = ProcessContext_::StatesAndFlags_::kTempoValid | ProcessContext_::StatesAndFlags_::kProjectTimeMusicValid | ProcessContext_::StatesAndFlags_::kTimeSigValid;
        if t.playing != 0 {
            state |= ProcessContext_::StatesAndFlags_::kPlaying;
        }
        let mut context: ProcessContext = unsafe { std::mem::zeroed() };
        context.state = state as u32;
        context.sampleRate = self.rate;
        context.projectTimeSamples = (t.seconds * self.rate).round() as i64;
        context.projectTimeMusic = t.beat;
        context.tempo = t.tempo;
        context.timeSigNumerator = 4;
        context.timeSigDenominator = 4;

        let mut data = ProcessData {
            processMode: ProcessModes_::kRealtime as _,
            symbolicSampleSize: SymbolicSampleSizes_::kSample32 as _,
            numSamples: n as i32,
            numInputs: bus_count[0] as i32,
            numOutputs: bus_count[1] as i32,
            inputs: buses[0].as_mut_ptr(),
            outputs: buses[1].as_mut_ptr(),
            inputParameterChanges: self.changes_in_ptr.as_ptr(),
            outputParameterChanges: self.changes_out_ptr.as_ptr(),
            inputEvents: if self.layout.notes { self.events_ptr.as_ptr() } else { ptr::null_mut() },
            outputEvents: ptr::null_mut(),
            processContext: &mut context,
        };
        let result = unsafe { self.processor.process(&mut data) };
        if result != kResultOk {
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
        // What the processor changed itself (rarely: meters, a learned value).
        for (id, value) in self.changes_out.last() {
            if let Some(c) = controller.as_mut() {
                c.push((id, value, true));
                told = true;
            }
        }
        drop(controller);
        if told {
            host.hooks.wake();
        }
    }
}
