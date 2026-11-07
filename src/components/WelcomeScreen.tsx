import { useEffect, useRef, useState } from "react";
import { inTauri } from "../engine/bridge";
import { LESSONS } from "../state/lessons";
import { useStore } from "../state/store";
import type { TemplateInfo } from "../state/types";
import { Icon, type IconName } from "./Icon";

const TEMPLATE_ICONS: Record<string, IconName> = { empty: "plus", beat: "drum", band: "mic", podcast: "headphones", editing: "wave" };

/** "today", "yesterday", or the date. */
function when(seconds: number): string {
  const date = new Date(seconds * 1000);
  const days = Math.floor((Date.now() - date.getTime()) / 86_400_000);
  if (days < 1 && date.getDate() === new Date().getDate()) return `today, ${date.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}`;
  if (days < 2) return "yesterday";
  return date.toLocaleDateString([], { day: "numeric", month: "short", year: date.getFullYear() === new Date().getFullYear() ? undefined : "numeric" });
}

function TemplateCard({ template }: { template: TemplateInfo }) {
  const { newProject, deleteTemplate } = useStore.getState();
  return (
    <div className="wl-template">
      <button type="button" className="wl-card" onClick={() => void newProject(template.id)}>
        <span className={`wl-icon${template.builtin ? "" : " own"}`}>
          <Icon name={TEMPLATE_ICONS[template.id] ?? "folder"} size={18} />
        </span>
        <span className="wl-text">
          <b>{template.name}</b>
          <span className="muted">{template.description}</span>
        </span>
      </button>
      {!template.builtin && (
        <button className="ib xs wl-remove" type="button" aria-label={`Delete the template ${template.name}`} title="Delete this template" onClick={() => void deleteTemplate(template.id)}>
          <Icon name="trash" size={13} />
        </button>
      )}
    </div>
  );
}

/** Saves the open song's setup as a template of your own. */
function SaveTemplate() {
  const saveAsTemplate = useStore((s) => s.saveAsTemplate);
  const songName = useStore((s) => s.project.name);
  const focus = useStore((s) => s.welcomeFocus);
  const input = useRef<HTMLInputElement>(null);
  const [name, setName] = useState("");
  useEffect(() => {
    if (focus === "template") input.current?.focus();
  }, [focus]);
  if (!inTauri) return null;
  return (
    <form
      className="wl-save"
      onSubmit={(e) => {
        e.preventDefault();
        void saveAsTemplate(name || songName).then((ok) => ok && setName(""));
      }}
    >
      <input ref={input} className="wl-input" placeholder={`Save “${songName}” as a template…`} aria-label="Template name" value={name} onChange={(e) => setName(e.target.value)} />
      <button className="btn sm" type="submit">
        Save template
      </button>
    </form>
  );
}

/** The start page: a new song from a template, a recent one, the demo, or a lesson. */
export function WelcomeScreen() {
  const templates = useStore((s) => s.templates);
  const recent = useStore((s) => s.recent);
  const done = useStore((s) => s.lessonsDone);
  const atStart = useStore((s) => s.welcomeAtStart);
  const { setWelcomeOpen, openProject, openDemo, openSketch, forgetRecent, startLesson, setWelcomeAtStart, loadStart } = useStore.getState();
  useEffect(() => {
    void loadStart();
  }, [loadStart]);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && setWelcomeOpen(false);
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [setWelcomeOpen]);

  return (
    <div className="scrim welcome-scrim" onPointerDown={(e) => e.target === e.currentTarget && setWelcomeOpen(false)}>
      <section className="welcome shine" role="dialog" aria-modal="true" aria-labelledby="welcome-title">
        <header className="wl-head">
          <span className="mark" aria-hidden="true">
            <svg viewBox="0 0 24 24" width="20" height="20">
              <path d="M3 12h2.5l2-5 3 10 3-8 2 5 1.5-2H21" />
            </svg>
          </span>
          <div>
            <h2 id="welcome-title">Soniq Studio</h2>
            <p className="muted">Start a song, pick up where you left off, or learn your way around.</p>
          </div>
          <button className="ib" type="button" aria-label="Close the start page" onClick={() => setWelcomeOpen(false)} autoFocus>
            <Icon name="close" size={16} />
          </button>
        </header>

        <div className="wl-body">
          <div className="wl-col">
            <h3 className="wl-h">New song</h3>
            <div className="wl-templates">
              {templates.map((t) => (
                <TemplateCard key={t.id} template={t} />
              ))}
            </div>
            <SaveTemplate />
          </div>

          <div className="wl-col">
            <h3 className="wl-h">Open</h3>
            <div className="wl-actions">
              <button className="btn sm" type="button" onClick={() => void openProject()}>
                <Icon name="folder" size={14} />
                Open…
              </button>
              <button className="btn sm" type="button" onClick={() => void openDemo()}>
                <Icon name="play" size={12} />
                Demo song
              </button>
              <button className="btn sm" type="button" title="One big record button and a loop: record layers over it" onClick={() => void openSketch(true)}>
                <Icon name="mic" size={13} />
                Sketch an idea
              </button>
            </div>
            {recent.length === 0 ? (
              <p className="muted wl-empty">Songs you open or save show up here.</p>
            ) : (
              <ul className="wl-recent" aria-label="Recent songs">
                {recent.map((r) => (
                  <li key={r.path} className={r.exists ? undefined : "gone"}>
                    <button type="button" className="wl-row" disabled={!r.exists} onClick={() => void openProject(r.path)} title={r.path}>
                      <b>{r.name}</b>
                      <span className="muted">{r.exists ? when(r.opened) : "Not found where it was"}</span>
                    </button>
                    <button className="ib xs" type="button" aria-label={`Remove ${r.name} from the list`} title="Remove from the list" onClick={() => void forgetRecent(r.path)}>
                      <Icon name="close" size={13} />
                    </button>
                  </li>
                ))}
              </ul>
            )}

            <h3 className="wl-h">Lessons</h3>
            <ul className="wl-lessons">
              {LESSONS.map((l) => (
                <li key={l.id}>
                  <button type="button" className="wl-row" onClick={() => startLesson(l.id)}>
                    <span className="wl-lesson-title">
                      {done.includes(l.id) && <Icon name="check" size={13} className="wl-done" />}
                      <b>{l.title}</b>
                      <span className="tag">{l.minutes} min</span>
                    </span>
                    <span className="muted">{l.description}</span>
                  </button>
                </li>
              ))}
            </ul>
          </div>
        </div>

        <footer className="wl-foot">
          <label className="wl-check">
            <input type="checkbox" checked={atStart} onChange={(e) => setWelcomeAtStart(e.target.checked)} />
            Show this when Soniq starts
          </label>
          <span className="muted">File › Start page brings it back.</span>
        </footer>
      </section>
    </div>
  );
}
