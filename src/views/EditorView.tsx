import { useEffect, useMemo, useRef, useState, type PointerEvent } from "react";
import { useShallow } from "zustand/react/shallow";
import { Waveform } from "../components/controls/ClipCanvas";
import { Icon } from "../components/Icon";
import { Popover } from "../components/Menu";
import { trackColor } from "../components/trackColor";
import { backend } from "../engine/backend";
import { EFFECT_GROUPS, EFFECTS } from "../state/effects";
import { EDIT_LABELS } from "../state/edits";
import { formatClock } from "../state/format";
import { clipPeaks, usePeaks } from "../state/peaks";
import { editorTarget, positionBeats, useStore, type SpectralMode } from "../state/store";
import { beatAfter, clipSeconds, tempoOf } from "../state/tempo";
import type { AudioClip, FrequencyScale, SimpleEditOp } from "../state/types";
import { DetailWave, SpectrogramCanvas, formatHz, frequencyTicks, hzAt, posOf, useLatest } from "./editorCanvas";

const QUICK: SimpleEditOp[] = ["normalize", "amplify", "fadeIn", "fadeOut", "reverse", "silence"];
const SCALES: [FrequencyScale, string][] = [
  ["log", "Log"],
  ["mel", "Mel"],
  ["linear", "Linear"],
];

/** Tracks an element's size; a callback ref so it also works when the element mounts later. */
function useSize<T extends HTMLElement>() {
  const [el, setEl] = useState<T | null>(null);
  const [size, setSize] = useState({ width: 0, height: 0 });
  useEffect(() => {
    if (!el) return;
    const ro = new ResizeObserver(([entry]) =>
      setSize({ width: Math.floor(entry.contentRect.width), height: Math.floor(entry.contentRect.height) }),
    );
    ro.observe(el);
    return () => ro.disconnect();
  }, [el]);
  return [setEl, size, el] as const;
}

/** A ruler step that leaves room for labels, and a matching label format. */
function rulerTicks(start: number, end: number, width: number) {
  const span = Math.max(1e-6, end - start);
  const steps = [0.0005, 0.001, 0.002, 0.005, 0.01, 0.02, 0.05, 0.1, 0.25, 0.5, 1, 2, 5, 10, 30, 60, 120, 300];
  const step = steps.find((s) => (s / span) * width >= 70) ?? 600;
  const decimals = Math.max(0, -Math.floor(Math.log10(step)));
  const ticks: number[] = [];
  for (let t = Math.ceil(start / step) * step; t <= end + 1e-9; t += step) ticks.push(t);
  const label = (t: number) => (t >= 60 ? `${Math.floor(t / 60)}:${(t % 60).toFixed(decimals).padStart(decimals ? 3 + decimals : 2, "0")}` : `${t.toFixed(decimals)}s`);
  return { ticks, label };
}

function EditorPlayhead({ clip, width, viewStart, span }: { clip: AudioClip; width: number; viewStart: number; span: number }) {
  const beats = useStore(positionBeats);
  const tempo = useStore((s) => tempoOf(s.project));
  const sec = clipSeconds(clip, tempo, clip.start, beats);
  if (sec < viewStart || sec > viewStart + span) return null;
  return <div className="ed-playhead" style={{ transform: `translateX(${((sec - viewStart) / span) * width}px)` }} aria-hidden="true" />;
}

function EffectsMenu({ disabled }: { disabled: boolean }) {
  const [anchor, setAnchor] = useState<HTMLElement | null>(null);
  const { edit, openEffect } = useStore.getState();
  const close = () => setAnchor(null);
  return (
    <>
      <button className="btn sm" type="button" aria-haspopup="menu" disabled={disabled} onClick={(e) => setAnchor(anchor ? null : e.currentTarget)}>
        <Icon name="fx" size={14} />
        Effects
        <Icon name="chevronDown" size={13} />
      </button>
      {anchor && (
        <Popover anchor={anchor} onClose={close} label="Effects">
          <div className="mi-menu mi-group">Quick</div>
          {QUICK.map((op) => (
            <button
              key={op}
              type="button"
              role="menuitem"
              className="mi"
              onClick={() => {
                close();
                void edit(op);
              }}
            >
              <span className="mi-check" />
              <span className="mi-label">{EDIT_LABELS[op]}</span>
            </button>
          ))}
          {EFFECT_GROUPS.map((group) => (
            <div key={group} style={{ display: "contents" }}>
              <div className="msep" role="separator" />
              <div className="mi-menu mi-group">{group}</div>
              {EFFECTS.filter((e) => e.group === group).map((e) => (
                <button
                  key={e.type}
                  type="button"
                  role="menuitem"
                  className="mi"
                  onClick={() => {
                    close();
                    openEffect(e.type);
                  }}
                >
                  <span className="mi-check" />
                  <span className="mi-label">{e.name}…</span>
                </button>
              ))}
            </div>
          ))}
        </Popover>
      )}
    </>
  );
}

