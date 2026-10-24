// SPDX-License-Identifier: GPL-3.0-or-later
//! The mixer (docs/research/10-audio-rust.md §5.7; docs/PLAN.md §2.3): every 10 ms block it
//! turns each stem's audio into that stem's track, and the stems into the mix track.
//!
//! A stem track (game, mic, voice chat, other apps: T2 to T5) is its source as captured,
//! hard-clipped to ±1 for its AAC encoder. The mix track (T1) is the sum of the stems in the
//! mix times their gains, through the limiter. A gain change, or a stem joining or leaving the
//! mix, ramps linearly across one block, so it makes no zipper noise. Stem tracks don't carry
//! the gains, so a later copy can mix them again (PLAN §2.3: share copies leave voice chat
//! out); the gains belong with the clip's other settings. A mono stem (the mic) goes to both
//! channels of the mix. Each stem has a meter after its gain, and the mix one after the
//! limiter. Nothing allocates after [`Mixer::new`].

use std::fmt;

use crate::limiter::{Limiter, clean};
use crate::meter::{Levels, Meter, gain_db};

/// Frames per block: 10 ms at 48 kHz.
pub const BLOCK_FRAMES: usize = 480;
/// The most stems a mixer takes: T2 to T5.
pub const MAX_STEMS: usize = 4;
/// The lowest gain a stem may have, in dB; a stem leaves the mix with [`Mixer::set_in_mix`].
pub const MIN_GAIN_DB: f32 = -60.0;
/// The highest gain a stem may have, in dB.
pub const MAX_GAIN_DB: f32 = 12.0;
/// How far behind real time the mixer reads its sources: 200 ms (research 10 §5.2).
pub const MIX_DELAY_FRAMES: i64 = 9_600;
/// The most blocks the mixer runs in one go to catch up after a stall: 1 second's worth.
pub const MAX_CATCH_UP_BLOCKS: u64 = 100;

// Each source's aligner holds enough audio for the mixer's longest catch-up.
const _: () = assert!(
    crate::aligner::MAX_WAITING_FRAMES
        >= MAX_CATCH_UP_BLOCKS as usize * BLOCK_FRAMES + MIX_DELAY_FRAMES as usize
);

/// A stem's channels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// One channel, like the mic.
    Mono,
    /// Two interleaved channels.
    Stereo,
}

impl Layout {
    /// Samples per frame.
    pub const fn channels(self) -> usize {
        match self {
            Self::Mono => 1,
            Self::Stereo => 2,
        }
    }
}

/// A stem's starting settings.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StemConfig {
    /// Its channels.
    pub layout: Layout,
    /// Its gain in the mix, in dB (research 10's defaults: game 0, mic 0, voice chat −3).
    pub gain_db: f32,
    /// Whether it is in the mix.
    pub in_mix: bool,
}

/// One stem's running state.
#[derive(Clone)]
struct Stem {
    layout: Layout,
    /// The linear gain reached at the end of the last block.
    gain: f32,
    /// The linear gain the next block ramps to.
    target_gain: f32,
    /// The mix weight (gain, or 0 out of the mix) reached at the end of the last block.
    weight: f32,
    /// The mix weight the next block ramps to.
    target_weight: f32,
    in_mix: bool,
    meter: Meter,
}

impl Stem {
    fn retarget(&mut self) {
        self.target_weight = if self.in_mix { self.target_gain } else { 0.0 };
    }
}

impl fmt::Debug for Stem {
    /// Leaves the meter out: its peak is a sample of the audio.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Stem")
            .field("layout", &self.layout)
            .field("gain", &self.gain)
            .field("target_gain", &self.target_gain)
            .field("in_mix", &self.in_mix)
            .finish_non_exhaustive()
    }
}

/// The mixer for one recording.
#[derive(Clone)]
pub struct Mixer {
    stems: Vec<Stem>,
    /// The mix of one block, interleaved stereo.
    sum: Vec<f32>,
    limiter: Limiter,
    mix_meter: Meter,
}

