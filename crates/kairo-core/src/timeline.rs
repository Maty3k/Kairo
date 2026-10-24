// SPDX-License-Identifier: GPL-3.0-or-later
//! The media clock (docs/research/15-implementation-cookbook.md §5.1): QPC time, output ticks
//! and audio samples, all counted from one epoch.
//!
//! Times are 100 ns counts on the QPC timeline, the unit of WGC's `SystemRelativeTime` and of
//! WASAPI's timestamps. With epoch `E`, output tick `k` (video pts `k`) falls at
//! `E + k × 10⁷ / fps` and audio sample `n` (audio pts `n`) at `E + n × 10⁷ / 48,000`. Every
//! supported frame rate divides 48,000, so a tick is a whole number of samples (800 at 60 fps).
//! The engine reads QPC and passes times in. Products are computed in `i128` or saturate, so
//! no input can overflow them.

use std::fmt;

/// 100 ns units per second.
pub const HNS_PER_SECOND: i64 = 10_000_000;
/// The sample rate of every audio source, mix and track: 48 kHz.
pub const SAMPLE_RATE: i64 = 48_000;

/// An output frame rate. Each one divides 48,000, so a tick is a whole number of samples.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FrameRate {
    /// 30 fps: 1,600 samples per tick.
    Fps30,
    /// 60 fps, the default: 800 samples per tick.
    Fps60,
    /// 120 fps, for the Smooth preset if SP2 allows it: 400 samples per tick.
    Fps120,
}

impl FrameRate {
    /// Ticks per second.
    pub const fn hz(self) -> i64 {
        match self {
            Self::Fps30 => 30,
            Self::Fps60 => 60,
            Self::Fps120 => 120,
        }
    }

    /// Audio samples per tick.
    pub const fn samples_per_tick(self) -> i64 {
        SAMPLE_RATE / self.hz()
    }
}

impl TryFrom<u32> for FrameRate {
    type Error = TimelineError;

    fn try_from(hz: u32) -> Result<Self, Self::Error> {
        match hz {
            30 => Ok(Self::Fps30),
            60 => Ok(Self::Fps60),
            120 => Ok(Self::Fps120),
            other => Err(TimelineError::UnsupportedFrameRate(other)),
        }
    }
}

/// Why a time could not be converted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimelineError {
    /// Only 30, 60 and 120 fps are supported.
    UnsupportedFrameRate(u32),
    /// A QPC frequency must be positive.
    BadFrequency(i64),
    /// The result does not fit in an `i64` count of 100 ns.
    OutOfRange,
}

impl fmt::Display for TimelineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedFrameRate(hz) => {
                write!(f, "the frame rate must be 30, 60 or 120 fps, not {hz}")
            }
            Self::BadFrequency(frequency) => {
                write!(f, "the QPC frequency must be positive, not {frequency}")
            }
            Self::OutOfRange => f.write_str("the time does not fit in 64 bits of 100 ns"),
        }
    }
}

impl std::error::Error for TimelineError {}

/// A QPC reading in 100 ns: `⌊counter × 10⁷ / frequency⌋`, where `frequency` comes from
/// `QueryPerformanceFrequency`.
pub fn hns_from_qpc(counter: i64, frequency: i64) -> Result<i64, TimelineError> {
    if frequency <= 0 {
        return Err(TimelineError::BadFrequency(frequency));
    }
    let hns = (i128::from(counter) * i128::from(HNS_PER_SECOND)).div_euclid(i128::from(frequency));
    i64::try_from(hns).map_err(|_| TimelineError::OutOfRange)
}

/// The audio sample playing at `t_hns`: `⌊(t − E) × 48,000 / 10⁷⌋`, negative before the epoch.
pub fn sample_at(t_hns: i64, epoch_hns: i64) -> i64 {
    // |t − E| < 2⁶⁴, so the result stays below 2⁵⁷ and always fits.
    saturate(
        (since(t_hns, epoch_hns) * i128::from(SAMPLE_RATE)).div_euclid(i128::from(HNS_PER_SECOND)),
    )
}

/// The tick nearest to `t_hns`, with halves rounding up. This turns a hotkey's QPC time into
/// the hotkey's tick `t` (cookbook §5.3, step 1); the clip ends at `t` plus the post-roll.
pub fn nearest_tick(t_hns: i64, epoch_hns: i64, rate: FrameRate) -> i64 {
    let hns = i128::from(HNS_PER_SECOND);
    saturate((2 * since(t_hns, epoch_hns) * i128::from(rate.hz()) + hns).div_euclid(2 * hns))
}

