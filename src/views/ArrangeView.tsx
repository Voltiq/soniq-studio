import { useEffect, useMemo, useRef, useState, type CSSProperties, type MouseEvent, type PointerEvent, type RefObject } from "react";
import { createPortal } from "react-dom";
import { fitArrangement } from "../commands";
import { DEVICE_DRAG_TYPE } from "../components/BrowserPanel";
import { NotePreview, Waveform } from "../components/controls/ClipCanvas";
import { Meter } from "../components/controls/Meter";
import { Icon } from "../components/Icon";
import { Popover } from "../components/Menu";
import { trackColor, trackVar } from "../components/trackColor";
import { formatDb } from "../state/format";
import { clipPeaks, usePeaks } from "../state/peaks";
import { WarpControls } from "../components/WarpControls";
import { trackHeight } from "../state/appearance";
import { positionBeats, tempoLaneShown, useStore } from "../state/store";
import { beatAfter, clipSeconds, tempoOf, type TempoMap } from "../state/tempo";
import { clipLength, projectLength, snapBeats, timelineClips, type Clip, type Track } from "../state/types";
import { AutomationRow, shownLanes } from "./AutomationLane";
import { TempoRow } from "./TempoLane";

export const HEADER_W = 224;
const CLIP_HEAD = 18;

function Ruler({ width, lengthBeats }: { width: number; lengthBeats: number }) {
  const pxPerBeat = useStore((s) => s.pxPerBeat);
  const bpb = useStore((s) => s.project.beatsPerBar);
  const loop = useStore((s) => s.loop);
  const snap = useStore((s) => s.snap);
  const punch = useStore((s) => s.punch);
  const { seekBeats, setLoopRange } = useStore.getState();
  const drag = useRef<number | null>(null);

  const barPx = pxPerBeat * bpb;
  const stride = barPx < 36 ? 4 : barPx < 72 ? 2 : 1;
  const bars = Math.ceil(lengthBeats / bpb) + 4;

  const toBeats = (e: PointerEvent<HTMLDivElement>) => {
    const x = e.clientX - e.currentTarget.getBoundingClientRect().left;
    return snapBeats(Math.max(0, x / pxPerBeat), snap === "off" ? "off" : snap, bpb);
  };

  return (
    <div
      className="ruler"
      style={{ width }}
      onPointerDown={(e) => {
        const b = toBeats(e);
        if (e.shiftKey) {
          e.currentTarget.setPointerCapture(e.pointerId);
          drag.current = b;
        } else {
          seekBeats(b);
        }
      }}
      onPointerMove={(e) => drag.current !== null && setLoopRange(drag.current, toBeats(e))}
      onPointerUp={() => (drag.current = null)}
      title="Click to move the playhead · Shift-drag to set the loop"
    >
      <div
        className={`loop-band${loop.enabled ? " on" : ""}${punch ? " punch" : ""}`}
        style={{ left: loop.start * pxPerBeat, width: (loop.end - loop.start) * pxPerBeat }}
        aria-hidden="true"
      />
      {Array.from({ length: Math.ceil(bars / stride) }, (_, i) => i * stride).map((bar) => (
        <span key={bar} className="bar-label" style={{ left: bar * barPx }}>
          {bar + 1}
        </span>
      ))}
    </div>
  );
}

interface MarkerDrag {
  id: string;
  mode: "move" | "start" | "end";
  x: number;
  dx: number;
  free: boolean;
}

