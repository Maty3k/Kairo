// SPDX-License-Identifier: GPL-3.0-or-later
//! The frame pacer (docs/PLAN.md §2.2; docs/research/15-implementation-cookbook.md §2.2): at
//! each output tick, the newest captured frame whose timestamp is at or before the tick, else
//! the last frame again.
//!
//! The output has a constant frame rate: whichever frame tick `k` shows gets pts `k`. Frames are
//! paced on their presentation time (`SystemRelativeTime`), never on arrival, because WGC
//! timestamps can be late or ahead of arrival (research 08 §4 and §10 item 15; OBS #13893). The
//! pacer names frames by an opaque id, such as the engine's texture slot, and never reads a
//! clock: `wgc-probe` and the engine pass times in, and [`crate::trace::replay`] runs the same
//! code over a recorded trace.

use crate::timeline::{self, FrameRate};

/// The most frames that can wait for a later tick. On overflow the frame furthest ahead is
/// dropped, never the one due next.
pub const MAX_PENDING: usize = 3;

/// Texture slots the engine needs: the waiting frames, the frame on screen and a free slot to
/// copy the next arrival into.
pub const SLOTS_NEEDED: usize = MAX_PENDING + 2;

/// A captured frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame {
    /// Presentation time (`SystemRelativeTime`) in 100 ns on the QPC timeline.
    pub ts_hns: i64,
    /// The engine's name for the frame, such as its texture slot. An id isn't reused while
    /// [`Pacer::held`] lists it.
    pub id: u32,
}

/// What one output tick shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tick {
    /// The tick number, which is also the output frame's pts.
    pub tick: i64,
    /// The frame to show; `None` until the first frame is due.
    pub frame: Option<Frame>,
    /// True if no earlier tick showed `frame`; false for a repeat or an empty tick.
    pub fresh: bool,
}

/// What the pacer has done so far. Cadence (repeats, skips, judder) is measured from the
/// frames shown, by [`crate::cadence::cadence`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PacerStats {
    /// Ticks run.
    pub ticks: u64,
    /// Ticks that showed a new frame.
    pub fresh: u64,
    /// Ticks before the first frame, which show nothing.
    pub empty: u64,
    /// Frames dropped because a newer frame was due by the same tick.
    pub superseded: u64,
    /// Frames dropped because they were not newer than the frame already shown.
    pub stale: u64,
    /// Frames that replaced a waiting frame with the same timestamp.
    pub duplicate_ts: u64,
    /// Frames dropped because [`MAX_PENDING`] frames were already waiting.
    pub pending_overflow: u64,
    /// The most frames waiting at once.
    pub max_pending: usize,
    /// The longest time from a fresh frame's timestamp to the deadline of the tick that showed
    /// it, in 100 ns.
    pub max_staleness_hns: i64,
}

impl PacerStats {
    /// Ticks that showed the previous frame again.
    pub fn repeated(&self) -> u64 {
        self.ticks
            .saturating_sub(self.fresh)
            .saturating_sub(self.empty)
    }

    /// Fresh ticks as a share of the ticks that showed a frame (SP2 asks for at least 99.5%), or
    /// `None` before any tick has shown one.
    pub fn fresh_share(&self) -> Option<f64> {
        let shown = self.ticks.saturating_sub(self.empty);
        (shown > 0).then(|| self.fresh as f64 / shown as f64)
    }
}

/// The pacer. Frames go in through [`Pacer::push`] as they arrive, and [`Pacer::tick`] runs once
/// per tick, at or after [`Pacer::next_deadline_hns`].
#[derive(Debug, Clone)]
pub struct Pacer {
    rate: FrameRate,
    epoch_hns: i64,
    next_tick: i64,
    /// Frames waiting for a tick, reserved in `new`, so pushing never allocates.
    pending: Vec<Frame>,
    shown: Option<Frame>,
    stats: PacerStats,
}

