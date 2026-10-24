// SPDX-License-Identifier: GPL-3.0-or-later
//! The per-source audio aligner (docs/research/10-audio-rust.md §5.4 and §5.6): it places one
//! capture source's packets on the 48 kHz timeline, follows the drift of the source's clock with
//! a PI loop that steers rubato's resampler, puts silence in place of lost audio and splices
//! over gaps, overlaps and pauses with short fades.
//!
//! Each packet carries the QPC time of its first frame, turned into a timeline sample with
//! [`crate::timeline::sample_at`]. A source's clock drifts from QPC by up to a few hundred ppm
//! and its timestamps jitter by milliseconds, so the loop low-passes the error between where a
//! packet's timestamp says it belongs and where the resampler would put it, and trims the
//! resample ratio to keep that error near zero.
//!
//! The mixer reads 10 ms blocks about 200 ms behind real time (research 10 §5.2), whether or not
//! packets arrive, and the aligner keeps track of where it has read. Audio that arrives after
//! the mixer has read its place is dropped, never shifted, and no splice cuts audio the mixer
//! has already played. When a source goes quiet for longer than the mixer's delay, its last
//! audio is pushed out of the resampler and faded out before the mixer reaches its end, and its
//! next packet re-anchors. After [`SourceAligner::new`], nothing allocates except an error's
//! message.

use std::collections::VecDeque;
use std::fmt;

use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Adjustable, Async, FixedAsync, Indexing, PolynomialDegree, Resampler};

use crate::timeline::SAMPLE_RATE;

/// Frames per resampler chunk: 10 ms at 48 kHz.
pub const CHUNK_FRAMES: usize = 480;
/// The most aligned audio that can wait for the mixer: 1.25 s. That is the mixer's 1-second
/// catch-up after a stall, plus its 200 ms delay and a packet, so a backlog pushed before the
/// mixer reads it still fits.
pub const MAX_WAITING_FRAMES: usize = 60_000;
/// An error larger than this, in samples (50 ms), re-anchors the source instead of steering.
const HARD: f64 = 2_400.0;
/// The innovation clip: one packet's error counts at most this far from the low-passed error, in
/// samples (2 ms), so an outlier moves the low-passed error by at most `ALPHA × CLIP`, about 1.9
/// samples.
const CLIP: f64 = 96.0;
/// Each packet's weight in the low-passed error: about a 0.5 s time constant.
const ALPHA: f64 = 0.02;
/// The loop's time constant, in seconds.
const TAU_S: f64 = 15.0;
/// Proportional gain, per sample of error.
const KP: f64 = 2.0 / (TAU_S * SAMPLE_RATE as f64);
/// Integral gain, per sample-second of error.
const KI: f64 = KP / TAU_S;
/// The ratio stays within ±1,000 ppm of 1 (rubato is built to allow ±1%).
const LIMIT: f64 = 1.001;
/// Splices fade out and in over 2 ms (research 10 suggests 2 to 5 ms).
const FADE_FRAMES: usize = 96;
/// How far the resampler's output trails its input: rubato 5.0.1's Septic interpolator puts
/// input frame `n` at output frame `n + 3`, one frame earlier than its `output_delay()` of 4
/// suggests. `rubato_puts_input_frame_n_at_output_frame_n_plus_3` checks it.
const RESAMPLER_LAG: i64 = 3;

/// One packet from a capture source.
#[derive(Clone, Copy)]
pub struct AudioPacket<'a> {
    /// The timeline sample of the packet's first frame: `sample_at` of its QPC timestamp.
    pub ts: i64,
    /// Interleaved 48 kHz samples, one per channel per frame.
    pub data: &'a [f32],
    /// WASAPI's data-discontinuity flag: the device lost audio before this packet, which
    /// re-anchors at `ts`.
    pub discontinuity: bool,
    /// WASAPI's timestamp-error flag: `ts` is unreliable, so the packet is placed by prediction.
    /// The first packet, and one that follows a discontinuity or a pause, still anchor at `ts`,
    /// since nothing better is known then.
    pub timestamp_error: bool,
    /// Frames the capture thread dropped since its previous packet because the queue to the
    /// mixer had no room; it queues a packet only when there is room for both its samples and
    /// its timestamp. Up to 50 ms of them become silence, faded at both ends, without a
    /// re-anchor; more is left to the timestamp check.
    pub lost_frames: u32,
}

impl fmt::Debug for AudioPacket<'_> {
    /// Leaves the samples out: logs never hold audio.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AudioPacket")
            .field("ts", &self.ts)
            .field("samples", &self.data.len())
            .field("discontinuity", &self.discontinuity)
            .field("timestamp_error", &self.timestamp_error)
            .field("lost_frames", &self.lost_frames)
            .finish()
    }
}

/// What the aligner has done so far: the source's health counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AlignerStats {
    /// Packets pushed.
    pub packets: u64,
    /// Packets flagged with a timestamp error.
    pub timestamp_errors: u64,
    /// Re-anchors after the device flagged a discontinuity.
    pub discontinuity_resyncs: u64,
    /// Re-anchors after the source's position jumped more than 50 ms ahead.
    pub gap_resyncs: u64,
    /// Re-anchors after the source's position jumped more than 50 ms back.
    pub overlap_resyncs: u64,
    /// Times the source went quiet long enough for the mixer to reach the end of its audio,
    /// which was then faded out.
    pub stalls: u64,
    /// Lost frames replaced with silence.
    pub filled_frames: u64,
    /// Frames dropped because the mixer had already read their place, or older audio being
    /// faded out still held it.
    pub late_frames: u64,
    /// Frames the mixer read that had no audio, and got silence.
    pub underrun_frames: u64,
    /// Frames dropped because [`MAX_WAITING_FRAMES`] were already waiting for the mixer.
    pub overflow_frames: u64,
}

/// Why the aligner refused a setting or a buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AlignError {
    /// Only mono (the microphone) and stereo sources are supported.
    Channels(usize),
    /// rubato refused a setting or a buffer; the message is rubato's.
    Resampler(String),
    /// A packet's length is not a whole number of frames.
    RaggedPacket {
        /// The packet's length in samples.
        len: usize,
        /// The source's channels.
        channels: usize,
    },
    /// A block's length is not a whole number of frames.
    RaggedBlock {
        /// The block's length in samples.
        len: usize,
        /// The source's channels.
        channels: usize,
    },
}

impl fmt::Display for AlignError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Channels(channels) => {
                write!(f, "a source must have 1 or 2 channels, not {channels}")
            }
            Self::Resampler(message) => write!(f, "the resampler refused: {message}"),
            Self::RaggedPacket { len, channels } => write!(
                f,
                "a packet of {len} samples is not whole frames of {channels} channels"
            ),
            Self::RaggedBlock { len, channels } => write!(
                f,
                "a block of {len} samples is not whole frames of {channels} channels"
            ),
        }
    }
}

impl std::error::Error for AlignError {}

/// Why the aligner re-anchors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Resync {
    /// The first packet.
    Initial,
    /// The device flagged a discontinuity.
    Discontinuity,
    /// The source's position is more than 50 ms ahead of where its audio would land.
    Gap,
    /// The source's position is more than 50 ms behind.
    Overlap,
    /// The first packet after the source went quiet, within 50 ms of its prediction.
    Resume,
}

