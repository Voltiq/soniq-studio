import { useEffect, useMemo, useState } from "react";
import { backend } from "../engine/backend";
import { formatClock } from "../state/format";
import { editorTarget, useStore } from "../state/store";
import type { Diagnosis, Effect, FixAnalysis, FixKind, FixStep } from "../state/types";
import { Icon } from "./Icon";
import { Select } from "./Select";

/** Where the loudness ends up, in LUFS. */
const TARGETS: [number, string][] = [
  [-16, "−16 LUFS · voice, podcasts, Apple"],
  [-14, "−14 LUFS · music for Spotify, YouTube"],
  [-19, "−19 LUFS · a mono podcast"],
  [-23, "−23 LUFS · broadcast (EBU R128)"],
];

const num = (v: number, digits = 0) => v.toFixed(digits).replace("-", "−");
const seconds = (v: number) => `${num(v, 1)} s`;

function loudnessEffects(target: number): Effect[] {
  return [
    { type: "loudness", target },
    { type: "limiter", ceiling: -1, release: 50 },
  ];
}

/** The step's effects, with the loudness step following the chosen target. */
function effectsOf(step: FixStep, target: number): Effect[] {
  return step.kind === "loudness" ? loudnessEffects(target) : step.effects;
}

function loudnessNeeded(d: Diagnosis, target: number): boolean {
  return d.loudness !== null && (Math.abs(d.loudness - target) > 0.5 || d.peakDb > -0.95);
}

/** What each repair is called, and what was found. */
function describe(kind: FixKind, d: Diagnosis, step: FixStep, target: number): { title: string; found: string } {
  switch (kind) {
    case "dcOffset":
      return {
        title: "Centre the waveform",
        found: step.suggested ? `It sits ${num(d.dcOffset * 100, 1)} % off the zero line, which wastes headroom and makes cuts click.` : "It sits on the zero line.",
      };
    case "rumble":
      return {
        title: "Cut rumble below 70 Hz",
        found: step.suggested
          ? `Strong sound below 60 Hz (${num(d.rumbleDb)} dB of the whole): rumble, wind or handling noise. If this is music with deep bass, leave it.`
          : "Little sound below 60 Hz: nothing to cut.",
      };
    case "clicks":
      return { title: "Remove clicks and pops", found: d.clicks === 0 ? "No clicks found." : `Found ${d.clicks} ${d.clicks === 1 ? "click" : "clicks"}.` };
    case "noise": {
      const effect = step.effects[0];
      const by = effect?.type === "denoise" ? effect.reduction : 12;
      const under = d.signalDb - d.noiseDb;
      const found = step.suggested
        ? `A background of ${num(d.noiseDb)} dB, ${num(under)} dB under the sound: lowered ${by} dB, learned from the quiet moments.`
        : d.seconds < 1
          ? "Too short to learn the noise from."
          : under < 15
            ? "The sound hardly pauses, so there is no quiet moment to learn the noise from. Noise reduction with a profile can still help."
            : `The background sits ${num(under)} dB under the sound: quiet enough to leave.`;
      return { title: "Reduce background noise", found };
    }
    case "ends":
      return {
        title: "Trim silence at the ends",
        found: step.suggested ? `${seconds(d.lead)} before the first sound, ${seconds(d.tail)} after the last: each cut to a quarter second.` : "No long silence at the ends.",
      };
    case "pauses":
      return {
        title: "Shorten long pauses",
        found:
          d.pauses === 0
            ? "No pauses longer than 1.5 s."
            : `${d.pauses} ${d.pauses === 1 ? "pause" : "pauses"} over 1.5 s (${seconds(d.pauseSeconds)} in all), each cut to 0.8 s. This changes the timing, so it is yours to tick.`,
      };
    case "loudness": {
      const lufs = d.loudness ?? target;
      const change = target - lufs;
      return {
        title: `Set the loudness to ${num(target)} LUFS`,
        found: loudnessNeeded(d, target)
          ? `Now ${num(lufs, 1)} LUFS with peaks at ${num(d.peakDb, 1)} dBFS: ${change >= 0 ? "raised" : "lowered"} ${num(Math.abs(change), 1)} dB, peaks held under −1 dBFS.`
          : "Already there.",
      };
    }
  }
}

