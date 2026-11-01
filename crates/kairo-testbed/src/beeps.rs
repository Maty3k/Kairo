// SPDX-License-Identifier: GPL-3.0-or-later
//! Beeps on the QPC timeline (docs/research/16-verify-testbed/README.md §4.7, §5): the
//! testbed's 1 kHz "game" beep, aimed at a flash frame's display, and `kairo-tone.exe`'s 2 kHz
//! "voice" beep at every whole QPC second + 500 ms.
//!
//! The render stream is 48 kHz stereo f32. Its frame f plays when the audio clock's position
//! reaches f; `IAudioClock::GetPosition` pairs a position with the QPC time it was read at, so
//! [`ClockPoint`] maps stream frames to QPC time and back. A beep is aimed at a QPC time, which
//! lands between two samples: each sample is the burst's value at its own time after the onset
//! (`testsig::tone_burst_at`), so the onset is exact to well under a sample. A beep is written
//! only if its onset is still ahead of what has been written; once the clock has passed it, it
//! is retired with the time its first sample played, mapped through the latest clock reading.
//! Everything here is pure; `audio.rs` feeds it the device's clock and buffers.

use std::collections::VecDeque;

use kairo_core::testsig::{self, BEEP_MS, BEEP_PEAK_DBFS};
use kairo_core::timeline::HNS_PER_SECOND;

/// The stream's rate.
pub const SAMPLE_RATE: u32 = 48_000;
/// The stream's channels.
pub const CHANNELS: usize = 2;
/// How far ahead a beep is handed to the audio thread: at least 200 ms (README §4.7).
pub const LEAD_HNS: i64 = 4 * HNS_PER_SECOND / 10;

/// One reading of the audio clock: stream frame `frame` (fractional) played at QPC `t_hns`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClockPoint {
    /// The stream position, in frames.
    pub frame: f64,
    /// When, in QPC 100 ns.
    pub t_hns: i64,
}

impl ClockPoint {
    /// `IAudioClock::GetPosition`'s `(position, qpc)` with `GetFrequency`'s `frequency`: the
    /// position is in units of `frequency` per second and the QPC time is already in 100 ns.
    pub fn from_position(position: u64, frequency: u64, qpc_hns: u64) -> Option<ClockPoint> {
        if frequency == 0 {
            return None;
        }
        let frame = position as f64 * f64::from(SAMPLE_RATE) / frequency as f64;
        Some(ClockPoint {
            frame,
            t_hns: i64::try_from(qpc_hns).ok()?,
        })
    }

    /// When stream frame `frame` plays.
    pub fn time_of(&self, frame: f64) -> f64 {
        self.t_hns as f64 + (frame - self.frame) * HNS_PER_SECOND as f64 / f64::from(SAMPLE_RATE)
    }

    /// The stream frame that plays at `t_hns`.
    pub fn frame_at(&self, t_hns: i64) -> f64 {
        self.frame + (t_hns - self.t_hns) as f64 * f64::from(SAMPLE_RATE) / HNS_PER_SECOND as f64
    }
}

/// The gain that takes a −30 dBFS burst to `dbfs`.
pub fn gain(dbfs: f64) -> f32 {
    10_f64.powf((dbfs - BEEP_PEAK_DBFS) / 20.0) as f32
}

/// A beep handed to the stream.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Beep {
    /// Its number.
    pub index: u64,
    /// What it is aimed at (the flash frame N_f for the game beep; unused for voice).
    pub frame: u64,
    /// When its first sample should play.
    pub target_hns: i64,
}

/// A beep whose fate is known.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Played {
    /// The beep.
    pub beep: Beep,
    /// When its first sample played, if it did.
    pub played_hns: Option<i64>,
}

#[derive(Debug, Clone, Copy)]
struct Scheduled {
    beep: Beep,
    /// The stream frame of its onset, fractional.
    onset: f64,
}

/// Mixes beeps of one frequency and level into the stream.
#[derive(Debug, Clone)]
pub struct Beeper {
    hz: f64,
    gain: f32,
    /// The beep's length in frames, plus one for a fractional onset.
    span: u64,
    /// The next stream frame to write.
    written: u64,
    queue: VecDeque<Scheduled>,
    /// Beeps that came too late, reported at the next retire.
    late: Vec<Beep>,
}

impl Beeper {
    /// Beeps of `hz` at `dbfs` peak.
    pub fn new(hz: u32, dbfs: f64) -> Beeper {
        Beeper {
            hz: f64::from(hz),
            gain: gain(dbfs),
            span: u64::from(SAMPLE_RATE) * u64::from(BEEP_MS) / 1_000 + 1,
            written: 0,
            queue: VecDeque::new(),
            late: Vec::new(),
        }
    }

    /// The next stream frame to write.
    pub fn written(&self) -> u64 {
        self.written
    }

