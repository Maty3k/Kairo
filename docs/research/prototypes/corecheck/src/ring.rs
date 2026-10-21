//! kairo-core replay ring (platform-neutral; the engine stores ffmpeg Packets as the payload P).
//! Video pts = pacer tick (time base 1/fps); audio pts = 48 kHz sample index; both from the same QPC epoch.
use std::collections::{BTreeMap, VecDeque};

#[derive(Clone, Debug)]
pub struct Pkt<P> { pub pts: i64, pub dts: i64, pub dur: i64, pub key: bool, pub epoch: u32, pub bytes: usize, pub payload: P }

pub struct Ring<P> {
    pub fps: i64,                       // 30/60/120: 48000 % fps == 0, so a tick is a whole number of samples
    pub video: VecDeque<Pkt<P>>,        // bf=0 => decode order == presentation order
    pub audio: Vec<VecDeque<Pkt<P>>>,   // per track; all tracks share one 1024-sample grid
    pub keep_ticks: i64,                // longest binding + post-roll + 1 GOP
    pub cap_bytes: usize,               // safety cap (RAM)
    bytes: usize,
    pins: BTreeMap<u64, i64>,           // save id -> earliest video pts a pending save still needs
}

#[derive(Debug, PartialEq)]
pub struct CutPlan { pub epoch: u32, pub start_pts: i64, pub end_pts: i64, pub a0: i64, pub audio_first: Vec<Option<i64>>, pub truncated: bool }

impl<P: Clone> Ring<P> {
    pub fn new(fps: i64, tracks: usize, keep_ticks: i64, cap_bytes: usize) -> Self {
        assert!(48_000 % fps == 0);
        Self { fps, video: VecDeque::new(), audio: (0..tracks).map(|_| VecDeque::new()).collect(), keep_ticks, cap_bytes, bytes: 0, pins: BTreeMap::new() }
    }
    pub fn bytes(&self) -> usize { self.bytes }
    pub fn push_video(&mut self, p: Pkt<P>) { self.bytes += p.bytes; self.video.push_back(p); self.evict(); }
    pub fn push_audio(&mut self, track: usize, p: Pkt<P>) { self.bytes += p.bytes; self.audio[track].push_back(p); }

    /// Drop whole GOPs from the front: when older than keep_ticks or over the byte cap, but always keep
    /// >= 2 GOPs and every pinned GOP. Audio is trimmed to 2 packets before the new first keyframe.
    fn evict(&mut self) {
        let Some(newest) = self.video.back().map(|p| p.pts) else { return };
        loop {
            let keys: Vec<usize> = self.video.iter().enumerate().filter(|(_, p)| p.key).map(|(i, _)| i).take(3).collect();
            if keys.len() < 3 { break; }                       // dropping GOP 1 must still leave >= 2 GOPs
            let next = &self.video[keys[1]];
            let too_old = newest - next.pts >= self.keep_ticks; // the 2nd GOP alone already covers keep_ticks
            if !(too_old || self.bytes > self.cap_bytes) { break; }
            if self.pins.values().any(|&pin| pin < next.pts) { break; }
            let cut_pts = next.pts;
            for p in self.video.drain(..keys[1]) { self.bytes -= p.bytes; }
            let a_keep = cut_pts * (48_000 / self.fps) - 2 * 1024;
            for t in &mut self.audio {
                while t.front().is_some_and(|p| p.pts + p.dur <= a_keep) { self.bytes -= t.pop_front().unwrap().bytes; }
            }
        }
    }

    /// Hotkey at tick `t` (QPC-stamped on the shell thread, converted to ticks). Never spans two epochs:
    /// if the newest epoch starts after t - pre, the clip starts at that epoch's first IDR (truncated = true).
    pub fn plan_cut(&self, t: i64, pre_ticks: i64, post_ticks: i64) -> Option<CutPlan> {
        let epoch = self.video.back()?.epoch;
        let want = t - pre_ticks;
        let in_epoch = || self.video.iter().filter(|p| p.key && p.epoch == epoch);
        let (k, truncated) = match in_epoch().filter(|p| p.pts <= want).last() {
            Some(k) => (k, false),
            None => (in_epoch().next()?, true),
        };
        let a0 = k.pts * (48_000 / self.fps);               // the IDR's exact sample time
        let audio_first = self.audio.iter().map(|tr| audio_start(tr, a0)).collect();
        Some(CutPlan { epoch, start_pts: k.pts, end_pts: t + post_ticks, a0, audio_first, truncated })
    }
    pub fn pin(&mut self, save: u64, from_pts: i64) { self.pins.insert(save, from_pts); }
    pub fn unpin(&mut self, save: u64) { self.pins.remove(&save); self.evict(); }
}

