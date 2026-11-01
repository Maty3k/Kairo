// SPDX-License-Identifier: GPL-3.0-or-later
//! The audio detectors (docs/research/16-verify-testbed/signals.md §3.3–3.7, README §6.3), on
//! one track's (L+R)/2 at 48 kHz: beep onsets from a demodulated 1 ms envelope refined by the
//! carrier's phase, narrowband levels, sinusoid fits for the tone SNR, the frequency from an FFT
//! peak, and a matched filter for Kairo's cue. Positions are in samples from the track's first
//! decoded sample, fractional where a detector resolves less than a sample.

use std::f64::consts::{PI, TAU};

use kairo_core::testsig;

/// The sample rate the checks work at (README §6.2 `audio.format`).
pub const RATE: f64 = 48_000.0;

/// Samples in 1 ms.
pub const MS: f64 = RATE / 1_000.0;

/// The boxcar after demodulation: 1 ms, which nulls every tone a whole number of kHz away, so
/// the 1, 2 and 3 kHz detectors can't see each other's beeps (signals.md §3.3).
const BOXCAR: usize = 48;

/// How long the envelope must stay under a quarter of the plateau before an onset counts: 4 ms.
const QUIET: usize = 192;

/// The quietest beep that counts, in dBFS.
pub const BEEP_MIN_DB: f64 = -60.0;

/// How far a beep's frequency may be from its tone's, in Hz.
pub const BEEP_HZ_TOLERANCE: f64 = 1.0;

/// Windows quieter than this RMS, in dBFS, are silence to the cue's matched filter.
pub const SILENCE_DB: f64 = -90.0;

/// The share of a search window's envelope taken as its noise: a beep fills at most half of any
/// window the detectors search, so the lowest quarter is noise.
const FLOOR_QUANTILE: f64 = 0.25;

/// How far, in dB, a beep's 1 ms envelope may disagree with its narrowband level, and its
/// narrowband level wander over its steady part. Another sound leaking into the envelope moves
/// them further; AAC at 96 kbps keeps a clean beep's steady part within 0.3 dB.
const STEADY_DB: f64 = 2.0;

/// How far under its steady level a beep may be just after its ramp, from 2 to 10 ms: AAC's
/// smeared attack takes up to 4.2 dB off a 3 kHz marker at 96 kbps, while a beep timed at
/// another sound's start reads 20 dB or more under there.
const ATTACK_DB: f64 = 6.0;

/// Appends the mean of each frame of `interleaved` to `out`: (L+R)/2 for stereo, the samples
/// themselves for mono. A partial last frame is dropped.
pub fn downmix(interleaved: &[f32], channels: usize, out: &mut Vec<f32>) {
    if channels <= 1 {
        out.extend_from_slice(interleaved);
        return;
    }
    out.extend(
        interleaved
            .chunks_exact(channels)
            .map(|frame| frame.iter().sum::<f32>() / channels as f32),
    );
}

/// dBFS of an amplitude, with silence at −240.
pub fn db(amplitude: f64) -> f64 {
    20.0 * amplitude.max(1e-12).log10()
}

/// The phase of a `hz` carrier at sample `n`, in radians, kept accurate on long tracks.
fn carrier(hz: f64, n: usize) -> f64 {
    TAU * (hz * n as f64 / RATE).fract()
}

/// The amplitude of the tone at `hz` around each sample of `from..to`: `x` demodulated at `hz`,
/// averaged over the centred 1 ms boxcar, times 2 (signals.md §3.3 steps 1–2). Samples outside
/// `x` count as silence.
fn envelope(x: &[f32], hz: f64, from: usize, to: usize) -> Vec<f64> {
    let half = BOXCAR / 2;
    let lo = from.saturating_sub(half);
    let hi = to.saturating_add(half).min(x.len());
    let mut re = Vec::with_capacity(hi.saturating_sub(lo) + 1);
    let mut im = Vec::with_capacity(re.capacity());
    let (mut sr, mut si) = (0.0, 0.0);
    re.push(0.0);
    im.push(0.0);
    for (n, &v) in x.iter().enumerate().take(hi).skip(lo) {
        let (s, c) = carrier(hz, n).sin_cos();
        sr += f64::from(v) * c;
        si -= f64::from(v) * s;
        re.push(sr);
        im.push(si);
    }
    (from..to)
        .map(|n| {
            let a = n.saturating_sub(half).clamp(lo, hi) - lo;
            let b = n.saturating_add(half).clamp(lo, hi) - lo;
            let (dr, di) = (re[b] - re[a], im[b] - im[a]);
            2.0 * (dr * dr + di * di).sqrt() / BOXCAR as f64
        })
        .collect()
}

/// The `q`-quantile (0–1) of `values`, by rank.
fn quantile(values: &[f64], q: f64) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    let rank = ((sorted.len() - 1) as f64 * q).round() as usize;
    let (_, value, _) = sorted.select_nth_unstable_by(rank, f64::total_cmp);
    Some(*value)
}