/** Markers and regions (Audacity's labels): drag to move, drag a region's edges, double-click to rename. */
function MarkerLane({ width }: { width: number }) {
  const markers = useStore((s) => s.project.markers);
  const pxPerBeat = useStore((s) => s.pxPerBeat);
  const selected = useStore((s) => s.selectedMarkerId);
  const [editing, setEditing] = useState<string | null>(null);
  const [drag, setDrag] = useState<MarkerDrag | null>(null);
  const { selectMarker, moveMarker, renameMarker, seekBeats, run } = useStore.getState();
  const snap = (beats: number, free: boolean) => {
    const s = useStore.getState();
    return Math.max(0, free ? beats : snapBeats(beats, s.snap, s.project.beatsPerBar));
  };

  return (
    <div
      className="lane markers"
      style={{ width }}
      title="Double-click to add a marker"
      onDoubleClick={(e) => {
        if (e.target !== e.currentTarget) return;
        const x = e.clientX - e.currentTarget.getBoundingClientRect().left;
        void run({ op: "addMarker", start: snap(x / pxPerBeat, e.altKey) });
      }}
    >
      {markers.map((m) => {
        let { start, end } = m;
        if (drag?.id === m.id && Math.abs(drag.dx) > 3) {
          const delta = drag.dx / pxPerBeat;
          if (drag.mode === "move") {
            start = snap(m.start + delta, drag.free);
            if (end !== undefined) end += start - m.start;
          } else if (drag.mode === "start" && end !== undefined) {
            start = Math.min(end - MIN_BEATS, snap(m.start + delta, drag.free));
          } else if (drag.mode === "end" && end !== undefined) {
            end = Math.max(start + MIN_BEATS, snap(end + delta, drag.free));
          }
        }
        const region = end !== undefined;
        return (
          <div
            key={m.id}
            className={`marker${region ? " region" : ""}${selected === m.id ? " on" : ""}`}
            style={{ left: start * pxPerBeat, width: region ? Math.max(6, ((end ?? start) - start) * pxPerBeat) : undefined }}
            onPointerDown={(e) => {
              e.stopPropagation();
              if (e.button !== 0 || editing === m.id) return;
              e.currentTarget.setPointerCapture(e.pointerId);
              const mode = ((e.target as HTMLElement).dataset.edge as MarkerDrag["mode"] | undefined) ?? "move";
              setDrag({ id: m.id, mode, x: e.clientX, dx: 0, free: e.altKey });
            }}
            onPointerMove={(e) => drag?.id === m.id && setDrag({ ...drag, dx: e.clientX - drag.x, free: e.altKey })}
            onPointerUp={() => {
              const d = drag;
              setDrag(null);
              if (!d) return;
              if (Math.abs(d.dx) <= 3) {
                selectMarker(m.id);
                seekBeats(m.start);
              } else if (d.mode === "move") moveMarker(m.id, start);
              else moveMarker(m.id, start, end);
            }}
            onPointerCancel={() => setDrag(null)}
            onDoubleClick={(e) => {
              e.stopPropagation();
              setEditing(m.id);
            }}
            title={`${m.name} · drag to move${region ? ", drag the edges to resize" : ""} · double-click to rename`}
          >
            {region && <span className="mk-edge start" data-edge="start" aria-hidden="true" />}
            <span className="mk-flag">
              {editing === m.id ? (
                <input
                  className="mk-input"
                  defaultValue={m.name}
                  autoFocus
                  aria-label="Marker name"
                  onFocus={(e) => e.currentTarget.select()}
                  onBlur={(e) => {
                    setEditing(null);
                    if (e.currentTarget.value.trim() && e.currentTarget.value !== m.name) renameMarker(m.id, e.currentTarget.value);
                  }}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") e.currentTarget.blur();
                    if (e.key === "Escape") {
                      e.currentTarget.value = m.name;
                      e.currentTarget.blur();
                    }
                  }}
                />
              ) : (
                m.name
              )}
            </span>
            {region && <span className="mk-edge end" data-edge="end" aria-hidden="true" />}
          </div>
        );
      })}
    </div>
  );
}

function Playhead({ scroller }: { scroller: RefObject<HTMLDivElement | null> }) {
  const beats = useStore(positionBeats);
  const pxPerBeat = useStore((s) => s.pxPerBeat);
  const playing = useStore((s) => s.playing);
  const x = HEADER_W + beats * pxPerBeat;

  // Follow the playhead while playing, page by page like a tape machine.
  useEffect(() => {
    const el = scroller.current;
    if (!el || !playing) return;
    if (x > el.scrollLeft + el.clientWidth - 40 || x < el.scrollLeft + HEADER_W) {
      el.scrollLeft = Math.max(0, x - HEADER_W - 40);
    }
  }, [x, playing, scroller]);

  return <div className="playhead" style={{ transform: `translateX(${x}px)` }} aria-hidden="true" />;
}

type DragMode = "move" | "trim-start" | "trim-end" | "fade-in" | "fade-out";

interface Drag {
  mode: DragMode;
  x: number;
  y: number;
  dx: number;
  dy: number;
  /** Past the 3 px threshold: a drag, not a click. */
  moved: boolean;
  /** Alt bypasses the grid. */
  free: boolean;
}

const EDGE_PX = 6;
const MIN_BEATS = 1 / 16;

/** Which part of a clip the pointer is over. */
function zoneAt(e: PointerEvent<HTMLDivElement>): DragMode {
  const handle = (e.target as HTMLElement).dataset.handle as DragMode | undefined;
  if (handle) return handle;
  const rect = e.currentTarget.getBoundingClientRect();
  const x = e.clientX - rect.left;
  if (x <= EDGE_PX) return "trim-start";
  if (x >= rect.width - EDGE_PX) return "trim-end";
  return "move";
}

