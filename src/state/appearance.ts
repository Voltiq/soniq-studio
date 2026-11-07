/*
 * The theme editor's choices: the accent, the set of track colours and how
 * dense the interface is. They override tokens on the root element, so
 * every component (which only ever reads tokens) follows.
 */
import { resetTrackColors } from "../components/trackColor";

export type Density = "comfortable" | "compact";

export interface Appearance {
  /** `#rrggbb`. */
  accent: string;
  colorSet: string;
  density: Density;
}

export const DEFAULT_ACCENT = "#6457f0";

export const ACCENTS: [string, string][] = [
  [DEFAULT_ACCENT, "Graphiq violet"],
  ["#3b7bf6", "Blue"],
  ["#0e9f92", "Teal"],
  ["#2f9e5b", "Green"],
  ["#d98a1c", "Amber"],
  ["#e5603f", "Coral"],
  ["#d9467a", "Rose"],
];

export interface ColorSet {
  id: string;
  name: string;
  /** Eight colours for --track-1…8. */
  colors: string[];
}

export const COLOR_SETS: ColorSet[] = [
  { id: "graphiq", name: "Graphiq", colors: ["#6b8dff", "#f59a72", "#c9658a", "#e9b85c", "#2bb5a0", "#8b7cf6", "#5bb8e8", "#a3b86c"] },
  { id: "vivid", name: "Vivid", colors: ["#4f7cff", "#ff7a45", "#e5487f", "#f5c02e", "#14c4a6", "#9b6bff", "#2fb6f0", "#8fcc3a"] },
  { id: "soft", name: "Soft", colors: ["#9fb4ff", "#f4b398", "#dc9bb4", "#ecce8c", "#82cfc2", "#b3a9f8", "#94cdee", "#c3d39b"] },
  { id: "earth", name: "Earth", colors: ["#7f9fb0", "#c98b5b", "#a8655e", "#cfa75e", "#5f9c8f", "#8a7fa8", "#6f97b3", "#9aa86e"] },
];

const HEX = /^#[0-9a-f]{6}$/i;

export function isHex(value: string): boolean {
  return HEX.test(value);
}

function luminance(hex: string): number {
  const channel = (i: number) => {
    const v = parseInt(hex.slice(i, i + 2), 16) / 255;
    return v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * channel(1) + 0.7152 * channel(3) + 0.0722 * channel(5);
}

/** White or graphite text on the accent, whichever reads better (WCAG contrast). */
export function onAccent(hex: string): string {
  const l = luminance(isHex(hex) ? hex : DEFAULT_ACCENT);
  const dark = luminance("#111318");
  return (1.05) / (l + 0.05) >= (l + 0.05) / (dark + 0.05) ? "#ffffff" : "#111318";
}

export function colorSet(id: string): ColorSet {
  return COLOR_SETS.find((c) => c.id === id) ?? COLOR_SETS[0];
}

/** Puts the choices on the root element; the Graphiq defaults come from tokens.css. */
export function applyAppearance({ accent, colorSet: set, density }: Appearance) {
  const root = document.documentElement;
  if (isHex(accent) && accent.toLowerCase() !== DEFAULT_ACCENT) {
    root.style.setProperty("--accent", accent);
    root.style.setProperty("--on-accent", onAccent(accent));
  } else {
    root.style.removeProperty("--accent");
    root.style.removeProperty("--on-accent");
  }
  const colors = colorSet(set);
  colors.colors.forEach((c, i) => {
    if (colors.id === "graphiq") root.style.removeProperty(`--track-${i + 1}`);
    else root.style.setProperty(`--track-${i + 1}`, c);
  });
  root.dataset.density = density;
  resetTrackColors();
}

/** Arrangement lane height for the density. */
export function trackHeight(density: Density): number {
  return density === "compact" ? 56 : 76;
}
