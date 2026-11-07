//! The sandbox end to end: real processes, real shared memory, and the two
//! test plugins from `examples/test_plugins.rs`.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use soniq_dsp::{BlockEvent, External, FxContext, NoteEvent, ParamBank};
use soniq_host::{Gesture, Message, Plugin, SandboxCommand, scan};

const RATE: f64 = 48_000.0;
const GAIN: &str = "studio.soniq.test.gain";
const SYNTH: &str = "studio.soniq.test.synth";

fn sandbox() -> SandboxCommand {
    SandboxCommand { program: PathBuf::from(env!("CARGO_BIN_EXE_soniq-plugin-host")), args: Vec::new() }
}

/// The test plugins, copied to a `.clap` file as a host would find them.
fn plugin_file() -> PathBuf {
    static FILE: OnceLock<PathBuf> = OnceLock::new();
    FILE.get_or_init(|| {
        let bin = PathBuf::from(env!("CARGO_BIN_EXE_soniq-plugin-host"));
        let built = bin.parent().unwrap().join("examples").join(if cfg!(windows) { "test_plugins.dll" } else { "libtest_plugins.so" });
        assert!(built.exists(), "build the test plugins first (cargo test builds examples): {}", built.display());
        let file = std::env::temp_dir().join(format!("soniq-test-{}.clap", std::process::id()));
        std::fs::copy(&built, &file).unwrap();
        file
    })
    .clone()
}

fn spawn(id: &str) -> (Plugin, Receiver<Message>) {
    let (tx, rx) = channel();
    let tx = Mutex::new(tx);
    let plugin = Plugin::spawn(&sandbox(), &plugin_file(), id, RATE, move |m| {
        let _ = tx.lock().unwrap().send(m);
    })
    .unwrap();
    (plugin, rx)
}

fn ctx() -> FxContext {
    FxContext { playing: true, pos: 0, beat: 0.0, samples_per_beat: RATE / 2.0, sample_rate: RATE as f32 }
}

fn bank(values: &[(&str, f64)]) -> ParamBank {
    ParamBank::new(values.iter().map(|(k, v)| (k.to_string(), *v)))
}

/// Runs one block of 0.5 through an effect.
fn block(fx: &mut dyn External) -> Vec<f32> {
    let (mut l, mut r) = (vec![0.5f32; 256], vec![0.5f32; 256]);
    fx.process(&ctx(), &mut l, &mut r, &[]);
    l
}

#[test]
fn a_scan_lists_the_plugins_in_a_file() {
    let plugins = scan(&sandbox(), &plugin_file(), Duration::from_secs(10)).unwrap();
    let names: Vec<_> = plugins.iter().map(|p| (p.name.as_str(), p.is_instrument())).collect();
    assert_eq!(names, [("Soniq Test Gain", false), ("Soniq Test Synth", true)]);
    assert!(scan(&sandbox(), &std::env::temp_dir().join("no-such.clap"), Duration::from_secs(10)).is_err());
}

#[test]
fn an_effect_processes_through_the_sandbox() {
    let (plugin, _) = spawn(GAIN);
    assert_eq!((plugin.info.name.as_str(), plugin.info.inputs, plugin.info.outputs, plugin.info.gui), ("Soniq Test Gain", 2, 2, cfg!(windows)));
    let params = plugin.params().unwrap();
    assert_eq!(params.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["Gain", "Crash", "Hang", "Nudge"]);
    assert_eq!((params[0].min, params[0].max, params[0].text.as_str()), (-60.0, 12.0, "0.0 dB"));

    let mut fx = plugin.processor(&[("gain".into(), 0)], false, true);
    fx.configure(&bank(&[("gain", -6.0)]));
    let out = block(&mut fx);
    let expected = 0.5 * 10f32.powf(-6.0 / 20.0);
    assert!(out.iter().all(|s| (s - expected).abs() < 1e-6), "{}", out[0]);
    assert_eq!(plugin.texts(&[0]).unwrap(), [(0, -6.0, "-6.0 dB".to_string())]);
}

#[test]
fn state_carries_over_to_a_new_instance() {
    let (first, _) = spawn(GAIN);
    let mut fx = first.processor(&[("gain".into(), 0)], false, true);
    fx.configure(&bank(&[("gain", -12.0)]));
    block(&mut fx);
    let state = first.save_state().unwrap();

    let (second, _) = spawn(GAIN);
    second.load_state(&state).unwrap();
    assert_eq!(second.texts(&[0]).unwrap()[0].2, "-12.0 dB");
}

