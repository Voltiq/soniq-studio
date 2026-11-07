/*
 * Meter levels, kept outside React state so 60 fps updates never re-render
 * the tree; <Meter> reads this map in its own animation frame.
 *
 * In the desktop app the engine streams real peaks (ingestFrame). In the
 * browser preview they are estimated from the clips under the playhead.
 */
import type { MeterFrame } from "../engine/bridge";
import { ingestAnalysis } from "./analysis";
import { dbToGain } from "./format";
import { cachedPeaks } from "./peaks";
import type { Store } from "./store";
import { clipSeconds, tempoOf, type TempoMap } from "./tempo";
import { timelineClips, type Track } from "./types";

export const meterLevels = new Map<string, number>();

/** Gain reduction of dynamics devices (dB), held and let fall, by device id. */
export const deviceReduction = new Map<string, number>();

/** The live spectrum of one device (sixth-octave bands from 20 Hz, dB). */
export const deviceSpectrum: { id: string | null; bands: number[] } = { id: null, bands: [] };

/** When the safety limiter last acted (performance.now()) and by how much (dB). */
export const safetyActivity = { at: -1e9, db: 0 };

/** How far through its loop each track's session clip is (0–1), from the engine. */
export const clipProgress = new Map<string, number>();

const FALL = 0.85;

function smooth(id: string, level: number) {
  const prev = meterLevels.get(id) ?? 0;
  meterLevels.set(id, level >= prev ? level : prev * FALL);
}

/** Armed tracks show what comes in at the input, like a tape machine. */
export function ingestFrame(frame: MeterFrame, armed: Set<string>) {
  const input = Math.max(frame.input[0], frame.input[1]);
  for (const [id, [l, r]] of frame.tracks) smooth(id, Math.max(l, r, armed.has(id) ? input : 0));
  smooth("master", Math.max(frame.master[0], frame.master[1]));
  // Reduction meters rise at once and fall back about 20 dB a second.
  const seen = new Set<string>();
  for (const [id, db] of frame.devices ?? []) {
    seen.add(id);
    deviceReduction.set(id, Math.max(db, (deviceReduction.get(id) ?? 0) - 0.7));
  }
  for (const [id, db] of deviceReduction) if (!seen.has(id)) deviceReduction.set(id, Math.max(0, db - 0.7));
  if (frame.tap) [deviceSpectrum.id, deviceSpectrum.bands] = frame.tap;
  else deviceSpectrum.id = null;
  if (frame.limiter && frame.limiter > 0.05) {
    const now = performance.now();
    safetyActivity.db = now - safetyActivity.at < 1500 ? Math.max(safetyActivity.db, frame.limiter) : frame.limiter;
    safetyActivity.at = now;
  }
  clipProgress.clear();
  for (const [id, state] of frame.clips) clipProgress.set(id, state.progress);
  ingestAnalysis(frame.analysis);
}

function estimate(track: Track, beats: number, tempo: TempoMap): number {
  for (const clip of timelineClips(track)) {
    if (clip.kind === "audio") {
      const sec = clipSeconds(clip, tempo, clip.start, beats);
      if (sec < 0 || sec >= clip.duration) continue;
      const data = cachedPeaks(clip.asset);
      if (!data) return 0;
      const i = Math.floor(((clip.offset + sec) * data.sampleRate) / data.step);
      return Math.max(...data.channels.map((ch) => ch[i] ?? 0));
    }
  }
  return 0;
}

/** Browser preview only: approximate levels from the overview peaks. */
export function simulateMeters(s: Store, beats: number) {
  const { tracks, masterVolume } = s.project;
  const tempo = tempoOf(s.project);
  const anySolo = tracks.some((t) => t.solo);
  let sumSq = 0;
  for (const t of tracks) {
    const audible = s.playing && !t.mute && (!anySolo || t.solo);
    const level = audible ? estimate(t, beats, tempo) * dbToGain(t.volume) : 0;
    smooth(t.id, level);
    sumSq += level * level;
  }
  smooth("master", Math.min(1.2, Math.sqrt(sumSq) * dbToGain(masterVolume)));
}
