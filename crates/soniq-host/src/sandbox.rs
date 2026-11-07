//! The sandbox process: one plugin, its window and its audio thread, so a
//! plugin that crashes or hangs takes nothing else with it.
//!
//! The main thread runs a Win32 message loop. Commands arrive on stdin (a
//! reader thread posts them to a message-only window), and so do the
//! plugin's own requests (callbacks, timers, its window, its editor's
//! edits), all handled there because CLAP and VST3 want them on the main
//! thread. The audio thread answers the app's blocks in the shared memory.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::fs::File;
use std::io::{self, Write};
use std::os::windows::io::FromRawHandle;
use std::path::Path;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::*};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use windows_sys::Win32::Foundation::{DUPLICATE_SAME_ACCESS, DuplicateHandle, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::System::Console::{GetStdHandle, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE, SetStdHandle};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_TIME_CRITICAL};
use windows_sys::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForWindow, SetProcessDpiAwarenessContext};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::host::{Hooks, HostState, OutEvent, mark_audio_thread};
use crate::instance::{Instance, Processor, scan};
use crate::protocol::{Command, Message, read_frame, write_frame};
use crate::shm::Shared;

const WAKE: u32 = WM_APP;

unsafe extern "C" {
    fn _dup2(from: i32, to: i32) -> i32;
}

/// The sandbox's entry point: `--scan <file>` lists a file's plugins,
/// `--host` hosts one plugin until told to quit. Returns the exit code.
pub fn main(args: &[String]) -> i32 {
    let mut out = take_stdout();
    match args {
        [flag, path] if flag == "--scan" => {
            let reply = match scan(Path::new(path)) {
                Ok(plugins) => Message::Scanned { plugins },
                Err(message) => Message::Failed { message },
            };
            let _ = write_frame(&mut out, &reply, &[]);
            0
        }
        [flag] if flag == "--host" => host(out),
        _ => {
            eprintln!("usage: --scan <file.clap|file.vst3> | --host");
            2
        }
    }
}

