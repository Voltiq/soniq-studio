/*
 * Piano roll tools in the spirit of FL Studio's and Live 12's: transforms
 * that rework the selected notes (or all of them), and generators that write
 * new notes across the clip. All pure: notes in, one `NoteEdit` out.
 */
import type { ParamSpec } from "./effects";
import { moveInScale, snapToScale } from "./music";
import { MIN_NOTE, commit, targets, type NoteEdit } from "./notes";
import type { Note } from "./types";

export type NoteTool =
  | { type: "arpeggiate"; style: "up" | "down" | "updown" | "random"; rate: number; gate: number }
  | { type: "chop"; division: number }
  | { type: "strum"; amount: number; direction: "up" | "down" }
  | { type: "flam"; offset: number; velocity: number }
  | { type: "chords"; chord: "triad" | "seventh" | "power" | "octave" }
  | { type: "humanize"; timing: number; velocity: number; seed: number }
  | { type: "span"; percent: number }
  | { type: "reverse" }
  | { type: "invert" }
  | { type: "glue" }
  | { type: "rhythm"; steps: number; pulses: number; rotate: number; pitch: number; rate: number; replace: boolean }
  | { type: "seed"; density: number; low: number; high: number; rate: number; seed: number; replace: boolean };

export type NoteToolType = NoteTool["type"];

export interface NoteToolSpec {
  type: NoteToolType;
  name: string;
  group: "Transform" | "Generate";
  description: string;
  params: ParamSpec[];
  defaults: NoteTool;
}

/** What a tool needs to know about the clip. */
export interface ToolContext {
  /** Clip length in beats. */
  length: number;
  /** Pitch classes of the song key, or null when chromatic. */
  inKey: Set<number> | null;
}

const RATES: [number, string][] = [
  [1, "1/4"],
  [0.5, "1/8"],
  [1 / 3, "1/8T"],
  [0.25, "1/16"],
  [1 / 6, "1/16T"],
  [0.125, "1/32"],
];

const num = (id: string, label: string, min: number, max: number, step: number, unit?: string, hint?: string): ParamSpec => ({
  id,
  label,
  kind: "number",
  min,
  max,
  step,
  unit,
  hint,
});
const rate = (id = "rate", label = "Rate"): ParamSpec => ({ id, label, kind: "select", options: RATES });
const toggle = (id: string, label: string, hint?: string): ParamSpec => ({ id, label, kind: "toggle", hint });

