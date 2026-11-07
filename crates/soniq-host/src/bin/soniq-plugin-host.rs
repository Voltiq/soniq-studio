//! The plugin sandbox on its own, for tests: `--scan <file>` or `--host`.
//! The app starts itself as the sandbox instead.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(soniq_host::sandbox_main(&args));
}
