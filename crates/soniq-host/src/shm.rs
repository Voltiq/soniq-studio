//! The memory a plugin's sandbox shares with the app: one block of audio,
//! notes and parameter changes, handed back and forth with two events. The
//! app fills the block, bumps `request` and signals "go"; the sandbox
//! processes, sets `done` to the same number and signals "done". A number
//! that does not match is an old answer, so a late reply is never mistaken
//! for the current one.

use std::sync::atomic::AtomicU32;

/// Most frames in a block (the engine's largest block).
pub const MAX_FRAMES: usize = 2048;
/// Most notes and parameter changes in a block.
pub const MAX_EVENTS: usize = 1024;

/// A note or parameter change `at` frames into the block.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Event {
    pub at: u32,
    pub kind: u32,
    /// The key of a note, or the parameter's id.
    pub id: u32,
    /// Velocity 0–1, the parameter's value, or the pedal (1 down).
    pub value: f64,
}

pub mod kind {
    pub const NOTE_ON: u32 = 1;
    pub const NOTE_OFF: u32 = 2;
    pub const PARAM: u32 = 3;
    pub const SUSTAIN: u32 = 4;
    pub const ALL_NOTES_OFF: u32 = 5;
    pub const ALL_SOUND_OFF: u32 = 6;
}

/// Where the block sits in the song.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct Transport {
    pub playing: u32,
    pub tempo: f64,
    pub beat: f64,
    pub seconds: f64,
}

/// Flag: start afresh (tails and voices cleared) before this block.
pub const RESET: u32 = 1;

#[repr(C)]
pub struct Block {
    pub request: AtomicU32,
    pub done: AtomicU32,
    pub frames: u32,
    pub flags: u32,
    pub transport: Transport,
    pub event_count: u32,
    pub events: [Event; MAX_EVENTS],
    pub input: [[f32; MAX_FRAMES]; 2],
    pub output: [[f32; MAX_FRAMES]; 2],
}

#[cfg(windows)]
pub use windows::{Shared, Watch};

#[cfg(windows)]
mod windows {
    use std::io;
    use std::ptr;

    use windows_sys::Win32::Foundation::{CloseHandle, DUPLICATE_SAME_ACCESS, DuplicateHandle, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0, WAIT_TIMEOUT};
    use windows_sys::Win32::System::Memory::{
        CreateFileMappingW, FILE_MAP_ALL_ACCESS, MEMORY_MAPPED_VIEW_ADDRESS, MapViewOfFile, OpenFileMappingW, PAGE_READWRITE, UnmapViewOfFile,
    };
    use windows_sys::Win32::System::Threading::{CreateEventW, EVENT_ALL_ACCESS, GetCurrentProcess, OpenEventW, SetEvent, WaitForSingleObject};

