/*
 * Every offline effect and generator with its parameters, so one dialog can
 * draw controls for all of them. Ranges and defaults follow Audacity where it
 * has the same effect; the Rust side clamps again.
 */
import type { Effect, EffectType, Generator, GeneratorType, Macro } from "./types";

type Value = number | string | boolean;

export interface ParamSpec {
  id: string;
  label: string;
  kind: "number" | "select" | "toggle" | "text";
  min?: number;
  max?: number;
  step?: number;
  unit?: string;
  /** Logarithmic slider (frequencies, times). */
  log?: boolean;
  options?: [Value, string][];
  hint?: string;
}

export type EffectGroup = "Repair" | "Filter & EQ" | "Dynamics" | "Time & pitch" | "Space" | "Level";

export interface EffectSpec {
  type: EffectType;
  name: string;
  group: EffectGroup;
  description: string;
  params: ParamSpec[];
  defaults: Effect;
  /** The result can be shorter or longer than the selection. */
  changesLength?: boolean;
}

const num = (id: string, label: string, min: number, max: number, step: number, unit?: string, extra: Partial<ParamSpec> = {}): ParamSpec => ({
  id,
  label,
  kind: "number",
  min,
  max,
  step,
  unit,
  ...extra,
});

const SLOPES: [Value, string][] = [
  [12, "12 dB/oct"],
  [24, "24 dB/oct"],
  [36, "36 dB/oct"],
  [48, "48 dB/oct"],
];

export const EQ_BANDS = [31, 62, 125, 250, 500, 1000, 2000, 4000, 8000, 16000];

const LOUDNESS_TARGETS: [Value, string][] = [
  [-16, "−16 LUFS · voice, podcasts"],
  [-14, "−14 LUFS · music for streaming"],
  [-19, "−19 LUFS · a mono podcast"],
  [-23, "−23 LUFS · broadcast"],
];