impl fmt::Debug for Mixer {
    /// Leaves the audio out: logs never hold it.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Mixer")
            .field("stems", &self.stems)
            .field("limiter", &self.limiter)
            .finish_non_exhaustive()
    }
}

/// How many blocks the mixer runs now, after skipping ahead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Due {
    /// Frames to skip before the first block: what is left after a stall of more than a second.
    /// Every track still gets them, as silence: see [`blocks_due`].
    pub skip_frames: i64,
    /// Blocks to run, each [`BLOCK_FRAMES`] long.
    pub blocks: u64,
}

/// Why the mixer refused a setting or a block.
#[derive(Debug, Clone, PartialEq)]
pub enum MixError {
    /// A mixer takes 1 to 4 stems.
    StemCount(usize),
    /// A block came with a different number of stems than the mixer has.
    WrongStems {
        /// The mixer's stems.
        expected: usize,
        /// The block's stems.
        got: usize,
    },
    /// No such stem.
    UnknownStem(usize),
    /// A gain must be a number from −60 to +12 dB.
    BadGain(f32),
    /// A block is not 480 frames long.
    BadBlock {
        /// The stem, or `None` for the mix.
        stem: Option<usize>,
        /// The block's length in samples.
        len: usize,
    },
}

impl fmt::Display for MixError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StemCount(stems) => {
                write!(f, "a mixer takes 1 to {MAX_STEMS} stems, not {stems}")
            }
            Self::WrongStems { expected, got } => {
                write!(
                    f,
                    "a block must have the mixer's {expected} stems, not {got}"
                )
            }
            Self::UnknownStem(stem) => write!(f, "there is no stem {stem}"),
            Self::BadGain(gain) => write!(
                f,
                "a gain must be {MIN_GAIN_DB} to {MAX_GAIN_DB} dB, not {gain}"
            ),
            Self::BadBlock {
                stem: Some(stem),
                len,
            } => write!(
                f,
                "stem {stem}'s block must be {BLOCK_FRAMES} frames, not {len} samples"
            ),
            Self::BadBlock { stem: None, len } => write!(
                f,
                "the mix block must be {BLOCK_FRAMES} stereo frames, not {len} samples"
            ),
        }
    }
}

impl std::error::Error for MixError {}

/// The gain for `gain_db`, if it is allowed.
fn linear(gain_db: f32) -> Result<f32, MixError> {
    if (MIN_GAIN_DB..=MAX_GAIN_DB).contains(&gain_db) {
        Ok(10_f32.powf(gain_db / 20.0))
    } else {
        Err(MixError::BadGain(gain_db))
    }
}

impl Mixer {
    /// A mixer for `stems`, in track order (T2 first).
    pub fn new(stems: &[StemConfig]) -> Result<Self, MixError> {
        if !(1..=MAX_STEMS).contains(&stems.len()) {
            return Err(MixError::StemCount(stems.len()));
        }
        let stems = stems
            .iter()
            .map(|config| {
                let gain = linear(config.gain_db)?;
                let weight = if config.in_mix { gain } else { 0.0 };
                Ok(Stem {
                    layout: config.layout,
                    gain,
                    target_gain: gain,
                    weight,
                    target_weight: weight,
                    in_mix: config.in_mix,
                    meter: Meter::new(),
                })
            })
            .collect::<Result<Vec<Stem>, MixError>>()?;
        Ok(Self {
            stems,
            sum: vec![0.0; 2 * BLOCK_FRAMES],
            limiter: Limiter::new(),
            mix_meter: Meter::new(),
        })
    }

    /// Sets a stem's gain; the next block ramps to it.
    pub fn set_gain_db(&mut self, stem: usize, gain_db: f32) -> Result<(), MixError> {
        let gain = linear(gain_db)?;
        let stem = self
            .stems
            .get_mut(stem)
            .ok_or(MixError::UnknownStem(stem))?;
        stem.target_gain = gain;
        stem.retarget();
        Ok(())
    }