/// The first tick at or after `t_hns`: `⌈(t − E) × fps / 10⁷⌉`.
pub fn tick_at_or_after(t_hns: i64, epoch_hns: i64, rate: FrameRate) -> i64 {
    let scaled = since(t_hns, epoch_hns) * i128::from(rate.hz());
    saturate(-((-scaled).div_euclid(i128::from(HNS_PER_SECOND))))
}

/// The first audio sample of tick `tick`.
pub fn tick_to_sample(tick: i64, rate: FrameRate) -> i64 {
    tick.saturating_mul(rate.samples_per_tick())
}

/// True if `ts_hns` is at or before tick `tick`'s exact time, `E + tick × 10⁷ / fps`. At every
/// supported rate that time is a whole count of 100 ns only when `tick` is a multiple of 3
/// (10⁷ has no factor of 3); otherwise it falls between two. So this compares exactly rather
/// than against [`deadline_hns`].
pub fn at_or_before(ts_hns: i64, tick: i64, epoch_hns: i64, rate: FrameRate) -> bool {
    since(ts_hns, epoch_hns) * i128::from(rate.hz())
        <= i128::from(tick) * i128::from(HNS_PER_SECOND)
}

/// When tick `tick` is due: the first whole 100 ns at or after its exact time,
/// `E + ⌈tick × 10⁷ / fps⌉`. Each deadline comes from its tick number, never from adding up a
/// rounded period, so deadlines never drift.
pub fn deadline_hns(tick: i64, epoch_hns: i64, rate: FrameRate) -> i64 {
    let exact = i128::from(tick) * i128::from(HNS_PER_SECOND);
    let ceil = -((-exact).div_euclid(i128::from(rate.hz())));
    saturate(i128::from(epoch_hns) + ceil)
}

/// `t − E`, which can't overflow in `i128`.
fn since(t_hns: i64, epoch_hns: i64) -> i128 {
    i128::from(t_hns) - i128::from(epoch_hns)
}

