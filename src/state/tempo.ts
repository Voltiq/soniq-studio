/*
 * The tempo track: where beats fall in time when the tempo changes. Mirrors
 * `soniq_project::TempoMap`: between two points the tempo glides in a
 * straight line (per beat), two points at one beat make a jump, and before
 * the first point and after the last the tempo holds.
 */
import type { Clip, Project, TempoPoint } from "./types";

export const MIN_BPM = 20;
export const MAX_BPM = 999;

interface Segment {
  beat: number;
  seconds: number;
  bpm: number;
  /** Tempo change per beat (0: constant). */
  slope: number;
}

const secondsFor = (s: Segment, beats: number) => (Math.abs(s.slope) < 1e-9 ? (beats * 60) / s.bpm : (60 / s.slope) * Math.log((s.bpm + s.slope * beats) / s.bpm));
const beatsFor = (s: Segment, seconds: number) => (Math.abs(s.slope) < 1e-9 ? (seconds * s.bpm) / 60 : (s.bpm * (Math.exp((s.slope * seconds) / 60) - 1)) / s.slope);
const clampBpm = (bpm: number) => Math.min(MAX_BPM, Math.max(MIN_BPM, bpm));

export class TempoMap {
  readonly segments: Segment[];

  constructor(bpm: number, points: TempoPoint[] = []) {
    const sorted = points.filter((p) => Number.isFinite(p.time) && Number.isFinite(p.bpm)).map((p) => ({ time: Math.max(0, p.time), bpm: clampBpm(p.bpm) }));
    sorted.sort((a, b) => a.time - b.time);
    if (!sorted.length) {
      this.segments = [{ beat: 0, seconds: 0, bpm: clampBpm(bpm), slope: 0 }];
      return;
    }
    const parts: [number, number, number][] = [[0, sorted[0].bpm, 0]];
    for (let i = 0; i + 1 < sorted.length; i++) {
      const [a, b] = [sorted[i], sorted[i + 1]];
      if (b.time - a.time > 1e-9) parts.push([a.time, a.bpm, (b.bpm - a.bpm) / (b.time - a.time)]);
    }
    const last = sorted[sorted.length - 1];
    parts.push([last.time, last.bpm, 0]);
    const segments: Segment[] = [];
    for (const [beat, segBpm, slope] of parts) {
      const prev = segments[segments.length - 1];
      const seconds = prev ? prev.seconds + secondsFor(prev, beat - prev.beat) : 0;
      if (prev && beat - prev.beat < 1e-9) segments.pop();
      segments.push({ beat, seconds, bpm: segBpm, slope });
    }
    this.segments = segments;
  }

  private byBeat(beats: number): Segment {
    let i = this.segments.length - 1;
    while (i > 0 && this.segments[i].beat > beats) i--;
    return this.segments[i];
  }

  private bySeconds(seconds: number): Segment {
    let i = this.segments.length - 1;
    while (i > 0 && this.segments[i].seconds > seconds) i--;
    return this.segments[i];
  }

  secondsAt(beats: number): number {
    const s = this.byBeat(beats);
    return s.seconds + secondsFor(s, beats - s.beat);
  }

  beatsAt(seconds: number): number {
    const s = this.bySeconds(seconds);
    return s.beat + beatsFor(s, seconds - s.seconds);
  }

  bpmAt(beats: number): number {
    const s = this.byBeat(beats);
    return clampBpm(s.bpm + s.slope * Math.max(0, beats - s.beat));
  }

  get constant(): boolean {
    return this.segments.length === 1 && this.segments[0].slope === 0;
  }
}

let cached: { bpm: number; points: TempoPoint[] | undefined; map: TempoMap } | null = null;

/** The song's tempo map (remembered while the tempo and its track stay the same). */
export function tempoOf(project: Pick<Project, "bpm" | "tempo">): TempoMap {
  if (!cached || cached.bpm !== project.bpm || cached.points !== project.tempo) {
    cached = { bpm: project.bpm, points: project.tempo, map: new TempoMap(project.bpm, project.tempo) };
  }
  return cached.map;
}

let looping: TempoMap | null = null;

/** Session clips loop at the song's starting tempo, whatever the tempo track does later (mirrors the engine's `loop_beat`). */
export function loopTempo(project: Pick<Project, "bpm" | "tempo">): TempoMap {
  const bpm = tempoOf(project).bpmAt(0);
  if (!looping || looping.bpmAt(0) !== bpm) looping = new TempoMap(bpm);
  return looping;
}

/** Seconds of a clip's audio between two timeline beats: warped clips count in their own tempo (mirrors `Clip::audio_seconds`). */
export function clipSeconds(clip: Clip, tempo: TempoMap, from: number, to: number): number {
  if (clip.kind === "audio" && clip.warp) return ((to - from) * 60) / clip.warp.bpm;
  return tempo.secondsAt(to) - tempo.secondsAt(from);
}

/** The timeline beat `seconds` of a clip's audio after beat `from` (mirrors `Clip::beat_after`). */
export function beatAfter(clip: Clip, tempo: TempoMap, from: number, seconds: number): number {
  if (clip.kind === "audio" && clip.warp) return from + (seconds * clip.warp.bpm) / 60;
  return tempo.beatsAt(tempo.secondsAt(from) + seconds);
}
