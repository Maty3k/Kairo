// SPDX-License-Identifier: GPL-3.0-or-later
//! Cutting a clip from the replay ring (docs/research/15-implementation-cookbook.md §5.3, steps
//! 2 to 5; P8 in its §0.2 plan-impacts table): plan the cut, wait until the ring holds all of
//! it, take a snapshot and rebase it so the clip starts at 0.
//!
//! A clip starts on a video keyframe K and never spans two encoder epochs. Each audio track
//! starts with the AAC packet covering K's sample time plus one earlier packet, so the decoder's
//! warm-up frame lands before the clip starts (research 10 §5.8); at the very start of a track,
//! where there is no earlier packet, it starts with the covering packet. The packets keep their
//! negative pts. With `use_editlist=0`, FFmpeg's MP4 muxer then holds video frame 0 for at most
//! 42.7 ms instead of moving the audio (cookbook P8 and §5.3, to be confirmed by M3's
//! flash-and-beep test).

use std::collections::VecDeque;
use std::fmt;

use crate::ring::{AAC_FRAME, Pkt, Ring};
use crate::timeline::FrameRate;

/// Where a clip starts and ends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CutPlan {
    /// The encoder epoch the clip comes from.
    pub epoch: u32,
    /// The first video packet's pts: keyframe K.
    pub start_pts: i64,
    /// The last tick the clip asks for: the hotkey's tick plus the post-roll.
    pub end_pts: i64,
    /// K's sample time, `start_pts` × samples per tick; audio is rebased by it.
    pub a0: i64,
    /// Per audio track, the first packet's pts as planned: the packet before the one covering
    /// `a0`, or the covering packet at the track's start. Audio reaches the ring about 250 ms
    /// after video, so [`Ring::snapshot`] chooses again once it has arrived.
    pub audio_first: Vec<i64>,
    /// The clip starts later than asked: the ring or the epoch began after `t − pre`, or audio
    /// began after the first keyframe.
    pub truncated: bool,
}

/// Packets copied out of the ring and rebased: video pts 0 is keyframe K, and audio pts 0 is K's
/// sample time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot<P> {
    /// The encoder epoch, whose codec parameters the file uses.
    pub epoch: u32,
    /// The output frame rate: video pts count its ticks.
    pub rate: FrameRate,
    /// Video packets, pts = dts from 0, each 1 tick long.
    pub video: Vec<Pkt<P>>,
    /// Per audio track, its packets. The first has a pts from −2,047 to −1,024, or from −1,023
    /// to 0 when it is the track's very first packet.
    pub audio: Vec<Vec<Pkt<P>>>,
    /// The epoch ended, or ticks were missing, before `end_pts`, so the clip ends sooner than
    /// asked.
    pub end_truncated: bool,
}

/// Why no clip could be planned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CutError {
    /// The ring holds no video.
    Empty,
    /// `pre_ticks` must be positive and `post_ticks` not negative.
    BadLength {
        /// The length before the hotkey.
        pre_ticks: i64,
        /// The post-roll.
        post_ticks: i64,
    },
    /// The hotkey's tick is older than every retained video packet.
    BeforeBuffer(i64),
    /// No keyframe at or before the hotkey has audio on every track.
    NoAudio,
    /// The epoch has no retained keyframe. The ring's own checks rule this out; it is reported
    /// rather than assumed.
    NoKeyframe(u32),
}

impl fmt::Display for CutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("the replay ring is empty"),
            Self::BadLength {
                pre_ticks,
                post_ticks,
            } => write!(
                f,
                "the clip needs a positive length and no negative post-roll, not {pre_ticks} and {post_ticks}"
            ),
            Self::BeforeBuffer(tick) => write!(f, "tick {tick} is older than the replay ring"),
            Self::NoAudio => f.write_str("no keyframe before the hotkey has audio on every track"),
            Self::NoKeyframe(epoch) => write!(f, "epoch {epoch} has no keyframe in the ring"),
        }
    }
}

impl std::error::Error for CutError {}

/// The first audio packet to write for a clip starting at sample `a0`: the packet before the one
/// covering `a0` (`pts ≤ a0 < pts + 1,024`; tracks are gapless, so the last packet starting at
/// or before `a0` covers it unless the track ends sooner), or that packet itself when it is the
/// track's first. `None` if every packet starts after `a0`.
pub fn audio_start<P>(track: &VecDeque<Pkt<P>>, a0: i64) -> Option<i64> {
    let covering = track.iter().rposition(|p| p.pts <= a0)?;
    track.get(covering.saturating_sub(1)).map(|p| p.pts)
}