impl Resync {
    /// The kind of re-anchor for a packet `error` samples from its prediction after a pause.
    fn after_pause(error: f64) -> Self {
        if error > HARD {
            Self::Gap
        } else if error < -HARD {
            Self::Overlap
        } else {
            Self::Resume
        }
    }
}

/// What the loop decided for one packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    /// Re-anchor so the packet's first frame lands at this timeline sample.
    Resync(Resync, i64),
    /// The timestamp is unreliable: place the packet by prediction at the estimated rate.
    Hold,
    /// The loop updated the ratio.
    Track,
}

/// The PI loop (research 10 §5.4), kept apart from the audio so its maths can be tested over
/// long runs cheaply.
#[derive(Debug, Clone, Copy)]
struct ClockLoop {
    /// Timeline sample where the next input frame will land.
    pred: f64,
    /// The low-passed error between timestamps and `pred`, in samples.
    err_lp: f64,
    /// The low-passed error of a run of flagged timestamps, kept apart and never steered by.
    flag_lp: f64,
    /// The integrated error, in sample-seconds: the estimate of the source's rate.
    integ: f64,
    /// Output frames per input frame.
    ratio: f64,
    locked: bool,
}

impl ClockLoop {
    fn new() -> Self {
        Self {
            pred: 0.0,
            err_lp: 0.0,
            flag_lp: 0.0,
            integ: 0.0,
            ratio: 1.0,
            locked: false,
        }
    }

    /// Decides how to place a packet of `frames` frames stamped `ts`, and updates the loop.
    /// `paused` says the source went quiet before this packet.
    ///
    /// A packet flagged with a timestamp error is placed at the estimated rate, without the
    /// proportional term, which only steers phase from timestamps that are now unreliable, so a
    /// storm of bad timestamps neither re-anchors nor steers. Its clipped error still feeds a
    /// separate low-pass: if a long run of flagged packets drifts more than 50 ms that way, the
    /// source re-anchors where that low-pass puts it, never at a raw flagged timestamp. The
    /// integral stops growing while the ratio is held at its limit (anti-windup), so a loss the
    /// device doesn't report can't wind it up into a long overshoot.
    fn step(
        &mut self,
        ts: i64,
        frames: usize,
        discontinuity: bool,
        timestamp_error: bool,
        paused: bool,
    ) -> Step {
        if !self.locked {
            return Step::Resync(Resync::Initial, ts);
        }
        if discontinuity {
            return Step::Resync(Resync::Discontinuity, ts);
        }
        let error = ts as f64 - self.pred;
        if paused {
            return Step::Resync(Resync::after_pause(error), ts);
        }
        if timestamp_error {
            self.flag_lp += ALPHA * (error - self.flag_lp).clamp(-CLIP, CLIP);
            if self.flag_lp.abs() > HARD {
                let why = if self.flag_lp > 0.0 {
                    Resync::Gap
                } else {
                    Resync::Overlap
                };
                return Step::Resync(why, (self.pred + self.flag_lp).round() as i64);
            }
            self.ratio = rate_ratio(self.integ);
            return Step::Hold;
        }
        self.flag_lp = 0.0;
        if error > HARD {
            return Step::Resync(Resync::Gap, ts);
        }
        if error < -HARD {
            return Step::Resync(Resync::Overlap, ts);
        }
        self.err_lp += ALPHA * (error - self.err_lp).clamp(-CLIP, CLIP);
        let step = self.err_lp * frames as f64 / SAMPLE_RATE as f64;
        let raw = 1.0 + KP * self.err_lp + KI * (self.integ + step);
        let winding_up = (raw > LIMIT && step > 0.0) || (raw < 1.0 / LIMIT && step < 0.0);
        if !winding_up {
            self.integ += step;
        }
        self.ratio = (1.0 + KP * self.err_lp + KI * self.integ).clamp(1.0 / LIMIT, LIMIT);
        Step::Track
    }

    /// Re-anchors so the next input frame lands at timeline sample `at`, keeping the rate
    /// estimate.
    fn anchor(&mut self, at: i64) {
        self.pred = at as f64;
        self.err_lp = 0.0;
        self.flag_lp = 0.0;
        self.ratio = rate_ratio(self.integ);
        self.locked = true;
    }

    /// Moves the prediction past `frames` input frames.
    fn advance(&mut self, frames: usize) {
        self.pred += frames as f64 * self.ratio;
    }

    /// The timeline sample where the next input frame lands, rounded.
    fn next_frame(&self) -> i64 {
        self.pred.round() as i64
    }
}

/// The ratio from the rate estimate alone.
fn rate_ratio(integ: f64) -> f64 {
    (1.0 + KI * integ).clamp(1.0 / LIMIT, LIMIT)
}

/// One source's aligner.
pub struct SourceAligner {
    channels: usize,
    resampler: Async<f32>,
    /// Input waiting for a whole chunk.
    input: Vec<f32>,
    /// The resampler's output for one chunk.
    scratch: Vec<f32>,
    /// Aligned audio the mixer hasn't read, starting at timeline sample `out_pos`.
    out: VecDeque<f32>,
    /// The timeline sample of `out`'s first frame, or where the next frame goes when it is
    /// empty. It never moves past the audio itself, even when the mixer has read beyond it.
    out_pos: i64,
    /// The end of the last block the mixer read: everything before it has been played.
    read_end: i64,
    /// Output frames still to drop after a splice: new audio for places already taken.
    discard: usize,
    clock: ClockLoop,
    /// Frames of fade-in done since the last splice; `FADE_FRAMES` when none is running.
    faded_in: usize,
    /// The source went quiet and its audio was faded out; the next packet re-anchors.
    paused: bool,
    stats: AlignerStats,
}

impl fmt::Debug for SourceAligner {
    /// Leaves the audio out: logs never hold it.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SourceAligner")
            .field("channels", &self.channels)
            .field("waiting_frames", &(self.out.len() / self.channels))
            .field("out_pos", &self.out_pos)
            .field("read_end", &self.read_end)
            .field("clock", &self.clock)
            .field("paused", &self.paused)
            .field("stats", &self.stats)
            .finish_non_exhaustive()
    }
}

impl SourceAligner {
    /// An aligner for a 48 kHz source with `channels` interleaved channels (1 or 2).
    pub fn new(channels: usize) -> Result<Self, AlignError> {
        if !(1..=2).contains(&channels) {
            return Err(AlignError::Channels(channels));
        }
        let resampler = Async::<f32>::new_poly(
            1.0,
            1.01,
            PolynomialDegree::Septic,
            CHUNK_FRAMES,
            channels,
            FixedAsync::Input,
        )
        .map_err(resampler_error)?;
        let scratch = vec![0.0; resampler.output_frames_max() * channels];
        Ok(Self {
            channels,
            resampler,
            input: Vec::with_capacity(CHUNK_FRAMES * channels),
            scratch,
            out: VecDeque::with_capacity(MAX_WAITING_FRAMES * channels),
            out_pos: 0,
            read_end: i64::MIN,
            discard: 0,
            clock: ClockLoop::new(),
            faded_in: FADE_FRAMES,
            paused: false,
            stats: AlignerStats::default(),
        })
    }

