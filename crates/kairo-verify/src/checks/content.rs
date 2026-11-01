// SPDX-License-Identifier: GPL-3.0-or-later
//! The content checks on decoded pictures and audio (docs/research/16-verify-testbed/README.md
//! §6.3, §6.5): `decode.clean`, `content.barcode`, `cadence.drops_dups`, `cadence.judder`,
//! `content.flash`, `content.bars`, `content.colour_matrix`, `content.bars_lines`,
//! `content.levels`, `hdr.curve_levels`, `source.stability`, and the audio checks of
//! [`super::audio`]. With the testbed's log (`--testbed-log`, README §4.9) the rates, the run tag
//! and the HDR levels come from it, steps that touch a frame the testbed itself missed count as
//! the source's and not the clip's, `source.stability` judges the testbed, and A/V is exact.
//!
//! Every picture's and sample's time comes from Kairo's sample tables, never from the decoder.

use std::ops::ControlFlow;

use serde_json::json;

use super::source::Source;
use super::{Ctx, Finding, Outcome, Status, audio, skipped, weigh};
use crate::analyze::bars::{self, BandSums, Bands, FrameBars};
use crate::analyze::cadence::{self, Counts, Event};
use crate::analyze::flash::{self, Flashes};
use crate::analyze::levels::{self, Expected, Patch};
use crate::analyze::{self, Geometry, Line, NoLine, Reading};
use crate::decode::{DecodeError, DecodeStats, Decoder};
use crate::profile::{self, Content, Mode, Plan, Profile};

/// What the decoder gave: its kind, how the decode went, and what each picture showed.
#[derive(Debug, Clone)]
pub struct Decoded {
    /// `synthetic`, `cli` or `linked`.
    pub kind: &'static str,
    /// The decoder's FFmpeg licence, when it says: a spawned FFmpeg's, reported only, or the
    /// linked one's, which `build.lgpl` reads from [`crate::decode::linked_ffmpeg`] instead.
    pub licence: Option<&'static str>,
    /// The decode's outcome.
    pub stats: Result<DecodeStats, DecodeError>,
    /// One reading per picture, in presentation order.
    pub readings: Vec<Reading>,
    /// Each picture's bars, scored when its barcode read.
    bars: Vec<Option<FrameBars>>,
    /// The band sums of the pictures whose barcode didn't read, kept to score them once their
    /// neighbours' frame numbers are known: at most [`PENDING_MAX`].
    pending: Vec<(usize, BandSums)>,
    /// Unreadable pictures past [`PENDING_MAX`], left unscored.
    pending_dropped: usize,
    /// The bands of the pictures' size, if they fit.
    bands: Option<Bands>,
    /// Each picture's whole patches, measured when its barcode read.
    patches: Vec<Vec<Patch>>,
    /// Each audio track as decoded, in file order.
    pub audio: Vec<DecodedAudio>,
}

/// One audio track as decoded.
#[derive(Debug, Clone)]
pub struct DecodedAudio {
    /// Its (L+R)/2 at 48 kHz, at most [`profile::AUDIO_MAX_SECONDS`].
    pub samples: Vec<f32>,
    /// How its decode went, in sample frames.
    pub stats: Result<DecodeStats, DecodeError>,
    /// Whether it was cut at [`profile::AUDIO_MAX_SECONDS`].
    pub truncated: bool,
}

/// How many unreadable pictures' band sums may wait for the barcode line. A picture is scored as
/// soon as the next readable one shows its neighbours at most 64 frames apart, so only long or
/// out-of-order unreadable runs wait: about 169 KB each at 1440p, 20 MB at most.
const PENDING_MAX: usize = 120;

/// The checks this build runs.
pub(crate) const RUNS: [&str; 17] = [
    "decode.clean",
    "content.barcode",
    "cadence.drops_dups",
    "cadence.judder",
    "content.flash",
    "content.bars",
    "content.colour_matrix",
    "content.bars_lines",
    "content.levels",
    "hdr.curve_levels",
    "source.stability",
    "av.offset",
    "av.offset_tight",
    "av.inter_track",
    "audio.tone_snr",
    "audio.routing",
    "audio.cue",
];

/// The audio checks.
const AUDIO_CHECKS: [&str; 6] = [
    "av.offset",
    "av.offset_tight",
    "av.inter_track",
    "audio.tone_snr",
    "audio.routing",
    "audio.cue",
];

/// What the readings add up to.
struct Analysis {
    /// Each picture's testbed frame number, unwrapped.
    numbers: Vec<Option<i64>>,
    /// The barcode line, or why none fits.
    line: Result<Line, NoLine>,
    /// The testbed's nominal rate: the barcode line's rounded.
    source_fps: Option<u32>,
    /// The line's testbed rate when it is faster than the expect file's `source_hz` allows.
    too_fast: Option<(f64, u32)>,
    /// The display's refresh rate and how many vblanks each testbed frame lasts.
    refresh: Option<(f64, u32)>,
    fps: Option<u32>,
    counts: Option<Counts>,
    /// The ideal's (drop-and-duplicate share, judder share).
    ideal: Option<(f64, f64)>,
    /// The flash at the nominal rate F the lit frames fit best (F or F ± 1), with that F.
    flashes: Option<(u32, Flashes)>,
    /// The 99th percentile of |step − tick| between consecutive readable frames, in 100 ns, from
    /// the line's slope (README §6.3 `cadence.judder`).
    p99_step_error_hns: Option<f64>,
    /// Each picture's bars, the unreadable ones scored against their likely frame numbers.
    bars: Vec<Option<FrameBars>>,
    /// Each picture's presentation time, in media ticks.
    pts: Vec<i64>,
    /// The testbed's log, when one was given.
    source: Option<Source>,
    /// The media timescale.
    timescale: u64,
}

/// How far the line's testbed rate may exceed the expect file's `source_hz`: a timer-paced
/// testbed runs at or below its display's rate (README §4.3).
const SOURCE_RATE_SLACK: f64 = 0.005;

/// The fastest display the cadence ideal is replayed for, in Hz.
const MAX_IDEAL_HZ: f64 = 2_000.0;

/// What the plan says the patches show.
fn expected_patches(plan: &Plan) -> Expected {
    // The curve comes from the expect file; the SDR white and the peak the testbed measured
    // come from its log when it drew in HDR (README §6.3 hdr.curve_levels).
    let logged = plan
        .testbed_log
        .as_ref()
        .filter(|log| log.header.hdr)
        .and_then(|log| log.header.sdr_white_nits.zip(log.header.max_nits));
    match plan.expect.as_ref().and_then(|e| e.hdr.as_ref()) {
        Some(hdr) => {
            let (white_nits, peak_nits) =
                logged.unwrap_or((f64::from(hdr.sdr_white_nits), f64::from(hdr.lw_nits)));
            Expected::Hdr {
                curve: if hdr.curve == "match" {
                    kairo_core::testsig::Curve::MatchSdrApps
                } else {
                    kairo_core::testsig::Curve::Natural
                },
                peak_nits,
                white_nits,
            }
        }
        None => Expected::Sdr,
    }
}

/// Runs `decoder` over the clip's video: each picture's barcodes and flash, and, once its frame
/// number is known, its bars and patches. Nothing but these numbers is kept (README §6.7). Then
/// each audio track, `audio` giving their channel and packet counts in file order: its (L+R)/2,
/// up to [`profile::AUDIO_MAX_SECONDS`].
pub(crate) fn decode(decoder: &mut dyn Decoder, plan: &Plan, audio: &[(usize, usize)]) -> Decoded {
    decode_capped(
        decoder,
        plan,
        audio,
        (profile::AUDIO_MAX_SECONDS * 48_000) as usize,
    )
}

