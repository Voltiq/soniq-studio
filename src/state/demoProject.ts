/*
 * The browser preview's demo song, mirroring src-tauri/src/demo.rs: drums,
 * bass and keys are MIDI, the pad is audio. Its waveform is drawn from
 * synthesised peak data so the UI can be designed without the engine.
 */
import { createDevice } from "./devices";
import type { Asset, AudioClip, MidiClip, Note, PeakData, Project, Track } from "./types";

const STEP = 256;
const RATE = 48_000;
const BPM = 112;

function rng(seed: number) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

type Shape = "drums" | "bass" | "pad";

function fakePeaks(beats: number, shape: Shape, seed: number): PeakData {
  const seconds = (beats * 60) / BPM;
  const n = Math.round((seconds * RATE) / STEP);
  const left = new Float32Array(n);
  const right = new Float32Array(n);
  const rand = rng(seed);
  for (let i = 0; i < n; i++) {
    const beat = ((i * STEP) / RATE) * (BPM / 60);
    const inBeat = beat % 1;
    let a: number;
    if (shape === "drums") a = 0.15 + 0.65 * Math.exp(-(beat % 0.5) * 9) * (inBeat < 0.5 ? 1 : 0.6);
    else if (shape === "bass") a = 0.25 + 0.35 * Math.exp(-(beat % 1) * 3);
    else a = 0.18 + 0.12 * Math.sin((beat / 4) * Math.PI);
    a *= 0.85 + 0.2 * rand();
    left[i] = Math.min(1, a);
    right[i] = Math.min(1, a * (0.9 + 0.1 * rand()));
  }
  return { channels: [left, right], step: STEP, sampleRate: RATE };
}

let nextId = 1;
const id = (prefix: string) => `${prefix}-preview-${nextId++}`;

function asset(name: string, data: PeakData): Asset {
  return { id: id("a"), name, channels: 2, sampleRate: RATE, frames: data.channels[0].length * STEP };
}

function audioClip(name: string, a: Asset, start: number, offsetBeats: number, beats: number): AudioClip {
  const sec = 60 / BPM;
  return { id: id("c"), name, kind: "audio", start, asset: a.id, offset: offsetBeats * sec, duration: beats * sec, gain: 0, fadeIn: 0, fadeOut: 0 };
}

function midiClip(name: string, start: number, length: number, notes: Note[]): MidiClip {
  return { id: id("c"), name, kind: "midi", start, length, notes };
}

const CHORDS = [
  [57, 60, 64],
  [53, 57, 60],
  [55, 60, 64],
  [55, 59, 62],
];

function chords(bars: number, lift = 0): Note[] {
  const notes: Note[] = [];
  for (let bar = 0; bar < bars; bar++) {
    CHORDS[bar % 4].forEach((pitch) => notes.push({ start: bar * 4, length: 3.75, pitch: pitch + lift, velocity: 0.7 }));
  }
  return notes;
}

/** The same groove as src-tauri/src/demo.rs: kick, snare, hats on the GM drum map. */
function drumNotes(from: number, bars: number): Note[] {
  const notes: Note[] = [];
  for (let bar = from; bar < from + bars; bar++) {
    const o = (bar - from) * 4;
    const intro = bar < 4;
    for (const k of intro ? [0, 2] : [0, 1.5, 2, 3.5]) notes.push({ start: o + k, length: 0.25, pitch: 36, velocity: 0.95 });
    if (!intro) for (const sn of [1, 3]) notes.push({ start: o + sn, length: 0.25, pitch: 38, velocity: 0.85 });
    for (let h = 0; h < 8; h++) notes.push({ start: o + h * 0.5, length: 0.2, pitch: 42, velocity: h % 2 ? 0.45 : 0.7 });
    if (bar % 4 === 3 && !intro) notes.push({ start: o + 3.5, length: 0.5, pitch: 46, velocity: 0.6 });
  }
  return notes.sort((a, b) => a.start - b.start || a.pitch - b.pitch);
}

const ROOTS = [45, 41, 48, 43];

function bassNotes(bars: number): Note[] {
  const notes: Note[] = [];
  for (let bar = 0; bar < bars; bar++) {
    const o = bar * 4;
    const root = ROOTS[bar % 4];
    notes.push(
      { start: o, length: 0.75, pitch: root, velocity: 0.9 },
      { start: o + 0.75, length: 0.25, pitch: root + 12, velocity: 0.6 },
      { start: o + 1.5, length: 0.5, pitch: root, velocity: 0.75 },
      { start: o + 2.5, length: 0.5, pitch: root + 7, velocity: 0.7 },
      { start: o + 3, length: 0.75, pitch: root, velocity: 0.8 },
    );
  }
  return notes;
}

