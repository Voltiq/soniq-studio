import { useState, type PointerEvent } from "react";
import { Icon } from "../components/Icon";
import { useStore } from "../state/store";
import { MAX_BPM, MIN_BPM } from "../state/tempo";
import { snapBeats, type TempoPoint } from "../state/types";
import { AUTO_H } from "./AutomationLane";

const PAD = 7;
const clamp01 = (v: number) => Math.min(1, Math.max(0, v));
const formatBpm = (bpm: number) => `${Math.round(bpm * 10) / 10}`;

/** The song's tempos with room around them: at least 80 BPM tall, so a small change still shows. */
function bpmRange(points: TempoPoint[], bpm: number): [number, number] {
  const all = [bpm, ...points.map((p) => p.bpm)];
  const [slow, fast] = [Math.min(...all), Math.max(...all)];
  const room = Math.max(20, (80 - (fast - slow)) / 2);
  return [Math.max(MIN_BPM, slow - room), Math.min(MAX_BPM, fast + room)];
}

interface Drag {
  index: number;
  points: TempoPoint[];
}

/**
 * The tempo track under the markers: breakpoints joined by lines, where the
 * tempo glides from one point to the next and holds after the last. Click to
 * add a point, drag one to move it, double-click or right-click to delete it.
 * Alt moves off the grid and in tenths of a BPM.
 */
