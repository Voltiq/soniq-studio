//! Offline renders of demo parts to audio. The demo song uses the pad, so the
//! audio editor has a clip to work on; drums and bass are played live by the
//! instruments now, but stay here for quick test audio. Deliberately simple
//! oscillators, envelopes and one-pole filters.

use std::f32::consts::TAU;

use soniq_io::AudioBuffer;

#[derive(Debug, Clone, Copy)]
pub struct DemoNote {
    /// Beats from the start of the part.
    pub start: f64,
    pub length: f64,
    pub pitch: u8,
    pub velocity: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Voice {
    /// GM drum map: 36 kick, 38 snare, 42 closed hat, 46 open hat.
    Drums,
    Bass,
    Pad,
}

/// Renders `notes` with `voice` into a stereo buffer `length_beats` long.
pub fn render_part(notes: &[DemoNote], voice: Voice, sample_rate: u32, bpm: f64, length_beats: f64) -> AudioBuffer {
    let sr = sample_rate as f32;
    let samples_per_beat = f64::from(sample_rate) * 60.0 / bpm;
    let total = (length_beats * samples_per_beat).ceil() as usize;
    let mut out = AudioBuffer::silent(sample_rate, 2, total);
    let mut noise = Noise(0x9e37_79b9);

    for note in notes {
        let start = (note.start * samples_per_beat) as usize;
        let held = (note.length * samples_per_beat) as usize;
        let [left, right] = &mut out.channels[..] else { unreachable!("two channels") };
        match voice {
            Voice::Drums => drum(note, start, sr, &mut noise, left, right),
            Voice::Bass => bass(note, start, held, sr, left, right),
            Voice::Pad => pad(note, start, held, sr, left, right),
        }
    }

    let target = match voice {
        Voice::Drums => 0.8,
        Voice::Bass => 0.6,
        Voice::Pad => 0.42,
    };
    normalize(&mut out, target);
    out
}

struct Noise(u32);

impl Noise {
    fn next(&mut self) -> f32 {
        // xorshift32
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

fn one_pole(cutoff: f32, sr: f32) -> f32 {
    1.0 - (-TAU * cutoff / sr).exp()
}

fn freq(pitch: u8) -> f32 {
    440.0 * 2f32.powf((f32::from(pitch) - 69.0) / 12.0)
}

fn add(buf: &mut [f32], i: usize, v: f32) {
    if let Some(s) = buf.get_mut(i) {
        *s += v;
    }
}

fn drum(note: &DemoNote, start: usize, sr: f32, noise: &mut Noise, left: &mut [f32], right: &mut [f32]) {
    let v = note.velocity;
    match note.pitch {
        36 => {
            let mut phase = 0.0_f32;
            for i in 0..(0.45 * sr) as usize {
                let t = i as f32 / sr;
                phase += TAU * (45.0 + 110.0 * (-t * 28.0).exp()) / sr;
                let s = phase.sin() * (-t * 7.0).exp() * v;
                add(left, start + i, s);
                add(right, start + i, s);
            }
        }
        38 => {
            let (mut lp, a) = (0.0_f32, one_pole(6500.0, sr));
            let mut phase = 0.0_f32;
            for i in 0..(0.25 * sr) as usize {
                let t = i as f32 / sr;
                lp += a * (noise.next() - lp);
                phase += TAU * 190.0 / sr;
                let s = (lp * 0.6 * (-t * 18.0).exp() + phase.sin() * 0.45 * (-t * 25.0).exp()) * v;
                add(left, start + i, s);
                add(right, start + i, s);
            }
        }
        42 | 46 => {
            let open = note.pitch == 46;
            let (mut lp, a) = (0.0_f32, one_pole(7000.0, sr));
            let decay = if open { 9.0 } else { 60.0 };
            for i in 0..((if open { 0.4 } else { 0.08 }) * sr) as usize {
                let t = i as f32 / sr;
                let n = noise.next();
                lp += a * (n - lp);
                let s = (n - lp) * 0.3 * (-t * decay).exp() * v;
                // Hats sit a little right of centre.
                add(left, start + i, s * 0.8);
                add(right, start + i, s);
            }
        }
        _ => {}
    }
}

fn envelope(i: usize, held: usize, attack: f32, release: f32, sr: f32) -> f32 {
    let t = i as f32 / sr;
    let a = (t / attack).min(1.0);
    if i < held { a } else { a * (-((i - held) as f32 / sr) / release).exp() }
}

fn bass(note: &DemoNote, start: usize, held: usize, sr: f32, left: &mut [f32], right: &mut [f32]) {
    let f = freq(note.pitch);
    let (mut lp1, mut lp2) = (0.0_f32, 0.0_f32);
    let mut phase = 0.0_f32;
    let len = held + (0.25 * sr) as usize;
    for i in 0..len {
        let t = i as f32 / sr;
        phase = (phase + f / sr).fract();
        let saw = phase * 2.0 - 1.0;
        // Filter opens on the attack, then settles: a plucky sub bass.
        let a = one_pole(350.0 + 1400.0 * (-t * 12.0).exp(), sr);
        lp1 += a * (saw - lp1);
        lp2 += a * (lp1 - lp2);
        let s = lp2 * envelope(i, held, 0.004, 0.05, sr) * note.velocity;
        add(left, start + i, s);
        add(right, start + i, s);
    }
}

fn pad(note: &DemoNote, start: usize, held: usize, sr: f32, left: &mut [f32], right: &mut [f32]) {
    let base = freq(note.pitch);
    let detune = [0.996_f32, 1.0, 1.004];
    let mut phases = [0.0_f32, 0.33, 0.66];
    let (mut lp_l, mut lp_r) = ([0.0_f32; 2], [0.0_f32; 2]);
    let a = one_pole(1300.0, sr);
    let len = held + (1.2 * sr) as usize;
    for i in 0..len {
        let mut voices = [0.0_f32; 3];
        for (k, v) in voices.iter_mut().enumerate() {
            phases[k] = (phases[k] + base * detune[k] / sr).fract();
            *v = phases[k] * 2.0 - 1.0;
        }
        // Outer voices spread left and right for width.
        let l_in = voices[0] + voices[1] * 0.5;
        let r_in = voices[2] + voices[1] * 0.5;
        lp_l[0] += a * (l_in - lp_l[0]);
        lp_l[1] += a * (lp_l[0] - lp_l[1]);
        lp_r[0] += a * (r_in - lp_r[0]);
        lp_r[1] += a * (lp_r[0] - lp_r[1]);
        let env = envelope(i, held, 0.45, 0.6, sr) * note.velocity * 0.3;
        add(left, start + i, lp_l[1] * env);
        add(right, start + i, lp_r[1] * env);
    }
}

fn normalize(buffer: &mut AudioBuffer, target: f32) {
    let peak = buffer.peak();
    if peak > 0.0 {
        let g = target / peak;
        buffer.channels.iter_mut().flatten().for_each(|s| *s *= g);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_audible_parts_of_the_right_length() {
        let notes = [DemoNote { start: 0.0, length: 1.0, pitch: 36, velocity: 1.0 }];
        let out = render_part(&notes, Voice::Drums, 48_000, 120.0, 4.0);
        assert_eq!(out.frames(), 96_000);
        assert!((out.peak() - 0.8).abs() < 1e-4);
    }
}