    /// Takes a packet from the source.
    pub fn push(&mut self, packet: &AudioPacket<'_>) -> Result<(), AlignError> {
        let channels = self.channels;
        if !packet.data.len().is_multiple_of(channels) {
            return Err(AlignError::RaggedPacket {
                len: packet.data.len(),
                channels,
            });
        }
        let frames = packet.data.len() / channels;
        self.stats.packets += 1;
        if packet.timestamp_error {
            self.stats.timestamp_errors += 1;
        }
        let lost = usize::try_from(packet.lost_frames).unwrap_or(usize::MAX);
        if self.clock.locked && !self.paused && lost > 0 && lost as f64 <= HARD {
            self.fill(lost)?;
        }
        match self.clock.step(
            packet.ts,
            frames,
            packet.discontinuity,
            packet.timestamp_error,
            self.paused,
        ) {
            Step::Resync(why, at) => self.resync(at, why)?,
            Step::Hold | Step::Track => self
                .resampler
                .set_resample_ratio_relative(self.clock.ratio, true)
                .map_err(resampler_error)?,
        }
        self.clock.advance(frames);
        self.feed(Some(packet.data), frames)
    }

    /// Fills `dst` (interleaved, a whole number of frames) with the aligned audio for timeline
    /// samples from `pos` on. Silence stands in for what hasn't arrived; audio for samples
    /// before `pos` that is still waiting arrived too late to be mixed and is dropped. If the
    /// source has gone quiet and its audio would run out within this block plus 2 ms, the rest
    /// of it is pushed out and faded out first.
    pub fn read_block(&mut self, pos: i64, dst: &mut [f32]) -> Result<(), AlignError> {
        let channels = self.channels;
        if !dst.len().is_multiple_of(channels) {
            return Err(AlignError::RaggedBlock {
                len: dst.len(),
                channels,
            });
        }
        let frames = dst.len() / channels;
        if self.clock.locked && !self.paused {
            let waiting_end = self
                .out_pos
                .saturating_add(to_i64(self.out.len() / channels));
            if waiting_end < pos.saturating_add(to_i64(frames + FADE_FRAMES)) {
                self.flush()?;
                // The flush pads with silence past the source's last frame; cut there, unless
                // older audio still being faded out after a splice reaches further.
                self.cut_and_fade(self.clock.next_frame().max(waiting_end));
                self.paused = true;
                self.stats.stalls += 1;
            }
        }
        dst.fill(0.0);
        if self.out_pos < pos {
            let late = usize::try_from(pos.saturating_sub(self.out_pos))
                .unwrap_or(usize::MAX)
                .min(self.out.len() / channels);
            self.out.drain(..late * channels);
            self.out_pos = self.out_pos.saturating_add(to_i64(late));
            self.stats.late_frames += to_u64(late);
        }
        let lead = usize::try_from(self.out_pos.saturating_sub(pos).max(0))
            .unwrap_or(usize::MAX)
            .min(frames);
        let take = (self.out.len() / channels).min(frames - lead);
        for (slot, sample) in dst
            .iter_mut()
            .skip(lead * channels)
            .zip(self.out.drain(..take * channels))
        {
            *slot = sample;
        }
        self.out_pos = self.out_pos.saturating_add(to_i64(take));
        self.read_end = self.read_end.max(pos.saturating_add(to_i64(frames)));
        self.stats.underrun_frames += to_u64(frames - take);
        Ok(())
    }

    /// Output frames per input frame: below 1 when the source's clock runs fast.
    pub fn ratio(&self) -> f64 {
        self.clock.ratio
    }

    /// What the aligner has done so far.
    pub fn stats(&self) -> AlignerStats {
        self.stats
    }

    /// Re-anchors so the packet being pushed lands at timeline sample `at`.
    fn resync(&mut self, at: i64, why: Resync) -> Result<(), AlignError> {
        let old_end = if why == Resync::Initial {
            None
        } else {
            if !self.paused {
                self.flush()?;
            }
            Some(self.clock.next_frame())
        };
        self.resampler.reset();
        self.splice(old_end, at.saturating_sub(RESAMPLER_LAG));
        self.clock.anchor(at);
        self.resampler
            .set_resample_ratio_relative(self.clock.ratio, false)
            .map_err(resampler_error)?;
        self.paused = false;
        match why {
            Resync::Initial | Resync::Resume => {}
            Resync::Discontinuity => self.stats.discontinuity_resyncs += 1,
            Resync::Gap => self.stats.gap_resyncs += 1,
            Resync::Overlap => self.stats.overlap_resyncs += 1,
        }
        Ok(())
    }

    /// Puts `lost` frames of silence where the next input frame would go, faded like any splice,
    /// and carries the loop on without re-anchoring it.
    fn fill(&mut self, lost: usize) -> Result<(), AlignError> {
        self.flush()?;
        let old_end = self.clock.next_frame();
        self.clock.advance(lost);
        self.resampler.reset();
        self.splice(
            Some(old_end),
            self.clock.next_frame().saturating_sub(RESAMPLER_LAG),
        );
        self.resampler
            .set_resample_ratio_relative(self.clock.ratio, false)
            .map_err(resampler_error)?;
        self.stats.filled_frames += to_u64(lost);
        Ok(())
    }

    /// Joins new audio, whose first output frame falls at timeline sample `start`, to the
    /// waiting audio, which really ends at `old_end` (`None` before the first packet).
    ///
    /// Nothing the mixer has read is cut. The new audio starts at `start`, or later: at the
    /// mixer's position if it has passed `start`, and on an overlap with waiting audio, after
    /// 2 ms of that audio, so it can fade out. The old audio stops there or at its own end,
    /// fading out over its last 2 ms; silence fills any gap; the new audio's frames before its
    /// starting point are dropped as late, and the rest fades in.
    fn splice(&mut self, old_end: Option<i64>, start: i64) {
        let unread = self.read_end;
        let first_new = match old_end {
            Some(end) if start < end && end > unread => {
                start.max(end.min(unread.saturating_add(to_i64(FADE_FRAMES))))
            }
            _ => start.max(unread),
        };
        // After a pause the audio's end was faded already; fading again where it is cut keeps
        // the end smooth even when the new audio starts earlier.
        let cut = old_end.map_or(first_new, |end| end.min(first_new));
        self.cut_and_fade(cut);
        if !self.out.is_empty() {
            let end = self
                .out_pos
                .saturating_add(to_i64(self.out.len() / self.channels));
            let pad = usize::try_from(first_new.saturating_sub(end).max(0)).unwrap_or(usize::MAX);
            if pad > MAX_WAITING_FRAMES {
                // A position far ahead: nothing waiting can be kept next to that much silence.
                self.stats.overflow_frames += to_u64(self.out.len() / self.channels);
                self.out.clear();
            } else {
                self.make_room(pad);
                self.out
                    .extend(std::iter::repeat_n(0.0, pad * self.channels));
            }
        }
        if self.out.is_empty() {
            self.out_pos = first_new;
        }
        self.discard = usize::try_from(first_new.saturating_sub(start)).unwrap_or(usize::MAX);
        self.faded_in = 0;
    }

    /// Cuts the waiting audio at timeline sample `at` and fades out its last 2 ms.
    fn cut_and_fade(&mut self, at: i64) {
        let channels = self.channels;
        let keep = self.truncate_at(at);
        let fade = keep.min(FADE_FRAMES);
        for frame in 0..fade {
            let gain = (fade - 1 - frame) as f32 / fade as f32;
            let first = (keep - fade + frame) * channels;
            for sample in self.out.range_mut(first..first + channels) {
                *sample *= gain;
            }
        }
    }