/// [`decode`], keeping at most `cap` sample frames of each audio track.
fn decode_capped(
    decoder: &mut dyn Decoder,
    plan: &Plan,
    audio: &[(usize, usize)],
    cap: usize,
) -> Decoded {
    let expected = expected_patches(plan);
    let mut decoded = Decoded {
        kind: decoder.kind(),
        licence: decoder.licence(),
        stats: Ok(DecodeStats::default()),
        readings: Vec::new(),
        bars: Vec::new(),
        pending: Vec::new(),
        pending_dropped: 0,
        bands: None,
        patches: Vec::new(),
        audio: Vec::new(),
    };
    let mut geometry: Option<(usize, usize, Option<Geometry>)> = None;
    let mut previous: Option<i64> = None;
    // The last readable picture: (index, frame number).
    let mut last_readable: Option<(usize, i64)> = None;
    let stats = decoder.video(&mut |index, planes| {
        let size = (planes.width(), planes.height());
        if geometry.as_ref().is_none_or(|g| (g.0, g.1) != size) {
            let layout = u32::try_from(size.0)
                .ok()
                .zip(u32::try_from(size.1).ok())
                .and_then(|(w, h)| Geometry::new(w, h).ok());
            geometry = Some((size.0, size.1, layout));
            decoded.bands = Bands::new(size.0, size.1);
        }
        let reading = match geometry.as_ref().and_then(|g| g.2.as_ref()) {
            Some(layout) => analyze::read(layout, planes),
            None => Reading::default(),
        };
        // The same unwrapping as analyze::frame_numbers, as the pictures come.
        let number = reading.code.map(|c| {
            let n = previous.map_or(i64::from(c.frame24), |p| {
                kairo_core::testsig::unwrap_frame(p, c.frame24)
            });
            previous = Some(n);
            n
        });
        let sums = decoded.bands.as_ref().and_then(|b| b.sums(planes));
        let (frame_bars, patches) = match (number.and_then(|n| u64::try_from(n).ok()), sums) {
            (Some(n), Some(sums)) => (
                decoded.bands.as_ref().map(|b| bars::judge(b, &sums, n)),
                levels::patches(planes, n, &expected),
            ),
            (None, Some(sums)) => {
                if decoded.pending.len() < PENDING_MAX {
                    decoded.pending.push((index, sums));
                } else {
                    decoded.pending_dropped += 1;
                }
                (None, Vec::new())
            }
            _ => (None, Vec::new()),
        };
        decoded.bars.push(frame_bars);
        decoded.patches.push(patches);
        decoded.readings.push(reading);
        if let Some(n) = number {
            // The unreadable pictures since the last readable one show a frame between the two:
            // score them now if that span is short, as candidates() would.
            if let (Some((last, before)), Some(bands)) = (last_readable, decoded.bands.as_ref())
                && n >= before
                && n - before <= 64
            {
                let span: Vec<u64> = (before.max(0)..=n)
                    .filter_map(|m| u64::try_from(m).ok())
                    .collect();
                let waiting = decoded.pending.iter().position(|(i, _)| *i > last);
                if let Some(from) = waiting {
                    for (i, sums) in decoded.pending.drain(from..) {
                        if let Some(slot) = decoded.bars.get_mut(i) {
                            *slot = bars::judge_best(bands, &sums, span.clone());
                        }
                    }
                }
            }
            last_readable = Some((index, n));
        }
        ControlFlow::Continue(())
    });
    decoded.stats = stats;
    for (track, &(channels, packets)) in audio.iter().enumerate() {
        // AAC gives 1,024 sample frames a packet: room for exactly those, so a 10-minute track
        // holds 115 MB, not the 134 MB a doubling vector would reach.
        let mut samples = Vec::with_capacity(cap.min(packets.saturating_mul(1_024)));
        let mut truncated = false;
        let stats = decoder.audio(track, channels, &mut |chunk| {
            analyze::audio::downmix(chunk, channels, &mut samples);
            if samples.len() >= cap {
                samples.truncate(cap);
                truncated = true;
                return ControlFlow::Break(());
            }
            ControlFlow::Continue(())
        });
        decoded.audio.push(DecodedAudio {
            samples,
            stats,
            truncated,
        });
    }
    decoded
}

/// The frame numbers an unreadable picture may show: everything between its readable
/// neighbours' when they are at most 64 apart, else the barcode line's estimate ± two output
/// frames' worth (README §6.3 `content.bars`).
fn candidates(
    index: usize,
    numbers: &[Option<i64>],
    line: Option<&Line>,
    pts: &[i64],
    step: f64,
) -> Vec<u64> {
    let before = numbers[..index].iter().rev().find_map(|n| *n);
    let after = numbers
        .get(index + 1..)
        .and_then(|rest| rest.iter().find_map(|n| *n));
    if let (Some(a), Some(b)) = (before, after)
        && b >= a
        && b - a <= 64
    {
        return (a.max(0)..=b)
            .filter_map(|n| u64::try_from(n).ok())
            .collect();
    }
    let (Some(line), Some(&t)) = (line, pts.get(index)) else {
        return Vec::new();
    };
    let estimate = (t as f64 - line.alpha) / line.beta;
    let spread = (2.0 * step).ceil().max(1.0);
    let (from, to) = ((estimate - spread).floor(), (estimate + spread).ceil());
    if !(from.is_finite() && to.is_finite()) || to - from > 4_096.0 {
        return Vec::new();
    }
    (from.max(0.0) as u64..=to.max(0.0) as u64).collect()
}

/// Each picture's presentation time in media ticks, earliest first: decode time plus `ctts`.
fn presentation_times(ctx: &Ctx<'_>) -> Vec<i64> {
    let mut offsets = Vec::with_capacity(ctx.samples.len());
    match &ctx.video.ctts {
        Some(runs) => {
            for &(count, offset) in runs {
                offsets.extend(std::iter::repeat_n(offset, count as usize));
            }
        }
        None => offsets.resize(ctx.samples.len(), 0),
    }
    let mut times: Vec<i64> = ctx
        .samples
        .iter()
        .zip(offsets.into_iter().chain(std::iter::repeat(0)))
        .map(|(s, offset)| {
            i64::try_from(s.dts)
                .unwrap_or(i64::MAX)
                .saturating_add(offset)
        })
        .collect();
    times.sort_unstable();
    times
}

