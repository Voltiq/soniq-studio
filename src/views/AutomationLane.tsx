import { useState, type PointerEvent } from "react";
import { Icon } from "../components/Icon";
import { autoRange, isMidiFx, isModulator, targetLabel } from "../state/devices";
import { formatDb, formatPan, formatParam } from "../state/format";
import { useStore } from "../state/store";
import { laneValue, sameAutoTarget, snapBeats, type AutoPoint, type AutoTarget, type Track } from "../state/types";
import { Select } from "../components/Select";

export const AUTO_H = 58;
const PAD = 7;
const clamp01 = (v: number) => Math.min(1, Math.max(0, v));

/** The lanes a track shows: those with points and those asked for, or Volume to start with. */
export function shownLanes(track: Track, asked: AutoTarget[] | undefined): AutoTarget[] {
  const lanes = (track.automation ?? []).map((l) => l.target);
  for (const t of asked ?? []) if (!lanes.some((l) => sameAutoTarget(l, t))) lanes.push(t);
  return lanes.length ? lanes : [{ kind: "volume" }];
}

/** What a target is set to without automation: the fader, the send, the knob. */
function setValue(track: Track, target: AutoTarget): number {
  switch (target.kind) {
    case "volume":
      return track.volume;
    case "pan":
      return track.pan;
    case "send":
      return track.sends[target.index] ?? 0;
    case "param":
      return track.devices.find((d) => d.id === target.device)?.params.find((p) => p.id === target.param)?.value ?? 0;
  }
}

function formatValue(track: Track, target: AutoTarget, value: number): string {
  switch (target.kind) {
    case "volume":
      return `${formatDb(value)} dB`;
    case "pan":
      return formatPan(value);
    case "send":
      return `${Math.round(value * 100)}%`;
    case "param": {
      const param = autoRange(track, target).param;
      return param ? formatParam({ ...param, value }) : value.toFixed(2);
    }
  }
}

/** Everything on a track that automation can move: the mixer, the instrument and the audio effects. */
function laneChoices(track: Track): { group: string; items: { label: string; target: AutoTarget }[] }[] {
  const mixer = [
    { label: "Volume", target: { kind: "volume" } as AutoTarget },
    { label: "Pan", target: { kind: "pan" } as AutoTarget },
    { label: "Send A", target: { kind: "send", index: 0 } as AutoTarget },
    { label: "Send B", target: { kind: "send", index: 1 } as AutoTarget },
  ];
  const devices = track.devices
    .filter((d) => !isModulator(d.type) && !isMidiFx(d.type))
    .map((d) => ({
      group: d.name,
      items: d.params
        .filter((p) => !p.options && !p.group?.startsWith("pad:"))
        .map((p) => ({ label: p.label, target: { kind: "param", device: d.id, param: p.id } as AutoTarget })),
    }));
  return [{ group: "Mixer", items: mixer }, ...devices];
}

interface Drag {
  index: number;
  points: AutoPoint[];
}

/**
 * One automation lane under its track: breakpoints joined by lines. Click
 * to add a point (and drag it), drag a point to move it, double-click or
 * right-click a point to delete it. Alt moves off the grid.
 */
