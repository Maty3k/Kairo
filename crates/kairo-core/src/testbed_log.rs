// SPDX-License-Identifier: GPL-3.0-or-later
//! The logs `testbed.exe` and `kairo-tone.exe` write as ground truth
//! (docs/research/16-verify-testbed/README.md §4.9 and §5).
//!
//! Both are UTF-8 text in the style of the `wgc-probe` trace ([`crate::trace`]): a version line,
//! one `h` header line, then one record per line, each written in a single `write_all` so a
//! program stopped mid-run leaves whole lines. A writer gets each line from `line()`, which
//! refuses values the parser would refuse. Lines starting with `#` after the first are comments.
//! A last line without its line break is dropped even if it parses, since it may have been cut
//! inside its last number ([`TestbedLog::torn_tail`]). Times are absolute QPC in 100 ns
//! ([`crate::timeline::hns_from_qpc`]), so the two programs share a clock without talking.
//!
//! The header lines, with every key, in this order:
//! - `h pattern=1 run=<0–127> mode=borderless|windowed|exclusive pace=vsync|timer fps=<n> F=<n>
//!   size=<w>x<h> refresh_mhz=<n> hdr=0|1 sdr_white_nits=<nits|-> max_nits=<nits|->
//!   start_hns=<t>` (`kairo-testbed 1`);
//! - `h run=<0–127> hz=<n> offset_ms=<0–999> dbfs=<level>` (`kairo-tone 1`).
//!
//! A log holds no device paths, monitor, adapter or PC names, window titles or wall-clock time.
//! It is a raw result: it stays in `C:\kairo\logs` and `.dogfood/`.
//!
//! The testbed's records:
//! - `f <N> <target_hns> <present_hns> <display_hns|->`: frame N, the time it was drawn for, when
//!   `Present` returned, and the vblank where it first appeared (`-` if unknown or never shown).
//! - `b <m> <N_f> <target_hns> <played_hns|->`: beep m, aimed at flash frame N_f's display, and
//!   when its first sample played.
//! - `m <N> vblank|slot`: frame N missed its vblank (vsync pacing) or its slot (timer pacing).
//! - `e <name> [key=value …]`: an event such as `ready`, `fse result=ok`, `end reason=seconds`.
//!
//! `kairo-tone`'s records: `v <k> <target_hns> <played_hns|->` and the same `e` lines.

use std::fmt;

/// The first line of every testbed log.
pub const TESTBED_LOG_HEADER: &str = "# kairo-testbed 1";
/// The first line of every tone log.
pub const TONE_LOG_HEADER: &str = "# kairo-tone 1";

/// How the testbed's window is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// A popup covering the monitor.
    Borderless,
    /// A framed window.
    Windowed,
    /// Exclusive fullscreen.
    Exclusive,
}

/// How the testbed paces its frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pace {
    /// One frame per vblank group.
    Vsync,
    /// Frames on a QPC timer.
    Timer,
}

/// What a missed frame missed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Miss {
    /// Its vblank (vsync pacing).
    Vblank,
    /// Its timer slot (timer pacing).
    Slot,
}

/// The testbed log's `h` line.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TestbedHeader {
    /// The pattern's version.
    pub pattern: u8,
    /// The run's tag, 0 to 127.
    pub run_tag: u8,
    /// The window mode.
    pub mode: Mode,
    /// The pacing.
    pub pace: Pace,
    /// Frames drawn per second.
    pub fps: u32,
    /// The nominal rate that times the flash (`F`).
    pub nominal_fps: u32,
    /// The client width, in pixels.
    pub width: u32,
    /// The client height, in pixels.
    pub height: u32,
    /// The monitor's refresh rate, in mHz.
    pub refresh_mhz: u64,
    /// Whether the swap chain is HDR.
    pub hdr: bool,
    /// The OS's SDR white in HDR mode, in nits.
    pub sdr_white_nits: Option<f64>,
    /// The display's peak in HDR mode, in nits.
    pub max_nits: Option<f64>,
    /// Frame 0's target time.
    pub start_hns: i64,
}

/// A frame record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameRecord {
    /// The frame number.
    pub frame: u64,
    /// The time it was drawn for.
    pub target_hns: i64,
    /// When `Present` returned.
    pub present_hns: i64,
    /// The vblank where it first appeared, if known.
    pub display_hns: Option<i64>,
}

/// A beep record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BeepRecord {
    /// The beep's number.
    pub beep: u64,
    /// The flash frame it is aimed at.
    pub flash_frame: u64,
    /// When its first sample was meant to play.
    pub target_hns: i64,
    /// When its first sample played, if known.
    pub played_hns: Option<i64>,
}

