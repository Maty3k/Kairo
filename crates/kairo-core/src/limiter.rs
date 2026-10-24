// SPDX-License-Identifier: GPL-3.0-or-later
//! The mix track's look-ahead limiter (docs/research/10-audio-rust.md §5.7): stereo-linked,
//! a −1 dBFS ceiling, 5 ms of look-ahead and a 100 ms release.
//!
//! For each frame it works out the gain that would keep that frame under the ceiling, takes the
//! smallest such gain over the coming look-ahead, follows a fall at once and a rise with a
//! one-pole release, then smooths the result with a moving average as long as the look-ahead.
//! The output is the input delayed by the look-ahead times that gain. Research 10 takes the
//! minimum over L frames; this takes it over L + 1, so every frame's gain is at or below what
//! its own peak needs (with L frames the gain comes one frame late). Samples that aren't finite
//! count as silence, and samples beyond ±[`MAX_INPUT`] are clamped to it. That keeps the moving
//! average's rounding inside the ceiling's margin, so the output never goes over the ceiling,
//! whatever the input. Nothing allocates after [`Limiter::new`].

use std::collections::VecDeque;
use std::fmt;

use crate::timeline::SAMPLE_RATE;

/// Look-ahead, and so the delay the limiter adds: 240 frames, 5 ms.
pub const LOOKAHEAD_FRAMES: usize = 240;
/// The ceiling, in dBFS.
pub const CEILING_DBFS: f64 = -1.0;
/// The release time constant, in seconds.
pub const RELEASE_S: f64 = 0.1;
/// The loudest sample the limiter takes, +60 dBFS; louder ones are clamped to it.
pub const MAX_INPUT: f32 = 1_024.0;

/// A sample as the limiter takes it: silence if it isn't finite, else clamped to ±[`MAX_INPUT`].
pub(crate) fn clean(sample: f32) -> f32 {
    if sample.is_finite() {
        sample.clamp(-MAX_INPUT, MAX_INPUT)
    } else {
        0.0
    }
}

/// The limiter for one stereo bus.
#[derive(Clone)]
pub struct Limiter {
    /// The ceiling as a linear amplitude: 0.891.
    ceiling: f64,
    /// The release's per-frame coefficient.
    release: f64,
    /// Frames seen so far.
    frame: usize,
    /// (frame, gain needed) pairs whose gains rise from front to back: the front is the minimum
    /// over the last L + 1 frames.
    window: VecDeque<(usize, f64)>,
    /// The released envelope.
    envelope: f64,
    /// The last L envelope values, for the moving average.
    history: Vec<f64>,
    /// Their sum, kept up frame by frame and added up afresh every L frames, so rounding can't
    /// build up.
    sum: f64,
    /// Input frames waiting out the look-ahead, interleaved.
    delay: Vec<f32>,
    /// The smallest gain applied since the last [`Limiter::take_min_gain`].
    min_gain: f64,
}

/// Why the limiter refused a block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LimitError {
    /// A stereo block must hold whole frames: an even number of samples.
    RaggedBlock(usize),
}

impl fmt::Display for LimitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RaggedBlock(len) => {
                write!(
                    f,
                    "a stereo block must have an even number of samples, not {len}"
                )
            }
        }
    }
}

impl std::error::Error for LimitError {}

impl fmt::Debug for Limiter {
    /// Leaves the audio out: logs never hold it.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Limiter")
            .field("frame", &self.frame)
            .field("envelope", &self.envelope)
            .field("min_gain", &self.min_gain)
            .finish_non_exhaustive()
    }
}

impl Default for Limiter {
    fn default() -> Self {
        Self::new()
    }
}

impl Limiter {
    /// A limiter at rest: no gain reduction, and silence in the look-ahead.
    pub fn new() -> Self {
        Self {
            ceiling: 10_f64.powf(CEILING_DBFS / 20.0),
            release: (-1.0 / (RELEASE_S * SAMPLE_RATE as f64)).exp(),
            frame: 0,
            window: VecDeque::with_capacity(LOOKAHEAD_FRAMES + 2),
            envelope: 1.0,
            history: vec![1.0; LOOKAHEAD_FRAMES],
            sum: LOOKAHEAD_FRAMES as f64,
            delay: vec![0.0; 2 * LOOKAHEAD_FRAMES],
            min_gain: 1.0,
        }
    }