export const EFFECTS: EffectSpec[] = [
  {
    type: "fixAudio",
    name: "Fix my audio",
    group: "Repair",
    description: "Listens to the recording and fixes what it finds: rumble, clicks, background noise, silence at the ends and the loudness.",
    params: [{ id: "target", label: "Loudness", kind: "select", options: LOUDNESS_TARGETS }],
    defaults: { type: "fixAudio", target: -16 },
    changesLength: true,
  },
  {
    type: "noiseReduction",
    name: "Noise reduction",
    group: "Repair",
    description: "Learns the hiss, hum or room noise from a quiet passage, then lowers it everywhere.",
    params: [
      num("reduction", "Reduction", 0, 48, 1, "dB", { hint: "How far the noise is pushed down." }),
      num("sensitivity", "Sensitivity", 0, 24, 0.5, "dB", { hint: "Higher treats more of the sound as noise." }),
      num("smoothing", "Frequency smoothing", 0, 12, 1, "bands", { hint: "Smooths the result; raise it if you hear warbling." }),
    ],
    defaults: { type: "noiseReduction", reduction: 12, sensitivity: 6, smoothing: 3 },
  },
  {
    type: "denoise",
    name: "Reduce noise (automatic)",
    group: "Repair",
    description: "Learns the hiss, hum or room noise from the quietest moments of the audio itself, then lowers it. No profile needed.",
    params: [num("reduction", "Reduction", 0, 30, 1, "dB", { hint: "How far the noise is pushed down." })],
    defaults: { type: "denoise", reduction: 12 },
  },
  {
    type: "clickRemoval",
    name: "Click removal",
    group: "Repair",
    description: "Finds short clicks and pops (vinyl crackle, mouth clicks, edits) and redraws the waveform under them.",
    params: [num("sensitivity", "Sensitivity", 1, 10, 0.5, "", { hint: "Higher finds quieter clicks." }), num("maxWidth", "Longest click", 0.1, 10, 0.1, "ms")],
    defaults: { type: "clickRemoval", sensitivity: 5, maxWidth: 1 },
  },
  {
    type: "dcOffset",
    name: "Remove DC offset",
    group: "Repair",
    description: "Centres a waveform that sits above or below the zero line, which wastes headroom and makes cuts click.",
    params: [],
    defaults: { type: "dcOffset" },
  },
  {
    type: "highPass",
    name: "High-pass filter",
    group: "Filter & EQ",
    description: "Removes rumble, wind and handling noise below a frequency.",
    params: [num("frequency", "Cutoff", 20, 2000, 1, "Hz", { log: true }), { id: "slope", label: "Slope", kind: "select", options: SLOPES }],
    defaults: { type: "highPass", frequency: 80, slope: 24 },
  },
  {
    type: "lowPass",
    name: "Low-pass filter",
    group: "Filter & EQ",
    description: "Removes hiss and harshness above a frequency.",
    params: [num("frequency", "Cutoff", 500, 20000, 10, "Hz", { log: true }), { id: "slope", label: "Slope", kind: "select", options: SLOPES }],
    defaults: { type: "lowPass", frequency: 12000, slope: 24 },
  },
  {
    type: "bassTreble",
    name: "Bass and treble",
    group: "Filter & EQ",
    description: "Gentle shelves at 250 Hz and 4 kHz, like the tone knobs on a stereo.",
    params: [num("bass", "Bass", -15, 15, 0.5, "dB"), num("treble", "Treble", -15, 15, 0.5, "dB")],
    defaults: { type: "bassTreble", bass: 0, treble: 0 },
  },
  {
    type: "graphicEq",
    name: "Graphic EQ",
    group: "Filter & EQ",
    description: "Ten bands, an octave apart.",
    params: EQ_BANDS.map((f, i) => num(`bands.${i}`, f >= 1000 ? `${f / 1000}k` : String(f), -12, 12, 0.5, "dB")),
    defaults: { type: "graphicEq", bands: EQ_BANDS.map(() => 0) },
  },
  {
    type: "compressor",
    name: "Compressor",
    group: "Dynamics",
    description: "Evens out the level: loud parts are turned down so quiet parts can come up.",
    params: [
      num("threshold", "Threshold", -60, 0, 0.5, "dB"),
      num("ratio", "Ratio", 1, 20, 0.1, ":1"),
      num("attack", "Attack", 0.1, 200, 0.1, "ms", { log: true }),
      num("release", "Release", 5, 2000, 1, "ms", { log: true }),
      { id: "makeup", label: "Make-up gain to −1 dB", kind: "toggle" },
    ],
    defaults: { type: "compressor", threshold: -20, ratio: 3, attack: 10, release: 150, makeup: true },
  },
  {
    type: "limiter",
    name: "Limiter",
    group: "Dynamics",
    description: "Stops peaks from going over a ceiling, looking ahead so nothing distorts.",
    params: [num("ceiling", "Ceiling", -12, 0, 0.1, "dBFS"), num("release", "Release", 5, 1000, 1, "ms", { log: true })],
    defaults: { type: "limiter", ceiling: -1, release: 50 },
  },
  {
    type: "truncateSilence",
    name: "Truncate silence",
    group: "Dynamics",
    description: "Shortens long pauses in speech, keeping a natural gap.",
    params: [num("threshold", "Quieter than", -80, -20, 1, "dBFS"), num("minSilence", "Longer than", 0.1, 10, 0.1, "s"), num("keep", "Shorten to", 0, 5, 0.05, "s")],
    defaults: { type: "truncateSilence", threshold: -40, minSilence: 0.5, keep: 0.3 },
    changesLength: true,
  },
  {
    type: "trimSilence",
    name: "Trim silence",
    group: "Dynamics",
    description: "Cuts the silence before the first sound and after the last, keeping a little of it.",
    params: [num("threshold", "Quieter than", -80, -20, 1, "dBFS"), num("keep", "Keep", 0, 2, 0.05, "s")],
    defaults: { type: "trimSilence", threshold: -50, keep: 0.25 },
    changesLength: true,
  },
  {
    type: "level",
    name: "Level a voice",
    group: "Dynamics",
    description: "Rides the level so quiet and loud stretches of speech come out alike, holding still through pauses so the room is not pulled up.",
    params: [num("target", "Target", -30, -10, 0.5, "LUFS"), num("maxGain", "At most", 0, 24, 1, "dB", { hint: "How far it may turn a stretch up or down." })],
    defaults: { type: "level", target: -18, maxGain: 12 },
  },
  {
    type: "changeTempo",
    name: "Change tempo",
    group: "Time & pitch",
    description: "Faster or slower without changing the pitch.",
    params: [num("percent", "Faster by", -50, 100, 0.5, "%")],
    defaults: { type: "changeTempo", percent: 10 },
    changesLength: true,
  },
  {
    type: "changePitch",
    name: "Change pitch",
    group: "Time & pitch",
    description: "Higher or lower without changing the length.",
    params: [num("semitones", "Shift", -12, 12, 0.1, "semitones")],
    defaults: { type: "changePitch", semitones: 2 },
  },
  {
    type: "changeSpeed",
    name: "Change speed",
    group: "Time & pitch",
    description: "Like a tape running faster or slower: pitch and length change together.",
    params: [num("percent", "Faster by", -50, 100, 0.5, "%")],
    defaults: { type: "changeSpeed", percent: 10 },
    changesLength: true,
  },
  {
    type: "echo",
    name: "Echo",
    group: "Space",
    description: "Repeats the sound, each repeat quieter than the last.",
    params: [num("delay", "Delay", 0.01, 3, 0.01, "s"), num("decay", "Decay", 0, 0.95, 0.01, "×")],
    defaults: { type: "echo", delay: 0.35, decay: 0.4 },
  },
  {
    type: "reverb",
    name: "Reverb",
    group: "Space",
    description: "Places the sound in a room.",
    params: [num("room", "Room size", 0, 1, 0.01), num("damping", "Damping", 0, 1, 0.01), num("mix", "Wet mix", 0, 1, 0.01)],
    defaults: { type: "reverb", room: 0.6, damping: 0.5, mix: 0.25 },
  },
  {
    type: "normalize",
    name: "Normalize",
    group: "Level",
    description: "Raises or lowers the level so the loudest peak hits a target.",
    params: [num("peak", "Peak", -24, 0, 0.1, "dBFS")],
    defaults: { type: "normalize", peak: -1 },
  },
  {
    type: "loudness",
    name: "Loudness normalize",
    group: "Level",
    description: "Sets the perceived loudness (LUFS), the way streaming services measure it.",
    params: [num("target", "Target", -36, -6, 0.5, "LUFS")],
    defaults: { type: "loudness", target: -16 },
  },
  {
    type: "amplify",
    name: "Amplify",
    group: "Level",
    description: "Changes the level by a fixed amount.",
    params: [num("gain", "Gain", -24, 24, 0.1, "dB")],
    defaults: { type: "amplify", gain: 3 },
  },
];

