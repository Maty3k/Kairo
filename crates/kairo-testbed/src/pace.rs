// SPDX-License-Identifier: GPL-3.0-or-later
//! Pacing and frame numbering (docs/research/16-verify-testbed/README.md §4.3).
//!
//! Frame number N is time-based, the way a game's clock keeps running: N is the index of the
//! vblank group (vsync pacing) or the timer slot (timer pacing) the frame is drawn for. A frame
//! that can't make its vblank or slot shows up as a skipped N plus an `m` line, so a testbed miss
//! is never mistaken for a capture drop. Frame 0 is drawn for the first vblank, or is the slot,
//! at or after the next whole QPC second.
//!
//! Everything here is pure: the Windows loop passes in QPC times (100 ns) and the swap chain's
//! frame statistics, and gets back what to draw and which log records to write.
//!
//! **Vsync.** `Present(k, 0)` shows each frame for k vblanks. [`VsyncPacer`] keeps the vblank
//! grid from the latest statistics (or from DWM's timing when DXGI has none) and aims each frame
//! k vblanks after the previous one: after the vblank it was shown at once the statistics say,
//! else after the one it was drawn for. When that vblank has already gone by, it skips ahead by
//! whole groups of k. N is the group on the grid that starts at frame 0's vblank, and only grows. Statistics report only the newest frame on screen and the vblank
//! it first appeared at. A frame shown later than k vblanks after the frame before it, by DXGI's
//! own vblank counts, missed its vblank. A frame the statistics never reported is filled in from
//! its neighbours when the two of them were exactly on cadence, since each frame stays at least
//! k vblanks; otherwise its display time is unknown (`-`) and the next frame shown is the late
//! one.
//!
//! **Timer.** [`TimerPacer`] presents frame N at slot N's time, `start + ⌈N × 10⁷ / fps⌉`
//! (`kairo_core::timeline::deadline_hns`'s rounding, at any rate). A frame whose `Present`
//! returns after its slot ends missed it.

use std::collections::VecDeque;
use std::ops::Range;

use kairo_core::testbed_log::FrameRecord;
use kairo_core::timeline::HNS_PER_SECOND;

/// The QPC time, in 100 ns, of the first whole second at or after `now_hns`.
pub fn next_whole_second(now_hns: i64) -> i64 {
    let rest = now_hns.rem_euclid(HNS_PER_SECOND);
    if rest == 0 {
        now_hns
    } else {
        now_hns.saturating_add(HNS_PER_SECOND - rest)
    }
}

/// A refresh period as an exact fraction of 100 ns: 10⁷ × denominator / numerator for a rate
/// of numerator / denominator Hz.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Period {
    num: i128,
    den: i128,
}

impl Period {
    /// The period of a `numerator / denominator` Hz refresh rate, or `None` unless both are
    /// above 0.
    pub fn from_refresh(numerator: u32, denominator: u32) -> Option<Period> {
        (numerator > 0 && denominator > 0).then(|| Period {
            num: i128::from(HNS_PER_SECOND) * i128::from(denominator),
            den: i128::from(numerator),
        })
    }

    /// `n` periods, in 100 ns, rounded to nearest (halves away from zero).
    pub fn times(self, n: i64) -> i64 {
        let x = i128::from(n) * self.num;
        let rounded = if x >= 0 {
            (2 * x + self.den).div_euclid(2 * self.den)
        } else {
            -(2 * -x + self.den).div_euclid(2 * self.den)
        };
        saturate(rounded)
    }

    /// The whole periods in `d` 100 ns, rounded down (towards −∞).
    pub fn count(self, d: i64) -> i64 {
        saturate((i128::from(d) * self.den).div_euclid(self.num))
    }
}

fn saturate(x: i128) -> i64 {
    i64::try_from(x).unwrap_or(if x < 0 { i64::MIN } else { i64::MAX })
}

/// Widens a wrapping 32-bit count (DXGI's present and vblank counts) to 64 bits, assuming it
/// moves by less than 2³¹ between readings.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Widen {
    last: Option<i64>,
}

