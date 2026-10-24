// SPDX-License-Identifier: GPL-3.0-or-later
//! `wgc-probe` traces (docs/milestones/M1.md, Formats) and their offline replay through the
//! pacer (docs/PLAN.md §3.2: the pacer is tested on real SP2 traces).
//!
//! A trace is UTF-8 text. The first line is `# kairo-trace 1`. Then comes one line per frame WGC
//! delivered, in arrival order: `ts arr ready`, three times in 100 ns, all counted from the first
//! frame's `SystemRelativeTime`.
//! - `ts`: the frame's `SystemRelativeTime`, so the first frame's is 0. It may repeat or go back.
//!   Gate 3's `SystemRelativeTime` deltas are the differences between consecutive `ts`.
//! - `arr`: the QPC time the `FrameArrived` handler ran. It never goes back, and it is below
//!   `ts` when a timestamp is ahead of its arrival.
//! - `ready`: the QPC time the frame reached the pacer, never before `arr` and never going back;
//!   or `-` if it never did, because a newer frame replaced it in the handler's mailbox or the
//!   probe dropped it while taking it (its content size was empty or had changed, or its texture
//!   didn't match). So gate 3 can tell the probe's own drops from gaps in what WGC delivered.
//!
//! Other lines starting with `#` are comments. Every time is within 24 hours of the first
//! frame's. A trace, comments included, holds no monitor, adapter or PC names, and no refresh
//! rate, interval or wall-clock time: those stay with the job's record in `.dogfood/`.
//!
//! A writer puts out the header, then each frame's line (its `Display` and a line break) in a
//! single `write_all`, so a probe stopped mid-run leaves only whole lines. If a text still ends
//! inside a line, [`Trace::parse`] drops that unfinished line and [`Trace::torn_tail`] says so.

use std::fmt;

use crate::pacer::{Frame, Pacer, PacerStats};
use crate::timeline::{self, FrameRate, HNS_PER_SECOND};

/// The first line of every trace.
pub const TRACE_HEADER: &str = "# kairo-trace 1";

/// The furthest any time in a trace may be from the first frame's timestamp: 24 hours.
pub const MAX_TRACE_HNS: i64 = 24 * 60 * 60 * HNS_PER_SECOND;

/// One frame WGC delivered. All three times count from the first frame's `SystemRelativeTime`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TraceFrame {
    /// The frame's `SystemRelativeTime`.
    pub ts_hns: i64,
    /// When the `FrameArrived` handler ran.
    pub arr_hns: i64,
    /// When the frame reached the pacer; `None` if it never did (replaced in the mailbox, or
    /// dropped by the probe).
    pub ready_hns: Option<i64>,
}

impl fmt::Display for TraceFrame {
    /// The frame as one trace line, without the line break.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.ready_hns {
            Some(ready) => write!(f, "{} {} {ready}", self.ts_hns, self.arr_hns),
            None => write!(f, "{} {} -", self.ts_hns, self.arr_hns),
        }
    }
}

/// A trace that follows the format's rules and holds at least one frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trace {
    frames: Vec<TraceFrame>,
    torn_tail: bool,
}

/// Why a trace was rejected. Each number is the line in the text form, counting the header as
/// line 1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TraceError {
    /// The first line is not `# kairo-trace 1`.
    MissingHeader,
    /// No frames.
    Empty,
    /// Not three fields separated by single spaces: two integers, then an integer or `-`.
    BadLine(usize),
    /// A time more than 24 hours from the first frame's timestamp.
    OutOfRange(usize),
    /// The first frame's `ts` is not 0.
    FirstNotZero(usize),
    /// `arr` went back.
    ArrivalWentBack(usize),
    /// `ready` is before `arr`.
    ReadyBeforeArrival(usize),
    /// `ready` went back.
    ReadyWentBack(usize),
}

