import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { useEffect, useState } from "react";
import { inTauri } from "../engine/bridge";
import { EFFECTS, defaultEffect, describeEffect, effectSpec } from "../state/effects";
import { formatClock } from "../state/format";
import { AUDIO_EXTENSIONS, editorTarget, useStore } from "../state/store";
import type { BatchProgress, BatchReport, Effect, EffectType, Encoding, Macro } from "../state/types";
import { EncodingFields, defaultEncoding } from "./ExportDialog";
import { Icon } from "./Icon";
import { ParamForm } from "./ParamForm";
import { Select } from "./Select";

const newId = () => `macro-${Date.now().toString(36)}`;
const fileName = (path: string) => path.split(/[\\/]/).pop() ?? path;

/** The steps of one macro; built-in ones are read-only. */
function Steps({ macro, onChange }: { macro: Macro; onChange: (steps: Effect[]) => void }) {
  const [open, setOpen] = useState<number | null>(null);
  const readOnly = !!macro.builtIn;
  const move = (i: number, by: number) => {
    const next = macro.steps.slice();
    const [step] = next.splice(i, 1);
    next.splice(i + by, 0, step);
    onChange(next);
    setOpen(null);
  };
  return (
    <ol className="steps">
      {macro.steps.map((step, i) => (
        <li key={i} className={open === i ? "step on" : "step"}>
          <div className="step-row">
            <span className="step-n">{i + 1}</span>
            <span className="step-name">{describeEffect(step)}</span>
            {!readOnly && (
              <>
                <button className="ib xs" type="button" aria-label="Settings" aria-expanded={open === i} onClick={() => setOpen(open === i ? null : i)}>
                  <Icon name="gear" size={14} />
                </button>
                <button className="ib xs" type="button" aria-label="Move up" disabled={i === 0} onClick={() => move(i, -1)}>
                  <Icon name="chevronDown" size={14} className="flip" />
                </button>
                <button className="ib xs" type="button" aria-label="Move down" disabled={i === macro.steps.length - 1} onClick={() => move(i, 1)}>
                  <Icon name="chevronDown" size={14} />
                </button>
                <button className="ib xs" type="button" aria-label="Remove step" onClick={() => onChange(macro.steps.filter((_, k) => k !== i))}>
                  <Icon name="close" size={14} />
                </button>
              </>
            )}
          </div>
          {open === i && !readOnly && (
            <ParamForm specs={effectSpec(step.type).params} target={step} compact onChange={(next) => onChange(macro.steps.map((s, k) => (k === i ? next : s)))} />
          )}
        </li>
      ))}
      {!macro.steps.length && <li className="muted step-empty">No steps yet: add one below.</li>}
      {!readOnly && (
        <li className="step-add">
          <span className="selwrap">
            <Select className="select sm" aria-label="Add a step" value="" onChange={(e) => e.target.value && onChange([...macro.steps, defaultEffect(e.target.value as EffectType)])}>
              <option value="">Add a step…</option>
              {/* Auto Duck listens to a track and the curve is drawn: neither fits a macro step. */}
              {EFFECTS.filter((e) => e.type !== "autoDuck" && e.type !== "curveEq").map((e) => (
                <option key={e.type} value={e.type}>
                  {e.name}
                </option>
              ))}
            </Select>
          </span>
        </li>
      )}
    </ol>
  );
}

