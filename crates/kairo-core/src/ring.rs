// SPDX-License-Identifier: GPL-3.0-or-later
//! The replay ring (docs/PLAN.md §2.2, Ring; docs/research/15-implementation-cookbook.md §5.1
//! and §5.2): the encoded video and audio of the last few minutes, indexed by keyframe, evicted
//! a whole GOP at a time, with pins that keep a save's packets until it has copied them.
//!
//! Video pts is the pacer tick (time base 1/fps) and audio pts the 48 kHz sample index, both on
//! [`crate::timeline`]'s clock. Ticks keep counting across encoder epochs, so video pts rises
//! strictly through the whole ring. Each audio track is gapless: its encoder gets silence for
//! any gap (research 10 §5.8), so every packet starts where the previous one ended.
//!
//! The ring is generic over the payload: the tests store `()`. [`Ring::snapshot`] clones each
//! payload of a clip while the caller holds the ring's lock, so the payload's `Clone` must only
//! add a reference: the engine stores `Arc<ffmpeg_next::Packet>`, never `ffmpeg_next::Packet`,
//! whose `clone` copies the data (research 09 §1.3).

use std::collections::{BTreeMap, VecDeque};
use std::fmt;

use crate::timeline::FrameRate;

/// AAC frame length in samples: every audio packet starts on this grid (research 10 §5.8).
pub const AAC_FRAME: i64 = 1_024;
/// The longest a ring may keep: 10 minutes. The longest default binding is 2 minutes.
pub const MAX_KEEP_SECONDS: i64 = 600;
/// The largest video pts the ring accepts, so that ticks × samples per tick can't overflow.
pub const MAX_VIDEO_PTS: i64 = 1 << 48;
/// The largest audio pts the ring accepts, so that pts plus duration can't overflow.
pub const MAX_AUDIO_PTS: i64 = 1 << 59;
/// The most audio tracks a ring takes. PLAN §2.3 has five tracks, T1 to T5.
pub const MAX_AUDIO_TRACKS: usize = 8;

/// One encoded packet, as the encoder produced it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pkt<P> {
    /// Presentation time: a tick for video, a sample index for audio.
    pub pts: i64,
    /// Decode time, equal to `pts` (no B-frames: research 09's summary).
    pub dts: i64,
    /// Duration in the same unit: 1 tick for video, 1,024 samples for AAC.
    pub dur: i64,
    /// A keyframe (an IDR for video; every AAC packet is one).
    pub key: bool,
    /// Memory the packet holds, as the engine counts it (size plus padding, research 09 §5.1).
    pub bytes: usize,
    /// The packet itself. A snapshot clones it under the ring's lock, so cloning must only add
    /// a reference.
    pub payload: P,
}

/// A video packet and the encoder epoch that made it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VideoPkt<P> {
    pub(crate) epoch: u32,
    pub(crate) pkt: Pkt<P>,
}

/// The ring.
#[derive(Debug, Clone)]
pub struct Ring<P> {
    rate: FrameRate,
    keep_ticks: i64,
    cap_bytes: usize,
    video: VecDeque<VideoPkt<P>>,
    /// Pts and epoch of every retained video keyframe, oldest first.
    keys: VecDeque<(i64, u32)>,
    audio: Vec<VecDeque<Pkt<P>>>,
    bytes: usize,
    /// Save id → the oldest video pts that save still needs.
    pins: BTreeMap<u64, i64>,
}

/// Why the ring refused a setting or a packet. A refused packet leaves the ring unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RingError {
    /// `keep_ticks` must be 1 tick to 10 minutes of ticks.
    BadKeep(i64),
    /// More than [`MAX_AUDIO_TRACKS`] audio tracks.
    TooManyTracks(usize),
    /// No such audio track.
    UnknownTrack(usize),
    /// A packet's pts is not above the previous packet's on the same stream.
    NotIncreasing {
        /// The previous packet's pts.
        last: i64,
        /// The refused packet's pts.
        got: i64,
    },
    /// pts and dts differ (for video, the encoder made B-frames).
    PtsNotDts {
        /// The packet's pts.
        pts: i64,
        /// The packet's dts.
        dts: i64,
    },
    /// The encoder epoch went back.
    EpochWentBack {
        /// The previous video packet's epoch.
        last: u32,
        /// The refused packet's epoch.
        got: u32,
    },
    /// An epoch's first video packet is not a keyframe; every encoder session opens with an IDR.
    EpochWithoutKeyframe(u32),
    /// An audio packet's duration is not 1 to 2⁵⁹ samples.
    BadDuration(i64),
    /// An audio packet doesn't start where the previous one on its track ended.
    AudioGap {
        /// Where the previous packet ended.
        expected: i64,
        /// The refused packet's pts.
        got: i64,
    },
    /// A pts beyond ±2⁴⁸ ticks for video or ±2⁵⁹ samples for audio.
    OutOfRange {
        /// The refused packet's pts.
        pts: i64,
        /// The limit for its stream.
        max: i64,
    },
}