/** The clip as it would be if the drag ended now. */
function preview(clip: Clip, drag: Drag | null, tempo: TempoMap, pxPerBeat: number, snap: (b: number, free: boolean) => number, assetSeconds: number) {
  const length = clipLength(clip, tempo);
  const audio = clip.kind === "audio" ? clip : null;
  let start = clip.start;
  let len = length;
  let fadeIn = audio?.fadeIn ?? 0;
  let fadeOut = audio?.fadeOut ?? 0;
  if (drag?.moved) {
    const deltaBeats = drag.dx / pxPerBeat;
    const end = clip.start + length;
    // Seconds of the clip's audio a drag of `deltaBeats` covers.
    const dragSeconds = clipSeconds(clip, tempo, clip.start, clip.start + deltaBeats);
    switch (drag.mode) {
      case "move":
        start = Math.max(0, snap(clip.start + deltaBeats, drag.free));
        break;
      case "trim-start": {
        // Audio cannot reveal more than the file holds before its offset.
        const earliest = audio ? beatAfter(clip, tempo, clip.start, -audio.offset) : 0;
        start = Math.min(end - MIN_BEATS, Math.max(earliest, 0, snap(clip.start + deltaBeats, drag.free)));
        len = end - start;
        break;
      }
      case "trim-end": {
        const latest = audio ? beatAfter(clip, tempo, clip.start, assetSeconds - audio.offset) : Infinity;
        len = Math.min(latest, Math.max(clip.start + MIN_BEATS, snap(end + deltaBeats, drag.free))) - clip.start;
        break;
      }
      case "fade-in":
        if (audio) fadeIn = Math.min(Math.max(0, audio.fadeIn + dragSeconds), audio.duration - fadeOut);
        break;
      case "fade-out":
        if (audio) fadeOut = Math.min(Math.max(0, audio.fadeOut - dragSeconds), audio.duration - fadeIn);
        break;
    }
  }
  return { start, len, fadeIn, fadeOut };
}

/** Beats each clip overlaps the one before it, where they crossfade (mirrors the engine's rule). */
function crossfades(clips: Clip[], tempo: TempoMap): Map<string, number> {
  const out = new Map<string, number>();
  const sorted = clips.filter((c) => c.kind === "audio").sort((a, b) => a.start - b.start);
  for (let i = 1; i < sorted.length; i++) {
    const [a, b] = [sorted[i - 1], sorted[i]];
    const aEnd = a.start + clipLength(a, tempo);
    if (b.start > a.start && b.start < aEnd && b.start + clipLength(b, tempo) > aEnd) out.set(b.id, aEnd - b.start);
  }
  return out;
}