impl<P> Ring<P> {
    /// Plans a clip of `pre_ticks` before the hotkey's tick `t` and `post_ticks` after it.
    ///
    /// The epoch is the one holding tick `t` (the newest if `t` is ahead of the encoder), so an
    /// encoder restart between the hotkey and the plan can't move the clip past its end. K is that
    /// epoch's last keyframe at or before `t − pre_ticks`, or its first retained keyframe if none
    /// is. If a track has no audio at K, K moves to the epoch's next keyframe at or before `t`
    /// that has audio on every track. The save pins the plan's `start_pts` in the same lock.
    pub fn plan_cut(&self, t: i64, pre_ticks: i64, post_ticks: i64) -> Result<CutPlan, CutError> {
        let (Some(front), Some(newest)) = (self.video().front(), self.video().back()) else {
            return Err(CutError::Empty);
        };
        if pre_ticks <= 0 || post_ticks < 0 {
            return Err(CutError::BadLength {
                pre_ticks,
                post_ticks,
            });
        }
        if t < front.pkt.pts {
            return Err(CutError::BeforeBuffer(t));
        }
        let epoch = if t >= newest.pkt.pts {
            newest.epoch
        } else {
            let at = self.video().partition_point(|v| v.pkt.pts <= t);
            self.video()
                .get(at.saturating_sub(1))
                .map_or(newest.epoch, |v| v.epoch)
        };
        let keys: Vec<i64> = self
            .keys()
            .iter()
            .filter(|&&(_, key_epoch)| key_epoch == epoch)
            .map(|&(pts, _)| pts)
            .collect();
        let want = t.saturating_sub(pre_ticks);
        let (mut start, mut truncated) = match keys.iter().rev().find(|&&k| k <= want) {
            Some(&k) => (k, false),
            None => (*keys.first().ok_or(CutError::NoKeyframe(epoch))?, true),
        };
        let spt = self.rate().samples_per_tick();
        let has_audio = |k: i64| {
            self.audio()
                .iter()
                .all(|track| audio_start(track, k * spt).is_some())
        };
        if !has_audio(start) {
            start = *keys
                .iter()
                .find(|&&k| k > start && k <= t && has_audio(k))
                .ok_or(CutError::NoAudio)?;
            truncated = true;
        }
        let a0 = start * spt;
        let audio_first = self
            .audio()
            .iter()
            .map(|track| audio_start(track, a0).ok_or(CutError::NoAudio))
            .collect::<Result<Vec<i64>, CutError>>()?;
        Ok(CutPlan {
            epoch,
            start_pts: start,
            end_pts: t.saturating_add(post_ticks),
            a0,
            audio_first,
            truncated,
        })
    }

    /// True once the ring holds the whole clip: video has reached `end_pts` (a packet at or past
    /// it, or a newer epoch), and every track has audio up to the end of the last video frame
    /// taken.
    ///
    /// This checks the packets themselves. FFmpeg's native AAC encoder hands out a packet once
    /// its input is 2,048 samples past the packet's end (measured with FFmpeg 8.0.1), so this
    /// turns true about one video frame plus one to two AAC frames after cookbook §5.3 step 3's
    /// rule on the mixer's position: 38 to 59 ms later at 60 fps, 55 to 75 ms at 30 fps.
    pub fn ready(&self, plan: &CutPlan) -> bool {
        let video_done = self
            .video()
            .back()
            .is_some_and(|v| v.epoch > plan.epoch || v.pkt.pts >= plan.end_pts);
        let Some(last) = self.last_video(plan) else {
            return false;
        };
        if !video_done {
            return false;
        }
        let audio_end = last
            .saturating_add(1)
            .saturating_mul(self.rate().samples_per_tick());
        self.audio()
            .iter()
            .all(|track| track.back().is_some_and(|p| p.pts + p.dur >= audio_end))
    }