/// A voice beep record from `kairo-tone`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VoiceRecord {
    /// The beep's number.
    pub beep: u64,
    /// When its first sample was meant to play.
    pub target_hns: i64,
    /// When its first sample played, if known.
    pub played_hns: Option<i64>,
}

/// An event: a name and `key=value` fields. Built only through [`Event::new`], so every event
/// can be written as a line the parser reads back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    name: String,
    fields: Vec<(String, String)>,
}

impl Event {
    /// An event called `name` with `fields`, or `None` unless the name and keys are words
    /// (lower-case ASCII letters, digits and `_`) and each value is non-empty with no space,
    /// `=` or line break.
    pub fn new(name: &str, fields: &[(&str, &str)]) -> Option<Event> {
        let value_ok = |v: &str| !v.is_empty() && !v.contains([' ', '=', '\r', '\n']);
        if !is_word(name) || !fields.iter().all(|(k, v)| is_word(k) && value_ok(v)) {
            return None;
        }
        Some(Event {
            name: name.to_owned(),
            fields: fields
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
        })
    }

    /// What happened, e.g. `ready` or `end`.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Its details, in order.
    pub fn fields(&self) -> &[(String, String)] {
        &self.fields
    }
}

/// One line of a testbed log after the header.
#[derive(Debug, Clone, PartialEq)]
pub enum TestbedRecord {
    /// `f`.
    Frame(FrameRecord),
    /// `b`.
    Beep(BeepRecord),
    /// `m`.
    Miss(u64, Miss),
    /// `e`.
    Event(Event),
}

/// One line of a tone log after the header.
#[derive(Debug, Clone, PartialEq)]
pub enum ToneRecord {
    /// `v`.
    Voice(VoiceRecord),
    /// `e`.
    Event(Event),
}

/// The tone log's `h` line.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ToneHeader {
    /// The run's tag, 0 to 127.
    pub run_tag: u8,
    /// The tone's frequency, in Hz.
    pub hz: u32,
    /// Where in each second it starts, in ms.
    pub offset_ms: u32,
    /// Its peak level, in dBFS.
    pub dbfs: f64,
}

/// Why a log was refused. Line numbers count from 1; the line itself is never quoted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogError {
    /// The first line isn't the version line this parser reads.
    Version,
    /// No `h` line follows the version line.
    NoHeader,
    /// A line that isn't a valid record or header.
    Line(usize),
    /// A frame, beep or voice number not above the previous one.
    Order(usize),
}

impl fmt::Display for LogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Version => write!(f, "not a log of a version this parser reads"),
            Self::NoHeader => write!(f, "the log has no header line"),
            Self::Line(line) => write!(f, "line {line} is not a valid record"),
            Self::Order(line) => write!(f, "line {line} goes back in numbering"),
        }
    }
}

impl std::error::Error for LogError {}

/// A testbed log that follows the format's rules.
#[derive(Debug, Clone, PartialEq)]
pub struct TestbedLog {
    /// The header.
    pub header: TestbedHeader,
    /// The records, in file order.
    pub records: Vec<TestbedRecord>,
    torn_tail: bool,
}

/// A tone log that follows the format's rules.
#[derive(Debug, Clone, PartialEq)]
pub struct ToneLog {
    /// The header.
    pub header: ToneHeader,
    /// The records, in file order.
    pub records: Vec<ToneRecord>,
    torn_tail: bool,
}

// ---------------------------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------------------------

/// `-` for nothing.
struct Maybe<T>(Option<T>);

impl<T: fmt::Display> fmt::Display for Maybe<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            Some(value) => write!(f, "{value}"),
            None => write!(f, "-"),
        }
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Borderless => "borderless",
            Self::Windowed => "windowed",
            Self::Exclusive => "exclusive",
        })
    }
}

impl fmt::Display for Pace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Vsync => "vsync",
            Self::Timer => "timer",
        })
    }
}

impl fmt::Display for Miss {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Vblank => "vblank",
            Self::Slot => "slot",
        })
    }
}

impl fmt::Display for TestbedHeader {
    /// The `h` line, without the line break.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "h pattern={} run={} mode={} pace={} fps={} F={} size={}x{} refresh_mhz={} hdr={} \
             sdr_white_nits={} max_nits={} start_hns={}",
            self.pattern,
            self.run_tag,
            self.mode,
            self.pace,
            self.fps,
            self.nominal_fps,
            self.width,
            self.height,
            self.refresh_mhz,
            u8::from(self.hdr),
            Maybe(self.sdr_white_nits),
            Maybe(self.max_nits),
            self.start_hns
        )
    }
}

impl fmt::Display for ToneHeader {
    /// The `h` line, without the line break.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "h run={} hz={} offset_ms={} dbfs={}",
            self.run_tag, self.hz, self.offset_ms, self.dbfs
        )
    }
}