function ClipView({ track, clip, pxPerBeat, selected, xfade }: { track: Track; clip: Clip; pxPerBeat: number; selected: boolean; xfade: number }) {
  const { selectClip, setView, splitClip, moveClip, trimClip, setClipFades, run, openPianoRoll } = useStore.getState();
  const tempo = useStore((s) => tempoOf(s.project));
  const TRACK_H = useStore((s) => trackHeight(s.density));
  const assetSeconds = useStore((s) => {
    const a = clip.kind === "audio" ? s.project.assets.find((x) => x.id === clip.asset) : undefined;
    return a ? a.frames / a.sampleRate : Infinity;
  });
  const [drag, setDrag] = useState<Drag | null>(null);
  const snap = (beats: number, free: boolean) => {
    const s = useStore.getState();
    return free ? beats : snapBeats(beats, s.snap, s.project.beatsPerBar);
  };
  const p = preview(clip, drag, tempo, pxPerBeat, snap, assetSeconds);
  const width = Math.max(4, p.len * pxPerBeat - 2);
  const color = trackColor(track.color);
  const body = TRACK_H - 8 - CLIP_HEAD;
  // While trimming the left edge, the audio inside stays put on the timeline.
  const shiftBeats = p.start - clip.start;
  const shiftSeconds = clipSeconds(clip, tempo, clip.start, p.start);
  const audioSeconds = clipSeconds(clip, tempo, p.start, p.start + p.len);
  // Fades are seconds of audio; on screen they follow the clip's timing (warp or tempo track).
  const fadeInPx = (beatAfter(clip, tempo, p.start, p.fadeIn) - p.start) * pxPerBeat;
  const fadeOutPx = (p.start + p.len - beatAfter(clip, tempo, p.start, audioSeconds - p.fadeOut)) * pxPerBeat;

  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    e.stopPropagation();
    if (e.button !== 0) return;
    selectClip(track.id, clip.id);
    const tool = useStore.getState().tool;
    const beatAt = clip.start + (e.clientX - e.currentTarget.getBoundingClientRect().left) / pxPerBeat;
    if (tool === "split") return splitClip(clip.id, snap(beatAt, e.altKey));
    if (tool === "erase") return void run({ op: "deleteClip", clip: clip.id });
    e.currentTarget.setPointerCapture(e.pointerId);
    setDrag({ mode: zoneAt(e), x: e.clientX, y: e.clientY, dx: 0, dy: 0, moved: false, free: e.altKey });
  };

  const onPointerMove = (e: PointerEvent<HTMLDivElement>) => {
    if (!drag) {
      e.currentTarget.dataset.zone = zoneAt(e);
      return;
    }
    const dx = e.clientX - drag.x;
    const dy = e.clientY - drag.y;
    setDrag({ ...drag, dx, dy, moved: drag.moved || Math.abs(dx) > 3 || Math.abs(dy) > 3, free: e.altKey });
  };

  const onPointerUp = (e: PointerEvent<HTMLDivElement>) => {
    const d = drag;
    setDrag(null);
    if (!d?.moved) return;
    if (d.mode === "move") {
      // The lane under the pointer, if the clip was dragged to another track.
      const lane = document.elementsFromPoint(e.clientX, e.clientY).find((el) => (el as HTMLElement).dataset?.trackId) as HTMLElement | undefined;
      const target = lane?.dataset.trackId ?? track.id;
      if (target !== track.id || p.start !== clip.start) moveClip(clip.id, target, p.start);
    } else if (d.mode === "trim-start" || d.mode === "trim-end") {
      trimClip(clip.id, p.start, p.len);
    } else {
      setClipFades(clip.id, p.fadeIn, p.fadeOut);
    }
  };

  const dragging = drag?.moved ? ` dragging ${drag.mode}` : "";
  return (
    <div
      className={`clip${selected ? " on" : ""}${dragging}`}
      style={
        {
          left: p.start * pxPerBeat + 1,
          width,
          transform: drag?.moved && drag.mode === "move" ? `translateY(${drag.dy}px)` : undefined,
          "--c": trackVar(track.color),
        } as CSSProperties
      }
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={() => setDrag(null)}
      onDoubleClick={() => (clip.kind === "audio" ? setView("editor") : openPianoRoll(clip.id))}
      title={
        clip.kind === "audio"
          ? `${clip.name} · drag to move, edges to trim, corners to fade · double-click to edit`
          : `${clip.name} · drag to move, edges to trim · double-click to edit the notes`
      }
    >
      <div className="clip-name">
        {clip.kind === "midi" && clip.link && (
          <span className="clip-link" title="Linked: clips with this mark share their notes. Edit one and they all change. Clip › Unlink clip makes it its own.">
            <Icon name="link" size={11} />
          </span>
        )}
        {clip.kind === "audio" && clip.warp && (
          <span className="clip-link" title={`Warped: follows the song's tempo (its own is ${clip.warp.bpm} BPM, ${clip.warp.mode} mode)`}>
            <Icon name="warp" size={11} />
          </span>
        )}
        {clip.name}
      </div>
      <div className="clip-body">
        {clip.kind === "audio" ? (
          <>
            <AudioClipBody
              asset={clip.asset}
              offset={clip.offset + shiftSeconds}
              duration={audioSeconds}
              width={width}
              height={body}
              color={color}
            />
            <FadeOverlay width={width} height={body} fadeIn={fadeInPx} fadeOut={fadeOutPx} />
            {xfade > 0 && !drag?.moved && <CrossfadeMark width={Math.min(width, xfade * pxPerBeat)} height={body} />}
          </>
        ) : (
          <NotePreview
            notes={shiftBeats ? clip.notes.map((n) => ({ ...n, start: n.start - shiftBeats })) : clip.notes}
            lengthBeats={p.len}
            width={width}
            height={body}
            color={color}
          />
        )}
      </div>
      {clip.kind === "audio" && (
        <>
          <span className="fade-handle" data-handle="fade-in" style={{ left: Math.min(width - 6, fadeInPx) }} aria-hidden="true" />
          <span className="fade-handle" data-handle="fade-out" style={{ right: Math.min(width - 6, fadeOutPx) }} aria-hidden="true" />
        </>
      )}
    </div>
  );
}

/** Darkens the part of the clip a fade attenuates, with the fade line on top. */
function FadeOverlay({ width, height, fadeIn, fadeOut }: { width: number; height: number; fadeIn: number; fadeOut: number }) {
  if (fadeIn < 0.5 && fadeOut < 0.5) return null;
  return (
    <svg className="fade-overlay" width={width} height={height} aria-hidden="true">
      {fadeIn >= 0.5 && (
        <>
          <polygon points={`0,0 ${fadeIn},0 0,${height}`} />
          <line x1={0} y1={height} x2={fadeIn} y2={0} />
        </>
      )}
      {fadeOut >= 0.5 && (
        <>
          <polygon points={`${width - fadeOut},0 ${width},0 ${width},${height}`} />
          <line x1={width - fadeOut} y1={0} x2={width} y2={height} />
        </>
      )}
    </svg>
  );
}

/** The overlap with the clip before: one fades out while this one fades in. */
function CrossfadeMark({ width, height }: { width: number; height: number }) {
  if (width < 2) return null;
  return (
    <svg className="xfade" width={width} height={height} aria-label="Crossfade">
      <line x1={0} y1={height} x2={width} y2={0} />
      <line x1={0} y1={0} x2={width} y2={height} />
    </svg>
  );
}

