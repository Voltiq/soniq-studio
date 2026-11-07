import { useRef, useState } from "react";
import { MENUS, type MenuId } from "../commands";
import { useStore } from "../state/store";
import { Icon } from "./Icon";
import { HistoryPopover } from "./HistoryPopover";
import { CommandMenu } from "./Menu";

export function TopBar() {
  const [open, setOpen] = useState<{ menu: MenuId; anchor: HTMLElement } | null>(null);
  const mode = useStore((s) => s.mode);
  const theme = useStore((s) => s.theme);
  const canUndo = useStore((s) => s.history.canUndo);
  const canRedo = useStore((s) => s.history.canRedo);
  const undoLabel = useStore((s) => s.history.undoLabel);
  const redoLabel = useStore((s) => s.history.redoLabel);
  const projectName = useStore((s) => s.project.name);
  const dirty = useStore((s) => s.dirty);
  const path = useStore((s) => s.path);
  const historyOpen = useStore((s) => s.historyOpen);
  const historyButton = useRef<HTMLButtonElement>(null);
  const { setMode, setTheme, undo, redo, setPaletteOpen, setSettingsOpen, setExportOpen } = useStore.getState();

  return (
    <header className="topbar">
      <div className="logo">
        <span className="mark" aria-hidden="true">
          <svg viewBox="0 0 24 24" width="16" height="16">
            <path d="M3 12h2.5l2-5 3 10 3-8 2 5 1.5-2H21" />
          </svg>
        </span>
        <span>Soniq</span>
      </div>

      <nav className="menus" aria-label="Main menu">
        {MENUS.map((menu) => (
          <button
            key={menu}
            className={open?.menu === menu ? "menubtn on" : "menubtn"}
            type="button"
            aria-haspopup="menu"
            aria-expanded={open?.menu === menu}
            onClick={(e) => setOpen(open?.menu === menu ? null : { menu, anchor: e.currentTarget })}
            onPointerEnter={(e) => open && open.menu !== menu && setOpen({ menu, anchor: e.currentTarget })}
          >
            {menu}
          </button>
        ))}
      </nav>
      {open && <CommandMenu menu={open.menu} anchor={open.anchor} onClose={() => setOpen(null)} />}

      <button className="field cmd" type="button" onClick={() => setPaletteOpen(true)}>
        <Icon name="search" size={15} className="dim" />
        <span className="cmd-text">Search commands, devices and views</span>
        <span className="kbd">Ctrl K</span>
      </button>

      <div className="top-right">
        <span className="proj" title={path ?? "Not saved yet · Ctrl S to save"}>
          {dirty && <span className="unsaved" aria-label="Unsaved changes" />}
          {projectName}
        </span>
        <div className="seg" role="group" aria-label="Interface mode">
          <button type="button" aria-pressed={mode === "essentials"} onClick={() => setMode("essentials")} data-tip="Fewer controls, helpful tips" className="tip-b">
            Essentials
          </button>
          <button type="button" aria-pressed={mode === "pro"} onClick={() => setMode("pro")} data-tip="Every control  ·  Ctrl Shift P" className="tip-b">
            Pro
          </button>
        </div>
        <button
          className="ib tip-b"
          type="button"
          aria-label={theme === "dark" ? "Switch to light theme" : "Switch to dark theme"}
          data-tip={theme === "dark" ? "Light theme" : "Dark theme"}
          onClick={() => setTheme(theme === "dark" ? "light" : "dark")}
        >
          <Icon name={theme === "dark" ? "sun" : "moon"} />
        </button>
        <button className="ib tip-b" type="button" aria-label="Audio settings" data-tip="Audio & MIDI settings" onClick={() => setSettingsOpen(true)}>
          <Icon name="gear" />
        </button>
        <span className="vsep" />
        <button
          className="ib tip-b"
          type="button"
          aria-label={undoLabel ? `Undo ${undoLabel}` : "Undo"}
          data-tip={undoLabel ? `Undo ${undoLabel}  ·  Ctrl Z` : "Nothing to undo"}
          disabled={!canUndo}
          onClick={() => void undo()}
        >
          <Icon name="undo" />
        </button>
        <button
          className="ib tip-b"
          type="button"
          aria-label={redoLabel ? `Redo ${redoLabel}` : "Redo"}
          data-tip={redoLabel ? `Redo ${redoLabel}  ·  Ctrl Shift Z` : "Nothing to redo"}
          disabled={!canRedo}
          onClick={() => void redo()}
        >
          <Icon name="redo" />
        </button>
        <button
          ref={historyButton}
          className="ib tip-b history-btn"
          type="button"
          aria-label="History"
          aria-expanded={historyOpen}
          data-tip="Every step, to go back to  ·  Ctrl H"
          onClick={() => useStore.getState().setHistoryOpen(!historyOpen)}
        >
          <Icon name="clock" />
        </button>
        {historyOpen && historyButton.current && <HistoryPopover anchor={historyButton.current} />}
        <span className="vsep" />
        <button className="btn btn-primary tip-b" type="button" data-tip="Export mixdown  ·  Ctrl Shift E" onClick={() => setExportOpen(true)}>
          <Icon name="export" size={15} />
          Export
        </button>
      </div>
    </header>
  );
}