    use super::Block;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    fn checked(handle: HANDLE, what: &str) -> io::Result<HANDLE> {
        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            let e = io::Error::last_os_error();
            Err(io::Error::new(e.kind(), format!("{what}: {e}")))
        } else {
            Ok(handle)
        }
    }

    /// The shared block and its two events.
    pub struct Shared {
        map: HANDLE,
        view: *mut Block,
        go: HANDLE,
        done: HANDLE,
    }

    // The block is only touched by whoever holds the turn (see the module notes).
    unsafe impl Send for Shared {}
    unsafe impl Sync for Shared {}

    impl Shared {
        fn names(name: &str) -> [Vec<u16>; 3] {
            [wide(&format!("Local\\{name}-block")), wide(&format!("Local\\{name}-go")), wide(&format!("Local\\{name}-done"))]
        }

        /// Makes the block (the app's side), zeroed.
        pub fn create(name: &str) -> io::Result<Self> {
            let [map_name, go_name, done_name] = Self::names(name);
            let size = std::mem::size_of::<Block>();
            unsafe {
                let map = checked(CreateFileMappingW(INVALID_HANDLE_VALUE, ptr::null(), PAGE_READWRITE, 0, size as u32, map_name.as_ptr()), "shared memory")?;
                let shared = Self::finish(map, CreateEventW(ptr::null(), 0, 0, go_name.as_ptr()), CreateEventW(ptr::null(), 0, 0, done_name.as_ptr()))?;
                Ok(shared)
            }
        }

        /// Opens the block the app made (the sandbox's side).
        pub fn open(name: &str) -> io::Result<Self> {
            let [map_name, go_name, done_name] = Self::names(name);
            unsafe {
                let map = checked(OpenFileMappingW(FILE_MAP_ALL_ACCESS, 0, map_name.as_ptr()), "shared memory")?;
                Self::finish(map, OpenEventW(EVENT_ALL_ACCESS, 0, go_name.as_ptr()), OpenEventW(EVENT_ALL_ACCESS, 0, done_name.as_ptr()))
            }
        }

        unsafe fn finish(map: HANDLE, go: HANDLE, done: HANDLE) -> io::Result<Self> {
            let close = |h: HANDLE| unsafe {
                if !h.is_null() {
                    CloseHandle(h);
                }
            };
            let view = unsafe { MapViewOfFile(map, FILE_MAP_ALL_ACCESS, 0, 0, std::mem::size_of::<Block>()) }.Value as *mut Block;
            if view.is_null() || go.is_null() || done.is_null() {
                let e = io::Error::last_os_error();
                if !view.is_null() {
                    unsafe { UnmapViewOfFile(MEMORY_MAPPED_VIEW_ADDRESS { Value: view.cast() }) };
                }
                [map, go, done].into_iter().for_each(close);
                return Err(io::Error::new(e.kind(), format!("shared memory: {e}")));
            }
            Ok(Self { map, view, go, done })
        }

        pub fn block(&self) -> *mut Block {
            self.view
        }

        pub fn signal_go(&self) {
            unsafe { SetEvent(self.go) };
        }

        pub fn signal_done(&self) {
            unsafe { SetEvent(self.done) };
        }

        /// Waits up to `ms` for "go"; whether it came.
        pub fn wait_go(&self, ms: u32) -> bool {
            unsafe { WaitForSingleObject(self.go, ms) == WAIT_OBJECT_0 }
        }

        pub fn wait_done(&self, ms: u32) -> bool {
            unsafe { WaitForSingleObject(self.done, ms) == WAIT_OBJECT_0 }
        }
    }

    impl Drop for Shared {
        fn drop(&mut self) {
            unsafe {
                UnmapViewOfFile(MEMORY_MAPPED_VIEW_ADDRESS { Value: self.view.cast() });
                CloseHandle(self.go);
                CloseHandle(self.done);
                CloseHandle(self.map);
            }
        }
    }

    /// Tells whether a process is still running, cheaply enough for the audio thread.
    pub struct Watch(HANDLE);

    unsafe impl Send for Watch {}
    unsafe impl Sync for Watch {}

    impl Watch {
        /// Watches a child process (with a handle of its own).
        pub fn new(child: &std::process::Child) -> io::Result<Self> {
            use std::os::windows::io::AsRawHandle;
            let handle = child.as_raw_handle();
            let mut own = ptr::null_mut();
            let me = unsafe { GetCurrentProcess() };
            if unsafe { DuplicateHandle(me, handle, me, &mut own, 0, 0, DUPLICATE_SAME_ACCESS) } == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(Self(own))
        }

        pub fn alive(&self) -> bool {
            unsafe { WaitForSingleObject(self.0, 0) == WAIT_TIMEOUT }
        }
    }

    impl Drop for Watch {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.0) };
        }
    }
}

#[cfg(not(windows))]
pub use unsupported::{Shared, Watch};

#[cfg(not(windows))]
mod unsupported {
    use std::io;

    use super::Block;

    fn unsupported<T>() -> io::Result<T> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "plugins run on Windows only for now"))
    }

    pub struct Shared;

    impl Shared {
        pub fn create(_: &str) -> io::Result<Self> {
            unsupported()
        }

        pub fn open(_: &str) -> io::Result<Self> {
            unsupported()
        }

        pub fn block(&self) -> *mut Block {
            std::ptr::null_mut()
        }

        pub fn signal_go(&self) {}

        pub fn signal_done(&self) {}

        pub fn wait_go(&self, _: u32) -> bool {
            false
        }

        pub fn wait_done(&self, _: u32) -> bool {
            false
        }
    }

    pub struct Watch;

    impl Watch {
        pub fn alive(&self) -> bool {
            false
        }
    }
}

/// A name for a new shared block, unique to this process.
pub fn unique_name() -> String {
    use std::sync::atomic::Ordering::Relaxed;
    static NEXT: AtomicU32 = AtomicU32::new(0);
    format!("soniq-plugin-{}-{}", std::process::id(), NEXT.fetch_add(1, Relaxed))
}