impl Widen {
    /// `raw`, unwrapped next to the last value seen.
    pub fn widen(&mut self, raw: u32) -> i64 {
        let value = match self.last {
            None => i64::from(raw),
            Some(last) => last + i64::from(raw.wrapping_sub(last as u32) as i32),
        };
        self.last = Some(value);
        value
    }
}

/// The vblank grid: vblank `count` happened at `t_hns`, and the others follow every `period`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Grid {
    count: i64,
    t_hns: i64,
    period: Period,
}

impl Grid {
    /// The grid through vblank `count` at `t_hns`.
    pub fn new(count: i64, t_hns: i64, period: Period) -> Grid {
        Grid {
            count,
            t_hns,
            period,
        }
    }

    /// When vblank `v` happens.
    pub fn time_of(&self, v: i64) -> i64 {
        self.t_hns
            .saturating_add(self.period.times(v.saturating_sub(self.count)))
    }

    /// The first vblank strictly after `t_hns`: the earliest a frame presented then can show.
    pub fn next_after(&self, t_hns: i64) -> i64 {
        let mut v = self
            .count
            .saturating_add(self.period.count(t_hns.saturating_sub(self.t_hns)));
        // Rounding in `time_of` can put a vblank a unit either side of the exact fraction.
        while self.time_of(v) > t_hns {
            v -= 1;
        }
        while self.time_of(v) <= t_hns {
            v += 1;
        }
        v
    }

    /// The first vblank at or after `t_hns`.
    pub fn at_or_after(&self, t_hns: i64) -> i64 {
        self.next_after(t_hns.saturating_sub(1))
    }
}

/// One `GetFrameStatistics` reading, as DXGI gives it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stats {
    /// The present that is on screen (`PresentCount`).
    pub present_count: u32,
    /// The vblank it first appeared at (`PresentRefreshCount`).
    pub present_refresh: u32,
    /// A recent vblank (`SyncRefreshCount`)…
    pub sync_refresh: u32,
    /// …and when it happened, in 100 ns (`SyncQPCTime`).
    pub sync_hns: i64,
}

/// A frame whose fate is known: its log record, and whether it missed its vblank or slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resolved {
    /// The `f` record.
    pub record: FrameRecord,
    /// Whether to write an `m` line after it.
    pub missed: bool,
}

/// What to draw next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Black, until frame 0's vblank is next (vsync only).
    Warmup,
    /// Frame `frame`, drawn for `target_hns`. The frames in `skipped` were never drawn: each gets
    /// an `m` line.
    Frame {
        /// N.
        frame: u64,
        /// The vblank or slot time it is drawn for.
        target_hns: i64,
        /// Frames passed over because their vblank or slot had gone by.
        skipped: Range<u64>,
    },
}

#[derive(Debug, Clone, Copy)]
struct Pending {
    frame: u64,
    present_id: i64,
    target_hns: i64,
    present_hns: i64,
    /// Already known to have missed (a timer slot that ended before `Present` returned).
    missed: bool,
}

/// Which frames reached the screen, and when, from DXGI's statistics, which report only the
/// newest present on screen and the vblank it first appeared at. Frames come out in order.
#[derive(Debug, Clone)]
pub struct Shown {
    /// Vblanks per frame, when frames should keep that cadence (vsync); `None` (timer pacing)
    /// marks nothing late, since frames above the refresh rate are meant to be passed over.
    k: Option<i64>,
    period: Period,
    grid: Option<Grid>,
    pending: VecDeque<Pending>,
    /// Frames the statistics passed over, waiting for the next frame they report.
    unseen: Vec<Pending>,
    /// The last frame reported on screen, and its first vblank.
    last_shown: Option<(u64, i64)>,
    presents: Widen,
    shown_vblanks: Widen,
    sync_vblanks: Widen,
}

impl Shown {
    /// A tracker for a display of `period`, with frames `k` vblanks apart (`None`: timer).
    pub fn new(k: Option<u32>, period: Period) -> Shown {
        Shown {
            k: k.map(|k| i64::from(k.max(1))),
            period,
            grid: None,
            pending: VecDeque::new(),
            unseen: Vec::new(),
            last_shown: None,
            presents: Widen::default(),
            shown_vblanks: Widen::default(),
            sync_vblanks: Widen::default(),
        }
    }