export const EFFECT_GROUPS: EffectGroup[] = ["Repair", "Filter & EQ", "Dynamics", "Time & pitch", "Space", "Level"];

export function effectSpec(type: EffectType): EffectSpec {
  return EFFECTS.find((e) => e.type === type) ?? EFFECTS[0];
}

export function defaultEffect(type: EffectType): Effect {
  return structuredClone(effectSpec(type).defaults);
}

/** Reads a parameter; `bands.3` reaches into an array. */
export function getParam(target: Effect | Generator, id: string): Value {
  const [key, index] = id.split(".");
  const value = (target as unknown as Record<string, Value | number[]>)[key];
  return Array.isArray(value) ? value[Number(index)] : value;
}

export function setParam<T extends Effect | Generator>(target: T, id: string, value: Value): T {
  const [key, index] = id.split(".");
  const record = target as unknown as Record<string, Value | number[]>;
  const current = record[key];
  if (Array.isArray(current)) {
    const next = current.slice();
    next[Number(index)] = Number(value);
    return { ...target, [key]: next };
  }
  return { ...target, [key]: value };
}

function fmt(v: number, unit = "") {
  const s = Math.abs(v) >= 100 ? v.toFixed(0) : Number(v.toFixed(2)).toString();
  return `${s.replace("-", "−")}${unit ? ` ${unit}` : ""}`;
}

/** A short description of one step, for macro lists: "High-pass 80 Hz". */
export function describeEffect(effect: Effect): string {
  switch (effect.type) {
    case "noiseReduction":
      return `Noise reduction ${fmt(effect.reduction, "dB")}`;
    case "clickRemoval":
      return "Click removal";
    case "dcOffset":
      return "Remove DC offset";
    case "highPass":
      return `High-pass ${fmt(effect.frequency, "Hz")}`;
    case "lowPass":
      return `Low-pass ${fmt(effect.frequency, "Hz")}`;
    case "bassTreble":
      return `Bass ${fmt(effect.bass, "dB")}, treble ${fmt(effect.treble, "dB")}`;
    case "graphicEq":
      return "Graphic EQ";
    case "compressor":
      return `Compressor ${fmt(effect.threshold, "dB")} · ${fmt(effect.ratio)}:1`;
    case "limiter":
      return `Limiter ${fmt(effect.ceiling, "dBFS")}`;
    case "truncateSilence":
      return `Truncate pauses over ${fmt(effect.minSilence, "s")}`;
    case "changeTempo":
      return `Tempo ${effect.percent >= 0 ? "+" : ""}${fmt(effect.percent, "%")}`;
    case "changePitch":
      return `Pitch ${effect.semitones >= 0 ? "+" : ""}${fmt(effect.semitones, "st")}`;
    case "changeSpeed":
      return `Speed ${effect.percent >= 0 ? "+" : ""}${fmt(effect.percent, "%")}`;
    case "echo":
      return `Echo ${fmt(effect.delay, "s")}`;
    case "reverb":
      return "Reverb";
    case "normalize":
      return `Normalize to ${fmt(effect.peak, "dBFS")}`;
    case "loudness":
      return `Loudness ${fmt(effect.target, "LUFS")}`;
    case "amplify":
      return `Amplify ${effect.gain >= 0 ? "+" : ""}${fmt(effect.gain, "dB")}`;
    case "denoise":
      return `Reduce noise ${fmt(effect.reduction, "dB")} (automatic)`;
    case "trimSilence":
      return "Trim silence at the ends";
    case "level":
      return `Level to ${fmt(effect.target, "LUFS")}`;
    case "fixAudio":
      return `Fix my audio · ${fmt(effect.target, "LUFS")}`;
  }
}