    /// Puts a stem in the mix or takes it out; the next block ramps it in or out.
    pub fn set_in_mix(&mut self, stem: usize, in_mix: bool) -> Result<(), MixError> {
        let stem = self
            .stems
            .get_mut(stem)
            .ok_or(MixError::UnknownStem(stem))?;
        stem.in_mix = in_mix;
        stem.retarget();
        Ok(())
    }

    /// Mixes one block. `stems[i]` holds stem `i`'s 480 frames in its layout and becomes its
    /// track, clipped to ±1. `mix` gets 480 stereo frames of the limited mix, which comes out
    /// [`LOOKAHEAD_FRAMES`](crate::limiter::LOOKAHEAD_FRAMES) late: if the block started at timeline sample `pos`, the mix block
    /// starts at `pos − 240`. Samples that aren't finite count as silence, and samples beyond
    /// ±[`MAX_INPUT`](crate::limiter::MAX_INPUT) are clamped to it.
    pub fn process(&mut self, stems: &mut [&mut [f32]], mix: &mut [f32]) -> Result<(), MixError> {
        if stems.len() != self.stems.len() {
            return Err(MixError::WrongStems {
                expected: self.stems.len(),
                got: stems.len(),
            });
        }
        for (index, (block, stem)) in stems.iter().zip(&self.stems).enumerate() {
            if block.len() != BLOCK_FRAMES * stem.layout.channels() {
                return Err(MixError::BadBlock {
                    stem: Some(index),
                    len: block.len(),
                });
            }
        }
        if mix.len() != 2 * BLOCK_FRAMES {
            return Err(MixError::BadBlock {
                stem: None,
                len: mix.len(),
            });
        }
        self.sum.fill(0.0);
        for (block, stem) in stems.iter_mut().zip(&mut self.stems) {
            let channels = stem.layout.channels();
            for (frame, (samples, mixed)) in block
                .chunks_exact_mut(channels)
                .zip(self.sum.as_chunks_mut::<2>().0.iter_mut())
                .enumerate()
            {
                let ramp = (frame + 1) as f32 / BLOCK_FRAMES as f32;
                let gain = stem.gain + (stem.target_gain - stem.gain) * ramp;
                let weight = stem.weight + (stem.target_weight - stem.weight) * ramp;
                for sample in samples.iter_mut() {
                    *sample = clean(*sample);
                }
                let (left, right) = match &*samples {
                    [mono] => (*mono, *mono),
                    [left, right] => (*left, *right),
                    _ => (0.0, 0.0),
                };
                mixed[0] += weight * left;
                mixed[1] += weight * right;
                for &sample in samples.iter() {
                    stem.meter.push(&[gain * sample]);
                }
                for sample in samples.iter_mut() {
                    *sample = sample.clamp(-1.0, 1.0);
                }
            }
            stem.gain = stem.target_gain;
            stem.weight = stem.target_weight;
        }
        mix.copy_from_slice(&self.sum);
        // The length was checked above, so the limiter can't refuse it.
        if self.limiter.process(mix).is_err() {
            mix.fill(0.0);
        }
        self.mix_meter.push(mix);
        Ok(())
    }

    /// Stem `stem`'s levels after its gain since the last call, then starts over. They are for
    /// display: to tell whether the source itself is silent, meter its track instead (clipped,
    /// before the gain).
    pub fn take_stem_levels(&mut self, stem: usize) -> Option<Levels> {
        self.stems.get_mut(stem).map(|stem| stem.meter.take())
    }

    /// The mix's levels after the limiter, and the limiter's deepest gain reduction in dB (0 or
    /// below), since the last call; then starts over.
    pub fn take_mix_levels(&mut self) -> (Levels, f64) {
        (self.mix_meter.take(), gain_db(self.limiter.take_min_gain()))
    }
}