impl fmt::Display for TraceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingHeader => write!(f, "the first line must be `{TRACE_HEADER}`"),
            Self::Empty => f.write_str("the trace has no frames"),
            Self::BadLine(line) => {
                write!(
                    f,
                    "line {line} must be `ts arr ready`: integers, ready may be `-`"
                )
            }
            Self::OutOfRange(line) => {
                write!(
                    f,
                    "line {line} has a time more than 24 hours from the first frame"
                )
            }
            Self::FirstNotZero(line) => write!(f, "line {line}: the first frame's ts must be 0"),
            Self::ArrivalWentBack(line) => write!(f, "line {line}: arr went back"),
            Self::ReadyBeforeArrival(line) => write!(f, "line {line}: ready is before arr"),
            Self::ReadyWentBack(line) => write!(f, "line {line}: ready went back"),
        }
    }
}

impl std::error::Error for TraceError {}

impl Trace {
    /// A trace of `frames`, checked against the format's rules.
    pub fn from_frames(frames: Vec<TraceFrame>) -> Result<Self, TraceError> {
        let mut rules = Rules::default();
        for (index, frame) in frames.iter().enumerate() {
            rules.check(frame, index + 2)?;
        }
        if frames.is_empty() {
            return Err(TraceError::Empty);
        }
        Ok(Self {
            frames,
            torn_tail: false,
        })
    }

    /// Reads a trace's text. Lines may end in LF or CRLF. If the text ends inside a line that
    /// breaks the format, as a writer stopped mid-line leaves it, that line is dropped and
    /// [`Trace::torn_tail`] is true.
    pub fn parse(text: &str) -> Result<Self, TraceError> {
        let mut lines = text.split_inclusive('\n').map(|piece| {
            let line = piece.strip_suffix('\n');
            let finished = line.is_some();
            let line = line.unwrap_or(piece);
            (line.strip_suffix('\r').unwrap_or(line), finished)
        });
        if lines.next().map(|(line, _)| line) != Some(TRACE_HEADER) {
            return Err(TraceError::MissingHeader);
        }
        let mut rules = Rules::default();
        let mut frames = Vec::new();
        let mut torn_tail = false;
        for (index, (line, finished)) in lines.enumerate() {
            let number = index + 2;
            if line.starts_with('#') {
                continue;
            }
            let frame = parse_line(line)
                .ok_or(TraceError::BadLine(number))
                .and_then(|frame| rules.check(&frame, number).map(|()| frame));
            match frame {
                Ok(frame) => frames.push(frame),
                // Only the last piece can lack a line break.
                Err(_) if !finished => torn_tail = true,
                Err(error) => return Err(error),
            }
        }
        if frames.is_empty() {
            return Err(TraceError::Empty);
        }
        Ok(Self { frames, torn_tail })
    }

    /// The frames, in arrival order.
    pub fn frames(&self) -> &[TraceFrame] {
        &self.frames
    }

    /// True if [`Trace::parse`] dropped an unfinished last line, as a writer stopped mid-line
    /// leaves.
    pub fn torn_tail(&self) -> bool {
        self.torn_tail
    }

    /// The trace as text, the header first and every line ending in LF.
    pub fn to_text(&self) -> String {
        let mut text = format!("{TRACE_HEADER}\n");
        for frame in &self.frames {
            text.push_str(&format!("{frame}\n"));
        }
        text
    }
}

/// The format's rules, checked one frame at a time.
#[derive(Default)]
struct Rules {
    frames: usize,
    last_arr: Option<i64>,
    last_ready: Option<i64>,
}