/// Each beep-like rise of `hz` in `from..to`, in order, as the interpolated upward crossing of
/// half its own plateau after at least 4 ms under a quarter of it (signals.md §3.3 steps 3–4).
/// A rise starts where the envelope passes a trigger after 4 ms under it: four times the
/// window's noise, and at least half of 20 dB under the quietest beep that counts, so a beep
/// too quiet to count is still measured. Its plateau is the median envelope over its steady
/// part, 10 to 90 ms on, so a louder sound elsewhere in the window can't set it. A rise whose plateau disagrees with the
/// narrowband level there by more than [`STEADY_DB`] is another sound leaking into the
/// envelope, and is passed over.
fn rises(x: &[f32], hz: f64, from: usize, to: usize) -> Vec<f64> {
    let (steady, ramp) = ((10.0 * MS) as usize, (90.0 * MS) as usize);
    // The envelope runs on past `to`, so a rise near the window's end has its steady part.
    let env = envelope(x, hz, from, (to + ramp).min(x.len()).max(to));
    let window = &env[..to.saturating_sub(from).min(env.len())];
    let floor = 10_f64.powf((BEEP_MIN_DB - 20.0) / 20.0) / 2.0;
    let trigger = quantile(window, FLOOR_QUANTILE).map_or(floor, |q| floor.max(4.0 * q));
    let mut out = Vec::new();
    let mut quiet = 0;
    for i in 0..window.len() {
        if env[i] < trigger {
            quiet += 1;
            continue;
        }
        let armed = quiet >= QUIET;
        quiet = 0;
        if !armed {
            continue;
        }
        let Some(plateau) = env.get(i + steady..i + ramp).and_then(|s| quantile(s, 0.5)) else {
            continue;
        };
        let consistent = level_db(x, hz, from + i + steady, ramp - steady)
            .is_some_and(|level| (db(plateau) - level).abs() <= STEADY_DB);
        if plateau < 2.0 * trigger || !consistent {
            continue;
        }
        let (half, quarter) = (plateau / 2.0, plateau / 4.0);
        // The half crossing within 10 ms of the rise, and 4 ms under a quarter before it.
        let Some(j) =
            (i.max(1)..(i + steady).min(env.len())).find(|&j| env[j - 1] < half && env[j] >= half)
        else {
            continue;
        };
        let Some(r) = (0..j).rev().find(|&k| env[k] < quarter) else {
            continue;
        };
        if r + 1 < QUIET || env[r + 1 - QUIET..=r].iter().any(|&v| v >= quarter) {
            continue;
        }
        let fraction = (half - env[j - 1]) / (env[j] - env[j - 1]);
        out.push((from + j - 1) as f64 + fraction);
    }
    out
}

/// The first rise in `from..to` (see [`rises`]).
fn half_crossing(x: &[f32], hz: f64, from: usize, to: usize) -> Option<f64> {
    rises(x, hz, from, to).first().copied()
}

/// Where the envelope of the beep starting near `onset` falls through half its plateau (the
/// median over its steady part), interpolated, within 10 ms of where a beep's end belongs.
fn fall_crossing(x: &[f32], hz: f64, onset: f64) -> Option<f64> {
    let at = |ms: f64| (onset + ms * MS).round().max(0.0) as usize;
    let plateau = quantile(&envelope(x, hz, at(10.0), at(90.0)), 0.5)?;
    let (from, to) = (
        at(f64::from(testsig::BEEP_MS) - 10.0),
        at(f64::from(testsig::BEEP_MS) + 10.0),
    );
    if plateau <= 0.0 || to > x.len() {
        return None;
    }
    let half = plateau / 2.0;
    let env = envelope(x, hz, from, to);
    let i = env.windows(2).position(|p| p[0] >= half && p[1] < half)?;
    Some((from + i) as f64 + (env[i] - half) / (env[i] - env[i + 1]))
}

/// Whether the narrowband level of `hz` holds once the beep at `at` is up: within
/// [`ATTACK_DB`] of `level` over [at + 2, at + 10] ms, and within [`STEADY_DB`] over each half
/// of [at + 10, at + 90] ms. A beep timed at another sound's start, or overlapped by one at its
/// own frequency, doesn't.
fn steady(x: &[f32], hz: f64, at: f64, level: f64) -> bool {
    [
        (2.0, 8.0, ATTACK_DB),
        (10.0, 40.0, STEADY_DB),
        (50.0, 40.0, STEADY_DB),
    ]
    .iter()
    .all(|&(offset, ms, tolerance)| {
        let start = (at + offset * MS).round();
        start >= 0.0
            && level_db(x, hz, start as usize, (ms * MS) as usize)
                .is_some_and(|l| (l - level).abs() <= tolerance)
    })
}

/// The onset whose carrier phase over [onset + 10, onset + 90] ms matches the samples, on the
/// cycle nearest `coarse` (signals.md §3.3 step 5).
fn phase_onset(x: &[f32], hz: f64, coarse: f64) -> Option<f64> {
    let (start, len) = plateau(coarse)?;
    let span = x.get(start..start + len)?;
    let (mut re, mut im) = (0.0, 0.0);
    for (i, &v) in span.iter().enumerate() {
        let (s, c) = carrier(hz, start + i).sin_cos();
        re += f64::from(v) * c;
        im -= f64::from(v) * s;
    }
    if re == 0.0 && im == 0.0 {
        return None;
    }
    let period = RATE / hz;
    let tau = (-(im.atan2(re) + PI / 2.0) / TAU).rem_euclid(1.0) * period;
    Some(tau + ((coarse - tau) / period).round() * period)
}

/// A beep's steady part, [onset + 10, onset + 90] ms, as (first sample, length); `None` before
/// the track's start.
pub fn plateau(onset: f64) -> Option<(usize, usize)> {
    let start = (onset + 10.0 * MS).round();
    (start >= 0.0).then_some((start as usize, (80.0 * MS) as usize))
}

/// A beep found in a track.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Beep {
    /// Its onset, in samples, from the carrier's phase.
    pub at: f64,
    /// The envelope's estimate of it, in samples.
    pub coarse: f64,
    /// Its narrowband level over its steady part, in dBFS.
    pub level_db: f64,
    /// Its frequency from the FFT peak over the same span, in Hz.
    pub hz: f64,
}