impl fmt::Display for Event {
    /// The `e` line, without the line break.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "e {}", self.name)?;
        for (key, value) in &self.fields {
            write!(f, " {key}={value}")?;
        }
        Ok(())
    }
}

impl fmt::Display for TestbedRecord {
    /// The record's line, without the line break.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Frame(r) => write!(
                f,
                "f {} {} {} {}",
                r.frame,
                r.target_hns,
                r.present_hns,
                Maybe(r.display_hns)
            ),
            Self::Beep(r) => write!(
                f,
                "b {} {} {} {}",
                r.beep,
                r.flash_frame,
                r.target_hns,
                Maybe(r.played_hns)
            ),
            Self::Miss(frame, miss) => write!(f, "m {frame} {miss}"),
            Self::Event(event) => write!(f, "{event}"),
        }
    }
}

impl fmt::Display for ToneRecord {
    /// The record's line, without the line break.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Voice(r) => write!(f, "v {} {} {}", r.beep, r.target_hns, Maybe(r.played_hns)),
            Self::Event(event) => write!(f, "{event}"),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------------------------

/// Numbered lines: (line number counted from 1, text).
type Lines<'a> = Vec<(usize, &'a str)>;

/// The lines that count: complete ones, after the version line, skipping comments; and whether
/// a torn last line was dropped.
fn body<'a>(text: &'a str, version: &str) -> Result<(Lines<'a>, bool), LogError> {
    let torn = !text.is_empty() && !text.ends_with('\n');
    let complete = if torn {
        text.rfind('\n').map_or("", |end| &text[..=end])
    } else {
        text
    };
    let mut lines = complete.lines().enumerate().map(|(i, line)| (i + 1, line));
    match lines.next() {
        Some((_, first)) if first.trim_end_matches('\r') == version => {}
        _ => return Err(LogError::Version),
    }
    let kept = lines
        .map(|(n, line)| (n, line.trim_end_matches('\r')))
        .filter(|(_, line)| !line.starts_with('#'))
        .collect();
    Ok((kept, torn))
}

/// A word made only of lower-case ASCII letters, digits and `_`.
fn is_word(text: &str) -> bool {
    !text.is_empty()
        && text
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// A canonical non-negative integer: digits only, no sign, no leading zero.
fn number<T: std::str::FromStr>(text: &str) -> Option<T> {
    let canonical = !text.is_empty()
        && text.bytes().all(|b| b.is_ascii_digit())
        && (text == "0" || !text.starts_with('0'));
    if canonical { text.parse().ok() } else { None }
}

/// A time: a canonical non-negative integer of 100 ns.
fn time(text: &str) -> Option<i64> {
    number::<i64>(text)
}

/// A time, or `-`.
fn maybe_time(text: &str) -> Option<Option<i64>> {
    if text == "-" {
        Some(None)
    } else {
        time(text).map(Some)
    }
}

/// A finite level written the way the writer writes it (Rust's shortest exact form, so `240`
/// and not `240.0` or `2.4e2`), or `-`.
fn maybe_level(text: &str) -> Option<Option<f64>> {
    if text == "-" {
        return Some(None);
    }
    let value = text.parse::<f64>().ok().filter(|v| v.is_finite())?;
    (value.to_string() == text).then_some(Some(value))
}

/// A luminance: a level above 0 nits, or `-`.
fn maybe_nits(text: &str) -> Option<Option<f64>> {
    maybe_level(text).filter(|nits| nits.is_none_or(|n| n > 0.0))
}

/// The fields of an `h` line in order, checked against `keys`.
fn header_values<'a>(line: &'a str, keys: &[&str]) -> Option<Vec<&'a str>> {
    let mut words = line.split(' ');
    if words.next()? != "h" {
        return None;
    }
    let mut values = Vec::with_capacity(keys.len());
    for key in keys {
        let (k, v) = words.next()?.split_once('=')?;
        if k != *key || v.is_empty() {
            return None;
        }
        values.push(v);
    }
    words.next().is_none().then_some(values)
}

fn event(words: &[&str]) -> Option<Event> {
    let (name, rest) = words.split_first()?;
    let fields = rest
        .iter()
        .map(|word| word.split_once('='))
        .collect::<Option<Vec<_>>>()?;
    Event::new(name, &fields)
}