impl fmt::Display for RingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadKeep(ticks) => {
                write!(
                    f,
                    "keep must be 1 tick to {MAX_KEEP_SECONDS} s of ticks, not {ticks}"
                )
            }
            Self::TooManyTracks(tracks) => write!(
                f,
                "a ring takes at most {MAX_AUDIO_TRACKS} audio tracks, not {tracks}"
            ),
            Self::UnknownTrack(track) => write!(f, "there is no audio track {track}"),
            Self::NotIncreasing { last, got } => {
                write!(f, "pts {got} is not above the previous packet's {last}")
            }
            Self::PtsNotDts { pts, dts } => write!(f, "pts {pts} and dts {dts} differ"),
            Self::EpochWentBack { last, got } => write!(f, "epoch {got} follows epoch {last}"),
            Self::EpochWithoutKeyframe(epoch) => {
                write!(f, "epoch {epoch} must start with a keyframe")
            }
            Self::BadDuration(dur) => write!(
                f,
                "an audio packet's duration must be 1 to {MAX_AUDIO_PTS} samples, not {dur}"
            ),
            Self::AudioGap { expected, got } => write!(
                f,
                "an audio packet must start at {expected}, where the previous one ended, not {got}"
            ),
            Self::OutOfRange { pts, max } => write!(f, "pts {pts} is outside ±{max}"),
        }
    }
}

impl std::error::Error for RingError {}

/// Ticks to keep: the longest binding plus the post-roll plus one 1-second GOP (cookbook §5.1).
pub fn keep_ticks(longest_binding_s: u32, post_roll_s: u32, rate: FrameRate) -> i64 {
    (i64::from(longest_binding_s) + i64::from(post_roll_s) + 1).saturating_mul(rate.hz())
}

impl<P> Ring<P> {
    /// A ring with `audio_tracks` audio tracks (0 for a video-only ring) that keeps at least
    /// `keep_ticks` ticks in whole GOPs, and drops the oldest GOPs while it holds more than
    /// `cap_bytes`, but never below two GOPs or past a pin (cookbook §5.1).
    ///
    /// It reserves room for `keep_ticks` plus two 1-second GOPs of video, the same plus 3 s of
    /// audio (the encoder watchdog's 2-second stall timeout and a restart; audio is trimmed only
    /// when video evicts) and 8 more keyframes for encoder restarts. So pushing doesn't allocate
    /// while video arrives every tick in 1-second GOPs and nothing is pinned. A pin, a longer
    /// video stall or many restarts can grow the deques, which then keep their size.
    pub fn new(
        rate: FrameRate,
        audio_tracks: usize,
        keep_ticks: i64,
        cap_bytes: usize,
    ) -> Result<Self, RingError> {
        if !(1..=MAX_KEEP_SECONDS * rate.hz()).contains(&keep_ticks) {
            return Err(RingError::BadKeep(keep_ticks));
        }
        if audio_tracks > MAX_AUDIO_TRACKS {
            return Err(RingError::TooManyTracks(audio_tracks));
        }
        // keep_ticks is at most 10 minutes at 120 fps and the tracks at most 8, so these fit.
        let ticks = usize::try_from(keep_ticks + 2 * rate.hz()).unwrap_or(0);
        let gops = ticks / usize::try_from(rate.hz()).unwrap_or(1) + 2 + 8;
        let audio_ticks = ticks + usize::try_from(3 * rate.hz()).unwrap_or(0);
        let audio_packets =
            audio_ticks * usize::try_from(rate.samples_per_tick()).unwrap_or(0) / 1_024 + 4;
        Ok(Self {
            rate,
            keep_ticks,
            cap_bytes,
            video: VecDeque::with_capacity(ticks + 1),
            keys: VecDeque::with_capacity(gops),
            audio: (0..audio_tracks)
                .map(|_| VecDeque::with_capacity(audio_packets))
                .collect(),
            bytes: 0,
            pins: BTreeMap::new(),
        })
    }

