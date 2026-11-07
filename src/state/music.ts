/*
 * Notes, keys and scales. Octaves follow Ableton Live: middle C (MIDI 60) is C3.
 */

export const NOTE_NAMES = ["C", "C♯", "D", "D♯", "E", "F", "F♯", "G", "G♯", "A", "A♯", "B"];
const ROOT_NAMES = ["C", "C♯", "D", "E♭", "E", "F", "F♯", "G", "A♭", "A", "B♭", "B"];

export function noteName(pitch: number): string {
  return `${NOTE_NAMES[((pitch % 12) + 12) % 12]}${Math.floor(pitch / 12) - 2}`;
}

export const isBlackKey = (pitch: number) => [1, 3, 6, 8, 10].includes(((pitch % 12) + 12) % 12);

/** Scales by name, as semitones above the root. */
export const SCALES: Record<string, number[]> = {
  major: [0, 2, 4, 5, 7, 9, 11],
  minor: [0, 2, 3, 5, 7, 8, 10],
  dorian: [0, 2, 3, 5, 7, 9, 10],
  phrygian: [0, 1, 3, 5, 7, 8, 10],
  lydian: [0, 2, 4, 6, 7, 9, 11],
  mixolydian: [0, 2, 4, 5, 7, 9, 10],
  "harmonic minor": [0, 2, 3, 5, 7, 8, 11],
  "melodic minor": [0, 2, 3, 5, 7, 9, 11],
  "major pentatonic": [0, 2, 4, 7, 9],
  "minor pentatonic": [0, 3, 5, 7, 10],
  blues: [0, 3, 5, 6, 7, 10],
  chromatic: [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
};

export const SCALE_NAMES = Object.keys(SCALES);
export const ROOTS = ROOT_NAMES;

export interface Key {
  root: number;
  scale: string;
}

/** "A minor", "E♭ dorian", "Bb major" → root pitch class and scale. */
export function parseKey(key: string): Key {
  const match = /^\s*([A-Ga-g])\s*([#♯b♭]?)\s*(.*)$/.exec(key);
  if (!match) return { root: 0, scale: "major" };
  const base = { c: 0, d: 2, e: 4, f: 5, g: 7, a: 9, b: 11 }[match[1].toLowerCase() as "c"] ?? 0;
  const shift = match[2] === "#" || match[2] === "♯" ? 1 : match[2] === "b" || match[2] === "♭" ? -1 : 0;
  const scale = match[3].trim().toLowerCase();
  return { root: (base + shift + 12) % 12, scale: SCALES[scale] ? scale : "major" };
}

export function formatKey(key: Key): string {
  return `${ROOT_NAMES[key.root]} ${key.scale}`;
}

/** Pitch classes in the key. */
export function scaleClasses(key: Key): Set<number> {
  return new Set((SCALES[key.scale] ?? SCALES.major).map((s) => (key.root + s) % 12));
}

/** The pitch `steps` scale degrees away (or semitones when chromatic). */
export function moveInScale(pitch: number, steps: number, inKey: Set<number> | null): number {
  if (!inKey || inKey.size >= 12) return pitch + steps;
  let p = pitch;
  const dir = Math.sign(steps);
  for (let left = Math.abs(steps); left > 0; ) {
    p += dir;
    if (p < 0 || p > 127) return pitch;
    if (inKey.has(((p % 12) + 12) % 12)) left--;
  }
  return p;
}

/** The nearest pitch in the key (ties go up). */
export function snapToScale(pitch: number, inKey: Set<number> | null): number {
  if (!inKey || inKey.has(((pitch % 12) + 12) % 12)) return pitch;
  for (let d = 1; d < 12; d++) {
    if (pitch + d <= 127 && inKey.has((pitch + d) % 12)) return pitch + d;
    if (pitch - d >= 0 && inKey.has((((pitch - d) % 12) + 12) % 12)) return pitch - d;
  }
  return pitch;
}