interface AudioBodyProps {
  asset: string;
  offset: number;
  duration: number;
  width: number;
  height: number;
  color: string;
}

function AudioClipBody({ asset, offset, duration, width, height, color }: AudioBodyProps) {
  const missing = useStore((s) => s.project.assets.find((a) => a.id === asset)?.missing ?? false);
  const data = usePeaks(missing ? undefined : asset);
  const slice = useMemo(() => (data ? clipPeaks(data, offset, duration) : null), [data, offset, duration]);
  if (missing) return <span className="clip-missing">Missing audio file</span>;
  if (!slice) return null;
  return <Waveform peaks={slice[0]} peaksR={slice[1]} width={width} height={height} color={color} />;
}

/** The take growing under the playhead while recording (only inside the range when punching). */
function RecordingRegion({ pxPerBeat }: { pxPerBeat: number }) {
  const start = useStore((s) => s.recordStartBeats);
  const now = useStore(positionBeats);
  const end = useStore((s) => (s.punch ? s.loop.end : Infinity));
  if (start === null || now < start) return null;
  const width = Math.max(2, (Math.min(now, end) - start) * pxPerBeat);
  return (
    <div className="clip recording" style={{ left: start * pxPerBeat + 1, width }} aria-label="Recording">
      <div className="clip-name">
        <span className="rec-dot" aria-hidden="true" /> Recording
      </div>
    </div>
  );
}

/** Right-click on a track header: grouping, automation and freezing. */
function TrackMenu({ track, anchor, onClose }: { track: Track; anchor: HTMLElement; onClose: () => void }) {
  const tracks = useStore((s) => s.project.tracks);
  const picked = useStore((s) => s.trackSelection);
  const shown = useStore((s) => s.automationOpen[track.id] !== undefined);
  const { groupTracks, ungroup, setTrackGroup, toggleAutomation, removeTrack, freezeTrack, unfreezeTrack, flattenTrack } = useStore.getState();
  const groups = tracks.filter((t) => t.kind === "group" && t.id !== track.group);
  const pickable = (picked.includes(track.id) ? picked : [track.id]).filter((id) => tracks.find((t) => t.id === id)?.kind !== "group");
  const item = (label: string, run: () => void, key?: string) => (
    <button
      type="button"
      role="menuitem"
      className="mi"
      onClick={() => {
        onClose();
        run();
      }}
    >
      <span className="mi-check" />
      <span className="mi-label">{label}</span>
      {key && <span className="mi-k">{key}</span>}
    </button>
  );
  // Out of the sticky header (its own stacking layer, under the next track's), and its clicks stay out of the header's.
  return createPortal(
    <div onPointerDown={(e) => e.stopPropagation()} onContextMenu={(e) => e.preventDefault()}>
      <Popover anchor={anchor} onClose={onClose} label={`${track.name} menu`}>
        {pickable.length > 0 && item(pickable.length > 1 ? `Group ${pickable.length} tracks` : "Put in a new group", () => groupTracks(pickable), "Ctrl G")}
        {track.kind === "group" && item(`Ungroup ${track.name}`, () => ungroup(track.id), "Ctrl Shift G")}
        {track.group && item("Take out of the group", () => setTrackGroup(track.id, null))}
        {track.kind !== "group" && groups.map((g) => <div key={g.id}>{item(`Move into ${g.name}`, () => setTrackGroup(track.id, g.id))}</div>)}
        <div className="msep" role="separator" />
        {item(shown ? "Hide automation" : "Show automation", () => toggleAutomation(track.id))}
        {track.kind !== "group" && <div className="msep" role="separator" />}
        {track.kind !== "group" && !track.frozen && item(`Freeze ${track.name}`, () => void freezeTrack(track.id))}
        {track.frozen && item(`Unfreeze ${track.name}`, () => unfreezeTrack(track.id))}
        {track.frozen && item("Flatten to audio", () => flattenTrack(track.id))}
        <div className="msep" role="separator" />
        {item(`Delete ${track.name}`, () => removeTrack(track.id))}
      </Popover>
    </div>,
    document.body,
  );
}