#[test]
fn a_plugin_moving_its_own_knob_tells_the_app() {
    let (plugin, notices) = spawn(GAIN);
    let mut fx = plugin.processor(&[("nudge".into(), 3)], false, true);
    fx.configure(&bank(&[("nudge", 1.0)]));
    block(&mut fx);
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut seen = Vec::new();
    while Instant::now() < deadline {
        if let Ok(Message::ParamChanged { id, value, gesture }) = notices.recv_timeout(Duration::from_millis(100)) {
            seen.push((id, gesture, value));
            if gesture == Gesture::End {
                break;
            }
        }
    }
    assert_eq!(seen, [(0, Gesture::Begin, None), (0, Gesture::None, Some(1.0)), (0, Gesture::End, None)]);
    assert_eq!(plugin.texts(&[0]).unwrap()[0].2, "1.0 dB");
}

#[test]
fn a_crash_takes_only_the_plugin_down() {
    let (plugin, notices) = spawn(GAIN);
    let mut fx = plugin.processor(&[("crash".into(), 1)], false, true);
    fx.configure(&bank(&[("crash", 1.0)]));
    let started = Instant::now();
    // The block passes by untouched instead of waiting on a dead process.
    assert!(block(&mut fx).iter().all(|&s| s == 0.5));
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(notices.recv_timeout(Duration::from_secs(5)).unwrap(), Message::Exited);
    assert!(plugin.crashed());
    assert!(plugin.params().is_err());
    assert!(block(&mut fx).iter().all(|&s| s == 0.5));
}

#[test]
fn a_hanging_plugin_is_passed_by_without_stalling_the_engine() {
    let (plugin, _) = spawn(GAIN);
    let mut fx = plugin.processor(&[("gain".into(), 0), ("hang".into(), 2)], false, true);
    fx.configure(&bank(&[("gain", -6.0), ("hang", 1.0)]));
    let started = Instant::now();
    for _ in 0..10 {
        assert!(block(&mut fx).iter().all(|&s| s == 0.5));
    }
    // A 256-frame block may wait about 11 ms; after three misses blocks pass at once.
    assert!(started.elapsed() < Duration::from_millis(500), "{:?}", started.elapsed());
    assert!(!plugin.crashed());
}

#[test]
fn an_instrument_plays_the_notes_it_is_given() {
    let (plugin, _) = spawn(SYNTH);
    assert_eq!((plugin.info.inputs, plugin.info.notes), (0, true));
    let mut synth = plugin.processor(&[("level".into(), 10)], true, true);
    synth.configure(&bank(&[("level", -6.0)]));
    let (mut l, mut r) = (vec![0.0f32; 2048], vec![0.0f32; 2048]);
    let notes = [BlockEvent { at: 100, event: NoteEvent::On { pitch: 69, velocity: 1.0 } }];
    synth.process(&ctx(), &mut l, &mut r, &notes);
    assert!(l[..100].iter().all(|&s| s == 0.0), "silent before the note");
    let peak = l.iter().fold(0f32, |m, s| m.max(s.abs()));
    assert!((peak - 0.501).abs() < 0.01, "{peak}");
    let crossings = (101..2048).filter(|&i| l[i - 1] <= 0.0 && l[i] > 0.0).count();
    let hz = crossings as f64 * RATE / 1948.0;
    assert!((hz - 440.0).abs() < 30.0, "{hz}");
}

#[test]
fn an_export_notices_a_crash_at_once() {
    let (plugin, _) = spawn(GAIN);
    let mut export = plugin.processor(&[("crash".into(), 1)], false, false);
    export.configure(&bank(&[("crash", 1.0)]));
    let started = Instant::now();
    assert!(block(&mut export).iter().all(|&s| s == 0.5));
    assert!(started.elapsed() < Duration::from_secs(2), "{:?}", started.elapsed());
    assert!(plugin.crashed());
}

#[test]
fn an_export_has_the_plugin_to_itself() {
    let (plugin, _) = spawn(GAIN);
    let mut live = plugin.processor(&[("gain".into(), 0)], false, true);
    live.configure(&bank(&[("gain", -6.0)]));
    assert!(block(&mut live)[0] < 0.4);
    let mut export = plugin.processor(&[("gain".into(), 0)], false, false);
    export.configure(&bank(&[("gain", -12.0)]));
    // While the export runs, the live engine passes the plugin by.
    assert_eq!(block(&mut live)[0], 0.5);
    assert!((block(&mut export)[0] - 0.5 * 10f32.powf(-12.0 / 20.0)).abs() < 1e-6);
    drop(export);
    assert!((block(&mut live)[0] - 0.5 * 10f32.powf(-6.0 / 20.0)).abs() < 1e-6);
}
