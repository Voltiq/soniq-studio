import { useEffect, useState } from "react";
import { useShallow } from "zustand/react/shallow";
import { parseKey, scaleClasses } from "../state/music";
import { NOTE_TOOLS, applyTool, noteToolSpec, type NoteTool, type NoteToolType } from "../state/noteTools";
import { pianoTarget, useStore } from "../state/store";
import { Icon } from "./Icon";
import { Popover } from "./Menu";
import { ParamForm } from "./ParamForm";

/** Settings stay as they were left, per tool, for this session. */
const remembered = new Map<NoteToolType, NoteTool>();

function NoteToolPanel({ type, onClose }: { type: NoteToolType; onClose: () => void }) {
  const spec = noteToolSpec(type);
  const [tool, setTool] = useState<NoteTool>(() => remembered.get(type) ?? structuredClone(spec.defaults));
  const target = useStore(useShallow(pianoTarget));
  const selected = useStore((s) => s.pianoSelection.length);
  const keyName = useStore((s) => s.project.key);
  const bpb = useStore((s) => s.project.beatsPerBar);
  const editNotes = useStore((s) => s.editNotes);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const change = (next: NoteTool) => {
    remembered.set(type, next);
    setTool(next);
  };
  const apply = () => {
    if (!target) return;
    const key = parseKey(keyName);
    const inKey = key.scale === "chromatic" ? null : scaleClasses(key);
    editNotes((notes, sel) => applyTool(tool, notes, sel, { length: target.clip.length, inKey }), spec.name);
    onClose();
  };
  const count = target?.clip.notes.length ?? 0;
  const where =
    spec.group === "Generate"
      ? `Writes across the whole clip (${+(target ? target.clip.length / bpb : 0).toFixed(2)} bars) in ${keyName}.`
      : selected
        ? `Works on the ${selected} selected ${selected === 1 ? "note" : "notes"}.`
        : `Works on all ${count} notes (select some to change only those).`;

  return (
    <div className="scrim" onPointerDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="dialog shine fx-dialog" role="dialog" aria-modal="true" aria-labelledby="tool-title">
        <header className="dialog-head">
          <h2 id="tool-title">{spec.name}</h2>
          <button className="ib" type="button" aria-label="Close" onClick={onClose} autoFocus>
            <Icon name="close" size={16} />
          </button>
        </header>
        <p className="muted fx-desc">{spec.description}</p>
        {spec.params.length > 0 && <ParamForm specs={spec.params} target={tool} onChange={change} />}
        <div className="status-line">
          <Icon name="info" size={15} />
          <span>{where}</span>
        </div>
        <footer className="dialog-foot">
          <span className="muted">One undo step.</span>
          <div className="tgroup">
            <button className="btn btn-ghost" type="button" onClick={onClose}>
              Cancel
            </button>
            <button className="btn btn-primary" type="button" disabled={!target || (spec.group === "Transform" && !count)} onClick={apply}>
              {spec.group === "Generate" ? "Generate" : "Apply"}
            </button>
          </div>
        </footer>
      </div>
    </div>
  );
}

export function NoteToolDialog() {
  const type = useStore((s) => s.noteTool);
  const close = useStore((s) => s.setNoteTool);
  return type ? <NoteToolPanel key={type} type={type} onClose={() => close(null)} /> : null;
}

/** The piano roll's Tools menu: transforms for the selection, generators for the clip. */
export function NoteToolsMenu({ disabled }: { disabled?: boolean }) {
  const [anchor, setAnchor] = useState<HTMLElement | null>(null);
  const open = useStore((s) => s.setNoteTool);
  return (
    <>
      <button
        className="btn sm tip-b"
        type="button"
        aria-haspopup="menu"
        disabled={disabled}
        data-tip="Arpeggiate, chop, strum, chords, humanize… or generate a rhythm or melody"
        onClick={(e) => setAnchor(anchor ? null : e.currentTarget)}
      >
        Tools
        <Icon name="chevronDown" size={13} />
      </button>
      {anchor && (
        <Popover anchor={anchor} onClose={() => setAnchor(null)} label="Note tools">
          {(["Transform", "Generate"] as const).map((group, g) => (
            <div key={group} style={{ display: "contents" }}>
              {g > 0 && <div className="msep" role="separator" />}
              <span className="mi-group">{group}</span>
              {NOTE_TOOLS.filter((t) => t.group === group).map((t) => (
                <button
                  key={t.type}
                  type="button"
                  role="menuitem"
                  className="mi"
                  title={t.description}
                  onClick={() => {
                    setAnchor(null);
                    open(t.type);
                  }}
                >
                  <span className="mi-check" />
                  <span className="mi-label">
                    {t.name}
                    {t.params.length ? "…" : ""}
                  </span>
                </button>
              ))}
            </div>
          ))}
        </Popover>
      )}
    </>
  );
}