function TrackHeader({ track, selected }: { track: Track; selected: boolean }) {
  const { selectTrack, toggleTrackSelection, updateTrack, toggleFold, toggleAutomation } = useStore.getState();
  const picked = useStore((s) => s.trackSelection.length > 1 && s.trackSelection.includes(track.id));
  const folded = useStore((s) => s.folded.includes(track.id));
  const members = useStore((s) => (track.kind === "group" ? s.project.tracks.filter((t) => t.group === track.id).length : 0));
  const groupColor = useStore((s) => (track.group ? s.project.tracks.find((t) => t.id === track.group)?.color : undefined));
  const automation = useStore((s) => s.automationOpen[track.id] !== undefined);
  const [menu, setMenu] = useState<HTMLElement | null>(null);
  const instrument = track.devices.find((d) => d.role === "instrument")?.name;
  const pct = ((track.volume + 60) / 66) * 100;
  const group = track.kind === "group";

  return (
    <div
      className={`thead${selected ? " on" : ""}${picked ? " picked" : ""}${group ? " group" : ""}${track.group ? " in-group" : ""}${track.frozen ? " frozen" : ""}`}
      style={groupColor ? ({ "--g": trackVar(groupColor) } as CSSProperties) : undefined}
      onPointerDown={(e) => (e.ctrlKey || e.metaKey || e.shiftKey ? toggleTrackSelection(track.id) : selectTrack(track.id))}
      onContextMenu={(e) => {
        e.preventDefault();
        setMenu(e.currentTarget);
      }}
    >
      <span className="tcolor" style={{ background: trackVar(track.color) }} />
      <div className="tinfo">
        <span className="tname">
          {group && (
            <button
              className="ib xs fold"
              type="button"
              aria-expanded={!folded}
              aria-label={`${folded ? "Show" : "Hide"} the tracks in ${track.name}`}
              onPointerDown={(e) => e.stopPropagation()}
              onClick={() => toggleFold(track.id)}
            >
              <Icon name={folded ? "chevronRight" : "chevronDown"} size={13} />
            </button>
          )}
          {track.name}
        </span>
        <span className="tsub">
          <Icon name={group ? "folder" : track.kind === "midi" ? "piano" : "wave"} size={13} />
          {group ? `Group · ${members} ${members === 1 ? "track" : "tracks"}` : track.kind === "midi" ? (instrument ?? "No instrument") : "Audio"}
          {track.frozen && <span className="tag frozen tip-b" data-tip="Plays its rendered sound while its devices rest">Frozen</span>}
        </span>
        <div className="tbtns">
          <button className="tg mute" type="button" aria-pressed={track.mute} aria-label={`Mute ${track.name}`} onClick={() => updateTrack(track.id, { mute: !track.mute })}>
            M
          </button>
          <button className="tg solo" type="button" aria-pressed={track.solo} aria-label={`Solo ${track.name}`} onClick={() => updateTrack(track.id, { solo: !track.solo })}>
            S
          </button>
          {!group && (
            <button className="tg arm" type="button" aria-pressed={track.arm} aria-label={`Arm ${track.name} for recording`} onClick={() => updateTrack(track.id, { arm: !track.arm })}>
              <span className="rec-dot" aria-hidden="true" />
            </button>
          )}
          <button
            className="tg auto"
            type="button"
            aria-pressed={automation}
            aria-label={`${automation ? "Hide" : "Show"} automation for ${track.name}`}
            title="Automation lanes"
            onClick={() => toggleAutomation(track.id)}
          >
            <Icon name="automation" size={13} />
          </button>
          <input
            className="range mini"
            type="range"
            min={-60}
            max={6}
            step={0.5}
            value={track.volume}
            aria-label={`${track.name} volume`}
            aria-valuetext={`${formatDb(track.volume)} dB`}
            style={{ "--pct": `${pct}%` } as CSSProperties}
            onChange={(e) => updateTrack(track.id, { volume: Number(e.target.value) })}
            onDoubleClick={() => updateTrack(track.id, { volume: 0 })}
          />
        </div>
      </div>
      <Meter id={track.id} label={track.name} width={5} />
      {menu && <TrackMenu track={track} anchor={menu} onClose={() => setMenu(null)} />}
    </div>
  );
}

/** A group's lane: the clips of its tracks, small, so a folded group still shows where things play. */
function GroupLane({ group, laneWidth, selected }: { group: Track; laneWidth: number; selected: boolean }) {
  const tracks = useStore((s) => s.project.tracks);
  const pxPerBeat = useStore((s) => s.pxPerBeat);
  const tempo = useStore((s) => tempoOf(s.project));
  const { selectTrack, seekBeats } = useStore.getState();
  const members = tracks.filter((t) => t.group === group.id);
  const rows = Math.max(1, members.length);
  return (
    <div
      className={`lane group-lane${selected ? " on" : ""}`}
      style={{ width: laneWidth }}
      data-track-id={group.id}
      data-track-kind="group"
      onPointerDown={(e) => {
        selectTrack(group.id);
        const s = useStore.getState();
        const x = e.clientX - e.currentTarget.getBoundingClientRect().left;
        seekBeats(snapBeats(x / pxPerBeat, s.snap, s.project.beatsPerBar));
      }}
    >
      {members.map((t, row) =>
        timelineClips(t).map((c) => (
          <span
            key={c.id}
            className="gclip"
            style={
              {
                left: c.start * pxPerBeat + 1,
                width: Math.max(3, clipLength(c, tempo) * pxPerBeat - 2),
                top: `calc(6px + ${row} * (100% - 12px) / ${rows})`,
                height: `calc((100% - 12px) / ${rows} - 2px)`,
                "--c": trackVar(t.color),
              } as CSSProperties
            }
            aria-hidden="true"
          />
        )),
      )}
      {!members.length && <span className="group-empty">Right-click a track header to move it into this group</span>}
    </div>
  );
}