fn analyse(ctx: &Ctx<'_>, decoded: &Decoded) -> Analysis {
    let numbers = analyze::frame_numbers(&decoded.readings);
    let pts = presentation_times(ctx);
    let line = analyze::barcode_line(&pts, &numbers);
    let timescale = ctx.timing.timescale;
    let rate = line.as_ref().ok().map(|l| l.rate(timescale));
    let source = ctx.plan.testbed_log.as_ref().map(Source::new);
    // The log's rate when there is a log; else the line's, rounded (README §6.3).
    let source_fps = source.as_ref().map(|s| s.fps).or_else(|| {
        rate.map(f64::round)
            .filter(|&r| (1.0..=10_000.0).contains(&r))
            .map(|r| r as u32)
    });
    let source_hz = ctx.plan.expect.as_ref().and_then(|e| e.source_hz);
    let too_fast = rate
        .zip(source_hz)
        .filter(|&(rate, hz)| rate > f64::from(hz) * (1.0 + SOURCE_RATE_SLACK));
    let refresh = match &source {
        Some(s) => Some((s.refresh_hz, s.every())),
        None => line_refresh(rate, source_fps, source_hz),
    };
    let fps = ctx
        .timing
        .measured
        .and_then(|m| (m.fps.1 == 1).then_some(m.fps.0))
        .and_then(|f| u32::try_from(f).ok());
    // Steps that touch a frame the testbed missed are the source's (README §6.3).
    let counts = source_fps.zip(fps).and_then(|(fs, f)| match &source {
        Some(s) => cadence::count_excluding(&numbers, fs, f, |from, to| s.affects(from, to)),
        None => cadence::count(&numbers, fs, f),
    });
    // The ideal replays every vblank for 10 s: bounded to displays up to 2 kHz.
    let ideal = refresh
        .filter(|&(hz, _)| hz <= MAX_IDEAL_HZ)
        .zip(source_fps)
        .zip(fps.and_then(cadence::pacer_rate))
        .and_then(|(((hz, every), fs), rate)| cadence::ideal(hz, every, fs, rate));
    // F from the log; else from the line, checked against the lit frames: F ± 1 when it fits
    // them better (README §6.3 `content.flash`).
    let flashes = match &source {
        Some(s) => Some((
            s.nominal_fps,
            flash::flashes(&decoded.readings, &numbers, s.nominal_fps),
        )),
        None => source_fps.map(|fs| {
            let mut best = (fs, flash::flashes(&decoded.readings, &numbers, fs));
            for f in [fs.saturating_sub(1), fs + 1] {
                if f > 0 {
                    let other = flash::flashes(&decoded.readings, &numbers, f);
                    if other.wrong.len() < best.1.wrong.len() {
                        best = (f, other);
                    }
                }
            }
            best
        }),
    };
    let p99_step_error_hns = match (&source, &line, fps) {
        (Some(s), _, Some(fps)) if s.timed => display_step_error_p99(s, &numbers, fps),
        (_, Ok(line), Some(fps)) => step_error_p99(line, &numbers, timescale, fps),
        _ => None,
    };
    let mut frame_bars = decoded.bars.clone();
    if let Some(bands) = decoded.bands.as_ref() {
        let step = source_fps
            .zip(fps)
            .map_or(1.0, |(fs, f)| f64::from(fs) / f64::from(f.max(1)));
        for (index, sums) in &decoded.pending {
            let numbers_near = candidates(*index, &numbers, line.as_ref().ok(), &pts, step);
            if let Some(slot) = frame_bars.get_mut(*index) {
                *slot = bars::judge_best(bands, sums, numbers_near);
            }
        }
    }
    Analysis {
        numbers,
        line,
        bars: frame_bars,
        pts,
        source_fps,
        too_fast,
        refresh,
        fps,
        counts,
        ideal,
        flashes,
        p99_step_error_hns,
        source,
        timescale,
    }
}

/// The display's refresh rate and vblanks per testbed frame from the barcode line's rate and the
/// expect file's `source_hz`.
fn line_refresh(
    rate: Option<f64>,
    source_fps: Option<u32>,
    source_hz: Option<u32>,
) -> Option<(f64, u32)> {
    rate.zip(source_fps).map(|(rate, fs)| {
        // With the testbed's refresh rate known, each frame lasts that many vblanks of it (more
        // than four for a timer-paced testbed, 72 fps on 360 Hz; README §4.3); without, one: a
        // testbed running at its display's refresh rate.
        let every = source_hz.map_or(1, |hz| {
            ((f64::from(hz) / f64::from(fs)).round() as u32).max(1)
        });
        let measured = rate * f64::from(every);
        // The expect file's rate when it agrees within 0.5%: on a short clip the line's
        // estimate is off by a fraction of a percent, which would move the ideal.
        let hz = source_hz
            .map(f64::from)
            .filter(|&hz| (hz - measured).abs() <= SOURCE_RATE_SLACK * hz)
            .unwrap_or(measured);
        (hz, every)
    })
}

/// The 99th percentile (nearest rank) of |display(to) − display(from) − one output frame| over
/// the steps between consecutive readable frames whose testbed frames the log says when it
/// showed, in 100 ns (README §6.3 `cadence.judder`, with a log).
fn display_step_error_p99(source: &Source, numbers: &[Option<i64>], fps: u32) -> Option<f64> {
    let tick = 1.0 / f64::from(fps);
    let mut errors: Vec<f64> = numbers
        .windows(2)
        .filter_map(|pair| {
            let (a, b) = (source.display_s(pair[0]?)?, source.display_s(pair[1]?)?);
            Some((b - a - tick).abs() * 10_000_000.0)
        })
        .collect();
    if errors.is_empty() {
        return None;
    }
    errors.sort_by(f64::total_cmp);
    let rank = ((0.99 * errors.len() as f64).ceil() as usize).clamp(1, errors.len());
    Some(errors[rank - 1])
}

/// The 99th percentile (nearest rank) of |β·ΔN − one output frame| over the steps between
/// consecutive readable frames, in 100 ns.
fn step_error_p99(line: &Line, numbers: &[Option<i64>], timescale: u64, fps: u32) -> Option<f64> {
    let hns = 10_000_000.0 / timescale as f64;
    let tick = 10_000_000.0 / f64::from(fps);
    let mut errors: Vec<f64> = numbers
        .windows(2)
        .filter_map(|pair| {
            let (a, b) = (pair[0]?, pair[1]?);
            Some((line.beta * (b - a) as f64 * hns - tick).abs())
        })
        .collect();
    if errors.is_empty() {
        return None;
    }
    errors.sort_by(f64::total_cmp);
    let rank = ((0.99 * errors.len() as f64).ceil() as usize).clamp(1, errors.len());
    Some(errors[rank - 1])
}

/// The content findings, in the order of [`profile::CONTENT_CHECKS`].
pub(crate) fn findings(
    ctx: Option<&Ctx<'_>>,
    plan: &Plan,
    decoded: Option<&Decoded>,
) -> Vec<Finding> {
    let (Some(ctx), Some(decoded)) = (ctx, decoded) else {
        return super::content_findings(plan);
    };
    let started = decoded.stats.as_ref().ok();
    // A clip that needn't hold the pattern is judged on it only if it shows it early on.
    let shows_pattern = decoded
        .readings
        .iter()
        .take(profile::PATTERN_PROBE_FRAMES)
        .any(|r| r.code.is_some());
    let analysis = analyse(ctx, decoded);
    // The audio checks judge every track or none: one that didn't decode would read as silence
    // and fail routes it never got to show (README §6.6: a decoder that can't, exit 3).
    let audio_failed = decoded.audio.iter().enumerate().find_map(|(i, a)| {
        let error = a.stats.as_ref().err()?;
        Some(format!("audio track {} didn't decode: {error}", i + 1))
    });
    // The audio analysis, made when the first audio check runs: `Some(None)` without audio.
    let mut sound: Option<Option<audio::Audio<'_>>> = None;
    let mut out = Vec::with_capacity(profile::CONTENT_CHECKS.len());
    for id in profile::CONTENT_CHECKS {
        let mode = plan.content_mode(id);
        let required = mode == Mode::Required;
        if mode == Mode::Off {
            out.push(skipped(
                id,
                false,
                format!("not in the {} profile", plan.profile),
            ));
            continue;
        }
        if id == "build.lgpl" {
            let linked = crate::decode::linked_ffmpeg();
            out.push(super::lgpl_finding(
                required,
                linked.as_ref(),
                Some(decoded.kind),
                decoded.licence,
            ));
            continue;
        }
        if !RUNS.contains(&id) {
            out.push(not_yet(id, mode, plan, shows_pattern));
            continue;
        }
        if id == "decode.clean" {
            out.push(weigh(id, decode_clean(ctx, decoded), mode));
            continue;
        }
        if started.is_none() {
            out.push(skipped(
                id,
                required,
                "the decode failed (decode.clean)".to_owned(),
            ));
            continue;
        }
        if plan.content == Content::IfPattern && !shows_pattern {
            out.push(skipped(
                id,
                required,
                format!(
                    "no testbed pattern in the first {} frames",
                    profile::PATTERN_PROBE_FRAMES
                ),
            ));
            continue;
        }
        let hdr = plan.expect.as_ref().is_some_and(|e| e.hdr.is_some());
        if let Some(why) = audio_failed.as_ref().filter(|_| AUDIO_CHECKS.contains(&id)) {
            out.push(weigh(id, Outcome::cannot(why.clone()), mode));
            continue;
        }
        if AUDIO_CHECKS.contains(&id) && sound.is_none() {
            sound = Some(audio_analysis(ctx, plan, decoded, &analysis));
        }
        let outcome = match (id, sound.as_ref()) {
            (_, Some(None)) if AUDIO_CHECKS.contains(&id) => Outcome::skip("no audio track"),
            ("av.offset", Some(Some(a))) => audio::av_offset(a),
            ("av.offset_tight", Some(Some(a))) => audio::av_offset_tight(a),
            ("av.inter_track", Some(Some(a))) => audio::av_inter_track(a),
            ("audio.tone_snr", Some(Some(a))) => audio::tone_snr(a, plan),
            ("audio.routing", Some(Some(a))) => audio::routing(a),
            ("audio.cue", Some(Some(a))) => audio::cue(a),
            _ => video_check(id, plan, decoded, &analysis, hdr),
        };
        out.push(weigh(id, outcome, mode));
    }
    out
}