export function AutomationRow({ track, target, laneWidth, last }: { track: Track; target: AutoTarget; laneWidth: number; last: boolean }) {
  const pxPerBeat = useStore((s) => s.pxPerBeat);
  const asked = useStore((s) => s.automationOpen[track.id]);
  const { setAutomation, hideAutomationLane, showAutomation } = useStore.getState();
  const [drag, setDrag] = useState<Drag | null>(null);
  const lane = track.automation?.find((l) => sameAutoTarget(l.target, target));
  const points = drag?.points ?? lane?.points ?? [];
  const range = autoRange(track, target);
  const label = targetLabel(track, target);

  const norm = (v: number) => {
    if (range.max === range.min) return 0.5;
    return range.log ? Math.log(v / range.min) / Math.log(range.max / range.min) : (v - range.min) / (range.max - range.min);
  };
  const y = (v: number) => PAD + (1 - clamp01(norm(v))) * (AUTO_H - 2 * PAD);
  const valueAt = (py: number) => {
    const t = clamp01(1 - (py - PAD) / (AUTO_H - 2 * PAD));
    let v = range.log ? range.min * Math.pow(range.max / range.min, t) : range.min + (range.max - range.min) * t;
    const step = range.param?.step;
    if (step) v = range.min + Math.round((v - range.min) / step) * step;
    return Math.min(range.max, Math.max(range.min, v));
  };
  const timeAt = (px: number, free: boolean) => {
    const s = useStore.getState();
    return Math.max(0, free ? px / pxPerBeat : snapBeats(px / pxPerBeat, s.snap, s.project.beatsPerBar));
  };
  const local = (e: PointerEvent<Element>) => {
    const rect = (e.currentTarget.closest(".auto-lane") as HTMLElement).getBoundingClientRect();
    return { x: e.clientX - rect.left, y: e.clientY - rect.top };
  };

  const commit = (next: AutoPoint[]) => setAutomation(track.id, target, [...next].sort((a, b) => a.time - b.time));
  const remove = (index: number) => commit(points.filter((_, i) => i !== index));

  const onLaneDown = (e: PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0 || (e.target as HTMLElement).dataset.point) return;
    e.currentTarget.setPointerCapture(e.pointerId);
    const at = local(e);
    const point = { time: timeAt(at.x, e.altKey), value: valueAt(at.y) };
    const next = [...points, point].sort((a, b) => a.time - b.time);
    setDrag({ index: next.indexOf(point), points: next });
  };
  const onPointDown = (e: PointerEvent<HTMLElement>, index: number) => {
    if (e.button !== 0) return;
    e.stopPropagation();
    (e.currentTarget.closest(".auto-lane") as HTMLElement).setPointerCapture(e.pointerId);
    setDrag({ index, points });
  };
  const onMove = (e: PointerEvent<HTMLDivElement>) => {
    if (!drag) return;
    const at = local(e);
    const prev = drag.points[drag.index - 1]?.time ?? 0;
    const next = drag.points[drag.index + 1]?.time ?? Infinity;
    const moved = { time: Math.min(next, Math.max(prev, timeAt(at.x, e.altKey))), value: valueAt(at.y) };
    setDrag({ ...drag, points: drag.points.map((p, i) => (i === drag.index ? moved : p)) });
  };
  const onUp = () => {
    if (drag) commit(drag.points);
    setDrag(null);
  };

  const fallback = setValue(track, target);
  const xy = points.map((p) => [p.time * pxPerBeat, y(p.value)] as const);
  const line = xy.length
    ? `M0 ${xy[0][1]} ${xy.map(([x, py]) => `L${x} ${py}`).join(" ")} L${laneWidth} ${xy[xy.length - 1][1]}`
    : `M0 ${y(fallback)} L${laneWidth} ${y(fallback)}`;
  const asks = asked?.some((t) => sameAutoTarget(t, target)) ?? false;
  const start = laneValue(points, 0) ?? fallback;

  return (
    <div className="arr-row auto-row" style={{ height: AUTO_H }}>
      <div className="thead auto-head">
        <span className="tcolor" aria-hidden="true" />
        <div className="tinfo">
          <div className="auto-top">
            <span className="auto-name" title={label}>
              <Icon name="automation" size={13} className="dim" />
              {label}
            </span>
            {points.length > 0 && (
              <button className="ib xs" type="button" aria-label={`Clear ${label} automation`} title="Clear this lane" onClick={() => commit([])}>
                <Icon name="trash" size={13} />
              </button>
            )}
            {!points.length && asks && (
              <button className="ib xs" type="button" aria-label={`Hide the ${label} lane`} title="Hide this lane" onClick={() => hideAutomationLane(track.id, target)}>
                <Icon name="close" size={13} />
              </button>
            )}
            {last && (
              <span className="ib xs auto-add" title="Show another lane">
                <Icon name="plus" size={13} />
                <Select
                  aria-label={`Show another automation lane on ${track.name}`}
                  value=""
                  onChange={(e) => showAutomation(track.id, JSON.parse(e.target.value) as AutoTarget)}
                >
                  <option value="" disabled>
                    Show a lane for…
                  </option>
                  {laneChoices(track).map((g) => (
                    <optgroup key={g.group} label={g.group}>
                      {g.items.map((it) => (
                        <option key={it.label} value={JSON.stringify(it.target)}>
                          {it.label}
                        </option>
                      ))}
                    </optgroup>
                  ))}
                </Select>
              </span>
            )}
          </div>
          <span className="tsub">{points.length ? `${points.length} ${points.length === 1 ? "point" : "points"} · starts at ${formatValue(track, target, start)}` : "Click the lane to draw"}</span>
        </div>
      </div>
      <div
        className={`lane auto-lane${points.length ? "" : " empty"}`}
        style={{ width: laneWidth }}
        onPointerDown={onLaneDown}
        onPointerMove={onMove}
        onPointerUp={onUp}
        onPointerCancel={() => setDrag(null)}
        title={`${label} · click to add a point, drag points to move them, double-click one to delete it`}
      >
        <svg className="auto-svg" width={laneWidth} height={AUTO_H} aria-hidden="true">
          {xy.length > 0 && <path className="auto-fill" d={`${line} L${laneWidth} ${AUTO_H} L0 ${AUTO_H} Z`} />}
          <path className="auto-line" d={line} />
        </svg>
        {points.map((p, i) => (
          <span
            key={i}
            className={`auto-pt${drag?.index === i ? " on" : ""}`}
            data-point="true"
            style={{ left: p.time * pxPerBeat, top: y(p.value) }}
            title={`${formatValue(track, target, p.value)} · beat ${Math.round(p.time * 100) / 100 + 1}`}
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
        ))}
        {drag && points[drag.index] && (
          <span className="auto-readout" style={{ left: points[drag.index].time * pxPerBeat, top: y(points[drag.index].value) }}>
            {formatValue(track, target, points[drag.index].value)}
          </span>
        )}
      </div>
    </div>
  );
}