impl Rules {
    fn check(&mut self, frame: &TraceFrame, line: usize) -> Result<(), TraceError> {
        let in_range = |t: i64| (-MAX_TRACE_HNS..=MAX_TRACE_HNS).contains(&t);
        if !(in_range(frame.ts_hns)
            && in_range(frame.arr_hns)
            && frame.ready_hns.is_none_or(in_range))
        {
            return Err(TraceError::OutOfRange(line));
        }
        if self.frames == 0 && frame.ts_hns != 0 {
            return Err(TraceError::FirstNotZero(line));
        }
        if self.last_arr.is_some_and(|last| frame.arr_hns < last) {
            return Err(TraceError::ArrivalWentBack(line));
        }
        if let Some(ready) = frame.ready_hns {
            if ready < frame.arr_hns {
                return Err(TraceError::ReadyBeforeArrival(line));
            }
            if self.last_ready.is_some_and(|last| ready < last) {
                return Err(TraceError::ReadyWentBack(line));
            }
            self.last_ready = Some(ready);
        }
        self.last_arr = Some(frame.arr_hns);
        self.frames += 1;
        Ok(())
    }
}

fn parse_line(line: &str) -> Option<TraceFrame> {
    let mut fields = line.split(' ');
    let ts_hns = fields.next()?.parse().ok()?;
    let arr_hns = fields.next()?.parse().ok()?;
    let ready_hns = match fields.next()? {
        "-" => None,
        ready => Some(ready.parse().ok()?),
    };
    fields.next().is_none().then_some(TraceFrame {
        ts_hns,
        arr_hns,
        ready_hns,
    })
}

/// A trace replayed through the pacer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Replay {
    /// The pacer's counts.
    pub stats: PacerStats,
    /// The `ts` shown at each tick from the first frame shown on, repeats included: the input to
    /// [`crate::cadence::cadence`].
    pub shown_hns: Vec<i64>,
}