    /// Takes the clip's packets and rebases them (cookbook §5.3, steps 4 and 5). Call it under
    /// the ring's lock once [`Ring::ready`] is true, then unpin. Video runs from K to `end_pts`
    /// within the epoch. Each audio track runs from its first packet, chosen now that the audio
    /// has arrived, to the end of the last video frame taken; a track with nothing at or before
    /// K comes out empty. Every payload is cloned, so `P`'s `Clone` must only add a reference.
    pub fn snapshot(&self, plan: &CutPlan) -> Snapshot<P>
    where
        P: Clone,
    {
        let last = self.last_video(plan).unwrap_or(plan.start_pts);
        let video = self
            .video()
            .iter()
            .filter(|v| v.epoch == plan.epoch && (plan.start_pts..=last).contains(&v.pkt.pts))
            .map(|v| Pkt {
                pts: v.pkt.pts.saturating_sub(plan.start_pts),
                dts: v.pkt.pts.saturating_sub(plan.start_pts),
                dur: 1,
                key: v.pkt.key,
                bytes: v.pkt.bytes,
                payload: v.pkt.payload.clone(),
            })
            .collect();
        let audio_end = last
            .saturating_add(1)
            .saturating_mul(self.rate().samples_per_tick());
        let audio = self
            .audio()
            .iter()
            .map(|track| {
                let Some(first) = audio_start(track, plan.a0) else {
                    return Vec::new();
                };
                track
                    .iter()
                    .filter(|p| (first..audio_end).contains(&p.pts))
                    .map(|p| Pkt {
                        pts: p.pts.saturating_sub(plan.a0),
                        dts: p.pts.saturating_sub(plan.a0),
                        dur: AAC_FRAME,
                        key: true,
                        bytes: p.bytes,
                        payload: p.payload.clone(),
                    })
                    .collect()
            })
            .collect();
        Snapshot {
            epoch: plan.epoch,
            rate: self.rate(),
            video,
            audio,
            end_truncated: last < plan.end_pts,
        }
    }

    /// The last video pts the clip takes: the epoch's last packet at or before `end_pts`.
    fn last_video(&self, plan: &CutPlan) -> Option<i64> {
        self.video()
            .iter()
            .rev()
            .find(|v| v.epoch == plan.epoch && v.pkt.pts <= plan.end_pts)
            .map(|v| v.pkt.pts)
    }
}