/// Finds one tone's beeps.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Detector {
    hz: f64,
    /// Where the envelope of the beep itself crosses half its plateau, in samples after its
    /// onset: subtracted from every crossing.
    crossing: f64,
    /// Where it falls through half its plateau again, in samples after its onset.
    fall: f64,
}

impl Detector {
    /// A detector for beeps of `hz`, calibrated on `testsig`'s own beep.
    pub fn new(hz: f64) -> Self {
        let lead = 250.0 * MS;
        let beep = f64::from(testsig::BEEP_MS) * MS;
        let template: Vec<f32> = (0..(2.0 * lead + beep) as usize)
            .map(|n| testsig::tone_burst_at(hz, testsig::BEEP_MS, (n as f64 - lead) / RATE) as f32)
            .collect();
        let crossing = half_crossing(&template, hz, 0, template.len()).map_or(0.0, |c| c - lead);
        let fall = fall_crossing(&template, hz, lead).map_or(beep, |f| f - lead);
        Self { hz, crossing, fall }
    }

    /// The tone's frequency.
    pub fn hz(&self) -> f64 {
        self.hz
    }

    /// The template's own crossing time, in ms: 0.961 at 1 kHz and 0.993 at 2 kHz, about 10 µs
    /// later than signals.md §3.3's 0.953 and 0.986 because the 48-sample boxcar centres half a
    /// sample late; measurements subtract it, so the offset cancels.
    pub fn crossing_ms(&self) -> f64 {
        self.crossing / MS
    }

    /// The first beep whose onset lies within `radius` samples of `center`, measured whatever
    /// its level or frequency; see [`Beep`] and [`Detector::counts`]. A rise whose narrowband
    /// level doesn't hold once it is up is passed over for the next: a beep that another sound
    /// overlaps at its start goes unmeasured rather than timed at that sound.
    pub fn measure(&self, x: &[f32], center: f64, radius: f64) -> Option<Beep> {
        let from = (center - radius).max(0.0) as usize;
        let to = ((center + radius).max(0.0) as usize).min(x.len());
        if to <= from + QUIET {
            return None;
        }
        rises(x, self.hz, from, to)
            .into_iter()
            .find_map(|crossing| {
                // The carrier's cycle is chosen from the mean of the beep's start and end, every
                // beep lasting 100 ms: AAC smears a 3 kHz rise by −180 to +270 µs, more than half a
                // cycle, but its fall by ±70 µs at 96–160 kbps (PR 5a's review dsp-3), while another
                // tone's ramp on the fall can move that by 0.3 ms; the mean stays within 140 µs of
                // both. An end that disagrees with the start by over 0.5 ms isn't the beep's.
                let rise = crossing - self.crossing;
                let coarse = fall_crossing(x, self.hz, rise)
                    .map(|f| f - self.fall)
                    .filter(|&f| (f - rise).abs() <= 0.5 * MS)
                    .map_or(rise, |f| (rise + f) / 2.0);
                let at = phase_onset(x, self.hz, coarse)?;
                let (start, len) = plateau(at)?;
                let level = level_db(x, self.hz, start, len)?;
                if !steady(x, self.hz, at, level) {
                    return None;
                }
                Some(Beep {
                    at,
                    coarse,
                    level_db: level,
                    hz: peak_hz(x, self.hz, start, len)?,
                })
            })
    }

    /// Whether `beep` counts as one of this tone's: at least −60 dBFS and within 1 Hz.
    pub fn counts(&self, beep: &Beep) -> bool {
        beep.level_db >= BEEP_MIN_DB && (beep.hz - self.hz).abs() <= BEEP_HZ_TOLERANCE
    }

    /// The beep that counts within `radius` samples of `center`, if any.
    pub fn find(&self, x: &[f32], center: f64, radius: f64) -> Option<Beep> {
        self.measure(x, center, radius).filter(|b| self.counts(b))
    }

    /// Every beep in `x` that counts, in order: wherever the envelope rises past a trigger after
    /// 4 ms under it, the beep found within 250 ms and then again around its own onset. The
    /// trigger is half of −60 dBFS, or four times the envelope's median over each 10 s if that
    /// is higher, so a noisy track triggers on its beeps, not its noise; after a trigger finds
    /// nothing, the next 125 ms (already searched) is skipped.
    pub fn scan(&self, x: &[f32]) -> Vec<Beep> {
        let floor = 10_f64.powf(BEEP_MIN_DB / 20.0) / 2.0;
        let beep = f64::from(testsig::BEEP_MS) * MS;
        let mut beeps: Vec<Beep> = Vec::new();
        let chunk = (10.0 * RATE) as usize;
        let mut quiet = 0;
        let mut skip_to = 0.0;
        let mut start = 0;
        while start < x.len() {
            let end = (start + chunk).min(x.len());
            let env = envelope(x, self.hz, start, end);
            let trigger = quantile(&env, 0.5).map_or(floor, |m| floor.max(4.0 * m));
            for (i, &level) in env.iter().enumerate() {
                if level < trigger {
                    quiet += 1;
                    continue;
                }
                let rising = quiet >= QUIET;
                quiet = 0;
                let n = (start + i) as f64;
                if !rising || n < skip_to || beeps.last().is_some_and(|b| n <= b.at + beep) {
                    continue;
                }
                // Found again around its own onset: a window that held only the beep's start
                // can't time it.
                match self
                    .find(x, n, 250.0 * MS)
                    .and_then(|b| self.find(x, b.at, 100.0 * MS))
                {
                    Some(found) if beeps.last().is_none_or(|b| found.at > b.at + beep) => {
                        beeps.push(found);
                    }
                    _ => skip_to = n + 125.0 * MS,
                }
            }
            start = end;
        }
        beeps
    }
}