    /// Drops the waiting audio from timeline sample `at` on, and returns the frames kept.
    fn truncate_at(&mut self, at: i64) -> usize {
        let keep = usize::try_from(at.saturating_sub(self.out_pos).max(0))
            .unwrap_or(usize::MAX)
            .min(self.out.len() / self.channels);
        self.out.truncate(keep * self.channels);
        keep
    }

    /// Feeds `frames` frames to the resampler: from `data`, or silence.
    fn feed(&mut self, data: Option<&[f32]>, frames: usize) -> Result<(), AlignError> {
        let channels = self.channels;
        let mut done = 0;
        while done < frames {
            let take = (CHUNK_FRAMES - self.input.len() / channels).min(frames - done);
            match data.and_then(|data| data.get(done * channels..(done + take) * channels)) {
                Some(samples) => self.input.extend_from_slice(samples),
                None => self.input.resize(self.input.len() + take * channels, 0.0),
            }
            done += take;
            if self.input.len() == CHUNK_FRAMES * channels {
                self.process_chunk(None)?;
            }
        }
        Ok(())
    }

    /// Pushes everything the resampler still holds out to the waiting audio: the input waiting
    /// for a whole chunk, padded with silence, and the 8 frames rubato holds back after each
    /// chunk, which take a second, empty chunk when the first was partly full.
    fn flush(&mut self) -> Result<(), AlignError> {
        let waiting = self.input.len() / self.channels;
        self.process_chunk(Some(waiting))?;
        if waiting > 0 {
            self.process_chunk(Some(0))?;
        }
        Ok(())
    }

    /// Resamples the waiting input, a whole chunk or (`partial`) fewer frames padded with
    /// silence, and queues the output for the mixer.
    fn process_chunk(&mut self, partial: Option<usize>) -> Result<(), AlignError> {
        let channels = self.channels;
        let input =
            InterleavedSlice::new(self.input.as_slice(), channels, self.input.len() / channels)
                .map_err(resampler_error)?;
        let capacity = self.scratch.len() / channels;
        let mut output = InterleavedSlice::new_mut(self.scratch.as_mut_slice(), channels, capacity)
            .map_err(resampler_error)?;
        let indexing = partial.map(|frames| Indexing::new().partial_len(frames));
        let (_, written) = self
            .resampler
            .process_into_buffer(&input, &mut output, indexing.as_ref())
            .map_err(resampler_error)?;
        self.input.clear();
        let dropped = written.min(self.discard);
        self.discard -= dropped;
        self.stats.late_frames += to_u64(dropped);
        self.make_room(written - dropped);
        for frame in self
            .scratch
            .chunks_exact(channels)
            .take(written)
            .skip(dropped)
        {
            let gain = if self.faded_in < FADE_FRAMES {
                self.faded_in += 1;
                self.faded_in as f32 / FADE_FRAMES as f32
            } else {
                1.0
            };
            self.out.extend(frame.iter().map(|sample| sample * gain));
        }
        Ok(())
    }

    /// Drops the oldest waiting frames so `frames` more fit in [`MAX_WAITING_FRAMES`].
    fn make_room(&mut self, frames: usize) {
        let channels = self.channels;
        let waiting = self.out.len() / channels;
        let excess = (waiting + frames)
            .saturating_sub(MAX_WAITING_FRAMES)
            .min(waiting);
        if excess > 0 {
            self.out.drain(..excess * channels);
            self.out_pos = self.out_pos.saturating_add(to_i64(excess));
            self.stats.overflow_frames += to_u64(excess);
        }
    }
}

fn resampler_error(error: impl fmt::Display) -> AlignError {
    AlignError::Resampler(error.to_string())
}

fn to_i64(frames: usize) -> i64 {
    i64::try_from(frames).unwrap_or(i64::MAX)
}

