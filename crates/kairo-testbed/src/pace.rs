// SPDX-License-Identifier: GPL-3.0-or-later
//! Pacing and frame numbering (docs/research/16-verify-testbed/README.md §4.3).
//!
//! Frame number N is time-based, the way a game's clock keeps running: N is the vblank group
//! (vsync pacing) or the timer slot (timer pacing) the frame is drawn for, counted from frame 0's
//! anchor, the first vblank or slot at or after a whole QPC second. A frame that can't make its
//! vblank or slot shows up as a skipped N plus an `m` line, so a testbed miss is never mistaken
//! for a capture drop.
//!
//! Everything here is pure: the Windows loop passes in QPC times (100 ns), the swap chain's frame
//! statistics and the monitor's own vblank times, and gets back what to draw and which log
//! records to write.
//!
//! **Vsync.** With the flip model, a present's sync interval is how long that frame stays on
//! screen: the next frame can show no earlier than k vblanks after it (Microsoft's
//! `IDXGISwapChain::Present`). So every frame is presented with interval k, and frame 0 follows
//! warm-up frames of interval 1, the last of which shows the vblank before the anchor.
//! [`VsyncPacer`] keeps frame 0's anchor and every target as QPC times, never as vblank counts:
//! DXGI counts vblanks in one domain (from boot in full screen, from the swap chain's creation
//! in a window) and the monitor's own vblank clock in another, so counts from two sources never
//! meet. Each frame is aimed k vblanks after the vblank the previous frame was taken at, or at
//! the first vblank still reachable when that one has gone by, and is numbered by the vblank
//! group its target falls in. The vblanks are counted step by step from frame to frame, so a
//! panel a little off its reported rate doesn't drift the count. When the previous frame was
//! taken is known from the newest frame the statistics report: its vblank is taken from its
//! predecessor's by DXGI's own count of the vblanks between their displays, which a composed
//! window's latency (a frame shows a vblank after it is taken) doesn't change, and the frames
//! after it follow k vblanks apart. Without a frame-latency waitable object (exclusive
//! fullscreen) the loop plans while the frame it just presented is still queued, and the pacer
//! counts that frame's vblank as taken.
//!
//! Statistics report only the newest frame on screen and the vblank it first appeared at
//! ([`Shown`]). A frame shown later than k vblanks after the frame before it, by DXGI's own
//! vblank counts, missed its vblank. A frame the statistics never reported is filled in from its
//! neighbours when the two of them were exactly on cadence, since each frame stays at least k
//! vblanks; otherwise its display time is unknown (`-`) and the next frame shown is the late
//! one.
//!
//! **Timer.** [`TimerPacer`] presents frame N at slot N's time, `start + ⌈N × 10⁷ / fps⌉`
//! (`kairo_core::timeline::deadline_hns`'s rounding, at any rate). A frame whose `Present`
//! returns after its slot ends missed it.

use std::collections::VecDeque;
use std::ops::Range;

use kairo_core::testbed_log::FrameRecord;
use kairo_core::timeline::HNS_PER_SECOND;