/// First audio packet to write: the one covering a0 (pts <= a0 < pts+1024) plus one pre-roll packet, so the
/// decoder's warm-up frame lands before the IDR. Written with pts - a0 (negative) and use_editlist=0:
/// make_zero then shifts all streams and movenc holds video frame 0 for <= 42.7 ms; A/V exact afterwards.
pub fn audio_start<P>(track: &VecDeque<Pkt<P>>, a0: i64) -> Option<i64> {
    let i = track.iter().rposition(|p| p.pts <= a0)?;
    Some(track[i.saturating_sub(1)].pts)
}

#[cfg(test)]
mod t {
    use super::*;
    fn fill(r: &mut Ring<()>, ticks: i64, epoch: u32, from: i64) {
        for tick in from..from + ticks {
            r.push_video(Pkt { pts: tick, dts: tick, dur: 1, key: tick % 60 == 0, epoch, bytes: 50_000, payload: () });
            let s = tick * 800; // samples so far; emit AAC packets on the 1024 grid
            while r.audio[0].back().map_or(-1024, |p| p.pts) + 1024 <= s {
                let pts = r.audio[0].back().map_or(0, |p| p.pts + 1024);
                for tr in 0..r.audio.len() { r.push_audio(tr, Pkt { pts, dts: pts, dur: 1024, key: true, epoch, bytes: 400, payload: () }); }
            }
        }
    }
    #[test] fn cut_and_evict() {
        let mut r = Ring::<()>::new(60, 2, 31 * 60, usize::MAX);
        fill(&mut r, 120 * 60, 0, 0);                          // 2 min of 60 fps
        assert!(r.video.front().unwrap().key);
        assert!(r.video.len() <= 32 * 60 && r.video.len() >= 31 * 60, "{}", r.video.len());
        let c = r.plan_cut(7199, 30 * 60, 0).unwrap();         // hotkey on the last tick, 30 s
        assert_eq!(c.start_pts % 60, 0);
        assert!(c.start_pts <= 7199 - 1800 && c.start_pts > 7199 - 1800 - 60);
        assert_eq!(c.a0, c.start_pts * 800);
        let a = c.audio_first[0].unwrap();
        assert!(a <= c.a0 - 1024 && a > c.a0 - 2048, "a={a} a0={}", c.a0); // p-1 starts 1024..2047 before the IDR
        assert_eq!(c.audio_first[0], c.audio_first[1]);       // same grid on every track
        assert!(!c.truncated);
    }
    #[test] fn never_spans_epochs() {
        let mut r = Ring::<()>::new(60, 1, 200 * 60, usize::MAX);
        fill(&mut r, 600, 0, 0);
        fill(&mut r, 600, 1, 600);                            // encoder restarted at tick 600 (new epoch)
        let c = r.plan_cut(1199, 1800, 0).unwrap();
        assert_eq!((c.epoch, c.start_pts, c.truncated), (1, 600, true));
    }
    #[test] fn byte_cap_keeps_two_gops_and_pins() {
        let mut r = Ring::<()>::new(60, 1, i64::MAX, 10_000_000);
        r.pin(1, 0);
        fill(&mut r, 1200, 0, 0);
        assert_eq!(r.video.front().unwrap().pts, 0);          // pinned: nothing evicted
        r.unpin(1);
        let gops = r.video.iter().filter(|p| p.key).count();
        assert!(gops >= 2 && r.bytes() <= 10_000_000 + 60 * 50_000, "gops={gops} bytes={}", r.bytes());
    }
}
