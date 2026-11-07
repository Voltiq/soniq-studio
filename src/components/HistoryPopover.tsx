import { useEffect, useState } from "react";
import { backend } from "../engine/backend";
import { useStore } from "../state/store";
import type { HistoryList } from "../state/types";
import { Icon } from "./Icon";
import { Popover } from "./Menu";

/** Every step that can be undone or redone; clicking one goes back (or forward) to just after it. */
export function HistoryPopover({ anchor }: { anchor: HTMLElement }) {
  const project = useStore((s) => s.project);
  const { setHistoryOpen, jumpHistory } = useStore.getState();
  const [list, setList] = useState<HistoryList | null>(null);

  // Fetched again whenever the song changes, so it never shows stale steps.
  useEffect(() => {
    let live = true;
    void backend.historyList().then((l) => live && setList(l));
    return () => {
      live = false;
    };
  }, [project]);

  const past = list?.past ?? [];
  const future = list?.future ?? [];
  // Newest at the top: what redo reaches (last first), now, then the past.
  const go = (steps: number) => void jumpHistory(steps);
  return (
    <Popover anchor={anchor} onClose={() => setHistoryOpen(false)} label="History" align="right">
      <div className="hist">
        <div className="hist-head">
          <b>History</b>
          <span className="muted">
            {past.length} {past.length === 1 ? "step" : "steps"} back{future.length ? ` · ${future.length} ahead` : ""}
          </span>
        </div>
        <ol className="hist-list" aria-label="Steps, newest first">
          {[...future].reverse().map((label, i) => {
            const ahead = future.length - i;
            return (
              <li key={`f${ahead}`}>
                <button type="button" className="hist-row ahead" onClick={() => go(ahead)} title="Redo up to here">
                  <Icon name="redo" size={13} className="dim" />
                  <span>{label}</span>
                </button>
              </li>
            );
          })}
          {past
            .map((label, i) => ({ label, back: past.length - 1 - i }))
            .reverse()
            .map(({ label, back }) => (
              <li key={`p${back}`}>
                <button type="button" className={`hist-row${back === 0 ? " now" : ""}`} onClick={() => back && go(-back)} title={back ? "Go back to just after this" : "Where you are"} aria-current={back === 0 ? "step" : undefined}>
                  <Icon name={back === 0 ? "check" : "undo"} size={13} className="dim" />
                  <span>{label}</span>
                  {back === 0 && <span className="tag">Now</span>}
                </button>
              </li>
            ))}
          <li>
            <button type="button" className={`hist-row origin${past.length === 0 ? " now" : ""}`} onClick={() => past.length && go(-past.length)} title="Undo everything">
              <Icon name="folder" size={13} className="dim" />
              <span>Opened or started</span>
              {past.length === 0 && <span className="tag">Now</span>}
            </button>
          </li>
        </ol>
      </div>
    </Popover>
  );
}
