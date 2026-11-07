//! Instrument rack: several instruments layered, or split across the
//! keyboard and the velocity range, each with its own level and pan.

use crate::common::{db_gain, pan_gains};
use crate::{Instrument, NoteEvent, ParamBank, Params};

/// Frames each layer renders at a time into the rack's own buffers.
const SCRATCH: usize = 1024;

/// The rack's macro parameters, by index.
const MACRO_IDS: [&str; 8] = ["macro1", "macro2", "macro3", "macro4", "macro5", "macro6", "macro7", "macro8"];

/// A macro turning one parameter of a chain's instrument (mirrors `MacroMap`).
#[derive(Debug, Clone, Copy)]
pub struct MacroLink {
    /// 0–7.
    pub macro_index: usize,
    /// The parameter's index in the chain's bank.
    pub param: usize,
    /// The parameter's value at macro 0 % and 100 %.
    pub from: f64,
    pub to: f64,
    /// The parameter's range, curve and steps.
    pub min: f64,
    pub max: f64,
    pub log: bool,
    pub step: Option<f64>,
}

impl MacroLink {
    fn value(&self, amount: f64) -> f64 {
        let t = amount.clamp(0.0, 1.0);
        let v = if self.log && self.from > 0.0 && self.to > 0.0 { self.from * (self.to / self.from).powf(t) } else { self.from + (self.to - self.from) * t };
        let v = match self.step.filter(|s| *s > 0.0) {
            Some(step) => self.min + ((v - self.min) / step).round() * step,
            None => v,
        };
        v.clamp(self.min, self.max)
    }
}

/// One chain of a rack, as the control thread builds it.
pub struct RackLayer {
    /// The chain's id, so a rebuilt rack can take over its voices.
    pub id: String,
    pub instrument: Instrument,
    /// The instrument's own parameter values; macros write into them.
    pub bank: ParamBank,
    pub links: Vec<MacroLink>,
    /// Notes it plays, inclusive.
    pub keys: (u8, u8),
    /// Velocities (1–127) it plays, inclusive.
    pub velocities: (u8, u8),
    /// Level in dB and pan −1…1.
    pub volume: f64,
    pub pan: f32,
}

struct Layer {
    id: String,
    instrument: Instrument,
    bank: ParamBank,
    links: Vec<MacroLink>,
    keys: (u8, u8),
    velocities: (u8, u8),
    gains: (f32, f32),
    /// Notes this layer took, so their note-offs come to it and no other.
    held: [u64; 2],
}

impl Layer {
    fn holds(&self, pitch: u8) -> bool {
        self.held[usize::from(pitch >> 6) & 1] & (1 << (pitch & 63)) != 0
    }

    fn set_held(&mut self, pitch: u8, on: bool) {
        let (word, bit) = (usize::from(pitch >> 6) & 1, 1u64 << (pitch & 63));
        if on { self.held[word] |= bit } else { self.held[word] &= !bit }
    }
}

pub struct Rack {
    layers: Vec<Layer>,
    gain: f32,
    left: Vec<f32>,
    right: Vec<f32>,
}

impl Rack {
    /// `p` holds the rack's own parameters: its macros and volume (dB).
    pub fn new(layers: Vec<RackLayer>, p: &dyn Params) -> Self {
        let mut rack = Self {
            layers: layers
                .into_iter()
                .map(|l| {
                    let (gl, gr) = pan_gains(l.pan);
                    let gain = db_gain(l.volume);
                    Layer {
                        id: l.id,
                        instrument: l.instrument,
                        bank: l.bank,
                        links: l.links,
                        keys: l.keys,
                        velocities: l.velocities,
                        gains: (gl * gain, gr * gain),
                        held: [0; 2],
                    }
                })
                .collect(),
            gain: 1.0,
            left: vec![0.0; SCRATCH],
            right: vec![0.0; SCRATCH],
        };
        rack.configure(p);
        rack
    }

    /// Takes new macro and volume values: each chain's instrument hears what
    /// its macros now say.
    pub fn configure(&mut self, p: &dyn Params) {
        self.gain = db_gain(p.or("volume", 0.0));
        for layer in self.layers.iter_mut().filter(|l| !l.links.is_empty()) {
            for link in &layer.links {
                let amount = p.or(MACRO_IDS[link.macro_index.min(7)], 0.0) / 100.0;
                layer.bank.set(link.param, link.value(amount));
            }
            layer.instrument.configure(&layer.bank);
        }
    }

    pub fn handle(&mut self, event: NoteEvent) {
        match event {
            NoteEvent::On { pitch, velocity } => {
                let vel = (velocity * 127.0).round().clamp(1.0, 127.0) as u8;
                for layer in self.layers.iter_mut() {
                    let in_keys = (layer.keys.0..=layer.keys.1).contains(&pitch);
                    if in_keys && (layer.velocities.0..=layer.velocities.1).contains(&vel) {
                        layer.set_held(pitch, true);
                        layer.instrument.handle(event);
                    }
                }
            }
            NoteEvent::Off { pitch } => {
                for layer in self.layers.iter_mut().filter(|l| l.holds(pitch)) {
                    layer.set_held(pitch, false);
                    layer.instrument.handle(event);
                }
            }
            NoteEvent::AllNotesOff | NoteEvent::AllSoundOff => {
                for layer in self.layers.iter_mut() {
                    layer.held = [0; 2];
                    layer.instrument.handle(event);
                }
            }
            NoteEvent::Sustain(_) => {
                for layer in self.layers.iter_mut() {
                    layer.instrument.handle(event);
                }
            }
        }
    }