/// The Hann window's weight at `i` of `len`.
fn hann(i: usize, len: usize) -> f64 {
    0.5 - 0.5 * (TAU * i as f64 / len as f64).cos()
}

/// The narrowband level of `hz` in `x[start..start + len]`, in dBFS: one DFT bin under a Hann
/// window, scaled so a sine's peak reads its own level (signals.md §3.6).
pub fn level_db(x: &[f32], hz: f64, start: usize, len: usize) -> Option<f64> {
    let span = x.get(start..start.checked_add(len)?)?;
    if span.is_empty() {
        return None;
    }
    let (mut re, mut im, mut weight) = (0.0, 0.0, 0.0);
    for (i, &v) in span.iter().enumerate() {
        let w = hann(i, len);
        let (s, c) = carrier(hz, start + i).sin_cos();
        re += w * f64::from(v) * c;
        im -= w * f64::from(v) * s;
        weight += w;
    }
    Some(db(2.0 * (re * re + im * im).sqrt() / weight))
}

/// The frequency of the strongest peak within 100 Hz of `near` in `x[start..start + len]`: an
/// FFT under a Hann window, zero-padded to 4× and interpolated on a parabola through the log
/// magnitudes (signals.md §3.5).
pub fn peak_hz(x: &[f32], near: f64, start: usize, len: usize) -> Option<f64> {
    let span = x.get(start..start.checked_add(len)?)?;
    if span.len() < 8 {
        return None;
    }
    let size = (4 * span.len()).next_power_of_two();
    let mut re = vec![0.0; size];
    let mut im = vec![0.0; size];
    for (i, &v) in span.iter().enumerate() {
        re[i] = hann(i, len) * f64::from(v);
    }
    fft(&mut re, &mut im, false);
    let bin = RATE / size as f64;
    let first = (((near - 100.0) / bin).floor().max(1.0)) as usize;
    let last = (((near + 100.0) / bin).ceil() as usize).min(size / 2 - 2);
    if first >= last {
        return None;
    }
    let magnitude = |k: usize| (re[k] * re[k] + im[k] * im[k]).sqrt().max(1e-300).ln();
    let peak = (first..=last).max_by(|&a, &b| magnitude(a).total_cmp(&magnitude(b)))?;
    let (a, b, c) = (magnitude(peak - 1), magnitude(peak), magnitude(peak + 1));
    let curvature = a - 2.0 * b + c;
    let offset = if curvature < 0.0 {
        (0.5 * (a - c) / curvature).clamp(-0.5, 0.5)
    } else {
        0.0
    };
    Some((peak as f64 + offset) * bin)
}

/// A least-squares fit of sinusoids to a span.
#[derive(Debug, Clone, PartialEq)]
pub struct Fit {
    /// The fitted sinusoids' power over the residual's, in dB.
    pub snr_db: f64,
    /// Each sinusoid's frequency after refinement, in Hz.
    pub hz: Vec<f64>,
}

/// Fits a sinusoid at each of `freqs` (each refined from the phase advance between the span's
/// halves) and a constant to `x[start..start + len]` by least squares; the SNR is the
/// sinusoids' power over the residual's (signals.md §3.5). Several frequencies fit jointly,
/// for overlapping beeps in the Mix.
pub fn fit(x: &[f32], freqs: &[f64], start: usize, len: usize) -> Option<Fit> {
    let span = x.get(start..start.checked_add(len)?)?;
    if freqs.is_empty() || span.len() < 16 {
        return None;
    }
    let hz: Vec<f64> = freqs.iter().map(|&f| refine(span, start, f)).collect();
    // Columns: cos and sin of each sinusoid, then the constant.
    let m = 2 * hz.len() + 1;
    let column = |row: usize, col: usize| -> f64 {
        if col == m - 1 {
            return 1.0;
        }
        let phase = TAU * hz[col / 2] * row as f64 / RATE;
        if col.is_multiple_of(2) {
            phase.cos()
        } else {
            phase.sin()
        }
    };
    let mut normal = vec![vec![0.0; m + 1]; m];
    let mut row_values = vec![0.0; m];
    for (row, &v) in span.iter().enumerate() {
        for (col, value) in row_values.iter_mut().enumerate() {
            *value = column(row, col);
        }
        for i in 0..m {
            for j in 0..m {
                normal[i][j] += row_values[i] * row_values[j];
            }
            normal[i][m] += row_values[i] * f64::from(v);
        }
    }
    let coefficients = solve(normal)?;
    let (mut signal, mut residual) = (0.0, 0.0);
    for (row, &v) in span.iter().enumerate() {
        let tone: f64 = (0..m - 1)
            .map(|col| coefficients[col] * column(row, col))
            .sum();
        let fitted = tone + coefficients[m - 1];
        signal += tone * tone;
        residual += (f64::from(v) - fitted).powi(2);
    }
    let snr_db = 10.0 * (signal.max(1e-300) / residual.max(1e-300)).log10();
    Some(Fit { snr_db, hz })
}