    /// Adds a video packet from encoder epoch `epoch`, then evicts what has aged out.
    pub fn push_video(&mut self, epoch: u32, pkt: Pkt<P>) -> Result<(), RingError> {
        if !(-MAX_VIDEO_PTS..=MAX_VIDEO_PTS).contains(&pkt.pts) {
            return Err(RingError::OutOfRange {
                pts: pkt.pts,
                max: MAX_VIDEO_PTS,
            });
        }
        if pkt.pts != pkt.dts {
            return Err(RingError::PtsNotDts {
                pts: pkt.pts,
                dts: pkt.dts,
            });
        }
        let new_epoch = match self.video.back() {
            Some(last) if pkt.pts <= last.pkt.pts => {
                return Err(RingError::NotIncreasing {
                    last: last.pkt.pts,
                    got: pkt.pts,
                });
            }
            Some(last) if epoch < last.epoch => {
                return Err(RingError::EpochWentBack {
                    last: last.epoch,
                    got: epoch,
                });
            }
            Some(last) => epoch > last.epoch,
            None => true,
        };
        if new_epoch && !pkt.key {
            return Err(RingError::EpochWithoutKeyframe(epoch));
        }
        if pkt.key {
            self.keys.push_back((pkt.pts, epoch));
        }
        self.bytes = self.bytes.saturating_add(pkt.bytes);
        self.video.push_back(VideoPkt { epoch, pkt });
        self.evict();
        Ok(())
    }

    /// Adds a packet to audio track `track`, right where the previous one ended. Audio is trimmed
    /// when video evicts its GOPs.
    pub fn push_audio(&mut self, track: usize, pkt: Pkt<P>) -> Result<(), RingError> {
        let Some(queue) = self.audio.get_mut(track) else {
            return Err(RingError::UnknownTrack(track));
        };
        if !(-MAX_AUDIO_PTS..=MAX_AUDIO_PTS).contains(&pkt.pts) {
            return Err(RingError::OutOfRange {
                pts: pkt.pts,
                max: MAX_AUDIO_PTS,
            });
        }
        if pkt.pts != pkt.dts {
            return Err(RingError::PtsNotDts {
                pts: pkt.pts,
                dts: pkt.dts,
            });
        }
        if !(1..=MAX_AUDIO_PTS).contains(&pkt.dur) {
            return Err(RingError::BadDuration(pkt.dur));
        }
        if let Some(last) = queue.back() {
            if pkt.pts <= last.pts {
                return Err(RingError::NotIncreasing {
                    last: last.pts,
                    got: pkt.pts,
                });
            }
            if pkt.pts != last.pts + last.dur {
                return Err(RingError::AudioGap {
                    expected: last.pts + last.dur,
                    got: pkt.pts,
                });
            }
        }
        self.bytes = self.bytes.saturating_add(pkt.bytes);
        queue.push_back(pkt);
        Ok(())
    }

    /// Keeps every GOP from video pts `from_pts` on until [`Ring::unpin`]`(save)`, past the byte
    /// cap if need be. A save pins its plan's `start_pts` in the same lock as
    /// [`Ring::plan_cut`] (cookbook §5.3, step 2), and unpins on every way out, timeout included.
    pub fn pin(&mut self, save: u64, from_pts: i64) {
        self.pins.insert(save, from_pts);
    }

    /// Releases a save's pin, then evicts what has aged out meanwhile.
    pub fn unpin(&mut self, save: u64) {
        self.pins.remove(&save);
        self.evict();
    }

    /// Bytes held by every retained packet.
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// Ticks from the oldest retained video packet to the newest; 0 when empty. The UI shows the
    /// shorter of this and the binding's length (docs/PLAN.md §2.2).
    pub fn retained_ticks(&self) -> i64 {
        match (self.video.front(), self.video.back()) {
            (Some(front), Some(back)) => back.pkt.pts - front.pkt.pts + 1,
            _ => 0,
        }
    }