impl TestbedLog {
    /// Reads a testbed log, dropping a torn last line.
    pub fn parse(text: &str) -> Result<Self, LogError> {
        let (lines, torn_tail) = body(text, TESTBED_LOG_HEADER)?;
        let mut lines = lines.into_iter();
        let (n, first) = lines.next().ok_or(LogError::NoHeader)?;
        let header = parse_testbed_header(first).ok_or(if first.starts_with("h ") {
            LogError::Line(n)
        } else {
            LogError::NoHeader
        })?;
        let mut records = Vec::new();
        let (mut last_frame, mut last_beep) = (None, None);
        for (n, line) in lines {
            let record = parse_testbed_record(line).ok_or(LogError::Line(n))?;
            match &record {
                TestbedRecord::Frame(r) => {
                    if last_frame.is_some_and(|last| r.frame <= last) {
                        return Err(LogError::Order(n));
                    }
                    last_frame = Some(r.frame);
                }
                TestbedRecord::Beep(r) => {
                    if last_beep.is_some_and(|last| r.beep <= last) {
                        return Err(LogError::Order(n));
                    }
                    last_beep = Some(r.beep);
                }
                TestbedRecord::Miss(..) | TestbedRecord::Event(_) => {}
            }
            records.push(record);
        }
        Ok(Self {
            header,
            records,
            torn_tail,
        })
    }

    /// Whether the text ended inside a line, which [`TestbedLog::parse`] dropped.
    pub fn torn_tail(&self) -> bool {
        self.torn_tail
    }

    /// The frame records, in order.
    pub fn frames(&self) -> impl Iterator<Item = &FrameRecord> {
        self.records.iter().filter_map(|r| match r {
            TestbedRecord::Frame(frame) => Some(frame),
            _ => None,
        })
    }

    /// The beep records, in order.
    pub fn beeps(&self) -> impl Iterator<Item = &BeepRecord> {
        self.records.iter().filter_map(|r| match r {
            TestbedRecord::Beep(beep) => Some(beep),
            _ => None,
        })
    }

    /// The whole log as text, ending with a line break.
    pub fn to_text(&self) -> String {
        let mut text = format!("{TESTBED_LOG_HEADER}\n{}\n", self.header);
        for record in &self.records {
            text.push_str(&format!("{record}\n"));
        }
        text
    }
}

fn parse_testbed_record(line: &str) -> Option<TestbedRecord> {
    let words: Vec<&str> = line.split(' ').collect();
    Some(match words.as_slice() {
        ["f", frame, target, present, display] => TestbedRecord::Frame(FrameRecord {
            frame: number(frame)?,
            target_hns: time(target)?,
            present_hns: time(present)?,
            display_hns: maybe_time(display)?,
        }),
        ["b", beep, flash, target, played] => TestbedRecord::Beep(BeepRecord {
            beep: number(beep)?,
            flash_frame: number(flash)?,
            target_hns: time(target)?,
            played_hns: maybe_time(played)?,
        }),
        ["m", frame, kind] => {
            let miss = match *kind {
                "vblank" => Miss::Vblank,
                "slot" => Miss::Slot,
                _ => return None,
            };
            TestbedRecord::Miss(number(frame)?, miss)
        }
        ["e", rest @ ..] => TestbedRecord::Event(event(rest)?),
        _ => return None,
    })
}

fn parse_tone_record(line: &str) -> Option<ToneRecord> {
    let words: Vec<&str> = line.split(' ').collect();
    Some(match words.as_slice() {
        ["v", beep, target, played] => ToneRecord::Voice(VoiceRecord {
            beep: number(beep)?,
            target_hns: time(target)?,
            played_hns: maybe_time(played)?,
        }),
        ["e", rest @ ..] => ToneRecord::Event(event(rest)?),
        _ => return None,
    })
}

/// `value`'s line with its line break, if the parser reads it back as `value`.
fn checked_line<T: fmt::Display + PartialEq>(
    value: &T,
    parse: fn(&str) -> Option<T>,
) -> Option<String> {
    let line = value.to_string();
    (parse(&line).as_ref() == Some(value)).then(|| line + "\n")
}

impl TestbedHeader {
    /// The `h` line with its line break, or `None` if a value is outside the format (a run tag
    /// above 127, a zero rate or size, a nits value that isn't finite and above 0, a negative
    /// time), so the testbed can't write a header its own parser refuses.
    pub fn line(&self) -> Option<String> {
        checked_line(self, parse_testbed_header)
    }
}

impl TestbedRecord {
    /// The record's line with its line break, or `None` if the parser would refuse it (a
    /// negative time).
    pub fn line(&self) -> Option<String> {
        checked_line(self, parse_testbed_record)
    }
}

impl ToneHeader {
    /// The `h` line with its line break, or `None` if a value is outside the format.
    pub fn line(&self) -> Option<String> {
        checked_line(self, parse_tone_header)
    }
}

impl ToneRecord {
    /// The record's line with its line break, or `None` if the parser would refuse it.
    pub fn line(&self) -> Option<String> {
        checked_line(self, parse_tone_record)
    }
}

