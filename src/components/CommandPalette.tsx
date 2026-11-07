import { useEffect, useMemo, useRef, useState } from "react";
import { COMMANDS, isEnabled, shortcutOf } from "../commands";
import { useStore } from "../state/store";
import { Icon } from "./Icon";

function Palette({ onClose }: { onClose: () => void }) {
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const list = useRef<HTMLDivElement>(null);

  const results = useMemo(() => {
    const words = query.toLowerCase().split(/\s+/).filter(Boolean);
    return COMMANDS.filter((c) => {
      const hay = `${c.label} ${c.menu}`.toLowerCase();
      return words.every((w) => hay.includes(w));
    }).sort((a, b) => Number(isEnabled(b)) - Number(isEnabled(a)));
  }, [query]);

  useEffect(() => setActive(0), [query]);
  useEffect(() => {
    list.current?.querySelector(`[data-index="${active}"]`)?.scrollIntoView({ block: "nearest" });
  }, [active]);

  const run = (index: number) => {
    const cmd = results[index];
    if (!cmd || !isEnabled(cmd)) return;
    onClose();
    cmd.run?.();
  };

  return (
    <div className="scrim" onPointerDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="palette glass" role="dialog" aria-modal="true" aria-label="Command palette">
        <label className="pal-input">
          <Icon name="search" size={17} className="dim" />
          <input
            autoFocus
            role="combobox"
            aria-expanded="true"
            aria-controls="palette-list"
            aria-activedescendant={results[active] ? `pal-${results[active].id}` : undefined}
            placeholder="Type a command, a device or a view…"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "ArrowDown") setActive((a) => Math.min(results.length - 1, a + 1));
              else if (e.key === "ArrowUp") setActive((a) => Math.max(0, a - 1));
              else if (e.key === "Enter") run(active);
              else if (e.key === "Escape") onClose();
              else return;
              e.preventDefault();
              e.stopPropagation();
            }}
          />
        </label>
        <div className="pal-list" id="palette-list" role="listbox" ref={list}>
          {results.length === 0 && <p className="pal-empty">No command matches “{query}”.</p>}
          {results.map((cmd, i) => {
            const enabled = isEnabled(cmd);
            return (
              <div
                key={cmd.id}
                id={`pal-${cmd.id}`}
                data-index={i}
                role="option"
                aria-selected={i === active}
                aria-disabled={!enabled || undefined}
                className={`mi${i === active ? " on" : ""}${enabled ? "" : " disabled"}`}
                onPointerMove={() => setActive(i)}
                onClick={() => run(i)}
              >
                <span className="mi-label">{cmd.label}</span>
                <span className="mi-menu">{cmd.menu}</span>
                {!cmd.run && cmd.phase ? <span className="tag">Phase {cmd.phase}</span> : shortcutOf(cmd) && <span className="kbd">{shortcutOf(cmd)}</span>}
              </div>
            );
          })}
        </div>
        <div className="pal-foot">
          <span>
            <span className="kbd">↑</span> <span className="kbd">↓</span> move
          </span>
          <span>
            <span className="kbd">Enter</span> run
          </span>
          <span>
            <span className="kbd">Esc</span> close
          </span>
          <span className="pal-note" title="Dimmed commands are planned; the tag names their roadmap phase.">
            Dimmed = planned, see its phase tag
          </span>
        </div>
      </div>
    </div>
  );
}

export function CommandPalette() {
  const open = useStore((s) => s.paletteOpen);
  const setOpen = useStore((s) => s.setPaletteOpen);
  return open ? <Palette onClose={() => setOpen(false)} /> : null;
}