    /// Retained video packets.
    pub fn len(&self) -> usize {
        self.video.len()
    }

    /// True if no video packet is retained.
    pub fn is_empty(&self) -> bool {
        self.video.is_empty()
    }

    /// The output frame rate, which sets the samples per tick.
    pub fn rate(&self) -> FrameRate {
        self.rate
    }

    pub(crate) fn video(&self) -> &VecDeque<VideoPkt<P>> {
        &self.video
    }

    pub(crate) fn keys(&self) -> &VecDeque<(i64, u32)> {
        &self.keys
    }

    pub(crate) fn audio(&self) -> &[VecDeque<Pkt<P>>] {
        &self.audio
    }

    /// Drops whole GOPs from the front while the second GOP alone covers `keep_ticks` or the
    /// ring is over its byte cap, always keeping at least 2 GOPs and every pinned GOP. Audio
    /// keeps two AAC packets of pre-roll before the new first keyframe.
    fn evict(&mut self) {
        let spt = self.rate.samples_per_tick();
        while self.keys.len() >= 3 {
            let (Some(&(second, _)), Some(newest)) =
                (self.keys.get(1), self.video.back().map(|v| v.pkt.pts))
            else {
                break;
            };
            if newest - second < self.keep_ticks && self.bytes <= self.cap_bytes {
                break;
            }
            if self.pins.values().any(|&pin| pin < second) {
                break;
            }
            let gop = self.video.partition_point(|v| v.pkt.pts < second);
            for v in self.video.drain(..gop) {
                self.bytes = self.bytes.saturating_sub(v.pkt.bytes);
            }
            self.keys.pop_front();
            let keep_from = second * spt - 2 * AAC_FRAME;
            for queue in &mut self.audio {
                while let Some(front) = queue.pop_front_if(|p| p.pts + p.dur <= keep_from) {
                    self.bytes = self.bytes.saturating_sub(front.bytes);
                }
            }
        }
    }
}

/// Test fills shared with `cut`'s tests.
#[cfg(test)]
pub(crate) mod fill {
    use super::*;

    /// A video packet of 50,000 bytes, a keyframe every 60 ticks.
    pub(crate) fn video(tick: i64) -> Pkt<()> {
        Pkt {
            pts: tick,
            dts: tick,
            dur: 1,
            key: tick % 60 == 0,
            bytes: 50_000,
            payload: (),
        }
    }

    /// An AAC packet of 400 bytes.
    pub(crate) fn audio(pts: i64) -> Pkt<()> {
        Pkt {
            pts,
            dts: pts,
            dur: AAC_FRAME,
            key: true,
            bytes: 400,
            payload: (),
        }
    }

