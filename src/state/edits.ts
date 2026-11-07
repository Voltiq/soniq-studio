/*
 * Edit labels, plus a peaks-only imitation of the Rust edits for the browser
 * preview. In the desktop app the real edits run on samples in soniq-io.
 */
import type { EditOp, PeakData, SimpleEditOp } from "./types";

export const EDIT_LABELS: Record<SimpleEditOp, string> = {
  normalize: "Normalize",
  amplify: "Amplify +3 dB",
  fadeIn: "Fade in",
  fadeOut: "Fade out",
  reverse: "Reverse",
  silence: "Silence",
};

export function editLabel(op: EditOp): string {
  return typeof op === "string" ? EDIT_LABELS[op] : `Loudness ${op.loudness} LUFS`;
}

export function editPeaks(data: PeakData, op: SimpleEditOp, from: number, to: number): { data: PeakData; clipped: boolean } {
  const i0 = Math.max(0, Math.floor(from));
  let clipped = false;
  const channels = data.channels.map((src) => {
    const ch = src.slice();
    const i1 = Math.min(ch.length, Math.ceil(to));
    const len = Math.max(1, i1 - i0);
    let peak = 0;
    for (const c of data.channels) for (let i = i0; i < i1; i++) peak = Math.max(peak, c[i] ?? 0);
    for (let i = i0; i < i1; i++) {
      const k = i - i0;
      if (op === "normalize") ch[i] *= peak > 0 ? 0.891 / peak : 1;
      else if (op === "amplify") {
        ch[i] *= 1.4125;
        clipped ||= ch[i] > 1;
      } else if (op === "fadeIn") ch[i] *= k / len;
      else if (op === "fadeOut") ch[i] *= 1 - k / len;
      else if (op === "silence") ch[i] = 0;
    }
    if (op === "reverse") ch.subarray(i0, i1).reverse();
    return ch;
  });
  return { data: { ...data, channels }, clipped };
}
