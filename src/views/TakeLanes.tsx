import { useMemo, useState, type PointerEvent } from "react";
import { Waveform } from "../components/controls/ClipCanvas";
import { Icon } from "../components/Icon";
import { trackColor } from "../components/trackColor";
import { clipPeaks, usePeaks } from "../state/peaks";
import { useStore } from "../state/store";
import { tempoOf, type TempoMap } from "../state/tempo";
import { clipLength, snapBeats, timelineClips, type AudioClip, type Clip, type TakeLane, type Track } from "../state/types";

export const TAKE_H = 46;

/** A take's audio, drawn small. */
function TakeWave({ clip, width, color }: { clip: AudioClip; width: number; color: string }) {
  const missing = useStore((s) => s.project.assets.find((a) => a.id === clip.asset)?.missing ?? false);
  const data = usePeaks(missing ? undefined : clip.asset);
  const slice = useMemo(() => (data ? clipPeaks(data, clip.offset, clip.duration) : null), [data, clip.offset, clip.duration]);
  if (!slice) return null;
  return <Waveform peaks={slice[0]} peaksR={slice[1]} width={width} height={TAKE_H - 12} color={color} />;
}

/**
 * The stretches (beats) of a take that play on the track's timeline: where a
 * timeline clip shows the same moment of the same recording as the take does.
 */
function comped(lane: TakeLane, main: Clip[], tempo: TempoMap): [number, number][] {
  const out: [number, number][] = [];
  for (const take of lane.clips) {
    if (take.kind !== "audio") continue;
    const takeEnd = take.start + clipLength(take, tempo);
    for (const clip of main) {
      if (clip.kind !== "audio" || clip.asset !== take.asset || clip.warp) continue;
      // The same audio at the same time: both started the recording at the same point of the song.
      const shift = clip.offset - take.offset - (tempo.secondsAt(clip.start) - tempo.secondsAt(take.start));
      if (Math.abs(shift) > 0.005) continue;
      const from = Math.max(take.start, clip.start);
      const to = Math.min(takeEnd, clip.start + clipLength(clip, tempo));
      if (to - from > 1e-6) out.push([from, to]);
    }
  }
  return out;
}

/**
 * One take under its track. Swipe across it to put that stretch on the
 * track (over what is there); double-click to use the whole take. The parts
 * of it that play are lit.
 */
function TakeRow({ track, lane, laneWidth }: { track: Track; lane: TakeLane; laneWidth: number }) {
  const pxPerBeat = useStore((s) => s.pxPerBeat);
  const tempo = useStore((s) => tempoOf(s.project));
  const { compTake, deleteTake } = useStore.getState();
  const [swipe, setSwipe] = useState<{ from: number; to: number } | null>(null);
  const main = useMemo(() => timelineClips(track), [track]);
  const used = useMemo(() => comped(lane, main, tempo), [lane, main, tempo]);
  const color = trackColor(track.color);
  const span = lane.clips.reduce<[number, number] | null>((r, c) => {
    const end = c.start + clipLength(c, tempo);
    return r ? [Math.min(r[0], c.start), Math.max(r[1], end)] : [c.start, end];
  }, null);
  const whole = () => span && compTake(track.id, lane.id, span[0], span[1]);

  const beatAt = (e: PointerEvent<HTMLDivElement>) => {
    const s = useStore.getState();
    const beats = Math.max(0, (e.clientX - e.currentTarget.getBoundingClientRect().left) / pxPerBeat);
    return e.altKey ? beats : snapBeats(beats, s.snap, s.project.beatsPerBar);
  };
  const onDown = (e: PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    e.currentTarget.setPointerCapture(e.pointerId);
    const at = beatAt(e);
    setSwipe({ from: at, to: at });
  };
  const onMove = (e: PointerEvent<HTMLDivElement>) => swipe && setSwipe({ from: swipe.from, to: beatAt(e) });
  const onUp = () => {
    if (swipe && Math.abs(swipe.to - swipe.from) > 1e-6) compTake(track.id, lane.id, Math.min(swipe.from, swipe.to), Math.max(swipe.from, swipe.to));
    setSwipe(null);
  };

  return (
    <div className="arr-row take-row" style={{ height: TAKE_H }}>
      <div className="thead take-head">
        <span className="tcolor" aria-hidden="true" />
        <div className="take-top">
          <span className="take-name" title={`${lane.name} of ${track.name}`}>
            {lane.name}
          </span>
          <button className="btn xs" type="button" title="Put this whole take on the track" onClick={whole}>
            Use
          </button>
          <button className="ib xs" type="button" aria-label={`Delete ${lane.name}`} title="Delete this take (what was comped from it stays)" onClick={() => deleteTake(track.id, lane.id)}>
            <Icon name="close" size={13} />
          </button>
        </div>
      </div>
      <div
        className="lane take-lane"
        style={{ width: laneWidth }}
        role="group"
        aria-label={`${lane.name} of ${track.name}: swipe across a stretch to put it on the track`}
        title="Swipe across a stretch to put it on the track · double-click uses the whole take · Alt ignores the grid"
        onPointerDown={onDown}
        onPointerMove={onMove}
        onPointerUp={onUp}
        onPointerCancel={() => setSwipe(null)}
        onDoubleClick={whole}
      >
        {lane.clips.map((clip) => {
          const width = Math.max(4, clipLength(clip, tempo) * pxPerBeat - 2);
          return (
            <div key={clip.id} className="take-clip" style={{ left: clip.start * pxPerBeat + 1, width, ["--c" as string]: color }}>
              {clip.kind === "audio" && <TakeWave clip={clip} width={width} color={color} />}
            </div>
          );
        })}
        {used.map(([from, to]) => (
          <span key={`${from}-${to}`} className="take-used" style={{ left: from * pxPerBeat + 1, width: Math.max(2, (to - from) * pxPerBeat - 2), ["--c" as string]: color }} />
        ))}
        {swipe && Math.abs(swipe.to - swipe.from) > 1e-6 && (
          <span className="take-swipe" style={{ left: Math.min(swipe.from, swipe.to) * pxPerBeat, width: Math.abs(swipe.to - swipe.from) * pxPerBeat }} />
        )}
      </div>
    </div>
  );
}

/** A track's takes, newest at the bottom, while they are shown. */
export function TakeLanes({ track, laneWidth }: { track: Track; laneWidth: number }) {
  const open = useStore((s) => s.takesOpen[track.id] ?? false);
  if (!open || !track.takes?.length) return null;
  return (
    <>
      {track.takes.map((lane) => (
        <TakeRow key={lane.id} track={track} lane={lane} laneWidth={laneWidth} />
      ))}
    </>
  );
}