/// How many blocks the mixer runs when its next block starts at `next_pos` and real time is at
/// timeline sample `now_sample`: every whole block at least `delay_frames` old. After a stall of
/// more than a second it skips ahead and runs only the last second, so it doesn't spend a burst
/// mixing audio its sources have long dropped.
///
/// The skipped frames still reach every track, as silence: each track is gapless, with its
/// timestamps on the timeline, and the ring refuses a gap. To keep the mix in step with the
/// stems, a skip goes like this:
/// 1. Run one block of silent stems at `next_pos` and pass on its tracks as usual. This brings
///    the limiter's last 240 frames of audio out where they belong.
/// 2. Give every track `skip_frames − 480` frames of silence: the stems from `next_pos + 480`,
///    the mix from `next_pos + 240`.
/// 3. Carry on at `next_pos + skip_frames`, adding [`BLOCK_FRAMES`] per block run.
///
/// The skip saves only the mixing: the encoders still encode the silence, a few milliseconds per
/// track for each second skipped.
pub fn blocks_due(next_pos: i64, now_sample: i64, delay_frames: i64) -> Due {
    let behind = i128::from(now_sample) - i128::from(delay_frames) - i128::from(next_pos);
    let block = i128::from(BLOCK_FRAMES as u32);
    let blocks = if behind < block { 0 } else { behind / block };
    let most = i128::from(MAX_CATCH_UP_BLOCKS);
    if blocks <= most {
        Due {
            skip_frames: 0,
            blocks: u64::try_from(blocks).unwrap_or(0),
        }
    } else {
        Due {
            skip_frames: i64::try_from((blocks - most) * block).unwrap_or(i64::MAX),
            blocks: MAX_CATCH_UP_BLOCKS,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limiter::LOOKAHEAD_FRAMES;

    fn stereo(gain_db: f32) -> StemConfig {
        StemConfig {
            layout: Layout::Stereo,
            gain_db,
            in_mix: true,
        }
    }

    /// Runs `blocks` blocks of one constant stereo stem, calling `between` before each, and
    /// returns the left channel of the mix, already shifted back by the limiter's delay.
    fn run_dc(
        mixer: &mut Mixer,
        level: f32,
        blocks: usize,
        mut between: impl FnMut(usize, &mut Mixer),
    ) -> Vec<f32> {
        let mut out = Vec::new();
        for block in 0..blocks {
            between(block, mixer);
            let mut stem = vec![level; 2 * BLOCK_FRAMES];
            let mut mix = vec![0.0; 2 * BLOCK_FRAMES];
            mixer.process(&mut [&mut stem], &mut mix).unwrap();
            out.extend(mix.as_chunks::<2>().0.iter().map(|frame| frame[0]));
        }
        out.split_off(LOOKAHEAD_FRAMES)
    }

    #[test]
    fn a_gain_change_ramps_across_one_block() {
        let mut mixer = Mixer::new(&[stereo(0.0)]).unwrap();
        let out = run_dc(&mut mixer, 0.25, 4, |block, mixer| {
            if block == 1 {
                mixer.set_gain_db(0, -6.0206).unwrap();
            }
        });
        for (i, &y) in out.iter().enumerate() {
            let expected = match i {
                0..480 => 0.25,
                480..960 => 0.25 * (1.0 - 0.5 * (i - 479) as f32 / 480.0),
                _ => 0.125,
            };
            assert!((y - expected).abs() < 1e-6, "{i}: {y} vs {expected}");
        }
    }

    #[test]
    fn constant_gains_sum_exactly() {
        let mut mixer = Mixer::new(&[stereo(0.0), stereo(0.0)]).unwrap();
        let mut a = vec![0.25; 2 * BLOCK_FRAMES];
        let mut b = vec![0.5; 2 * BLOCK_FRAMES];
        let mut mix = vec![0.0; 2 * BLOCK_FRAMES];
        mixer.process(&mut [&mut a, &mut b], &mut mix).unwrap();
        assert!(mix[2 * LOOKAHEAD_FRAMES..].iter().all(|&y| y == 0.75));
    }

    #[test]
    fn a_stem_out_of_the_mix_stays_out_then_ramps_in() {
        let mut mixer = Mixer::new(&[StemConfig {
            in_mix: false,
            ..stereo(0.0)
        }])
        .unwrap();
        let out = run_dc(&mut mixer, 0.25, 6, |block, mixer| match block {
            1 => mixer.set_gain_db(0, -6.0206).unwrap(),
            3 => mixer.set_in_mix(0, true).unwrap(),
            _ => {}
        });
        // A gain change leaves it out; joining ramps it in across block 3.
        let gain = 10_f32.powf(-6.0206 / 20.0);
        for (i, &y) in out.iter().enumerate() {
            let expected = match i {
                0..1_440 => 0.0,
                1_440..1_920 => 0.25 * gain * (i - 1_439) as f32 / 480.0,
                _ => 0.25 * gain,
            };
            assert!((y - expected).abs() < 1e-6, "{i}: {y} vs {expected}");
        }
        assert!(out[..1_440].iter().all(|&y| y == 0.0));
    }

    #[test]
    fn leaving_the_mix_ramps_out_then_is_silent() {
        let mut mixer = Mixer::new(&[stereo(0.0)]).unwrap();
        let out = run_dc(&mut mixer, 0.25, 4, |block, mixer| {
            if block == 1 {
                mixer.set_in_mix(0, false).unwrap();
            }
        });
        for (i, &y) in out.iter().enumerate() {
            let expected = match i {
                0..480 => 0.25,
                480..960 => 0.25 * (1.0 - (i - 479) as f32 / 480.0),
                _ => 0.0,
            };
            assert!((y - expected).abs() < 1e-6, "{i}: {y} vs {expected}");
        }
        assert!(out[960..].iter().all(|&y| y == 0.0));
    }

    #[test]
    fn a_mono_stem_reaches_both_channels() {
        let mono = StemConfig {
            layout: Layout::Mono,
            gain_db: 0.0,
            in_mix: true,
        };
        let mut mixer = Mixer::new(&[mono]).unwrap();
        let mut stem = vec![0.25; BLOCK_FRAMES];
        let mut mix = vec![0.0; 2 * BLOCK_FRAMES];
        mixer.process(&mut [&mut stem], &mut mix).unwrap();
        assert!(mix[2 * LOOKAHEAD_FRAMES..].iter().all(|&y| y == 0.25));
    }

    #[test]
    fn tracks_are_clipped_but_the_mix_hears_the_raw_audio() {
        let mut mixer = Mixer::new(&[stereo(-12.0)]).unwrap();
        let mut stem = vec![0.0; 2 * BLOCK_FRAMES];
        stem[..6].copy_from_slice(&[1.5, -2.0, f32::NAN, 0.3, f32::INFINITY, -0.5]);
        let mut mix = vec![0.0; 2 * BLOCK_FRAMES];
        mixer.process(&mut [&mut stem], &mut mix).unwrap();
        assert_eq!(stem[..6], [1.0, -1.0, 0.0, 0.3, 0.0, -0.5]);
        // The mix of frame 0 comes out 240 frames later: 1.5 and −2 at −12 dB, not ±1.
        let gain = 10_f32.powf(-12.0 / 20.0);
        let frame0 = &mix[2 * LOOKAHEAD_FRAMES..2 * LOOKAHEAD_FRAMES + 2];
        assert!((frame0[0] - 1.5 * gain).abs() < 1e-6, "{frame0:?}");
        assert!((frame0[1] + 2.0 * gain).abs() < 1e-6, "{frame0:?}");
    }

    #[test]
    fn takes_four_stems_and_the_whole_gain_range() {
        let mic = StemConfig {
            layout: Layout::Mono,
            gain_db: -60.0,
            in_mix: true,
        };
        let mut mixer = Mixer::new(&[stereo(12.0), mic, stereo(-3.0), stereo(0.0)]).unwrap();
        assert_eq!(mixer.set_gain_db(0, -60.0), Ok(()));
        assert_eq!(mixer.set_gain_db(3, 12.0), Ok(()));
        let mut mixed = Vec::new();
        for _ in 0..2 {
            let mut game = vec![0.05; 2 * BLOCK_FRAMES];
            let mut voice = vec![0.05; BLOCK_FRAMES];
            let mut chat = vec![0.05; 2 * BLOCK_FRAMES];
            let mut other = vec![0.05; 2 * BLOCK_FRAMES];
            let mut mix = vec![0.0; 2 * BLOCK_FRAMES];
            mixer
                .process(
                    &mut [&mut game, &mut voice, &mut chat, &mut other],
                    &mut mix,
                )
                .unwrap();
            mixed.extend(mix);
        }
        // The second block, after the ramps: 0.05 × (0.001 + 0.001 + 0.708 + 3.981).
        let expected = 0.05 * (0.001 + 0.001 + 10_f32.powf(-3.0 / 20.0) + 10_f32.powf(0.6));
        let steady = &mixed[2 * (BLOCK_FRAMES + LOOKAHEAD_FRAMES)..];
        assert!(
            steady.iter().all(|&y| (y - expected).abs() < 1e-5),
            "{expected}"
        );
    }

    #[test]
    fn meters_read_stems_after_their_gain_and_the_mix_after_the_limiter() {
        let mut mixer = Mixer::new(&[stereo(-6.0206)]).unwrap();
        run_dc(&mut mixer, 0.5, 2, |_, _| {});
        let stem = mixer.take_stem_levels(0).unwrap();
        assert!((stem.peak_dbfs + 12.0412).abs() < 0.001, "{stem:?}");
        let (mix, reduction) = mixer.take_mix_levels();
        assert!((mix.peak_dbfs + 12.0412).abs() < 0.001, "{mix:?}");
        assert_eq!(reduction, 0.0);
        assert_eq!(mixer.take_stem_levels(1), None);
        // A loud stem: the limiter holds the mix at the ceiling, 7 dB down.
        let mut loud = Mixer::new(&[stereo(0.0)]).unwrap();
        run_dc(&mut loud, 2.0, 50, |_, _| {});
        let (mix, reduction) = loud.take_mix_levels();
        assert!(mix.peak_dbfs <= -1.0 + 1e-4, "{mix:?}");
        assert!((reduction - (-1.0 - 6.0206)).abs() < 0.001, "{reduction}");
    }

    #[test]
    fn stem_meters_read_after_the_gain_whatever_the_mix_and_the_clip() {
        // Out of the mix, 1.5 at −6 dB reads 0.75; in it, 0.75 at +6 dB reads 1.5, unclipped.
        for (gain_db, in_mix, level, expected) in
            [(-6.0206, false, 1.5, 0.75), (6.0206, true, 0.75, 1.5)]
        {
            let mut mixer = Mixer::new(&[StemConfig {
                in_mix,
                ..stereo(gain_db)
            }])
            .unwrap();
            run_dc(&mut mixer, level, 1, |_, _| {});
            let levels = mixer.take_stem_levels(0).unwrap();
            let peak = 20.0 * f64::log10(expected);
            assert!((levels.peak_dbfs - peak).abs() < 0.001, "{levels:?}");
        }
    }

    #[test]
    fn the_gain_reduction_reads_0_db_again_after_limiting() {
        let mut mixer = Mixer::new(&[stereo(0.0)]).unwrap();
        run_dc(&mut mixer, 2.0, 1, |_, _| {});
        run_dc(&mut mixer, 0.25, 300, |_, _| {});
        assert!(mixer.take_mix_levels().1 < -6.0);
        run_dc(&mut mixer, 0.25, 100, |_, _| {});
        let (mix, reduction) = mixer.take_mix_levels();
        assert_eq!(reduction, 0.0);
        assert!((mix.peak_dbfs + 12.0412).abs() < 0.001, "{mix:?}");
    }

    #[test]
    fn huge_or_broken_samples_leave_the_mix_finite_and_under_the_ceiling() {
        let ceiling = 10_f64.powf(-1.0 / 20.0) as f32;
        let cases: [&[(f32, f32)]; 4] = [
            &[(12.0, 1e38)],
            &[(12.0, 3e38), (12.0, -3e38)],
            &[(0.0, f32::MAX), (0.0, f32::MAX)],
            &[(12.0, f32::INFINITY), (12.0, f32::NAN)],
        ];
        for case in cases {
            let configs: Vec<StemConfig> = case.iter().map(|&(gain, _)| stereo(gain)).collect();
            let mut mixer = Mixer::new(&configs).unwrap();
            let mut mixed = Vec::new();
            for block in 0..2 {
                let mut stems: Vec<Vec<f32>> = case
                    .iter()
                    .map(|&(_, sample)| {
                        let mut stem = vec![0.25; 2 * BLOCK_FRAMES];
                        if block == 0 {
                            stem[0] = sample;
                        }
                        stem
                    })
                    .collect();
                let mut blocks: Vec<&mut [f32]> = stems.iter_mut().map(Vec::as_mut_slice).collect();
                let mut mix = vec![0.0; 2 * BLOCK_FRAMES];
                mixer.process(&mut blocks, &mut mix).unwrap();
                assert!(stems.iter().flatten().all(|y| y.abs() <= 1.0), "{case:?}");
                mixed.extend(mix);
            }
            let fine = |y: &f32| y.is_finite() && y.abs() <= ceiling;
            assert!(mixed.iter().all(fine), "{case:?}");
            // A huge sample reads as ±1,024 times the gain, one that isn't finite as silence.
            for (stem, &(gain_db, sample)) in case.iter().enumerate() {
                let level = if sample.is_finite() {
                    sample.abs().min(1_024.0)
                } else {
                    0.25
                };
                let peak = 20.0 * f64::from(level).log10() + f64::from(gain_db);
                let levels = mixer.take_stem_levels(stem).unwrap();
                assert!((levels.peak_dbfs - peak).abs() < 0.001, "{levels:?}");
            }
        }
    }

    #[test]
    fn a_skip_done_as_documented_keeps_every_track_in_place() {
        // A distinct, quiet value for every timeline sample, which the limiter passes unchanged.
        fn source(at: i64) -> f32 {
            0.000_8 * (at.rem_euclid(997) as f32 + 1.0)
        }
        /// Mixes the block at `pos`, from `source` or silent, and appends each track's left
        /// channel: T2 at `pos`, T1 at `pos − 240`. Both are gapless from timeline sample 0.
        fn block(mixer: &mut Mixer, pos: i64, live: bool, t1: &mut Vec<f32>, t2: &mut Vec<f32>) {
            let mut stem: Vec<f32> = (pos..pos + 480)
                .flat_map(|at| [if live { source(at) } else { 0.0 }; 2])
                .collect();
            let mut mix = vec![0.0; 2 * BLOCK_FRAMES];
            mixer.process(&mut [&mut stem], &mut mix).unwrap();
            for (track, at, stereo) in [(t2, pos, &stem), (t1, pos - 240, &mix)] {
                assert_eq!(usize::try_from(at).unwrap(), track.len(), "a gap");
                track.extend(stereo.as_chunks::<2>().0.iter().map(|frame| frame[0]));
            }
        }
        let mut mixer = Mixer::new(&[stereo(0.0)]).unwrap();
        let (mut t1, mut t2) = (Vec::new(), vec![0.0; 240]);
        let mut next_pos = 240;
        for _ in 0..50 {
            block(&mut mixer, next_pos, true, &mut t1, &mut t2);
            next_pos += 480;
        }
        // The mixer wakes 3 s late, and skips as blocks_due's doc says.
        let now = next_pos + MIX_DELAY_FRAMES + 300 * 480;
        let due = blocks_due(next_pos, now, MIX_DELAY_FRAMES);
        assert_eq!(
            due,
            Due {
                skip_frames: 200 * 480,
                blocks: 100
            }
        );
        let skipped = next_pos..next_pos + due.skip_frames;
        block(&mut mixer, next_pos, false, &mut t1, &mut t2);
        let silence = usize::try_from(due.skip_frames - 480).unwrap();
        t1.resize(t1.len() + silence, 0.0);
        t2.resize(t2.len() + silence, 0.0);
        next_pos += due.skip_frames;
        for _ in 0..due.blocks {
            block(&mut mixer, next_pos, true, &mut t1, &mut t2);
            next_pos += 480;
        }
        assert_eq!(t2.len(), t1.len() + 240);
        for (name, track) in [("T1", &t1), ("T2", &t2)] {
            for (at, &y) in (0_i64..).zip(track) {
                let expected = if at < 240 || skipped.contains(&at) {
                    0.0
                } else {
                    source(at)
                };
                assert_eq!(y, expected, "{name} at {at}");
            }
        }
    }

    #[test]
    fn debug_output_holds_no_audio() {
        let mut mixer = Mixer::new(&[stereo(0.0)]).unwrap();
        run_dc(&mut mixer, 0.123_456, 1, |_, _| {});
        let text = format!("{mixer:?}");
        assert!(!text.contains("0.12345"), "{text}");
        assert!(text.len() < 400, "{}", text.len());
    }

    #[test]
    fn refuses_bad_settings_and_blocks() {
        assert_eq!(Mixer::new(&[]).map(|_| ()), Err(MixError::StemCount(0)));
        assert_eq!(
            Mixer::new(&[stereo(0.0); 5]).map(|_| ()),
            Err(MixError::StemCount(5))
        );
        assert_eq!(
            Mixer::new(&[stereo(12.5)]).map(|_| ()),
            Err(MixError::BadGain(12.5))
        );
        let mut mixer = Mixer::new(&[stereo(0.0)]).unwrap();
        assert_eq!(mixer.set_gain_db(9, 0.0), Err(MixError::UnknownStem(9)));
        assert_eq!(mixer.set_in_mix(9, true), Err(MixError::UnknownStem(9)));
        assert!(matches!(
            mixer.set_gain_db(0, f32::NAN),
            Err(MixError::BadGain(_))
        ));
        assert_eq!(mixer.set_gain_db(0, -60.5), Err(MixError::BadGain(-60.5)));
        assert_eq!(mixer.set_gain_db(0, 12.0), Ok(()));
        let mut short = vec![0.0; 2 * BLOCK_FRAMES - 2];
        let mut fine = vec![0.0; 2 * BLOCK_FRAMES];
        let mut mix = vec![0.0; 2 * BLOCK_FRAMES];
        assert_eq!(
            mixer.process(&mut [&mut short], &mut mix),
            Err(MixError::BadBlock {
                stem: Some(0),
                len: 958
            })
        );
        assert_eq!(
            mixer.process(&mut [], &mut mix),
            Err(MixError::WrongStems {
                expected: 1,
                got: 0
            })
        );
        let mut small_mix = vec![0.0; 480];
        assert_eq!(
            mixer.process(&mut [&mut fine], &mut small_mix),
            Err(MixError::BadBlock {
                stem: None,
                len: 480
            })
        );
    }

    #[test]
    fn catching_up_runs_at_most_a_second() {
        assert_eq!(
            blocks_due(0, 9_600 + 3 * 480 + 100, MIX_DELAY_FRAMES),
            Due {
                skip_frames: 0,
                blocks: 3
            }
        );
        assert_eq!(blocks_due(0, 9_600 + 479, MIX_DELAY_FRAMES).blocks, 0);
        assert_eq!(blocks_due(0, 9_600 + 480, MIX_DELAY_FRAMES).blocks, 1);
        assert_eq!(blocks_due(0, 0, MIX_DELAY_FRAMES).blocks, 0);
        assert_eq!(
            blocks_due(480, 9_600 + 480 + 100 * 480, MIX_DELAY_FRAMES),
            Due {
                skip_frames: 0,
                blocks: 100
            }
        );
        // One block over the cap is skipped.
        assert_eq!(
            blocks_due(0, 9_600 + 101 * 480, MIX_DELAY_FRAMES),
            Due {
                skip_frames: 480,
                blocks: 100
            }
        );
        // A 5 s stall: skip 4 s, then mix the last second.
        assert_eq!(
            blocks_due(0, 9_600 + 500 * 480, MIX_DELAY_FRAMES),
            Due {
                skip_frames: 400 * 480,
                blocks: 100
            }
        );
        assert_eq!(blocks_due(i64::MIN, i64::MAX, 0).blocks, 100);
    }
}
