import { noteName } from "./music";
import type { DeviceParam } from "./types";

export const MIN_DB = -60;

export function dbToGain(db: number): number {
  return db <= MIN_DB ? 0 : Math.pow(10, db / 20);
}

export function gainToDb(gain: number): number {
  return gain <= 0 ? -Infinity : 20 * Math.log10(gain);
}

export function formatDb(db: number, digits = 1): string {
  if (!Number.isFinite(db) || db <= MIN_DB) return "−∞";
  const sign = db > 0 ? "+" : db < 0 ? "−" : "";
  return sign + Math.abs(db).toFixed(digits);
}

export function formatPan(pan: number): string {
  const v = Math.round(pan * 50);
  if (v === 0) return "C";
  return v < 0 ? `${-v}L` : `${v}R`;
}

export function formatParam(param: DeviceParam): string {
  const v = param.value;
  if (param.options) return param.options[Math.round(v)] ?? String(v);
  switch (param.unit) {
    case "note":
      return noteName(Math.round(v));
    case "ct":
      return `${Math.round(v)} ct`;
    case "dB":
      return `${formatDb(v)} dB`;
    case "Hz":
      // Slow rates (LFOs, chorus) need their decimals.
      if (v < 10) return `${v.toFixed(v < 1 ? 2 : 1)} Hz`;
      return v >= 1000 ? `${(v / 1000).toFixed(v >= 10000 ? 1 : 2)} kHz` : `${Math.round(v)} Hz`;
    case "ms":
      return v < 10 ? `${v.toFixed(1)} ms` : `${Math.round(v)} ms`;
    case "s":
      return `${v.toFixed(1)} s`;
    case ":1":
      return `${v.toFixed(1)} : 1`;
    case "st":
      return `${v > 0 ? "+" : ""}${Math.round(v)} st`;
    case "%":
      return `${Math.round(v)}%`;
    case "":
      return param.bipolar ? `${v > 0 ? "+" : ""}${Math.round(v)}` : `${Math.round(v)}`;
    default:
      return `${Math.round(v)} ${param.unit}`;
  }
}

/** Seconds → "mm:ss.mmm". */
export function formatClock(seconds: number): string {
  const s = Math.max(0, seconds);
  const m = Math.floor(s / 60);
  const rest = s - m * 60;
  return `${String(m).padStart(2, "0")}:${rest.toFixed(3).padStart(6, "0")}`;
}

/** Beats → "bar. beat. sixteenth", 1-based like every DAW. */
export function formatBars(beats: number, beatsPerBar: number): string {
  const b = Math.max(0, beats);
  const bar = Math.floor(b / beatsPerBar) + 1;
  const beat = Math.floor(b % beatsPerBar) + 1;
  const sixteenth = Math.floor((b % 1) * 4) + 1;
  return `${bar}. ${beat}. ${sixteenth}`;
}
