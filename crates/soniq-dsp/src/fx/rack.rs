//! Audio effect rack: chains of effects side by side. Every chain hears the
//! rack's input, runs it through its own effects in order, and the chains
//! are added up, each at its own level and pan. Eight macros turn any of the
//! effects' knobs. A rack with no chains passes the sound through.
//!
//! Nothing inside makes up for a delay, so the effects in a rack are the
//! built-in ones, and an EQ in a rack stays minimum phase.

use super::{AudioFx, Sidechain, Smooth};
use crate::common::{db_gain, pan_gains};
use crate::rack::{MACRO_IDS, MacroLink};
use crate::{FxContext, ParamBank, Params};

/// Frames each chain processes at a time in the rack's own buffers.
const SCRATCH: usize = 1024;

/// One effect of a chain, as the control thread builds it.
pub struct RackFx {
    /// The device's id, so a rebuilt rack carries its state over.
    pub id: String,
    pub fx: AudioFx,
    /// Its own parameter values; macros write into them.
    pub bank: ParamBank,
    pub links: Vec<MacroLink>,
}

/// One chain of an effect rack, as the control thread builds it.
pub struct FxRackChain {
    pub id: String,
    pub fx: Vec<RackFx>,
    /// Level in dB and pan −1…1.
    pub volume: f64,
    pub pan: f32,
}

struct Chain {
    id: String,
    fx: Vec<RackFx>,
    gains: [Smooth; 2],
}

pub struct FxRack {
    chains: Vec<Chain>,
    buf: [Vec<f32>; 2],
    sum: [Vec<f32>; 2],
}

impl FxRack {
    /// `p` holds the rack's own parameters: its macros.
    pub fn new(chains: Vec<FxRackChain>, sr: f32, p: &dyn Params) -> Self {
        let mut rack = Self {
            chains: chains
                .into_iter()
                .map(|c| {
                    let (gl, gr) = pan_gains(c.pan);
                    let gain = db_gain(c.volume);
                    Chain { id: c.id, fx: c.fx, gains: [Smooth::new(gl * gain, sr), Smooth::new(gr * gain, sr)] }
                })
                .collect(),
            buf: [vec![0.0; SCRATCH], vec![0.0; SCRATCH]],
            sum: [vec![0.0; SCRATCH], vec![0.0; SCRATCH]],
        };
        rack.configure(p);
        rack
    }

    /// Takes new macro values: every mapped knob hears what its macro now says.
    pub fn configure(&mut self, p: &dyn Params) {
        for unit in self.chains.iter_mut().flat_map(|c| c.fx.iter_mut()).filter(|u| !u.links.is_empty()) {
            for link in &unit.links {
                let amount = p.or(MACRO_IDS[link.macro_index.min(7)], 0.0) / 100.0;
                unit.bank.set(link.param, link.value(amount));
            }
            unit.fx.configure(&unit.bank);
        }
    }

    pub fn process(&mut self, left: &mut [f32], right: &mut [f32], sidechain: Sidechain<'_>, ctx: &FxContext) {
        if self.chains.is_empty() {
            return;
        }
        let mut done = 0;
        while done < left.len() {
            let n = (left.len() - done).min(SCRATCH);
            let (l, r) = (&mut left[done..done + n], &mut right[done..done + n]);
            let key = sidechain.map(|(kl, kr)| (&kl[done..done + n], &kr[done..done + n]));
            let here = FxContext { pos: ctx.pos + done as u64, beat: ctx.beat + done as f64 / ctx.samples_per_beat.max(1.0), ..*ctx };
            self.sum[0][..n].fill(0.0);
            self.sum[1][..n].fill(0.0);
            for chain in &mut self.chains {
                let (bl, br) = self.buf.split_at_mut(1);
                let (bl, br) = (&mut bl[0][..n], &mut br[0][..n]);
                bl.copy_from_slice(l);
                br.copy_from_slice(r);
                for unit in &mut chain.fx {
                    unit.fx.process(bl, br, key, &here);
                }
                for i in 0..n {
                    self.sum[0][i] += bl[i] * chain.gains[0].next();
                    self.sum[1][i] += br[i] * chain.gains[1].next();
                }
            }
            l.copy_from_slice(&self.sum[0][..n]);
            r.copy_from_slice(&self.sum[1][..n]);
            done += n;
        }
    }

