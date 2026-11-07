/*
 * Keyboard shortcuts: a set to start from (Soniq's own, or Audacity's or
 * Live's keys for the same commands) and the user's own changes on top.
 * Remembered in this computer's app storage.
 */

export type KeymapPreset = "soniq" | "audacity" | "live";

export interface Keymap {
  preset: KeymapPreset;
  /** The user's own keys by command id; null takes a command's key away. */
  custom: Record<string, string | null>;
}

export interface PresetInfo {
  id: KeymapPreset;
  name: string;
  description: string;
  /** Keys that differ from Soniq's, by command id; null: no key. */
  keys: Record<string, string | null>;
}

export const KEYMAP_PRESETS: PresetInfo[] = [
  { id: "soniq", name: "Soniq", description: "Soniq's own keys.", keys: {} },
  {
    id: "audacity",
    name: "Audacity",
    description: "Audacity's keys for the commands Soniq shares with it: Ctrl+I splits, Ctrl+L silences, Ctrl+T trims, Ctrl+B adds a region, Ctrl+1 and Ctrl+3 zoom.",
    keys: {
      "clip.split": "Ctrl+I",
      "file.import": "Ctrl+Shift+I",
      "clip.silence": "Ctrl+L",
      "edit.trim": "Ctrl+T",
      "track.audio": "Ctrl+Shift+N",
      "edit.region": "Ctrl+B",
      "edit.redo": "Ctrl+Y",
      "file.stems": "Ctrl+Shift+L",
      "view.theme": null,
      "view.zoomIn": "Ctrl+1",
      "view.zoomOut": "Ctrl+3",
      "view.zoomFit": "Ctrl+F",
      "file.settings": "Ctrl+P",
    },
  },
  {
    id: "live",
    name: "Ableton Live",
    description: "Live's keys for the commands Soniq shares with it: F9 records, Ctrl+L loops, M plays notes from the computer keyboard, B draws, Ctrl+Shift+M adds a MIDI clip.",
    keys: {
      "transport.record": "F9",
      "transport.loop": "Ctrl+L",
      "transport.keys": "M",
      "transport.metronome": null,
      "tool.draw": "B",
      "clip.newMidi": "Ctrl+Shift+M",
      "edit.region": null,
      "file.export": "Ctrl+Shift+R",
      "view.zoomIn": "=",
      "view.zoomOut": "-",
    },
  },
];

const STORAGE_KEY = "soniq.keymap";

export function loadKeymap(): Keymap {
  try {
    const saved = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? "null") as Partial<Keymap> | null;
    const preset = KEYMAP_PRESETS.some((p) => p.id === saved?.preset) ? (saved?.preset as KeymapPreset) : "soniq";
    return { preset, custom: saved?.custom && typeof saved.custom === "object" ? saved.custom : {} };
  } catch {
    return { preset: "soniq", custom: {} };
  }
}

export function saveKeymap(keymap: Keymap) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(keymap));
  } catch {
    // Kept for this session only.
  }
}

/** A command's key in the preset alone, before the user's own changes. */
export function presetKey(id: string, fallback: string | undefined, preset: KeymapPreset): string | undefined {
  const keys = KEYMAP_PRESETS.find((p) => p.id === preset)?.keys ?? {};
  return id in keys ? (keys[id] ?? undefined) : fallback;
}

/** A command's key now: the user's, else the preset's, else Soniq's. */
export function keyFor(id: string, fallback: string | undefined, keymap: Keymap): string | undefined {
  return id in keymap.custom ? (keymap.custom[id] ?? undefined) : presetKey(id, fallback, keymap.preset);
}

const MODIFIERS = new Set(["Control", "Shift", "Alt", "Meta", "AltGraph", "CapsLock", "OS"]);

/** The shortcut a key press makes ("Ctrl+Shift+K"), or null for a modifier on its own. */
export function eventShortcut(e: KeyboardEvent): string | null {
  if (MODIFIERS.has(e.key)) return null;
  const parts: string[] = [];
  if (e.ctrlKey || e.metaKey) parts.push("Ctrl");
  if (e.altKey) parts.push("Alt");
  if (e.shiftKey) parts.push("Shift");
  parts.push(e.key === " " ? "Space" : e.key === "+" ? "Plus" : e.key.length === 1 ? e.key.toUpperCase() : e.key);
  return parts.join("+");
}
