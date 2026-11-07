import { useEffect, useRef } from "react";
import { meterLevels } from "../../state/meters";

/** Linear level → meter height. −60 dBFS is the floor, 0 dBFS sits at 91 %, +6 at the top. */
function levelToFrac(level: number): number {
  if (level <= 0.001) return 0;
  const db = 20 * Math.log10(level);
  return Math.min(1, Math.max(0, (db + 60) / 66));
}

interface MeterProps {
  id: string;
  label: string;
  width?: number;
  showClip?: boolean;
}

/** Peak meter with a 1.2 s peak hold. Reads `meterLevels` in its own frame loop. */
export function Meter({ id, label, width = 6, showClip }: MeterProps) {
  const fill = useRef<HTMLSpanElement>(null);
  const hold = useRef<HTMLSpanElement>(null);
  const clip = useRef<HTMLSpanElement>(null);

  useEffect(() => {
    let raf = 0;
    let peak = 0;
    let peakAt = 0;
    let clipAt = -1e9;
    const loop = (t: number) => {
      const level = meterLevels.get(id) ?? 0;
      const frac = levelToFrac(level);
      if (fill.current) fill.current.style.clipPath = `inset(${((1 - frac) * 100).toFixed(2)}% 0 0 0)`;
      if (frac >= peak || t - peakAt > 1200) {
        peak = frac;
        peakAt = t;
      }
      if (hold.current) {
        hold.current.style.bottom = `${(peak * 100).toFixed(2)}%`;
        hold.current.style.opacity = peak > 0.02 ? "1" : "0";
      }
      if (level >= 1) clipAt = t;
      if (clip.current) clip.current.dataset.on = t - clipAt < 2000 ? "true" : "false";
      raf = requestAnimationFrame(loop);
    };
    raf = requestAnimationFrame(loop);
    return () => cancelAnimationFrame(raf);
  }, [id]);

  return (
    <div className="meter" role="img" aria-label={`${label} level meter`} style={{ width }}>
      {showClip && (
        <span ref={clip} className="meter-clip" title="Clipped in the last 2 seconds">
          clip
        </span>
      )}
      <span className="meter-well">
        <span ref={fill} className="meter-fill" />
        <span ref={hold} className="meter-hold" />
      </span>
    </div>
  );
}