/// A video content check.
fn video_check(
    id: &str,
    plan: &Plan,
    decoded: &Decoded,
    analysis: &Analysis,
    hdr: bool,
) -> Outcome {
    match id {
        "content.barcode" => barcode(
            plan,
            &decoded.readings,
            analysis.source.as_ref().map(|s| s.run_tag),
        ),
        "source.stability" => source_stability(analysis),
        "cadence.drops_dups" => drops_dups(plan, analysis),
        "cadence.judder" => judder(plan, analysis),
        "content.flash" => flash(analysis),
        "content.bars" => bars_psnr(plan, decoded, analysis),
        "content.colour_matrix" => colour_matrix(decoded, analysis),
        "content.bars_lines" => bars_lines(plan, decoded, analysis),
        "content.levels" if hdr => {
            Outcome::skip("the patches are HDR: hdr.curve_levels judges them")
        }
        "content.levels" | "hdr.curve_levels" => patch_levels(decoded, hdr),
        _ => Outcome::skip("unknown check"),
    }
}

/// The audio tracks with what each should carry, measured against the flashes; `None` for a
/// clip without audio.
fn audio_analysis<'d>(
    ctx: &Ctx<'_>,
    plan: &Plan,
    decoded: &'d Decoded,
    analysis: &Analysis,
) -> Option<audio::Audio<'d>> {
    let titles: Vec<String> = ctx
        .audio()
        .map(|(_, t)| super::tracks::title(t).unwrap_or_default().to_owned())
        .collect();
    if titles.is_empty() {
        return None;
    }
    let has_voice = titles.iter().any(|t| t == "Voice chat");
    let tracks = titles
        .into_iter()
        .enumerate()
        .map(|(i, title)| audio::Track {
            expected: audio::Expected::of(plan, i, &title, has_voice),
            title,
            samples: decoded
                .audio
                .get(i)
                .map_or(&[][..], |a| a.samples.as_slice()),
            truncated: decoded.audio.get(i).is_some_and(|a| a.truncated),
        })
        .collect();
    let flash_fps = analysis
        .flashes
        .as_ref()
        .map(|(f, _)| *f)
        .or(analysis.source_fps);
    // With the testbed's log, its beeps' played times on the video's clock (exact A/V, README
    // §6.3 av.offset); without, the flashes on the barcode line.
    let clock = analysis
        .source
        .as_ref()
        .and_then(|s| s.clip_clock(&analysis.numbers, &analysis.pts, ctx.timing.timescale));
    let logged = analysis
        .source
        .as_ref()
        .zip(clock)
        .filter(|(s, _)| s.beeps.iter().any(|b| b.2.is_some()));
    let game = match (logged, &analysis.line, flash_fps) {
        (Some((s, clock)), _, _) => Ok(audio::game_from_log(s, clock, &analysis.numbers)),
        (None, Ok(line), Some(f)) => Ok(audio::game_expected(
            line,
            &analysis.numbers,
            &analysis.pts,
            ctx.timing.timescale,
            f,
        )),
        (None, Err(reason), _) => Err(format!("no barcode line to time the flashes: {reason}")),
        (None, Ok(_), None) => Err("no testbed rate to time the flashes".to_owned()),
    };
    let longest = decoded
        .audio
        .iter()
        .map(|a| a.samples.len())
        .max()
        .unwrap_or(0) as f64;
    let voice = plan
        .voice_log
        .as_ref()
        .zip(clock)
        .map(|(log, clock)| audio::voice_from_log(&super::source::voices(log), clock, longest));
    let placeholder;
    let cue = match plan.cue.as_deref() {
        Some(cue) => cue,
        None => {
            placeholder = kairo_core::testsig::cue_placeholder(48_000).unwrap_or_default();
            &placeholder
        }
    };
    Some(audio::Audio::new(tracks, game, voice, cue))
}

/// A content check this build can't run yet: an error when it is required and the clip must
/// show the pattern, or does.
fn not_yet(id: &'static str, mode: Mode, plan: &Plan, shows_pattern: bool) -> Finding {
    let required = mode == Mode::Required;
    if required && (plan.content == Content::Required || plan.gate_m3 || shows_pattern) {
        Finding {
            status: Status::Error,
            detail: "this build can't run it yet (research 16's next PRs)".to_owned(),
            ..skipped(id, true, String::new())
        }
    } else {
        skipped(id, required, "this build can't run it yet".to_owned())
    }
}