export const NOTE_TOOLS: NoteToolSpec[] = [
  {
    type: "arpeggiate",
    name: "Arpeggiate",
    group: "Transform",
    description: "Turns each chord into a run of its notes, for as long as the chord lasts.",
    params: [
      {
        id: "style",
        label: "Style",
        kind: "select",
        options: [
          ["up", "Up"],
          ["down", "Down"],
          ["updown", "Up & down"],
          ["random", "Random"],
        ],
      },
      rate(),
      num("gate", "Gate", 10, 100, 1, "%"),
    ],
    defaults: { type: "arpeggiate", style: "up", rate: 0.25, gate: 80 },
  },
  {
    type: "chop",
    name: "Chop",
    group: "Transform",
    description: "Cuts every note into repeated shorter notes.",
    params: [rate("division", "Pieces")],
    defaults: { type: "chop", division: 0.25 },
  },
  {
    type: "strum",
    name: "Strum",
    group: "Transform",
    description: "Staggers the notes of each chord like a strummed guitar.",
    params: [
      num("amount", "Delay per note", 0.01, 0.25, 0.01, "beats"),
      {
        id: "direction",
        label: "Direction",
        kind: "select",
        options: [
          ["up", "Low to high"],
          ["down", "High to low"],
        ],
      },
    ],
    defaults: { type: "strum", amount: 0.05, direction: "up" },
  },
  {
    type: "flam",
    name: "Flam",
    group: "Transform",
    description: "Adds a soft grace note just before each note.",
    params: [num("offset", "Ahead by", 0.01, 0.25, 0.01, "beats"), num("velocity", "Grace velocity", 10, 100, 1, "%")],
    defaults: { type: "flam", offset: 0.05, velocity: 60 },
  },
  {
    type: "chords",
    name: "Make chords",
    group: "Transform",
    description: "Stacks a chord on every note, in the song key (or major when the key is chromatic).",
    params: [
      {
        id: "chord",
        label: "Chord",
        kind: "select",
        options: [
          ["triad", "Triad"],
          ["seventh", "Seventh"],
          ["power", "Power (5th + octave)"],
          ["octave", "Octave"],
        ],
      },
    ],
    defaults: { type: "chords", chord: "triad" },
  },
  {
    type: "humanize",
    name: "Humanize",
    group: "Transform",
    description: "Nudges timing and velocity by small random amounts, so the part breathes.",
    params: [num("timing", "Timing", 0, 0.1, 0.005, "beats"), num("velocity", "Velocity", 0, 40, 1, "%"), num("seed", "Variation", 1, 99, 1)],
    defaults: { type: "humanize", timing: 0.02, velocity: 12, seed: 1 },
  },
  {
    type: "span",
    name: "Span",
    group: "Transform",
    description: "Scales note lengths: below 100% is staccato, above is legato.",
    params: [num("percent", "Length", 10, 300, 1, "%")],
    defaults: { type: "span", percent: 50 },
  },
  { type: "reverse", name: "Reverse", group: "Transform", description: "Plays the notes backwards in time.", params: [], defaults: { type: "reverse" } },
  { type: "invert", name: "Invert", group: "Transform", description: "Mirrors the pitches upside down around their middle.", params: [], defaults: { type: "invert" } },
  { type: "glue", name: "Glue", group: "Transform", description: "Joins notes of the same pitch that touch into one.", params: [], defaults: { type: "glue" } },
  {
    type: "rhythm",
    name: "Rhythm",
    group: "Generate",
    description: "Spreads a number of hits as evenly as possible over a cycle of steps (a Euclidean rhythm), across the clip.",
    params: [
      num("steps", "Steps", 2, 32, 1),
      num("pulses", "Hits", 1, 32, 1),
      num("rotate", "Rotate", 0, 31, 1),
      num("pitch", "Note", 0, 127, 1, "", "36 is the kick on a Drum Rack, 38 the snare, 42 the closed hat"),
      rate(),
      toggle("replace", "Replace notes of that pitch"),
    ],
    defaults: { type: "rhythm", steps: 16, pulses: 5, rotate: 0, pitch: 36, rate: 0.25, replace: true },
  },
  {
    type: "seed",
    name: "Seed",
    group: "Generate",
    description: "Writes a random melody in the song key across the clip; change the variation for another one.",
    params: [
      num("density", "Density", 5, 100, 1, "%"),
      num("low", "Lowest note", 24, 108, 1),
      num("high", "Highest note", 24, 108, 1),
      rate(),
      num("seed", "Variation", 1, 99, 1),
      toggle("replace", "Replace all notes"),
    ],
    defaults: { type: "seed", density: 45, low: 57, high: 76, rate: 0.25, seed: 1, replace: false },
  },
];

export function noteToolSpec(type: NoteToolType): NoteToolSpec {
  return NOTE_TOOLS.find((t) => t.type === type) ?? NOTE_TOOLS[0];
}