/// Replays `trace` through a [`Pacer`] whose tick 0 falls `phase_hns` after the trace's zero.
///
/// Each frame with a `ready` time reaches the pacer at that time, in trace order; frames marked
/// `-` never do. Each tick runs at its exact time, so a frame ready even a fraction of 100 ns
/// after it waits for the next tick; a live tick runs a little later, once its timer wakes.
/// Ticks run from the first at or after the first `ready` time to the last at or before the last
/// one, so the end of a trace adds no repeats. Sweeping `phase_hns` across one tick shows how the
/// ticks' phase against the display's refresh changes the result.
pub fn replay(trace: &Trace, rate: FrameRate, phase_hns: i64) -> Replay {
    let mut delivered = trace
        .frames
        .iter()
        .filter_map(|f| f.ready_hns.map(|ready| (ready, f.ts_hns)))
        .peekable();
    let first = delivered.peek().map(|&(ready, _)| ready);
    let last = trace.frames.iter().rev().find_map(|f| f.ready_hns);
    let (Some(first), Some(last)) = (first, last) else {
        return Replay {
            stats: PacerStats::default(),
            shown_hns: Vec::new(),
        };
    };
    let first_tick = timeline::tick_at_or_after(first, phase_hns, rate);
    let mut pacer = Pacer::new(rate, phase_hns, first_tick);
    let mut shown_hns = Vec::new();
    let mut id: u32 = 0;
    // Every time is within 24 hours of zero, so this runs at most 48 hours of ticks.
    while pacer.next_deadline_hns() <= last {
        let tick = pacer.next_tick();
        while let Some((_, ts_hns)) =
            delivered.next_if(|&(ready, _)| timeline::at_or_before(ready, tick, phase_hns, rate))
        {
            pacer.push(Frame { ts_hns, id });
            id = id.wrapping_add(1);
        }
        if let Some(frame) = pacer.tick().frame {
            shown_hns.push(frame.ts_hns);
        }
    }
    Replay {
        stats: pacer.stats(),
        shown_hns,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cadence::cadence;

    // Expected values come from an independent Python model with exact fractions.

    /// Frames on every `every`-th vblank of a `refresh` Hz display for `seconds`, delivered the
    /// moment they are stamped.
    fn vblank_frames(refresh: i64, every: i64, seconds: i64) -> Vec<TraceFrame> {
        (0..refresh * seconds / every)
            .map(|frame| {
                let ts = frame * every * HNS_PER_SECOND / refresh;
                TraceFrame {
                    ts_hns: ts,
                    arr_hns: ts,
                    ready_hns: Some(ts),
                }
            })
            .collect()
    }

    fn steps(shown: &[i64]) -> Vec<i64> {
        let mut steps: Vec<i64> = shown.windows(2).map(|w| w[1] - w[0]).collect();
        steps.sort_unstable();
        steps.dedup();
        steps
    }

    #[test]
    fn text_round_trips() {
        let text = "# kairo-trace 1\n0 -31250 0\n83333 52083 60000\n# a comment\n166667 135416 -\n166667 135420 140000\n";
        let trace = Trace::parse(text).unwrap();
        assert_eq!(
            trace.frames()[2],
            TraceFrame {
                ts_hns: 166_667,
                arr_hns: 135_416,
                ready_hns: None
            }
        );
        assert_eq!(trace.frames().len(), 4);
        assert_eq!(
            trace.to_text(),
            "# kairo-trace 1\n0 -31250 0\n83333 52083 60000\n166667 135416 -\n166667 135420 140000\n"
        );
        assert_eq!(Trace::parse(&trace.to_text()), Ok(trace.clone()));
        assert_eq!(Trace::parse(&text.replace('\n', "\r\n")), Ok(trace.clone()));
        assert_eq!(Trace::from_frames(trace.frames().to_vec()), Ok(trace));
    }

    #[test]
    fn broken_traces_name_their_line() {
        let cases = [
            ("", TraceError::MissingHeader),
            ("# kairo-trace 2\n0 0 0\n", TraceError::MissingHeader),
            (
                "\u{feff}# kairo-trace 1\n0 0 0\n",
                TraceError::MissingHeader,
            ),
            ("# kairo-trace 1\n", TraceError::Empty),
            ("# kairo-trace 1\n# only a comment\n", TraceError::Empty),
            ("# kairo-trace 1\n0 0\n", TraceError::BadLine(2)),
            ("# kairo-trace 1\n0 0 0 0\n", TraceError::BadLine(2)),
            ("# kairo-trace 1\n0  0 0\n", TraceError::BadLine(2)),
            ("# kairo-trace 1\n0 0 0\n\n", TraceError::BadLine(3)),
            ("# kairo-trace 1\n0 0 x\n", TraceError::BadLine(2)),
            ("# kairo-trace 1\n0 - 0\n", TraceError::BadLine(2)),
            ("# kairo-trace 1\n0 0 1.5\n", TraceError::BadLine(2)),
            ("# kairo-trace 1\n7 7 7\n", TraceError::FirstNotZero(2)),
            (
                "# kairo-trace 1\n0 10 10\n5 9 20\n",
                TraceError::ArrivalWentBack(3),
            ),
            (
                "# kairo-trace 1\n0 10 9\n",
                TraceError::ReadyBeforeArrival(2),
            ),
            (
                "# kairo-trace 1\n0 0 20\n5 5 19\n",
                TraceError::ReadyWentBack(3),
            ),
            (
                "# kairo-trace 1\n# x\n0 0 864000000001\n",
                TraceError::OutOfRange(3),
            ),
            (
                "# kairo-trace 1\n0 -864000000001 0\n",
                TraceError::OutOfRange(2),
            ),
        ];
        for (text, expected) in cases {
            assert_eq!(Trace::parse(text), Err(expected), "{text:?}");
        }
        assert_eq!(Trace::from_frames(Vec::new()), Err(TraceError::Empty));
        let edge = TraceFrame {
            ts_hns: 0,
            arr_hns: -MAX_TRACE_HNS,
            ready_hns: Some(MAX_TRACE_HNS),
        };
        assert!(Trace::from_frames(vec![edge]).is_ok());
        // Replaced frames don't count for `ready`'s order.
        assert!(Trace::parse("# kairo-trace 1\n0 0 20\n5 5 -\n6 6 21\n").is_ok());
    }

    #[test]
    fn replays_of_ideal_vblank_traces() {
        // (refresh Hz, every nth vblank delivered, distinct steps in 100 ns, p99 deviation in
        // 100 ns, skipped frames over 599 steps). At 60 fps the 72/s and 90/s rows are
        // research 13's cheaper intervals, and refresh rates that are not multiples of 60 judder.
        let rows: [(i64, i64, &[i64], f64, u64); 6] = [
            (360, 3, &[166_666, 166_667], 0.667, 0),
            (360, 5, &[138_888, 138_889, 277_778], 111_111.333, 119),
            (180, 1, &[166_666, 166_667], 0.667, 0),
            (180, 2, &[111_111, 222_222, 222_223], 55_556.333, 0),
            (175, 1, &[114_286, 171_428, 171_429], 52_381.667, 0),
            (170, 1, &[117_648, 176_470, 176_471], 49_019.667, 0),
        ];
        for (refresh, every, expected_steps, p99, skipped) in rows {
            let trace = Trace::from_frames(vblank_frames(refresh, every, 10)).unwrap();
            for sweep in 0..7 {
                // Ticks fall 1/14 to 13/14 of a vblank after a frame.
                let phase = (2 * sweep + 1) * HNS_PER_SECOND / (refresh * 14);
                let replay = replay(&trace, FrameRate::Fps60, phase);
                let what = format!("{refresh} Hz, every {every}, phase {phase}");
                assert_eq!(replay.stats.ticks, 600, "{what}");
                assert_eq!(replay.stats.fresh, 600, "{what}");
                assert_eq!(replay.stats.max_pending, 1, "{what}");
                let cadence = cadence(&replay.shown_hns, FrameRate::Fps60).unwrap();
                assert_eq!(cadence.steps, 599, "{what}");
                assert_eq!(cadence.skipped, skipped, "{what}");
                assert_eq!(cadence.repeats, 0, "{what}");
                assert!(
                    (cadence.p99_deviation_hns - p99).abs() < 1.01,
                    "{what}: p99 {}",
                    cadence.p99_deviation_hns
                );
                if sweep == 6 {
                    assert_eq!(steps(&replay.shown_hns), expected_steps, "{what}");
                }
            }
        }
    }

    #[test]
    fn replay_at_120_fps() {
        // 180/s on a 360 Hz display under 120 fps ticks: steps of one and two frames (two and
        // four vblanks), so the p99 deviation is one refresh period plus timestamp rounding:
        // 27,778.67 against 27,777.78 hns.
        let trace = Trace::from_frames(vblank_frames(360, 2, 10)).unwrap();
        let replay = replay(&trace, FrameRate::Fps120, 8_333);
        assert_eq!((replay.stats.ticks, replay.stats.fresh), (1_200, 1_200));
        let cadence = cadence(&replay.shown_hns, FrameRate::Fps120).unwrap();
        assert!((cadence.p99_deviation_hns - 27_778.667).abs() < 0.01);
        assert_eq!(steps(&replay.shown_hns), [55_555, 55_556, 111_111, 111_112]);
    }

    #[test]
    fn replay_through_a_one_second_hole() {
        // 120/s on the 360 Hz grid with no frames stamped in [2 s, 3 s).
        let frames: Vec<TraceFrame> = vblank_frames(360, 3, 10)
            .into_iter()
            .filter(|f| !(2 * HNS_PER_SECOND..3 * HNS_PER_SECOND).contains(&f.ts_hns))
            .collect();
        let trace = Trace::from_frames(frames).unwrap();
        let replay = replay(&trace, FrameRate::Fps60, 8_333);
        let stats = replay.stats;
        assert_eq!((stats.ticks, stats.fresh, stats.repeated()), (600, 541, 59));
        assert_eq!(stats.fresh_share(), Some(541.0 / 600.0));
        let cadence = cadence(&replay.shown_hns, FrameRate::Fps60).unwrap();
        assert_eq!(cadence.repeats, 59);
        assert_eq!(cadence.longest_hold, 60);
        assert_eq!(cadence.max_step_ticks, 61);
        assert_eq!(cadence.skipped, 60);
    }

    #[test]
    fn replay_delivers_frames_by_each_ticks_exact_time() {
        // Ready at 166,667 is after tick 1's exact time (166,666.67), so frame 100,000 waits
        // for tick 2.
        let text = "# kairo-trace 1\n0 0 0\n100000 100000 166667\n300000 300000 333334\n";
        let replay = replay(&Trace::parse(text).unwrap(), FrameRate::Fps60, 0);
        assert_eq!(replay.shown_hns, [0, 0, 100_000]);
        assert_eq!((replay.stats.ticks, replay.stats.fresh), (3, 2));
    }

    #[test]
    fn a_trace_cut_inside_its_last_line_keeps_its_whole_lines() {
        let text = "# kairo-trace 1\n0 -31250 0\n83333 52083 60000\n166667 135416 140000\n";
        let last = text.len() - "166667 135416 140000\n".len();
        let at_boundary = Trace::parse(&text[..last]).unwrap();
        assert_eq!(
            (at_boundary.frames().len(), at_boundary.torn_tail()),
            (2, false)
        );
        // Every cut inside the last line breaks a rule, so the line is dropped.
        for cut in last + 1..text.len() - 1 {
            let trace = Trace::parse(&text[..cut]).unwrap();
            assert_eq!(
                (trace.frames().len(), trace.torn_tail()),
                (2, true),
                "{cut}"
            );
            assert_eq!(trace.to_text(), text[..last]);
        }
        // Only the final line break is missing: the line is whole and kept.
        let unfinished = Trace::parse(&text[..text.len() - 1]).unwrap();
        assert_eq!(
            (unfinished.frames().len(), unfinished.torn_tail()),
            (3, false)
        );
        // A finished line that breaks a rule is still an error, and a torn first frame leaves
        // nothing.
        assert_eq!(
            Trace::parse("# kairo-trace 1\n0 0 0\n5 5\n"),
            Err(TraceError::BadLine(3))
        );
        assert_eq!(Trace::parse("# kairo-trace 1\n0 0"), Err(TraceError::Empty));
    }

    #[test]
    fn replaced_frames_never_reach_the_pacer() {
        let text = "# kairo-trace 1\n0 0 10000\n100000 100000 -\n150000 150000 160000\n400000 400000 410000\n";
        let replay = replay(&Trace::parse(text).unwrap(), FrameRate::Fps60, 0);
        // Tick 1 shows 150,000, as 100,000 never arrived; tick 2 repeats it, and the last
        // frame arrives after tick 2, the last tick before its ready time.
        assert_eq!(replay.shown_hns, [150_000, 150_000]);
        assert_eq!((replay.stats.ticks, replay.stats.fresh), (2, 1));
        assert_eq!(replay.stats.superseded, 1);
    }

    #[test]
    fn short_replays() {
        let one = Trace::parse("# kairo-trace 1\n0 0 0\n").unwrap();
        let replay_at_zero = replay(&one, FrameRate::Fps60, 0);
        assert_eq!(replay_at_zero.shown_hns, [0]);
        assert_eq!(replay_at_zero.stats.ticks, 1);
        // No tick falls between the first and last ready times.
        let none = replay(&one, FrameRate::Fps60, 5);
        assert_eq!(
            (none.stats, none.shown_hns),
            (PacerStats::default(), Vec::new())
        );
        let replaced = Trace::parse("# kairo-trace 1\n0 0 -\n").unwrap();
        assert!(replay(&replaced, FrameRate::Fps60, 0).shown_hns.is_empty());
        // A far-off phase changes only the tick numbers.
        let trace = Trace::from_frames(vblank_frames(180, 2, 1)).unwrap();
        let near = replay(&trace, FrameRate::Fps60, 3_968);
        let far = replay(&trace, FrameRate::Fps60, 3_968 - 1_000_000 * HNS_PER_SECOND);
        assert_eq!(near, far);
    }
}