/// Keeps stdout for the protocol: whatever a plugin prints goes to stderr.
fn take_stdout() -> File {
    unsafe {
        let me = GetCurrentProcess();
        let mut own = ptr::null_mut();
        DuplicateHandle(me, GetStdHandle(STD_OUTPUT_HANDLE), me, &mut own, 0, 0, DUPLICATE_SAME_ACCESS);
        SetStdHandle(STD_OUTPUT_HANDLE, GetStdHandle(STD_ERROR_HANDLE));
        _dup2(2, 1);
        File::from_raw_handle(own)
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

type Inbox = Arc<Mutex<VecDeque<(Command, Vec<u8>)>>>;

thread_local! {
    static SANDBOX: RefCell<Option<Sandbox>> = const { RefCell::new(None) };
    /// Handling is under way: a nested wake-up only asks for another round.
    static BUSY: Cell<bool> = const { Cell::new(false) };
    static AGAIN: Cell<bool> = const { Cell::new(false) };
    /// The user closed the plugin's window.
    static CLOSED: Cell<bool> = const { Cell::new(false) };
}

/// Handles everything waiting, on the main thread. Safe to call from
/// inside itself (a plugin call that pumps messages): it then only asks
/// the outer call for another round.
fn pump() {
    if BUSY.get() {
        AGAIN.set(true);
        return;
    }
    BUSY.set(true);
    loop {
        AGAIN.set(false);
        let quit = SANDBOX.with_borrow_mut(|s| s.as_mut().is_some_and(|s| s.handle()));
        if quit {
            SANDBOX.with_borrow_mut(|s| *s = None);
            unsafe { PostQuitMessage(0) };
            break;
        }
        if !AGAIN.get() {
            break;
        }
    }
    BUSY.set(false);
}

unsafe extern "system" fn message_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WAKE => pump(),
        WM_TIMER if !BUSY.get() => SANDBOX.with_borrow(|s| {
            if let Some(plugin) = s.as_ref().and_then(|s| s.plugin.as_ref()) {
                plugin.on_timer(wparam as u32);
            }
        }),
        _ => return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
    0
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_CLOSE => {
            CLOSED.set(true);
            pump();
            0
        }
        WM_SIZE if !BUSY.get() => {
            let (w, h) = ((lparam & 0xffff) as u32, ((lparam >> 16) & 0xffff) as u32);
            SANDBOX.with_borrow(|s| {
                if let Some(p) = s.as_ref().and_then(|s| s.plugin.as_ref())
                    && p.can_resize()
                {
                    p.set_size(w, h);
                }
            });
            0
        }
        WM_DPICHANGED => {
            let dpi = (wparam & 0xffff) as f64;
            if !BUSY.get() {
                SANDBOX.with_borrow(|s| {
                    if let Some(p) = s.as_ref().and_then(|s| s.plugin.as_ref()) {
                        p.set_scale(dpi / 96.0);
                    }
                });
            }
            let r = unsafe { &*(lparam as *const RECT) };
            unsafe { SetWindowPos(hwnd, ptr::null_mut(), r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE) };
            0
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// The plugin's requests, turned into window messages.
struct Wake {
    /// The message-only window.
    messages: usize,
    /// The plugin's window, when open.
    window: Arc<AtomicUsize>,
}

impl Hooks for Wake {
    fn wake(&self) {
        unsafe { PostMessageW(self.messages as HWND, WAKE, 0, 0) };
    }

    fn timer(&self, id: u32, period_ms: Option<u32>) -> bool {
        unsafe {
            match period_ms {
                Some(ms) => SetTimer(self.messages as HWND, id as usize, ms, None) != 0,
                None => KillTimer(self.messages as HWND, id as usize) != 0,
            }
        }
    }

    fn resize(&self, width: u32, height: u32) -> bool {
        let hwnd = self.window.load(Relaxed) as HWND;
        !hwnd.is_null() && fit_window(hwnd, width, height)
    }
}

const STYLE: WINDOW_STYLE = WS_OVERLAPPEDWINDOW & !WS_MAXIMIZEBOX;

/// Sizes a window so its inside is `width` × `height`.
fn fit_window(hwnd: HWND, width: u32, height: u32) -> bool {
    let mut r = RECT { left: 0, top: 0, right: width as i32, bottom: height as i32 };
    unsafe {
        AdjustWindowRectExForDpi(&mut r, STYLE, 0, 0, GetDpiForWindow(hwnd));
        SetWindowPos(hwnd, ptr::null_mut(), 0, 0, r.right - r.left, r.bottom - r.top, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE) != 0
    }
}

struct Audio {
    quit: Arc<AtomicBool>,
    thread: JoinHandle<()>,
}

struct Sandbox {
    out: File,
    inbox: Inbox,
    messages: HWND,
    window: Arc<AtomicUsize>,
    floating: bool,
    shared: Option<Arc<Shared>>,
    host: Option<Arc<HostState>>,
    // Dropped in this order: the processor, then the plugin (which closes its file).
    processor: Arc<Mutex<Option<Processor>>>,
    audio: Option<Audio>,
    plugin: Option<Instance>,
    rate: f64,
    changes: (Sender<OutEvent>, Receiver<OutEvent>),
}

impl Sandbox {
    fn send(&mut self, message: &Message, payload: &[u8]) {
        if write_frame(&mut self.out, message, payload).is_err() {
            // The app is gone.
            std::process::exit(0);
        }
    }

    /// Handles commands and the plugin's requests; whether to quit.
    fn handle(&mut self) -> bool {
        loop {
            let next = self.inbox.lock().map(|mut q| q.pop_front()).unwrap_or(None);
            let Some((command, payload)) = next else { break };
            if command == Command::Quit {
                self.stop();
                self.send(&Message::Done, &[]);
                return true;
            }
            let (reply, data) = match self.command(command, &payload) {
                Ok(reply) => reply,
                Err(message) => (Message::Failed { message }, Vec::new()),
            };
            self.send(&reply, &data);
        }
        if CLOSED.take() {
            self.close_window();
            self.send(&Message::GuiClosed, &[]);
        }
        let Some(host) = self.host.clone() else { return false };
        // The plugin's main-thread work, and what its editor or processor changed.
        let mut changed = self.plugin.as_ref().map(|p| p.main_thread(host.callback.swap(false, Relaxed))).unwrap_or_default();
        changed.extend(host.edits.lock().map(|mut e| std::mem::take(&mut *e)).unwrap_or_default());
        for e in changed {
            self.send(&Message::ParamChanged { id: e.id, value: e.value, gesture: e.gesture }, &[]);
        }
        if host.restart.swap(false, Relaxed) {
            let _ = self.restart(self.rate);
        }
        if host.rescan.swap(false, Relaxed) {
            self.send(&Message::ParamsChanged, &[]);
        }
        if host.dirty.swap(false, Relaxed) {
            self.send(&Message::StateChanged, &[]);
        }
        if host.latency.swap(false, Relaxed) {
            let latency = self.plugin.as_ref().map_or(0, |p| p.latency());
            self.send(&Message::LatencyChanged { latency }, &[]);
        }
        if host.gui_closed.swap(false, Relaxed) && (self.floating || self.window.load(Relaxed) != 0) {
            self.close_window();
            self.send(&Message::GuiClosed, &[]);
        }
        // A flush request needs nothing: the engine hands every plugin a block all the time.
        host.flush.store(false, Relaxed);
        let changes: Vec<OutEvent> = self.changes.1.try_iter().collect();
        for e in changes {
            self.send(&Message::ParamChanged { id: e.id, value: e.value, gesture: e.gesture }, &[]);
        }
        false
    }

    fn command(&mut self, command: Command, payload: &[u8]) -> Result<(Message, Vec<u8>), String> {
        let done = Ok((Message::Done, Vec::new()));
        match command {
            Command::Load { path, id, shm, rate } => {
                let shared = Arc::new(Shared::open(&shm).map_err(|e| e.to_string())?);
                let host = Arc::new(HostState::new(Box::new(Wake { messages: self.messages as usize, window: Arc::clone(&self.window) })));
                let plugin = Instance::open(Path::new(&path), &id, Arc::clone(&host))?;
                (self.shared, self.host, self.plugin) = (Some(shared), Some(host), Some(plugin));
                self.restart(rate)?;
                let info = self.plugin.as_ref().ok_or("The plugin is gone.")?.loaded();
                Ok((Message::Loaded { info }, Vec::new()))
            }
            Command::Activate { rate } => {
                self.restart(rate)?;
                done
            }
            Command::Params => Ok((Message::Params { params: self.plugin()?.params() }, Vec::new())),
            Command::Texts { ids } => Ok((Message::Texts { texts: self.plugin()?.texts(&ids) }, Vec::new())),
            Command::SaveState => Ok((Message::State, self.plugin()?.save_state()?)),
            Command::LoadState => {
                self.plugin()?.load_state(payload)?;
                done
            }
            Command::ShowGui { title } => {
                self.open_window(&title)?;
                done
            }
            Command::HideGui => {
                self.close_window();
                done
            }
            Command::Quit => done,
        }
    }

    fn plugin(&self) -> Result<&Instance, String> {
        self.plugin.as_ref().ok_or_else(|| "No plugin is loaded.".to_string())
    }

    /// Stops the audio thread (it stops the plugin processing on its way out).
    fn stop_audio(&mut self) {
        if let Some(audio) = self.audio.take() {
            audio.quit.store(true, Relaxed);
            let _ = audio.thread.join();
        }
    }

    /// (Re)starts the plugin at `rate` with a fresh audio thread.
    fn restart(&mut self, rate: f64) -> Result<(), String> {
        self.stop_audio();
        let (Some(shared), Some(host)) = (self.shared.clone(), self.host.clone()) else { return Err("No plugin is loaded.".into()) };
        let plugin = self.plugin.as_mut().ok_or("No plugin is loaded.")?;
        let processor = plugin.activate(rate)?;
        self.rate = rate;
        *self.processor.lock().map_err(|_| "The plugin's audio failed.")? = Some(processor);
        let quit = Arc::new(AtomicBool::new(false));
        let (q, slot, changes) = (Arc::clone(&quit), Arc::clone(&self.processor), self.changes.0.clone());
        let thread = std::thread::Builder::new()
            .name("plugin audio".into())
            .spawn(move || audio_thread(&shared, &slot, &host, &changes, &q))
            .map_err(|e| e.to_string())?;
        self.audio = Some(Audio { quit, thread });
        Ok(())
    }

    fn stop(&mut self) {
        self.close_window();
        self.stop_audio();
        self.processor.lock().map(|mut p| p.take()).ok();
        self.plugin = None;
    }

    fn open_window(&mut self, title: &str) -> Result<(), String> {
        let hwnd = self.window.load(Relaxed) as HWND;
        if !hwnd.is_null() {
            unsafe {
                ShowWindow(hwnd, SW_SHOWNORMAL);
                SetForegroundWindow(hwnd);
            }
            return Ok(());
        }
        if self.floating {
            return Ok(());
        }
        let plugin = self.plugin.as_mut().ok_or("No plugin is loaded.")?;
        if plugin.embeds() {
            let class = wide("SoniqPluginWindow");
            let name = wide(title);
            let hwnd = unsafe {
                CreateWindowExW(0, class.as_ptr(), name.as_ptr(), STYLE, CW_USEDEFAULT, CW_USEDEFAULT, 480, 320, ptr::null_mut(), ptr::null_mut(), GetModuleHandleW(ptr::null()), ptr::null())
            };
            if hwnd.is_null() {
                return Err("Could not open a window.".into());
            }
            // Known before the plugin attaches: it may ask to be resized at once.
            self.window.store(hwnd as usize, Relaxed);
            let scale = f64::from(unsafe { GetDpiForWindow(hwnd) }) / 96.0;
            let (w, h) = match plugin.embed(hwnd, scale) {
                Ok(size) => size,
                Err(e) => {
                    self.window.store(0, Relaxed);
                    unsafe { DestroyWindow(hwnd) };
                    return Err(e);
                }
            };
            if !plugin.can_resize() {
                unsafe { SetWindowLongPtrW(hwnd, GWL_STYLE, (STYLE & !WS_THICKFRAME) as isize) };
            }
            fit_window(hwnd, w, h);
            unsafe {
                ShowWindow(hwnd, SW_SHOW);
                SetForegroundWindow(hwnd);
            }
            Ok(())
        } else if plugin.floats() {
            plugin.float(title)?;
            self.floating = true;
            Ok(())
        } else {
            Err("This plugin has no window Soniq can show.".into())
        }
    }

    fn close_window(&mut self) {
        let hwnd = self.window.swap(0, Relaxed) as HWND;
        if !hwnd.is_null() || std::mem::take(&mut self.floating) {
            if let Some(p) = self.plugin.as_mut() {
                p.unembed();
            }
            if !hwnd.is_null() {
                unsafe { DestroyWindow(hwnd) };
            }
        }
    }
}

/// Answers the app's blocks until told to stop.
fn audio_thread(shared: &Shared, slot: &Mutex<Option<Processor>>, host: &HostState, changes: &Sender<OutEvent>, quit: &AtomicBool) {
    unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_TIME_CRITICAL) };
    mark_audio_thread(host);
    while !quit.load(Relaxed) {
        if !shared.wait_go(50) {
            continue;
        }
        let block = shared.block();
        let request = unsafe { (*block).request.load(Acquire) };
        if request == unsafe { (*block).done.load(Acquire) } {
            continue;
        }
        let mut changed = false;
        if let Ok(mut guard) = slot.lock() {
            match guard.as_mut() {
                Some(processor) => {
                    unsafe { processor.process(block) };
                    processor.drain_out(|e| {
                        changed = true;
                        let _ = changes.send(e);
                    });
                }
                None => unsafe {
                    (*block).output[0].fill(0.0);
                    (*block).output[1].fill(0.0);
                },
            }
        }
        unsafe { (*block).done.store(request, Release) };
        shared.signal_done();
        if changed {
            host.hooks.wake();
        }
    }
    if let Ok(mut guard) = slot.lock()
        && let Some(processor) = guard.as_mut()
    {
        processor.stop();
    }
}

fn register(name: &str, proc_: WNDPROC) {
    let class = wide(name);
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: 0,
        lpfnWndProc: proc_,
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: unsafe { GetModuleHandleW(ptr::null()) },
        hIcon: ptr::null_mut(),
        hCursor: unsafe { LoadCursorW(ptr::null_mut(), IDC_ARROW) },
        hbrBackground: ptr::null_mut(),
        lpszMenuName: ptr::null(),
        lpszClassName: class.as_ptr(),
        hIconSm: ptr::null_mut(),
    };
    unsafe { RegisterClassExW(&wc) };
}

