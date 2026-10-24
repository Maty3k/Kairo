// SPDX-License-Identifier: GPL-3.0-or-later
//! Level meters (docs/research/10-audio-rust.md §5.7): the peak and RMS of a stream in dBFS,
//! read and restarted about 20 times a second for the UI's level meters (research 10 §5.7's
//! `audio.levels`, with the limiter's gain reduction).

/// The level the meters show for silence: −180 dBFS.
pub const SILENCE_DBFS: f64 = -180.0;

/// A stream's levels over the samples since the last [`Meter::take`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Levels {
    /// The largest absolute sample, in dBFS.
    pub peak_dbfs: f64,
    /// The root mean square of every sample, in dBFS.
    pub rms_dbfs: f64,
}

/// A meter that adds up samples until it is read.
#[derive(Debug, Clone, Copy, Default)]
pub struct Meter {
    peak: f32,
    sum_of_squares: f64,
    samples: u64,
}

impl Meter {
    /// A meter with nothing counted.
    pub fn new() -> Self {
        Self::default()
    }

    /// Counts `samples`, every channel alike. Samples that aren't finite count as silence.
    pub fn push(&mut self, samples: &[f32]) {
        for &sample in samples {
            let sample = if sample.is_finite() { sample } else { 0.0 };
            self.peak = self.peak.max(sample.abs());
            self.sum_of_squares += f64::from(sample) * f64::from(sample);
        }
        self.samples = self
            .samples
            .saturating_add(u64::try_from(samples.len()).unwrap_or(u64::MAX));
    }

    /// The levels since the last call, then starts over. With nothing counted, both are
    /// [`SILENCE_DBFS`].
    pub fn take(&mut self) -> Levels {
        let levels = if self.samples == 0 {
            Levels {
                peak_dbfs: SILENCE_DBFS,
                rms_dbfs: SILENCE_DBFS,
            }
        } else {
            Levels {
                peak_dbfs: 20.0 * f64::from(self.peak).max(1e-9).log10(),
                rms_dbfs: 10.0
                    * (self.sum_of_squares / self.samples as f64)
                        .max(1e-18)
                        .log10(),
            }
        };
        *self = Self::default();
        levels
    }
}

/// A linear gain in dB, with −180 dB for a gain of 0.
pub fn gain_db(gain: f64) -> f64 {
    20.0 * gain.max(1e-9).log10()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn levels(samples: &[f32]) -> (f64, f64) {
        let mut meter = Meter::new();
        meter.push(samples);
        let levels = meter.take();
        (levels.peak_dbfs, levels.rms_dbfs)
    }

    fn sine(amplitude: f64) -> Vec<f32> {
        (0..2_400)
            .map(|i| {
                (amplitude * (std::f64::consts::TAU * 1_000.0 * f64::from(i) / 48_000.0).sin())
                    as f32
            })
            .collect()
    }

    fn close(actual: (f64, f64), expected: (f64, f64)) -> bool {
        (actual.0 - expected.0).abs() < 0.001 && (actual.1 - expected.1).abs() < 0.001
    }

    #[test]
    fn known_signals_read_their_levels() {
        // Research 10 §8.4.
        let square: Vec<f32> = (0..2_400)
            .map(|i| if i % 2 == 0 { 1.0 } else { -1.0 })
            .collect();
        let cases = [
            (square, (0.0, 0.0)),
            (sine(1.0), (0.0, -3.0103)),
            (sine(0.031_623), (-30.0, -33.0103)),
            (vec![0.5; 2_400], (-6.0206, -6.0206)),
            (vec![0.0; 2_400], (-180.0, -180.0)),
        ];
        for (samples, expected) in cases {
            let actual = levels(&samples);
            assert!(close(actual, expected), "{actual:?} vs {expected:?}");
        }
    }

    #[test]
    fn taking_starts_over() {
        let mut meter = Meter::new();
        assert_eq!(
            meter.take(),
            Levels {
                peak_dbfs: SILENCE_DBFS,
                rms_dbfs: SILENCE_DBFS
            }
        );
        meter.push(&[0.5, -0.5]);
        meter.push(&[f32::NAN, f32::INFINITY]);
        let first = meter.take();
        assert!((first.peak_dbfs + 6.0206).abs() < 0.001);
        // Half the samples are silence, so the RMS is 3 dB under the peak.
        assert!((first.rms_dbfs + 9.0309).abs() < 0.001);
        assert_eq!(meter.take().peak_dbfs, SILENCE_DBFS);
    }

    #[test]
    fn gains_in_db() {
        assert!((gain_db(0.5) + 6.0206).abs() < 0.001);
        assert_eq!(gain_db(1.0), 0.0);
        assert_eq!(gain_db(0.0), -180.0);
    }
}
