import { useEffect, useState } from "react";
import { GENERATORS, generatorSpec } from "../state/effects";
import { formatClock } from "../state/format";
import { editorTarget, useStore } from "../state/store";
import type { Generator, GeneratorType } from "../state/types";
import { Icon } from "./Icon";
import { ParamForm } from "./ParamForm";

const remembered = new Map<GeneratorType, Generator>();

function Generate({ onClose }: { onClose: () => void }) {
  const [type, setType] = useState<GeneratorType>("tone");
  const [gen, setGen] = useState<Generator>(() => remembered.get("tone") ?? structuredClone(generatorSpec("tone").defaults));
  const view = useStore((s) => s.view);
  const clip = useStore((s) => editorTarget(s)?.clip ?? null);
  const selection = useStore((s) => s.editorSelection);
  const cursor = useStore((s) => s.editorCursor);
  const track = useStore((s) => s.project.tracks.find((t) => t.id === s.selectedTrackId && t.kind === "audio")?.name ?? null);
  const busy = useStore((s) => s.busy);
  const generate = useStore((s) => s.generate);
  // In the editor, generated audio goes into the clip; elsewhere it becomes a new clip.
  const [into, setInto] = useState<"clip" | "track">(view === "editor" && clip ? "clip" : "track");
  const spec = generatorSpec(type);
  const sel = selection && selection.end - selection.start > 0.001 ? selection : null;

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const pick = (next: GeneratorType) => {
    setType(next);
    setGen(remembered.get(next) ?? structuredClone(generatorSpec(next).defaults));
  };
  const change = (next: Generator) => {
    remembered.set(type, next);
    setGen(next);
  };

  return (
    <div className="scrim" onPointerDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="dialog shine fx-dialog" role="dialog" aria-modal="true" aria-labelledby="gen-title">
        <header className="dialog-head">
          <h2 id="gen-title">Generate</h2>
          <button className="ib" type="button" aria-label="Close" onClick={onClose} autoFocus>
            <Icon name="close" size={16} />
          </button>
        </header>
        <div className="seg" role="group" aria-label="Generator">
          {GENERATORS.map((g) => (
            <button key={g.type} type="button" aria-pressed={type === g.type} onClick={() => pick(g.type)}>
              {g.name}
            </button>
          ))}
        </div>
        <p className="muted fx-desc">{spec.description}</p>
        <ParamForm specs={spec.params} target={gen} onChange={change} />
        <div className="setting">
          <span className="lab">Where</span>
          <div className="seg" role="group" aria-label="Where">
            <button type="button" aria-pressed={into === "clip"} disabled={!clip} onClick={() => setInto("clip")}>
              {sel ? "Replace the selection" : "Insert at the editor cursor"}
            </button>
            <button type="button" aria-pressed={into === "track"} onClick={() => setInto("track")}>
              New clip at the playhead
            </button>
          </div>
        </div>
        <div className="status-line">
          <Icon name="info" size={15} />
          <span>
            {into === "clip" && clip
              ? sel
                ? `Replaces ${formatClock(sel.end - sel.start)} of “${clip.name}”; the clip changes length to fit.`
                : `Goes into “${clip.name}” at ${formatClock(cursor)}; the clip gets longer.`
              : track
                ? `A new clip on ${track}, at the playhead.`
                : "A new audio track with the clip, at the playhead."}
          </span>
        </div>
        <footer className="dialog-foot">
          <span className="muted">Undo removes it again.</span>
          <div className="tgroup">
            <button className="btn btn-ghost" type="button" onClick={onClose}>
              Cancel
            </button>
            <button className="btn btn-primary" type="button" disabled={!!busy} onClick={() => void generate(gen, into).then((ok) => ok && onClose())}>
              Generate
            </button>
          </div>
        </footer>
      </div>
    </div>
  );
}

export function GenerateDialog() {
  const open = useStore((s) => s.generatorOpen);
  const setOpen = useStore((s) => s.setGeneratorOpen);
  return open ? <Generate onClose={() => setOpen(false)} /> : null;
}