    /// Limits an interleaved stereo block in place. Each output frame is the input frame from
    /// [`LOOKAHEAD_FRAMES`] earlier times its gain, so the block's audio comes out 5 ms late.
    pub fn process(&mut self, stereo: &mut [f32]) -> Result<(), LimitError> {
        if !stereo.len().is_multiple_of(2) {
            return Err(LimitError::RaggedBlock(stereo.len()));
        }
        for frame in stereo.as_chunks_mut::<2>().0.iter_mut() {
            let slot = self.frame % LOOKAHEAD_FRAMES;
            let (left, right) = (clean(frame[0]), clean(frame[1]));
            let peak = f64::from(left.abs().max(right.abs()));
            let needed = if peak > self.ceiling {
                self.ceiling / peak
            } else {
                1.0
            };
            while self.window.back().is_some_and(|&(_, gain)| gain >= needed) {
                self.window.pop_back();
            }
            self.window.push_back((self.frame, needed));
            while self
                .window
                .front()
                .is_some_and(|&(seen, _)| seen + LOOKAHEAD_FRAMES < self.frame)
            {
                self.window.pop_front();
            }
            let lowest = self.window.front().map_or(1.0, |&(_, gain)| gain);
            self.envelope = if lowest < self.envelope {
                lowest
            } else {
                let released = lowest + (self.envelope - lowest) * self.release;
                // Rounding would stall the release just short of its target, so the gain would
                // never read 1 again; this last step is under 1e-8 dB.
                if lowest - released < 1e-9 {
                    lowest
                } else {
                    released
                }
            };
            // Added up afresh every L frames, the sum is off by at most about 7e-12. With input
            // within ±1,024 it is at least 240·c/1,024 ≈ 0.21, so the gain is off by at most
            // 3.3e-11 of itself: less than the 4.9e-11 by which c sits under the f32 rounding
            // midpoint above it, so no output rounds up past the ceiling.
            if slot == 0 {
                self.sum = self.history.iter().sum();
            }
            self.sum += self.envelope - self.history[slot];
            self.history[slot] = self.envelope;
            let gain = self.sum / LOOKAHEAD_FRAMES as f64;
            self.min_gain = self.min_gain.min(gain);
            let delayed = (self.delay[2 * slot], self.delay[2 * slot + 1]);
            self.delay[2 * slot] = left;
            self.delay[2 * slot + 1] = right;
            frame[0] = (f64::from(delayed.0) * gain) as f32;
            frame[1] = (f64::from(delayed.1) * gain) as f32;
            self.frame = self.frame.wrapping_add(1);
        }
        Ok(())
    }