/// The longest stall, in seconds, before the pacer takes its clock for lost: a real stall that
/// long is a run to redo, not hundreds of thousands of `m` lines.
pub const LOST_AFTER_S: i64 = 3_600;

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

    /// The periods in `d` 100 ns, rounded to nearest (halves up).
    pub fn nearest(self, d: i64) -> i64 {
        saturate((2 * i128::from(d) * self.den + self.num).div_euclid(2 * self.num))
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
/// Counts mean something only within one grid's source; times mean the same in all of them.
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

    /// The time of the vblank nearest `t_hns`.
    pub fn snap(&self, t_hns: i64) -> i64 {
        let half = self.period.times(1) / 2;
        self.time_of(self.at_or_after(t_hns.saturating_sub(half)))
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
    /// The pacer's clock jumped by more than [`LOST_AFTER_S`] of frames: the run can't go on.
    Lost,
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

/// A frame the statistics reported on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reported {
    /// N.
    pub frame: u64,
    /// The vblank it first appeared at, in DXGI's count.
    vblank: i64,
    /// When it first appeared.
    pub shown_hns: i64,
    /// The vblank it was taken at: its display less a composed window's latency.
    pub taken_hns: i64,
    /// When its `Present` returned.
    present_hns: i64,
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
    /// The last frame reported on screen.
    last_shown: Option<Reported>,
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

    /// The last frame reported on screen.
    pub fn last_shown(&self) -> Option<Reported> {
        self.last_shown
    }

    /// The display's latency, from the newest frame reported: how long after the first vblank
    /// following its present it showed (a composed window shows a frame a vblank after it
    /// takes it). 0 before a report.
    pub fn latency_hns(&self) -> i64 {
        match (self.last_shown, self.grid) {
            (Some(r), Some(g)) => (r.shown_hns - g.time_of(g.next_after(r.present_hns))).max(0),
            _ => 0,
        }
    }

    /// The statistics went away and came back (`DXGI_ERROR_FRAME_STATISTICS_DISJOINT`, as after
    /// a change between full screen and a window): their vblank counts may now be in another
    /// domain, so no count is compared across the change. The grid's times stay valid until
    /// the next reading replaces it.
    pub fn disjoint(&mut self) {
        self.last_shown = None;
        self.shown_vblanks = Widen::default();
        self.sync_vblanks = Widen::default();
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

    /// Whether frames still wait for statistics.
    pub fn waiting(&self) -> bool {
        !self.pending.is_empty() || !self.unseen.is_empty()
    }

    /// The newest frame presented and the vblank it was taken at: from the newest frame
    /// reported, each frame presented after it was taken at its target or `k_hns` after the one
    /// before it, whichever is later (the flip model's rule). `None` before any report.
    pub fn newest_taken(&self, k_hns: i64) -> Option<(u64, i64)> {
        let r = self.last_shown?;
        let mut taken = (r.frame, r.taken_hns);
        for p in &self.pending {
            taken = (p.frame, p.target_hns.max(taken.1.saturating_add(k_hns)));
        }
        Some(taken)
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
            let shown_t = grid.time_of(shown_v);
            let mut late = false;
            let unseen = std::mem::take(&mut self.unseen);
            match (self.k, self.last_shown) {
                (Some(k), Some(r)) => {
                    let cadence = k * (p.frame - r.frame) as i64;
                    if shown_v - r.vblank == cadence {
                        // Every frame between them stayed exactly k vblanks.
                        for u in unseen {
                            let at = r.vblank + k * (u.frame - r.frame) as i64;
                            out.push(resolved(u, Some(grid.time_of(at)), false));
                        }
                    } else {
                        out.extend(unseen.into_iter().map(|u| resolved(u, None, false)));
                        late = shown_v - r.vblank > cadence;
                    }
                }
                _ => out.extend(unseen.into_iter().map(|u| resolved(u, None, false))),
            }
            // Taken as many vblanks after its predecessor as it showed after it; the first frame
            // reported is taken to have been on time, unless it showed before its target.
            let taken_hns = match self.last_shown {
                Some(r) => r.taken_hns + self.period.times(shown_v - r.vblank),
                None => p.target_hns.min(shown_t),
            };
            out.push(resolved(p, Some(shown_t), late));
            self.last_shown = Some(Reported {
                frame: p.frame,
                vblank: shown_v,
                shown_hns: shown_t,
                taken_hns,
                present_hns: p.present_hns,
            });
        }
        out
    }

    /// The frames presented more than a second before `now_hns` that the statistics still
    /// haven't reported: their display times are unknown (statistics that stopped, or none).
    pub fn expire(&mut self, now_hns: i64) -> Vec<Resolved> {
        let cutoff = now_hns.saturating_sub(HNS_PER_SECOND);
        let old = self
            .unseen
            .iter()
            .take_while(|u| u.present_hns < cutoff)
            .count();
        let mut out: Vec<Resolved> = self
            .unseen
            .drain(..old)
            .map(|u| resolved(u, None, false))
            .collect();
        if self.unseen.is_empty() {
            while let Some(p) = self.pending.pop_front_if(|p| p.present_hns < cutoff) {
                out.push(resolved(p, None, false));
            }
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

/// The last frame a vsync pacer drew.
#[derive(Debug, Clone, Copy)]
struct Drawn {
    frame: u64,
    /// The vblank it was drawn for.
    target_hns: i64,
    /// That vblank, counted from frame 0's anchor.
    index: i64,
}

/// Vsync pacing: one frame every k vblanks (README §4.3).
#[derive(Debug, Clone)]
pub struct VsyncPacer {
    k: i64,
    period: Period,
    /// The whole second frame 0 is anchored at.
    start_hns: i64,
    /// Whether the loop plans while the frame it presented last is still queued (`Present`
    /// blocks, as in exclusive fullscreen, rather than a waitable object).
    blocking: bool,
    /// The earliest vblank the next frame can take behind a frame still queued: that frame's
    /// vblank plus its interval.
    queued: Option<i64>,
    /// The monitor's own vblank clock, for when DXGI has no statistics.
    clock: Option<Grid>,
    /// Frame 0's vblank, once a grid has placed it.
    t0: Option<i64>,
    last: Option<Drawn>,
    shown: Shown,
}

impl VsyncPacer {
    /// A pacer for one frame every `k` vblanks of `period`, frame 0 at the first vblank at or
    /// after `start_hns` (a whole second); if no grid came until that vblank had gone by, at the
    /// first vblank at or after the next whole second. `blocking`: the loop plans right after a
    /// `Present` that queued its frame, instead of after a waitable object said it was taken.
    pub fn new(k: u32, period: Period, start_hns: i64, blocking: bool) -> VsyncPacer {
        VsyncPacer {
            k: i64::from(k.max(1)),
            period,
            start_hns,
            blocking,
            queued: None,
            clock: None,
            t0: None,
            last: None,
            shown: Shown::new(Some(k), period),
        }
    }

    /// The interval to present every frame with, frame 0 included (the warm-up uses 1).
    pub fn interval(&self) -> u32 {
        u32::try_from(self.k).unwrap_or(1)
    }

    /// Frame 0's vblank, once known: the log header's `start_hns`.
    pub fn anchor(&self) -> Option<i64> {
        self.t0
    }

    /// A vblank of the monitor's own (`IDXGIOutput::WaitForVBlank`): the grid while DXGI's
    /// statistics have none.
    pub fn on_vblank(&mut self, t_hns: i64) {
        self.clock = Some(Grid::new(0, t_hns, self.period));
    }

    /// A statistics reading: updates the grid and returns the frames it settles, in order.
    pub fn on_stats(&mut self, stats: Stats) -> Vec<Resolved> {
        self.shown.on_stats(stats)
    }

    /// The statistics were disjoint: see [`Shown::disjoint`].
    pub fn disjoint(&mut self) {
        self.shown.disjoint();
    }

    /// The frames on their way to the screen.
    pub fn shown(&mut self) -> &mut Shown {
        &mut self.shown
    }

    /// The grid: DXGI's, else the monitor's clock, else, once a frame is out, one through the
    /// last target, so the run never goes back to warm-up.
    fn grid(&self) -> Option<Grid> {
        self.shown
            .grid()
            .or(self.clock)
            .or_else(|| self.last.map(|d| Grid::new(0, d.target_hns, self.period)))
    }

    /// When the last frame drawn was taken: at least the vblank it was drawn for, and as late as
    /// the frames presented since the newest one reported say (see [`Shown::newest_taken`]).
    fn last_taken(&self) -> Option<(u64, i64)> {
        let last = self.last?;
        let taken = match self.shown.newest_taken(self.period.times(self.k)) {
            Some((frame, taken)) if frame == last.frame => last.target_hns.max(taken),
            _ => last.target_hns,
        };
        Some((last.frame, taken))
    }

    /// The frames an hour of this pacing holds.
    fn lost_after(&self) -> u64 {
        let vblanks = self.period.count(LOST_AFTER_S * HNS_PER_SECOND);
        u64::try_from(vblanks / self.k).unwrap_or(u64::MAX)
    }

    /// What to draw now, at `now_hns`.
    pub fn next(&mut self, now_hns: i64) -> Step {
        let Some(grid) = self.grid() else {
            return Step::Warmup;
        };
        let mut reachable = grid.next_after(now_hns);
        if let Some(queued) = self.queued.filter(|_| self.blocking) {
            reachable = reachable.max(grid.at_or_after(queued - self.period.times(1) / 2));
        }
        let earliest = grid.time_of(reachable);
        let t0 = match self.t0 {
            Some(t0) => t0,
            None => {
                let mut v = grid.at_or_after(self.start_hns);
                if v < reachable {
                    v = grid.at_or_after(next_whole_second(earliest));
                }
                *self.t0.insert(grid.time_of(v))
            }
        };
        let (target, previous) = match self.last_taken() {
            // Warm-up frames of interval 1 show until the anchor's vblank is next; a late start
            // takes the first vblank still reachable, and the frames before it are skipped.
            None if earliest < t0 - self.period.times(1) / 2 => return Step::Warmup,
            None => (earliest, None),
            Some((previous, taken)) => {
                let want = grid.snap(taken.saturating_add(self.period.times(self.k)));
                (want.max(earliest), Some(previous))
            }
        };
        // The vblanks since frame 0, counted step by step: exact for steps of a few periods
        // however far the panel's rate is from the one it reports.
        let index = match self.last {
            Some(d) => d.index + self.period.nearest(target - d.target_hns),
            None => self.period.nearest(target - t0),
        };
        let group = u64::try_from((2 * index + self.k).div_euclid(2 * self.k)).unwrap_or(0);
        let first = previous.map_or(0, |p| p + 1);
        let frame = group.max(first);
        if frame - first > self.lost_after() {
            return Step::Lost;
        }
        self.last = Some(Drawn {
            frame,
            target_hns: target,
            index,
        });
        Step::Frame {
            frame,
            target_hns: target,
            skipped: first..frame,
        }
    }

    /// A black warm-up frame was presented with interval 1, and `Present` returned at
    /// `present_hns`.
    pub fn warmup_presented(&mut self, present_hns: i64) {
        if let Some(grid) = self.grid().filter(|_| self.blocking) {
            let taken = grid.time_of(grid.next_after(present_hns));
            self.queued = Some(taken + self.period.times(1));
        }
    }

    /// Frame `frame` (drawn for `target_hns`) was presented as present `present_id`
    /// (`GetLastPresentCount`), and `Present` returned at `present_hns`.
    pub fn presented(&mut self, frame: u64, target_hns: i64, present_id: u32, present_hns: i64) {
        if self.blocking {
            self.queued = Some(target_hns + self.period.times(self.k));
        }
        self.shown
            .presented(frame, target_hns, present_id, present_hns, false);
    }

    /// When a future frame should first show, on the cadence as it stands (README §4.7: the last
    /// known vblank plus whole refresh periods): k vblanks a frame after the vblank the last
    /// frame drawn was taken at, plus the display's latency, which the newest frame reported
    /// shows (its display less the vblank it was taken at). `None` before frame 0 or for a
    /// frame already drawn.
    pub fn predict(&self, frame: u64) -> Option<i64> {
        let grid = self.grid()?;
        let (previous, taken) = self.last_taken()?;
        let ahead = i64::try_from(frame.checked_sub(previous)?)
            .ok()
            .filter(|&n| n > 0)?;
        let target =
            grid.snap(taken.saturating_add(self.period.times(ahead.saturating_mul(self.k))));
        let latency = self
            .shown
            .last_shown()
            .map_or(0, |r| (r.shown_hns - r.taken_hns).max(0));
        Some(target.saturating_add(latency))
    }

    /// Whether frames still wait for statistics.
    pub fn waiting(&self) -> bool {
        self.shown.waiting()
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

/// The monitor's vblank grid from the times a thread wakes after each
/// `IDXGIOutput::WaitForVBlank`, for runs without frame statistics (README §4.9). A wake comes
/// after its vblank by however long the scheduler takes, and on a busy PC often by much more, so
/// the grid runs through the earliest wake of the densest eighth of a period among the last
/// second's wake phases: the prompt wakes cluster just after the vblank, however many late
/// ones there are.
#[derive(Debug, Clone)]
pub struct WakeGrid {
    period: Period,
    /// A vblank time the grid runs through.
    base: Option<i64>,
    /// The last second's wakes.
    wakes: VecDeque<i64>,
}

impl WakeGrid {
    /// A grid for a display of `period`.
    pub fn new(period: Period) -> WakeGrid {
        WakeGrid {
            period,
            base: None,
            wakes: VecDeque::new(),
        }
    }

    /// A wake at `t_hns`; returns the time of the grid's last vblank at or before it.
    pub fn on_wake(&mut self, t_hns: i64) -> i64 {
        self.wakes.push_back(t_hns);
        while self
            .wakes
            .front()
            .is_some_and(|&w| t_hns - w > HNS_PER_SECOND)
        {
            self.wakes.pop_front();
        }
        let origin = self.base.unwrap_or(t_hns);
        let period = self.period.times(1);
        // Each wake's phase in the period, from the old base.
        let mut phases: Vec<i64> = self
            .wakes
            .iter()
            .map(|&w| {
                let d = w - origin;
                d - self.period.times(self.period.count(d))
            })
            .collect();
        phases.sort_unstable();
        // The window of an eighth of a period, wrapping round, that holds the most phases.
        let n = phases.len();
        let at = |j: usize| phases[j % n] + period * (j / n) as i64;
        let (mut best, mut start, mut j) = (0, phases.first().copied().unwrap_or(0), 0);
        for (i, &from) in phases.iter().enumerate() {
            j = j.max(i);
            while j < i + n && at(j) - from < period / 8 {
                j += 1;
            }
            if j - i > best {
                (best, start) = (j - i, from);
            }
        }
        let base = origin + start;
        self.base = Some(base);
        base.saturating_add(self.period.times(self.period.count(t_hns - base)))
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
        let hour = u64::try_from(self.fps).unwrap_or(u64::MAX) * LOST_AFTER_S as u64;
        if frame - self.next > hour {
            return Step::Lost;
        }
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
        assert_eq!(p.nearest(27_783), 0);
        assert_eq!(p.nearest(27_784), 1);
        assert_eq!(p.nearest(-27_000), 0);
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
        assert_eq!(g.snap(1_020_000), 1_000_000);
        assert_eq!(g.snap(1_030_000), 1_055_556);
        // Exhaustively, over a few seconds: next_after is the first vblank later than t.
        for t in (0..30_000_000).step_by(997) {
            let v = g.next_after(t);
            assert!(g.time_of(v) > t && g.time_of(v - 1) <= t, "t {t}");
        }
    }

    /// A display that follows the flip model's rule: a frame is taken at the first vblank after
    /// it is presented, but no earlier than the previous frame's vblank plus the previous
    /// present's interval, and `slow` vblanks later when the GPU is late with it. A composed
    /// window shows it `latency(v)` vblanks after it takes it at vblank v. Its vblanks come every
    /// `period`, which may differ from the rate it reports (the pacer's). DXGI's statistics count
    /// vblanks from `dxgi_offset`; the monitor's own clock from 0.
    struct Display {
        period: Period,
        latency: Box<dyn Fn(i64) -> i64>,
        dxgi_offset: i64,
        /// Present id → the vblank it was taken at.
        taken: Vec<(u32, i64)>,
        presents: u32,
        last_v: i64,
        last_interval: i64,
    }

    impl Display {
        fn new(latency: i64, dxgi_offset: i64) -> Display {
            Display::with(HZ180, Box::new(move |_| latency), dxgi_offset)
        }

        fn with(period: Period, latency: Box<dyn Fn(i64) -> i64>, dxgi_offset: i64) -> Display {
            Display {
                period,
                latency,
                dxgi_offset,
                taken: Vec::new(),
                presents: 0,
                last_v: -1,
                last_interval: 1,
            }
        }

        fn time(&self, v: i64) -> i64 {
            self.period.times(v)
        }

        /// The last vblank at or before `t`.
        fn vblank_at(&self, t: i64) -> i64 {
            Grid::new(0, 0, self.period).next_after(t) - 1
        }

        /// Presents at `t` with sync interval `interval`; returns the present id.
        fn present(&mut self, t: i64, interval: i64, slow: i64) -> u32 {
            self.presents += 1;
            let earliest = self.vblank_at(t) + 1 + slow;
            let v = if self.last_v < 0 {
                earliest
            } else {
                earliest.max(self.last_v + self.last_interval)
            };
            self.last_v = v;
            self.last_interval = interval;
            self.taken.push((self.presents, v));
            self.presents
        }

        /// The statistics at time `t`: the newest present on screen, in DXGI's counts; none
        /// until the first present shows.
        fn stats(&self, t: i64) -> Option<Stats> {
            let now_v = self.vblank_at(t);
            let (id, v) = self
                .taken
                .iter()
                .rev()
                .find(|(_, v)| v + (self.latency)(*v) <= now_v)
                .copied()?;
            Some(Stats {
                present_count: id,
                present_refresh: (v + (self.latency)(v) + self.dxgi_offset) as u32,
                sync_refresh: (now_v + self.dxgi_offset) as u32,
                sync_hns: self.time(now_v),
            })
        }
    }

    /// What a vsync run did: the records, the frames skipped, and frame 0's anchor.
    struct Run {
        out: Vec<Resolved>,
        skipped: Vec<u64>,
        anchor: i64,
    }

    /// How the loop waits for the swap chain before it plans a frame.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Wait {
        /// The waitable object: the loop wakes `wake(i)` hns after the vblank the last present
        /// was taken at.
        Waitable,
        /// A blocking `Present` (exclusive fullscreen): it returns once its frame is queued,
        /// which is when the frame before it was taken, and the loop plans `wake(i)` hns after.
        Blocking,
    }

    /// Runs `frames` frames on `d` with the waitable object; see [`run_loop`].
    fn run_vsync(
        k: i64,
        frames: usize,
        d: Display,
        start: i64,
        wake: impl Fn(usize) -> i64,
        slow: impl Fn(u64) -> i64,
    ) -> Run {
        run_loop(Wait::Waitable, k, frames, d, start, wake, slow)
    }

    /// Runs `frames` frames on `d`: each pass reads statistics (and the monitor's own clock
    /// until they come), asks the pacer, draws and presents; the GPU is `slow(n)` vblanks late
    /// with frame n.
    fn run_loop(
        wait: Wait,
        k: i64,
        frames: usize,
        mut d: Display,
        start: i64,
        wake: impl Fn(usize) -> i64,
        slow: impl Fn(u64) -> i64,
    ) -> Run {
        let mut pacer = VsyncPacer::new(k as u32, HZ180, start, wait == Wait::Blocking);
        let mut out = Vec::new();
        let mut skipped = Vec::new();
        let mut now = wake(0);
        let mut drawn = 0;
        let mut i = 0;
        while drawn < frames {
            match d.stats(now) {
                Some(stats) => out.extend(pacer.on_stats(stats)),
                None => pacer.on_vblank(d.time(d.vblank_at(now))),
            }
            // A blocking Present queues its frame once the one before it is taken.
            let queued = match wait {
                Wait::Waitable => now,
                Wait::Blocking if d.last_v >= 0 => now.max(d.time(d.last_v)),
                Wait::Blocking => now,
            };
            match pacer.next(now) {
                Step::Warmup => {
                    d.present(queued, 1, 0);
                    pacer.warmup_presented(queued + 1_000);
                }
                Step::Frame {
                    frame,
                    target_hns,
                    skipped: s,
                } => {
                    skipped.extend(s);
                    let id = d.present(queued, i64::from(pacer.interval()), slow(frame));
                    pacer.presented(frame, target_hns, id, queued + 1_000);
                    drawn += 1;
                }
                Step::Lost => panic!("lost the clock"),
            }
            i += 1;
            now = match wait {
                Wait::Waitable => d.time(d.last_v) + wake(i),
                Wait::Blocking => queued + 1_000 + wake(i),
            };
        }
        let v = d.last_v;
        for extra in 1..=6 {
            if let Some(stats) = d.stats(d.time(v + extra * k + 2)) {
                out.extend(pacer.on_stats(stats));
            }
        }
        out.extend(pacer.finish());
        Run {
            out,
            skipped,
            anchor: pacer.anchor().unwrap(),
        }
    }

    /// `a` and `b` within 2 hns: each grid's vblank times are rounded once from the reading and
    /// once from the period, and a target and a display time come from different readings.
    fn near(a: i64, b: i64) -> bool {
        (a - b).abs() <= 2
    }

    #[test]
    fn steady_vsync_numbers_frames_by_vblank_group() {
        for latency in [0, 1] {
            for k in [1, 2, 3, 4] {
                let start = 2 * HNS_PER_SECOND / 10;
                let run = run_vsync(k, 200, Display::new(latency, 0), start, |_| 20_000, |_| 0);
                assert!(run.skipped.is_empty(), "k {k} latency {latency}");
                assert_eq!(run.out.len(), 200, "k {k} latency {latency}");
                // Frame 0 at the first vblank at or after the start: vblank 36 at 180 Hz.
                assert!(near(run.anchor, HZ180.times(36)));
                for (n, r) in run.out.iter().enumerate() {
                    let what = format!("k {k} latency {latency} frame {n}");
                    assert_eq!(r.record.frame, n as u64, "{what}");
                    assert!(!r.missed, "{what}");
                    let want = HZ180.times(36 + k * n as i64);
                    assert!(near(r.record.target_hns, want), "{what}");
                    let shown = r.record.display_hns.unwrap();
                    let latency_hns = HZ180.times(36 + k * n as i64 + latency);
                    assert!(near(shown, latency_hns), "{what}");
                }
            }
        }
    }

    #[test]
    fn a_stall_skips_frames_numbers_by_group_and_keeps_showing_on_target() {
        // The loop stalls 3.5 vblanks once, after frame 24 (taken at vblank V): for k ≤ 3 frame
        // 25's vblank V + k has gone by, so the pacer aims at V + 4. That is group 28 for k = 1
        // (25–27 skipped), 26 for k = 2 (25 skipped) and 25 for k = 3, shown a vblank late;
        // k = 4 isn't held up.
        let cases: [(i64, &[u64], &[u64]); 4] = [
            (1, &[25, 26, 27], &[]),
            (2, &[25], &[]),
            (3, &[], &[25]),
            (4, &[], &[]),
        ];
        for latency in [0, 1] {
            for (k, skipped, late) in cases {
                let stall = |i: usize| {
                    if i == 60 { 3 * 55_556 + 30_000 } else { 20_000 }
                };
                let start = 2 * HNS_PER_SECOND / 10;
                let run = run_vsync(k, 120, Display::new(latency, 0), start, stall, |_| 0);
                let what = format!("k {k} latency {latency}");
                assert_eq!(run.skipped, skipped, "{what}");
                let missed: Vec<u64> = run
                    .out
                    .iter()
                    .filter(|r| r.missed)
                    .map(|r| r.record.frame)
                    .collect();
                assert_eq!(missed, late, "{what}");
                let frames: Vec<u64> = run.out.iter().map(|r| r.record.frame).collect();
                for s in &run.skipped {
                    assert!(!frames.contains(s), "{what}");
                }
                assert!(frames.windows(2).all(|w| w[0] < w[1]), "{what}");
                for r in &run.out {
                    // Every frame shows where it was drawn for, and N is the vblank group of its
                    // target counted from frame 0.
                    let shown = r.record.display_hns.unwrap();
                    let latency_hns = HZ180.times(latency);
                    assert!(
                        near(shown, r.record.target_hns + latency_hns),
                        "{what}: {r:?}"
                    );
                    let vblanks = HZ180.nearest(r.record.target_hns - run.anchor);
                    let group = (2 * vblanks + k).div_euclid(2 * k);
                    assert_eq!(r.record.frame as i64, group, "{what}: {r:?}");
                }
            }
        }
    }

    #[test]
    fn a_frame_shown_late_is_marked() {
        for latency in [0, 1] {
            for (k, skips) in [(1, 1), (2, 1), (3, 0)] {
                // The GPU is slow with frame 40: it is taken a vblank after its target, and the
                // cadence carries on k vblanks after it. That vblank is a frame at k = 1, half a
                // group at k = 2 (one number skipped) and a third of one at k = 3 (none).
                let slow = |n| i64::from(n == 40) * k;
                let start = 2 * HNS_PER_SECOND / 10;
                let run = run_vsync(k, 80, Display::new(latency, 0), start, |_| 20_000, slow);
                let late: Vec<u64> = run
                    .out
                    .iter()
                    .filter(|r| r.missed)
                    .map(|r| r.record.frame)
                    .collect();
                let what = format!("k {k} latency {latency}");
                assert_eq!(late, [40], "{what}");
                let numbers: Vec<u64> = run.out.iter().map(|r| r.record.frame).collect();
                assert!(numbers.windows(2).all(|w| w[0] < w[1]));
                // Once the statistics show the late frame, the frames are aimed where they are
                // taken again.
                assert_eq!(run.skipped.len(), skips, "{what}: {:?}", run.skipped);
                for r in run
                    .out
                    .iter()
                    .filter(|r| r.record.frame < 40 || r.record.frame >= 44)
                {
                    let shown = r.record.display_hns.unwrap();
                    let latency_hns = HZ180.times(latency);
                    assert!(
                        near(shown, r.record.target_hns + latency_hns),
                        "{what}: {r:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn exclusive_frames_show_on_their_targets() {
        // A blocking Present: the loop plans while its last frame is still queued. Frame 0
        // takes the anchor's vblank and every frame shows on its target, also after a stall
        // of 3.5 vblanks (k = 1 skips the two frames it cost; at k = 2 and 3 the queued frame
        // covers it).
        for (k, skips) in [(1, 2), (2, 0), (3, 0)] {
            let stall = |i: usize| if i == 60 { 3 * 55_556 + 30_000 } else { 0 };
            let start = 2 * HNS_PER_SECOND / 10;
            let run = run_loop(
                Wait::Blocking,
                k,
                120,
                Display::new(0, 0),
                start,
                stall,
                |_| 0,
            );
            let what = format!("k {k}");
            assert!(near(run.anchor, HZ180.times(36)), "{what}");
            assert!(
                near(run.out[0].record.display_hns.unwrap(), run.anchor),
                "{what}"
            );
            assert_eq!(run.skipped.len(), skips, "{what}: {:?}", run.skipped);
            for r in &run.out {
                assert!(!r.missed, "{what}: {r:?}");
                let shown = r.record.display_hns.unwrap();
                assert!(near(shown, r.record.target_hns), "{what}: {r:?}");
            }
        }
    }

    #[test]
    fn a_panel_off_its_reported_rate_numbers_frames_without_skips() {
        // The panel runs 50 ppm slower than the 180 Hz it reports, as QPC's crystal may: over
        // 150 s (the harness's run) no frame number is skipped and none is late.
        let slow = Period {
            num: 10_000_000 * 100_005,
            den: 180 * 100_000,
        };
        for k in [1, 3] {
            let frames = 150 * 180 / k as usize;
            let d = Display::with(slow, Box::new(|_| 1), 0);
            let start = 2 * HNS_PER_SECOND / 10;
            let run = run_vsync(k, frames, d, start, |_| 20_000, |_| 0);
            assert!(run.skipped.is_empty(), "k {k}: {:?}", &run.skipped[..1]);
            assert!(run.out.iter().all(|r| !r.missed), "k {k}");
        }
    }

    #[test]
    fn composition_coming_and_going_moves_no_numbers_once_settled() {
        // A window shown composed (a vblank of latency), then by independent flip (none) from
        // vblank 120 to 199, then composed again; a wake 1.2 periods late after vblank 300.
        // Coming back to composition shows as one late frame; after that no number is skipped
        // or marked, and every frame shows the same time after its target.
        let k = 2;
        let latency = |v: i64| if (120..200).contains(&v) { 0 } else { 1 };
        let d = Display::with(HZ180, Box::new(latency), 0);
        let wake = |i: usize| if i == 167 { 66_667 } else { 20_000 };
        let start = 2 * HNS_PER_SECOND / 10;
        let run = run_vsync(k, 200, d, start, wake, |_| 0);
        assert!(run.skipped.iter().all(|&n| n < 100), "{:?}", run.skipped);
        let settled: Vec<&Resolved> = run.out.iter().filter(|r| r.record.frame >= 100).collect();
        assert!(settled.iter().all(|r| !r.missed));
        let offsets: Vec<i64> = settled
            .iter()
            .map(|r| HZ180.nearest(r.record.display_hns.unwrap() - r.record.target_hns))
            .collect();
        assert!(offsets.windows(2).all(|w| w[0] == w[1]), "{offsets:?}");
    }

    #[test]
    fn a_disjoint_reading_after_frame_0_keeps_drawing_frames() {
        // Statistics only, no monitor clock: a DISJOINT (exclusive fullscreen lost) mustn't
        // send the pacer back to black warm-up frames.
        let mut pacer = VsyncPacer::new(1, HZ180, 0, false);
        let stats = |id: u32, v: u32| Stats {
            present_count: id,
            present_refresh: v,
            sync_refresh: v,
            sync_hns: HZ180.times(i64::from(v)),
        };
        assert!(pacer.on_stats(stats(0, 0)).is_empty());
        for i in 0..3_u32 {
            let Step::Frame {
                frame, target_hns, ..
            } = pacer.next(HZ180.times(i64::from(i)) - 1)
            else {
                panic!("warm-up")
            };
            assert_eq!(frame, u64::from(i));
            pacer.presented(frame, target_hns, 1 + i, target_hns);
        }
        pacer.disjoint();
        let step = pacer.next(HZ180.times(3) - 1);
        assert!(matches!(step, Step::Frame { frame: 3, .. }), "{step:?}");
    }

    #[test]
    fn dxgi_and_the_monitor_clock_never_mix_counts() {
        // Review 8a-gpu-1: frame 0 is chosen on the monitor's own clock before DXGI's first
        // statistics, whose counts start millions of vblanks away (or before 0).
        for offset in [5_000_000, -40] {
            for k in [1, 3] {
                let start = 2 * HNS_PER_SECOND / 10;
                let run = run_vsync(k, 60, Display::new(1, offset), start, |_| 20_000, |_| 0);
                let what = format!("offset {offset} k {k}");
                assert!(run.skipped.is_empty(), "{what}: {:?}", run.skipped);
                let frames: Vec<u64> = run.out.iter().map(|r| r.record.frame).collect();
                assert_eq!(frames, (0..60).collect::<Vec<u64>>(), "{what}");
            }
        }
    }

    #[test]
    fn a_late_start_skips_to_the_frame_now_due() {
        // Frame 0 is placed at vblank 36, then the loop stalls until 10 vblanks after it: frames
        // 0–10 never drawn, frame 0's time still the anchor.
        let start = 2 * HNS_PER_SECOND / 10;
        let mut pacer = VsyncPacer::new(1, HZ180, start, false);
        pacer.on_vblank(0);
        assert_eq!(pacer.next(20_000), Step::Warmup);
        assert!(near(pacer.anchor().unwrap(), HZ180.times(36)));
        pacer.on_vblank(HZ180.times(46));
        let now = HZ180.times(46) + 20_000;
        let Step::Frame {
            frame,
            target_hns,
            skipped,
        } = pacer.next(now)
        else {
            panic!("warm-up")
        };
        assert_eq!((frame, skipped), (11, 0..11));
        assert!(near(target_hns, HZ180.times(47)));
        assert!(near(pacer.anchor().unwrap(), HZ180.times(36)));
    }

    #[test]
    fn a_grid_after_the_start_moves_frame_0_to_the_next_whole_second() {
        // The first vblank timing comes after vblank 36, the one after the start.
        let start = 2 * HNS_PER_SECOND / 10;
        let mut pacer = VsyncPacer::new(1, HZ180, start, false);
        pacer.on_vblank(HZ180.times(46));
        assert_eq!(pacer.next(HZ180.times(46) + 20_000), Step::Warmup);
        assert!(near(pacer.anchor().unwrap(), HNS_PER_SECOND));
        // A grid that comes in time keeps the start's vblank.
        let mut pacer = VsyncPacer::new(1, HZ180, start, false);
        pacer.on_vblank(HZ180.times(35));
        let Step::Frame { frame, .. } = pacer.next(HZ180.times(35) + 20_000) else {
            panic!("warm-up")
        };
        assert_eq!(frame, 0);
        assert!(near(pacer.anchor().unwrap(), HZ180.times(36)));
    }

    #[test]
    fn frames_the_statistics_never_report_expire_after_a_second() {
        let mut shown = Shown::new(Some(1), HZ180);
        for f in 0..4_u32 {
            let t = HZ180.times(i64::from(f));
            shown.presented(u64::from(f), t, f + 1, t, false);
        }
        assert!(shown.expire(HNS_PER_SECOND).is_empty());
        // A second after frame 1 was presented, frames 0 and 1 go out unknown, in order.
        let gone = shown.expire(HZ180.times(1) + HNS_PER_SECOND + 1);
        let frames: Vec<(u64, Option<i64>)> = gone
            .iter()
            .map(|r| (r.record.frame, r.record.display_hns))
            .collect();
        assert_eq!(frames, [(0, None), (1, None)]);
        assert!(gone.iter().all(|r| !r.missed));
        assert!(shown.waiting());
        // Frame 3 reported: frame 2 between is filled in from its neighbours' cadence only when
        // the one before it was reported, so here it stays unknown.
        let out = shown.on_stats(Stats {
            present_count: 4,
            present_refresh: 3,
            sync_refresh: 3,
            sync_hns: HZ180.times(3),
        });
        let frames: Vec<u64> = out.iter().map(|r| r.record.frame).collect();
        assert_eq!(frames, [2, 3]);
        assert_eq!(out[0].record.display_hns, None);
        assert!(!shown.waiting());
    }

    #[test]
    fn the_wake_grid_finds_the_vblanks_under_late_wakes() {
        // The display runs 50 ppm slow: vblank v at 1,234 + v periods × 1.00005. A wake comes
        // 0.1–0.85 ms after its vblank, 3 % of them 3 ms after (past half a period), and 3 % of
        // the vblanks are slept through.
        let true_period = 10_000_000.0 * 1.000_05 / 180.0;
        let mut grid = WakeGrid::new(HZ180);
        let mut seed = 7_u64;
        for v in 0..3_000_i64 {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let r = ((seed >> 33) % 1_000) as i64;
            if r < 30 {
                continue;
            }
            let lag = if r < 60 { 30_000 } else { 500 + 8 * r };
            let vblank = 1_234 + (v as f64 * true_period).round() as i64;
            let t = vblank + lag;
            let last = grid.on_wake(t);
            assert!(last <= t);
            if v >= 180 {
                // On the display's grid, at most the lag of the soonest tenth of wakes late.
                let off = (last - vblank).rem_euclid(55_558);
                assert!(off <= 2_500 || off >= 55_558 - 600, "vblank {v}: {off}");
            }
        }
    }

    #[test]
    fn future_frames_are_predicted_on_the_cadence() {
        let mut pacer = VsyncPacer::new(2, HZ180, 0, false);
        assert_eq!(pacer.predict(10), None);
        pacer.on_vblank(0);
        let Step::Frame {
            frame, target_hns, ..
        } = pacer.next(-1)
        else {
            panic!("warm-up")
        };
        assert_eq!(frame, 0);
        pacer.presented(0, target_hns, 1, 2);
        // Frame 0 at vblank 0, so frame 180 at vblank 360.
        assert!(near(pacer.predict(180).unwrap(), HZ180.times(360)));
        assert_eq!(pacer.predict(0), None);
        // Frame 0 reported a vblank after it was taken (a composed window): the prediction is a
        // display time, a vblank later too.
        pacer.on_stats(Stats {
            present_count: 1,
            present_refresh: 1,
            sync_refresh: 1,
            sync_hns: HZ180.times(1),
        });
        assert!(near(pacer.predict(180).unwrap(), HZ180.times(361)));
        // A frame shown later than its cadence moves the ones after it.
        let Step::Frame {
            frame, target_hns, ..
        } = pacer.next(HZ180.times(1) + 1)
        else {
            panic!("warm-up")
        };
        assert_eq!((frame, near(target_hns, HZ180.times(2))), (1, true));
        pacer.presented(1, target_hns, 2, HZ180.times(1) + 2);
        pacer.on_stats(Stats {
            present_count: 2,
            present_refresh: 4,
            sync_refresh: 4,
            sync_hns: HZ180.times(4),
        });
        // Frame 1 showed at vblank 4 (taken at 3, a vblank late): frame 180 at 3 + 2 × 179 + 1.
        assert!(near(pacer.predict(180).unwrap(), HZ180.times(362)));
    }

    #[test]
    fn the_wake_grid_ignores_however_many_late_wakes() {
        // 180 Hz: a fifth of the wakes come 2.9–4.5 ms late, past half a period. 360 Hz: the
        // first wake is late and a tenth of the rest too. The grid stays on the prompt wakes.
        for (period, late_share, late_from, first) in [
            (HZ180, 200, 29_000, 3_000),
            (Period::from_refresh(360, 1).unwrap(), 100, 15_000, 20_000),
        ] {
            let p = period.times(1);
            let mut grid = WakeGrid::new(period);
            let mut seed = 11_u64;
            for v in 0..2_000_i64 {
                seed = seed
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                let r = ((seed >> 33) % 1_000) as i64;
                let lag = if v == 0 {
                    first
                } else if r < late_share {
                    late_from + 16 * r
                } else {
                    500 + r * 2
                };
                let vblank = 777 + period.times(v);
                let last = grid.on_wake(vblank + lag);
                if v >= 100 {
                    let off = (last - vblank).rem_euclid(p);
                    assert!(off <= 2_600, "{p}: vblank {v}: {off}");
                }
            }
        }
    }

    #[test]
    fn frames_the_statistics_skip_are_filled_in_on_cadence() {
        let mut pacer = VsyncPacer::new(1, HZ180, 0, false);
        pacer.on_vblank(0);
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
        let mut pacer = VsyncPacer::new(1, HZ180, 0, false);
        pacer.on_vblank(0);
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
    fn a_disjoint_reading_resets_the_counts_but_not_the_frames() {
        let mut pacer = VsyncPacer::new(1, HZ180, 0, false);
        pacer.on_vblank(0);
        let stats = |id: u32, v: u32, sync: u32, t: i64| Stats {
            present_count: id,
            present_refresh: v,
            sync_refresh: sync,
            sync_hns: t,
        };
        for i in 0..3_u32 {
            let Step::Frame {
                frame, target_hns, ..
            } = pacer.next(HZ180.times(i64::from(i)) - 1)
            else {
                panic!("warm-up")
            };
            pacer.presented(frame, target_hns, 1 + i, target_hns);
        }
        assert_eq!(pacer.on_stats(stats(1, 1_000, 1_000, 0)).len(), 1);
        // Full screen to windowed: counts restart near 0; frame 2 isn't taken as late.
        pacer.disjoint();
        let rest = pacer.on_stats(stats(3, 2, 2, HZ180.times(2)));
        assert_eq!(rest.len(), 2);
        assert!(rest.iter().all(|r| !r.missed), "{rest:?}");
    }

    #[test]
    fn without_a_grid_the_vsync_pacer_warms_up() {
        let mut pacer = VsyncPacer::new(1, HZ180, 0, false);
        assert_eq!(pacer.next(0), Step::Warmup);
        // Frames never reported get unknown display times at the end.
        pacer.on_vblank(0);
        let Step::Frame {
            frame, target_hns, ..
        } = pacer.next(-1)
        else {
            panic!("warm-up")
        };
        pacer.presented(frame, target_hns, 1, 2);
        assert!(pacer.waiting());
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
        // A clock that jumps an hour and more is lost, not a million `m` lines.
        assert_eq!(t.next(t.slot_time(3_600 * 360 + 100)), Step::Lost);
    }
}
