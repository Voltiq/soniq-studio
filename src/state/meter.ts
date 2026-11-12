/** Time signatures over the song: where the bar lines fall (mirrors `soniq_project::MeterMap`). */

export interface MeterChange {
  /** 0-based bar the change starts at (1 or later: the song's own signature covers bar 0). */
  bar: number;
  beatsPerBar: number;
}

/** A stretch of bars of one length. */
export interface MeterSegment {
  /** The beat its first bar starts on. */
  start: number;
  bar: number;
  beatsPerBar: number;
}

export class MeterMap {
  readonly segments: MeterSegment[];

  constructor(base: number, changes: MeterChange[] = []) {
    this.segments = [{ start: 0, bar: 0, beatsPerBar: Math.max(1, base) }];
    for (const change of [...changes].filter((c) => c.bar > 0).sort((a, b) => a.bar - b.bar)) {
      const last = this.segments[this.segments.length - 1];
      const beats = Math.max(1, change.beatsPerBar);
      if (change.bar <= last.bar || beats === last.beatsPerBar) continue;
      this.segments.push({ start: last.start + (change.bar - last.bar) * last.beatsPerBar, bar: change.bar, beatsPerBar: beats });
    }
  }

  /** Whether the bar length changes along the song. */
  get changes(): boolean {
    return this.segments.length > 1;
  }

  private segmentAt(beat: number): MeterSegment {
    let found = this.segments[0];
    for (const s of this.segments) if (s.start <= beat + 1e-9) found = s;
    return found;
  }

  /** The bar `beat` is in, how far into it (beats), and that bar's length. */
  barAt(beat: number): { bar: number; into: number; beatsPerBar: number } {
    const b = Math.max(0, beat);
    const seg = this.segmentAt(b);
    const bars = Math.floor((b - seg.start) / seg.beatsPerBar + 1e-9);
    return { bar: seg.bar + bars, into: b - seg.start - bars * seg.beatsPerBar, beatsPerBar: seg.beatsPerBar };
  }

  /** The beat bar `bar` starts on. */
  barStart(bar: number): number {
    let seg = this.segments[0];
    for (const s of this.segments) if (s.bar <= bar) seg = s;
    return seg.start + (bar - seg.bar) * seg.beatsPerBar;
  }

  /** The bar line nearest to `beat`. */
  nearestBar(beat: number): number {
    const { bar, into, beatsPerBar } = this.barAt(beat);
    return this.barStart(into * 2 >= beatsPerBar ? bar + 1 : bar);
  }
}

export function meterOf(project: { beatsPerBar: number; meters?: MeterChange[] }): MeterMap {
  return new MeterMap(project.beatsPerBar, project.meters ?? []);
}

/**
 * The open song's bar lines, for the helpers that are called with only the
 * song's own beats per bar (snapping to bars, the bar counter): the store
 * keeps this current.
 */
let song: MeterMap | null = null;

export function setSongMeters(map: MeterMap | null) {
  song = map;
}

export function songMeters(): MeterMap | null {
  return song?.changes ? song : null;
}