function TrackRow({ track, laneWidth }: { track: Track; laneWidth: number }) {
  const selected = useStore((s) => s.selectedTrackId === track.id);
  const selectedClip = useStore((s) => s.selectedClipId);
  const pxPerBeat = useStore((s) => s.pxPerBeat);
  const recordingHere = useStore((s) => s.recording && track.arm);
  const tempo = useStore((s) => tempoOf(s.project));
  const clips = useMemo(() => timelineClips(track), [track]);
  const overlaps = useMemo(() => crossfades(clips, tempo), [clips, tempo]);
  const asked = useStore((s) => s.automationOpen[track.id]);
  const automationTool = useStore((s) => s.tool === "automation");
  const TRACK_H = useStore((s) => trackHeight(s.density));
  const lanes = asked !== undefined || automationTool ? shownLanes(track, asked) : [];
  const { selectTrack, seekBeats, addDevice, newMidiClip } = useStore.getState();
  if (track.kind === "group") {
    return (
      <>
        <div className={`arr-row group-row${track.mute ? " muted" : ""}`} style={{ height: TRACK_H }}>
          <TrackHeader track={track} selected={selected} />
          <GroupLane group={track} laneWidth={laneWidth} selected={selected} />
        </div>
        {lanes.map((target, i) => (
          <AutomationRow key={JSON.stringify(target)} track={track} target={target} laneWidth={laneWidth} last={i === lanes.length - 1} />
        ))}
      </>
    );
  }
  /** A one-bar MIDI clip at the bar under the pointer. */
  const newClipAt = (e: MouseEvent<HTMLDivElement>) => {
    const s = useStore.getState();
    const x = e.clientX - e.currentTarget.getBoundingClientRect().left;
    const bar = s.project.beatsPerBar;
    void newMidiClip(track.id, Math.floor(x / pxPerBeat / bar) * bar, bar);
  };

  return (
    <>
    <div className={`arr-row${track.mute ? " muted" : ""}`} style={{ height: TRACK_H }}>
      <TrackHeader track={track} selected={selected} />
      <div
        className={selected ? "lane on" : "lane"}
        style={{ width: laneWidth }}
        data-track-id={track.id}
        data-track-kind={track.kind}
        onPointerDown={(e) => {
          selectTrack(track.id);
          const s = useStore.getState();
          // The Draw tool makes MIDI clips on an empty stretch of a MIDI track.
          if (s.tool === "draw" && track.kind === "midi" && e.target === e.currentTarget) return newClipAt(e);
          const x = e.clientX - e.currentTarget.getBoundingClientRect().left;
          seekBeats(snapBeats(x / pxPerBeat, s.snap, s.project.beatsPerBar));
        }}
        onDoubleClick={(e) => track.kind === "midi" && e.target === e.currentTarget && newClipAt(e)}
        title={track.kind === "midi" ? "Double-click to make a MIDI clip" : undefined}
        onDragOver={(e) => e.dataTransfer.types.includes(DEVICE_DRAG_TYPE) && e.preventDefault()}
        onDrop={(e) => {
          const type = e.dataTransfer.getData(DEVICE_DRAG_TYPE);
          if (type) {
            selectTrack(track.id);
            addDevice(track.id, type);
          }
        }}
      >
        {clips.map((clip) => (
          <ClipView key={clip.id} track={track} clip={clip} pxPerBeat={pxPerBeat} selected={clip.id === selectedClip} xfade={overlaps.get(clip.id) ?? 0} />
        ))}
        {recordingHere && <RecordingRegion pxPerBeat={pxPerBeat} />}
      </div>
    </div>
    {lanes.map((target, i) => (
      <AutomationRow key={JSON.stringify(target)} track={track} target={target} laneWidth={laneWidth} last={i === lanes.length - 1} />
    ))}
    </>
  );
}

function ZoomHud() {
  const pxPerBeat = useStore((s) => s.pxPerBeat);
  const zoom = useStore((s) => s.zoom);
  return (
    <div className="hud glass" role="group" aria-label="Zoom">
      <button className="ib" type="button" aria-label="Zoom out" onClick={() => zoom(0.8)}>
        <Icon name="minus" size={15} />
      </button>
      <output className="zv">{Math.round((pxPerBeat / 14) * 100)}%</output>
      <button className="ib" type="button" aria-label="Zoom in" onClick={() => zoom(1.25)}>
        <Icon name="plus" size={15} />
      </button>
      <span className="vsep" />
      <button className="btn sm pill" type="button" onClick={fitArrangement}>
        Fit
      </button>
    </div>
  );
}