fn decode_clean(ctx: &Ctx<'_>, decoded: &Decoded) -> Outcome {
    let samples = ctx.samples.len();
    // A decoder that couldn't start, or can't decode the audio, says nothing about the clip
    // (README §6.6: exit 3).
    if let Err(error @ DecodeError::Start(_)) = &decoded.stats {
        return Outcome {
            value: json!({"samples": samples}),
            ..Outcome::cannot(error.to_string())
        };
    }
    let mut outcome = Outcome {
        limit: json!({"errors": 0, "frames": "one per video sample",
                      "audio": "1,024 per AAC packet; the last packet's surplus is reported",
                      "source": "R09:981, README §6.3"}),
        ..Outcome::default()
    };
    let mut audio = Vec::new();
    // Tracks this decoder can't decode: the clip can't be judged on them, but failures found
    // elsewhere still fail it.
    let mut cannot = Vec::new();
    for (i, ((_, track), decoded)) in ctx.audio().zip(&decoded.audio).enumerate() {
        let n = i + 1;
        let packets = track.sizes.len() as u64;
        let ticks: u64 = track
            .stts
            .iter()
            .map(|&(count, delta)| u64::from(count) * u64::from(delta))
            .sum();
        match &decoded.stats {
            Err(error @ (DecodeError::Start(_) | DecodeError::Unsupported(_))) => {
                cannot.push(format!("audio track {n}: {error}"));
            }
            Err(error) => outcome.fails.push(format!("audio track {n}: {error}")),
            Ok(stats) => {
                let want = 1_024 * packets;
                let got = stats.frames as u64;
                outcome.require(stats.errors == 0, || {
                    format!(
                        "audio track {n}: {} decode errors; the last: {}",
                        stats.errors,
                        stats.last_error.as_deref().unwrap_or("?")
                    )
                });
                if !decoded.truncated {
                    outcome.require(got == want, || {
                        format!("audio track {n}: {got} samples decoded from {packets} packets")
                    });
                }
                audio.push(json!({"track": n, "samples": got, "packets": packets,
                                  "surplus": want.saturating_sub(ticks), "truncated": decoded.truncated}));
            }
        }
    }
    match &decoded.stats {
        Err(error) => {
            outcome.value = json!({"samples": samples, "audio": audio});
            outcome.fails.insert(0, error.to_string());
        }
        Ok(stats) => {
            outcome.value = json!({"frames": stats.frames, "samples": samples,
                                   "errors": stats.errors, "audio": audio});
            // The pictures' problems first: they name the clip's defect.
            let mut video = Vec::new();
            if stats.errors != 0 {
                video.push(format!(
                    "{} decode errors; the last: {}",
                    stats.errors,
                    stats.last_error.as_deref().unwrap_or("?")
                ));
            }
            if stats.frames != samples {
                video.push(format!(
                    "{} pictures decoded from {samples} video samples",
                    stats.frames
                ));
            }
            outcome.fails.splice(0..0, video);
            outcome.detail = format!(
                "video: {} of {samples} pictures, audio: {} tracks, no errors",
                stats.frames,
                audio.len()
            );
        }
    }
    if outcome.fails.is_empty() && !cannot.is_empty() {
        return Outcome {
            value: outcome.value,
            ..Outcome::cannot(cannot.join("; "))
        };
    }
    outcome
}

fn barcode(plan: &Plan, readings: &[Reading], logged_tag: Option<u8>) -> Outcome {
    let frames = readings.len();
    let readable = readings.iter().filter(|r| r.code.is_some()).count();
    let torn = readings.iter().filter(|r| r.torn.is_some()).count();
    let share = if frames == 0 {
        0.0
    } else {
        readable as f64 / frames as f64
    };
    let min_share = if plan.profile == Profile::Test {
        1.0
    } else {
        profile::BARCODE_MIN_READABLE
    };
    let margin = readings
        .iter()
        .filter_map(|r| r.code.map(|c| c.margin))
        .min_by(f64::total_cmp);
    let expected_tag = plan
        .expect
        .as_ref()
        .and_then(|e| e.run_tag)
        .and_then(|t| u8::try_from(t).ok());
    // The testbed's log names the run; the expect file must agree with it (README §4.6).
    let tag = logged_tag.or(expected_tag);
    let mut outcome = Outcome {
        value: json!({"frames": frames, "readable": readable, "share": share, "torn": torn,
                      "min_margin": margin, "run_tag": tag}),
        limit: json!({"min_share": min_share, "run_tag": tag, "margin_warn": profile::BARCODE_MARGIN_WARN,
                      "source": "README §6.5 (proposed outside test)"}),
        detail: format!("{readable}/{frames} barcodes, {torn} torn"),
        ..Outcome::default()
    };
    outcome.require(frames > 0, || "no pictures".to_owned());
    if let (Some(logged), Some(expected)) = (logged_tag, expected_tag) {
        outcome.require(logged == expected, || {
            format!("the testbed log's run tag is {logged}, the expect file's {expected}")
        });
    }
    if frames > 0 && readable == 0 {
        // README §4.6 step 1: the layout assumes the clip shows the testbed's client area scaled
        // with its aspect kept, or centre-cropped.
        outcome.fails.push(
            "barcode not found: check geometry (the clip must show the testbed's client area \
             scaled with its aspect kept, or centre-cropped)"
                .to_owned(),
        );
    } else if share < min_share {
        for (index, reading) in readings.iter().enumerate() {
            match (reading.torn, reading.error) {
                (Some((a, b)), _) => outcome.fails.push(format!(
                    "frame {index} is torn: barcode A shows {a}, B shows {b}"
                )),
                (None, Some(error)) => outcome.fails.push(format!("frame {index}: {error}")),
                _ => {}
            }
        }
        outcome.fails.insert(
            0,
            format!(
                "{} of {frames} frames have no readable barcode ({:.2}%, limit {:.2}%)",
                frames - readable,
                100.0 * (1.0 - share),
                100.0 * (1.0 - min_share)
            ),
        );
    }
    if let Some(tag) = tag {
        let wrong: Vec<usize> = readings
            .iter()
            .enumerate()
            .filter(|(_, r)| r.code.is_some_and(|c| c.tag != tag))
            .map(|(i, _)| i)
            .collect();
        if let Some(&first) = wrong.first() {
            outcome.fails.push(format!(
                "{} frames carry another run tag than {tag}, from frame {first}: a clip of another run?",
                wrong.len()
            ));
        }
    }
    if let Some(m) = margin {
        outcome.prefer(m >= profile::BARCODE_MARGIN_WARN, || {
            format!(
                "a barcode read with margin {m:.2}, under {}",
                profile::BARCODE_MARGIN_WARN
            )
        });
    }
    outcome
}

/// The steps set aside as the testbed's own misses, for a detail line.
fn set_aside(counts: &Counts) -> String {
    match counts.source_events.len() {
        0 => String::new(),
        n => format!("; {n} steps at testbed misses set aside (source.stability)"),
    }
}

fn describe(e: &Event, what: &str) -> String {
    format!(
        "frame {}: testbed frame {} → {} ({what})",
        e.frame, e.from, e.to
    )
}

fn drops_dups(plan: &Plan, a: &Analysis) -> Outcome {
    let Some(fps) = a.fps else {
        return Outcome::skip("no whole frame rate to step at");
    };
    let (counts, fs) = match (&a.line, a.counts.as_ref(), a.source_fps) {
        (Ok(_), Some(counts), Some(fs)) => (counts, fs),
        (Err(stall @ (NoLine::Frozen(_) | NoLine::NoAdvance)), _, _) => {
            return stalled(&a.numbers, *stall);
        }
        (Err(reason), _, _) => return Outcome::cannot(format!("no barcode line: {reason}")),
        (Ok(_), _, _) => return Outcome::cannot("the steps couldn't be counted at this rate"),
    };
    let ideal = a.ideal.map_or(0.0, |i| i.0);
    let share = counts.drop_dup_share();
    let excess = share - ideal;
    let test = plan.profile == Profile::Test;
    let mut outcome = Outcome {
        value: json!({"steps": counts.steps, "repeats": counts.repeats, "skipped": counts.skipped,
                      "backwards": counts.backwards, "share": share, "ideal_share": ideal,
                      "excess": excess, "longest_hold": counts.longest_hold,
                      "max_step_ticks": counts.max_step_ticks, "source_fps": fs, "fps": fps,
                      "refresh_hz": a.refresh.map(|r| r.0),
                      "source_events": counts.source_events.len(), "testbed_log": a.source.is_some()}),
        limit: json!({"max_excess": if test { 0.0 } else { profile::DROPS_DUPS_MAX_EXCESS },
                      "backwards": 0,
                      "source": if test { "M1:188, README §2 item 11" } else { "PLAN:351, README §2 item 11 (proposed)" }}),
        detail: format!(
            "{} steps: {} repeats, {} skipped, {} backward; ideal {:.2}%{}",
            counts.steps,
            counts.repeats,
            counts.skipped,
            counts.backwards,
            100.0 * ideal,
            set_aside(counts)
        ),
        ..Outcome::default()
    };
    if let Some((rate, hz)) = a.too_fast {
        outcome.fails.push(format!(
            "the barcodes show a {rate:.2} fps testbed, faster than the expect file's {hz} Hz"
        ));
    }
    let events = || {
        counts.events.iter().map(|e| {
            let what = if e.to == e.from {
                "repeat"
            } else if e.to < e.from {
                "backward"
            } else {
                "skip"
            };
            describe(e, what)
        })
    };
    if test {
        // Gate 1's synthetic clips: not one event (M1:188).
        outcome.fails.extend(events());
    } else {
        outcome.require(excess <= profile::DROPS_DUPS_MAX_EXCESS, || {
            format!(
                "{:.2}% repeated or skipped, {:.2}% above the ideal's {:.2}%",
                100.0 * share,
                100.0 * excess,
                100.0 * ideal
            )
        });
        outcome.require(counts.backwards == 0, || {
            format!("{} steps go back in time", counts.backwards)
        });
        if plan.profile == Profile::Engine {
            let (hold, step) = profile::ENGINE_MAX_HOLD;
            outcome.require(counts.longest_hold <= hold, || {
                format!("one picture held for {} frames", counts.longest_hold)
            });
            outcome.require(counts.max_step_ticks <= step, || {
                format!("a step of {} ticks", counts.max_step_ticks)
            });
        }
        if !outcome.fails.is_empty() {
            outcome.fails.extend(events());
        }
    }
    outcome
}

