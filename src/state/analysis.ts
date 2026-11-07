/*
 * The master's analysis (loudness, peaks, correlation, spectrum), kept
 * outside the store like the meters: it changes 30 times a second and only
 * the meters panel listens.
 */
import { useSyncExternalStore } from "react";
import type { AnalysisReading } from "../engine/bridge";

/** Seconds of loudness the history graph shows. */
export const HISTORY_SECONDS = 20;
/** Readings arrive about 30 times a second. */
const HISTORY_LENGTH = HISTORY_SECONDS * 30;

let latest: AnalysisReading | null = null;
/** Momentary and short-term loudness, oldest first. */
export const loudnessHistory: { momentary: (number | null)[]; shortTerm: (number | null)[] } = { momentary: [], shortTerm: [] };
const listeners = new Set<() => void>();

export function ingestAnalysis(reading: AnalysisReading | null | undefined) {
  if (!reading) return;
  latest = reading;
  for (const [list, value] of [
    [loudnessHistory.momentary, reading.momentary],
    [loudnessHistory.shortTerm, reading.shortTerm],
  ] as const) {
    list.push(value);
    if (list.length > HISTORY_LENGTH) list.shift();
  }
  listeners.forEach((listen) => listen());
}

/** Forgets the history and the last reading (the panel was closed or reset). */
export function clearAnalysis() {
  latest = null;
  loudnessHistory.momentary.length = 0;
  loudnessHistory.shortTerm.length = 0;
  listeners.forEach((listen) => listen());
}

function subscribe(listen: () => void) {
  listeners.add(listen);
  return () => listeners.delete(listen);
}

/** The latest reading; re-renders with each one. */
export function useAnalysis(): AnalysisReading | null {
  return useSyncExternalStore(subscribe, () => latest);
}

/** Centre of spectrum band `k` in Hz: sixths of an octave up from 20 Hz. */
export const bandHz = (k: number) => 20 * 2 ** (k / 6);