/** The selected audio clip's warping, floating over the arrangement. */
function ClipHud() {
  const clip = useStore((s) => {
    const c = s.selectedClipId ? s.project.tracks.flatMap((t) => t.clips).find((x) => x.id === s.selectedClipId) : undefined;
    return c?.kind === "audio" && c.slot == null ? c : undefined;
  });
  if (!clip) return null;
  return (
    <div className="clip-hud glass">
      <span className="clip-hud-name" title={clip.name}>
        {clip.name}
      </span>
      <WarpControls clip={clip} />
    </div>
  );
}

export function ArrangeView() {
  const allTracks = useStore((s) => s.project.tracks);
  const folded = useStore((s) => s.folded);
  // A folded group hides its tracks.
  const tracks = useMemo(() => allTracks.filter((t) => !t.group || !folded.includes(t.group)), [allTracks, folded]);
  const pxPerBeat = useStore((s) => s.pxPerBeat);
  const lengthBeats = useStore((s) => projectLength(s.project));
  const bpb = useStore((s) => s.project.beatsPerBar);
  const addTrack = useStore((s) => s.addTrack);
  const importAudio = useStore((s) => s.importAudio);
  const addMarker = useStore((s) => s.addMarker);
  const addRegionFromLoop = useStore((s) => s.addRegionFromLoop);
  const tool = useStore((s) => s.tool);
  const tempoShown = useStore(tempoLaneShown);
  const audioSelected = useStore((s) => {
    const c = s.selectedClipId ? s.project.tracks.flatMap((t) => t.clips).find((x) => x.id === s.selectedClipId) : undefined;
    return c?.kind === "audio" && c.slot == null;
  });
  const toggleTempoLane = useStore((s) => s.toggleTempoLane);
  const scroller = useRef<HTMLDivElement>(null);
  const laneWidth = (lengthBeats + 16) * pxPerBeat;

  return (
    <div className="arrange" data-tool={tool}>
      <div className="arr-scroll" ref={scroller}>
        <div
          className="arr-inner"
          style={{ width: HEADER_W + laneWidth, "--bar": `${pxPerBeat * bpb}px`, "--beat": `${pxPerBeat}px` } as CSSProperties}
        >
          <div className="arr-row ruler-row">
            <div className="corner">
              <span className="eyebrow">{allTracks.length} tracks</span>
            </div>
            <Ruler width={laneWidth} lengthBeats={lengthBeats} />
          </div>
          <div className="arr-row marker-row">
            <div className="corner">
              <span className="eyebrow">Markers</span>
              <button className="ib xs tip-b" type="button" aria-label="Add marker at the playhead" data-tip="Marker at the playhead  ·  Ctrl M" onClick={addMarker}>
                <Icon name="plus" size={13} />
              </button>
              <button className="btn xs tip-b" type="button" data-tip="Turn the loop range into a region  ·  Ctrl Shift M" onClick={addRegionFromLoop}>
                Region
              </button>
              <button className="btn xs tip-b" type="button" aria-pressed={tempoShown} data-tip="The tempo track: change the tempo over the song" onClick={toggleTempoLane}>
                Tempo
              </button>
            </div>
            <MarkerLane width={laneWidth} />
          </div>
          {tempoShown && <TempoRow laneWidth={laneWidth} />}
          {tracks.map((t) => (
            <TrackRow key={t.id} track={t} laneWidth={laneWidth} />
          ))}
          <div className="arr-row add-row">
            <div className="thead add">
              <button className="btn sm" type="button" onClick={() => addTrack("audio")}>
                <Icon name="plus" size={14} />
                Audio
              </button>
              <button className="btn sm" type="button" onClick={() => addTrack("midi")}>
                <Icon name="plus" size={14} />
                MIDI
              </button>
            </div>
            <div className="lane ghost" style={{ width: laneWidth }} data-new-track="true">
              <span>Drop audio files here to make new tracks, or</span>
              <button className="btn sm" type="button" onClick={() => void importAudio()}>
                Import audio… <span className="kbd">Ctrl I</span>
              </button>
            </div>
          </div>
          <Playhead scroller={scroller} />
        </div>
      </div>
      {audioSelected ? (
        <ClipHud />
      ) : (
        <div className="chip glass tipchip essentials-only">
          <Icon name="sparkle" size={14} />
          <span>
            <b>Space</b> plays · drag audio or MIDI files onto a track · double-click a clip to edit it · double-click a MIDI track to make a clip
          </span>
        </div>
      )}
      <ZoomHud />
    </div>
  );
}
