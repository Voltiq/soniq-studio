//! Electric Piano: a two-operator FM tine model in the spirit of the classic
//! FM electric pianos. Velocity brightens the tone, low notes ring longer,
//! and a stereo tremolo pans it gently.

use crate::common::{db_gain, decay_coef, midi_hz, pick_voice, sin_turns};
use crate::{NoteEvent, Params};

const VOICES: usize = 16;
const VOICE_GAIN: f32 = 0.3;
const TREMOLO_HZ: f32 = 4.5;

#[derive(Debug, Clone, Copy, Default)]
struct Voice {
    active: bool,
    held: bool,
    sustained: bool,
    damped: bool,
    pitch: u8,
    velocity: f32,
    age: u64,
    carrier: f32,
    modulator: f32,
    tine: f32,
    tine_mod: f32,
    /// Brightness of the FM attack and of the tine; both fade fast.
    mod_env: f32,
    tine_env: f32,
    /// Body level; decays while held, faster once damped.
    level: f32,
    /// Per-sample decay of the body while the key is held.
    ring: f32,
}

pub struct EPiano {
    sr: f32,
    /// 0 (dark) to 1 (bright).
    brightness: f32,
    tremolo: f32,
    damper: f32,
    gain: f32,
    voices: [Voice; VOICES],
    clock: u64,
    pedal: bool,
    trem_phase: f32,
}

impl EPiano {
    pub fn new(sr: f32, p: &dyn Params) -> Self {
        let mut piano = Self {
            sr,
            brightness: 0.5,
            tremolo: 0.0,
            damper: 0.0,
            gain: VOICE_GAIN,
            voices: [Voice::default(); VOICES],
            clock: 0,
            pedal: false,
            trem_phase: 0.0,
        };
        piano.configure(p);
        piano
    }

    /// Takes new parameter values; playing voices carry on.
    pub fn configure(&mut self, p: &dyn Params) {
        self.brightness = ((p.or("tone", 10.0) + 100.0) / 200.0) as f32;
        self.tremolo = (p.or("trem", 18.0) / 100.0) as f32;
        self.damper = decay_coef((p.or("release", 420.0) / 1000.0) as f32, self.sr);
        self.gain = db_gain(p.or("volume", 0.0)) * VOICE_GAIN;
    }

    pub fn handle(&mut self, event: NoteEvent) {
        match event {
            NoteEvent::On { pitch, velocity } => {
                self.clock += 1;
                let i = pick_voice(self.voices.iter().map(|v| (v.active, v.held, v.age)));
                // Low notes ring for seconds, high ones for well under one.
                let ring_seconds = (4.0 * (-(f32::from(pitch) - 48.0) / 24.0).exp2()).clamp(0.6, 9.0);
                self.voices[i] = Voice {
                    active: true,
                    held: true,
                    pitch,
                    velocity,
                    age: self.clock,
                    level: 1.0,
                    mod_env: 1.0,
                    tine_env: 1.0,
                    ring: decay_coef(ring_seconds, self.sr),
                    ..Voice::default()
                };
            }
            NoteEvent::Off { pitch } => {
                let pedal = self.pedal;
                for v in self.voices.iter_mut().filter(|v| v.active && v.held && v.pitch == pitch) {
                    v.held = false;
                    if pedal { v.sustained = true } else { v.damped = true }
                }
            }
            NoteEvent::Sustain(down) => {
                self.pedal = down;
                if !down {
                    for v in self.voices.iter_mut().filter(|v| v.sustained) {
                        v.sustained = false;
                        v.damped = true;
                    }
                }
            }
            NoteEvent::AllNotesOff => {
                self.pedal = false;
                for v in self.voices.iter_mut().filter(|v| v.active) {
                    v.held = false;
                    v.sustained = false;
                    v.damped = true;
                }
            }
            NoteEvent::AllSoundOff => {
                self.pedal = false;
                self.voices = [Voice::default(); VOICES];
            }
        }
    }

    pub fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        let sr = self.sr;
        let trem_step = TREMOLO_HZ / sr;
        let mod_decay = decay_coef(1.1, sr);
        let tine_decay = decay_coef(0.12, sr);
        for v in self.voices.iter_mut().filter(|v| v.active) {
            let f = midi_hz(f32::from(v.pitch));
            let step = (f / sr).min(0.45);
            // A bright, inharmonic "tine" partial at 14 times the pitch gives the bell-like attack.
            let tine_step = (f * 14.0 / sr).min(0.45);
            let index_peak = (0.6 + 2.4 * self.brightness) * (0.3 + 0.7 * v.velocity);
            let tine_peak = 0.25 * v.velocity * (0.4 + self.brightness);
            let mut trem = self.trem_phase;
            let gain = self.gain * (0.25 + 0.75 * v.velocity);
            for (l, r) in left.iter_mut().zip(right.iter_mut()) {
                // The modulation index is in radians; phases are in turns.
                let index = index_peak * v.mod_env + 0.12;
                let body = sin_turns(v.carrier + index * sin_turns(v.modulator) * 0.16);
                let tine = sin_turns(v.tine + 0.3 * sin_turns(v.tine_mod)) * tine_peak * v.tine_env;
                v.carrier = (v.carrier + step).fract();
                v.modulator = (v.modulator + step).fract();
                v.tine = (v.tine + step).fract();
                v.tine_mod = (v.tine_mod + tine_step).fract();
                v.mod_env *= mod_decay;
                v.tine_env *= tine_decay;
                v.level *= if v.damped { self.damper } else { v.ring };
                let out = (body + tine) * v.level * gain;
                // Stereo tremolo: the sound sways between the speakers.
                let sway = sin_turns(trem) * self.tremolo * 0.5;
                trem += trem_step;
                *l += out * (1.0 - sway);
                *r += out * (1.0 + sway);
            }
            if v.level < 1e-4 {
                v.active = false;
            }
        }
        self.trem_phase = (self.trem_phase + trem_step * left.len() as f32).fract();
    }

    pub fn adopt(&mut self, old: &mut EPiano) {
        std::mem::swap(&mut self.voices, &mut old.voices);
        self.clock = old.clock;
        self.pedal = old.pedal;
        self.trem_phase = old.trem_phase;
    }

    pub fn is_silent(&self) -> bool {
        !self.voices.iter().any(|v| v.active)
    }
}

#[cfg(test)]
mod tests {
    use crate::test_util::*;
    use crate::{Instrument, NoteEvent};

    #[test]
    fn rings_at_the_note_and_dies_away_when_released() {
        let mut ep = Instrument::build("epiano", SR, &defaults, &[]).unwrap();
        ep.handle(NoteEvent::On { pitch: 57, velocity: 0.8 });
        let (l, r) = render(&mut ep, SR as usize);
        assert!(peak(&l) > 0.05 && peak(&r) > 0.05);
        // After the bright attack it settles towards the fundamental.
        let f = frequency(&l[24_000..], SR);
        assert!((f - 220.0).abs() < 3.0, "{f}");
        ep.handle(NoteEvent::Off { pitch: 57 });
        render(&mut ep, SR as usize * 2);
        assert!(ep.is_silent());
    }
}
