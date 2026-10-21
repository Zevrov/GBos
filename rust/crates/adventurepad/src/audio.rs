//! The game's sound: the core's 48 kHz stereo into a ring the output device pulls from (cpal), resampled to
//! the device's rate.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// Interleaved stereo samples at [`mgba_sys::SAMPLE_RATE`], written by the emulator thread.
#[derive(Clone, Default)]
pub struct Ring(Arc<Mutex<VecDeque<i16>>>);

/// At most this much sound waits (in stereo frames): more is dropped, oldest first.
const MAX_QUEUED: usize = mgba_sys::SAMPLE_RATE as usize / 5;

impl Ring {
    pub fn push(&self, samples: &[i16]) {
        let mut q = self.0.lock().unwrap();
        q.extend(samples.iter().copied());
        let over = q.len().saturating_sub(MAX_QUEUED * 2);
        q.drain(..over - over % 2);
    }

    /// Stereo frames waiting to play.
    pub fn queued(&self) -> usize {
        self.0.lock().unwrap().len() / 2
    }

    pub fn clear(&self) {
        self.0.lock().unwrap().clear();
    }
}

/// Fast-forward's sound at its normal pitch: the core makes speed x as much audio as plays in real time, so
/// this plays slices of it ([`GRAIN`] long) at their own speed, skips what falls between them, and
/// crossfades each slice into the next ([`OVERLAP`]). A slice is taken only when the output runs low, so it
/// keeps up with any speed, uncapped included. The music jumps ahead in steps instead of chipmunking.
#[derive(Default)]
pub struct FastSound {
    /// Interleaved stereo from the core since the last slice.
    input: Vec<i16>,
    /// The last slice's end, faded into the next slice's start.
    tail: Vec<i16>,
}

/// A slice: 50 ms at 48 kHz, in stereo frames.
pub const GRAIN: usize = 2400;
/// Its crossfade into the next: 5 ms.
pub const OVERLAP: usize = 240;
/// Below this much queued sound (stereo frames, ~60 ms) the next slice goes out.
const FAST_TARGET: usize = 2880;

impl FastSound {
    /// The core's audio for one fast-forwarded frame; slices go to `ring` as it needs them.
    pub fn feed(&mut self, samples: &[i16], ring: &Ring) {
        self.input.extend_from_slice(samples);
        if self.input.len() < (GRAIN + OVERLAP) * 2 {
            return;
        }
        if ring.queued() >= FAST_TARGET {
            // Enough is playing: keep only the newest slice's worth, so the next one is current.
            let keep = (GRAIN + OVERLAP) * 2;
            let drop = self.input.len().saturating_sub(keep * 2);
            self.input.drain(..drop - drop % 2);
            return;
        }
        let slice: Vec<i16> = self.input.drain(..(GRAIN + OVERLAP) * 2).collect();
        self.input.clear();
        let mut out = Vec::with_capacity(GRAIN * 2);
        for f in 0..GRAIN {
            for c in 0..2 {
                let s = slice[f * 2 + c] as f32;
                let v = match self.tail.get(f * 2 + c) {
                    // Equal-power-ish crossfade: the last slice out, this one in.
                    Some(&t) if f < OVERLAP => {
                        let x = (f as f32 + 0.5) / OVERLAP as f32;
                        t as f32 * (1.0 - x) + s * x
                    }
                    _ => s,
                };
                out.push(v.round().clamp(i16::MIN as f32, i16::MAX as f32) as i16);
            }
        }
        // What follows this slice in the core's audio: faded into the next slice.
        self.tail = slice[GRAIN * 2..].to_vec();
        ring.push(&out);
    }

    /// Fast-forward ended: what's held goes, so normal play starts clean.
    pub fn reset(&mut self) {
        self.input.clear();
        self.tail.clear();
    }
}

/// `input` (interleaved stereo, any length) squeezed or stretched to `frames` stereo frames: each output frame
/// the average of its share of the input. Fast-forward's sound effects, made at speed x, played in real time:
/// sped up (higher, shorter), in step with the game.
pub fn squeeze(input: &[i16], frames: usize) -> Vec<i16> {
    let n = input.len() / 2;
    if n == 0 || frames == 0 {
        return vec![0; frames * 2];
    }
    let mut out = Vec::with_capacity(frames * 2);
    for k in 0..frames {
        let a = k * n / frames;
        let b = ((k + 1) * n / frames).max(a + 1).min(n);
        for c in 0..2 {
            let sum: i32 = (a..b).map(|i| input[i * 2 + c] as i32).sum();
            out.push((sum / (b - a) as i32) as i16);
        }
    }
    out
}

/// `a` + `b`, sample by sample, clipped (the 1x music and the sped-up effects).
pub fn mix(a: &[i16], b: &[i16]) -> Vec<i16> {
    a.iter()
        .zip(b.iter().chain(std::iter::repeat(&0)))
        .map(|(&x, &y)| (x as i32 + y as i32).clamp(i16::MIN as i32, i16::MAX as i32) as i16)
        .collect()
}

/// The open output stream; sound stops when it's dropped.
pub struct Output {
    _stream: cpal::Stream,
}