impl Pacer {
    /// A pacer on the clock with epoch `epoch_hns`, whose first tick is `first_tick`.
    ///
    /// Ticks keep counting across encoder epochs, so an encoder restart needs no new pacer. If
    /// the textures behind [`Pacer::held`]'s ids are lost (a device loss), the engine starts a new
    /// pacer at this one's [`Pacer::next_tick`]; its ticks are empty until a frame is due. If the
    /// engine only re-creates them (a pool size or format change), it keeps the held ones until
    /// `held` stops listing them and gives the new ones new ids.
    pub fn new(rate: FrameRate, epoch_hns: i64, first_tick: i64) -> Self {
        Self {
            rate,
            epoch_hns,
            next_tick: first_tick,
            pending: Vec::with_capacity(MAX_PENDING + 1),
            shown: None,
            stats: PacerStats::default(),
        }
    }

    /// The output frame rate.
    pub fn rate(&self) -> FrameRate {
        self.rate
    }

    /// The tick that [`Pacer::tick`] runs next.
    pub fn next_tick(&self) -> i64 {
        self.next_tick
    }

    /// When the next tick is due.
    pub fn next_deadline_hns(&self) -> i64 {
        timeline::deadline_hns(self.next_tick, self.epoch_hns, self.rate)
    }

    /// True once `now_hns` has reached the next tick's time.
    pub fn is_due(&self, now_hns: i64) -> bool {
        now_hns >= self.next_deadline_hns()
    }

    /// Takes a frame that has just arrived.
    ///
    /// A frame no newer than the one on screen is dropped (`stale`), and one with the same
    /// timestamp as a waiting frame replaces it (`duplicate_ts`). Of the frames due by the next
    /// tick only the newest waits, since the others can never be shown (`superseded`); frames
    /// with later timestamps wait for their own tick. Above [`MAX_PENDING`], the frame furthest
    /// ahead is dropped (`pending_overflow`), so a burst of early frames never pushes out the
    /// frame due next.
    pub fn push(&mut self, frame: Frame) {
        if self.shown.is_some_and(|shown| frame.ts_hns <= shown.ts_hns) {
            self.stats.stale += 1;
            return;
        }
        if let Some(same) = self
            .pending
            .iter_mut()
            .find(|waiting| waiting.ts_hns == frame.ts_hns)
        {
            *same = frame;
            self.stats.duplicate_ts += 1;
            return;
        }
        self.pending.push(frame);
        self.drop_superseded();
        if self.pending.len() > MAX_PENDING {
            let furthest = self
                .pending
                .iter()
                .enumerate()
                .max_by_key(|(_, waiting)| waiting.ts_hns)
                .map(|(index, _)| index);
            if let Some(index) = furthest {
                self.pending.swap_remove(index);
            }
            self.stats.pending_overflow += 1;
        }
        self.stats.max_pending = self.stats.max_pending.max(self.pending.len());
    }

    /// Runs the next tick: shows the newest waiting frame due by it, else the last frame again.
    /// Then, of the frames due by the following tick, only the newest keeps waiting.
    pub fn tick(&mut self) -> Tick {
        let (tick, epoch, rate) = (self.next_tick, self.epoch_hns, self.rate);
        let due = |ts| timeline::at_or_before(ts, tick, epoch, rate);
        let newest = self
            .pending
            .iter()
            .filter(|waiting| due(waiting.ts_hns))
            .max_by_key(|waiting| waiting.ts_hns)
            .copied();
        if let Some(frame) = newest {
            let before = self.pending.len();
            self.pending.retain(|waiting| !due(waiting.ts_hns));
            // Every due frame but the one shown is superseded.
            self.stats.superseded += count((before - self.pending.len()).saturating_sub(1));
            self.stats.fresh += 1;
            let staleness = timeline::deadline_hns(tick, epoch, rate).saturating_sub(frame.ts_hns);
            self.stats.max_staleness_hns = self.stats.max_staleness_hns.max(staleness);
            self.shown = Some(frame);
        } else if self.shown.is_none() {
            self.stats.empty += 1;
        }
        self.stats.ticks += 1;
        self.next_tick = self.next_tick.saturating_add(1);
        self.drop_superseded();
        Tick {
            tick,
            frame: self.shown,
            fresh: newest.is_some(),
        }
    }