fn to_u64(frames: usize) -> u64 {
    u64::try_from(frames).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ops::Range;

    // The loop's thresholds were checked with a Python port of the same loop; on the seeds used
    // here the narrowest margin is 1.7×. The audio tests generate their own timestamps, run a
    // mixer that reads on its own clock (as M3's mixer thread and audio-soak do, packets or
    // not), and check where impulses land and that every splice is faded.

    /// How far behind real time the mixer reads: 200 ms (research 10 §5.2).
    const MIX_DELAY: i64 = crate::mix::MIX_DELAY_FRAMES;
    /// One millisecond, in samples.
    const MS: f64 = 48.0;

    struct Lcg(u64);

    impl Lcg {
        /// A uniform value in [0, 1).
        fn next(&mut self) -> f64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1);
            (self.0 >> 11) as f64 / (1_u64 << 53) as f64
        }

        /// A uniform value in [−`half_width`, `half_width`).
        fn uniform(&mut self, half_width: f64) -> f64 {
            (self.next() * 2.0 - 1.0) * half_width
        }
    }

    /// The nearest-rank percentile `percent` of `values`.
    fn percentile(values: &[f64], percent: usize) -> f64 {
        let mut sorted = values.to_vec();
        sorted.sort_by(f64::total_cmp);
        sorted[(percent * sorted.len()).div_ceil(100) - 1]
    }

    /// A run of the loop alone over 10 ms packets.
    struct LoopCase {
        /// How fast the source's clock runs, as a fraction (200e-6 is 200 ppm fast).
        ppm: f64,
        /// Uniform timestamp jitter, ± samples.
        jitter: f64,
        /// How late 1 packet in 100 comes, in samples.
        outlier: f64,
        packets: usize,
        /// Packets flagged with a timestamp error, whose jitter is `flagged_jitter` instead.
        flagged: Range<usize>,
        flagged_jitter: f64,
        /// Packets that never arrive, and that nothing reports.
        lost: Range<usize>,
        seed: u64,
    }

    impl LoopCase {
        fn new(ppm: f64, jitter: f64, packets: usize, seed: u64) -> Self {
            Self {
                ppm,
                jitter,
                outlier: 0.0,
                packets,
                flagged: 0..0,
                flagged_jitter: 0.0,
                lost: 0..0,
                seed,
            }
        }
    }

    struct LoopRun {
        /// pred − true position at each packet, in samples.
        errors: Vec<f64>,
        ratios: Vec<f64>,
        integs: Vec<f64>,
        resyncs: usize,
    }

    fn run_loop(case: &LoopCase) -> LoopRun {
        let mut clock = ClockLoop::new();
        let mut random = Lcg(case.seed);
        let mut run = LoopRun {
            errors: Vec::new(),
            ratios: Vec::new(),
            integs: Vec::new(),
            resyncs: 0,
        };
        for k in 0..case.packets {
            let truth = k as f64 * 480.0 / (1.0 + case.ppm);
            let flagged = case.flagged.contains(&k);
            let jitter = if flagged {
                case.flagged_jitter
            } else {
                case.jitter
            };
            let mut ts = truth + random.uniform(jitter);
            if random.next() < 0.01 {
                ts += case.outlier;
            }
            if case.lost.contains(&k) {
                continue;
            }
            if let Step::Resync(_, at) = clock.step(ts.round() as i64, 480, false, flagged, false) {
                run.resyncs += 1;
                clock.anchor(at);
            }
            run.errors.push(clock.pred - truth);
            run.ratios.push(clock.ratio);
            run.integs.push(clock.integ);
            clock.advance(480);
        }
        run
    }

    #[test]
    fn the_loop_follows_200_ppm_drift_through_jitter() {
        // Research 10 §8.4: ±200 ppm, ±3 ms of jitter and 1% outliers 8 ms late, 600 s.
        for ppm in [200e-6, -200e-6] {
            for seed in 1..=5 {
                let case = LoopCase {
                    outlier: 384.0,
                    ..LoopCase::new(ppm, 144.0, 60_000, seed)
                };
                let run = run_loop(&case);
                let what = format!("{ppm} at seed {seed}");
                // Only the first packet anchors.
                assert_eq!(run.resyncs, 1, "{what}");
                let errors: Vec<f64> = run.errors[6_000..].iter().map(|e| e.abs()).collect();
                let p99 = percentile(&errors, 99);
                assert!(p99 < 0.5 * MS, "{what}: p99 {p99}");
                // Every 120 s window from 60 s on averages within ±10 ppm of the true rate.
                let target = 1.0 / (1.0 + ppm);
                for start in (6_000..=48_000).step_by(1_000) {
                    let mean = run.ratios[start..start + 12_000].iter().sum::<f64>() / 12_000.0;
                    assert!((mean - target).abs() < 10e-6, "{what}: window at {start}");
                }
            }
        }
    }

    #[test]
    fn a_timestamp_error_storm_neither_resyncs_nor_drifts() {
        // Locked for 60 s at +100 ppm, then 10 s of flagged timestamps off by up to ±500 ms.
        for seed in 1..=5 {
            let case = LoopCase {
                flagged: 6_000..7_000,
                flagged_jitter: 24_000.0,
                ..LoopCase::new(100e-6, 144.0, 13_000, seed)
            };
            let run = run_loop(&case);
            assert_eq!(run.resyncs, 1, "seed {seed}");
            // Through the storm the ratio is the rate estimate alone, not the last ratio with
            // its proportional term.
            for k in 6_000..7_000 {
                assert_eq!(run.ratios[k], rate_ratio(run.integs[k]), "seed {seed}, {k}");
            }
            assert_ne!(run.ratios[6_000], run.ratios[5_999], "seed {seed}");
            let errors: Vec<f64> = run.errors[6_000..].iter().map(|e| e.abs()).collect();
            let p99 = percentile(&errors, 99);
            assert!(p99 < 0.5 * MS, "seed {seed}: p99 {p99}");
        }
    }

    #[test]
    fn a_long_run_of_flagged_timestamps_stays_within_50_ms() {
        // Every packet after the first flagged, for an hour, at ±200 ppm: the loop can't steer,
        // so the source re-anchors where the flagged errors' low-pass puts it.
        for ppm in [200e-6, -200e-6] {
            let case = LoopCase {
                flagged: 1..360_000,
                flagged_jitter: 72.0,
                ..LoopCase::new(ppm, 72.0, 360_000, 1)
            };
            let run = run_loop(&case);
            let worst = run
                .errors
                .iter()
                .fold(0.0_f64, |worst, e| worst.max(e.abs()));
            assert!(worst < 52.0 * MS, "{ppm}: {worst}");
            assert!(run.resyncs > 10, "{ppm}: {}", run.resyncs);
        }
        // A minute of garbage timestamps (±500 ms) never adds up to a re-anchor.
        for seed in 1..=5 {
            let case = LoopCase {
                flagged: 6_000..12_000,
                flagged_jitter: 24_000.0,
                ..LoopCase::new(100e-6, 144.0, 18_000, seed)
            };
            assert_eq!(run_loop(&case).resyncs, 1, "seed {seed}");
        }
    }

    #[test]
    fn a_loss_the_device_misses_overshoots_little() {
        // 40 ms of packets vanish unreported after 60 s: below the 50 ms re-anchor. Holding
        // the integral at the ratio's limit keeps the overshoot near 1.7 ms; without that it
        // reaches 24 ms.
        for seed in 1..=5 {
            let case = LoopCase {
                lost: 6_000..6_004,
                ..LoopCase::new(0.0, 72.0, 30_000, seed)
            };
            let run = run_loop(&case);
            assert_eq!(run.resyncs, 1, "seed {seed}");
            // Right after the loss the audio lands 40 ms early; the overshoot is how far past
            // the truth it swings once it has caught up.
            let after = &run.errors[6_000..];
            assert!(after[0] < -38.0 * MS, "seed {seed}: {}", after[0]);
            let caught_up = after.iter().position(|&e| e >= 0.0).unwrap();
            let overshoot = after[caught_up..].iter().copied().fold(0.0, f64::max);
            assert!(overshoot < 3.0 * MS, "seed {seed}: {overshoot}");
        }
    }

    #[test]
    fn re_anchors_keep_the_rate_estimate_and_clip_outliers() {
        let run = run_loop(&LoopCase {
            lost: 6_000..6_030,
            ..LoopCase::new(200e-6, 0.0, 6_100, 1)
        });
        // The 300 ms gap re-anchors once; the rate estimate carries across it.
        assert_eq!(run.resyncs, 2);
        assert_eq!(run.integs[5_999], run.integs[6_000]);
        // One outlier moves the low-passed error by ALPHA × CLIP at most.
        let mut clock = ClockLoop::new();
        let _ = clock.step(0, 480, false, false, false);
        clock.anchor(0);
        clock.advance(480);
        assert_eq!(
            clock.step(480 + 2_000, 480, false, false, false),
            Step::Track
        );
        assert!(
            (clock.err_lp - ALPHA * CLIP).abs() < 1e-12,
            "{}",
            clock.err_lp
        );
    }

    /// Feeds an aligner and keeps channel 0 of what the mixer reads, 200 ms behind real time.
    struct Bench {
        aligner: SourceAligner,
        /// Mixed samples, indexed by timeline sample.
        mixed: Vec<f32>,
        read_pos: i64,
    }

    impl Bench {
        fn new() -> Self {
            Self {
                aligner: SourceAligner::new(1).unwrap(),
                mixed: Vec::new(),
                read_pos: 0,
            }
        }

        /// Reads every block at least 200 ms older than real time `now`.
        fn read_until(&mut self, now: i64) {
            let mut block = [0.0_f32; CHUNK_FRAMES];
            while self.read_pos + 480 <= now - MIX_DELAY {
                self.aligner.read_block(self.read_pos, &mut block).unwrap();
                self.mixed.extend_from_slice(&block);
                self.read_pos += 480;
            }
        }
    }

    /// Source frames `first..first + 480` of a mono signal: `dc` everywhere, plus 0.5 on every
    /// frame that is a multiple of 4,800 shifted by 2,400.
    fn signal(first: u64, dc: f32) -> Vec<f32> {
        (first..first + 480)
            .map(|frame| if frame % 4_800 == 2_400 { dc + 0.5 } else { dc })
            .collect()
    }

    fn packet(ts: i64, data: &[f32]) -> AudioPacket<'_> {
        AudioPacket {
            ts,
            data,
            discontinuity: false,
            timestamp_error: false,
            lost_frames: 0,
        }
    }

    /// Runs `slots` 10 ms slots of a jitter-free mono source at the timeline's own rate. In
    /// each slot `edit` chooses what to push; then the mixer reads on its own clock.
    fn run_audio(
        slots: u64,
        dc: f32,
        mut edit: impl FnMut(u64, &mut SourceAligner, AudioPacket<'_>),
    ) -> Bench {
        let mut bench = Bench::new();
        for k in 0..slots {
            let data = signal(k * 480, dc);
            edit(
                k,
                &mut bench.aligner,
                packet(i64::try_from(k * 480).unwrap(), &data),
            );
            bench.read_until(i64::try_from((k + 1) * 480).unwrap());
        }
        bench
    }

    /// The centre of the impulse near timeline sample `at`, after removing `dc`.
    fn centroid(mixed: &[f32], at: f64, dc: f32) -> f64 {
        let from = (at.round() as usize).saturating_sub(16);
        let (mut weighted, mut total) = (0.0, 0.0);
        for (i, &x) in mixed.iter().enumerate().skip(from).take(33) {
            let energy = f64::from((x - dc) * (x - dc));
            weighted += i as f64 * energy;
            total += energy;
        }
        weighted / total
    }

    /// Timeline samples of every impulse peak in `mixed`.
    fn peaks(mixed: &[f32], dc: f32) -> Vec<usize> {
        (1..mixed.len() - 1)
            .filter(|&i| {
                let x = mixed[i] - dc;
                x > 0.25 && x >= mixed[i - 1] - dc && x > mixed[i + 1] - dc
            })
            .collect()
    }

    /// The largest step between neighbouring samples within `span` of `at`. A splice without a
    /// fade on 0.25 of DC steps by 0.25; a faded one by about 0.01.
    fn max_step(mixed: &[f32], at: usize, span: usize) -> f32 {
        mixed[at - span..at + span]
            .windows(2)
            .map(|pair| (pair[1] - pair[0]).abs())
            .fold(0.0, f32::max)
    }

    /// Impulses after timeline sample `from` land within 2 samples of their source frame, less
    /// `shift`.
    fn assert_in_place(mixed: &[f32], from: usize, shift: usize, dc: f32) {
        let after: Vec<usize> = (2_400..mixed.len() - 32)
            .step_by(4_800)
            .filter(|&frame| frame >= from + shift)
            .collect();
        assert!(after.len() > 30, "{}", after.len());
        for frame in after {
            let at = frame - shift;
            let landed = centroid(mixed, at as f64, dc);
            assert!((landed - at as f64).abs() < 2.0, "{at}: {landed}");
        }
    }

    #[test]
    fn samples_land_where_their_timestamps_say() {
        // A source 100 ppm fast with ±1.5 ms of jitter, for 120 s.
        let ppm = 100e-6;
        let mut random = Lcg(1);
        let mut bench = Bench::new();
        let mut ratios = Vec::new();
        for k in 0..12_000_u64 {
            let truth = k as f64 * 480.0 / (1.0 + ppm);
            let data = signal(k * 480, 0.0);
            let ts = (truth + random.uniform(72.0)).round() as i64;
            bench.aligner.push(&packet(ts, &data)).unwrap();
            bench.read_until((truth + 480.0) as i64);
            if k >= 6_000 {
                ratios.push(bench.aligner.ratio());
            }
        }
        assert_eq!(bench.aligner.stats().packets, 12_000);
        assert_eq!(bench.aligner.stats().stalls, 0);
        // Once locked (from 45 s), every impulse lands within 0.5 ms of its true place.
        let mut checked = 0;
        for impulse in (2_400..12_000 * 480).step_by(4_800) {
            let truth = impulse as f64 / (1.0 + ppm);
            if truth < 45.0 * 48_000.0 || truth + 64.0 > bench.mixed.len() as f64 {
                continue;
            }
            let landed = centroid(&bench.mixed, truth, 0.0);
            assert!(
                (landed - truth).abs() < 0.5 * MS,
                "impulse at {truth}: {landed}"
            );
            checked += 1;
        }
        assert!(checked > 700, "{checked}");
        let mean = ratios.iter().sum::<f64>() / ratios.len() as f64;
        assert!((mean - 1.0 / (1.0 + ppm)).abs() < 10e-6, "{mean}");
    }

    #[test]
    fn a_gap_longer_than_the_delay_fades_out_before_the_mixer_runs_dry() {
        // 30 packets (300 ms) never come; the mixer keeps reading through the gap.
        let bench = run_audio(1_000, 0.25, |k, aligner, packet| {
            if !(500..530).contains(&k) {
                aligner.push(&packet).unwrap();
            }
        });
        let stats = bench.aligner.stats();
        assert_eq!(
            (stats.gap_resyncs, stats.overlap_resyncs, stats.stalls),
            (1, 0, 1)
        );
        // Only the 3 lag frames at the very start fell before the mixer.
        assert_eq!(stats.late_frames, 3);
        assert!(max_step(&bench.mixed, 240_000, 300) < 0.02);
        assert!(max_step(&bench.mixed, 254_400, 300) < 0.02);
        // Silence from the faded end to the new audio: 14,400 samples, give or take the
        // resampler's window.
        let silent = bench.mixed[239_800..254_600]
            .iter()
            .filter(|&&x| x == 0.0)
            .count();
        assert!((14_392..=14_408).contains(&silent), "{silent}");
        assert!(
            (14_390..=14_405).contains(&stats.underrun_frames),
            "{}",
            stats.underrun_frames
        );
        assert_in_place(&bench.mixed, 254_400, 0, 0.25);
    }

    #[test]
    fn a_gap_shorter_than_the_delay_splices_ahead_of_the_mixer() {
        // 16 packets (160 ms) never come: the next one arrives before the mixer gets there.
        let bench = run_audio(1_000, 0.25, |k, aligner, packet| {
            if !(500..516).contains(&k) {
                aligner.push(&packet).unwrap();
            }
        });
        let stats = bench.aligner.stats();
        assert_eq!(
            (stats.gap_resyncs, stats.stalls, stats.late_frames),
            (1, 0, 3)
        );
        assert!(max_step(&bench.mixed, 240_000, 300) < 0.02);
        assert!(max_step(&bench.mixed, 247_680, 300) < 0.02);
        let silent = bench.mixed[239_800..247_880]
            .iter()
            .filter(|&&x| x == 0.0)
            .count();
        assert!((7_672..=7_688).contains(&silent), "{silent}");
        assert_in_place(&bench.mixed, 247_680, 0, 0.25);
    }

    #[test]
    fn an_overlap_is_trimmed_not_doubled() {
        // From packet 500 on, timestamps jump back 4,000 samples (83 ms).
        let bench = run_audio(1_000, 0.0, |k, aligner, mut packet| {
            if k >= 500 {
                packet.ts -= 4_000;
            }
            aligner.push(&packet).unwrap();
        });
        let stats = bench.aligner.stats();
        assert_eq!((stats.overlap_resyncs, stats.gap_resyncs), (1, 0));
        // The new audio starts at 236,000 (its first output frame 3 earlier) and overwrites
        // the old from there; everything else plays once, in place.
        let expected: Vec<usize> = (2_400..1_000 * 480)
            .step_by(4_800)
            .filter_map(|frame| match frame {
                f if f < 236_000 - 3 => Some(f),
                f if f >= 500 * 480 => Some(f - 4_000),
                _ => None,
            })
            .filter(|&at| at + 2 < bench.mixed.len())
            .collect();
        assert_eq!(peaks(&bench.mixed, 0.0), expected);
    }

    #[test]
    fn an_overlap_at_the_read_position_fades_both_sides() {
        // The mixer has run ahead to 237,120 when timestamps jump back 2,850 samples, which puts
        // the new audio 27 frames after it, closer than a fade: the old audio fades out over the
        // 96 frames after the mixer, the new audio's first 69 frames are dropped, and the rest
        // fades in.
        let mut bench = Bench::new();
        for k in 0..1_000_u64 {
            let data = signal(k * 480, 0.25);
            let mut ts = i64::try_from(k * 480).unwrap();
            if k == 500 {
                bench.read_until(237_120 + MIX_DELAY);
                assert_eq!(bench.read_pos, 237_120);
            }
            if k >= 500 {
                ts -= 2_850;
            }
            bench.aligner.push(&packet(ts, &data)).unwrap();
            bench.read_until(i64::try_from((k + 1) * 480).unwrap());
        }
        let stats = bench.aligner.stats();
        assert_eq!((stats.overlap_resyncs, stats.stalls), (1, 0));
        assert_eq!(stats.late_frames, 3 + 69);
        assert!(max_step(&bench.mixed, 237_120, 300) < 0.02);
        assert_in_place(&bench.mixed, 237_400 + 2_850, 2_850, 0.25);
    }

    #[test]
    fn a_jump_back_past_the_mixer_fades_out_and_drops_what_is_too_late() {
        // From packet 500 on, timestamps jump back 250 ms: every later packet belongs to a
        // place the mixer has already played.
        let bench = run_audio(1_000, 0.25, |k, aligner, mut packet| {
            if k >= 500 {
                packet.ts -= 12_000;
            }
            aligner.push(&packet).unwrap();
        });
        let stats = bench.aligner.stats();
        assert_eq!(stats.overlap_resyncs, 1);
        assert!(stats.late_frames > 200_000, "{}", stats.late_frames);
        // The old audio fades out right after the mixer's position; then silence.
        assert!(max_step(&bench.mixed, 230_400, 300) < 0.02);
        assert!(bench.mixed[230_496..].iter().all(|&x| x == 0.0));
    }

    #[test]
    fn audio_that_arrives_after_its_read_is_dropped_not_shifted() {
        // Packets 500 to 549 are held back for 500 ms and then arrive together.
        let mut held = Vec::new();
        let bench = run_audio(1_000, 0.25, |k, aligner, packet| {
            if (500..550).contains(&k) {
                held.push(packet.data.to_vec());
                return;
            }
            if k == 550 {
                for (i, data) in held.iter().enumerate() {
                    let late = AudioPacket {
                        ts: i64::try_from((500 + i) * 480).unwrap(),
                        data,
                        ..packet
                    };
                    aligner.push(&late).unwrap();
                }
            }
            aligner.push(&packet).unwrap();
        });
        let stats = bench.aligner.stats();
        assert_eq!(
            (stats.gap_resyncs, stats.overlap_resyncs, stats.stalls),
            (0, 0, 1)
        );
        // By then the mixer had read up to 200 ms before packet 549's end, so 14,400 frames of
        // the backlog came too late, with the resampler's 3 warm-up frames before them, plus the
        // 3 lag frames at the very start.
        let read_by_then = 550 * 480 - 9_600;
        assert_eq!(
            stats.late_frames,
            u64::try_from(read_by_then - 500 * 480 + 6).unwrap()
        );
        // The audio faded out before the mixer ran dry, and faded back in where it could play.
        assert!(max_step(&bench.mixed, 240_000, 300) < 0.02);
        assert!(max_step(&bench.mixed, read_by_then, 300) < 0.02);
        let found = peaks(&bench.mixed, 0.25);
        let expected: Vec<usize> = (2_400..1_000 * 480)
            .step_by(4_800)
            .filter(|&at| (at < 500 * 480 || at >= read_by_then + 96) && at + 2 < bench.mixed.len())
            .collect();
        assert_eq!(found, expected);
    }

    #[test]
    fn a_source_that_stops_ends_with_a_fade_and_its_whole_tail() {
        // 441-frame packets leave part of a chunk waiting when the source stops.
        let mut bench = Bench::new();
        for k in 0..544_u64 {
            let data: Vec<f32> = (0..441).map(|_| 0.25).collect();
            bench
                .aligner
                .push(&packet(i64::try_from(k * 441).unwrap(), &data))
                .unwrap();
            bench.read_until(i64::try_from((k + 1) * 441).unwrap());
        }
        let end = 544 * 441;
        bench.read_until(i64::try_from(end).unwrap() + 20_000);
        assert_eq!(bench.aligner.stats().stalls, 1);
        // Every frame up to the source's last plays, the last 2 ms faded to nothing.
        let last_sound = bench.mixed.iter().rposition(|&x| x != 0.0).unwrap();
        assert_eq!(last_sound, end - 2);
        assert!(max_step(&bench.mixed, end - 48, 200) < 0.02);
        assert_eq!(bench.aligner.stats().late_frames, 3);
        // A last packet of 477 frames leaves more pending than one padded chunk lets out.
        let mut bench = Bench::new();
        for k in 0..500_u64 {
            let frames = if k == 499 { 477 } else { 480 };
            let data = vec![0.25_f32; frames];
            bench
                .aligner
                .push(&packet(i64::try_from(k * 480).unwrap(), &data))
                .unwrap();
            bench.read_until(i64::try_from((k + 1) * 480).unwrap());
        }
        let end = 499 * 480 + 477;
        bench.read_until(i64::try_from(end).unwrap() + 20_000);
        let last_sound = bench.mixed.iter().rposition(|&x| x != 0.0).unwrap();
        assert_eq!(last_sound, end - 2);
    }

    #[test]
    fn lost_frames_become_faded_silence_without_a_resync() {
        // Packets 500 to 502 are dropped by the capture thread, which says so on packet 503.
        let bench = run_audio(1_000, 0.25, |k, aligner, mut packet| {
            if (500..503).contains(&k) {
                return;
            }
            if k == 503 {
                packet.lost_frames = 1_440;
            }
            aligner.push(&packet).unwrap();
        });
        let stats = bench.aligner.stats();
        assert_eq!(stats.filled_frames, 1_440);
        assert_eq!(
            (
                stats.gap_resyncs,
                stats.overlap_resyncs,
                stats.discontinuity_resyncs,
                stats.stalls
            ),
            (0, 0, 0, 0)
        );
        assert!(max_step(&bench.mixed, 240_000, 200) < 0.02);
        assert!(max_step(&bench.mixed, 241_440, 200) < 0.02);
        let silent = bench.mixed[239_800..241_640]
            .iter()
            .filter(|&&x| x == 0.0)
            .count();
        assert_eq!(silent, 1_441);
        assert_in_place(&bench.mixed, 241_440, 0, 0.25);
    }

    #[test]
    fn lost_frames_beyond_50_ms_are_left_to_the_timestamps() {
        for (lost, filled, gaps) in [(2_400_u32, 2_400, 0), (2_401, 0, 1)] {
            let bench = run_audio(700, 0.25, |k, aligner, mut packet| {
                if k == 600 {
                    packet.ts += i64::from(lost);
                    packet.lost_frames = lost;
                }
                if k > 600 {
                    packet.ts += i64::from(lost);
                }
                aligner.push(&packet).unwrap();
            });
            let stats = bench.aligner.stats();
            assert_eq!(
                (stats.filled_frames, stats.gap_resyncs),
                (filled, gaps),
                "{lost}"
            );
        }
    }

    #[test]
    fn a_discontinuity_re_anchors_at_its_timestamp() {
        // The device lost packets 500 to 502 and says so on packet 503.
        let bench = run_audio(1_000, 0.25, |k, aligner, mut packet| {
            if (500..503).contains(&k) {
                return;
            }
            packet.discontinuity = k == 503;
            aligner.push(&packet).unwrap();
        });
        let stats = bench.aligner.stats();
        assert_eq!((stats.discontinuity_resyncs, stats.gap_resyncs), (1, 0));
        assert!(max_step(&bench.mixed, 241_440, 200) < 0.02);
        assert_in_place(&bench.mixed, 241_440, 0, 0.25);
    }

    #[test]
    fn held_packets_send_the_rate_estimate_to_the_resampler() {
        let mut aligner = SourceAligner::new(1).unwrap();
        let data = [0.1_f32; 480];
        // Timestamps 10 ms late for a while give the ratio a proportional term.
        for k in 0..50 {
            let ts = k * 480 + if k > 0 { 480 } else { 0 };
            aligner.push(&packet(ts, &data)).unwrap();
        }
        let tracked = aligner.ratio();
        let flagged = AudioPacket {
            timestamp_error: true,
            ..packet(50 * 480 + 480, &data)
        };
        aligner.push(&flagged).unwrap();
        let held = rate_ratio(aligner.clock.integ);
        assert_eq!(aligner.ratio(), held);
        assert_ne!(held, tracked);
        assert!((aligner.resampler.resample_ratio() - held).abs() < 1e-12);
    }

    #[test]
    fn rubato_puts_input_frame_n_at_output_frame_n_plus_3() {
        let mut resampler = Async::<f32>::new_poly(
            1.0,
            1.01,
            PolynomialDegree::Septic,
            CHUNK_FRAMES,
            1,
            FixedAsync::Input,
        )
        .unwrap();
        let mut input = [0.0_f32; CHUNK_FRAMES];
        input[10] = 1.0;
        let mut output = vec![0.0_f32; resampler.output_frames_max()];
        let frames = output.len();
        let (_, written) = resampler
            .process_into_buffer(
                &InterleavedSlice::new(&input[..], 1, CHUNK_FRAMES).unwrap(),
                &mut InterleavedSlice::new_mut(&mut output[..], 1, frames).unwrap(),
                None,
            )
            .unwrap();
        let peak = output[..written].iter().position(|&x| x == 1.0).unwrap();
        assert_eq!(i64::try_from(peak).unwrap(), 10 + RESAMPLER_LAG);
    }

    #[test]
    fn a_position_far_ahead_drops_what_was_waiting_without_growing() {
        let mut aligner = SourceAligner::new(1).unwrap();
        let reserved = aligner.out.capacity();
        for k in 0..30 {
            let data = signal(k * 480, 0.0);
            aligner
                .push(&packet(i64::try_from(k * 480).unwrap(), &data))
                .unwrap();
        }
        // Nothing was read, so everything pushed is waiting.
        let waiting = u64::try_from(aligner.out.len()).unwrap();
        let data = signal(0, 0.0);
        aligner.push(&packet(30 * 480 + 96_000, &data)).unwrap();
        // The flushed tail was waiting too: 8 frames.
        assert_eq!(aligner.stats().overflow_frames, waiting + 8);
        assert_eq!(aligner.stats().gap_resyncs, 1);
        assert_eq!(aligner.out.capacity(), reserved);
    }

    #[test]
    fn extreme_inputs_neither_panic_nor_stall() {
        let extremes = [i64::MIN, -1, 0, 1, i64::MAX];
        for channels in [1, 2] {
            for &ts in &extremes {
                for lost_frames in [0, 2_400, u32::MAX] {
                    let mut aligner = SourceAligner::new(channels).unwrap();
                    let data = vec![0.1_f32; 480 * channels];
                    let mut block = vec![0.0_f32; 480 * channels];
                    for &pos in &extremes {
                        aligner.push(&packet(0, &data)).unwrap();
                        let odd = AudioPacket {
                            lost_frames,
                            ..packet(ts, &data)
                        };
                        aligner.push(&odd).unwrap();
                        aligner.read_block(pos, &mut block).unwrap();
                    }
                }
            }
        }
    }

    #[test]
    fn running_never_reallocates() {
        let mut bench = Bench::new();
        let capacities = |aligner: &SourceAligner| {
            (
                aligner.input.capacity(),
                aligner.scratch.capacity(),
                aligner.out.capacity(),
            )
        };
        let before = capacities(&bench.aligner);
        let mut jump = 0;
        for k in 0..3_000_u64 {
            jump += match k {
                1_000 => 24_000,
                1_500 => 57_600,
                2_000 => 72_000,
                _ => 0,
            };
            let data = signal(k * 441, 0.1);
            let ts = i64::try_from(k * 441).unwrap() + jump;
            let packet = AudioPacket {
                discontinuity: k == 2_500,
                timestamp_error: (300..400).contains(&k),
                lost_frames: if k == 700 { 960 } else { 0 },
                ..packet(ts, &data)
            };
            bench.aligner.push(&packet).unwrap();
            bench.read_until(ts + 441);
        }
        // A mixer stalled for 2 s overflows the waiting audio, without growing it.
        for k in 3_000..3_200_u64 {
            let data = signal(k * 441, 0.1);
            let ts = i64::try_from(k * 441).unwrap() + jump;
            bench.aligner.push(&packet(ts, &data)).unwrap();
        }
        assert!(bench.aligner.stats().overflow_frames > 0);
        assert_eq!(capacities(&bench.aligner), before);
    }

    #[test]
    fn debug_output_holds_no_audio() {
        let mut aligner = SourceAligner::new(2).unwrap();
        let data = [0.123_456_f32; 960];
        aligner.push(&packet(0, &data)).unwrap();
        for text in [format!("{aligner:?}"), format!("{:?}", packet(0, &data))] {
            assert!(!text.contains("0.12345"), "{text}");
            assert!(text.len() < 1_000, "{}", text.len());
        }
    }

    #[test]
    fn refuses_bad_shapes_and_reads_silence_before_any_audio() {
        assert_eq!(
            SourceAligner::new(0).map(|_| ()),
            Err(AlignError::Channels(0))
        );
        assert_eq!(
            SourceAligner::new(3).map(|_| ()),
            Err(AlignError::Channels(3))
        );
        let mut aligner = SourceAligner::new(2).unwrap();
        let data = [0.0_f32; 961];
        assert_eq!(
            aligner.push(&packet(0, &data)),
            Err(AlignError::RaggedPacket {
                len: 961,
                channels: 2
            })
        );
        let mut odd = [1.0_f32; 959];
        assert_eq!(
            aligner.read_block(0, &mut odd),
            Err(AlignError::RaggedBlock {
                len: 959,
                channels: 2
            })
        );
        let mut block = [1.0_f32; 960];
        aligner.read_block(0, &mut block).unwrap();
        assert!(block.iter().all(|&x| x == 0.0));
        assert_eq!(aligner.stats().underrun_frames, 480);
        assert_eq!(aligner.stats().packets, 0);
    }
}