/// `cadence.drops_dups` for a clip whose testbed frame number never advances: each step between
/// readable frames repeats or goes back, at any rate.
fn stalled(numbers: &[Option<i64>], stall: NoLine) -> Outcome {
    let (mut steps, mut repeats, mut backwards) = (0, 0, 0);
    let mut events = Vec::new();
    for (i, pair) in numbers.windows(2).enumerate() {
        let (Some(from), Some(to)) = (pair[0], pair[1]) else {
            continue;
        };
        steps += 1;
        if to == from {
            repeats += 1;
        } else if to < from {
            backwards += 1;
            events.push(describe(
                &Event {
                    frame: i + 1,
                    from,
                    to,
                },
                "backward",
            ));
        }
    }
    let mut outcome = Outcome {
        value: json!({"steps": steps, "repeats": repeats, "backwards": backwards, "skipped": 0}),
        limit: json!({"advance": "the testbed frame number advances", "source": "README §6.3"}),
        ..Outcome::default()
    };
    outcome.fails.push(format!(
        "{stall}: {repeats} of {steps} steps repeat a picture and {backwards} go back"
    ));
    outcome.fails.extend(events);
    outcome
}

fn judder(plan: &Plan, a: &Analysis) -> Outcome {
    let Some(fps) = a.fps else {
        return Outcome::skip("no whole frame rate to step at");
    };
    if let Err(reason) = &a.line {
        return Outcome::cannot(format!("no barcode line: {reason}"));
    }
    let Some(counts) = a.counts.as_ref() else {
        return Outcome::cannot("the steps couldn't be counted at this rate");
    };
    let Some((_, ideal)) = a.ideal else {
        return Outcome::skip(format!("no pacer ideal at {fps} fps"));
    };
    let share = counts.judder_share();
    let excess = share - ideal;
    // test wants no excess; one step's worth of slack absorbs the ideal's quantisation over a
    // short clip.
    let limit = if plan.profile == Profile::Test {
        1.0 / counts.steps.max(1) as f64
    } else {
        profile::JUDDER_MAX_EXCESS
    };
    let mut outcome = Outcome {
        value: json!({"judder_steps": counts.judder, "steps": counts.steps, "share": share,
                      "ideal_share": ideal, "excess": excess, "refresh_hz": a.refresh.map(|r| r.0),
                      "source_fps": a.source_fps, "p99_step_error_hns": a.p99_step_error_hns,
                      "p99_from": if a.source.as_ref().is_some_and(|s| s.timed) { "the testbed log's display times" } else { "the barcode line" },
                      "source_events": counts.source_events.len()}),
        limit: json!({"max_excess": limit, "source": "README §2 item 12 (proposed outside test; test: one step)"}),
        detail: format!(
            "judder {:.1}%, ideal {:.1}%: excess {:.1} pp{}",
            100.0 * share,
            100.0 * ideal,
            100.0 * excess,
            set_aside(counts)
        ),
        ..Outcome::default()
    };
    // A tiny tolerance for the shares' floating-point division.
    outcome.require(excess <= limit + 1e-9, || {
        format!(
            "judder steps {:.1}% against an ideal {:.1}%: {:.1} pp over",
            100.0 * share,
            100.0 * ideal,
            100.0 * excess
        )
    });
    if !outcome.fails.is_empty() {
        outcome
            .fails
            .extend(counts.judder_events.iter().map(|e| describe(e, "judder")));
    }
    outcome
}

/// `source.stability` (README §6.3, §6.5): how many of the testbed frames the clip spans the
/// testbed itself missed, from its log. Above [`profile::SOURCE_MISSED_MAX`] the run is redone
/// and the clip isn't blamed. A log that doesn't cover the clip, or whose testbed drew at another
/// rate than the clip's barcodes advance, is another run's.
fn source_stability(a: &Analysis) -> Outcome {
    let Some(source) = &a.source else {
        return Outcome::cannot("needs the testbed's log (--testbed-log)");
    };
    let readable: Vec<i64> = a.numbers.iter().flatten().copied().collect();
    let (Some(&first), Some(&last)) = (readable.iter().min(), readable.iter().max()) else {
        return Outcome::cannot("no readable barcode places the clip in the log");
    };
    let st = source.stability(first, last);
    let share = st.share();
    let max = profile::SOURCE_MISSED_MAX;
    let rate = a.line.as_ref().ok().map(|l| l.rate(a.timescale));
    let mut outcome = Outcome {
        value: json!({"first": first, "last": last, "frames": st.frames, "missed": st.missed,
                      "unlogged": st.unlogged, "share": share, "log_fps": source.fps,
                      "barcode_rate": rate, "refresh_hz": source.refresh_hz}),
        limit: json!({"max_share": max, "rate_tolerance": profile::SOURCE_RATE_TOLERANCE,
                      "source": "README §6.3, §6.5 (proposed)"}),
        detail: format!(
            "the testbed missed {} of the clip's {} frames ({:.3}%)",
            st.missed,
            st.frames,
            100.0 * share
        ),
        ..Outcome::default()
    };
    outcome.require(st.unlogged == 0, || {
        format!(
            "the log has no record of {} of the clip's testbed frames {first}–{last}: another run's log?",
            st.unlogged
        )
    });
    if let Some(rate) = rate {
        let fps = f64::from(source.fps);
        outcome.require((rate - fps).abs() <= profile::SOURCE_RATE_TOLERANCE * fps, || {
            format!(
                "the clip's barcodes advance at {rate:.2} fps and the log's testbed drew {fps}: another run's log?"
            )
        });
    }
    outcome.require(share <= max, || {
        format!(
            "rerun: the testbed itself missed {} of the clip's {} frames ({:.3}%, limit {:.1}%); the clip isn't blamed",
            st.missed,
            st.frames,
            100.0 * share,
            100.0 * max
        )
    });
    outcome
}

