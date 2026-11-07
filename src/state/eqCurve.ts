/**
 * The EQ's frequency response, for drawing: the same RBJ biquads the engine
 * runs (crates/soniq-dsp/src/fx/eq.rs), evaluated on the unit circle.
 */
import type { Device } from "./types";

export const EQ_BANDS = 8;
/** Option index of each band type (mirrors the catalog). */
export const LOW_CUT = 0;
export const NOTCH = 3;
export const HIGH_CUT = 5;

export interface EqBand {
  on: boolean;
  type: number;
  freq: number;
  gain: number;
  q: number;
}

/** Bands whose gain moves them up and down (shelves and bells). */
export const hasGain = (type: number) => type !== LOW_CUT && type !== NOTCH && type !== HIGH_CUT;

export function eqBands(device: Device): EqBand[] {
  const v = (id: string, fallback: number) => device.params.find((p) => p.id === id)?.value ?? fallback;
  return Array.from({ length: EQ_BANDS }, (_, i) => {
    const n = i + 1;
    return { on: v(`b${n}on`, 0) >= 0.5, type: Math.round(v(`b${n}type`, 2)), freq: v(`b${n}freq`, 1000), gain: v(`b${n}gain`, 0), q: v(`b${n}q`, 0.71) };
  });
}

type Coefs = [number, number, number, number, number];

function biquad(type: number, freq: number, gainDb: number, q: number, sr: number): Coefs {
  const w = (2 * Math.PI * Math.min(Math.max(freq, 10), sr * 0.49)) / sr;
  const cos = Math.cos(w);
  const alpha = Math.sin(w) / (2 * Math.max(q, 0.05));
  const a = Math.pow(10, gainDb / 40);
  const sqa = 2 * Math.sqrt(a) * alpha;
  let c: [number, number, number, number, number, number];
  switch (type) {
    case 0:
      c = [(1 + cos) / 2, -(1 + cos), (1 + cos) / 2, 1 + alpha, -2 * cos, 1 - alpha];
      break;
    case 1:
      c = [
        a * (a + 1 - (a - 1) * cos + sqa),
        2 * a * (a - 1 - (a + 1) * cos),
        a * (a + 1 - (a - 1) * cos - sqa),
        a + 1 + (a - 1) * cos + sqa,
        -2 * (a - 1 + (a + 1) * cos),
        a + 1 + (a - 1) * cos - sqa,
      ];
      break;
    case 2:
      c = [1 + alpha * a, -2 * cos, 1 - alpha * a, 1 + alpha / a, -2 * cos, 1 - alpha / a];
      break;
    case 3:
      c = [1, -2 * cos, 1, 1 + alpha, -2 * cos, 1 - alpha];
      break;
    case 4:
      c = [
        a * (a + 1 + (a - 1) * cos + sqa),
        -2 * a * (a - 1 + (a + 1) * cos),
        a * (a + 1 + (a - 1) * cos - sqa),
        a + 1 - (a - 1) * cos + sqa,
        2 * (a - 1 - (a + 1) * cos),
        a + 1 - (a - 1) * cos - sqa,
      ];
      break;
    default:
      c = [(1 - cos) / 2, 1 - cos, (1 - cos) / 2, 1 + alpha, -2 * cos, 1 - alpha];
  }
  const a0 = c[3];
  return [c[0] / a0, c[1] / a0, c[2] / a0, c[4] / a0, c[5] / a0];
}

/** One biquad's gain in dB at angular frequency `w`. */
function magnitudeDb([b0, b1, b2, a1, a2]: Coefs, w: number): number {
  const c1 = Math.cos(w);
  const c2 = Math.cos(2 * w);
  const num = b0 * b0 + b1 * b1 + b2 * b2 + 2 * c1 * (b0 * b1 + b1 * b2) + 2 * c2 * b0 * b2;
  const den = 1 + a1 * a1 + a2 * a2 + 2 * c1 * (a1 + a1 * a2) + 2 * c2 * a2;
  return 10 * Math.log10(Math.max(num, 1e-12) / Math.max(den, 1e-12));
}

/** The stages a band runs: cuts are two (24 dB/oct), the rest one. */
function stages(band: EqBand, sr: number): Coefs[] {
  if (band.type === LOW_CUT || band.type === HIGH_CUT) {
    return [biquad(band.type, band.freq, 0, (band.q * 1.307) / 0.707, sr), biquad(band.type, band.freq, 0, 0.541, sr)];
  }
  return [biquad(band.type, band.freq, band.gain, band.q, sr)];
}

/** A band that changes nothing (off, or a bell or shelf at 0 dB). */
const silent = (band: EqBand) => !band.on || (hasGain(band.type) && Math.abs(band.gain) <= 0.01);

/** The whole EQ's gain in dB at each frequency (Hz), output gain included. */
export function eqResponse(bands: EqBand[], freqs: number[], outputDb = 0, sr = 48_000): number[] {
  const active = bands.filter((b) => !silent(b)).flatMap((b) => stages(b, sr));
  return freqs.map((f) => {
    const w = (2 * Math.PI * f) / sr;
    return active.reduce((sum, c) => sum + magnitudeDb(c, w), outputDb);
  });
}

/** One band's own curve, for highlighting it. */
export function bandResponse(band: EqBand, freqs: number[], sr = 48_000): number[] {
  return eqResponse([{ ...band, on: true }], freqs, 0, sr);
}