impl Output {
    /// The default output device, playing `ring`. None (no sound) if there's no device.
    pub fn open(ring: Ring) -> Option<Output> {
        let device = cpal::default_host().default_output_device()?;
        let config = device.default_output_config().ok()?;
        let format = config.sample_format();
        let config: cpal::StreamConfig = config.into();
        let channels = config.channels as usize;
        let step = mgba_sys::SAMPLE_RATE as f64 / config.sample_rate.0 as f64;
        let mut res = Resampler {
            pos: 0.0,
            prev: (0.0, 0.0),
            next: (0.0, 0.0),
            step,
        };
        let err = |e| eprintln!("audio: {e}");
        let stream = match format {
            cpal::SampleFormat::F32 => device.build_output_stream(
                &config,
                move |out: &mut [f32], _| res.fill(&ring, out, channels, |v| v),
                err,
                None,
            ),
            cpal::SampleFormat::I16 => device.build_output_stream(
                &config,
                move |out: &mut [i16], _| res.fill(&ring, out, channels, |v| (v * 32767.0) as i16),
                err,
                None,
            ),
            _ => return None,
        }
        .ok()?;
        stream.play().ok()?;
        Some(Output { _stream: stream })
    }
}

/// Linear interpolation from the core's rate to the device's.
struct Resampler {
    pos: f64,
    prev: (f32, f32),
    next: (f32, f32),
    step: f64,
}

impl Resampler {
    fn fill<T: Copy>(
        &mut self,
        ring: &Ring,
        out: &mut [T],
        channels: usize,
        conv: impl Fn(f32) -> T,
    ) {
        let mut q = ring.0.lock().unwrap();
        for frame in out.chunks_mut(channels) {
            while self.pos >= 1.0 {
                self.pos -= 1.0;
                self.prev = self.next;
                // An empty ring holds the last sample (no click), fading it out.
                self.next = match (q.pop_front(), q.pop_front()) {
                    (Some(l), Some(r)) => (l as f32 / 32768.0, r as f32 / 32768.0),
                    _ => (self.next.0 * 0.98, self.next.1 * 0.98),
                };
            }
            let t = self.pos as f32;
            let l = self.prev.0 + (self.next.0 - self.prev.0) * t;
            let r = self.prev.1 + (self.next.1 - self.prev.1) * t;
            for (c, s) in frame.iter_mut().enumerate() {
                *s = conv(if c % 2 == 0 { l } else { r });
            }
            self.pos += self.step;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn squeezing_keeps_the_level_and_takes_the_length() {
        // 4 frames of a constant to 2: the same level, half as long.
        assert_eq!(
            squeeze(&[100, -100, 100, -100, 100, -100, 100, -100], 2),
            [100, -100, 100, -100]
        );
        // Nothing to squeeze: silence of the right length.
        assert_eq!(squeeze(&[], 3), [0; 6]);
        // A 4x-sped tone: 4 cycles of it fit where 1 played.
        let tone: Vec<i16> = (0..3200)
            .flat_map(|i| {
                let v = ((i as f64 / 800.0 * std::f64::consts::TAU).sin() * 10000.0) as i16;
                [v, v]
            })
            .collect();
        let out = squeeze(&tone, 800);
        let rises = out
            .chunks(2)
            .collect::<Vec<_>>()
            .windows(2)
            .filter(|w| w[0][0] < 0 && w[1][0] >= 0)
            .count();
        assert!((3..=4).contains(&rises), "{rises}");
    }

    #[test]
    fn mixing_clips_instead_of_wrapping() {
        assert_eq!(
            mix(&[30000, -30000, 5], &[10000, -10000]),
            [32767, -32768, 5]
        );
    }

    /// A 440 Hz tone fast-forwarded at 4x: what plays is still 440 Hz, at real-time rate.
    #[test]
    fn fast_forward_keeps_the_pitch() {
        let ring = Ring::default();
        let mut fast = FastSound::default();
        let rate = mgba_sys::SAMPLE_RATE as f64;
        let mut t = 0u64; // input sample clock
        let mut played: Vec<i16> = Vec::new();
        // 3 seconds of real time in 10 ms ticks: the device takes 480 frames, the core makes 4x that.
        for _ in 0..300 {
            for _ in 0..4 {
                let chunk: Vec<i16> = (0..480)
                    .flat_map(|_| {
                        let v = ((t as f64 / rate * 440.0 * std::f64::consts::TAU).sin() * 12000.0)
                            as i16;
                        t += 1;
                        [v, v]
                    })
                    .collect();
                fast.feed(&chunk, &ring);
            }
            let mut q = ring.0.lock().unwrap();
            for _ in 0..480 {
                match (q.pop_front(), q.pop_front()) {
                    (Some(l), Some(_)) => played.push(l),
                    _ => played.push(0),
                }
            }
        }
        // Past the first half second: rising zero crossings per second = the frequency.
        let steady = &played[24_000..];
        let rises = steady.windows(2).filter(|w| w[0] < 0 && w[1] >= 0).count() as f64;
        let hz = rises / (steady.len() as f64 / rate);
        assert!((hz - 440.0).abs() < 440.0 * 0.05, "plays at {hz:.0} Hz");
        // And it never ran dry for long: silence would show as runs of zeros.
        let gaps = steady
            .windows(48)
            .filter(|w| w.iter().all(|&s| s == 0))
            .count();
        assert_eq!(gaps, 0, "the output ran dry");
    }
}
