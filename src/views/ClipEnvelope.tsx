import { useState, type PointerEvent } from "react";
import { useStore } from "../state/store";
import type { AudioClip, EnvPoint } from "../state/types";

/** The envelope's scale: +6 dB at the top of the clip, silence (−60 dB) at the bottom. */
const TOP = 6;
const BOTTOM = -60;

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));

/**
 * An audio clip's gain over time, drawn over it. With the Automation tool
 * (A) it is edited here: click to add a point, drag a point, double-click
 * one to remove it. Lines between points are straight in dB, as they play.
 */
export function ClipEnvelope({ clip, lengthBeats, width, height, editable }: { clip: AudioClip; lengthBeats: number; width: number; height: number; editable: boolean }) {
  const [drag, setDrag] = useState<{ index: number; points: EnvPoint[]; changed: boolean } | null>(null);
  const points = drag?.points ?? clip.envelope ?? [];
  if (!editable && points.length === 0) return null;

  const pxPerBeat = lengthBeats > 0 ? width / lengthBeats : 1;
  const x = (time: number) => clamp(time * pxPerBeat, 0, width);
  const y = (gain: number) => ((TOP - clamp(gain, BOTTOM, TOP)) / (TOP - BOTTOM)) * height;
  const at = (e: PointerEvent<Element>, host: Element): EnvPoint => {
    const r = host.getBoundingClientRect();
    return {
      time: clamp((e.clientX - r.left) / pxPerBeat, 0, lengthBeats),
      gain: Math.round(clamp(TOP - ((e.clientY - r.top) / Math.max(1, r.height)) * (TOP - BOTTOM), BOTTOM, TOP) * 10) / 10,
    };
  };
  const commit = (next: EnvPoint[]) => useStore.getState().setClipEnvelope(clip.id, next);

  const onDown = (e: PointerEvent<HTMLDivElement>) => {
    if (!editable || e.button !== 0) return;
    // The clip itself stays put: this press edits its envelope.
    e.stopPropagation();
    e.currentTarget.setPointerCapture(e.pointerId);
    const grabbed = (e.target as HTMLElement).dataset.point;
    if (grabbed !== undefined) {
      setDrag({ index: Number(grabbed), points, changed: false });
      return;
    }
    const point = at(e, e.currentTarget);
    const next = [...points, point].sort((a, b) => a.time - b.time);
    setDrag({ index: next.indexOf(point), points: next, changed: true });
  };
  const onMove = (e: PointerEvent<HTMLDivElement>) => {
    if (!drag) return;
    const moved = at(e, e.currentTarget);
    // A point stays between its neighbours.
    const lo = drag.points[drag.index - 1]?.time ?? 0;
    const hi = drag.points[drag.index + 1]?.time ?? lengthBeats;
    const next = drag.points.map((p, i) => (i === drag.index ? { time: clamp(moved.time, lo, hi), gain: moved.gain } : p));
    setDrag({ index: drag.index, points: next, changed: true });
  };
  const onUp = () => {
    if (drag?.changed) commit(drag.points);
    setDrag(null);
  };

  const line = points.length
    ? `M0 ${y(points[0].gain)} ${points.map((p) => `L${x(p.time)} ${y(p.gain)}`).join(" ")} L${width} ${y(points[points.length - 1].gain)}`
    : `M0 ${y(0)} L${width} ${y(0)}`;
  const held = drag ? drag.points[drag.index] : null;

  return (
    <div
      className={`clip-env${editable ? " editable" : ""}${points.length ? "" : " empty"}`}
      onPointerDown={onDown}
      onPointerMove={onMove}
      onPointerUp={onUp}
      onPointerCancel={() => setDrag(null)}
      onDoubleClick={(e) => {
        if (!editable) return;
        e.stopPropagation();
        // The press before this one captured the pointer, so the event names the
        // overlay, not the point: the point under the pointer is found by distance.
        const r = e.currentTarget.getBoundingClientRect();
        const near = points.findIndex((p) => Math.hypot(x(p.time) - (e.clientX - r.left), y(p.gain) - (e.clientY - r.top)) <= 7);
        if (near >= 0) commit(points.filter((_, i) => i !== near));
      }}
      title={editable ? "Clip gain: click to add a point, drag it, double-click it to remove it" : undefined}
    >
      <svg width={width} height={height} aria-hidden="true">
        <path d={line} className="clip-env-line" />
      </svg>
      {editable &&
        points.map((p, i) => (
          <span key={i} data-point={i} className={`clip-env-pt${drag?.index === i ? " on" : ""}`} style={{ left: x(p.time), top: y(p.gain) }} />
        ))}
      {held && (
        <span className="clip-env-read mono" style={{ left: clamp(x(held.time) + 8, 0, Math.max(0, width - 56)), top: clamp(y(held.gain) - 18, 0, Math.max(0, height - 14)) }}>
          {held.gain <= BOTTOM ? "−∞" : `${held.gain > 0 ? "+" : held.gain < 0 ? "−" : ""}${Math.abs(held.gain).toFixed(1)}`} dB
        </span>
      )}
    </div>
  );
}
