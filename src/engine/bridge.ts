/*
 * The audio engine: transport, devices and meters. Outside Tauri (plain
 * `npm run dev` in a browser) every call becomes a no-op and the UI runs on
 * its own clock. Project changes go through ./backend.ts instead.
 */
import { Channel, invoke } from "@tauri-apps/api/core";
import type { MappingTarget, MidiPortStatus } from "../state/types";

export interface EngineStatus {
  running: boolean;
  host: string;
  device: string;
  sampleRate: number;
  channels: number;
  bufferFrames: number;
  /** From rendering to hearing, as the driver reports it. */
  outputLatencyMs: number;
  error: string | null;
  /** Cores the mix is spread over (the audio thread and its workers). */
  cores?: number;
}

export interface DeviceInfo {
  host: string;
  name: string;
  direction: "input" | "output";
  isDefault: boolean;
  sampleRate: number | null;
  channels: number | null;
}

export interface DeviceOptions {
  sampleRates: number[];
  defaultSampleRate: number;
  bufferSizes: number[];
}

/** `null` everywhere means "system default". */
export interface EngineConfig {
  host: string | null;
  output: string | null;
  input: string | null;
  sampleRate: number | null;
  bufferFrames: number | null;
  /** Added to the measured latency when placing recordings, in ms. */
  recordOffsetMs: number;
  /** Cores to spread the mix over; null uses all but one (up to eight). */
  cores?: number | null;
  /** The safety limiter before the speakers is switched off. */
  safetyLimiterOff?: boolean;
}

/** What the device watch reports (event "audio-device"). */
export type DeviceEvent =
  | { kind: "lost"; previous: string; device: string; stopped: boolean }
  | { kind: "switched"; previous: string; device: string; stopped: boolean }
  | { kind: "noOutput"; error: string; stopped: boolean }
  | { kind: "inputLost"; device: string }
  | { kind: "inputBack"; device: string };

export interface InputStatus {
  open: boolean;
  device: string;
  sampleRate: number;
  channels: number;
  /** From the sound reaching the input to the app, as the driver reports it. */
  latencyMs: number;
  error: string | null;
}

export interface MeterFrame {
  position: number;
  playing: boolean;
  tracks: [string, [number, number]][];
  master: [number, number];
  /** The audio input, while a track is armed. */
  input: [number, number];
  countingIn: boolean;
  /** An effect preview is playing. */
  previewing: boolean;
  /** Counts live notes; a change means MIDI came in. */
  midi: number;
  /** Changes when a MIDI controller changed the project: time to fetch it. */
  revision: number;
  /** What each track's session plays, for tracks where that is anything. */
  clips: [string, ClipState][];
  /** The master's loudness, peaks and spectrum, while the meters panel is open. */
  analysis?: AnalysisReading | null;
  /** How far the safety limiter turned the master down since the last frame, in dB. */
  limiter?: number;
  /** A block of NaN or infinity (a broken plugin) was silenced. */
  silenced?: boolean;
}

/** The master measured as it plays (mirrors `soniq_io::Reading`). */
export interface AnalysisReading {
  /** LUFS over 400 ms, 3 s and since the reset; null below −70. */
  momentary: number | null;
  shortTerm: number | null;
  integrated: number | null;
  /** Loudness range since the reset, LU. */
  range: number | null;
  /** Highest true peak since the reset, dBTP. */
  truePeak: number | null;
  /** True peak per channel since the last reading, dBTP. */
  peak: [number | null, number | null];
  /** RMS per channel over 300 ms, dBFS. */
  rms: [number | null, number | null];
  /** −1 … +1 over 300 ms; null in silence. */
  correlation: number | null;
  /** dB per sixth-octave band from 20 Hz. */
  spectrum: number[];
}