export const BUILTIN_MACROS: Macro[] = [
  {
    id: "builtin-fix",
    name: "Fix my audio",
    builtIn: true,
    steps: [{ type: "fixAudio", target: -16 }],
  },
  {
    id: "builtin-podcast",
    name: "Podcast voice",
    builtIn: true,
    steps: [
      { type: "dcOffset" },
      { type: "highPass", frequency: 80, slope: 24 },
      { type: "compressor", threshold: -22, ratio: 3, attack: 10, release: 150, makeup: false },
      { type: "loudness", target: -16 },
      { type: "limiter", ceiling: -1, release: 50 },
    ],
  },
  {
    id: "builtin-master",
    name: "Music master at −14 LUFS",
    builtIn: true,
    steps: [
      { type: "loudness", target: -14 },
      { type: "limiter", ceiling: -1, release: 50 },
    ],
  },
  {
    id: "builtin-voice-note",
    name: "Tidy a voice note",
    builtIn: true,
    steps: [
      { type: "dcOffset" },
      { type: "clickRemoval", sensitivity: 5, maxWidth: 1 },
      { type: "truncateSilence", threshold: -40, minSilence: 1, keep: 0.4 },
      { type: "normalize", peak: -1 },
    ],
  },
  {
    id: "builtin-normalize",
    name: "Normalize to −1 dBFS",
    builtIn: true,
    steps: [{ type: "normalize", peak: -1 }],
  },
];

// ---------- Generators ----------

const WAVEFORMS: [Value, string][] = [
  ["sine", "Sine"],
  ["square", "Square"],
  ["sawtooth", "Sawtooth"],
  ["triangle", "Triangle"],
];

const amplitude = num("amplitude", "Level", -60, 0, 0.5, "dBFS");
const seconds = num("seconds", "Length", 0.01, 600, 0.01, "s", { log: true });

export interface GeneratorSpec {
  type: GeneratorType;
  name: string;
  description: string;
  params: ParamSpec[];
  defaults: Generator;
}

export const GENERATORS: GeneratorSpec[] = [
  {
    type: "tone",
    name: "Tone",
    description: "A steady test tone.",
    params: [{ id: "waveform", label: "Waveform", kind: "select", options: WAVEFORMS }, num("frequency", "Frequency", 20, 20000, 1, "Hz", { log: true }), amplitude, seconds],
    defaults: { type: "tone", waveform: "sine", frequency: 440, amplitude: -12, seconds: 5 },
  },
  {
    type: "chirp",
    name: "Chirp",
    description: "A sweep from one frequency to another, for testing speakers and rooms.",
    params: [
      { id: "waveform", label: "Waveform", kind: "select", options: WAVEFORMS },
      num("from", "From", 20, 20000, 1, "Hz", { log: true }),
      num("to", "To", 20, 20000, 1, "Hz", { log: true }),
      amplitude,
      seconds,
    ],
    defaults: { type: "chirp", waveform: "sine", from: 20, to: 20000, amplitude: -12, seconds: 10 },
  },
  {
    type: "noise",
    name: "Noise",
    description: "White, pink or brown noise.",
    params: [
      {
        id: "color",
        label: "Colour",
        kind: "select",
        options: [
          ["white", "White"],
          ["pink", "Pink"],
          ["brown", "Brown"],
        ],
      },
      amplitude,
      seconds,
    ],
    defaults: { type: "noise", color: "pink", amplitude: -18, seconds: 5 },
  },
  {
    type: "dtmf",
    name: "DTMF tones",
    description: "Telephone keypad tones: 0–9, *, # and A–D.",
    params: [{ id: "digits", label: "Digits", kind: "text" }, num("tone", "Tone", 20, 1000, 1, "ms"), num("gap", "Gap", 0, 1000, 1, "ms"), amplitude],
    defaults: { type: "dtmf", digits: "0123456789", tone: 100, gap: 100, amplitude: -12 },
  },
  {
    type: "silence",
    name: "Silence",
    description: "Digital silence, for gaps and padding.",
    params: [seconds],
    defaults: { type: "silence", seconds: 2 },
  },
];

export function generatorSpec(type: GeneratorType): GeneratorSpec {
  return GENERATORS.find((g) => g.type === type) ?? GENERATORS[0];
}