function Macros({ mode, onClose }: { mode: "clip" | "batch"; onClose: () => void }) {
  const macros = useStore((s) => s.macros);
  const busy = useStore((s) => s.busy);
  const hasClip = useStore((s) => !!editorTarget(s));
  const selection = useStore((s) => s.editorSelection);
  const { saveMacros, applyEffects, runBatch, setMacroDialog, toast } = useStore.getState();
  const [selectedId, setSelectedId] = useState(macros[0]?.id ?? "");
  const [files, setFiles] = useState<string[]>([]);
  const [folder, setFolder] = useState<string | null>(null);
  const [suffix, setSuffix] = useState("");
  const [encoding, setEncoding] = useState<Encoding>(defaultEncoding);
  const [progress, setProgress] = useState<BatchProgress | null>(null);
  const [report, setReport] = useState<BatchReport | null>(null);
  const macro = macros.find((m) => m.id === selectedId) ?? macros[0];
  const user = macros.filter((m) => !m.builtIn);
  const sel = selection && selection.end - selection.start > 0.001 ? selection : null;

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && !busy && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose, busy]);

  const update = (next: Macro) => void saveMacros(user.map((m) => (m.id === next.id ? next : m)));
  const add = (base?: Macro) => {
    const made: Macro = { id: newId(), name: base ? `${base.name} (copy)` : "New macro", steps: base ? structuredClone(base.steps) : [] };
    void saveMacros([...user, made]);
    setSelectedId(made.id);
  };
  const remove = (id: string) => {
    void saveMacros(user.filter((m) => m.id !== id));
    setSelectedId(macros[0]?.id ?? "");
  };

  const pickFiles = async () => {
    if (!inTauri) return toast({ tone: "info", title: "Batch processing needs the desktop app" });
    const picked = await openDialog({ multiple: true, filters: [{ name: "Audio", extensions: AUDIO_EXTENSIONS }] });
    if (picked) setFiles([...new Set([...files, ...(Array.isArray(picked) ? picked : [picked])])]);
  };
  const pickFolder = async () => {
    if (!inTauri) return;
    const picked = await openDialog({ directory: true, title: "Write the processed files into…" });
    if (typeof picked === "string") setFolder(picked);
  };
  const run = async () => {
    if (!macro || !folder) return;
    setReport(null);
    const result = await runBatch(files, macro.steps, { ...encoding, folder, suffix }, setProgress);
    setProgress(null);
    if (result) setReport(result);
  };

  return (
    <div className="scrim" onPointerDown={(e) => e.target === e.currentTarget && !busy && onClose()}>
      <div className="dialog shine macro-dialog" role="dialog" aria-modal="true" aria-labelledby="macro-title">
        <header className="dialog-head">
          <h2 id="macro-title">Macros</h2>
          <div className="seg" role="tablist" aria-label="Use">
            <button type="button" role="tab" aria-selected={mode === "clip"} aria-pressed={mode === "clip"} onClick={() => setMacroDialog("clip")}>
              Apply to a clip
            </button>
            <button type="button" role="tab" aria-selected={mode === "batch"} aria-pressed={mode === "batch"} onClick={() => setMacroDialog("batch")}>
              Batch process files
            </button>
          </div>
          <button className="ib" type="button" aria-label="Close" onClick={onClose} disabled={!!busy}>
            <Icon name="close" size={16} />
          </button>
        </header>

        <div className="macro-body">
          <div className="macro-list" role="listbox" aria-label="Macros">
            {macros.map((m) => (
              <button key={m.id} type="button" role="option" aria-selected={m.id === macro?.id} className={m.id === macro?.id ? "mi on" : "mi"} onClick={() => setSelectedId(m.id)}>
                <span className="mi-label">{m.name}</span>
                <span className="mi-k">{m.builtIn ? "built-in" : `${m.steps.length} steps`}</span>
              </button>
            ))}
            <div className="macro-actions">
              <button className="btn sm" type="button" onClick={() => add()}>
                <Icon name="plus" size={13} />
                New
              </button>
              <button className="btn sm" type="button" disabled={!macro} onClick={() => macro && add(macro)}>
                Duplicate
              </button>
              <button className="btn sm" type="button" disabled={!macro || macro.builtIn} onClick={() => macro && remove(macro.id)}>
                Delete
              </button>
            </div>
          </div>
          {macro && (
            <div className="macro-detail">
              {macro.builtIn ? (
                <p className="muted">Built-in macros can’t be changed. Duplicate one to make it your own.</p>
              ) : (
                <label className="field">
                  <input aria-label="Macro name" value={macro.name} onChange={(e) => update({ ...macro, name: e.target.value })} />
                </label>
              )}
              <Steps macro={macro} onChange={(steps) => update({ ...macro, steps })} />
            </div>
          )}
        </div>

        {mode === "batch" && (
          <div className="batch">
            <div className="batch-files">
              <button className="btn sm" type="button" onClick={() => void pickFiles()}>
                <Icon name="plus" size={13} />
                Add files…
              </button>
              <span className="muted">{files.length ? `${files.length} ${files.length === 1 ? "file" : "files"}: ${files.slice(0, 3).map(fileName).join(", ")}${files.length > 3 ? "…" : ""}` : "No files yet."}</span>
              {files.length > 0 && (
                <button className="btn sm btn-ghost" type="button" onClick={() => setFiles([])}>
                  Clear
                </button>
              )}
            </div>
            <div className="settings-grid">
              <EncodingFields encoding={encoding} onChange={setEncoding} />
              <div className="setting">
                <span className="lab">Save into</span>
                <button className="btn sm folder-pick" type="button" onClick={() => void pickFolder()} title={folder ?? undefined}>
                  <Icon name="folder" size={14} />
                  {folder ? fileName(folder) : "Choose a folder…"}
                </button>
              </div>
              <label className="setting">
                <span className="lab">Add to names</span>
                <span className="field">
                  <input value={suffix} placeholder="e.g. -clean" onChange={(e) => setSuffix(e.target.value)} />
                </span>
              </label>
            </div>
            {progress && (
              <div className="batch-progress" role="progressbar" aria-valuemin={0} aria-valuemax={progress.total} aria-valuenow={progress.done}>
                <span style={{ width: `${(progress.done / Math.max(1, progress.total)) * 100}%` }} />
                <em>
                  {progress.done} of {progress.total} · {progress.file}
                </em>
              </div>
            )}
            {report && (
              <div className={report.failed.length ? "status-line warn" : "status-line"}>
                <Icon name={report.failed.length ? "warning" : "check"} size={15} />
                <span>
                  Wrote {report.written.length} {report.written.length === 1 ? "file" : "files"}
                  {report.failed.length > 0 && `; ${report.failed.length} failed: ${report.failed.map((f) => `${f.file} (${f.error})`).join("; ")}`}
                </span>
                {report.written.length > 0 && (
                  <button className="btn sm" type="button" onClick={() => void revealItemInDir(report.written[0])}>
                    Show in folder
                  </button>
                )}
              </div>
            )}
          </div>
        )}

        <footer className="dialog-foot">
          <span className="muted">
            {mode === "clip"
              ? sel
                ? `Runs on the selection (${formatClock(sel.end - sel.start)}).`
                : "Runs on the whole clip open in the editor."
              : "Originals are never changed."}
          </span>
          <div className="tgroup">
            <button className="btn btn-ghost" type="button" disabled={!!busy} onClick={onClose}>
              Close
            </button>
            {mode === "clip" ? (
              <button
                className="btn btn-primary"
                type="button"
                disabled={!macro?.steps.length || !hasClip || !!busy}
                onClick={() => macro && void applyEffects(macro.steps, macro.name).then((ok) => ok && onClose())}
              >
                Apply “{macro?.name}”
              </button>
            ) : (
              <button className="btn btn-primary" type="button" disabled={!macro?.steps.length || !files.length || !folder || !!busy} onClick={() => void run()}>
                Process {files.length || ""} {files.length === 1 ? "file" : "files"}
              </button>
            )}
          </div>
        </footer>
      </div>
    </div>
  );
}

export function MacroDialog() {
  const mode = useStore((s) => s.macroDialog);
  const setMode = useStore((s) => s.setMacroDialog);
  return mode ? <Macros mode={mode} onClose={() => setMode(null)} /> : null;
}