/// `hz` corrected by the phase its carrier gains between the two halves of `span`, which starts
/// at sample `start`.
fn refine(span: &[f32], start: usize, hz: f64) -> f64 {
    let half = span.len() / 2;
    let sum = |from: usize| {
        let (mut re, mut im) = (0.0, 0.0);
        for (i, &v) in span[from..from + half].iter().enumerate() {
            let (s, c) = carrier(hz, start + from + i).sin_cos();
            re += f64::from(v) * c;
            im -= f64::from(v) * s;
        }
        (re, im)
    };
    let (a, b) = (sum(0), sum(half));
    // arg(b · conj(a)): how far the carrier drifted in `half` samples.
    let drift = (b.1 * a.0 - b.0 * a.1).atan2(b.0 * a.0 + b.1 * a.1);
    hz + drift * RATE / (TAU * half as f64)
}

/// Solves an augmented m × (m + 1) system by Gaussian elimination with partial pivoting.
fn solve(mut a: Vec<Vec<f64>>) -> Option<Vec<f64>> {
    let m = a.len();
    for col in 0..m {
        let pivot = (col..m).max_by(|&i, &j| a[i][col].abs().total_cmp(&a[j][col].abs()))?;
        if a[pivot][col].abs() < 1e-12 {
            return None;
        }
        a.swap(col, pivot);
        for row in col + 1..m {
            let (upper, lower) = a.split_at_mut(row);
            let (pivot_row, target) = (&upper[col], &mut lower[0]);
            let factor = target[col] / pivot_row[col];
            for (t, p) in target[col..].iter_mut().zip(&pivot_row[col..]) {
                *t -= factor * p;
            }
        }
    }
    let mut x = vec![0.0; m];
    for row in (0..m).rev() {
        let sum: f64 = (row + 1..m).map(|k| a[row][k] * x[k]).sum();
        x[row] = (a[row][m] - sum) / a[row][row];
    }
    Some(x)
}

/// The cue's best match in a track.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CueMatch {
    /// Where it starts, in samples.
    pub at: usize,
    /// Its normalised cross-correlation with the template, −1 to 1.
    pub ncc: f64,
    /// The level of the template-shaped part there: the template's peak scaled by the
    /// projection, in dBFS.
    pub level_db: f64,
}

/// The best match of `template` in `x` by normalised cross-correlation, computed with an
/// overlap-save FFT; windows quieter than −90 dBFS RMS count as silence, since AAC's exact
/// digital silence otherwise gives meaningless NCCs (signals.md §3.7). `None` if the track is
/// shorter than the template, or silent throughout.
pub fn match_cue(x: &[f32], template: &[f32]) -> Option<CueMatch> {
    let len = template.len();
    if len == 0 || x.len() < len {
        return None;
    }
    let energy: f64 = template.iter().map(|&v| f64::from(v).powi(2)).sum();
    let peak = template
        .iter()
        .map(|&v| f64::from(v).abs())
        .fold(0.0, f64::max);
    if energy <= 0.0 {
        return None;
    }
    let floor = len as f64 * 10_f64.powf(SILENCE_DB / 10.0);
    let size = (2 * len).next_power_of_two().max(1 << 15);
    let step = size - len + 1;
    // The reversed template's spectrum: correlation as a convolution.
    let table = twiddles(size);
    let (mut hr, mut hi) = (vec![0.0; size], vec![0.0; size]);
    for (i, &v) in template.iter().enumerate() {
        hr[len - 1 - i] = f64::from(v);
    }
    fft_with(&mut hr, &mut hi, false, &table);
    let last = x.len() - len;
    let mut best: Option<CueMatch> = None;
    let (mut re, mut im) = (vec![0.0; size], vec![0.0; size]);
    // The block's running sum of squares, for each window's energy: a block's windows all lie
    // inside it, so no whole-track array is needed.
    let mut energies = vec![0.0; size + 1];
    let mut block = 0;
    while block <= last {
        let mut total = 0.0;
        for (i, (r, m)) in re.iter_mut().zip(im.iter_mut()).enumerate() {
            *r = x.get(block + i).map_or(0.0, |&v| f64::from(v));
            *m = 0.0;
            total += *r * *r;
            energies[i + 1] = total;
        }
        fft_with(&mut re, &mut im, false, &table);
        for k in 0..size {
            let (a, b) = (re[k], im[k]);
            re[k] = a * hr[k] - b * hi[k];
            im[k] = a * hi[k] + b * hr[k];
        }
        fft_with(&mut re, &mut im, true, &table);
        for (m, &correlation) in re.iter().enumerate().skip(len - 1) {
            let at = block + m - (len - 1);
            if at > last {
                break;
            }
            let window = energies[m + 1] - energies[m + 1 - len];
            if window < floor {
                continue;
            }
            let ncc = correlation / (energy * window).sqrt();
            if best.is_none_or(|b| ncc > b.ncc) {
                best = Some(CueMatch {
                    at,
                    ncc,
                    level_db: db((correlation / energy).abs() * peak),
                });
            }
        }
        block += step;
    }
    best
}

/// An in-place radix-2 FFT of `re` + i·`im`, whose length must be a power of two; `inverse`
/// runs the inverse transform, scaled by 1/n.
pub(crate) fn fft(re: &mut [f64], im: &mut [f64], inverse: bool) {
    fft_with(re, im, inverse, &twiddles(re.len()));
}

/// The forward twiddle factors of an `n`-point FFT: e^(−2πik/n) for k < n/2.
fn twiddles(n: usize) -> Vec<(f64, f64)> {
    (0..n / 2)
        .map(|k| {
            let angle = -TAU * k as f64 / n as f64;
            (angle.cos(), angle.sin())
        })
        .collect()
}