fn parse_testbed_header(line: &str) -> Option<TestbedHeader> {
    const KEYS: [&str; 12] = [
        "pattern",
        "run",
        "mode",
        "pace",
        "fps",
        "F",
        "size",
        "refresh_mhz",
        "hdr",
        "sdr_white_nits",
        "max_nits",
        "start_hns",
    ];
    let v = header_values(line, &KEYS)?;
    let (width, height) = v[6].split_once('x')?;
    let run_tag = number::<u8>(v[1]).filter(|tag| *tag <= 0x7F)?;
    Some(TestbedHeader {
        pattern: number(v[0])?,
        run_tag,
        mode: match v[2] {
            "borderless" => Mode::Borderless,
            "windowed" => Mode::Windowed,
            "exclusive" => Mode::Exclusive,
            _ => return None,
        },
        pace: match v[3] {
            "vsync" => Pace::Vsync,
            "timer" => Pace::Timer,
            _ => return None,
        },
        fps: number(v[4]).filter(|fps| *fps > 0)?,
        nominal_fps: number(v[5]).filter(|fps| *fps > 0)?,
        width: number(width).filter(|w| *w > 0)?,
        height: number(height).filter(|h| *h > 0)?,
        refresh_mhz: number(v[7]).filter(|mhz| *mhz > 0)?,
        hdr: match v[8] {
            "0" => false,
            "1" => true,
            _ => return None,
        },
        sdr_white_nits: maybe_nits(v[9])?,
        max_nits: maybe_nits(v[10])?,
        start_hns: time(v[11])?,
    })
}

impl ToneLog {
    /// Reads a tone log, dropping a torn last line.
    pub fn parse(text: &str) -> Result<Self, LogError> {
        let (lines, torn_tail) = body(text, TONE_LOG_HEADER)?;
        let mut lines = lines.into_iter();
        let (n, first) = lines.next().ok_or(LogError::NoHeader)?;
        let header = parse_tone_header(first).ok_or(if first.starts_with("h ") {
            LogError::Line(n)
        } else {
            LogError::NoHeader
        })?;
        let mut records = Vec::new();
        let mut last = None;
        for (n, line) in lines {
            let record = parse_tone_record(line).ok_or(LogError::Line(n))?;
            if let ToneRecord::Voice(voice) = &record {
                if last.is_some_and(|last| voice.beep <= last) {
                    return Err(LogError::Order(n));
                }
                last = Some(voice.beep);
            }
            records.push(record);
        }
        Ok(Self {
            header,
            records,
            torn_tail,
        })
    }

    /// Whether the text ended inside a line, which [`ToneLog::parse`] dropped.
    pub fn torn_tail(&self) -> bool {
        self.torn_tail
    }

    /// The voice beep records, in order.
    pub fn voices(&self) -> impl Iterator<Item = &VoiceRecord> {
        self.records.iter().filter_map(|r| match r {
            ToneRecord::Voice(voice) => Some(voice),
            ToneRecord::Event(_) => None,
        })
    }

    /// The whole log as text, ending with a line break.
    pub fn to_text(&self) -> String {
        let mut text = format!("{TONE_LOG_HEADER}\n{}\n", self.header);
        for record in &self.records {
            text.push_str(&format!("{record}\n"));
        }
        text
    }
}

