/*
 * Note editing as pure functions: each takes the clip's notes and the
 * selected indices and returns the new notes and the new selection, ready
 * for one `setNotes` op (so every edit is one undo step).
 */
import { moveInScale } from "./music";
import { sortNotes, type Note } from "./types";

export interface NoteEdit {
  notes: Note[];
  selection: number[];
}

/** Grid steps the piano roll offers, in beats. */
export const GRIDS: [string, number][] = [
  ["1/4", 1],
  ["1/8", 0.5],
  ["1/8T", 1 / 3],
  ["1/16", 0.25],
  ["1/16T", 1 / 6],
  ["1/32", 0.125],
];

export const MIN_NOTE = 1 / 128;

/** Sorts like the backend and keeps track of which notes are selected. */
export function commit(notes: Note[], selected: (index: number, note: Note) => boolean): NoteEdit {
  const tagged = sortNotes(notes.map((n, i) => ({ ...n, sel: selected(i, n) })));
  return {
    notes: tagged.map(({ sel: _sel, ...n }) => n),
    selection: tagged.flatMap((n, i) => (n.sel ? [i] : [])),
  };
}

/** The indices to act on: the selection, or every note when nothing is selected. */
export function targets(notes: Note[], selection: number[]): number[] {
  return selection.length ? selection : notes.map((_, i) => i);
}

export function snap(beats: number, grid: number): number {
  return grid > 0 ? Math.round(beats / grid) * grid : beats;
}

/**
 * Moves notes toward the grid. `strength` 0–1 sets how far; `swing` 0–1
 * delays every second grid step by up to half a step; `ends` also
 * quantizes where notes stop.
 */
export function quantize(notes: Note[], selection: number[], grid: number, strength: number, swing: number, ends: boolean): NoteEdit {
  const chosen = new Set(targets(notes, selection));
  const place = (beats: number) => {
    const step = Math.round(beats / grid);
    const swung = step * grid + (step % 2 === 1 ? swing * grid * 0.5 : 0);
    return beats + (swung - beats) * strength;
  };
  const next = notes.map((n, i) => {
    if (!chosen.has(i)) return n;
    const start = Math.max(0, place(n.start));
    const end = ends ? Math.max(start + Math.min(grid, n.length), place(n.start + n.length)) : start + n.length;
    return { ...n, start, length: Math.max(MIN_NOTE, end - start) };
  });
  return commit(next, (i) => selection.includes(i));
}

/** Each note lasts until the next one starts (chords stay chords). */
export function legato(notes: Note[], selection: number[]): NoteEdit {
  const chosen = targets(notes, selection);
  const starts = [...new Set(chosen.map((i) => notes[i].start))].sort((a, b) => a - b);
  const next = notes.map((n, i) => {
    if (!chosen.includes(i)) return n;
    const following = starts.find((s) => s > n.start + 1e-9);
    return following === undefined ? n : { ...n, length: following - n.start };
  });
  return commit(next, (i) => selection.includes(i));
}

/** Shifts pitch by semitones, or by scale degrees when a key is given. */
export function transpose(notes: Note[], selection: number[], steps: number, inKey: Set<number> | null): NoteEdit {
  const chosen = new Set(targets(notes, selection));
  const moved = notes.map((n, i) => (chosen.has(i) ? moveInScale(n.pitch, steps, Math.abs(steps) === 12 ? null : inKey) : n.pitch));
  if (moved.some((p) => p < 0 || p > 127)) return { notes, selection };
  return commit(
    notes.map((n, i) => ({ ...n, pitch: moved[i] })),
    (i) => selection.includes(i),
  );
}

/** Moves notes in time by `beats` (never before the clip start). */
export function nudge(notes: Note[], selection: number[], beats: number): NoteEdit {
  const chosen = new Set(targets(notes, selection));
  const first = Math.min(...[...chosen].map((i) => notes[i].start));
  const shift = Math.max(beats, -first);
  return commit(
    notes.map((n, i) => (chosen.has(i) ? { ...n, start: n.start + shift } : n)),
    (i) => selection.includes(i),
  );
}

/** Lengthens or shortens notes by `beats`. */
export function stretchEnds(notes: Note[], selection: number[], beats: number): NoteEdit {
  const chosen = new Set(targets(notes, selection));
  return commit(
    notes.map((n, i) => (chosen.has(i) ? { ...n, length: Math.max(MIN_NOTE, n.length + beats) } : n)),
    (i) => selection.includes(i),
  );
}

/** Copies the selection right after itself; the copies become the selection. */
export function duplicate(notes: Note[], selection: number[]): NoteEdit {
  if (!selection.length) return { notes, selection };
  const chosen = selection.map((i) => notes[i]);
  const start = Math.min(...chosen.map((n) => n.start));
  const end = Math.max(...chosen.map((n) => n.start + n.length));
  const span = end - start;
  const copies = chosen.map((n) => ({ ...n, start: n.start + span }));
  return commit([...notes, ...copies], (i) => i >= notes.length);
}

export function remove(notes: Note[], selection: number[]): NoteEdit {
  const gone = new Set(selection);
  return { notes: notes.filter((_, i) => !gone.has(i)), selection: [] };
}

/** Pastes notes (relative to their first start) at `at` beats; the pasted notes become the selection. */
export function paste(notes: Note[], clipboard: Note[], at: number): NoteEdit {
  if (!clipboard.length) return { notes, selection: [] };
  const first = Math.min(...clipboard.map((n) => n.start));
  const pasted = clipboard.map((n) => ({ ...n, start: at + n.start - first }));
  return commit([...notes, ...pasted], (i) => i >= notes.length);
}

/** Copies of the selected notes, for the clipboard. */
export function copyOf(notes: Note[], selection: number[]): Note[] {
  return selection.map((i) => ({ ...notes[i] }));
}