/// `value` clamped into `i64`.
fn saturate(value: i128) -> i64 {
    i64::try_from(value).unwrap_or(if value < 0 { i64::MIN } else { i64::MAX })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Expected values come from an independent Python model with exact fractions.

    #[test]
    fn frame_rates() {
        assert_eq!(FrameRate::try_from(30), Ok(FrameRate::Fps30));
        assert_eq!(FrameRate::try_from(60), Ok(FrameRate::Fps60));
        assert_eq!(FrameRate::try_from(120), Ok(FrameRate::Fps120));
        assert_eq!(
            FrameRate::try_from(25),
            Err(TimelineError::UnsupportedFrameRate(25))
        );
        assert_eq!(
            FrameRate::try_from(144),
            Err(TimelineError::UnsupportedFrameRate(144))
        );
        let samples: Vec<i64> = [FrameRate::Fps30, FrameRate::Fps60, FrameRate::Fps120]
            .iter()
            .map(|rate| rate.samples_per_tick())
            .collect();
        assert_eq!(samples, [1_600, 800, 400]);
    }

    #[test]
    fn qpc_to_hns() {
        assert_eq!(hns_from_qpc(1, 3), Ok(3_333_333));
        assert_eq!(hns_from_qpc(3, 3), Ok(HNS_PER_SECOND));
        // The usual 10 MHz QPC frequency is already 100 ns.
        assert_eq!(hns_from_qpc(123_456_789, 10_000_000), Ok(123_456_789));
        assert_eq!(hns_from_qpc(0, 0), Err(TimelineError::BadFrequency(0)));
        assert_eq!(hns_from_qpc(5, -1), Err(TimelineError::BadFrequency(-1)));
        assert_eq!(hns_from_qpc(i64::MAX, 1), Err(TimelineError::OutOfRange));
        assert_eq!(hns_from_qpc(i64::MAX, i64::MAX), Ok(HNS_PER_SECOND));
    }

    #[test]
    fn samples_floor_toward_minus_infinity() {
        let epoch = 1_000_000;
        let cases = [
            (epoch, 0),
            (epoch + 624, 2),
            (epoch + 625, 3),
            (epoch - 1, -1),
            (epoch + HNS_PER_SECOND, 48_000),
            (epoch - HNS_PER_SECOND, -48_000),
        ];
        for (t, expected) in cases {
            assert_eq!(sample_at(t, epoch), expected, "t = {t}");
        }
        // The widest possible span still fits.
        assert_eq!(sample_at(i64::MAX, i64::MIN), 88_544_371_553_805_847);
        assert_eq!(sample_at(i64::MIN, i64::MAX), -88_544_371_553_805_848);
    }

    #[test]
    fn nearest_tick_rounds_halves_up() {
        let cases = [
            (FrameRate::Fps60, HNS_PER_SECOND, 60),
            (FrameRate::Fps60, 83_334, 1),
            (FrameRate::Fps60, 83_333, 0),
            (FrameRate::Fps60, -83_334, -1),
            (FrameRate::Fps60, -83_333, 0),
            // 1.5 ticks exactly at 60 fps.
            (FrameRate::Fps60, 250_000, 2),
            (FrameRate::Fps60, -250_000, -1),
            (FrameRate::Fps30, 166_667, 1),
            (FrameRate::Fps30, 166_666, 0),
            (FrameRate::Fps120, 41_667, 1),
            (FrameRate::Fps120, 41_666, 0),
        ];
        for (rate, since_epoch, expected) in cases {
            let epoch = -7;
            assert_eq!(
                nearest_tick(epoch + since_epoch, epoch, rate),
                expected,
                "{rate:?} at E + {since_epoch}"
            );
        }
    }

    #[test]
    fn deadlines_round_up_to_whole_hns() {
        let cases = [
            (0, 0),
            (1, 166_667),
            (2, 333_334),
            (3, 500_000),
            (60, 10_000_000),
            (1_000_000, 166_666_666_667),
            (-1, -166_666),
        ];
        for (tick, expected) in cases {
            assert_eq!(
                deadline_hns(tick, 0, FrameRate::Fps60),
                expected,
                "tick {tick}"
            );
            assert_eq!(
                deadline_hns(tick, 42, FrameRate::Fps60),
                expected + 42,
                "tick {tick} with E = 42"
            );
        }
        assert_eq!(deadline_hns(i64::MAX, 0, FrameRate::Fps30), i64::MAX);
        assert_eq!(deadline_hns(i64::MIN, 0, FrameRate::Fps30), i64::MIN);
    }

    #[test]
    fn at_or_before_uses_the_exact_tick_time() {
        assert!(at_or_before(166_666, 1, 0, FrameRate::Fps60));
        assert!(!at_or_before(166_667, 1, 0, FrameRate::Fps60));
        // Tick 3 at 60 fps falls exactly on 500,000.
        assert!(at_or_before(500_000, 3, 0, FrameRate::Fps60));
        assert!(!at_or_before(500_001, 3, 0, FrameRate::Fps60));
        assert!(at_or_before(-1, 0, 0, FrameRate::Fps60));
        // 30 fps tick times are fractional too: tick 1 falls at 333,333.33.
        assert!(at_or_before(333_333, 1, 0, FrameRate::Fps30));
        assert!(!at_or_before(333_334, 1, 0, FrameRate::Fps30));
        assert!(at_or_before(1_000_000, 3, 0, FrameRate::Fps30));
        // Extremes neither overflow nor compare wrongly.
        assert!(at_or_before(i64::MIN, 0, i64::MAX, FrameRate::Fps120));
        assert!(!at_or_before(i64::MAX, 0, i64::MIN, FrameRate::Fps120));
        assert!(at_or_before(i64::MAX, i64::MAX, i64::MIN, FrameRate::Fps30));
    }

    #[test]
    fn first_tick_at_or_after() {
        let cases = [
            (0, 0),
            (1, 1),
            (166_666, 1),
            (166_667, 2),
            (166_668, 2),
            (-1, 0),
            (-166_667, -1),
            (-166_666, 0),
        ];
        for (t, expected) in cases {
            assert_eq!(
                tick_at_or_after(t, 0, FrameRate::Fps60),
                expected,
                "t = {t}"
            );
        }
    }

    #[test]
    fn ticks_to_samples() {
        assert_eq!(tick_to_sample(5_340, FrameRate::Fps60), 4_272_000);
        assert_eq!(tick_to_sample(-1, FrameRate::Fps120), -400);
        assert_eq!(tick_to_sample(i64::MAX, FrameRate::Fps30), i64::MAX);
    }
}