    /// Aims `beep` with the clock reading `clock`. A beep whose onset is no longer ahead of the
    /// stream is reported unplayed at the next [`Beeper::retire`]. Beeps must come in order.
    pub fn schedule(&mut self, beep: Beep, clock: &ClockPoint) {
        let onset = clock.frame_at(beep.target_hns);
        let after_last = self.queue.back().is_none_or(|last| onset > last.onset);
        if onset >= self.written as f64 && after_last {
            self.queue.push_back(Scheduled { beep, onset });
        } else {
            self.late.push(beep);
        }
    }

    /// Fills `out` (interleaved, [`CHANNELS`] per frame) with the next frames of the stream.
    pub fn fill(&mut self, out: &mut [f32]) {
        let frames = (out.len() / CHANNELS) as u64;
        out.fill(0.0);
        let rate = f64::from(SAMPLE_RATE);
        for s in &self.queue {
            let first = s.onset.ceil().max(self.written as f64) as u64;
            let last = (s.onset.floor() as u64 + self.span).min(self.written + frames);
            for f in first..last {
                let t = (f as f64 - s.onset) / rate;
                let value = (testsig::tone_burst_at(self.hz, BEEP_MS, t) as f32) * self.gain;
                let at = (f - self.written) as usize * CHANNELS;
                for c in &mut out[at..at + CHANNELS] {
                    *c += value;
                }
            }
        }
        self.written += frames;
    }

    /// The beeps settled by the clock reading `clock`, in order: those whose onset has played
    /// (with the time it played) and those that came too late (unplayed). A beep whose onset is
    /// written but not yet played waits.
    pub fn retire(&mut self, clock: &ClockPoint) -> Vec<Played> {
        let mut out: Vec<Played> = self
            .late
            .drain(..)
            .map(|beep| Played {
                beep,
                played_hns: None,
            })
            .collect();
        while let Some(s) = self.queue.front() {
            if s.onset > clock.frame || s.onset.ceil() as u64 >= self.written {
                break;
            }
            out.push(Played {
                beep: s.beep,
                played_hns: Some(clock.time_of(s.onset).round() as i64),
            });
            self.queue.pop_front();
        }
        out.sort_by_key(|p| p.beep.index);
        out
    }

    /// The beeps still waiting at the end: unplayed.
    pub fn finish(&mut self) -> Vec<Played> {
        let mut out: Vec<Played> = self
            .late
            .drain(..)
            .chain(self.queue.drain(..).map(|s| s.beep))
            .map(|beep| Played {
                beep,
                played_hns: None,
            })
            .collect();
        out.sort_by_key(|p| p.beep.index);
        out
    }
}

/// Hands back settled beeps in their numbers' order, holding a beep until every beep sent before
/// it has settled: the logs need their `b` and `v` lines in order.
#[derive(Debug, Clone, Default)]
pub struct InOrder {
    sent: std::collections::BTreeMap<u64, Beep>,
    settled: std::collections::BTreeMap<u64, Played>,
}

impl InOrder {
    /// `beep` was handed to the stream.
    pub fn sent(&mut self, beep: Beep) {
        self.sent.insert(beep.index, beep);
    }

    /// `played` settled; returns every beep now due, in order.
    pub fn settle(&mut self, played: Played) -> Vec<Played> {
        self.settled.insert(played.beep.index, played);
        let mut out = Vec::new();
        while let Some(&first) = self.sent.keys().next() {
            let Some(p) = self.settled.remove(&first) else {
                break;
            };
            self.sent.remove(&first);
            out.push(p);
        }
        out
    }

    /// Every beep left at the end, in order: settled ones as they settled, and ones the stream
    /// never reported (it stopped) as unplayed.
    pub fn rest(&mut self) -> Vec<Played> {
        let mut out = std::mem::take(&mut self.settled);
        for (index, beep) in std::mem::take(&mut self.sent) {
            out.entry(index).or_insert(Played {
                beep,
                played_hns: None,
            });
        }
        out.into_values().collect()
    }
}