fn host(out: File) -> i32 {
    unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    register("SoniqPluginMessages", Some(message_proc));
    register("SoniqPluginWindow", Some(window_proc));
    let class = wide("SoniqPluginMessages");
    let messages = unsafe {
        CreateWindowExW(0, class.as_ptr(), ptr::null(), 0, 0, 0, 0, 0, HWND_MESSAGE, ptr::null_mut(), GetModuleHandleW(ptr::null()), ptr::null())
    };
    if messages.is_null() {
        return 1;
    }

    // Commands come in on stdin; a closed pipe means the app is gone.
    let inbox: Inbox = Arc::default();
    let (queue, target) = (Arc::clone(&inbox), messages as usize);
    std::thread::spawn(move || {
        let mut input = io::stdin().lock();
        loop {
            let command = read_frame::<Command>(&mut input).unwrap_or((Command::Quit, Vec::new()));
            let quit = command.0 == Command::Quit;
            if let Ok(mut q) = queue.lock() {
                q.push_back(command);
            }
            unsafe { PostMessageW(target as HWND, WAKE, 0, 0) };
            if quit {
                break;
            }
        }
    });

    SANDBOX.set(Some(Sandbox {
        out,
        inbox,
        messages,
        window: Arc::default(),
        floating: false,
        shared: None,
        host: None,
        processor: Arc::default(),
        audio: None,
        plugin: None,
        rate: 48_000.0,
        changes: channel(),
    }));
    let mut msg: MSG = unsafe { std::mem::zeroed() };
    while unsafe { GetMessageW(&mut msg, ptr::null_mut(), 0, 0) } > 0 {
        unsafe {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    SANDBOX.with_borrow_mut(|s| s.take());
    let _ = io::stderr().flush();
    0
}
