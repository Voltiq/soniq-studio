//! Dry/Wet for effects that have no mix of their own (the EQ, gate, limiter,
//! utility, de-esser, auto pan, and racks): the effect's output blended with
//! what went in. Where the effect comes late (the EQ in linear-phase mode),
//! the dry sound waits just as long, so the two line up instead of combing.

use super::{AudioFx, Sidechain, Smooth};
use crate::{FxContext, Params};

/// Frames blended at a time: the dry copy is this long.
const CHUNK: usize = 1024;

pub struct Mixed {
    inner: AudioFx,
    /// 0 (dry) … 1 (wet).
    wet: Smooth,
    dry: [Vec<f32>; 2],
    /// The dry sound, held back by the effect's latency.
    wait: [Vec<f32>; 2],
    at: usize,
}

fn amount(p: &dyn Params) -> f32 {
    (p.or("wet", 100.0) / 100.0).clamp(0.0, 1.0) as f32
}

impl Mixed {
    pub fn new(inner: AudioFx, sr: f32, p: &dyn Params) -> Self {
        let late = inner.latency() as usize;
        Self { inner, wet: Smooth::new(amount(p), sr), dry: [vec![0.0; CHUNK], vec![0.0; CHUNK]], wait: [vec![0.0; late], vec![0.0; late]], at: 0 }
    }

    pub fn inner(&self) -> &AudioFx {
        &self.inner
    }

    pub fn configure(&mut self, p: &dyn Params) {
        self.inner.configure(p);
        self.wet.set(amount(p));
    }

    pub fn process(&mut self, left: &mut [f32], right: &mut [f32], sidechain: Sidechain<'_>, ctx: &FxContext) {
        // All wet and nothing held back: the effect alone.
        if self.wait[0].is_empty() && self.wet.settled_at(1.0) {
            return self.inner.process(left, right, sidechain, ctx);
        }
        let late = self.wait[0].len();
        // A latency that changed since it was built (automation) cannot be made up for.
        let waits = late > 0 && self.inner.latency() as usize == late;
        let mut done = 0;
        while done < left.len() {
            let n = (left.len() - done).min(CHUNK);
            let (l, r) = (&mut left[done..done + n], &mut right[done..done + n]);
            self.dry[0][..n].copy_from_slice(l);
            self.dry[1][..n].copy_from_slice(r);
            let key = sidechain.map(|(kl, kr)| (&kl[done..done + n], &kr[done..done + n]));
            let here = FxContext { pos: ctx.pos + done as u64, beat: ctx.beat + done as f64 / ctx.samples_per_beat.max(1.0), ..*ctx };
            self.inner.process(l, r, key, &here);
            for i in 0..n {
                let (mut dl, mut dr) = (self.dry[0][i], self.dry[1][i]);
                if waits {
                    std::mem::swap(&mut dl, &mut self.wait[0][self.at]);
                    std::mem::swap(&mut dr, &mut self.wait[1][self.at]);
                    self.at = (self.at + 1) % late;
                }
                let w = self.wet.next();
                l[i] = dl + (l[i] - dl) * w;
                r[i] = dr + (r[i] - dr) * w;
            }
            done += n;
        }
    }

    pub fn reduction(&self) -> Option<f32> {
        self.inner.reduction()
    }

    pub fn latency(&self) -> u32 {
        self.inner.latency()
    }

    pub fn adopt(&mut self, old: &mut Mixed) {
        self.inner.adopt(&mut old.inner);
        self.wet.adopt(&old.wet);
        if self.wait[0].len() == old.wait[0].len() {
            std::mem::swap(&mut self.wait, &mut old.wait);
            self.at = old.at;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::AudioFx;
    use super::super::test_util::*;

    #[test]
    fn dry_wet_blends_any_effect_with_its_input() {
        let tone = sine(300.0, 0.5, 24_000);
        // A utility at −6 dB: all dry is the input, half way is the average of both.
        let level = |wet: f64| {
            let mut fx = AudioFx::build("utility", SR, &params(&[("gain", -6.0206), ("wet", wet)])).unwrap();
            let (l, _) = run(&mut fx, &tone);
            peak(&l[12_000..])
        };
        assert!((level(100.0) - 0.25).abs() < 1e-3, "{}", level(100.0));
        assert!((level(0.0) - 0.5).abs() < 1e-4, "{}", level(0.0));
        assert!((level(50.0) - 0.375).abs() < 1e-3, "{}", level(50.0));

        // Turned while it runs, it glides instead of jumping.
        let mut fx = AudioFx::build("utility", SR, &params(&[("gain", -60.0), ("wet", 0.0)])).unwrap();
        run(&mut fx, &tone);
        fx.configure(&params(&[("gain", -60.0), ("wet", 100.0)]));
        let (l, _) = run(&mut fx, &tone);
        assert!(peak(&l[..100]) > 0.3 && peak(&l[12_000..]) < 0.01);
    }

    #[test]
    fn the_dry_sound_waits_for_a_late_effect() {
        // A flat linear-phase EQ only delays: half dry, half wet must still be the input, late, with no comb.
        let mut eq = AudioFx::build("eq", SR, &params(&[("phase", 1.0), ("wet", 50.0)])).unwrap();
        let late = eq.latency() as usize;
        assert!(late > 0);
        let tone = sine(1000.0, 0.5, 48_000);
        let (l, _) = run(&mut eq, &tone);
        let worst = (late + 9600..40_000).fold(0.0f32, |m, i| m.max((l[i] - tone[i - late]).abs()));
        assert!(worst < 0.01, "{worst}");
    }
}