    /// Each effect carries on from the one with the same id in the old rack,
    /// each chain's level from where it had got to.
    pub fn adopt(&mut self, old: &mut FxRack) {
        for chain in &mut self.chains {
            let Some(before) = old.chains.iter_mut().find(|c| c.id == chain.id) else { continue };
            for (gain, was) in chain.gains.iter_mut().zip(&before.gains) {
                gain.adopt(was);
            }
            for unit in &mut chain.fx {
                if let Some(was) = before.fx.iter_mut().find(|u| u.id == unit.id) {
                    unit.fx.adopt(&mut was.fx);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_util::*;
    use super::*;

    fn unit(id: &str, kind: &str, values: &[(&str, f64)]) -> RackFx {
        let bank = ParamBank::new(values.iter().map(|(k, v)| (k.to_string(), *v)));
        RackFx { id: id.into(), fx: AudioFx::build(kind, SR, &bank).unwrap(), bank, links: Vec::new() }
    }

    fn chain(id: &str, fx: Vec<RackFx>, volume: f64) -> FxRackChain {
        FxRackChain { id: id.into(), fx, volume, pan: 0.0 }
    }

    fn rack(chains: Vec<FxRackChain>, p: &[(&str, f64)]) -> AudioFx {
        AudioFx::Rack(FxRack::new(chains, SR as f32, &params(p)))
    }

    #[test]
    fn chains_run_side_by_side_and_add_up() {
        let tone = sine(300.0, 0.4, 9600);
        // No chains: the sound passes.
        let (l, _) = run(&mut rack(Vec::new(), &[]), &tone);
        assert!(l.iter().zip(&tone).all(|(a, b)| (a - b).abs() < 1e-6));
        // An empty chain at −6 dB beside a chain that halves and then halves again: 0.5 + 0.25 of the input.
        let quarter = chain("b", vec![unit("u1", "utility", &[("gain", -6.0206)]), unit("u2", "utility", &[("gain", -6.0206)])], 0.0);
        let (l, r) = run(&mut rack(vec![chain("a", Vec::new(), -6.0206), quarter], &[]), &tone);
        assert!((peak(&l[4800..]) - 0.3).abs() < 1e-3 && (peak(&r[4800..]) - 0.3).abs() < 1e-3, "{}", peak(&l[4800..]));
    }

    #[test]
    fn a_macro_turns_a_knob_inside_and_a_rebuilt_rack_carries_on() {
        // Macro 2 sweeps a utility's gain from −60 dB to 0 dB.
        let build = |macro2: f64| {
            let mut u = unit("u", "utility", &[("gain", -60.0)]);
            let gain = u.bank.find("gain").unwrap();
            u.links = vec![MacroLink { macro_index: 1, param: gain, from: -60.0, to: 0.0, min: -60.0, max: 24.0, log: false, step: None }];
            rack(vec![chain("a", vec![u], 0.0)], &[("macro2", macro2)])
        };
        let tone = sine(300.0, 0.4, 9600);
        let mut closed = build(0.0);
        let (l, _) = run(&mut closed, &tone);
        assert!(peak(&l[4800..]) < 0.001);
        closed.configure(&params(&[("macro2", 100.0)]));
        let (l, _) = run(&mut closed, &tone);
        assert!((peak(&l[4800..]) - 0.4).abs() < 1e-3, "{}", peak(&l[4800..]));

        // A delay's echo survives the rack being rebuilt around it.
        let echo = || rack(vec![chain("a", vec![unit("d", "delay", &[("sync", 0.0), ("time", 100.0), ("fb", 0.0), ("mix", 100.0)])], 0.0)], &[]);
        let mut old = echo();
        let mut click = vec![0.0f32; 2400];
        click[0] = 1.0;
        run(&mut old, &click);
        let mut new = echo();
        new.adopt(&mut old);
        let (l, _) = run(&mut new, &vec![0.0; 4800]);
        assert!(peak(&l) > 0.3, "{}", peak(&l));
    }
}