export function TempoRow({ laneWidth }: { laneWidth: number }) {
  const pxPerBeat = useStore((s) => s.pxPerBeat);
  const saved = useStore((s) => s.project.tempo);
  const bpm = useStore((s) => s.project.bpm);
  const { setTempoPoints, toggleTempoLane } = useStore.getState();
  const [drag, setDrag] = useState<Drag | null>(null);
  const points = drag?.points ?? saved ?? [];
  // The range follows the saved points, so it holds still while dragging.
  const [lo, hi] = bpmRange(saved ?? [], bpm);

  const y = (v: number) => PAD + (1 - clamp01((v - lo) / (hi - lo))) * (AUTO_H - 2 * PAD);
  const bpmAt = (py: number, fine: boolean) => {
    const v = lo + (hi - lo) * clamp01(1 - (py - PAD) / (AUTO_H - 2 * PAD));
    return fine ? Math.round(v * 10) / 10 : Math.round(v);
  };
  const timeAt = (px: number, free: boolean) => {
    const s = useStore.getState();
    return Math.max(0, free ? px / pxPerBeat : snapBeats(px / pxPerBeat, s.snap, s.project.beatsPerBar));
  };
  const local = (e: PointerEvent<Element>) => {
    const rect = (e.currentTarget.closest(".tempo-lane") as HTMLElement).getBoundingClientRect();
    return { x: e.clientX - rect.left, y: e.clientY - rect.top };
  };

  const commit = (next: TempoPoint[]) => setTempoPoints([...next].sort((a, b) => a.time - b.time));
  const remove = (index: number) => commit(points.filter((_, i) => i !== index));

  const onLaneDown = (e: PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0 || (e.target as HTMLElement).dataset.point) return;
    e.currentTarget.setPointerCapture(e.pointerId);
    const at = local(e);
    const point = { time: timeAt(at.x, e.altKey), bpm: bpmAt(at.y, e.altKey) };
    // The first change starts from the song's tempo at the beginning.
    const start = points.length || point.time === 0 ? [] : [{ time: 0, bpm }];
    const next = [...start, ...points, point].sort((a, b) => a.time - b.time);
    setDrag({ index: next.indexOf(point), points: next });
  };
  const onPointDown = (e: PointerEvent<HTMLElement>, index: number) => {
    if (e.button !== 0) return;
    e.stopPropagation();
    (e.currentTarget.closest(".tempo-lane") as HTMLElement).setPointerCapture(e.pointerId);
    setDrag({ index, points });
  };
  const onMove = (e: PointerEvent<HTMLDivElement>) => {
    if (!drag) return;
    const at = local(e);
    const prev = drag.points[drag.index - 1]?.time ?? 0;
    const next = drag.points[drag.index + 1]?.time ?? Infinity;
    const moved = { time: Math.min(next, Math.max(prev, timeAt(at.x, e.altKey))), bpm: bpmAt(at.y, e.altKey) };
    setDrag({ ...drag, points: drag.points.map((p, i) => (i === drag.index ? moved : p)) });
  };
  const onUp = () => {
    if (drag) commit(drag.points);
    setDrag(null);
  };

  const xy = points.map((p) => [p.time * pxPerBeat, y(p.bpm)] as const);
  const line = xy.length
    ? `M0 ${xy[0][1]} ${xy.map(([x, py]) => `L${x} ${py}`).join(" ")} L${laneWidth} ${xy[xy.length - 1][1]}`
    : `M0 ${y(bpm)} L${laneWidth} ${y(bpm)}`;
  const slowest = Math.min(...points.map((p) => p.bpm));
  const fastest = Math.max(...points.map((p) => p.bpm));
  // Labels where there is room for them; a jump's new tempo goes under its point.
  const jump = xy.map(([x], i) => i > 0 && x - xy[i - 1][0] < 1);
  const labelled = xy.map(([x], i) => i === 0 || jump[i] || x - xy[i - 1][0] > 34);

  return (
    <div className="arr-row auto-row tempo-row" style={{ height: AUTO_H }}>
      <div className="thead auto-head">
        <span className="tcolor" aria-hidden="true" />
        <div className="tinfo">
          <div className="auto-top">
            <span className="auto-name">
              <Icon name="metronome" size={13} className="dim" />
              Tempo
            </span>
            {points.length > 0 && (
              <button className="ib xs" type="button" aria-label="Clear the tempo track" title="Back to one tempo for the whole song" onClick={() => commit([])}>
                <Icon name="trash" size={13} />
              </button>
            )}
            <button className="ib xs" type="button" aria-label="Hide the tempo lane" title="Hide this lane (the tempo changes stay)" onClick={toggleTempoLane}>
              <Icon name="close" size={13} />
            </button>
          </div>
          <span className="tsub">
            {points.length
              ? slowest === fastest
                ? `${formatBpm(slowest)} BPM throughout`
                : `${formatBpm(slowest)}–${formatBpm(fastest)} BPM · ${points.length} points`
              : `${formatBpm(bpm)} BPM · click the lane to change it over time`}
          </span>
        </div>
      </div>
      <div
        className={`lane auto-lane tempo-lane${points.length ? "" : " empty"}`}
        style={{ width: laneWidth }}
        onPointerDown={onLaneDown}
        onPointerMove={onMove}
        onPointerUp={onUp}
        onPointerCancel={() => setDrag(null)}
        title="Tempo · click to add a point, drag points to move them, double-click one to delete it · Alt for fine steps"
      >
        <svg className="auto-svg" width={laneWidth} height={AUTO_H} aria-hidden="true">
          {xy.length > 0 && <path className="auto-fill" d={`${line} L${laneWidth} ${AUTO_H} L0 ${AUTO_H} Z`} />}
          <path className="auto-line" d={line} />
        </svg>
        {points.map((p, i) => (
          <span key={i} style={{ display: "contents" }}>
            {labelled[i] && drag?.index !== i && (
              <span className={`tempo-val${jump[i] ? " below" : ""}`} style={{ left: p.time * pxPerBeat, top: y(p.bpm) }} aria-hidden="true">
                {formatBpm(p.bpm)}
              </span>
            )}
            <span
              className={`auto-pt${drag?.index === i ? " on" : ""}`}
              data-point="true"
              style={{ left: p.time * pxPerBeat, top: y(p.bpm) }}
              title={`${formatBpm(p.bpm)} BPM · beat ${Math.round(p.time * 100) / 100 + 1}`}
              onPointerDown={(e) => onPointDown(e, i)}
              onDoubleClick={(e) => {
                e.stopPropagation();
                remove(i);
              }}
              onContextMenu={(e) => {
                e.preventDefault();
                remove(i);
              }}
            />
          </span>
        ))}
        {drag && points[drag.index] && (
          <span className="auto-readout" style={{ left: points[drag.index].time * pxPerBeat, top: y(points[drag.index].bpm) }}>
            {formatBpm(points[drag.index].bpm)} BPM
          </span>
        )}
      </div>
    </div>
  );
}