    pub fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        let mut done = 0;
        while done < left.len() {
            let n = (left.len() - done).min(SCRATCH);
            for layer in self.layers.iter_mut().filter(|l| !l.instrument.is_silent()) {
                let (sl, sr) = (&mut self.left[..n], &mut self.right[..n]);
                sl.fill(0.0);
                sr.fill(0.0);
                layer.instrument.render(sl, sr);
                let (gl, gr) = (layer.gains.0 * self.gain, layer.gains.1 * self.gain);
                for (out, s) in left[done..done + n].iter_mut().zip(sl.iter()) {
                    *out += s * gl;
                }
                for (out, s) in right[done..done + n].iter_mut().zip(sr.iter()) {
                    *out += s * gr;
                }
            }
            done += n;
        }
    }

    /// Each chain carries on from the chain with the same id in the old rack.
    pub fn adopt(&mut self, old: &mut Rack) {
        for layer in self.layers.iter_mut() {
            if let Some(before) = old.layers.iter_mut().find(|l| l.id == layer.id) {
                layer.instrument.adopt(&mut before.instrument);
                layer.held = before.held;
            }
        }
    }

    pub fn is_silent(&self) -> bool {
        self.layers.iter().all(|l| l.instrument.is_silent())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::*;

    fn layer(id: &str, wave: f64, keys: (u8, u8), velocities: (u8, u8)) -> RackLayer {
        let values = [("osc1", wave), ("osc2", wave), ("sub", 0.0), ("lfoAmt", 0.0), ("vibrato", 0.0), ("volume", 0.0)];
        let bank = ParamBank::new(values.map(|(k, v)| (k.to_string(), v)));
        let instrument = Instrument::build("synth", SR, &bank, &[]).unwrap();
        RackLayer { id: id.into(), instrument, bank, links: Vec::new(), keys, velocities, volume: 0.0, pan: 0.0 }
    }

    fn rack_params(_: &str) -> Option<f64> {
        None
    }

    #[test]
    fn notes_go_to_the_chains_whose_zones_they_fall_in() {
        let mut rack = Instrument::Rack(Rack::new(vec![layer("low", 3.0, (0, 59), (1, 127)), layer("soft", 3.0, (60, 127), (1, 64))], &rack_params));
        // Above the split and loud: no chain plays it.
        rack.handle(NoteEvent::On { pitch: 72, velocity: 1.0 });
        assert!(rack.is_silent());
        rack.handle(NoteEvent::On { pitch: 72, velocity: 0.3 });
        rack.handle(NoteEvent::On { pitch: 48, velocity: 1.0 });
        let (l, _) = render(&mut rack, 4800);
        assert!(peak(&l) > 0.05);
        // Each note-off reaches the chain that took the note, and the rack falls silent.
        rack.handle(NoteEvent::Off { pitch: 72 });
        rack.handle(NoteEvent::Off { pitch: 48 });
        render(&mut rack, 48_000);
        assert!(rack.is_silent());
    }

    #[test]
    fn a_rebuilt_rack_keeps_its_chains_sounding() {
        let mut old = Instrument::Rack(Rack::new(vec![layer("a", 0.0, (0, 127), (1, 127))], &rack_params));
        old.handle(NoteEvent::On { pitch: 60, velocity: 0.8 });
        render(&mut old, 512);
        let quieter = |id: &str| (id == "volume").then_some(-6.0);
        let mut new = Instrument::Rack(Rack::new(vec![layer("a", 1.0, (0, 127), (1, 127)), layer("b", 0.0, (0, 127), (1, 127))], &quieter));
        new.adopt(&mut old);
        let (l, _) = render(&mut new, 512);
        assert!(peak(&l) > 0.01, "the held note carries on");
        new.handle(NoteEvent::Off { pitch: 60 });
        render(&mut new, 48_000);
        assert!(new.is_silent(), "and its note-off still finds it");
    }

    #[test]
    fn a_macro_retunes_a_playing_chain() {
        // Macro 1 turns the synth's volume from −36 dB (at 0 %) to 0 dB.
        let mut l = layer("a", 3.0, (0, 127), (1, 127));
        let volume = l.bank.find("volume").unwrap();
        l.links = vec![MacroLink { macro_index: 0, param: volume, from: -36.0, to: 0.0, min: -36.0, max: 6.0, log: false, step: None }];
        let mut rack = Instrument::Rack(Rack::new(vec![l], &rack_params));
        rack.handle(NoteEvent::On { pitch: 69, velocity: 1.0 });
        let (quiet, _) = render(&mut rack, 4800);
        rack.configure(&|id: &str| (id == "macro1").then_some(100.0));
        let (loud, _) = render(&mut rack, 4800);
        assert!(peak(&loud[2400..]) > peak(&quiet[2400..]) * 20.0, "{} {}", peak(&loud[2400..]), peak(&quiet[2400..]));
    }
}