/** A small seeded random generator (mulberry32), so a variation number gives the same result twice. */
function random(seed: number) {
  let a = Math.floor(seed * 2654435761) >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/** Selected notes grouped into chords (notes that start together). */
function chords(notes: Note[], chosen: number[]): number[][] {
  const groups = new Map<number, number[]>();
  for (const i of chosen) {
    const key = Math.round(notes[i].start * 1e6);
    groups.set(key, [...(groups.get(key) ?? []), i]);
  }
  return [...groups.values()];
}

const clampPitch = (p: number) => Math.min(127, Math.max(0, Math.round(p)));

/** Runs a tool over the selection (or every note) and returns the new notes and selection. */
export function applyTool(tool: NoteTool, notes: Note[], selection: number[], ctx: ToolContext): NoteEdit {
  const chosen = targets(notes, selection);
  const picked = new Set(chosen);
  const keep = notes.filter((_, i) => !picked.has(i));
  /** The untouched notes plus `added`, with `added` selected. */
  const result = (added: Note[], base = keep) => commit([...base, ...added], (i) => i >= base.length);
  /** The notes with the chosen ones changed in place, still selected. */
  const inPlace = (change: (n: Note, i: number) => Note) =>
    commit(
      notes.map((n, i) => (picked.has(i) ? change(n, i) : n)),
      (i) => selection.includes(i) || (!selection.length && picked.has(i)),
    );

  switch (tool.type) {
    case "arpeggiate": {
      const added: Note[] = [];
      for (const group of chords(notes, chosen)) {
        const chord = group.map((i) => notes[i]).sort((a, b) => a.pitch - b.pitch);
        const start = chord[0].start;
        const end = Math.max(...chord.map((n) => n.start + n.length));
        const up = chord.map((n) => n.pitch);
        const order = tool.style === "down" ? [...up].reverse() : tool.style === "updown" && up.length > 2 ? [...up, ...up.slice(1, -1).reverse()] : up;
        const rnd = random(start + 1);
        for (let t = start, k = 0; t < end - 1e-9; t += tool.rate, k++) {
          const pitch = tool.style === "random" ? up[Math.floor(rnd() * up.length)] : order[k % order.length];
          const velocity = chord.find((n) => n.pitch === pitch)?.velocity ?? chord[0].velocity;
          added.push({ start: t, length: Math.max(MIN_NOTE, Math.min(tool.rate * (tool.gate / 100), end - t)), pitch, velocity });
        }
      }
      return result(added);
    }
    case "chop": {
      const added: Note[] = [];
      for (const i of chosen) {
        const n = notes[i];
        for (let t = n.start; t < n.start + n.length - 1e-9; t += tool.division) {
          added.push({ ...n, start: t, length: Math.max(MIN_NOTE, Math.min(tool.division, n.start + n.length - t)) });
        }
      }
      return result(added);
    }
    case "strum": {
      const shift = new Map<number, number>();
      for (const group of chords(notes, chosen)) {
        const ordered = [...group].sort((a, b) => (tool.direction === "up" ? 1 : -1) * (notes[a].pitch - notes[b].pitch));
        ordered.forEach((i, k) => shift.set(i, k * tool.amount));
      }
      return inPlace((n, i) => {
        const d = Math.min(shift.get(i) ?? 0, n.length - MIN_NOTE);
        return { ...n, start: n.start + d, length: n.length - d };
      });
    }
    case "flam": {
      // A note at the very start of the clip has no room for one.
      const graces = chosen
        .map((i) => notes[i])
        .filter((n) => n.start >= MIN_NOTE)
        .map((n) => {
          const length = Math.min(tool.offset, n.start);
          return { ...n, start: n.start - length, length, velocity: Math.max(0.05, n.velocity * (tool.velocity / 100)) };
        });
      return result(graces, notes);
    }
    case "chords": {
      const exists = new Set(notes.map((n) => `${n.start.toFixed(6)}|${n.pitch}`));
      const added: Note[] = [];
      for (const i of chosen) {
        const n = notes[i];
        const stacked =
          tool.chord === "octave"
            ? [n.pitch + 12]
            : tool.chord === "power"
              ? [n.pitch + 7, n.pitch + 12]
              : ctx.inKey
                ? (tool.chord === "seventh" ? [2, 4, 6] : [2, 4]).map((degrees) => moveInScale(snapToScale(n.pitch, ctx.inKey), degrees, ctx.inKey))
                : (tool.chord === "seventh" ? [4, 7, 10] : [4, 7]).map((s) => n.pitch + s);
        for (const p of stacked) {
          const key = `${n.start.toFixed(6)}|${p}`;
          if (p <= 127 && !exists.has(key)) {
            exists.add(key);
            added.push({ ...n, pitch: p });
          }
        }
      }
      return result(added, notes);
    }
    case "humanize": {
      const rnd = random(tool.seed);
      return inPlace((n) => {
        const dt = (rnd() * 2 - 1) * tool.timing;
        const dv = (rnd() * 2 - 1) * (tool.velocity / 100);
        return { ...n, start: Math.max(0, n.start + dt), velocity: Math.min(1, Math.max(0.05, n.velocity + dv)) };
      });
    }
    case "span":
      return inPlace((n) => ({ ...n, length: Math.max(MIN_NOTE, (n.length * tool.percent) / 100) }));
    case "reverse": {
      const lo = Math.min(...chosen.map((i) => notes[i].start));
      const hi = Math.max(...chosen.map((i) => notes[i].start + notes[i].length));
      return inPlace((n) => ({ ...n, start: Math.max(0, lo + hi - (n.start + n.length)) }));
    }
    case "invert": {
      const pitches = chosen.map((i) => notes[i].pitch);
      const centre = (Math.min(...pitches) + Math.max(...pitches)) / 2;
      return inPlace((n) => ({ ...n, pitch: clampPitch(snapToScale(clampPitch(2 * centre - n.pitch), ctx.inKey)) }));
    }
    case "glue": {
      const byPitch = new Map<number, Note[]>();
      for (const i of chosen) byPitch.set(notes[i].pitch, [...(byPitch.get(notes[i].pitch) ?? []), { ...notes[i] }]);
      const added: Note[] = [];
      for (const list of byPitch.values()) {
        list.sort((a, b) => a.start - b.start);
        let last: Note | null = null;
        for (const n of list) {
          if (last && last.start + last.length >= n.start - 1e-6) last.length = Math.max(last.length, n.start + n.length - last.start);
          else {
            added.push(n);
            last = n;
          }
        }
      }
      return result(added);
    }
    case "rhythm": {
      const steps = Math.max(1, Math.round(tool.steps));
      const pulses = Math.min(steps, Math.max(1, Math.round(tool.pulses)));
      // Hit where the running total of pulses/steps crosses a whole number.
      const hits = Array.from({ length: steps }, (_, i) => ((i + steps - (Math.round(tool.rotate) % steps)) * pulses) % steps < pulses);
      const base = tool.replace ? notes.filter((n) => n.pitch !== tool.pitch) : notes;
      const added: Note[] = [];
      for (let t = 0, k = 0; t < ctx.length - 1e-9; t += tool.rate, k++) {
        if (hits[k % steps]) added.push({ start: t, length: tool.rate * 0.5, pitch: clampPitch(tool.pitch), velocity: k % steps === hits.indexOf(true) ? 1 : 0.75 });
      }
      return result(added, base);
    }
    case "seed": {
      const rnd = random(tool.seed);
      const [low, high] = [Math.min(tool.low, tool.high), Math.max(tool.low, tool.high)];
      const base = tool.replace ? [] : notes;
      const added: Note[] = [];
      for (let t = 0; t < ctx.length - 1e-9; t += tool.rate) {
        if (rnd() * 100 >= tool.density) continue;
        const pitch = clampPitch(snapToScale(Math.round(low + rnd() * (high - low)), ctx.inKey));
        const steps = 1 + Math.floor(rnd() * 2);
        added.push({ start: t, length: Math.min(tool.rate * steps, ctx.length - t), pitch, velocity: 0.6 + rnd() * 0.35 });
      }
      return result(added, base);
    }
  }
}