/// The voice beeps' times (README §5): beep k at whole QPC second `first_second + k`, plus
/// `offset_ms`.
pub fn voice_target(first_second: i64, k: u64, offset_ms: u32) -> i64 {
    (first_second + k as i64) * HNS_PER_SECOND + i64::from(offset_ms) * 10_000
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clock(frame: f64, t_hns: i64) -> ClockPoint {
        ClockPoint { frame, t_hns }
    }

    #[test]
    fn the_clock_maps_frames_and_times() {
        // A device clock counting bytes of 48 kHz stereo f32: 384,000 a second.
        let c = ClockPoint::from_position(384_000, 384_000, 5_000_000).unwrap();
        assert_eq!(c.frame, 48_000.0);
        assert_eq!(c.time_of(48_000.0 + 480.0), 5_100_000.0);
        assert_eq!(c.frame_at(4_900_000), 47_520.0);
        assert!(ClockPoint::from_position(1, 0, 1).is_none());
        assert!((gain(-30.0) - 1.0).abs() < 1e-7);
        assert!((gain(-36.0) - 0.501_187).abs() < 1e-6);
        assert_eq!(voice_target(100, 2, 500), 1_025_000_000);
    }

    #[test]
    fn a_beep_lands_between_samples_on_its_target() {
        let mut b = Beeper::new(1_000, -30.0);
        // Frame 0 plays at QPC 1 s; aim at 1.2 s + a third of a sample.
        let c = clock(0.0, HNS_PER_SECOND);
        let target = HNS_PER_SECOND + 2_000_000 + 69;
        b.schedule(
            Beep {
                index: 3,
                frame: 540,
                target_hns: target,
            },
            &c,
        );
        let mut out = vec![0.0_f32; 48_000 * CHANNELS];
        b.fill(&mut out);
        let onset = c.frame_at(target);
        assert!((onset - 9_600.331).abs() < 0.01, "{onset}");
        // Silence before the onset, and each sample is the burst at its own time after it.
        assert!(out[..9_600 * CHANNELS].iter().all(|&s| s == 0.0));
        for f in [9_601_usize, 9_700, 12_000, 14_400] {
            let t = (f as f64 - onset) / 48_000.0;
            let want = testsig::tone_burst_at(1_000.0, BEEP_MS, t) as f32;
            assert_eq!(out[f * CHANNELS], want, "frame {f}");
            assert_eq!(out[f * CHANNELS + 1], want, "frame {f}");
        }
        assert!(out[14_402 * CHANNELS..].iter().all(|&s| s == 0.0));
        // Not played until the clock passes the onset; then it played on target.
        assert!(
            b.retire(&clock(9_000.0, c.time_of(9_000.0) as i64))
                .is_empty()
        );
        let later = clock(20_000.0, c.time_of(20_000.0).round() as i64);
        let played = b.retire(&later);
        assert_eq!(played.len(), 1);
        assert_eq!(played[0].beep.index, 3);
        assert!((played[0].played_hns.unwrap() - target).abs() <= 1);
        assert!(b.finish().is_empty());
    }

    #[test]
    fn a_beep_split_across_buffers_is_whole() {
        let c = clock(0.0, 0);
        let target = c.time_of(1_000.5) as i64;
        let beep = Beep {
            index: 1,
            frame: 180,
            target_hns: target,
        };
        let mut whole = Beeper::new(2_000, -30.0);
        whole.schedule(beep, &c);
        let mut one = vec![0.0_f32; 8_000 * CHANNELS];
        whole.fill(&mut one);
        let mut split = Beeper::new(2_000, -30.0);
        split.schedule(beep, &c);
        let mut parts = Vec::new();
        for n in [999, 1, 2_500, 4_500] {
            let mut buffer = vec![0.0_f32; n * CHANNELS];
            split.fill(&mut buffer);
            parts.extend(buffer);
        }
        assert_eq!(parts, one);
        assert_eq!(split.written(), 8_000);
    }

    #[test]
    fn settled_beeps_come_out_in_order() {
        let played = |index| Played {
            beep: Beep {
                index,
                frame: 0,
                target_hns: 0,
            },
            played_hns: None,
        };
        let mut order = InOrder::default();
        for i in 1..=4 {
            order.sent(played(i).beep);
        }
        assert!(order.settle(played(2)).is_empty());
        let due: Vec<u64> = order
            .settle(played(1))
            .iter()
            .map(|p| p.beep.index)
            .collect();
        assert_eq!(due, [1, 2]);
        assert!(order.settle(played(4)).is_empty());
        // Beep 3 never settled (the stream stopped): it comes out unplayed, in its place.
        let rest: Vec<u64> = order.rest().iter().map(|p| p.beep.index).collect();
        assert_eq!(rest, [3, 4]);
    }

    #[test]
    fn late_beeps_are_reported_unplayed_in_order() {
        let mut b = Beeper::new(1_000, -30.0);
        let c = clock(0.0, 0);
        let mut out = vec![0.0_f32; 4_800 * CHANNELS];
        b.fill(&mut out);
        // 50 ms is already written: a beep aimed there comes too late; one at 200 ms doesn't.
        let at = |ms: i64| ms * 10_000;
        b.schedule(
            Beep {
                index: 1,
                frame: 0,
                target_hns: at(50),
            },
            &c,
        );
        b.schedule(
            Beep {
                index: 2,
                frame: 0,
                target_hns: at(200),
            },
            &c,
        );
        let mut rest = vec![0.0_f32; 48_000 * CHANNELS];
        b.fill(&mut rest);
        let settled = b.retire(&clock(40_000.0, c.time_of(40_000.0) as i64));
        let indices: Vec<u64> = settled.iter().map(|p| p.beep.index).collect();
        assert_eq!(indices, [1, 2]);
        assert_eq!(settled[0].played_hns, None);
        assert_eq!(settled[1].played_hns, Some(at(200)));
        // One still waiting at the end is unplayed.
        b.schedule(
            Beep {
                index: 3,
                frame: 0,
                target_hns: at(5_000),
            },
            &c,
        );
        assert_eq!(b.finish()[0].played_hns, None);
    }
}