fn parse_tone_header(line: &str) -> Option<ToneHeader> {
    let v = header_values(line, &["run", "hz", "offset_ms", "dbfs"])?;
    Some(ToneHeader {
        run_tag: number::<u8>(v[0]).filter(|tag| *tag <= 0x7F)?,
        hz: number(v[1]).filter(|hz| *hz > 0)?,
        offset_ms: number(v[2]).filter(|ms| *ms < 1_000)?,
        // A peak level can't be above full scale.
        dbfs: maybe_level(v[3])?.filter(|dbfs| *dbfs <= 0.0)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header() -> TestbedHeader {
        TestbedHeader {
            pattern: 1,
            run_tag: 93,
            mode: Mode::Borderless,
            pace: Pace::Vsync,
            // Different on purpose, so swapping the two can't pass.
            fps: 240,
            nominal_fps: 180,
            width: 2_560,
            height: 1_440,
            refresh_mhz: 179_960,
            hdr: false,
            sdr_white_nits: None,
            max_nits: None,
            start_hns: 123_450_000_000,
        }
    }

    fn event(name: &str, fields: &[(&str, &str)]) -> TestbedRecord {
        TestbedRecord::Event(Event::new(name, fields).unwrap())
    }

    fn sample() -> TestbedLog {
        TestbedLog {
            header: header(),
            records: vec![
                event("ready", &[]),
                TestbedRecord::Frame(FrameRecord {
                    frame: 0,
                    target_hns: 123_450_000_000,
                    present_hns: 123_450_000_400,
                    display_hns: Some(123_450_055_570),
                }),
                TestbedRecord::Beep(BeepRecord {
                    beep: 1,
                    flash_frame: 180,
                    target_hns: 123_460_055_570,
                    played_hns: None,
                }),
                TestbedRecord::Miss(2, Miss::Vblank),
                TestbedRecord::Frame(FrameRecord {
                    frame: 3,
                    target_hns: 123_450_166_680,
                    present_hns: 123_450_167_000,
                    display_hns: None,
                }),
                event("end", &[("reason", "seconds")]),
            ],
            torn_tail: false,
        }
    }

    const SAMPLE: &str = "# kairo-testbed 1\n\
        h pattern=1 run=93 mode=borderless pace=vsync fps=240 F=180 size=2560x1440 \
        refresh_mhz=179960 hdr=0 sdr_white_nits=- max_nits=- start_hns=123450000000\n\
        e ready\n\
        f 0 123450000000 123450000400 123450055570\n\
        b 1 180 123460055570 -\n\
        m 2 vblank\n\
        f 3 123450166680 123450167000 -\n\
        e end reason=seconds\n";

    #[test]
    fn testbed_logs_are_written_and_read_back() {
        assert_eq!(sample().to_text(), SAMPLE);
        let log = TestbedLog::parse(SAMPLE).unwrap();
        assert_eq!(log, sample());
        assert!(!log.torn_tail());
        assert_eq!(log.frames().map(|f| f.frame).collect::<Vec<_>>(), [0, 3]);
        assert_eq!(
            log.beeps().map(|b| b.flash_frame).collect::<Vec<_>>(),
            [180]
        );
        let mut hdr = sample();
        hdr.header.hdr = true;
        hdr.header.sdr_white_nits = Some(240.0);
        hdr.header.max_nits = Some(1_015.5);
        let text = hdr.to_text();
        assert!(text.contains(" hdr=1 sdr_white_nits=240 max_nits=1015.5 "));
        assert_eq!(TestbedLog::parse(&text).unwrap(), hdr);
        // Frames and beeps are numbered apart: beep 1 after frame 200 is fine.
        let long = SAMPLE.replacen("f 3 ", "f 200 ", 1) + "b 2 360 123470055570 -\n";
        assert!(TestbedLog::parse(&long).is_ok());
    }

    #[test]
    fn every_spelling_is_pinned() {
        // The writer's and the parser's string tables are separate: pin the literal text.
        for (mode, word) in [
            (Mode::Borderless, "borderless"),
            (Mode::Windowed, "windowed"),
            (Mode::Exclusive, "exclusive"),
        ] {
            for (pace, pace_word) in [(Pace::Vsync, "vsync"), (Pace::Timer, "timer")] {
                let h = TestbedHeader {
                    mode,
                    pace,
                    ..header()
                };
                let line = h.line().unwrap();
                assert!(
                    line.contains(&format!(" mode={word} pace={pace_word} ")),
                    "{line}"
                );
                assert_eq!(parse_testbed_header(line.trim_end()), Some(h));
            }
        }
        for (miss, line) in [(Miss::Vblank, "m 5 vblank\n"), (Miss::Slot, "m 5 slot\n")] {
            let record = TestbedRecord::Miss(5, miss);
            assert_eq!(record.line().as_deref(), Some(line));
            assert_eq!(parse_testbed_record(line.trim_end()), Some(record));
        }
    }

    #[test]
    fn every_documented_event_parses() {
        // research 16 README §4.1, §4.9 and §4.12.
        for line in [
            "e ready",
            "e activate result=ok",
            "e activate result=failed",
            "e fse result=ok",
            "e fse result=failed",
            "e fullscreen_lost",
            "e hdr_changed",
            "e device_lost",
            "e audio_failed",
            "e audio_device_changed",
            "e end reason=seconds",
            "e end reason=stdin",
        ] {
            let text = SAMPLE.replacen("e ready", line, 1);
            let log = TestbedLog::parse(&text).unwrap();
            let TestbedRecord::Event(event) = &log.records[0] else {
                panic!("{line}");
            };
            assert_eq!(
                TestbedRecord::Event(event.clone()).line().unwrap(),
                format!("{line}\n")
            );
        }
    }

    #[test]
    fn events_are_checked_when_built() {
        assert!(Event::new("fse", &[("result", "ok")]).is_some());
        for (name, fields) in [
            ("fse ok", &[][..]),
            ("", &[]),
            ("End", &[]),
            ("end", &[("Reason", "seconds")]),
            ("end", &[("reason", "")]),
            ("end", &[("reason", "window closed")]),
            ("end", &[("reason", "a=b")]),
            ("end", &[("reason", "x\nf 9 1 2 3")]),
            ("end", &[("reason", "x\r")]),
        ] {
            assert_eq!(Event::new(name, fields), None, "{name} {fields:?}");
        }
        let end = Event::new("end", &[("reason", "seconds")]).unwrap();
        assert_eq!(end.name(), "end");
        assert_eq!(end.fields(), [("reason".to_owned(), "seconds".to_owned())]);
    }

    #[test]
    fn writers_refuse_values_outside_the_format() {
        let bad_headers = [
            TestbedHeader {
                run_tag: 128,
                ..header()
            },
            TestbedHeader { fps: 0, ..header() },
            TestbedHeader {
                nominal_fps: 0,
                ..header()
            },
            TestbedHeader {
                width: 0,
                ..header()
            },
            TestbedHeader {
                refresh_mhz: 0,
                ..header()
            },
            TestbedHeader {
                sdr_white_nits: Some(f64::NAN),
                ..header()
            },
            TestbedHeader {
                max_nits: Some(0.0),
                ..header()
            },
            TestbedHeader {
                max_nits: Some(-80.0),
                ..header()
            },
            TestbedHeader {
                start_hns: -1,
                ..header()
            },
        ];
        for h in bad_headers {
            assert_eq!(h.line(), None, "{h:?}");
        }
        assert!(
            header()
                .line()
                .unwrap()
                .ends_with("start_hns=123450000000\n")
        );
        let late = TestbedRecord::Frame(FrameRecord {
            frame: 1,
            target_hns: -5,
            present_hns: 0,
            display_hns: None,
        });
        assert_eq!(late.line(), None);
        let tone = ToneHeader {
            run_tag: 1,
            hz: 2_000,
            offset_ms: 500,
            dbfs: -30.0,
        };
        assert_eq!(
            tone.line().as_deref(),
            Some("h run=1 hz=2000 offset_ms=500 dbfs=-30\n")
        );
        for bad in [
            ToneHeader {
                run_tag: 128,
                ..tone
            },
            ToneHeader { hz: 0, ..tone },
            ToneHeader {
                offset_ms: 1_000,
                ..tone
            },
            ToneHeader { dbfs: 3.0, ..tone },
            ToneHeader {
                dbfs: f64::NEG_INFINITY,
                ..tone
            },
        ] {
            assert_eq!(bad.line(), None, "{bad:?}");
        }
        let voice = ToneRecord::Voice(VoiceRecord {
            beep: 0,
            target_hns: 7,
            played_hns: Some(-1),
        });
        assert_eq!(voice.line(), None);
    }

    #[test]
    fn torn_lines_comments_and_crlf_are_handled() {
        let torn = format!("{SAMPLE}f 9 1234");
        let log = TestbedLog::parse(&torn).unwrap();
        assert!(log.torn_tail());
        assert_eq!(log.records, sample().records);
        // A last line cut inside its last number still parses; it is dropped all the same.
        let cut = format!("{SAMPLE}f 9 123450500000 123450500400 12345");
        let log = TestbedLog::parse(&cut).unwrap();
        assert!(log.torn_tail());
        assert_eq!(log.records, sample().records);
        // A header cut short leaves no header.
        let header_only = SAMPLE.lines().take(2).collect::<Vec<_>>().join("\n");
        assert_eq!(TestbedLog::parse(&header_only), Err(LogError::NoHeader));
        let commented = SAMPLE.replacen("e ready\n", "# a comment\ne ready\n", 1);
        assert_eq!(TestbedLog::parse(&commented).unwrap(), sample());
        let crlf = SAMPLE.replace('\n', "\r\n");
        assert_eq!(TestbedLog::parse(&crlf).unwrap(), sample());
    }

    #[test]
    fn bad_testbed_logs_are_refused() {
        assert_eq!(TestbedLog::parse(""), Err(LogError::Version));
        for version in [
            "# kairo-testbed 2\n",
            "# kairo-testbed 10\n",
            "# kairo-testbed\n",
        ] {
            assert_eq!(
                TestbedLog::parse(version),
                Err(LogError::Version),
                "{version}"
            );
        }
        assert_eq!(
            TestbedLog::parse("# kairo-testbed 1\n"),
            Err(LogError::NoHeader)
        );
        let no_header = "# kairo-testbed 1\nf 0 1 2 3\n";
        assert_eq!(TestbedLog::parse(no_header), Err(LogError::NoHeader));
        for (from, to) in [
            ("run=93", "run=128"),
            ("mode=borderless", "mode=fullscreen"),
            ("fps=240", "fps=0"),
            ("F=180", "F=0"),
            ("fps=240 F=180", "F=180 fps=240"),
            ("size=2560x1440", "size=2560"),
            ("size=2560x1440", "size=0x1440"),
            ("refresh_mhz=179960", "refresh_mhz=0"),
            ("hdr=0", "hdr=2"),
            ("sdr_white_nits=-", "sdr_white_nits=inf"),
            ("sdr_white_nits=-", "sdr_white_nits=240.0"),
            ("sdr_white_nits=-", "sdr_white_nits=+240"),
            ("sdr_white_nits=-", "sdr_white_nits=2.4e2"),
            ("sdr_white_nits=-", "sdr_white_nits=0"),
            ("max_nits=-", "max_nits=-80"),
            ("start_hns=123450000000", "start_hns=-5"),
            ("start_hns=123450000000", "start_hns=123450000000 x=1"),
            (" max_nits=-", ""),
            ("F=180", "f=180"),
        ] {
            let text = SAMPLE.replacen(from, to, 1);
            assert_eq!(TestbedLog::parse(&text), Err(LogError::Line(2)), "{to}");
        }
        for (from, to, line) in [
            ("m 2 vblank", "m 2 late", 6),
            ("m 2 vblank", "x 2 vblank", 6),
            ("f 3 123450166680", "f 03 123450166680", 7),
            ("f 3 123450166680", "f +3 123450166680", 7),
            ("f 3 123450166680", "f 3 0123450166680", 7),
            ("f 3 123450166680", "f 3 +123450166680", 7),
            ("b 1 180 123460055570 -", "b 1 180 123460055570", 5),
            ("e end reason=seconds", "e end reason=", 8),
            ("e end reason=seconds", "e End", 8),
            ("e end reason=seconds", "e end Reason=seconds", 8),
            ("e end reason=seconds", "e end a=b=c", 8),
            ("e end reason=seconds", "e fse ok", 8),
            ("e ready\n", "\n", 3),
        ] {
            let text = SAMPLE.replacen(from, to, 1);
            assert_eq!(TestbedLog::parse(&text), Err(LogError::Line(line)), "{to}");
        }
        let backwards = SAMPLE.replacen("f 3 ", "f 0 ", 1);
        assert_eq!(TestbedLog::parse(&backwards), Err(LogError::Order(7)));
        let beeps = format!("{SAMPLE}b 1 360 1 -\n");
        assert_eq!(TestbedLog::parse(&beeps), Err(LogError::Order(9)));
    }

    const TONE: &str = "# kairo-tone 1\nh run=93 hz=2000 offset_ms=500 dbfs=-30\nv 0 123455000000 123455000123\nv 1 123465000000 -\ne end reason=stdin\n";

    #[test]
    fn tone_logs_are_written_and_read_back() {
        let log = ToneLog::parse(TONE).unwrap();
        assert_eq!(
            log.header,
            ToneHeader {
                run_tag: 93,
                hz: 2_000,
                offset_ms: 500,
                dbfs: -30.0
            }
        );
        assert_eq!(
            log.voices()
                .map(|v| (v.beep, v.played_hns))
                .collect::<Vec<_>>(),
            [(0, Some(123_455_000_123)), (1, None)]
        );
        assert_eq!(log.to_text(), TONE);
        let torn = ToneLog::parse(&format!("{TONE}v 2 123475000000 12347")).unwrap();
        assert!(torn.torn_tail());
        assert_eq!(torn.records, log.records);
    }

    #[test]
    fn bad_tone_logs_are_refused() {
        for (from, to) in [
            ("run=93", "run=128"),
            ("hz=2000", "hz=0"),
            ("offset_ms=500", "offset_ms=1000"),
            ("dbfs=-30", "dbfs=-"),
            ("dbfs=-30", "dbfs=-30.0"),
            ("dbfs=-30", "dbfs=10"),
            ("hz=2000 offset_ms=500", "offset_ms=500 hz=2000"),
        ] {
            assert_eq!(
                ToneLog::parse(&TONE.replacen(from, to, 1)),
                Err(LogError::Line(2)),
                "{to}"
            );
        }
        assert_eq!(
            ToneLog::parse(&TONE.replacen("v 1 ", "v 0 ", 1)),
            Err(LogError::Order(4))
        );
        assert_eq!(ToneLog::parse("# kairo-tone 1\n"), Err(LogError::NoHeader));
        assert_eq!(
            ToneLog::parse("# kairo-tone 1\nv 0 1 2\n"),
            Err(LogError::NoHeader)
        );
        assert_eq!(ToneLog::parse(SAMPLE), Err(LogError::Version));
        assert_eq!(
            LogError::Order(4).to_string(),
            "line 4 goes back in numbering"
        );
    }
}