    /// Ids of the frames the pacer may still show: the waiting frames and the one on screen. The
    /// engine must not overwrite their textures.
    pub fn held(&self) -> impl Iterator<Item = u32> + '_ {
        self.pending
            .iter()
            .map(|waiting| waiting.id)
            .chain(self.shown.map(|shown| shown.id))
    }

    /// What the pacer has done so far.
    pub fn stats(&self) -> PacerStats {
        self.stats
    }

    /// Keeps only the newest waiting frame due by the next tick: the others can never be shown.
    /// `push` calls it after adding a frame and `tick` after moving on, so frames ahead of one
    /// tick that all fall due by the next never hold more than one slot.
    fn drop_superseded(&mut self) {
        let (tick, epoch, rate) = (self.next_tick, self.epoch_hns, self.rate);
        let due = |ts| timeline::at_or_before(ts, tick, epoch, rate);
        let newest_due = self
            .pending
            .iter()
            .filter(|waiting| due(waiting.ts_hns))
            .map(|waiting| waiting.ts_hns)
            .max();
        if let Some(newest) = newest_due {
            let before = self.pending.len();
            self.pending
                .retain(|waiting| !due(waiting.ts_hns) || waiting.ts_hns == newest);
            self.stats.superseded += count(before - self.pending.len());
        }
    }
}