/** The whole clip in miniature, with the visible part outlined; drag to scroll. */
function Overview({ peaks, length, view, color }: { peaks: Float32Array[] | null; length: number; view: { start: number; end: number }; color: string }) {
  const [ref, { width }] = useSize<HTMLDivElement>();
  const setView = useStore((s) => s.setEditorView);
  const drag = useRef(false);
  const span = view.end - view.start;
  const scrollTo = (e: PointerEvent<HTMLDivElement>) => {
    const x = e.clientX - e.currentTarget.getBoundingClientRect().left;
    const centre = (x / Math.max(1, width)) * length;
    const start = Math.min(length - span, Math.max(0, centre - span / 2));
    setView({ start, end: start + span });
  };
  return (
    <div
      ref={ref}
      className="ed-overview"
      onPointerDown={(e) => {
        e.currentTarget.setPointerCapture(e.pointerId);
        drag.current = true;
        scrollTo(e);
      }}
      onPointerMove={(e) => drag.current && scrollTo(e)}
      onPointerUp={() => (drag.current = false)}
      title="Drag to scroll"
    >
      {width > 0 && peaks && <Waveform peaks={peaks[0]} peaksR={peaks[1]} width={width} height={22} color={color} />}
      <div className="ed-viewport" style={{ left: (view.start / length) * width, width: Math.max(4, (span / length) * width) }} aria-hidden="true" />
    </div>
  );
}

interface DragState {
  kind: "select" | "band" | "draw";
  /** Seconds from the clip start where the drag began. */
  at: number;
  lane: number;
  y: number;
}

interface DrawState {
  channel: number;
  samples: Float32Array;
  lo: number;
  hi: number;
  last: number | null;
}