impl<P> Snapshot<P> {
    /// Every packet in the order the muxer takes them: by time, with video first on a tie and
    /// tracks in order. Stream 0 is video and stream `i + 1` audio track `i`. Times compare
    /// exactly, since a tick is a whole number of samples.
    pub fn interleaved(&self) -> Vec<(usize, &Pkt<P>)> {
        let spt = self.rate.samples_per_tick();
        let mut order: Vec<(i64, usize, usize)> = self
            .video
            .iter()
            .enumerate()
            .map(|(i, p)| (p.pts.saturating_mul(spt), 0, i))
            .chain(self.audio.iter().enumerate().flat_map(|(track, packets)| {
                packets
                    .iter()
                    .enumerate()
                    .map(move |(i, p)| (p.pts, track + 1, i))
            }))
            .collect();
        order.sort_unstable();
        order
            .into_iter()
            .filter_map(|(_, stream, i)| {
                let packet = if stream == 0 {
                    self.video.get(i)
                } else {
                    self.audio
                        .get(stream - 1)
                        .and_then(|packets| packets.get(i))
                };
                packet.map(|p| (stream, p))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ring::fill::{audio, fill, video};

    // Expected values come from an independent Python model of the ring and the cut.

    /// corecheck's case: 2 minutes at 60 fps, 2 tracks, a 31-second ring.
    fn two_minutes() -> Ring<()> {
        let mut ring = Ring::new(FrameRate::Fps60, 2, 1_860, usize::MAX).unwrap();
        fill(&mut ring, 0, 0, 7_200, 0);
        ring
    }

    /// Epoch 0 holds ticks 0 to 599 and epoch 1 starts at 600, one track.
    fn two_epochs() -> Ring<()> {
        let mut ring = Ring::new(FrameRate::Fps60, 1, 12_000, usize::MAX).unwrap();
        fill(&mut ring, 0, 0, 600, 0);
        fill(&mut ring, 1, 600, 600, 0);
        ring
    }

    /// Like `fill`, but each track's audio trails the video by `lag` ticks, as the mixer's
    /// 200 ms delay makes it.
    fn fill_lagging(ring: &mut Ring<()>, epoch: u32, from: i64, ticks: i64, lag: i64) {
        for tick in from..from + ticks {
            ring.push_video(epoch, video(tick)).unwrap();
            for track in 0..ring.audio().len() {
                let mut next = ring.audio()[track].back().map_or(0, |p| p.pts + AAC_FRAME);
                while next + AAC_FRAME <= (tick - lag) * 800 {
                    ring.push_audio(track, audio(next)).unwrap();
                    next += AAC_FRAME;
                }
            }
        }
    }

    #[test]
    fn plans_a_thirty_second_clip_on_a_keyframe() {
        let ring = two_minutes();
        let plan = ring.plan_cut(7_199, 1_800, 0).unwrap();
        assert_eq!(
            plan,
            CutPlan {
                epoch: 0,
                start_pts: 5_340,
                end_pts: 7_199,
                a0: 4_272_000,
                // 1,920 samples before a0: the packet before the one covering it.
                audio_first: vec![4_270_080, 4_270_080],
                truncated: false,
            }
        );
    }

    #[test]
    fn ready_waits_for_the_audio_of_the_last_frame() {
        let mut ring = two_minutes();
        let plan = ring.plan_cut(7_199, 1_800, 0).unwrap();
        ring.pin(1, plan.start_pts);
        // Audio up to the end of tick 7,199 (sample 5,760,000) is complete only at tick 7,200.
        assert!(!ring.ready(&plan));
        fill(&mut ring, 0, 7_200, 1, 0);
        assert!(ring.ready(&plan));
        let snapshot = ring.snapshot(&plan);
        ring.unpin(1);
        assert_eq!(snapshot.epoch, 0);
        assert!(!snapshot.end_truncated);
        let video: Vec<i64> = snapshot.video.iter().map(|p| p.pts).collect();
        assert_eq!(video, (0..1_860).collect::<Vec<i64>>());
        assert!(snapshot.video.iter().all(|p| p.dts == p.pts && p.dur == 1));
        assert!(snapshot.video[0].key);
        for track in &snapshot.audio {
            let pts: Vec<i64> = track.iter().map(|p| p.pts).collect();
            assert_eq!(pts.len(), 1_455);
            assert_eq!(pts[..2], [-1_920, -896]);
            assert_eq!(pts.last(), Some(&1_486_976));
            assert!(track.iter().all(|p| p.dts == p.pts && p.dur == 1_024));
        }
    }

    #[test]
    fn never_spans_two_epochs() {
        let ring = two_epochs();
        let plan = ring.plan_cut(1_199, 1_800, 0).unwrap();
        assert_eq!(
            (plan.epoch, plan.start_pts, plan.truncated, plan.audio_first),
            (1, 600, true, vec![478_208])
        );
    }

    #[test]
    fn uses_the_epoch_holding_the_hotkey() {
        // The newest epoch would start the clip at 600, after its end at 590.
        let ring = two_epochs();
        let plan = ring.plan_cut(590, 120, 0).unwrap();
        assert_eq!(
            (
                plan.epoch,
                plan.start_pts,
                plan.end_pts,
                plan.audio_first,
                plan.truncated
            ),
            (0, 420, 590, vec![334_848], false)
        );
    }

    #[test]
    fn a_post_roll_past_the_epoch_ends_with_the_epoch() {
        let ring = two_epochs();
        let plan = ring.plan_cut(590, 120, 60).unwrap();
        assert_eq!(plan.end_pts, 650);
        // Epoch 0 has ended, so the ring will never hold ticks 600 to 650 of it.
        assert!(ring.ready(&plan));
        let snapshot = ring.snapshot(&plan);
        assert!(snapshot.end_truncated);
        assert_eq!(snapshot.video.len(), 180);
        assert_eq!(snapshot.video.last().map(|p| p.pts), Some(179));
        // Audio stops with the last frame taken (sample 480,000), not at the end asked for.
        assert_eq!(snapshot.audio[0].last().map(|p| p.pts), Some(143_232));
    }

    #[test]
    fn late_audio_moves_the_start_to_a_keyframe_with_audio() {
        let mut ring = Ring::new(FrameRate::Fps60, 1, 10_000, usize::MAX).unwrap();
        fill(&mut ring, 0, 0, 251, 51_200);
        let plan = ring.plan_cut(150, 150, 0).unwrap();
        assert_eq!(
            (plan.start_pts, plan.a0, plan.audio_first, plan.truncated),
            (120, 96_000, vec![94_208], true)
        );
        // The next keyframe with audio, not a later one, and one that falls on `t` counts.
        assert_eq!(
            ring.plan_cut(250, 250, 0).map(|plan| plan.start_pts),
            Ok(120)
        );
        assert_eq!(
            ring.plan_cut(120, 120, 0).map(|plan| plan.start_pts),
            Ok(120)
        );
        assert_eq!(ring.plan_cut(119, 119, 0), Err(CutError::NoAudio));
    }

    #[test]
    fn the_start_keyframe_may_fall_exactly_on_t_minus_pre() {
        let ring = two_epochs();
        let plan = ring.plan_cut(300, 120, 0).unwrap();
        assert_eq!((plan.start_pts, plan.truncated), (180, false));
        // A hotkey on an epoch's first tick belongs to that epoch.
        let plan = ring.plan_cut(600, 60, 0).unwrap();
        assert_eq!((plan.epoch, plan.start_pts, plan.truncated), (1, 600, true));
    }

    #[test]
    fn a_hotkey_in_a_stall_between_epochs_uses_the_earlier_epoch() {
        // Epoch 0 ends at tick 599; the encoder comes back as epoch 1 at tick 720.
        let mut ring = Ring::new(FrameRate::Fps60, 1, 12_000, usize::MAX).unwrap();
        fill(&mut ring, 0, 0, 600, 0);
        fill(&mut ring, 1, 720, 600, 0);
        let plan = ring.plan_cut(650, 120, 0).unwrap();
        assert_eq!(
            (plan.epoch, plan.start_pts, plan.truncated),
            (0, 480, false)
        );
        assert!(ring.ready(&plan));
        let snapshot = ring.snapshot(&plan);
        assert_eq!((snapshot.video.len(), snapshot.end_truncated), (120, true));
    }

    #[test]
    fn a_video_only_clip_waits_for_its_post_roll() {
        let mut ring = Ring::new(FrameRate::Fps60, 0, 12_000, usize::MAX).unwrap();
        fill(&mut ring, 0, 0, 600, 0);
        let plan = ring.plan_cut(590, 120, 60).unwrap();
        assert_eq!((plan.start_pts, plan.end_pts), (420, 650));
        assert!(!ring.ready(&plan));
        fill(&mut ring, 0, 600, 50, 0);
        assert!(!ring.ready(&plan));
        fill(&mut ring, 0, 650, 1, 0);
        assert!(ring.ready(&plan));
        let snapshot = ring.snapshot(&plan);
        assert_eq!((snapshot.video.len(), snapshot.end_truncated), (231, false));
        assert!(snapshot.audio.is_empty());
    }

    #[test]
    fn missing_ticks_at_the_end_dont_stall_the_save() {
        // The encoder lost ticks 100 to 104 inside one epoch, and the clip asks for tick 102.
        let mut ring = Ring::new(FrameRate::Fps60, 1, 600, usize::MAX).unwrap();
        fill(&mut ring, 0, 0, 100, 0);
        let plan = ring.plan_cut(90, 60, 12).unwrap();
        assert_eq!((plan.epoch, plan.start_pts, plan.end_pts), (0, 0, 102));
        assert!(!ring.ready(&plan));
        fill(&mut ring, 0, 105, 100, 0);
        assert!(ring.ready(&plan));
        let snapshot = ring.snapshot(&plan);
        assert_eq!((snapshot.video.len(), snapshot.end_truncated), (100, true));
        // Audio ends with frame 99, the last one taken, and starts on the track's first packet.
        let audio: Vec<i64> = snapshot.audio[0].iter().map(|p| p.pts).collect();
        assert_eq!((audio.first(), audio.last()), (Some(&0), Some(&79_872)));
    }

    #[test]
    fn the_audio_start_is_chosen_once_the_audio_has_arrived() {
        // Audio trails video by 200 ms; the encoder restarts at tick 600 and the hotkey comes
        // 5 ticks later, so K = 600 before any audio covering it has reached the ring.
        let mut ring = Ring::new(FrameRate::Fps60, 1, 12_000, usize::MAX).unwrap();
        fill_lagging(&mut ring, 0, 0, 600, 12);
        fill_lagging(&mut ring, 1, 600, 6, 12);
        let plan = ring.plan_cut(605, 1_800, 0).unwrap();
        assert_eq!((plan.start_pts, plan.truncated), (600, true));
        assert_eq!(plan.audio_first, [472_064]);
        let mut tick = 606;
        while !ring.ready(&plan) {
            fill_lagging(&mut ring, 1, tick, 1, 12);
            tick += 1;
        }
        assert_eq!(tick, 620);
        // The packet covering K and the one before, as P8 asks, not the plan-time guess.
        let audio: Vec<i64> = ring.snapshot(&plan).audio[0]
            .iter()
            .map(|p| p.pts)
            .collect();
        assert_eq!(audio[..2], [-1_792, -768]);
    }

    #[test]
    fn hand_made_plans_cannot_overflow() {
        let ring = two_epochs();
        let wild = CutPlan {
            epoch: 0,
            start_pts: i64::MIN,
            end_pts: i64::MAX,
            a0: i64::MIN,
            audio_first: Vec::new(),
            truncated: false,
        };
        let snapshot = ring.snapshot(&wild);
        assert_eq!(snapshot.video.len(), 600);
        assert!(snapshot.audio[0].is_empty());
        let absent = CutPlan {
            epoch: 9,
            start_pts: i64::MAX,
            ..wild
        };
        assert!(!ring.ready(&absent));
        assert!(ring.snapshot(&absent).video.is_empty());
    }

    #[test]
    fn audio_starting_after_the_hotkey_is_no_audio() {
        let mut ring = Ring::new(FrameRate::Fps60, 1, 10_000, usize::MAX).unwrap();
        fill(&mut ring, 0, 0, 130, 94_208);
        // Keyframe 120 is the first with audio but comes after tick 100, so it can't start the
        // clip; for a hotkey at 125 it can.
        assert_eq!(ring.plan_cut(100, 100, 0), Err(CutError::NoAudio));
        let plan = ring.plan_cut(125, 100, 0).unwrap();
        assert_eq!(
            (plan.start_pts, plan.audio_first, plan.truncated),
            (120, vec![94_208], true)
        );
    }

    #[test]
    fn the_first_aac_packet_for_every_offset() {
        // Research 10 §8.4: for each offset of the keyframe inside a packet, the first packet
        // written starts 1,024 to 2,047 samples early and the second 0 to 1,023.
        for origin in [0, 3_072] {
            let track: VecDeque<Pkt<()>> = (0..20).map(|j| audio(origin + 1_024 * j)).collect();
            for offset in 0..1_024 {
                let a0 = 8 * 1_024 + offset;
                assert_eq!(audio_start(&track, a0), Some(7 * 1_024), "offset {offset}");
                let first = 7 * 1_024 - a0;
                assert!((-2_047..=-1_024).contains(&first));
                assert!((-1_023..=0).contains(&(first + 1_024)));
            }
        }
        let start: VecDeque<Pkt<()>> = [0, 1_024].into_iter().map(audio).collect();
        assert_eq!(audio_start(&start, 500), Some(0));
        assert_eq!(audio_start(&start, -1), None);
    }

    #[test]
    fn bad_requests() {
        let empty = Ring::<()>::new(FrameRate::Fps60, 1, 600, usize::MAX).unwrap();
        assert_eq!(empty.plan_cut(10, 10, 0), Err(CutError::Empty));
        let ring = two_epochs();
        assert_eq!(
            ring.plan_cut(10, 0, 0),
            Err(CutError::BadLength {
                pre_ticks: 0,
                post_ticks: 0
            })
        );
        assert_eq!(
            ring.plan_cut(10, 10, -1),
            Err(CutError::BadLength {
                pre_ticks: 10,
                post_ticks: -1
            })
        );
        assert_eq!(ring.plan_cut(-1, 10, 0), Err(CutError::BeforeBuffer(-1)));
        // A hotkey ahead of the encoder uses the newest epoch.
        assert_eq!(ring.plan_cut(5_000, 60, 0).map(|plan| plan.epoch), Ok(1));
    }

    #[test]
    fn interleaves_by_time_with_video_first() {
        let packet = |pts| Pkt {
            pts,
            dts: pts,
            dur: 1,
            key: true,
            bytes: 0,
            payload: (),
        };
        let snapshot = Snapshot {
            epoch: 0,
            rate: FrameRate::Fps60,
            video: vec![packet(0), packet(1), packet(2)],
            audio: vec![
                vec![packet(-1_920), packet(-896), packet(128), packet(1_152)],
                vec![packet(-1_920), packet(800)],
            ],
            end_truncated: false,
        };
        let order: Vec<(usize, i64)> = snapshot
            .interleaved()
            .into_iter()
            .map(|(stream, p)| (stream, p.pts))
            .collect();
        assert_eq!(
            order,
            [
                (1, -1_920),
                (2, -1_920),
                (1, -896),
                (0, 0),
                (1, 128),
                (0, 1),
                (2, 800),
                (1, 1_152),
                (0, 2),
            ]
        );
    }
}
