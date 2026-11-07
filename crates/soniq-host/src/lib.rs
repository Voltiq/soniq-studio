//! Plugin hosting: CLAP plugins, each in a sandbox process of its own so a
//! crash or a hang never takes the app down.
//!
//! The app side ([`Plugin`]) starts a sandbox, talks to it over pipes
//! ([`protocol`]) and hands the engine a [`Processor`] that sends each block
//! through shared memory ([`shm`]). The sandbox side ([`sandbox_main`]) is
//! the same program started with `--host` (or `--scan` to list a file's
//! plugins).

mod clap;
mod client;
mod host;
mod instance;
pub mod protocol;
#[cfg(windows)]
mod sandbox;
pub mod shm;
mod vst3;

pub use client::{Plugin, Processor, SandboxCommand, scan};
pub use protocol::{Descriptor, Gesture, Loaded, Message, ParamInfo};

/// Runs the sandbox with its arguments (after the flag that selected it);
/// returns the exit code.
pub fn sandbox_main(args: &[String]) -> i32 {
    #[cfg(windows)]
    {
        sandbox::main(args)
    }
    #[cfg(not(windows))]
    {
        let _ = args;
        eprintln!("Plugins run on Windows only for now.");
        1
    }
}