    /// Pushes video ticks `from..from + ticks` of `epoch`. After each tick `t`, every track gets
    /// the AAC packets complete by sample `t × 800`, on the 1,024 grid from `audio_from`, as a
    /// real encoder only emits a packet once it has its samples. (corecheck's fill pushed a
    /// packet as soon as it started.)
    pub(crate) fn fill(ring: &mut Ring<()>, epoch: u32, from: i64, ticks: i64, audio_from: i64) {
        let spt = ring.rate().samples_per_tick();
        for tick in from..from + ticks {
            ring.push_video(epoch, video(tick)).unwrap();
            for track in 0..ring.audio().len() {
                let mut next = ring.audio()[track]
                    .back()
                    .map_or(audio_from, |p| p.pts + AAC_FRAME);
                while next + AAC_FRAME <= tick * spt {
                    ring.push_audio(track, audio(next)).unwrap();
                    next += AAC_FRAME;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fill::{audio, fill, video};
    use super::*;

    // Expected values come from an independent Python model of the ring.

    fn retained_bytes(ring: &Ring<()>) -> usize {
        ring.video.iter().map(|v| v.pkt.bytes).sum::<usize>()
            + ring.audio.iter().flatten().map(|p| p.bytes).sum::<usize>()
    }

    #[test]
    fn evicts_whole_gops_once_the_second_covers_keep() {
        // corecheck's case: 2 minutes at 60 fps into a 31-second ring.
        let mut ring = Ring::new(FrameRate::Fps60, 2, 1_860, usize::MAX).unwrap();
        fill(&mut ring, 0, 0, 7_200, 0);
        assert!(ring.video.front().unwrap().pkt.key);
        assert_eq!(ring.video.front().unwrap().pkt.pts, 5_280);
        assert_eq!((ring.len(), ring.retained_ticks()), (1_920, 1_920));
        // Audio keeps two packets of pre-roll before the first keyframe's sample time.
        assert_eq!(ring.audio[0].front().unwrap().pts, 4_221_952);
        assert_eq!(
            ring.audio.iter().map(VecDeque::len).collect::<Vec<_>>(),
            [1_501, 1_501]
        );
        assert_eq!(ring.bytes(), 97_200_800);
        assert_eq!(ring.bytes(), retained_bytes(&ring));
    }

    #[test]
    fn running_without_pins_never_reallocates() {
        let mut ring = Ring::new(FrameRate::Fps60, 2, 1_860, usize::MAX).unwrap();
        let capacities = |ring: &Ring<()>| {
            let audio: Vec<usize> = ring.audio.iter().map(VecDeque::capacity).collect();
            (ring.video.capacity(), ring.keys.capacity(), audio)
        };
        let reserved = capacities(&ring);
        fill(&mut ring, 0, 0, 7_200, 0);
        assert_eq!(capacities(&ring), reserved);
        // A 2.5 s video stall, with audio still coming, then a new encoder epoch.
        for _ in 0..117 {
            for track in 0..2 {
                let next = ring.audio[track].back().unwrap().pts + AAC_FRAME;
                ring.push_audio(track, audio(next)).unwrap();
            }
        }
        fill(&mut ring, 1, 7_380, 3_000, 0);
        assert_eq!(capacities(&ring), reserved);
    }

    #[test]
    fn the_byte_cap_keeps_two_gops_and_pins() {
        // corecheck used keep = i64::MAX here; the ring now refuses that, and only the cap
        // matters in this test.
        let mut ring = Ring::new(FrameRate::Fps60, 1, 7_260, 10_000_000).unwrap();
        ring.pin(1, 0);
        fill(&mut ring, 0, 0, 1_200, 0);
        assert_eq!(ring.video.front().unwrap().pkt.pts, 0);
        assert_eq!(ring.bytes(), 60_374_400);
        ring.unpin(1);
        assert_eq!(ring.keys.len(), 3);
        assert_eq!(ring.video.front().unwrap().pkt.pts, 1_020);
        assert_eq!(ring.bytes(), 9_056_800);
        assert_eq!(ring.bytes(), retained_bytes(&ring));
    }

    #[test]
    fn the_byte_cap_never_goes_below_two_gops() {
        let mut ring = Ring::new(FrameRate::Fps60, 1, 7_260, 0).unwrap();
        fill(&mut ring, 0, 0, 1_200, 0);
        let keys: Vec<i64> = ring.keys.iter().map(|&(pts, _)| pts).collect();
        assert_eq!(keys, [1_080, 1_140]);
        assert_eq!((ring.len(), ring.bytes()), (120, 6_038_000));
    }

    #[test]
    fn a_pin_after_the_second_keyframe_does_not_block() {
        let mut ring = Ring::new(FrameRate::Fps60, 0, 120, usize::MAX).unwrap();
        ring.pin(7, 60);
        fill(&mut ring, 0, 0, 241, 0);
        // At tick 240 the GOP at 60 has aged out, but the pin holds it. The GOP at 0 went at
        // tick 180: a pin at 60 is not older than that GOP's successor.
        assert_eq!(ring.video.front().unwrap().pkt.pts, 60);
        ring.unpin(7);
        assert_eq!(ring.video.front().unwrap().pkt.pts, 120);
    }

    #[test]
    fn refused_packets_leave_the_ring_unchanged() {
        let mut ring = Ring::new(FrameRate::Fps60, 2, 600, usize::MAX).unwrap();
        assert_eq!(
            ring.push_video(0, video(1)),
            Err(RingError::EpochWithoutKeyframe(0))
        );
        ring.push_video(0, video(0)).unwrap();
        ring.push_video(0, video(1)).unwrap();
        ring.push_audio(0, audio(0)).unwrap();
        let before = (ring.len(), ring.bytes(), ring.keys.len());
        let cases = [
            (
                ring.push_video(0, video(1)),
                RingError::NotIncreasing { last: 1, got: 1 },
            ),
            (
                ring.push_video(0, Pkt { dts: 3, ..video(2) }),
                RingError::PtsNotDts { pts: 2, dts: 3 },
            ),
            // Refused keyframes don't reach the keyframe index either.
            (
                ring.push_video(
                    0,
                    Pkt {
                        dts: 61,
                        ..video(60)
                    },
                ),
                RingError::PtsNotDts { pts: 60, dts: 61 },
            ),
            (
                ring.push_video(1, video(5)),
                RingError::EpochWithoutKeyframe(1),
            ),
            (
                ring.push_video(
                    0,
                    Pkt {
                        key: true,
                        ..video(MAX_VIDEO_PTS + 1)
                    },
                ),
                RingError::OutOfRange {
                    pts: MAX_VIDEO_PTS + 1,
                    max: MAX_VIDEO_PTS,
                },
            ),
            (ring.push_audio(2, audio(0)), RingError::UnknownTrack(2)),
            (
                ring.push_audio(0, audio(0)),
                RingError::NotIncreasing { last: 0, got: 0 },
            ),
            (
                ring.push_audio(0, audio(2_048)),
                RingError::AudioGap {
                    expected: 1_024,
                    got: 2_048,
                },
            ),
            (
                ring.push_audio(0, audio(512)),
                RingError::AudioGap {
                    expected: 1_024,
                    got: 512,
                },
            ),
            (
                ring.push_audio(
                    0,
                    Pkt {
                        dur: MAX_AUDIO_PTS + 1,
                        ..audio(1_024)
                    },
                ),
                RingError::BadDuration(MAX_AUDIO_PTS + 1),
            ),
            (
                ring.push_audio(
                    0,
                    Pkt {
                        dur: 0,
                        ..audio(1_024)
                    },
                ),
                RingError::BadDuration(0),
            ),
            (
                ring.push_audio(1, Pkt { dts: 1, ..audio(0) }),
                RingError::PtsNotDts { pts: 0, dts: 1 },
            ),
            (
                ring.push_audio(1, audio(-MAX_AUDIO_PTS - 1)),
                RingError::OutOfRange {
                    pts: -MAX_AUDIO_PTS - 1,
                    max: MAX_AUDIO_PTS,
                },
            ),
        ];
        for (result, expected) in cases {
            assert_eq!(result, Err(expected));
        }
        assert_eq!((ring.len(), ring.bytes(), ring.keys.len()), before);
        ring.push_audio(0, audio(1_024)).unwrap();
        ring.push_video(2, video(60)).unwrap();
        assert_eq!(
            ring.push_video(1, video(120)),
            Err(RingError::EpochWentBack { last: 2, got: 1 })
        );
        assert_eq!(
            ring.push_video(2, video(60)),
            Err(RingError::NotIncreasing { last: 60, got: 60 })
        );
        assert_eq!(ring.keys.len(), 2);
    }

    #[test]
    fn keep_is_bounded() {
        assert_eq!(keep_ticks(30, 0, FrameRate::Fps60), 1_860);
        assert_eq!(keep_ticks(120, 0, FrameRate::Fps60), 7_260);
        assert_eq!(keep_ticks(30, 5, FrameRate::Fps120), 4_320);
        for keep in [0, -1, 600 * 60 + 1, i64::MAX] {
            assert_eq!(
                Ring::<()>::new(FrameRate::Fps60, 1, keep, 1).map(|_| ()),
                Err(RingError::BadKeep(keep))
            );
        }
        assert!(Ring::<()>::new(FrameRate::Fps120, 4, 600 * 120, usize::MAX).is_ok());
        assert!(Ring::<()>::new(FrameRate::Fps120, MAX_AUDIO_TRACKS, 600 * 120, 1).is_ok());
        for tracks in [MAX_AUDIO_TRACKS + 1, usize::MAX] {
            assert_eq!(
                Ring::<()>::new(FrameRate::Fps60, tracks, 600, 1).map(|_| ()),
                Err(RingError::TooManyTracks(tracks))
            );
        }
        let empty = Ring::<()>::new(FrameRate::Fps30, 0, 1, 0).unwrap();
        assert_eq!(
            (empty.len(), empty.retained_ticks(), empty.bytes()),
            (0, 0, 0)
        );
        assert!(empty.is_empty());
    }
}
