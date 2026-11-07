//! VST3 through the sandbox: the two test plugins from `examples/test_vst3.rs`.
#![cfg(windows)]

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use soniq_dsp::{BlockEvent, External, FxContext, NoteEvent, ParamBank};
use soniq_host::{Gesture, Message, Plugin, SandboxCommand, scan};

const RATE: f64 = 48_000.0;

fn sandbox() -> SandboxCommand {
    SandboxCommand { program: PathBuf::from(env!("CARGO_BIN_EXE_soniq-plugin-host")), args: Vec::new() }
}

/// The test module, copied to a `.vst3` file as a host would find it.
fn module() -> PathBuf {
    static FILE: OnceLock<PathBuf> = OnceLock::new();
    FILE.get_or_init(|| {
        let bin = PathBuf::from(env!("CARGO_BIN_EXE_soniq-plugin-host"));
        let built = bin.parent().unwrap().join("examples").join("test_vst3.dll");
        assert!(built.exists(), "build the test plugins first (cargo test builds examples): {}", built.display());
        let file = std::env::temp_dir().join(format!("soniq-test-{}.vst3", std::process::id()));
        std::fs::copy(&built, &file).unwrap();
        file
    })
    .clone()
}

/// The id of the module's plugin called `name`.
fn id(name: &str) -> String {
    scan(&sandbox(), &module(), Duration::from_secs(10)).unwrap().into_iter().find(|d| d.name == name).unwrap().id
}

fn spawn(name: &str) -> (Plugin, Receiver<Message>) {
    let (tx, rx) = channel();
    let tx = Mutex::new(tx);
    let plugin = Plugin::spawn(&sandbox(), &module(), &id(name), RATE, move |m| {
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

/// 0–1 for a gain of `db` on the delay (−60…+12 dB).
fn gain(db: f64) -> f64 {
    (db + 60.0) / 72.0
}

fn block(fx: &mut dyn External, frames: usize) -> Vec<f32> {
    let (mut l, mut r) = (vec![0.5f32; frames], vec![0.5f32; frames]);
    fx.process(&ctx(), &mut l, &mut r, &[]);
    l
}

#[test]
fn a_scan_lists_a_modules_plugins() {
    let plugins = scan(&sandbox(), &module(), Duration::from_secs(10)).unwrap();
    let found: Vec<_> = plugins.iter().map(|p| (p.name.as_str(), p.is_instrument(), p.vendor.as_str(), p.id.len())).collect();
    assert_eq!(found, [("Soniq Test Delay", false, "Soniq", 32), ("Soniq Test VST Synth", true, "Soniq", 32)]);
    assert_eq!(plugins[0].description, "Fx|Delay");
}

#[test]
fn an_effect_processes_and_reports_its_latency() {
    let (plugin, _) = spawn("Soniq Test Delay");
    assert_eq!((plugin.info.inputs, plugin.info.outputs, plugin.info.latency, plugin.info.gui), (2, 2, 256, true));
    let params = plugin.params().unwrap();
    assert_eq!((params[0].name.as_str(), params[0].min, params[0].max, params[0].text.as_str()), ("Gain", 0.0, 1.0, "0.0 dB"));

    let mut fx = plugin.processor(&[("0".into(), 0)], false, true);
    fx.configure(&bank(&[("0", gain(-6.0))]));
    let out = block(&mut fx, 512);
    // The first 256 samples are the delay's; then the input, 6 dB down.
    assert!(out[..256].iter().all(|&s| s == 0.0));
    let expected = 0.5 * 10f32.powf(-6.0 / 20.0);
    assert!(out[256..].iter().all(|&s| (s - expected).abs() < 1e-4), "{}", out[300]);
    assert_eq!(plugin.texts(&[0]).unwrap()[0].2, "-6.0 dB");
}

#[test]
fn state_carries_over_to_a_new_instance() {
    let (first, _) = spawn("Soniq Test Delay");
    let mut fx = first.processor(&[("0".into(), 0)], false, true);
    fx.configure(&bank(&[("0", gain(-12.0))]));
    block(&mut fx, 64);
    let state = first.save_state().unwrap();
    let (second, _) = spawn("Soniq Test Delay");
    second.load_state(&state).unwrap();
    assert_eq!(second.texts(&[0]).unwrap()[0].2, "-12.0 dB");
}

#[test]
fn an_edit_in_the_editor_reaches_the_app_and_the_processor() {
    let (plugin, notices) = spawn("Soniq Test Delay");
    let mut fx = plugin.processor(&[("0".into(), 0)], false, true);
    fx.configure(&bank(&[("0", gain(0.0))]));
    block(&mut fx, 64);
    // The test editor turns the gain up by 1 dB as it opens.
    plugin.show_gui("Soniq Test Delay").unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut seen = Vec::new();
    while Instant::now() < deadline {
        if let Ok(Message::ParamChanged { id, value, gesture }) = notices.recv_timeout(Duration::from_millis(100)) {
            seen.push((id, gesture, value.map(|v| (v * 1000.0).round() / 1000.0)));
            if gesture == Gesture::End {
                break;
            }
        }
    }
    let raised = (gain(1.0) * 1000.0).round() / 1000.0;
    assert_eq!(seen, [(0, Gesture::Begin, None), (0, Gesture::None, Some(raised)), (0, Gesture::End, None)]);
    // The processor hears it with the next block.
    let out = block(&mut fx, 512);
    let expected = 0.5 * 10f32.powf(1.0 / 20.0);
    assert!((out[400] - expected).abs() < 1e-3, "{}", out[400]);
    plugin.hide_gui().unwrap();
}

#[test]
fn an_instrument_plays_the_notes_it_is_given() {
    let (plugin, _) = spawn("Soniq Test VST Synth");
    assert_eq!((plugin.info.inputs, plugin.info.notes, plugin.info.gui), (0, true, false));
    let mut synth = plugin.processor(&[("0".into(), 0)], true, true);
    synth.configure(&bank(&[("0", 1.0 - 6.0 / 60.0)]));
    let (mut l, mut r) = (vec![0.0f32; 2048], vec![0.0f32; 2048]);
    let notes = [BlockEvent { at: 100, event: NoteEvent::On { pitch: 69, velocity: 1.0 } }];
    synth.process(&ctx(), &mut l, &mut r, &notes);
    assert!(l[..100].iter().all(|&s| s == 0.0));
    let peak = l.iter().fold(0f32, |m, s| m.max(s.abs()));
    assert!((peak - 0.501).abs() < 0.01, "{peak}");
    let crossings = (101..2048).filter(|&i| l[i - 1] <= 0.0 && l[i] > 0.0).count();
    let hz = crossings as f64 * RATE / 1948.0;
    assert!((hz - 440.0).abs() < 30.0, "{hz}");
}