export function EditorView() {
  const target = useStore(useShallow(editorTarget));
  const selection = useStore((s) => s.editorSelection);
  const tempo = useStore((s) => tempoOf(s.project));
  const tracks = useStore((s) => s.project.tracks);
  const busy = useStore((s) => s.busy);
  const meta = useStore((s) => (target ? s.project.assets.find((a) => a.id === target.clip.asset) : undefined));
  const cursor = useStore((s) => s.editorCursor);
  const zeroCross = useStore((s) => s.zeroCross);
  const hasClipboard = useStore((s) => s.hasClipboard);
  const view = useStore((s) => s.editorView);
  const display = useStore((s) => s.editorDisplay);
  const scale = useStore((s) => s.spectrumScale);
  const tool = useStore((s) => s.tool);
  const st = useStore.getState();
  const [areaRef, { width, height }, areaEl] = useSize<HTMLDivElement>();
  const drag = useRef<DragState | null>(null);
  const drawing = useRef<DrawState | null>(null);
  const [, redraw] = useState(0);
  const missing = meta?.missing ?? false;
  const data = usePeaks(target && !missing ? target.clip.asset : undefined);
  const whole = useMemo(() => (data && target ? clipPeaks(data, target.clip.offset, target.clip.duration) : null), [data, target]);

  const length = target?.clip.duration ?? 1;
  const vs = Math.max(0, Math.min(view?.start ?? 0, length));
  const ve = Math.max(vs + 1e-6, Math.min(view?.end ?? length, length));
  const span = ve - vs;
  const zoomed = span < length - 1e-6;
  const chH = Math.max(80, Math.floor(height / 2) - 1);
  const rate = meta?.sampleRate ?? data?.sampleRate ?? 48_000;
  const nyquist = rate / 2;
  const asset = target?.clip.asset ?? "";
  const a0 = (target?.clip.offset ?? 0) + vs;
  const a1 = (target?.clip.offset ?? 0) + ve;
  const ready = !!target && !missing && width > 0;

  const detail = useLatest(ready && display === "waveform" && zoomed ? `${asset}|${a0.toFixed(6)}|${a1.toFixed(6)}|${width}` : null, () =>
    backend.waveWindow(asset, a0, a1, width),
  );
  const columns = Math.min(2048, width);
  const spectro = useLatest(
    ready && display === "spectrogram" ? `${asset}|${a0.toFixed(6)}|${a1.toFixed(6)}|${columns}|${chH}|${scale}` : null,
    () => backend.spectrogram(asset, a0, a1, columns, chH, scale),
    60,
  );
  // A drawn stroke stays on screen until the edited audio comes back.
  useEffect(() => {
    drawing.current = null;
  }, [detail]);

  // Ctrl + wheel zooms around the pointer; the wheel alone scrolls. Native
  // listener: React's wheel handlers are passive and cannot stop page zoom.
  useEffect(() => {
    if (!areaEl) return;
    const onWheel = (e: WheelEvent) => {
      const s = useStore.getState();
      const t = editorTarget(s);
      if (!t) return;
      e.preventDefault();
      const rect = areaEl.getBoundingClientRect();
      const v = s.editorView ?? { start: 0, end: t.clip.duration };
      const sp = v.end - v.start;
      if (e.ctrlKey || e.metaKey) {
        s.zoomEditor(e.deltaY < 0 ? 1.25 : 0.8, v.start + ((e.clientX - rect.left) / rect.width) * sp);
      } else if (sp < t.clip.duration) {
        const shift = ((e.deltaX || e.deltaY) / rect.width) * sp;
        const start = Math.min(t.clip.duration - sp, Math.max(0, v.start + shift));
        s.setEditorView({ start, end: start + sp });
      }
    };
    areaEl.addEventListener("wheel", onWheel, { passive: false });
    return () => areaEl.removeEventListener("wheel", onWheel);
  }, [areaEl]);

  if (!target) {
    return (
      <div className="editor">
        <div className="empty">
          <Icon name="wave" size={22} />
          <p>
            <b>No audio clip yet</b>
            Import or drop an audio file, then double-click its clip to edit it here. Or generate a tone, noise or silence.
            <span>
              <button className="btn sm btn-primary" type="button" onClick={() => void st.importAudio()}>
                Import audio…
              </button>
              <button className="btn sm" type="button" onClick={() => st.setGeneratorOpen(true)}>
                Generate…
              </button>
            </span>
          </p>
        </div>
      </div>
    );
  }

  const { track, clip } = target;
  const color = trackColor(track.color);
  const sel = selection && selection.end - selection.start > 0.001 ? selection : null;
  const band = display === "spectrogram" && sel?.lowHz !== undefined && sel.highHz !== undefined ? sel : null;
  const secAt = (clientX: number) => {
    const x = clientX - (areaEl?.getBoundingClientRect().left ?? 0);
    return Math.min(length, Math.max(0, vs + (x / Math.max(1, width)) * span));
  };
  const xOf = (sec: number) => ((sec - vs) / span) * width;
  const laneOf = (y: number) => (y < chH + 1 ? 0 : 1);
  const hzOfY = (yIn: number) => hzAt(1 - yIn / chH, nyquist, scale);
  const channels = meta?.channels ?? data?.channels.length ?? 2;
  const canDraw = display === "waveform" && !!detail?.raw && span * rate <= width;
  const disabled = !!busy || missing;

  const drawAt = (clientX: number, clientY: number) => {
    const d = drawing.current;
    const area = areaEl?.getBoundingClientRect();
    if (!d || !detail || !area) return;
    const lane = drag.current?.lane ?? 0;
    const yIn = clientY - area.top - lane * (chH + 1);
    const value = Math.max(-1, Math.min(1, (chH / 2 - yIn) / (chH / 2 - 2)));
    const idx = Math.round((clip.offset + secAt(clientX)) * rate) - detail.first;
    if (idx < 0 || idx >= d.samples.length) return;
    const from = d.last ?? idx;
    const steps = Math.abs(idx - from);
    const startValue = d.last === null ? value : d.samples[from];
    for (let k = 0; k <= steps; k++) {
      const i = from + Math.sign(idx - from) * k;
      d.samples[i] = steps ? startValue + ((value - startValue) * k) / steps : value;
    }
    d.lo = Math.min(d.lo, from, idx);
    d.hi = Math.max(d.hi, from, idx);
    d.last = idx;
    redraw((n) => n + 1);
  };

  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    const area = e.currentTarget.getBoundingClientRect();
    const y = e.clientY - area.top;
    const lane = laneOf(y);
    const at = secAt(e.clientX);
    if (tool === "draw") {
      if (!canDraw || !detail) {
        st.toast({
          tone: "info",
          title: display === "waveform" ? "Zoom in to draw" : "Drawing works on the waveform",
          detail: "Zoom in (Ctrl + mouse wheel) until single samples show, then drag across them.",
        });
        return;
      }
      const channel = Math.min(lane, detail.channels.length - 1);
      e.currentTarget.setPointerCapture(e.pointerId);
      drag.current = { kind: "draw", at, lane, y };
      drawing.current = { channel, samples: Float32Array.from(detail.channels[channel]), lo: Infinity, hi: -Infinity, last: null };
      drawAt(e.clientX, e.clientY);
      return;
    }
    e.currentTarget.setPointerCapture(e.pointerId);
    drag.current = { kind: display === "spectrogram" ? "band" : "select", at, lane, y };
    st.setEditorSelection(null);
  };

  const onPointerMove = (e: PointerEvent<HTMLDivElement>) => {
    const d = drag.current;
    if (!d) return;
    if (d.kind === "draw") return drawAt(e.clientX, e.clientY);
    const t = secAt(e.clientX);
    const range = { start: Math.min(d.at, t), end: Math.max(d.at, t) };
    if (d.kind === "band") {
      const area = e.currentTarget.getBoundingClientRect();
      const top = d.lane * (chH + 1);
      const yIn = Math.min(chH, Math.max(0, e.clientY - area.top - top));
      const y0 = d.y - top;
      // A drag that is mostly horizontal selects time only.
      if (Math.abs(yIn - y0) >= 4) {
        st.setEditorSelection({ ...range, lowHz: hzOfY(Math.max(yIn, y0)), highHz: hzOfY(Math.min(yIn, y0)) });
        return;
      }
    }
    st.setEditorSelection(range);
  };

  const onPointerUp = (e: PointerEvent<HTMLDivElement>) => {
    const d = drag.current;
    drag.current = null;
    if (!d) return;
    if (d.kind === "draw") {
      const s = drawing.current;
      if (s && detail && s.hi >= s.lo) {
        const samples = Array.from(s.samples.subarray(s.lo, s.hi + 1));
        void st.drawSamples(channels > 1 ? s.channel : null, detail.first + s.lo, samples);
      }
      return;
    }
    const t = secAt(e.clientX);
    if (Math.abs(t - d.at) * (width / span) < 3) {
      st.setEditorSelection(null);
      st.setEditorCursor(t);
      st.seekBeats(beatAfter(clip, tempo, clip.start, t));
    }
  };

  const { ticks, label } = rulerTicks(vs, ve, width);
  const audioClips = tracks.flatMap((t) => t.clips.filter((c): c is AudioClip => c.kind === "audio").map((c) => ({ t, c })));
  const fallback = zoomed && data ? clipPeaks(data, clip.offset + vs, span) : whole;
  const override = (lane: number) => {
    const d = drawing.current;
    return d && Math.min(lane, (detail?.channels.length ?? 1) - 1) === d.channel ? d.samples : null;
  };
  const spectral = (mode: SpectralMode, text: string, tip: string) => (
    <button className="btn sm tip-t" type="button" disabled={disabled} data-tip={tip} onClick={() => void st.spectralEdit(mode)}>
      {text}
    </button>
  );

  return (
    <div className="editor" data-tool={tool}>
      <div className="ed-bar">
        <span className="selwrap">
          <select
            className="select"
            aria-label="Clip to edit"
            value={clip.id}
            onChange={(e) => {
              const hit = audioClips.find(({ c }) => c.id === e.target.value);
              if (hit) st.selectClip(hit.t.id, hit.c.id);
            }}
          >
            {audioClips.map(({ t, c }) => (
              <option key={c.id} value={c.id}>
                {t.name} — {c.name}
              </option>
            ))}
          </select>
          <Icon name="chevronDown" size={14} />
        </span>
        <div className="seg" role="group" aria-label="Display">
          <button type="button" aria-pressed={display === "waveform"} onClick={() => st.setEditorDisplay("waveform")}>
            Waveform
          </button>
          <button type="button" aria-pressed={display === "spectrogram"} onClick={() => st.setEditorDisplay("spectrogram")}>
            Spectrogram
          </button>
        </div>
        {display === "spectrogram" && (
          <span className="selwrap">
            <select className="select sm" aria-label="Frequency scale" value={scale} onChange={(e) => st.setSpectrumScale(e.target.value as FrequencyScale)}>
              {SCALES.map(([v, l]) => (
                <option key={v} value={v}>
                  {l}
                </option>
              ))}
            </select>
            <Icon name="chevronDown" size={13} />
          </span>
        )}
        <span className="vsep" />
        <div className="ed-ops" role="group" aria-label="Edit">
          <button className="btn sm tip-b" type="button" disabled={disabled || !sel} data-tip="Cut  ·  Ctrl X" onClick={() => void st.editorClipboard("cut")}>
            Cut
          </button>
          <button className="btn sm tip-b" type="button" disabled={disabled || !sel} data-tip="Copy  ·  Ctrl C" onClick={() => void st.editorClipboard("copy")}>
            Copy
          </button>
          <button
            className="btn sm tip-b"
            type="button"
            disabled={disabled || !hasClipboard}
            data-tip="Paste at the cursor, or over the selection  ·  Ctrl V"
            onClick={() => void st.editorClipboard("paste")}
          >
            Paste
          </button>
          <button className="btn sm tip-b" type="button" disabled={disabled || !sel} data-tip="Delete  ·  Del" onClick={() => void st.editorClipboard("delete")}>
            Delete
          </button>
          <button className="btn sm tip-b" type="button" disabled={disabled || !sel} data-tip="Keep only the selection" onClick={st.trimToSelection}>
            Trim
          </button>
        </div>
        <span className="vsep" />
        <div className="ed-ops" role="group" aria-label="Process">
          <button className="btn sm btn-fix tip-b" type="button" disabled={disabled} data-tip="Listens, then fixes noise, clicks, silence and loudness" onClick={() => st.openEffect("fixAudio")}>
            <Icon name="wand" size={14} />
            Fix my audio…
          </button>
          <EffectsMenu disabled={disabled} />
          <button className="btn sm tip-b" type="button" disabled={disabled} data-tip="Learn the noise, then remove it" onClick={() => st.openEffect("noiseReduction")}>
            Noise reduction…
          </button>
          <button className="btn sm tip-b" type="button" disabled={disabled} data-tip="Run a saved chain of effects" onClick={() => st.setMacroDialog("clip")}>
            Macros…
          </button>
          <button className="btn sm tip-b" type="button" disabled={disabled} data-tip="Tone, sweep, noise, DTMF or silence" onClick={() => st.setGeneratorOpen(true)}>
            Generate…
          </button>
        </div>
        <button className="chipb tip-b" type="button" aria-pressed={zeroCross} data-tip="Snap selection edges to zero crossings, so cuts never click" onClick={st.toggleZeroCross}>
          Zero crossings
        </button>
        <div className="ed-zoom" role="group" aria-label="Zoom">
          <button className="ib tip-b" type="button" aria-label="Zoom out" data-tip="Zoom out  ·  Ctrl −" onClick={() => st.zoomEditor(0.5)}>
            <Icon name="minus" size={15} />
          </button>
          <button className="ib tip-b" type="button" aria-label="Zoom in" data-tip="Zoom in  ·  Ctrl + or Ctrl-scroll" onClick={() => st.zoomEditor(2)}>
            <Icon name="plus" size={15} />
          </button>
          <button className="btn sm tip-b" type="button" disabled={!sel} data-tip="Zoom to the selection" onClick={st.zoomEditorToSelection}>
            Selection
          </button>
          <button className="btn sm tip-b" type="button" disabled={!zoomed} data-tip="Show the whole clip  ·  Ctrl 0" onClick={() => st.setEditorView(null)}>
            Fit
          </button>
        </div>
      </div>

      <div className="ed-main card-inset">
        {zoomed && <Overview peaks={whole} length={length} view={{ start: vs, end: ve }} color={color} />}
        <div className="ed-ruler" style={{ width }}>
          {ticks.map((t) => (
            <span key={t.toFixed(6)} style={{ left: xOf(t) }}>
              {label(t)}
            </span>
          ))}
        </div>
        <div ref={areaRef} className={`ed-area${tool === "draw" && canDraw ? " drawing" : ""}`} onPointerDown={onPointerDown} onPointerMove={onPointerMove} onPointerUp={onPointerUp}>
          {missing && <p className="ed-missing">The audio file for this clip is missing.</p>}
          {(["L", "R"] as const).map((name, i) => (
            <div key={name} className="ed-ch" style={{ height: chH }}>
              {width > 0 && display === "spectrogram" && spectro && <SpectrogramCanvas data={spectro} channel={i} width={width} height={chH} />}
              {width > 0 && display === "waveform" && zoomed && detail && (
                <DetailWave data={detail} channel={i} width={width} height={chH} color={color} viewStart={a0} viewEnd={a1} override={override(i)} />
              )}
              {width > 0 && display === "waveform" && !(zoomed && detail) && fallback && (
                <Waveform peaks={fallback[Math.min(i, fallback.length - 1)]} width={width} height={chH} color={color} guides />
              )}
              <span className="ed-chl">{channels === 1 ? (i === 0 ? "Mono" : "") : name}</span>
              {display === "spectrogram" && (
                <div className="ed-freqs" aria-hidden="true">
                  {frequencyTicks(nyquist, scale).map((f) => (
                    <span key={f} style={{ top: (1 - posOf(f, nyquist, scale)) * chH }}>
                      {formatHz(f)}
                    </span>
                  ))}
                </div>
              )}
              {band && (
                <div
                  className="ed-band"
                  style={{
                    left: xOf(band.start),
                    width: xOf(band.end) - xOf(band.start),
                    top: (1 - posOf(band.highHz ?? 0, nyquist, scale)) * chH,
                    height: (posOf(band.highHz ?? 0, nyquist, scale) - posOf(band.lowHz ?? 0, nyquist, scale)) * chH,
                  }}
                  aria-hidden="true"
                />
              )}
            </div>
          ))}
          {sel && !band ? (
            <div className="ed-sel" style={{ left: xOf(sel.start), width: xOf(sel.end) - xOf(sel.start) }} aria-hidden="true" />
          ) : (
            !sel &&
            cursor >= vs &&
            cursor <= ve && <div className="ed-cursor" style={{ transform: `translateX(${xOf(cursor)}px)` }} aria-hidden="true" />
          )}
          <EditorPlayhead clip={clip} width={width} viewStart={vs} span={span} />
        </div>
      </div>

      <div className="ed-foot">
        <dl className="ed-readout">
          <div>
            <dt>Start</dt>
            <dd>{formatClock(sel?.start ?? 0)}</dd>
          </div>
          <div>
            <dt>End</dt>
            <dd>{formatClock(sel?.end ?? length)}</dd>
          </div>
          <div>
            <dt>Length</dt>
            <dd>{formatClock((sel?.end ?? length) - (sel?.start ?? 0))}</dd>
          </div>
          {band && (
            <div>
              <dt>Band</dt>
              <dd>
                {formatHz(band.lowHz ?? 0)}–{formatHz(band.highHz ?? 0)} Hz
              </dd>
            </div>
          )}
        </dl>
        {band ? (
          <div className="ed-ops" role="group" aria-label="Spectral edit">
            {spectral("delete", "Delete band", "Remove these frequencies in this time range")}
            {spectral("attenuate", "Lower 12 dB", "Turn these frequencies down")}
            {spectral("isolate", "Keep only band", "Remove everything else in this time range")}
          </div>
        ) : (
          <span className="muted">
            {tool === "draw"
              ? canDraw
                ? "Drag across the samples to redraw them. Ctrl Z undoes."
                : "Draw tool: zoom in (Ctrl + mouse wheel) until single samples show."
              : display === "spectrogram"
                ? "Drag a box around a sound to select its frequencies, or drag sideways to select time."
                : sel
                  ? "Effects apply to the selection."
                  : "Drag across the waveform to select. With no selection, effects apply to the whole clip."}{" "}
            Edits change real samples and can be undone with Ctrl Z.
          </span>
        )}
      </div>
    </div>
  );
}