    /// The smallest gain applied since the last call (1 if none was reduced), for the mix
    /// meter's gain reduction; then starts over.
    pub fn take_min_gain(&mut self) -> f64 {
        std::mem::replace(&mut self.min_gain, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meter::gain_db;

    // Expected values come from a Python model of the same limiter, written from research 10
    // §5.7, and from closed forms.

    fn ceiling() -> f32 {
        10_f64.powf(CEILING_DBFS / 20.0) as f32
    }

    /// Runs (left, right) frames through `limiter` in 480-frame blocks.
    fn run_frames(limiter: &mut Limiter, input: &[(f32, f32)]) -> Vec<(f32, f32)> {
        let mut out = Vec::with_capacity(input.len());
        for block in input.chunks(480) {
            let mut stereo: Vec<f32> = block.iter().flat_map(|&(l, r)| [l, r]).collect();
            limiter.process(&mut stereo).unwrap();
            out.extend(stereo.as_chunks::<2>().0.iter().map(|&[l, r]| (l, r)));
        }
        out
    }

    /// Runs mono `input` through a limiter as identical left and right, in 480-frame blocks,
    /// and returns the left output.
    fn run(input: &[f32]) -> (Vec<f32>, Limiter) {
        let mut limiter = Limiter::new();
        let frames: Vec<(f32, f32)> = input.iter().map(|&x| (x, x)).collect();
        let out = run_frames(&mut limiter, &frames);
        (out.iter().map(|&(l, _)| l).collect(), limiter)
    }

    /// A 0.6 bed with 10-frame bursts of 4.0 every 0.5 s from frame 12,000, so the gain falls
    /// from 1 to c/4 while there is audio to read it back from.
    fn bed_and_bursts() -> Vec<f32> {
        let mut input = vec![0.6_f32; 96_000];
        for start in (12_000..input.len()).step_by(24_000) {
            input[start..start + 10].fill(4.0);
        }
        input
    }

    #[test]
    fn never_goes_over_the_ceiling() {
        // Research 10 §8.4 asks for random and impulsive input: here random input in [−4, 4]
        // plus an impulse train at 4.0, 10 s.
        let mut seed = 1_u64;
        let mut next = || {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            ((seed >> 11) as f64 / (1_u64 << 53) as f64 * 2.0 - 1.0) as f32 * 4.0
        };
        let mut input: Vec<f32> = (0..480_000).map(|_| next()).collect();
        for k in (1_000..input.len()).step_by(4_800) {
            input[k] = 4.0;
        }
        let (out, _) = run(&input);
        let peak = out.iter().fold(0.0_f32, |peak, &y| peak.max(y.abs()));
        assert!(peak <= ceiling(), "{peak}");
    }

    #[test]
    fn steps_gently_into_a_burst() {
        // Research 10 §8.4's "no steps above X dB per sample". The gain, read back from the
        // output over the delayed input, falls by at most 20·log10(1 + (1 − g)/(240·g)) a frame,
        // where g = c/P is the deepest gain for a bus peak P: 0.1253 dB at P = 4 (four
        // full-scale stems at 0 dB), 0.59 dB at 15.92 (four at the +12 dB maximum). A shorter
        // moving average, or none, steps further.
        let input = bed_and_bursts();
        let (out, _) = run(&input);
        let gain = |i: usize| f64::from(out[i]) / f64::from(input[i - LOOKAHEAD_FRAMES]);
        let worst = (LOOKAHEAD_FRAMES + 1..out.len())
            .map(|i| gain_db(gain(i - 1)) - gain_db(gain(i)))
            .fold(0.0_f64, f64::max);
        assert!(worst > 0.12 && worst < 0.13, "{worst}");
    }

    #[test]
    fn releases_over_100_ms() {
        let input = bed_and_bursts();
        let (out, _) = run(&input);
        // The first burst ends at frame 12,009 and leaves the window 240 frames later. From
        // there the envelope is 1 − (1 − c/4)·r^k with r = exp(−1/4,800), and the gain is its
        // mean over the last 240 frames: 0.721148 another 4,800 + 240 frames on.
        let at = 12_009 + LOOKAHEAD_FRAMES + 4_800 + LOOKAHEAD_FRAMES;
        let gain = f64::from(out[at]) / f64::from(input[at - LOOKAHEAD_FRAMES]);
        assert!((gain - 0.721_148).abs() < 1e-6, "{gain}");
    }

    #[test]
    fn links_the_channels() {
        let mut input = vec![(0.6_f32, 0.0_f32); 4_800];
        input[1_000].1 = 4.0;
        let out = run_frames(&mut Limiter::new(), &input);
        let (left, right) = out[1_000 + LOOKAHEAD_FRAMES];
        assert!(right <= ceiling() && right > 0.891, "{right}");
        // The left channel takes the right one's gain, c/4.
        let expected = 0.6 * 10_f64.powf(-1.0 / 20.0) / 4.0;
        assert!((f64::from(left) - expected).abs() < 1e-7, "{left}");
    }

    #[test]
    fn an_impulse_comes_out_at_the_ceiling_5_ms_late() {
        let mut input = vec![0.0_f32; 48_000];
        input[1_000] = 4.0;
        let (out, mut limiter) = run(&input);
        // With a minimum over only 240 frames the gain would come one frame late, and this
        // frame would come out about 2.7e-6 over the ceiling.
        assert!((out[1_240] - ceiling()).abs() < 1e-6, "{}", out[1_240]);
        assert!(out[1_240] <= ceiling());
        assert!((gain_db(limiter.take_min_gain()) + 13.041).abs() < 0.001);
        assert_eq!(limiter.take_min_gain(), 1.0);
    }

    #[test]
    fn quiet_audio_passes_unchanged_5_ms_late() {
        let input: Vec<f32> = (0..48_000)
            .map(|i| {
                (0.5 * (std::f64::consts::TAU * 1_000.0 * f64::from(i) / 48_000.0).sin()) as f32
            })
            .collect();
        let (out, mut limiter) = run(&input);
        assert!(out[..LOOKAHEAD_FRAMES].iter().all(|&y| y == 0.0));
        assert_eq!(
            out[LOOKAHEAD_FRAMES..],
            input[..input.len() - LOOKAHEAD_FRAMES]
        );
        assert_eq!(limiter.take_min_gain(), 1.0);
    }

    #[test]
    fn recovers_fully_after_limiting() {
        // An impulse on a quiet bed: the release reaches 1 after about 2.1 s, and from then on
        // quiet audio passes unchanged again and the gain reduction reads 0 dB.
        let mut input = vec![(0.25_f32, 0.25_f32); 192_000];
        input[1_000] = (4.0, 4.0);
        let mut limiter = Limiter::new();
        run_frames(&mut limiter, &input[..144_000]);
        assert!(limiter.take_min_gain() < 0.3);
        let out = run_frames(&mut limiter, &input[144_000..]);
        assert_eq!(limiter.take_min_gain(), 1.0);
        assert_eq!(
            out,
            input[144_000 - LOOKAHEAD_FRAMES..192_000 - LOOKAHEAD_FRAMES]
        );
    }

    #[test]
    fn garbage_comes_out_finite_and_under_the_ceiling() {
        let mut input = vec![(0.25_f32, 0.25_f32); 4_800];
        let garbage = [
            (f32::INFINITY, 0.25),
            (f32::NEG_INFINITY, f32::NEG_INFINITY),
            (f32::NAN, 0.25),
            (1e38, 1e38),
            (-3e38, 0.25),
            (f32::MAX, f32::MIN),
            (2_000.0, 0.25),
        ];
        for (k, &frame) in garbage.iter().enumerate() {
            input[1_000 + 7 * k] = frame;
        }
        let out = run_frames(&mut Limiter::new(), &input);
        for (i, &(left, right)) in out.iter().enumerate() {
            assert!(left.is_finite() && right.is_finite(), "{i}");
            assert!(left.abs() <= ceiling() && right.abs() <= ceiling(), "{i}");
        }
        // A sample that isn't finite counts as silence.
        assert_eq!(out[1_000 + LOOKAHEAD_FRAMES].0, 0.0);
        assert_eq!(out[1_014 + LOOKAHEAD_FRAMES].0, 0.0);
    }

    #[test]
    fn a_huge_burst_stays_under_the_ceiling_wherever_it_starts() {
        // A jump from quiet audio to a huge bus leaves the moving sum's rounding large next to
        // the tiny gain; unclamped, a bus of 1e5 comes out an f32 step over the ceiling.
        for offset in 0..480 {
            let mut input = vec![0.25_f32; 480 + offset];
            input.extend(std::iter::repeat_n(1e5_f32, 480));
            input.extend(std::iter::repeat_n(0.25_f32, 720));
            let (out, _) = run(&input);
            let peak = out.iter().fold(0.0_f32, |peak, &y| peak.max(y.abs()));
            assert!(peak <= ceiling(), "offset {offset}: {peak}");
        }
    }

    #[test]
    fn block_sizes_do_not_change_the_output() {
        let mut seed = 7_u64;
        let mut next = || {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            ((seed >> 11) as f64 / (1_u64 << 53) as f64 * 6.0 - 3.0) as f32
        };
        let input: Vec<f32> = (0..2 * 48_000).map(|_| next()).collect();
        let mut runs = Vec::new();
        for size in [2, 960, 14_000] {
            let mut limiter = Limiter::new();
            let mut out = input.clone();
            for block in out.chunks_mut(size) {
                limiter.process(block).unwrap();
            }
            // The moving sum too: it is added up afresh on the same frames whatever the blocks,
            // so its rounding can't build up in a long block.
            runs.push((out, limiter.sum.to_bits()));
        }
        assert!(runs[1..].iter().all(|run| *run == runs[0]));
    }

    #[test]
    fn debug_output_holds_no_audio() {
        let mut limiter = Limiter::new();
        let mut block = [0.123_456_f32; 960];
        limiter.process(&mut block).unwrap();
        let text = format!("{limiter:?}");
        assert!(!text.contains("0.12345"), "{text}");
        assert!(text.len() < 200, "{}", text.len());
    }

    #[test]
    fn refuses_half_frames() {
        let mut odd = [0.0_f32; 961];
        assert_eq!(
            Limiter::new().process(&mut odd),
            Err(LimitError::RaggedBlock(961))
        );
    }
}
