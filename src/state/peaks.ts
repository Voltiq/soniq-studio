/*
 * Waveform overviews, fetched once per asset and cached. Asset ids never
 * change meaning (an edit makes a new asset), so entries never go stale.
 */
import { useEffect, useState } from "react";
import { backend } from "../engine/backend";
import type { PeakData } from "./types";

const cache = new Map<string, PeakData>();
const inflight = new Map<string, Promise<PeakData>>();

export function cachedPeaks(asset: string): PeakData | undefined {
  return cache.get(asset);
}

export function loadPeaks(asset: string): Promise<PeakData> {
  const hit = cache.get(asset);
  if (hit) return Promise.resolve(hit);
  let pending = inflight.get(asset);
  if (!pending) {
    pending = backend.peaks(asset).then((data) => {
      cache.set(asset, data);
      inflight.delete(asset);
      return data;
    });
    pending.catch(() => inflight.delete(asset));
    inflight.set(asset, pending);
  }
  return pending;
}

/** The asset's peaks, or undefined while they load (or if the audio is missing). */
export function usePeaks(asset: string | undefined): PeakData | undefined {
  const [data, setData] = useState(() => (asset ? cache.get(asset) : undefined));
  useEffect(() => {
    if (!asset) {
      setData(undefined);
      return;
    }
    const hit = cache.get(asset);
    if (hit) {
      setData(hit);
      return;
    }
    let live = true;
    loadPeaks(asset)
      .then((d) => live && setData(d))
      .catch(() => live && setData(undefined));
    return () => {
      live = false;
    };
  }, [asset]);
  return data;
}

/** The slice of an asset's peaks that a clip plays, per channel. */
export function clipPeaks(data: PeakData, offset: number, duration: number): Float32Array[] {
  const perSec = data.sampleRate / data.step;
  const i0 = Math.max(0, Math.floor(offset * perSec));
  const i1 = Math.ceil((offset + duration) * perSec);
  return data.channels.map((ch) => ch.subarray(i0, Math.min(ch.length, i1)));
}
