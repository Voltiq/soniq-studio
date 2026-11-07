/**
 * Returns `next`, but reuses every part of `prev` that is deeply equal to it.
 * Each reply from the backend is a fresh copy of the whole project; this keeps
 * unchanged tracks and clips referentially stable, so React (and the waveform
 * canvases) only redraw what actually changed.
 */
export function reconcile<T>(prev: T, next: T): T {
  if (Object.is(prev, next)) return prev;

  if (Array.isArray(prev) && Array.isArray(next)) {
    let same = prev.length === next.length;
    const out = next.map((item, i) => {
      const r = reconcile(prev[i], item);
      if (r !== prev[i]) same = false;
      return r;
    });
    return (same ? prev : out) as T;
  }

  if (isPlainObject(prev) && isPlainObject(next)) {
    const keys = Object.keys(next);
    let same = keys.length === Object.keys(prev).length;
    const out: Record<string, unknown> = {};
    for (const key of keys) {
      const r = reconcile(prev[key], next[key]);
      out[key] = r;
      if (r !== prev[key]) same = false;
    }
    return (same ? prev : out) as T;
  }

  return next;
}

function isPlainObject(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && Object.getPrototypeOf(v) === Object.prototype;
}