export function FixPanel({ onClose }: { onClose: () => void }) {
  // Strings, not the target object: a fresh object every time would re-render forever.
  const clipId = useStore((s) => editorTarget(s)?.clip.id ?? null);
  const clipName = useStore((s) => editorTarget(s)?.clip.name ?? "the clip");
  const selection = useStore((s) => s.editorSelection);
  const busy = useStore((s) => s.busy);
  const previewing = useStore((s) => s.previewing);
  const { applyEffects, previewEffects, stopPreview } = useStore.getState();
  const sel = selection && selection.end - selection.start > 0.001 ? selection : null;
  const [lufs, setLufs] = useState(-16);
  const [analysis, setAnalysis] = useState<FixAnalysis | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [chosen, setChosen] = useState<Set<FixKind>>(new Set());

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  // Listen once per clip and selection; the target only changes the last step.
  useEffect(() => {
    if (!clipId) return;
    let live = true;
    setAnalysis(null);
    setError(null);
    backend
      .fixAnalyse(clipId, sel?.start, sel?.end, -16)
      .then((a) => {
        if (!live) return;
        setAnalysis(a);
        setChosen(new Set(a.steps.filter((s) => s.suggested).map((s) => s.kind)));
      })
      .catch((err) => live && setError(err instanceof Error ? err.message : String(err)));
    return () => {
      live = false;
    };
  }, [clipId, sel?.start, sel?.end]);

  // The loudness step is suggested again whenever the target moves.
  const pickTarget = (value: number) => {
    setLufs(value);
    if (!analysis) return;
    setChosen((now) => {
      const next = new Set(now);
      if (loudnessNeeded(analysis.diagnosis, value)) next.add("loudness");
      else next.delete("loudness");
      return next;
    });
  };

  const chain = useMemo(() => (analysis ? analysis.steps.filter((s) => chosen.has(s.kind)).flatMap((s) => effectsOf(s, lufs)) : []), [analysis, chosen, lufs]);
  const toggle = (kind: FixKind) =>
    setChosen((now) => {
      const next = new Set(now);
      if (next.has(kind)) next.delete(kind);
      else next.add(kind);
      return next;
    });

  const apply = async () => {
    if (await applyEffects(chain, "Fix my audio")) onClose();
  };

  return (
    <div className="scrim" onPointerDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="dialog shine fx-dialog fix-dialog" role="dialog" aria-modal="true" aria-labelledby="fix-title">
        <header className="dialog-head">
          <h2 id="fix-title">
            <Icon name="wand" size={17} />
            Fix my audio
          </h2>
          <button className="ib" type="button" aria-label="Close" onClick={onClose} autoFocus>
            <Icon name="close" size={16} />
          </button>
        </header>
        <p className="muted fx-desc">Listens to the recording and fixes what it finds. Untick anything you want to keep as it is.</p>

        {error ? (
          <div className="status-line warn">
            <Icon name="warning" size={15} />
            <span>{error}</span>
          </div>
        ) : !analysis ? (
          <div className="fix-listening" role="status">
            <span className="spinner" aria-hidden="true" />
            Listening to {sel ? "the selection" : `“${clipName}”`}…
          </div>
        ) : (
          <ul className="fix-steps" aria-label="Repairs">
            {analysis.steps.map((step) => {
              const needed = step.kind === "loudness" ? loudnessNeeded(analysis.diagnosis, lufs) : step.suggested;
              const { title, found } = describe(step.kind, analysis.diagnosis, { ...step, suggested: needed }, lufs);
              const on = chosen.has(step.kind);
              // Pauses are found but left for you to choose.
              const optional = !needed && step.kind === "pauses" && analysis.diagnosis.pauses > 0;
              return (
                <li key={step.kind} className={`fix-step${on ? " on" : ""}${needed ? " needed" : ""}`}>
                  <label>
                    <input type="checkbox" checked={on} onChange={() => toggle(step.kind)} />
                    <span className="fix-text">
                      <b>{title}</b>
                      <span className="muted">{found}</span>
                    </span>
                    <span className={`fix-tag${needed ? " needed" : ""}`}>{needed ? "Needed" : optional ? "Optional" : "Fine"}</span>
                  </label>
                </li>
              );
            })}
          </ul>
        )}

        <label className="setting wide fix-target">
          <span className="lab">Loudness</span>
          <span className="selwrap">
            <Select className="select" value={lufs} onChange={(e) => pickTarget(Number(e.target.value))}>
              {TARGETS.map(([v, label]) => (
                <option key={v} value={v}>
                  {label}
                </option>
              ))}
            </Select>
          </span>
        </label>

        <div className="status-line">
          <Icon name="info" size={15} />
          <span>
            {sel ? `Fixes the selection (${formatClock(sel.end - sel.start)}).` : `Fixes all of “${clipName}”.`} One step to undo. Preview plays up to 10
            seconds{analysis && analysis.diagnosis.seconds > 10 ? ", where noise and loudness are judged on those seconds alone" : ""}.
          </span>
        </div>

        <footer className="dialog-foot">
          <button
            className={previewing ? "btn on" : "btn"}
            type="button"
            disabled={!chain.length || (!!busy && !previewing)}
            onClick={() => (previewing ? stopPreview() : void previewEffects(chain))}
          >
            <Icon name={previewing ? "stop" : "play"} size={13} />
            {previewing ? "Stop" : "Preview"}
          </button>
          <div className="tgroup">
            <button className="btn btn-ghost" type="button" onClick={onClose}>
              Cancel
            </button>
            <button className="btn btn-primary" type="button" disabled={!chain.length || !!busy} onClick={() => void apply()}>
              <Icon name="wand" size={14} />
              {chain.length ? "Fix" : "Nothing to fix"}
            </button>
          </div>
        </footer>
      </div>
    </div>
  );
}
