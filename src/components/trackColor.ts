// Track colours live in tokens.css (--track-1…8), or the theme editor's set; canvases need the resolved value.
const cache = new Map<number, string>();
const FALLBACK = ["#6b8dff", "#f59a72", "#c9658a", "#e9b85c", "#2bb5a0", "#8b7cf6", "#5bb8e8", "#a3b86c"];

export function trackColor(n: number): string {
  const hit = cache.get(n);
  if (hit) return hit;
  const value = getComputedStyle(document.documentElement).getPropertyValue(`--track-${n}`).trim() || FALLBACK[(n - 1) % 8];
  cache.set(n, value);
  return value;
}

/** After the colour set changes: read the tokens again. */
export function resetTrackColors() {
  cache.clear();
}

export const trackVar = (n: number) => `var(--track-${n})`;
