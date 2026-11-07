import { useEffect, useState } from "react";
import { defaultEffect, effectSpec } from "../state/effects";
import { formatClock } from "../state/format";
import { editorTarget, useStore } from "../state/store";
import type { Effect, EffectType } from "../state/types";
import { CurveEditor } from "./CurveEditor";
import { FixPanel } from "./FixDialog";
import { Icon } from "./Icon";
import { ParamForm } from "./ParamForm";
import { Select } from "./Select";

/** The last settings used for each effect, for this session. */
const remembered = new Map<EffectType, Effect>();

function EffectPanel({ type, onClose }: { type: EffectType; onClose: () => void }) {
  const spec = effectSpec(type);
  const [effect, setEffect] = useState<Effect>(() => remembered.get(type) ?? defaultEffect(type));
  const selection = useStore((s) => s.editorSelection);
  const clipName = useStore((s) => editorTarget(s)?.clip.name ?? "");
  const busy = useStore((s) => s.busy);
  const previewing = useStore((s) => s.previewing);
  const profile = useStore((s) => s.noiseProfileSeconds);
  const { applyEffects, previewEffects, stopPreview, captureNoiseProfile } = useStore.getState();
  const sel = selection && selection.end - selection.start > 0.001 ? selection : null;
  const needsProfile = (type === "noiseReduction" && profile === null) || (effect.type === "autoDuck" && !effect.track);
  // Auto Duck: the tracks it can listen to (not the clip's own), as a stable list.
  const ownTrack = useStore((s) => editorTarget(s)?.track.id ?? "");
  const tracksKey = useStore((s) => s.project.tracks.map((t) => `${t.id}\u0001${t.name}`).join("\u0002"));
  const others = tracksKey
    .split("\u0002")
    .filter(Boolean)
    .map((row) => {
      const [id, name] = row.split("\u0001");
      return { id, name };
    })
    .filter((t) => t.id !== ownTrack);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const change = (next: Effect) => {
    remembered.set(type, next);
    setEffect(next);
  };

  const apply = async () => {
    if (await applyEffects([effect])) onClose();
  };

  return (
    <div className="scrim" onPointerDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="dialog shine fx-dialog" role="dialog" aria-modal="true" aria-labelledby="fx-title">
        <header className="dialog-head">
          <h2 id="fx-title">{spec.name}</h2>
          <button className="ib" type="button" aria-label="Close" onClick={onClose} autoFocus>
            <Icon name="close" size={16} />
          </button>
        </header>
        <p className="muted fx-desc">{spec.description}</p>

        {type === "noiseReduction" && (
          <div className="fx-step">
            <span className="fx-num">1</span>
            <div>
              <b>Learn the noise</b>
              <p className="muted">
                {profile !== null
                  ? `Profile learned from ${formatClock(profile)} of noise. Learn again any time.`
                  : sel
                    ? `Press Get profile to learn from the selection (${formatClock(sel.end - sel.start)}). It should hold only noise: no voice or music.`
                    : "Close this, select a stretch with only noise (no voice or music), and press Get profile."}
              </p>
            </div>
            <button className="btn sm" type="button" disabled={!sel || !!busy} onClick={() => void captureNoiseProfile()}>
              Get profile
            </button>
          </div>
        )}

        {type === "noiseReduction" ? (
          <div className="fx-step">
            <span className="fx-num">2</span>
            <div className="fx-grow">
              <b>Reduce it</b>
              <ParamForm specs={spec.params} target={effect} onChange={change} />
            </div>
          </div>
        ) : effect.type === "curveEq" ? (
          <CurveEditor points={effect.points} onChange={(points) => change({ ...effect, points })} />
        ) : effect.type === "autoDuck" ? (
          <>
            <label className="setting wide">
              <span className="lab">Listen to</span>
              <span className="selwrap">
                <Select className="select" value={effect.track} onChange={(e) => change({ ...effect, track: e.target.value })}>
                  {!effect.track && <option value="">Pick the track to duck under…</option>}
                  {others.map((t) => (
                    <option key={t.id} value={t.id}>
                      {t.name}
                    </option>
                  ))}
                </Select>
              </span>
            </label>
            <ParamForm specs={spec.params} target={effect} onChange={change} />
          </>
        ) : (
          <ParamForm specs={spec.params} target={effect} onChange={change} />
        )}

        <div className="status-line">
          <Icon name="info" size={15} />
          <span>
            {sel ? `Applies to the selection (${formatClock(sel.end - sel.start)}).` : `Applies to all of “${clipName}”.`}
            {spec.changesLength && " The clip gets shorter or longer to match."} Preview plays up to 10 seconds.
          </span>
        </div>

        <footer className="dialog-foot">
          <button
            className={previewing ? "btn on" : "btn"}
            type="button"
            disabled={needsProfile || (!!busy && !previewing)}
            onClick={() => (previewing ? stopPreview() : void previewEffects([effect]))}
          >
            <Icon name={previewing ? "stop" : "play"} size={13} />
            {previewing ? "Stop" : "Preview"}
          </button>
          <div className="tgroup">
            <button className="btn btn-ghost" type="button" onClick={onClose}>
              Cancel
            </button>
            <button className="btn btn-primary" type="button" disabled={needsProfile || !!busy} onClick={() => void apply()}>
              Apply
            </button>
          </div>
        </footer>
      </div>
    </div>
  );
}

export function EffectDialog() {
  const type = useStore((s) => s.effectDialog);
  const openEffect = useStore((s) => s.openEffect);
  if (!type) return null;
  return type === "fixAudio" ? <FixPanel onClose={() => openEffect(null)} /> : <EffectPanel key={type} type={type} onClose={() => openEffect(null)} />;
}