    /// The grid from the latest statistics.
    pub fn grid(&self) -> Option<Grid> {
        self.grid
    }

    /// The last frame reported on screen, and its first vblank.
    pub fn last_shown(&self) -> Option<(u64, i64)> {
        self.last_shown
    }

    /// Frame `frame` (drawn for `target_hns`) was presented as present `present_id`
    /// (`GetLastPresentCount`), and `Present` returned at `present_hns`; `missed` if it is
    /// already known to have missed its slot.
    pub fn presented(
        &mut self,
        frame: u64,
        target_hns: i64,
        present_id: u32,
        present_hns: i64,
        missed: bool,
    ) {
        let present_id = self.presents.widen(present_id);
        self.pending.push_back(Pending {
            frame,
            present_id,
            target_hns,
            present_hns,
            missed,
        });
    }

    /// A statistics reading: updates the grid and returns the frames it settles, in order.
    pub fn on_stats(&mut self, stats: Stats) -> Vec<Resolved> {
        let sync = self.sync_vblanks.widen(stats.sync_refresh);
        let grid = Grid::new(sync, stats.sync_hns, self.period);
        self.grid = Some(grid);
        let shown_id = self.presents.widen(stats.present_count);
        let shown_v = self.shown_vblanks.widen(stats.present_refresh);
        let mut out = Vec::new();
        while let Some(&p) = self.pending.front() {
            if p.present_id > shown_id {
                break;
            }
            self.pending.pop_front();
            if p.present_id < shown_id {
                self.unseen.push(p);
                continue;
            }
            // Frame p is on screen and first appeared at vblank shown_v.
            let mut late = false;
            let unseen = std::mem::take(&mut self.unseen);
            match (self.k, self.last_shown) {
                (Some(k), Some((frame, v))) => {
                    let cadence = k * (p.frame - frame) as i64;
                    if shown_v - v == cadence {
                        // Every frame between them stayed exactly k vblanks.
                        for u in unseen {
                            let at = v + k * (u.frame - frame) as i64;
                            out.push(resolved(u, Some(grid.time_of(at)), false));
                        }
                    } else {
                        out.extend(unseen.into_iter().map(|u| resolved(u, None, false)));
                        late = shown_v - v > cadence;
                    }
                }
                _ => out.extend(unseen.into_iter().map(|u| resolved(u, None, false))),
            }
            out.push(resolved(p, Some(grid.time_of(shown_v)), late));
            self.last_shown = Some((p.frame, shown_v));
        }
        out
    }

    /// The frames still waiting for statistics at the end: their display times are unknown.
    pub fn finish(&mut self) -> Vec<Resolved> {
        let mut out: Vec<Resolved> = std::mem::take(&mut self.unseen)
            .into_iter()
            .map(|u| resolved(u, None, false))
            .collect();
        out.extend(self.pending.drain(..).map(|p| resolved(p, None, false)));
        out
    }
}

/// Vsync pacing: one frame every k vblanks (README §4.3).
#[derive(Debug, Clone)]
pub struct VsyncPacer {
    k: i64,
    period: Period,
    start_hns: i64,
    /// DWM's grid, until DXGI's statistics give one.
    dwm_grid: Option<Grid>,
    /// Frame 0's vblank, once it is chosen.
    v0: Option<i64>,
    /// The last frame drawn, and the vblank it was drawn for.
    last: Option<(u64, i64)>,
    shown: Shown,
}

impl VsyncPacer {
    /// A pacer for one frame every `k` vblanks of `period`, starting at the first vblank at or
    /// after `start_hns`.
    pub fn new(k: u32, period: Period, start_hns: i64) -> VsyncPacer {
        VsyncPacer {
            k: i64::from(k.max(1)),
            period,
            start_hns,
            dwm_grid: None,
            v0: None,
            last: None,
            shown: Shown::new(Some(k), period),
        }
    }

