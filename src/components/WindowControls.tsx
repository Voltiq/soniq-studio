import { useEffect, useState, type MouseEvent as ReactMouseEvent } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { inTauri } from "../engine/bridge";

/**
 * The window has no system frame on Windows: the top bar is its title bar.
 * Elsewhere (macOS, Linux, the browser preview) the system draws the frame.
 */
export const customFrame = inTauri && navigator.userAgent.includes("Windows");

/** Where a press on the top bar is a press on the bar itself (not a control): it moves the window. */
const INTERACTIVE = "button, a, input, select, textarea, [role='menuitem'], [contenteditable='true'], .field";

/** Pressing the bar's empty space moves the window; double-clicking it maximises or restores, as a system title bar does. */
export function dragWindow(e: ReactMouseEvent) {
  const target = e.target as HTMLElement;
  // Menus opened from the bar sit elsewhere on the page; only the bar itself moves the window.
  if (!customFrame || e.button !== 0 || !e.currentTarget.contains(target) || target.closest(INTERACTIVE)) return;
  const win = getCurrentWindow();
  if (e.detail === 2) void win.toggleMaximize();
  else void win.startDragging();
}

/**
 * Minimise, maximise (restore when maximised) and close, drawn as Windows 11
 * draws them: 46 px wide, flush with the window's top-right corner, the
 * system's own Segoe Fluent glyphs, a neutral wash on hover and the red close.
 * Dimmed while the window is in the background, as the system's are.
 */
export function WindowControls() {
  const [maximized, setMaximized] = useState(false);
  const [active, setActive] = useState(true);

  useEffect(() => {
    if (!customFrame) return;
    const win = getCurrentWindow();
    const sync = () => void win.isMaximized().then(setMaximized).catch(() => {});
    sync();
    const resized = win.onResized(sync);
    const focus = win.onFocusChanged(({ payload }) => setActive(payload));
    return () => {
      void resized.then((off) => off());
      void focus.then((off) => off());
    };
  }, []);

  if (!customFrame) return null;
  const win = getCurrentWindow();
  return (
    <div className={active ? "caption" : "caption inactive"} role="group" aria-label="Window">
      <button type="button" className="caption-btn" aria-label="Minimise" title="Minimise" onClick={() => void win.minimize()}>
        <span aria-hidden="true">{""}</span>
      </button>
      <button
        type="button"
        className="caption-btn"
        aria-label={maximized ? "Restore" : "Maximise"}
        title={maximized ? "Restore Down" : "Maximise"}
        onClick={() => void win.toggleMaximize()}
      >
        <span aria-hidden="true">{maximized ? "" : ""}</span>
      </button>
      {/* Closing asks about unsaved work first (the close-requested handler). */}
      <button type="button" className="caption-btn close" aria-label="Close" title="Close" onClick={() => void win.close()}>
        <span aria-hidden="true">{""}</span>
      </button>
    </div>
  );
}