fn flash(a: &Analysis) -> Outcome {
    let Some((f, flashes)) = a.flashes.as_ref() else {
        return match &a.line {
            Err(reason) => Outcome::cannot(format!("no barcode line: {reason}")),
            Ok(_) => Outcome::cannot("no testbed rate for the flash"),
        };
    };
    if flashes.frames == 0 {
        return Outcome::cannot("no readable frames");
    }
    let mut outcome = Outcome {
        value: json!({"frames": flashes.frames, "lit": flashes.lit, "onsets": flashes.onsets,
                      "wrong": flashes.wrong.len(), "flash_fps": f, "source_fps": a.source_fps}),
        limit: json!({"lit": "N mod F < round(0.1 F)", "F": "the line's rate, or ± 1 where the lit frames fit that better",
                      "source": "README §2 item 8, §6.3"}),
        detail: format!(
            "{} lit frames, {} flashes at F = {f}",
            flashes.lit, flashes.onsets
        ),
        ..Outcome::default()
    };
    for &(frame, n, lit) in &flashes.wrong {
        outcome.fails.push(format!(
            "frame {frame} (testbed frame {n}) is {}",
            if lit {
                "lit but shouldn't be"
            } else {
                "dark but should be lit"
            }
        ));
    }
    outcome
}

/// The scored pictures, with their index.
fn scored(a: &Analysis) -> impl Iterator<Item = (usize, &FrameBars)> {
    a.bars
        .iter()
        .enumerate()
        .filter_map(|(i, b)| b.as_ref().map(|b| (i, b)))
}

fn bars_psnr(plan: &Plan, decoded: &Decoded, a: &Analysis) -> Outcome {
    if decoded.bands.is_none() {
        return Outcome::skip("the picture is too small for the bars band");
    }
    let limit = if plan.profile.is_share_export() {
        profile::EXPORT_BARS_MIN_DB
    } else {
        profile::BARS_MIN_DB
    };
    let worst = scored(a).min_by(|x, y| x.1.score.min().total_cmp(&y.1.score.min()));
    let unscored: Vec<usize> = a
        .bars
        .iter()
        .enumerate()
        .filter(|(_, b)| b.is_none())
        .map(|(i, _)| i)
        .collect();
    let ramp = scored(a)
        .map(|(_, b)| b.ramp_chroma)
        .fold(0.0_f64, f64::max);
    let mut outcome = Outcome {
        value: json!({"frames": a.bars.len(), "scored": a.bars.len() - unscored.len(),
                      "min_db": worst.map(|w| w.1.score.min()), "min_frame": worst.map(|w| w.0),
                      "ramp_chroma_max": ramp}),
        limit: json!({"min_db": limit, "every_frame": true, "ramp_chroma": profile::RAMP_CHROMA_TOLERANCE,
                      "ramp_chroma_in_exports": "warning",
                      "source": if limit < profile::BARS_MIN_DB { "README §2 item 14 (proposed)" } else { "PLAN:351" }}),
        detail: worst.map_or("no frame scored".to_owned(), |w| {
            format!("min {:.1} dB (frame {})", w.1.score.min(), w.0)
        }),
        ..Outcome::default()
    };
    for (i, b) in scored(a) {
        let s = b.score;
        outcome.require(s.min() >= limit, || {
            format!(
                "frame {i} (testbed frame {}): {:.1} dB (Y {:.1}, Cb {:.1}, Cr {:.1})",
                b.frame,
                s.min(),
                s.y,
                s.cb,
                s.cr
            )
        });
        // Exports keep to gross faults (README §2 item 14): there the ramp's chroma only warns.
        if b.ramp_chroma > profile::RAMP_CHROMA_TOLERANCE {
            outcome.violation(
                !plan.profile.is_share_export(),
                format!(
                    "frame {i}: the ramp's chroma is {:.1} codes off grey",
                    b.ramp_chroma
                ),
            );
        }
    }
    // Corruption that also hits the barcode can't hide as unreadable (README §6.3).
    for i in unscored {
        outcome.fails.push(format!(
            "frame {i}: no frame number to score its bars against"
        ));
    }
    if decoded.pending_dropped > 0 {
        outcome.fails.push(format!(
            "{} more unreadable frames weren't kept for scoring",
            decoded.pending_dropped
        ));
    }
    outcome
}