function track(partial: Partial<Track> & Pick<Track, "name" | "kind" | "color">): Track {
  return {
    id: id("t"),
    clips: [],
    volume: 0,
    pan: 0,
    mute: false,
    solo: false,
    arm: false,
    devices: [],
    sends: [0, 0],
    input: "1",
    monitor: "off",
    ...partial,
  };
}

/** Copies of the named timeline clips into the session's slots, from their own start (mirrors `Project::migrate_slots`). */
function withSlots(t: Track, names: (string | null)[]): Track {
  const copies = names.flatMap((name, scene) => {
    const clip = t.clips.find((c) => c.name === name);
    return clip ? [{ ...clip, id: id("c"), start: 0, slot: scene, ...(clip.kind === "midi" ? { link: undefined } : {}) }] : [];
  });
  return { ...t, clips: [...t.clips, ...copies] };
}

export function createDemoProject(): { project: Project; peaks: Map<string, PeakData> } {
  const peaks = new Map<string, PeakData>();
  const make = (name: string, beats: number, shape: Shape, seed: number) => {
    const data = fakePeaks(beats, shape, seed);
    const a = asset(name, data);
    peaks.set(a.id, data);
    return a;
  };
  const pad = make("Warm pad", 96, "pad", 11);

  const tracks: Track[] = [
    withSlots(
      track({
        name: "Drums",
        kind: "midi",
        color: 2,
        volume: -6,
        clips: [midiClip("Intro", 0, 16, drumNotes(0, 4)), midiClip("Groove", 16, 80, drumNotes(4, 20))],
        devices: [createDevice("drum-rack"), createDevice("comp", { thresh: -14, ratio: 3 })],
        sends: [0.12, 0],
      }),
      ["Intro", "Groove", "Groove", null, null],
    ),
    withSlots(
      track({
        name: "Bass",
        kind: "midi",
        color: 1,
        volume: -4,
        clips: [midiClip("Bassline", 16, 80, bassNotes(20))],
        devices: [
          createDevice("synth", { osc2: 1, osc2Semi: -12, detune: 0, mix: 35, sub: 30, cutoff: 420, res: 28, envAmt: 55, fDecay: 180, fSustain: 10, mode: 1 }),
          createDevice("saturator", { drive: 9 }),
        ],
      }),
      [null, "Bassline", "Bassline", null, null],
    ),
    withSlots(
      track({
        name: "Keys",
        kind: "midi",
        color: 6,
        volume: -8,
        pan: -0.2,
        clips: [midiClip("Chords", 0, 32, chords(8)), midiClip("Chords", 32, 32, chords(8)), midiClip("Chords high", 64, 32, chords(8, 12))],
        devices: [createDevice("epiano"), createDevice("reverb", { mix: 18 })],
        sends: [0.3, 0.15],
      }),
      ["Chords", "Chords", "Chords high", null, null],
    ),
    withSlots(
      track({
        name: "Pad",
        kind: "audio",
        color: 3,
        volume: -9,
        clips: [audioClip("Warm pad", pad, 0, 0, 96)],
        devices: [createDevice("eq", { b2freq: 150, b2gain: -6 }), createDevice("reverb", { size: 80, mix: 40 })],
        sends: [0.25, 0],
      }),
      ["Warm pad", "Warm pad", null, null, "Warm pad"],
    ),
    track({
      name: "Vocals",
      kind: "audio",
      color: 5,
      devices: [createDevice("eq", { b1on: 1, b1freq: 80, b2gain: -6, b7gain: 2 }), createDevice("comp")],
    }),
  ];

  return {
    peaks,
    project: {
      version: 1,
      id: id("p"),
      name: "Night Drive (demo)",
      bpm: BPM,
      beatsPerBar: 4,
      key: "A minor",
      tracks,
      returns: [
        { id: id("r"), name: "A · Reverb", volume: -6, pan: 0, mute: false, preFader: false, devices: [createDevice("reverb", { mix: 100 })] },
        { id: id("r"), name: "B · Delay", volume: -9, pan: 0, mute: false, preFader: false, devices: [createDevice("delay", { mix: 100 })] },
      ],
      scenes: ["Intro", "Verse", "Hook", "Break", "Outro"],
      masterVolume: 0,
      assets: [pad],
      markers: [
        { id: "m-intro", name: "Intro", start: 0, end: 16 },
        { id: "m-drop", name: "Drop", start: 64 },
      ],
      launchQuantize: 4,
    },
  };
}

export function emptyTrack(kind: "audio" | "midi" | "group", name: string, color: number): Track {
  return track({ name, kind, color, devices: kind === "midi" ? [createDevice("synth")] : [] });
}

export function newId(prefix: string) {
  return id(prefix);
}
