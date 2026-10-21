//! Shortening sound without changing its pitch (WSOLA: waveform-similarity overlap-add), for fast-forward's
//! sound effects: the game makes speed x as much sound as plays in real time, and this plays all of it, at
//! its own pitch, in that real time. Windows of the input go out half-overlapped, each taken speed x further
//! on than the last, nudged to where it best continues the sound already out so the joins don't click.

/// A window: 1024 stereo frames (~21 ms at 48 kHz); windows go out every half of one.
const WINDOW: usize = 1024;
const HOP: usize = WINDOW / 2;
/// How far a window may be nudged (either way) to line up with the sound before it: ~5 ms.
const SEEK: usize = 256;

/// Streams stereo i16 in, at any speed-up, and hands out exactly as many frames as asked for.
pub struct Wsola {
    /// Input not yet past (interleaved stereo, as f32).
    input: Vec<f32>,
    /// Where the next window is taken (frames into `input`), before its nudge.
    pos: f64,
    /// Where the last window's natural continuation starts (frames into `input`): what the next one is
    /// lined up against.
    continuation: Option<usize>,
    /// Output being overlap-added: WINDOW frames, the first HOP of them complete.
    ola: Vec<f32>,
    /// Finished output not yet handed out.
    out: std::collections::VecDeque<f32>,
    window: Vec<f32>,
}

impl Default for Wsola {
    fn default() -> Wsola {
        // A Hann window: two half-overlapped add up to 1, so a steady sound keeps its level.
        let window = (0..WINDOW)
            .map(|i| 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / WINDOW as f64).cos())
            .map(|w| w as f32)
            .collect();
        Wsola {
            input: Vec::new(),
            pos: 0.0,
            continuation: None,
            ola: vec![0.0; WINDOW * 2],
            out: Default::default(),
            window,
        }
    }
}

impl Wsola {
    /// `input` (the game's sound since the last call) played in `frames` frames: `input.len() / 2 / frames` is
    /// the speed-up. Exactly `frames` stereo frames come back.
    pub fn process(&mut self, input: &[i16], frames: usize) -> Vec<i16> {
        let in_frames = input.len() / 2;
        self.input.extend(input.iter().map(|&s| s as f32));
        let speed = if frames == 0 {
            1.0
        } else {
            (in_frames as f64 / frames as f64).max(0.25)
        };
        while self.out.len() / 2 < frames {
            if !self.step(speed) {
                break;
            }
        }
        // Short of input (a stall): what's missing is silence.
        let mut result = Vec::with_capacity(frames * 2);
        for _ in 0..frames * 2 {
            let v = self.out.pop_front().unwrap_or(0.0);
            result.push(v.round().clamp(i16::MIN as f32, i16::MAX as f32) as i16);
        }
        self.trim();
        result
    }

    fn frames(&self) -> usize {
        self.input.len() / 2
    }

    /// The nudge (in frames, -SEEK..=SEEK) that lines a window at `at` up best with `reference`.
    fn best_offset(&self, at: usize, reference: usize) -> isize {
        let lo = at.saturating_sub(SEEK);
        let hi = (at + SEEK).min(self.frames().saturating_sub(WINDOW));
        let mut best = (at as isize - lo as isize).min(0);
        let mut best_score = f32::MIN;
        // Mono, every other frame over the first half window: plenty to line two waveforms up.
        for cand in lo..=hi {
            let mut score = 0.0f32;
            for k in (0..HOP).step_by(2) {
                let a = self.input[(cand + k) * 2] + self.input[(cand + k) * 2 + 1];
                let b = self.input[(reference + k) * 2] + self.input[(reference + k) * 2 + 1];
                score += a * b;
            }
            if score > best_score {
                best_score = score;
                best = cand as isize - at as isize;
            }
        }
        best
    }

    /// One window out; false when there isn't enough input for it yet.
    fn step(&mut self, speed: f64) -> bool {
        let at = self.pos.round() as usize;
        if at + SEEK + WINDOW > self.frames() {
            return false;
        }
        let start = match self.continuation {
            Some(r) if r + HOP <= self.frames() => (at as isize + self.best_offset(at, r)) as usize,
            _ => at,
        };
        for i in 0..WINDOW {
            for c in 0..2 {
                self.ola[i * 2 + c] += self.input[(start + i) * 2 + c] * self.window[i];
            }
        }
        // The first HOP frames are finished; the rest wait for the next window.
        self.out.extend(self.ola.drain(..HOP * 2));
        self.ola.extend(std::iter::repeat_n(0.0, HOP * 2));
        self.continuation = Some(start + HOP);
        self.pos += HOP as f64 * speed;
        true
    }

    /// Drops input every future window is past.
    fn trim(&mut self) {
        let keep_from = (self.pos as usize)
            .saturating_sub(SEEK)
            .min(self.continuation.unwrap_or(usize::MAX));
        if keep_from > WINDOW * 4 {
            self.input.drain(..keep_from * 2);
            self.pos -= keep_from as f64;
            self.continuation = self.continuation.map(|c| c - keep_from);
        }
        // Never more than ~1 s behind: a stall's backlog is dropped, not played late.
        let max = 48_000 + WINDOW * 2;
        if self.frames() > max {
            let drop = self.frames() - max;
            self.input.drain(..drop * 2);
            self.pos = (self.pos - drop as f64).max(0.0);
            self.continuation = None;
        }
    }

    /// Fast-forward ended: nothing held over.
    pub fn reset(&mut self) {
        *self = Wsola::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(hz: f64, frames: usize, from: usize) -> Vec<i16> {
        (from..from + frames)
            .flat_map(|i| {
                let v =
                    ((i as f64 / 48_000.0 * hz * std::f64::consts::TAU).sin() * 10_000.0) as i16;
                [v, v]
            })
            .collect()
    }

    fn pitch(out: &[i16]) -> f64 {
        let left: Vec<i16> = out.chunks(2).map(|c| c[0]).collect();
        let rises = left.windows(2).filter(|w| w[0] < 0 && w[1] >= 0).count() as f64;
        rises / (left.len() as f64 / 48_000.0)
    }

    /// A 440 Hz tone at 3x: what plays is still 440 Hz, as many frames as asked for, at its level.
    #[test]
    fn three_times_as_fast_keeps_the_pitch() {
        let mut w = Wsola::default();
        let mut out = Vec::new();
        let mut t = 0;
        for _ in 0..180 {
            // One music frame's worth (800 frames) out, 3x that in.
            let input = tone(440.0, 2400, t);
            t += 2400;
            out.extend(w.process(&input, 800));
        }
        assert_eq!(out.len(), 180 * 800 * 2);
        let steady = &out[48_000..]; // past the start-up
        let hz = pitch(steady);
        assert!((hz - 440.0).abs() < 440.0 * 0.03, "plays at {hz:.0} Hz");
        let rms =
            (steady.iter().map(|&s| (s as f64).powi(2)).sum::<f64>() / steady.len() as f64).sqrt();
        assert!((rms - 7071.0).abs() < 7071.0 * 0.15, "level {rms:.0}");
    }

    /// Silence in, silence out; and short of input, it fills with silence instead of stalling.
    #[test]
    fn silence_and_stalls() {
        let mut w = Wsola::default();
        assert!(w.process(&[0; 4800], 800).iter().all(|&s| s == 0));
        assert_eq!(w.process(&[], 800).len(), 1600);
    }
}