fn colour_matrix(decoded: &Decoded, a: &Analysis) -> Outcome {
    if decoded.bands.is_none() {
        return Outcome::skip("the picture is too small for the bars band");
    }
    let margin = profile::COLOUR_MATRIX_MARGIN_DB;
    let mut outcome = Outcome {
        limit: json!({"margin_db": margin, "source": "README §2 item 14 (proposed)"}),
        detail: "BT.709 limited fits best".to_owned(),
        ..Outcome::default()
    };
    let mut worst: Option<(usize, &'static str, f64)> = None;
    for (i, b) in scored(a) {
        let gain = b.best_other.1 - b.score.min();
        if worst.is_none_or(|w| gain > w.2) {
            worst = Some((i, b.best_other.0.name(), gain));
        }
        outcome.require(gain < margin, || {
            format!(
                "frame {i}: {} fits {gain:.1} dB better than BT.709 limited",
                b.best_other.0.name()
            )
        });
    }
    outcome.value = json!({"best_other": worst.map(|w| w.1), "gain_db": worst.map(|w| w.2)});
    outcome
}

fn bars_lines(plan: &Plan, decoded: &Decoded, a: &Analysis) -> Outcome {
    if decoded.bands.is_none() {
        return Outcome::skip("the picture is too small for the bars band");
    }
    let (limit, source) = match plan.bars_lines_codes {
        Some(codes) => (
            codes,
            "the corpus's calibration, max(4, 2 × its worst good clip) (README §7.4)",
        ),
        None => (
            profile::BARS_LINES_MAX_CODES,
            "README §2 item 14 (proposed)",
        ),
    };
    let worst = scored(a).map(|(_, b)| b.line).fold(0.0_f64, f64::max);
    let mut outcome = Outcome {
        value: json!({"max_codes": worst}),
        limit: json!({"max_codes": limit, "source": source}),
        detail: format!("largest row or column residual {worst:.2} codes"),
        ..Outcome::default()
    };
    for (i, b) in scored(a) {
        outcome.require(b.line <= limit, || {
            format!("frame {i}: a row or column {:.1} codes off", b.line)
        });
    }
    outcome
}

fn patch_levels(decoded: &Decoded, hdr: bool) -> Outcome {
    let tolerance = profile::LEVELS_TOLERANCE;
    let measured: usize = decoded.patches.iter().map(Vec::len).sum();
    if measured == 0 {
        return Outcome::skip("no whole patch on a readable frame");
    }
    let worst = decoded
        .patches
        .iter()
        .flatten()
        .map(|p| (p.measured - p.expected).abs())
        .fold(0.0_f64, f64::max);
    let mut outcome = Outcome {
        value: json!({"patches": measured, "worst_error": worst}),
        limit: json!({"codes": tolerance, "top": if hdr { "one-sided at 235" } else { "two-sided" },
                      "source": if hdr { "PLAN:430, signals.md §4" } else { "README §6.5" }}),
        detail: format!("{measured} patches within ±{tolerance} codes"),
        ..Outcome::default()
    };
    for (i, patches) in decoded.patches.iter().enumerate() {
        for p in patches {
            outcome.require(levels::within(p, tolerance, hdr), || {
                format!(
                    "frame {i}: patch {} shows Y {:.1}, expected {:.1}",
                    p.k, p.measured, p.expected
                )
            });
        }
    }
    outcome
}

/// The content summary line, from the findings' values: barcodes read, drops and duplicates,
/// judder; or what was decoded when the barcodes weren't judged. `None` when no decoder ran.
pub(crate) fn summary(findings: &[Finding]) -> Option<String> {
    let finding = |id: &str| findings.iter().find(|f| f.id == id);
    let value = |id: &str| finding(id).map(|f| &f.value);
    let Some(barcode) = value("content.barcode").filter(|v| v.get("readable").is_some()) else {
        let clean = finding("decode.clean").filter(|f| f.status != Status::Skip)?;
        let why = finding("content.barcode").map_or("", |f| f.detail.as_str());
        return Some(match clean.value.get("frames") {
            Some(frames) => format!("{frames} pictures decoded; barcodes not judged: {why}"),
            None => format!("not decoded: {}", clean.detail),
        });
    };
    let mut parts = vec![format!(
        "{}/{} barcodes",
        barcode.get("readable")?,
        barcode.get("frames")?
    )];
    if let Some(d) = value("cadence.drops_dups").filter(|v| v.get("steps").is_some()) {
        parts.push(format!(
            "{} skipped, {} repeats",
            d.get("skipped")?,
            d.get("repeats")?
        ));
    }
    if let Some(excess) = value("cadence.judder").and_then(|v| v.get("excess")?.as_f64()) {
        parts.push(format!("judder excess {:.1} pp", 100.0 * excess));
    }
    if let Some(min) = value("content.bars").and_then(|v| v.get("min_db")?.as_f64()) {
        parts.push(format!("bars min {min:.1} dB"));
    }
    Some(parts.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::expect::Expect;
    use kairo_core::testsig::Barcode;

    /// A decoder with no pictures and `chunks` chunks of 1,024 stereo sample frames per track.
    struct Chunks(usize);

    impl Decoder for Chunks {
        fn kind(&self) -> &'static str {
            "synthetic"
        }

        fn video(
            &mut self,
            _: &mut dyn FnMut(usize, &crate::decode::Planes<'_>) -> ControlFlow<()>,
        ) -> Result<DecodeStats, DecodeError> {
            Ok(DecodeStats::default())
        }

        fn audio(
            &mut self,
            _: usize,
            channels: usize,
            each: &mut dyn FnMut(&[f32]) -> ControlFlow<()>,
        ) -> Result<DecodeStats, DecodeError> {
            let chunk = vec![0.25_f32; 1_024 * channels];
            let mut stats = DecodeStats::default();
            for _ in 0..self.0 {
                stats.frames += 1_024;
                if each(&chunk).is_break() {
                    break;
                }
            }
            Ok(stats)
        }
    }

    #[test]
    fn audio_fills_its_packets_room_and_stops_at_the_cap() {
        // Review F8: room for exactly 1,024 frames a packet, so no doubling past the cap.
        let decoded = decode_capped(&mut Chunks(3), &plan(Profile::Test), &[(2, 3)], 10_000);
        let track = &decoded.audio[0];
        assert_eq!((track.samples.len(), track.truncated), (3_072, false));
        assert!(
            track.samples.capacity() < 4_096,
            "{}",
            track.samples.capacity()
        );
        assert!(track.samples.iter().all(|&v| v == 0.25));
        // Cut at the cap: kept up to it and marked, so the checks judge the start only.
        let decoded = decode_capped(&mut Chunks(8), &plan(Profile::Test), &[(1, 8)], 5_000);
        let track = &decoded.audio[0];
        assert_eq!((track.samples.len(), track.truncated), (5_000, true));
    }

    fn plan(profile: Profile) -> Plan {
        let text = r#"{"kairo_expect":1,"pattern":1,"fps":60,"source_hz":180,"seconds":20,
            "tracks":[]}"#;
        Plan::new(profile, false, Some(Expect::parse(text).unwrap())).unwrap()
    }

    /// What 20 s of 180 fps content at 60 fps adds up to, with these frame numbers.
    fn analysis(numbers: Vec<Option<i64>>) -> Analysis {
        let counts = cadence::count(&numbers, 180, 60);
        Analysis {
            numbers,
            line: Ok(Line {
                alpha: 0.0,
                beta: 256.0 / 3.0,
                inliers: 1_200,
            }),
            source_fps: Some(180),
            too_fast: None,
            refresh: Some((180.0, 1)),
            fps: Some(60),
            counts,
            ideal: Some((0.0, 0.0)),
            flashes: None,
            p99_step_error_hns: None,
            bars: Vec::new(),
            pts: Vec::new(),
            source: None,
            timescale: 15_360,
        }
    }

    fn numbers(n: impl Fn(i64) -> i64) -> Vec<Option<i64>> {
        (0..1_200).map(|k| Some(n(k))).collect()
    }

    fn status(outcome: Outcome) -> Status {
        weigh("x", outcome, Mode::Required).status
    }

    #[test]
    fn capture_and_engine_tolerate_what_test_does_not() {
        // One drop in 1,199 steps (0.08%) and judder on 1% of the steps pass capture and fail
        // test; a picture held for three frames fails engine only (review F10).
        let dropped = analysis(numbers(|k| 1_000 + 3 * (k + i64::from(k >= 600))));
        let juddering = analysis(numbers(|k| 1_000 + 3 * k - i64::from(k % 100 == 50)));
        let hold = analysis(numbers(|k| match k {
            601 | 602 => 1_000 + 3 * 600,
            _ => 1_000 + 3 * k,
        }));
        for (profile, want) in [
            (Profile::Capture, Status::Pass),
            (Profile::Test, Status::Fail),
        ] {
            let plan = plan(profile);
            assert_eq!(status(drops_dups(&plan, &dropped)), want, "{profile}: drop");
            assert_eq!(status(judder(&plan, &juddering)), want, "{profile}: judder");
        }
        assert_eq!(
            status(drops_dups(&plan(Profile::Capture), &hold)),
            Status::Pass
        );
        assert_eq!(
            status(drops_dups(&plan(Profile::Engine), &hold)),
            Status::Fail
        );
    }

    #[test]
    fn one_unreadable_frame_in_a_thousand_passes_capture() {
        let code = Barcode {
            frame24: 0,
            hdr: false,
            tag: 0,
            margin: 1.0,
            white: 235.0,
            black: 16.0,
        };
        let mut readings: Vec<Reading> = (0..1_200)
            .map(|_| Reading {
                code: Some(code),
                ..Reading::default()
            })
            .collect();
        readings[700].code = None;
        assert_eq!(
            status(barcode(&plan(Profile::Capture), &readings, None)),
            Status::Pass
        );
        assert_eq!(
            status(barcode(&plan(Profile::Test), &readings, None)),
            Status::Fail
        );
        // None readable: the geometry is named.
        let blank = vec![Reading::default(); 10];
        let outcome = barcode(&plan(Profile::Capture), &blank, None);
        assert!(outcome.fails[0].starts_with("barcode not found: check geometry"));
    }

    #[test]
    fn checks_that_cannot_run_say_why() {
        let mut frozen = analysis(numbers(|_| 1_000));
        frozen.line = Err(NoLine::Frozen(1_000));
        frozen.counts = None;
        let plan = plan(Profile::Engine);
        let outcome = drops_dups(&plan, &frozen);
        assert!(
            outcome.fails[0].starts_with("every readable frame after frame 0 shows"),
            "{:?}",
            outcome.fails
        );
        assert_eq!(status(judder(&plan, &frozen)), Status::Error);
        assert_eq!(status(flash(&frozen)), Status::Error);
        let mut few = analysis(vec![Some(1_000)]);
        few.line = Err(NoLine::TooFew);
        few.counts = None;
        assert_eq!(status(drops_dups(&plan, &few)), Status::Error);
        // Not required, it skips.
        let outcome = weigh("x", judder(&plan, &frozen), Mode::Warning);
        assert_eq!(outcome.status, Status::Skip);
        // A testbed faster than the expect file says fails.
        let mut fast = analysis(numbers(|k| 1_000 + 3 * k));
        fast.too_fast = Some((200.0, 180));
        assert_eq!(status(drops_dups(&plan, &fast)), Status::Fail);
    }
}