/** What a track's session plays (mirrors `soniq_engine::ClipState`). */
export interface ClipState {
  /** The scene whose clip plays. */
  playing: number | null;
  /** The scene whose clip starts on the next launch line. */
  queued: number | null;
  /** A stop waits for the next launch line. */
  stopping: boolean;
  /** How far through its loop the playing clip is, 0–1. */
  progress: number;
  /** The track plays the arrangement. */
  arrangement: boolean;
}

export const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

function send(command: string, args: Record<string, unknown>) {
  if (!inTauri) return;
  invoke(command, args).catch((err) => console.error(`${command} failed`, err));
}

export const engine = {
  available: inTauri,
  status: () => (inTauri ? invoke<EngineStatus>("engine_status") : Promise.resolve(null)),
  config: () => (inTauri ? invoke<EngineConfig>("engine_config") : Promise.resolve(null)),
  configure: (config: EngineConfig) => invoke<EngineStatus>("engine_configure", { config }),
  /** The safety limiter before the speakers, on or off at once. */
  safety: (on: boolean) => (inTauri ? invoke<void>("engine_safety", { on }) : Promise.resolve()),
  devices: () => (inTauri ? invoke<DeviceInfo[]>("audio_devices") : Promise.resolve([] as DeviceInfo[])),
  deviceOptions: (host: string | null, output: string | null) => invoke<DeviceOptions>("audio_device_options", { host, output }),
  inputStatus: () => (inTauri ? invoke<InputStatus>("input_status") : Promise.resolve(null)),
  play: (playing: boolean) => send("transport_play", { playing }),
  seek: (seconds: number) => send("transport_seek", { seconds }),
  setMetronome: (enabled: boolean) => send("transport_set_metronome", { enabled }),
  setLoop: (enabled: boolean, start: number, end: number) => send("transport_set_loop", { enabled, start, end }),
  /** A live note: on `track` only (auditioning), or with `null` wherever a MIDI keyboard plays (and recorded). */
  note: (track: string | null, pitch: number, velocity: number, on: boolean) => send("note_play", { track, pitch, velocity, on }),
  sustain: (down: boolean) => send("note_sustain", { down }),
  panic: () => send("note_panic", {}),
  /** The selected track plays live notes while no MIDI track is armed. */
  midiSelect: (track: string | null) => send("midi_select", { track }),
  midiInputs: () => (inTauri ? invoke<MidiPortStatus[]>("midi_inputs") : Promise.resolve([] as MidiPortStatus[])),
  midiInputEnable: (name: string, enabled: boolean) => invoke<MidiPortStatus[]>("midi_input_enable", { name, enabled }),
  /** Maps the next MIDI controller that moves to `target`; null stops learning. */
  midiLearn: (target: MappingTarget | null) => send("midi_learn", { target }),
  /** Starts the clip in a scene's slot (null stops the track) on the next launch line; launching starts the transport. */
  launch: (track: string, scene: number | null) => (inTauri ? invoke<void>("session_launch", { track, scene }) : Promise.resolve()),
  launchScene: (scene: number) => (inTauri ? invoke<void>("session_launch_scene", { scene }) : Promise.resolve()),
  stopClips: () => (inTauri ? invoke<void>("session_stop_clips") : Promise.resolve()),
  backToArrangement: () => (inTauri ? invoke<void>("session_back_to_arrangement") : Promise.resolve()),
  /** Measures the master (loudness, peaks, spectrum) for the meter frames, or stops. */
  analysis: (on: boolean) => (inTauri ? invoke<void>("analysis_enable", { on }) : Promise.resolve()),
  /** Starts the integrated loudness, range and highest true peak over. */
  analysisReset: () => (inTauri ? invoke<void>("analysis_reset") : Promise.resolve()),
  /** Streams meters and the playhead about 30 times a second. */
  subscribeMeters: (onFrame: (frame: MeterFrame) => void) => {
    if (!inTauri) return;
    const channel = new Channel<MeterFrame>();
    channel.onmessage = onFrame;
    invoke("meters_subscribe", { channel }).catch((err) => console.error("meters_subscribe failed", err));
  },
};