    /// The grid from DWM's timing, for a swap chain without frame statistics: no display times
    /// are known then. Ignored once DXGI's statistics have given a grid, since the two count
    /// vblanks differently; switch before frame 0.
    pub fn on_dwm_vblank(&mut self, count: u64, t_hns: i64) {
        let count = i64::try_from(count).unwrap_or(i64::MAX);
        self.dwm_grid = Some(Grid::new(count, t_hns, self.period));
    }

    /// A statistics reading: updates the grid and returns the frames it settles, in order.
    pub fn on_stats(&mut self, stats: Stats) -> Vec<Resolved> {
        self.shown.on_stats(stats)
    }

    /// What to draw now, at `now_hns`.
    pub fn next(&mut self, now_hns: i64) -> Step {
        let Some(grid) = self.shown.grid().or(self.dwm_grid) else {
            return Step::Warmup;
        };
        let earliest = grid.next_after(now_hns);
        let (Some(v0), Some((previous, last_target))) = (self.v0, self.last) else {
            let start = grid.at_or_after(self.start_hns);
            if earliest < start {
                return Step::Warmup;
            }
            // Frame 0 at the start vblank, or at the first one still reachable.
            self.v0 = Some(earliest);
            self.last = Some((0, earliest));
            return Step::Frame {
                frame: 0,
                target_hns: grid.time_of(earliest),
                skipped: 0..0,
            };
        };
        // k vblanks after the last frame: after the vblank it was shown at once the statistics
        // say so, which keeps the cadence's phase after a late frame.
        let after = match self.shown.last_shown() {
            Some((frame, v)) if frame == previous => v,
            _ => last_target,
        };
        let want = after + self.k;
        // Gone by: the first vblank still reachable in the same phase.
        let groups = ((earliest - want).max(0) + self.k - 1) / self.k;
        let target = want + self.k * groups;
        let frame = (((target - v0) / self.k) as u64).max(previous + 1);
        self.last = Some((frame, target));
        Step::Frame {
            frame,
            target_hns: grid.time_of(target),
            skipped: previous + 1..frame,
        }
    }

    /// Frame `frame` (drawn for `target_hns`) was presented as present `present_id`
    /// (`GetLastPresentCount`), and `Present` returned at `present_hns`.
    pub fn presented(&mut self, frame: u64, target_hns: i64, present_id: u32, present_hns: i64) {
        self.shown
            .presented(frame, target_hns, present_id, present_hns, false);
    }

    /// The frames still waiting for statistics at the end: their display times are unknown.
    pub fn finish(&mut self) -> Vec<Resolved> {
        self.shown.finish()
    }
}

fn resolved(p: Pending, display_hns: Option<i64>, late: bool) -> Resolved {
    Resolved {
        record: FrameRecord {
            frame: p.frame,
            target_hns: p.target_hns,
            present_hns: p.present_hns,
            display_hns,
        },
        missed: late || p.missed,
    }
}

/// Timer pacing: frame N at slot N (README §4.3).
#[derive(Debug, Clone)]
pub struct TimerPacer {
    start_hns: i64,
    fps: i128,
    next: u64,
}

impl TimerPacer {
    /// Slots of `fps` per second from `start_hns`.
    pub fn new(start_hns: i64, fps: u32) -> TimerPacer {
        TimerPacer {
            start_hns,
            fps: i128::from(fps.max(1)),
            next: 0,
        }
    }

    /// Slot `slot`'s time: `start + ⌈slot × 10⁷ / fps⌉`.
    pub fn slot_time(&self, slot: u64) -> i64 {
        let exact = i128::from(slot) * i128::from(HNS_PER_SECOND);
        saturate(i128::from(self.start_hns) - (-exact).div_euclid(self.fps))
    }

    /// The slot under way at `t_hns`: the last one that started at or before it (0 before the
    /// start).
    pub fn slot_at(&self, t_hns: i64) -> u64 {
        let since = i128::from(t_hns) - i128::from(self.start_hns);
        if since <= 0 {
            return 0;
        }
        let mut slot = u64::try_from(since * self.fps / i128::from(HNS_PER_SECOND)).unwrap_or(0);
        while slot > 0 && self.slot_time(slot) > t_hns {
            slot -= 1;
        }
        while self.slot_time(slot + 1) <= t_hns {
            slot += 1;
        }
        slot
    }