fn count(frames: usize) -> u64 {
    u64::try_from(frames).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Expected values come from an independent Python model with exact fractions. At 60 fps
    // with E = 0, ticks 0 to 9 fall at 0, 166,666.67, 333,333.33, 500,000, 666,666.67,
    // 833,333.33, 1,000,000, 1,166,666.67, 1,333,333.33 and 1,500,000.

    fn frame(ts_hns: i64, id: u32) -> Frame {
        Frame { ts_hns, id }
    }

    fn shown(tick: &Tick) -> Option<(i64, u32)> {
        tick.frame.map(|f| (f.ts_hns, f.id))
    }

    fn held(pacer: &Pacer) -> Vec<u32> {
        let mut ids: Vec<u32> = pacer.held().collect();
        ids.sort_unstable();
        ids
    }

    #[test]
    fn shows_the_newest_frame_due_by_each_tick_then_repeats() {
        let mut pacer = Pacer::new(FrameRate::Fps60, 0, 0);
        let mut ticks = Vec::new();
        pacer.push(frame(0, 0));
        ticks.push(pacer.tick());
        pacer.push(frame(100_000, 1));
        ticks.push(pacer.tick());
        pacer.push(frame(200_000, 2));
        pacer.push(frame(300_000, 3)); // supersedes 200,000: both are due by tick 2
        ticks.push(pacer.tick());
        pacer.push(frame(500_000, 4)); // due by tick 3 at exact equality
        ticks.push(pacer.tick());
        ticks.push(pacer.tick());
        ticks.push(pacer.tick());
        let seen: Vec<_> = ticks
            .iter()
            .map(|tick| (tick.tick, shown(tick), tick.fresh))
            .collect();
        assert_eq!(
            seen,
            [
                (0, Some((0, 0)), true),
                (1, Some((100_000, 1)), true),
                (2, Some((300_000, 3)), true),
                (3, Some((500_000, 4)), true),
                (4, Some((500_000, 4)), false),
                (5, Some((500_000, 4)), false),
            ]
        );
        let stats = pacer.stats();
        assert_eq!((stats.ticks, stats.fresh, stats.repeated()), (6, 4, 2));
        assert_eq!(stats.superseded, 1);
        assert_eq!(held(&pacer), [4]);
    }

    #[test]
    fn late_ahead_duplicate_and_stale_frames() {
        let mut pacer = Pacer::new(FrameRate::Fps60, 0, 6);
        pacer.push(frame(600_000, 5)); // late: due by tick 4, arrives before tick 6
        assert_eq!(shown(&pacer.tick()), Some((600_000, 5)));
        pacer.push(frame(1_200_000, 6)); // ahead of tick 7's time
        let tick7 = pacer.tick();
        assert_eq!((shown(&tick7), tick7.fresh), (Some((600_000, 5)), false));
        assert_eq!(held(&pacer), [5, 6]);
        assert_eq!(shown(&pacer.tick()), Some((1_200_000, 6)));
        pacer.push(frame(1_400_000, 7));
        pacer.push(frame(1_400_000, 8)); // same timestamp: the later frame wins
        assert_eq!(held(&pacer), [6, 8]);
        assert_eq!(shown(&pacer.tick()), Some((1_400_000, 8)));
        pacer.push(frame(1_300_000, 9)); // older than the frame on screen
        pacer.push(frame(1_400_000, 10)); // as old as the frame on screen
        assert_eq!(held(&pacer), [8]);
        let stats = pacer.stats();
        assert_eq!(
            (stats.stale, stats.duplicate_ts, stats.superseded),
            (2, 1, 0)
        );
        // Tick 6's deadline is 1,000,000; its frame was stamped 600,000.
        assert_eq!(stats.max_staleness_hns, 400_000);
    }

    #[test]
    fn overflow_drops_the_frame_furthest_ahead_never_the_one_due_next() {
        let mut pacer = Pacer::new(FrameRate::Fps60, 0, 1);
        pacer.push(frame(100_000, 1)); // due by tick 1
        pacer.push(frame(200_000, 2));
        pacer.push(frame(300_000, 3));
        pacer.push(frame(400_000, 4)); // the fourth waiting frame
        assert_eq!(held(&pacer), [1, 2, 3]);
        let stats = pacer.stats();
        assert_eq!(
            (stats.pending_overflow, stats.max_pending),
            (1, MAX_PENDING)
        );
        let tick = pacer.tick();
        assert_eq!((shown(&tick), tick.fresh), (Some((100_000, 1)), true));
        // 200,000 and 300,000 are both due by tick 2, so only the newer keeps waiting.
        assert_eq!(held(&pacer), [1, 3]);
        assert_eq!(pacer.stats().superseded, 1);
        assert!(held(&pacer).len() < SLOTS_NEEDED);
    }

    #[test]
    fn overflow_keeps_the_frame_due_next_even_when_it_arrives_last() {
        let mut pacer = Pacer::new(FrameRate::Fps60, 0, 1);
        pacer.push(frame(200_000, 2));
        pacer.push(frame(300_000, 3));
        pacer.push(frame(400_000, 4));
        pacer.push(frame(100_000, 1)); // due by tick 1, and the fourth waiting frame
        assert_eq!(held(&pacer), [1, 2, 3]);
        assert_eq!(pacer.stats().pending_overflow, 1);
        let tick = pacer.tick();
        assert_eq!((shown(&tick), tick.fresh), (Some((100_000, 1)), true));
    }

    #[test]
    fn frames_that_fall_due_together_free_their_slots_at_once() {
        // Three frames ahead of tick 1 all fall due by tick 2. Only the newest keeps a slot,
        // so a fourth frame, due by tick 3, isn't lost to an overflow.
        let mut pacer = Pacer::new(FrameRate::Fps60, 0, 0);
        pacer.push(frame(0, 0));
        pacer.tick();
        pacer.push(frame(200_000, 1));
        pacer.push(frame(250_000, 2));
        pacer.push(frame(300_000, 3));
        assert!(!pacer.tick().fresh);
        assert_eq!(held(&pacer), [0, 3]);
        pacer.push(frame(400_000, 4));
        assert_eq!(shown(&pacer.tick()), Some((300_000, 3)));
        assert_eq!(shown(&pacer.tick()), Some((400_000, 4)));
        let stats = pacer.stats();
        assert_eq!((stats.superseded, stats.pending_overflow), (2, 0));
        assert_eq!(stats.max_pending, 3);
    }

    #[test]
    fn a_frame_is_due_by_the_exact_tick_time_not_the_rounded_deadline() {
        // Tick 1 at 60 fps falls at 166,666.67; its deadline rounds up to 166,667.
        let mut pacer = Pacer::new(FrameRate::Fps60, 0, 0);
        pacer.push(frame(0, 0));
        pacer.tick();
        pacer.push(frame(166_667, 1));
        assert!(!pacer.tick().fresh);
        assert_eq!(shown(&pacer.tick()), Some((166_667, 1)));
        let mut pacer = Pacer::new(FrameRate::Fps60, 0, 0);
        pacer.push(frame(0, 0));
        pacer.tick();
        pacer.push(frame(166_666, 1));
        assert_eq!(shown(&pacer.tick()), Some((166_666, 1)));
    }

    #[test]
    fn frames_that_become_due_together_keep_only_the_newest() {
        let mut pacer = Pacer::new(FrameRate::Fps60, 0, 0);
        pacer.push(frame(0, 0));
        pacer.tick();
        pacer.push(frame(200_000, 1)); // both ahead of tick 1, both due by tick 2
        pacer.push(frame(300_000, 2));
        assert_eq!(shown(&pacer.tick()), Some((0, 0)));
        assert_eq!(held(&pacer), [0, 2]);
        assert_eq!(pacer.stats().superseded, 1);
        assert_eq!(shown(&pacer.tick()), Some((300_000, 2)));
        assert_eq!(pacer.stats().superseded, 1);
        // A frame due by the next tick but older than one already waiting is dropped too.
        pacer.push(frame(450_000, 3));
        pacer.push(frame(400_000, 4));
        assert_eq!(held(&pacer), [2, 3]);
        assert_eq!(pacer.stats().superseded, 2);
    }

    #[test]
    fn ticks_before_the_first_frame_are_empty() {
        let mut pacer = Pacer::new(FrameRate::Fps60, 0, 0);
        assert_eq!(pacer.stats().fresh_share(), None);
        for expected in [0, 1] {
            let tick = pacer.tick();
            assert_eq!((tick.tick, tick.frame, tick.fresh), (expected, None, false));
        }
        pacer.push(frame(400_000, 0)); // due by tick 3, not tick 2
        assert_eq!(pacer.tick().frame, None);
        assert!(pacer.tick().fresh);
        assert!(!pacer.tick().fresh);
        let stats = pacer.stats();
        assert_eq!(
            (stats.ticks, stats.empty, stats.fresh, stats.repeated()),
            (5, 3, 1, 1)
        );
        assert_eq!(stats.fresh_share(), Some(0.5));
        assert_eq!(stats.max_staleness_hns, 100_000);
    }

    #[test]
    fn deadlines_and_due_times() {
        let mut pacer = Pacer::new(FrameRate::Fps60, 1_000, 1);
        assert_eq!(pacer.rate(), FrameRate::Fps60);
        assert_eq!(pacer.next_tick(), 1);
        assert_eq!(pacer.next_deadline_hns(), 167_667);
        assert!(!pacer.is_due(167_666));
        assert!(pacer.is_due(167_667));
        pacer.tick();
        assert_eq!(pacer.next_deadline_hns(), 334_334);
        let mut last = Pacer::new(FrameRate::Fps30, 0, i64::MAX);
        assert_eq!(last.tick().tick, i64::MAX);
        assert_eq!(last.next_tick(), i64::MAX);
    }
}