/// [`fft`] with its twiddle factors built once, by [`twiddles`], for many transforms of a size.
fn fft_with(re: &mut [f64], im: &mut [f64], inverse: bool, twiddles: &[(f64, f64)]) {
    let n = re.len();
    if n < 2 || !n.is_power_of_two() || im.len() != n || twiddles.len() != n / 2 {
        return;
    }
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let sign = if inverse { -1.0 } else { 1.0 };
    let mut len = 2;
    while len <= n {
        let stride = n / len;
        for start in (0..n).step_by(len) {
            for k in 0..len / 2 {
                let (wr, wi) = twiddles[k * stride];
                let wi = sign * wi;
                let (a, b) = (start + k, start + k + len / 2);
                let (tr, ti) = (re[b] * wr - im[b] * wi, re[b] * wi + im[b] * wr);
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
            }
        }
        len <<= 1;
    }
    if inverse {
        let scale = 1.0 / n as f64;
        for (r, i) in re.iter_mut().zip(im.iter_mut()) {
            *r *= scale;
            *i *= scale;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kairo_core::testsig::{BEEP_MS, Tone};

    /// `seconds` of silence with beeps of `hz` at these onsets (in samples, fractional), each at
    /// testsig's level times `gain`.
    fn track(seconds: f64, beeps: &[(f64, f64, f64)]) -> Vec<f32> {
        let mut x = vec![0.0_f32; (seconds * RATE) as usize];
        for &(hz, onset, gain) in beeps {
            let first = onset.floor().max(0.0) as usize;
            let last = ((onset + f64::from(BEEP_MS) * MS).ceil() as usize + 1).min(x.len());
            for (n, v) in x.iter_mut().enumerate().take(last).skip(first) {
                *v +=
                    (gain * testsig::tone_burst_at(hz, BEEP_MS, (n as f64 - onset) / RATE)) as f32;
            }
        }
        x
    }

    /// Uniform noise of this RMS from a fixed seed.
    fn noise(x: &mut [f32], rms_db: f64, seed: u64) {
        let amplitude = 10_f64.powf(rms_db / 20.0) * 3_f64.sqrt();
        let mut state = seed;
        for v in x {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let unit = (state >> 11) as f64 / (1_u64 << 53) as f64;
            *v += (amplitude * (2.0 * unit - 1.0)) as f32;
        }
    }

    /// Adds a burst of `hz`, `ms` long, at `onset` (samples) and `gain` on testsig's level.
    fn add(x: &mut [f32], hz: f64, ms: u32, onset: f64, gain: f64) {
        let first = onset.floor().max(0.0) as usize;
        let last = ((onset + f64::from(ms) * MS).ceil() as usize + 1).min(x.len());
        for (n, v) in x.iter_mut().enumerate().take(last).skip(first) {
            *v += (gain * testsig::tone_burst_at(hz, ms, (n as f64 - onset) / RATE)) as f32;
        }
    }

    #[test]
    fn a_scan_keeps_every_beep_under_noise() {
        // PR 5a's review dsp-1: a fixed trigger fired on noise near −60 dBFS and missed beeps
        // under −55 dBFS. Now each beep is found once, on its own cycle.
        let onsets: Vec<f64> = (0..20)
            .map(|k| 4_800.0 + 48_000.0 * f64::from(k) + 0.37)
            .collect();
        for (hz, rms) in [
            (2_000.0, -60.0),
            (2_000.0, -50.0),
            (3_000.0, -60.0),
            (1_000.0, -55.0),
        ] {
            let beeps: Vec<(f64, f64, f64)> = onsets.iter().map(|&o| (hz, o, 1.0)).collect();
            let mut x = track(20.5, &beeps);
            noise(&mut x, rms, 11);
            let found = Detector::new(hz).scan(&x);
            assert_eq!(found.len(), onsets.len(), "{hz} Hz at {rms} dBFS");
            for (b, &o) in found.iter().zip(&onsets) {
                assert!(
                    (b.at - o).abs() < 0.25 * RATE / hz / 2.0,
                    "{hz} Hz at {rms}: {} vs {o}",
                    b.at
                );
            }
        }
    }

    #[test]
    fn other_sounds_near_a_beep_never_move_it_a_cycle() {
        // PR 5a's review dsp-2: a louder sound 400–500 Hz off a beep, in the same search window,
        // set the plateau and slipped the onset a cycle, or timed the beep at its own start. Now
        // a beep comes back on its own onset, or unmeasured when the sound overlaps its start.
        let onset = 24_000.3;
        let cue = |x: &mut [f32], at: f64, gain: f64| {
            for (n, v) in x.iter_mut().enumerate() {
                let t = (n as f64 - at) / RATE;
                if (0.0..0.22).contains(&t) {
                    *v += (gain * testsig::cue_placeholder_at(t)) as f32;
                }
            }
        };
        let six_db = 10_f64.powf(6.0 / 20.0);
        for hz in [1_000.0, 2_000.0, 3_000.0] {
            let detector = Detector::new(hz);
            // Apart from the beep: the cue 6 dB louder ending 40 ms before it or starting 50 ms
            // after it ends, and a 100 ms tone 400 Hz off starting 20 ms after it ends.
            type Sound<'a> = Box<dyn Fn(&mut [f32]) + 'a>;
            let apart: [Sound<'_>; 3] = [
                Box::new(|x: &mut [f32]| cue(x, onset - 260.0 * MS, six_db)),
                Box::new(|x: &mut [f32]| cue(x, onset + 150.0 * MS, six_db)),
                Box::new(|x: &mut [f32]| add(x, hz + 400.0, 100, onset + 120.0 * MS, six_db)),
            ];
            for (i, sound) in apart.iter().enumerate() {
                let mut x = track(1.0, &[(hz, onset, 1.0)]);
                sound(&mut x);
                let beep = detector.find(&x, onset, 250.0 * MS);
                assert!(
                    beep.is_some_and(|b| (b.at - onset).abs() < 0.05),
                    "{hz} Hz, sound {i}: {beep:?}"
                );
            }
            // Overlapping its start: a 200 ms tone 400 Hz off, as loud, from 50 ms before it.
            let mut x = track(1.0, &[(hz, onset, 1.0)]);
            add(&mut x, hz - 400.0, 200, onset - 50.0 * MS, 1.0);
            let beep = detector.find(&x, onset, 250.0 * MS);
            assert!(
                beep.is_none_or(|b| (b.at - onset).abs() < 0.05),
                "{hz} Hz, overlapped: {beep:?}"
            );
        }
    }

    #[test]
    fn the_fft_inverts_and_finds_a_cosine() {
        let n = 64;
        let (mut re, mut im): (Vec<f64>, Vec<f64>) = (
            (0..n)
                .map(|i| (TAU * 5.0 * i as f64 / n as f64).cos())
                .collect(),
            vec![0.0; n],
        );
        let original = re.clone();
        fft(&mut re, &mut im, false);
        assert!((re[5] - 32.0).abs() < 1e-9 && (re[59] - 32.0).abs() < 1e-9);
        assert!(
            re.iter()
                .enumerate()
                .all(|(k, &v)| k == 5 || k == 59 || v.abs() < 1e-9)
        );
        fft(&mut re, &mut im, true);
        assert!(re.iter().zip(&original).all(|(a, b)| (a - b).abs() < 1e-12));
        assert!(im.iter().all(|v| v.abs() < 1e-12));
    }

    #[test]
    fn the_templates_cross_where_the_design_says() {
        // signals.md §3.3's 0.953 ms at 1 kHz and 0.986 ms at 2 kHz, each about 10 µs later
        // here: the 48-sample boxcar centres half a sample late (PR 5a's review dsp-7).
        for (hz, ms) in [(1_000.0, 0.961), (2_000.0, 0.993)] {
            let crossing = Detector::new(hz).crossing_ms();
            assert!((crossing - ms).abs() < 0.005, "{hz} Hz: {crossing} ms");
        }
    }

    #[test]
    fn onsets_come_back_within_a_microsecond() {
        // Beeps at sub-sample onsets, with the other tones playing at the same time.
        for (i, &tone) in [Tone::Game, Tone::Voice, Tone::Mic].iter().enumerate() {
            let hz = f64::from(tone.hz());
            let detector = Detector::new(hz);
            for onset in [24_000.0, 24_000.37, 30_011.82, 40_123.5 + i as f64 / 7.0] {
                let x = track(
                    2.0,
                    &[
                        (hz, onset, 1.0),
                        (1_000.0 * (1 + (i + 1) % 3) as f64, onset + 0.3, 1.0),
                        (1_000.0 * (1 + (i + 2) % 3) as f64, onset - 1_000.0, 1.0),
                    ],
                );
                let beep = detector.find(&x, onset + 30.0, 250.0 * MS).unwrap();
                let error_us = (beep.at - onset).abs() / RATE * 1e6;
                assert!(error_us < 1.0, "{hz} Hz at {onset}: {error_us} µs");
                // The coarse onset picks the carrier's cycle: it must be within half of one.
                assert!((beep.coarse - onset).abs() < 0.5 * RATE / hz, "{beep:?}");
                assert!((beep.level_db + 30.0).abs() < 0.05, "{beep:?}");
                assert!((beep.hz - hz).abs() < 0.1, "{beep:?}");
            }
        }
    }

    #[test]
    fn quiet_off_pitch_and_missing_beeps_dont_count() {
        let detector = Detector::new(1_000.0);
        assert_eq!(detector.find(&vec![0.0; 48_000], 24_000.0, 12_000.0), None);
        // −35 dB below the beep's −30 dBFS: −65 dBFS, under the −60 dBFS gate.
        let quiet = track(1.0, &[(1_000.0, 20_000.0, 10_f64.powf(-35.0 / 20.0))]);
        let beep = detector.measure(&quiet, 20_000.0, 12_000.0).unwrap();
        assert!((beep.level_db + 65.0).abs() < 0.1 && !detector.counts(&beep));
        // A 44.1 kHz beep played at 48 kHz: 918.75 Hz.
        let fast = track(1.0, &[(918.75, 20_000.0, 1.0)]);
        assert!(detector.find(&fast, 20_000.0, 12_000.0).is_none());
        // The cue's notes aren't beeps either.
        let mut cue = vec![0.0_f32; 48_000];
        for (n, v) in cue.iter_mut().enumerate().skip(20_000).take(11_000) {
            *v = testsig::cue_placeholder_at((n - 20_000) as f64 / RATE) as f32;
        }
        for hz in [1_000.0, 2_000.0, 3_000.0] {
            assert!(
                Detector::new(hz).find(&cue, 20_000.0, 12_000.0).is_none(),
                "{hz}"
            );
        }
    }

    #[test]
    fn a_scan_finds_every_beep_once() {
        let onsets = [12_000.5, 60_000.25, 108_000.0, 156_000.75];
        let beeps: Vec<(f64, f64, f64)> = onsets.iter().map(|&o| (2_000.0, o, 1.0)).collect();
        let mut x = track(3.5, &beeps);
        // Game beeps between them don't confuse the 2 kHz scan.
        for &o in &onsets {
            for (n, v) in x
                .iter_mut()
                .enumerate()
                .skip(o as usize + 24_000)
                .take(4_800)
            {
                *v += testsig::tone_burst_at(1_000.0, BEEP_MS, (n as f64 - o - 24_000.0) / RATE)
                    as f32;
            }
        }
        let found = Detector::new(2_000.0).scan(&x);
        assert_eq!(found.len(), 4, "{found:?}");
        for (beep, &onset) in found.iter().zip(&onsets) {
            assert!((beep.at - onset).abs() < 0.05, "{beep:?} vs {onset}");
        }
        assert!(Detector::new(3_000.0).scan(&x).is_empty());
    }

    #[test]
    fn levels_read_peaks_and_silence() {
        let x = track(1.0, &[(1_000.0, 10_000.0, 1.0)]);
        let (start, len) = plateau(10_000.0).unwrap();
        let level = level_db(&x, 1_000.0, start, len).unwrap();
        assert!((level + 30.0).abs() < 0.01, "{level}");
        // Another tone in the same window reads far below the absent limit.
        assert!(level_db(&x, 2_000.0, start, len).unwrap() < -100.0);
        assert_eq!(
            level_db(&vec![0.0; 48_000], 1_000.0, 0, 3_840),
            Some(-240.0)
        );
        assert_eq!(level_db(&x, 1_000.0, 47_000, 3_840), None);
    }

    #[test]
    fn frequencies_come_from_the_peak() {
        for hz in [998.6, 1_000.0, 1_000.37, 2_000.0, 2_999.2] {
            let x = track(1.0, &[(hz, 1_000.0, 1.0)]);
            let (start, len) = plateau(1_000.0).unwrap();
            let found = peak_hz(&x, hz.round(), start, len).unwrap();
            assert!((found - hz).abs() < 0.05, "{hz}: {found}");
        }
    }

    #[test]
    fn the_snr_reads_the_noise() {
        let clean = track(1.0, &[(1_000.0, 4_800.0, 1.0)]);
        let (start, len) = plateau(4_800.0).unwrap();
        let fit = fit(&clean, &[1_000.0], start, len).unwrap();
        assert!(fit.snr_db > 80.0, "{fit:?}");
        assert!((fit.hz[0] - 1_000.0).abs() < 0.01);
        // −55 dBFS RMS of noise under a −30 dBFS peak tone (−33 dBFS RMS): about 22 dB
        // (README §7.3's `noisy`).
        let mut noisy = clean.clone();
        noise(&mut noisy, -55.0, 7);
        let fit = super::fit(&noisy, &[1_000.0], start, len).unwrap();
        assert!((fit.snr_db - 22.0).abs() < 1.0, "{fit:?}");
        // Two beeps at once fit jointly; one alone leaves the other as residual.
        let both = track(1.0, &[(1_000.0, 4_800.0, 1.0), (2_000.0, 4_800.0, 1.0)]);
        assert!(
            super::fit(&both, &[1_000.0, 2_000.0], start, len)
                .unwrap()
                .snr_db
                > 80.0
        );
        assert!(
            super::fit(&both, &[1_000.0], start, len)
                .unwrap()
                .snr_db
                .abs()
                < 1.0
        );
        // A tone 0.4 Hz off is refined.
        let off = track(1.0, &[(1_000.4, 4_800.0, 1.0)]);
        let fit = super::fit(&off, &[1_000.0], start, len).unwrap();
        assert!(
            (fit.hz[0] - 1_000.4).abs() < 0.01 && fit.snr_db > 60.0,
            "{fit:?}"
        );
    }

    #[test]
    fn the_cue_is_found_where_it_plays() {
        let template = testsig::cue_placeholder(48_000).unwrap();
        let mut x = track(3.0, &[(1_000.0, 24_000.0, 1.0), (2_000.0, 48_000.0, 1.0)]);
        noise(&mut x, -70.0, 3);
        let none = match_cue(&x, &template).unwrap();
        assert!(none.ncc < 0.1, "{none:?}");
        // At −50 dBFS (20 dB under the template), 1.4 s in, clear of the beeps as the
        // synthetic clips' +750 ms cue is.
        let gain = 10_f64.powf(-20.0 / 20.0) as f32;
        for (i, v) in x.iter_mut().skip(67_200).take(template.len()).enumerate() {
            *v += gain * template[i];
        }
        let found = match_cue(&x, &template).unwrap();
        assert_eq!(found.at, 67_200);
        assert!(found.ncc > 0.98, "{found:?}");
        assert!((found.level_db + 50.0).abs() < 0.5, "{found:?}");
        // Under a beep 20 dB louder, the NCC falls with the window's energy: a leaked cue is
        // judged where it plays alone.
        let mut over = track(1.0, &[(1_000.0, 4_800.0, 1.0)]);
        for (i, v) in over.iter_mut().skip(2_400).take(template.len()).enumerate() {
            *v += gain * template[i];
        }
        assert!(match_cue(&over, &template).unwrap().ncc < 0.5);
        // Silence and short tracks.
        assert_eq!(match_cue(&vec![0.0; 96_000], &template), None);
        assert_eq!(match_cue(&x[..100], &template), None);
    }

    #[test]
    fn downmix_averages_channels() {
        let mut out = Vec::new();
        downmix(&[1.0, 0.0, 0.5, 0.5, 0.25], 2, &mut out);
        assert_eq!(out, [0.5, 0.5]);
        downmix(&[0.1, 0.2], 1, &mut out);
        assert_eq!(out, [0.5, 0.5, 0.1, 0.2]);
    }
}