    /// The frame to draw next at `now_hns`: the slot after the last frame, or the slot under
    /// way if that one has already gone by. Wait for its `target_hns` before presenting.
    pub fn next(&mut self, now_hns: i64) -> Step {
        let current = self.slot_at(now_hns);
        let frame = if now_hns >= self.start_hns {
            self.next.max(current)
        } else {
            self.next
        };
        let skipped = self.next..frame;
        self.next = frame + 1;
        Step::Frame {
            frame,
            target_hns: self.slot_time(frame),
            skipped,
        }
    }

    /// Whether frame `frame`, whose `Present` returned at `present_hns`, missed its slot.
    pub fn missed(&self, frame: u64, present_hns: i64) -> bool {
        present_hns >= self.slot_time(frame + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HZ180: Period = Period {
        num: 10_000_000,
        den: 180,
    };

    #[test]
    fn whole_seconds_and_periods() {
        assert_eq!(next_whole_second(0), 0);
        assert_eq!(next_whole_second(1), 10_000_000);
        assert_eq!(next_whole_second(29_999_999), 30_000_000);
        assert_eq!(next_whole_second(-1), 0);
        let p = Period::from_refresh(179_960, 1_000).unwrap();
        // 1/179.96 s = 55,567.9 hns.
        assert_eq!(p.times(1), 55_568);
        assert_eq!(p.times(179_960), 1_000 * 10_000_000);
        assert_eq!(p.count(55_567), 0);
        assert_eq!(p.count(55_568), 1);
        assert_eq!(p.count(-1), -1);
        assert!(Period::from_refresh(0, 1).is_none());
        assert!(Period::from_refresh(60, 0).is_none());
    }

    #[test]
    fn counts_widen_across_the_wrap() {
        let mut w = Widen::default();
        assert_eq!(w.widen(u32::MAX - 1), i64::from(u32::MAX - 1));
        assert_eq!(w.widen(u32::MAX), i64::from(u32::MAX));
        assert_eq!(w.widen(1), i64::from(u32::MAX) + 2);
        assert_eq!(w.widen(0), i64::from(u32::MAX) + 1);
    }

    #[test]
    fn the_grid_finds_vblanks() {
        let g = Grid::new(100, 1_000_000, HZ180);
        assert_eq!(g.time_of(100), 1_000_000);
        assert_eq!(g.time_of(101), 1_055_556);
        assert_eq!(g.time_of(99), 944_444);
        assert_eq!(g.next_after(1_000_000), 101);
        assert_eq!(g.next_after(999_999), 100);
        assert_eq!(g.at_or_after(1_000_000), 100);
        assert_eq!(g.at_or_after(1_000_001), 101);
        assert_eq!(g.next_after(1_055_555), 101);
        assert_eq!(g.next_after(1_055_556), 102);
        // Exhaustively, over a few seconds: next_after is the first vblank later than t.
        for t in (0..30_000_000).step_by(997) {
            let v = g.next_after(t);
            assert!(g.time_of(v) > t && g.time_of(v - 1) <= t, "t {t}");
        }
    }

    /// A display that shows each present at the first vblank it can reach, k vblanks after the
    /// one before (plus `slow` more for a present the GPU is late with), with DXGI's statistics
    /// read just before each frame is drawn.
    struct Display {
        period: Period,
        /// Present id → first vblank shown.
        shown: Vec<(u32, i64)>,
        presents: u32,
        last_v: i64,
    }

    impl Display {
        fn new() -> Display {
            Display {
                period: HZ180,
                shown: Vec::new(),
                presents: 0,
                last_v: -1,
            }
        }

        fn time(&self, v: i64) -> i64 {
            self.period.times(v)
        }

        /// Presents at `t` with sync interval `k`; returns the present id.
        fn present(&mut self, t: i64, k: i64, slow: i64) -> u32 {
            self.presents += 1;
            let earliest = Grid::new(0, 0, self.period).next_after(t) + slow;
            let v = if self.last_v < 0 {
                earliest
            } else {
                earliest.max(self.last_v + k)
            };
            self.last_v = v;
            self.shown.push((self.presents, v));
            self.presents
        }

        /// The statistics at time `t`: the newest present on screen.
        fn stats(&self, t: i64) -> Stats {
            let now_v = Grid::new(0, 0, self.period).next_after(t) - 1;
            let (id, v) = self
                .shown
                .iter()
                .rev()
                .find(|(_, v)| *v <= now_v)
                .copied()
                .unwrap_or((0, 0));
            Stats {
                present_count: id,
                present_refresh: v as u32,
                sync_refresh: now_v as u32,
                sync_hns: self.time(now_v),
            }
        }
    }

    /// Runs `frames` frames: the loop wakes `wake(i)` hns after each vblank it waits for, reads
    /// statistics, draws and presents, and the GPU is `slow(n)` vblanks late with frame n.
    /// Returns the records and the skipped frames.
    fn run_vsync(
        k: i64,
        frames: usize,
        wake: impl Fn(usize) -> i64,
        slow: impl Fn(u64) -> i64,
    ) -> (Vec<Resolved>, Vec<u64>) {
        let mut d = Display::new();
        let start = 2 * HNS_PER_SECOND / 10;
        let mut pacer = VsyncPacer::new(k as u32, HZ180, start);
        let mut out = Vec::new();
        let mut skipped = Vec::new();
        let mut v = 0_i64;
        let mut drawn = 0;
        let mut i = 0;
        while drawn < frames {
            let now = d.time(v) + wake(i);
            i += 1;
            if d.presents > 0 {
                out.extend(pacer.on_stats(d.stats(now)));
            } else {
                pacer.on_dwm_vblank(v as u64, d.time(v));
            }
            match pacer.next(now) {
                Step::Warmup => {
                    d.present(now, 1, 0);
                }
                Step::Frame {
                    frame,
                    target_hns,
                    skipped: s,
                } => {
                    skipped.extend(s);
                    let interval = if frame == 0 { 1 } else { k };
                    let id = d.present(now, interval, slow(frame));
                    pacer.presented(frame, target_hns, id, now + 1_000);
                    drawn += 1;
                }
            }
            // The waitable object signals once the last present is on screen.
            v = d.last_v;
        }
        let end = d.time(v + 4 * k);
        out.extend(pacer.on_stats(d.stats(end)));
        out.extend(pacer.finish());
        (out, skipped)
    }

    /// `a` and `b` within 1 hns: the grid is re-anchored on each reading, so a vblank's time can
    /// round either way.
    fn near(a: i64, b: i64) -> bool {
        (a - b).abs() <= 1
    }

    #[test]
    fn steady_vsync_numbers_frames_by_vblank_group() {
        for k in [1, 2, 3] {
            let (out, skipped) = run_vsync(k, 200, |_| 20_000, |_| 0);
            assert!(skipped.is_empty(), "k {k}");
            assert_eq!(out.len(), 200, "k {k}");
            let first = out[0].record;
            assert_eq!(first.frame, 0);
            // Frame 0 is drawn for the first vblank at or after the simulated start (0.2 s):
            // vblank 36 at 180 Hz.
            assert!(near(first.target_hns, HZ180.times(36)));
            for (n, r) in out.iter().enumerate() {
                assert_eq!(r.record.frame, n as u64, "k {k}");
                assert!(!r.missed, "k {k} frame {n}");
                let want = HZ180.times(36 + k * n as i64);
                assert!(near(r.record.target_hns, want), "k {k} frame {n}");
                let shown = r.record.display_hns.unwrap();
                assert!(near(shown, want), "k {k} frame {n}");
            }
        }
    }

    #[test]
    fn a_stall_skips_frames_and_keeps_time() {
        // At 180 fps the loop stalls 3.5 periods once.
        let stall = |i: usize| {
            if i == 52 { 3 * 55_556 + 30_000 } else { 20_000 }
        };
        let (out, skipped) = run_vsync(1, 120, stall, |_| 0);
        // The three vblanks that went by are three frames never drawn, each an `m` line.
        assert_eq!(skipped.len(), 3, "{skipped:?}");
        let frames: Vec<u64> = out.iter().map(|r| r.record.frame).collect();
        for s in &skipped {
            assert!(!frames.contains(s));
        }
        assert_eq!(
            frames.len() + skipped.len(),
            *frames.last().unwrap() as usize + 1
        );
        // Every frame drawn is shown at its own vblank group, so none of them was late.
        assert!(out.iter().all(|r| !r.missed));
        for r in &out {
            let shown = r.record.display_hns.unwrap();
            assert!(near(shown, r.record.target_hns), "frame {}", r.record.frame);
        }
    }

    #[test]
    fn a_frame_shown_late_is_marked() {
        for k in [1, 2] {
            // The GPU is a vblank late with frame 40, so it misses its vblank; the cadence then
            // carries on k vblanks after it.
            let slow = |n| i64::from(n == 40) * k;
            let (out, skipped) = run_vsync(k, 80, |_| 20_000, slow);
            let late: Vec<u64> = out
                .iter()
                .filter(|r| r.missed)
                .map(|r| r.record.frame)
                .collect();
            assert_eq!(late, [40], "k {k}; skipped {skipped:?}");
            // At k = 1 frame 40 took frame 41's vblank; at k = 2 the next frame keeps its number
            // in the new phase.
            let want: &[u64] = if k == 1 { &[41] } else { &[] };
            assert_eq!(skipped, want, "k {k}");
            let numbers: Vec<u64> = out.iter().map(|r| r.record.frame).collect();
            assert!(numbers.windows(2).all(|w| w[0] < w[1]));
            // Frames after it are shown where they were drawn for.
            for r in out.iter().filter(|r| r.record.frame > 41) {
                let shown = r.record.display_hns.unwrap();
                assert!(
                    near(shown, r.record.target_hns),
                    "k {k} frame {}",
                    r.record.frame
                );
            }
        }
    }

    #[test]
    fn frames_the_statistics_skip_are_filled_in_on_cadence() {
        let mut pacer = VsyncPacer::new(1, HZ180, 0);
        pacer.on_dwm_vblank(0, 0);
        let mut targets = Vec::new();
        for i in 0..4_u32 {
            let Step::Frame {
                frame, target_hns, ..
            } = pacer.next(HZ180.times(i64::from(i)) - 1)
            else {
                panic!("warm-up")
            };
            targets.push(target_hns);
            pacer.presented(frame, target_hns, 10 + i, target_hns - 1);
        }
        // Frames 0–3 are presents 10–13, shown at vblanks 0–3; the statistics catch 10 and 13.
        let stats = |id: u32, v: u32| Stats {
            present_count: id,
            present_refresh: v,
            sync_refresh: v,
            sync_hns: HZ180.times(i64::from(v)),
        };
        let first = pacer.on_stats(stats(10, 0));
        assert_eq!(first.len(), 1);
        let rest = pacer.on_stats(stats(13, 3));
        let shown: Vec<Option<i64>> = rest.iter().map(|r| r.record.display_hns).collect();
        assert_eq!(
            shown,
            [Some(targets[1]), Some(targets[2]), Some(targets[3])]
        );
        assert!(rest.iter().all(|r| !r.missed));
    }

    #[test]
    fn a_gap_the_statistics_skip_leaves_times_unknown_and_marks_the_next() {
        let mut pacer = VsyncPacer::new(1, HZ180, 0);
        pacer.on_dwm_vblank(0, 0);
        for i in 0..3_u32 {
            let Step::Frame {
                frame, target_hns, ..
            } = pacer.next(HZ180.times(i64::from(i)) - 1)
            else {
                panic!("warm-up")
            };
            pacer.presented(frame, target_hns, 1 + i, target_hns);
        }
        let stats = |id: u32, v: u32| Stats {
            present_count: id,
            present_refresh: v,
            sync_refresh: v,
            sync_hns: HZ180.times(i64::from(v)),
        };
        assert_eq!(pacer.on_stats(stats(1, 0)).len(), 1);
        // Present 3 first appears at vblank 4, not 2: present 2 was late or dropped.
        let rest = pacer.on_stats(stats(3, 4));
        assert_eq!(rest.len(), 2);
        assert_eq!(rest[0].record.display_hns, None);
        assert!(!rest[0].missed);
        assert!(rest[1].missed);
        assert!(pacer.finish().is_empty());
    }

    #[test]
    fn without_a_grid_the_vsync_pacer_warms_up() {
        let mut pacer = VsyncPacer::new(1, HZ180, 0);
        assert_eq!(pacer.next(0), Step::Warmup);
        // Frames never reported get unknown display times at the end.
        pacer.on_dwm_vblank(0, 0);
        let Step::Frame {
            frame, target_hns, ..
        } = pacer.next(1)
        else {
            panic!("warm-up")
        };
        pacer.presented(frame, target_hns, 1, 2);
        let end = pacer.finish();
        assert_eq!(end.len(), 1);
        assert_eq!(end[0].record.display_hns, None);
    }

    #[test]
    fn timer_frames_get_display_times_and_keep_their_slot_misses() {
        let mut shown = Shown::new(None, HZ180);
        // 360 fps on 180 Hz: frames 0–3 are presents 1–4; the display shows 2 and 4.
        for f in 0..4_u32 {
            shown.presented(u64::from(f), i64::from(f), f + 1, i64::from(f), f == 2);
        }
        let stats = |id: u32, v: u32| Stats {
            present_count: id,
            present_refresh: v,
            sync_refresh: v,
            sync_hns: HZ180.times(i64::from(v)),
        };
        let mut out = shown.on_stats(stats(2, 10));
        out.extend(shown.on_stats(stats(4, 11)));
        let shown_at: Vec<Option<i64>> = out.iter().map(|r| r.record.display_hns).collect();
        assert_eq!(
            shown_at,
            [None, Some(HZ180.times(10)), None, Some(HZ180.times(11))]
        );
        // Frames passed over aren't late; the slot it missed stays marked.
        let missed: Vec<bool> = out.iter().map(|r| r.missed).collect();
        assert_eq!(missed, [false, false, true, false]);
    }

    #[test]
    fn timer_slots_follow_the_deadline_rounding() {
        let t = TimerPacer::new(1_000, 240);
        assert_eq!(t.slot_time(0), 1_000);
        // 10⁷ / 240 = 41,666.67 → the next whole 100 ns.
        assert_eq!(t.slot_time(1), 1_000 + 41_667);
        assert_eq!(t.slot_time(240), 1_000 + 10_000_000);
        for now in (0..20_000_000).step_by(1_009) {
            let s = t.slot_at(now);
            if now >= 1_000 {
                assert!(
                    t.slot_time(s) <= now && t.slot_time(s + 1) > now,
                    "now {now}"
                );
            } else {
                assert_eq!(s, 0);
            }
        }
    }

    #[test]
    fn timer_frames_skip_slots_that_have_gone_by() {
        let mut t = TimerPacer::new(0, 360);
        assert_eq!(
            t.next(-5),
            Step::Frame {
                frame: 0,
                target_hns: 0,
                skipped: 0..0
            }
        );
        assert!(!t.missed(0, 27_000));
        assert!(t.missed(0, 27_778));
        // On time for slot 1.
        let Step::Frame { frame, skipped, .. } = t.next(27_000) else {
            panic!()
        };
        assert_eq!((frame, skipped), (1, 1..1));
        // Woken in slot 5: slots 2–4 are skipped and 5 is drawn now.
        let Step::Frame {
            frame,
            target_hns,
            skipped,
        } = t.next(t.slot_time(5) + 10)
        else {
            panic!()
        };
        assert_eq!((frame, skipped), (5, 2..5));
        assert_eq!(target_hns, t.slot_time(5));
    }
}
