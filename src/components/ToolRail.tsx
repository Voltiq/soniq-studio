import { useStore } from "../state/store";
import type { ToolId } from "../state/types";
import { Icon, type IconName } from "./Icon";

const TOOLS: (readonly [ToolId, IconName, string, string, boolean?] | "sep")[] = [
  ["pointer", "pointer", "Pointer", "V"],
  ["range", "range", "Range select", "S"],
  "sep",
  ["draw", "draw", "Draw", "D"],
  ["split", "split", "Split", "C"],
  ["erase", "erase", "Erase", "E"],
  ["automation", "automation", "Automation", "A", true],
  "sep",
  ["zoom", "zoom", "Zoom", "Z"],
  ["hand", "hand", "Hand", "H"],
];

export function ToolRail() {
  const tool = useStore((s) => s.tool);
  const setTool = useStore((s) => s.setTool);
  const browserOpen = useStore((s) => s.browserOpen);
  const toggleBrowser = useStore((s) => s.toggleBrowser);

  return (
    <aside className="rail shine" aria-label="Tools">
      <button
        className={browserOpen ? "tool toggled tip-r" : "tool tip-r"}
        type="button"
        aria-label="Browser"
        aria-pressed={browserOpen}
        data-tip="Browser  ·  Ctrl Alt B"
        onClick={toggleBrowser}
      >
        <Icon name="sidebar" />
      </button>
      <span className="sep" />
      {TOOLS.map((t, i) =>
        t === "sep" ? (
          <span key={`sep-${i}`} className="sep" />
        ) : (
          <button
            key={t[0]}
            className={`tool tip-r${tool === t[0] ? " on" : ""}${t[4] ? " pro-only" : ""}`}
            type="button"
            aria-label={t[2]}
            aria-pressed={tool === t[0]}
            data-tip={`${t[2]}  ·  ${t[3]}`}
            onClick={() => setTool(t[0])}
          >
            <Icon name={t[1]} />
          </button>
        ),
      )}
    </aside>
  );
}
