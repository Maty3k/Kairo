// SPDX-License-Identifier: GPL-3.0-or-later
//! What the decoded pictures show of pattern v1 (docs/research/16-verify-testbed/README.md
//! §4.6, §6.3): each frame's barcode and flash, the testbed frame numbers unwrapped, and the
//! barcode line that ties clip time to testbed frames. [`cadence`] and [`flash`] judge them.
//!
//! One pass over the decode keeps a few numbers per frame, never the pixels, so memory stays
//! flat however long the clip is (README §6.7).

pub mod audio;
pub mod bars;
pub mod cadence;
pub mod flash;
pub mod levels;

use kairo_core::testsig::{
    self, Barcode, BarcodeError, CODE_COLS, CODE_ROWS, CodeCopy, Load, Options, Part, Patches,
    Pattern, PatternError, Rect, Scene, Size,
};

use crate::decode::Planes;

/// The luma halfway between black and white, for a frame whose barcode doesn't read.
pub const DEFAULT_MIDPOINT: f64 = 125.5;

/// The central 50% × 50% of a rectangle, at least 1 px each way: the part of a cell or patch
/// that is measured, clear of blur at its edges.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    /// Left edge.
    pub x0: i64,
    /// Top edge.
    pub y0: i64,
    /// Right edge (exclusive).
    pub x1: i64,
    /// Bottom edge (exclusive).
    pub y1: i64,
}

impl Window {
    pub(crate) fn central(rect: &Rect) -> Self {
        let middle = |a: i64, b: i64| {
            let quarter = (b - a) / 4;
            let (lo, hi) = (a + quarter, b - quarter);
            if hi > lo { (lo, hi) } else { (a, a + 1) }
        };
        let (x0, x1) = middle(rect.x0, rect.x1);
        let (y0, y1) = middle(rect.y0, rect.y1);
        Self { x0, y0, x1, y1 }
    }

    /// The mean luma of this window in `planes`.
    pub fn mean(&self, planes: &Planes<'_>) -> Option<f64> {
        planes.luma_mean((self.x0, self.y0), (self.x1, self.y1))
    }
}

/// Where pattern v1's barcode cells and flash sit in a clip: the testbed's client area scaled
/// to the picture (README §4.6 step 1), so the same layout rules give the positions. That holds
/// when the clip shows the client with its aspect kept, centre-cropped or pillarboxed
/// (signals.md §1.2: `x_u = (x + 0.5)·1080/H_out`); a stretched or letterboxed client reads
/// nothing, and `content.barcode` says to check the geometry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Geometry {
    cells: [[[Window; CODE_COLS]; CODE_ROWS]; 2],
    flash: Window,
}

impl Geometry {
    /// The layout for a `width` × `height` picture.
    pub fn new(width: u32, height: u32) -> Result<Self, PatternError> {
        let options = Options {
            pattern: Pattern::Full,
            nominal_fps: 60,
            run_tag: 0,
            load: Load::None,
            patches: Patches::Sdr,
        };
        let scene = Scene::new(Size::new(width, height)?, 0, &options)?;
        let empty = Window {
            x0: 0,
            y0: 0,
            x1: 0,
            y1: 0,
        };
        let mut geometry = Self {
            cells: [[[empty; CODE_COLS]; CODE_ROWS]; 2],
            flash: empty,
        };
        for rect in scene.rects() {
            match rect.part {
                Part::Cell(copy, row, col) => {
                    let k = usize::from(copy == CodeCopy::B);
                    if let Some(cell) = geometry
                        .cells
                        .get_mut(k)
                        .and_then(|c| c.get_mut(row))
                        .and_then(|r| r.get_mut(col))
                    {
                        *cell = Window::central(rect);
                    }
                }
                Part::Flash => geometry.flash = Window::central(rect),
                Part::Patch(_) => {}
            }
        }
        Ok(geometry)
    }

    /// The flash patch's window.
    pub fn flash(&self) -> Window {
        self.flash
    }

    /// One copy's cell means; a cell outside the picture reads as NaN, so the copy fails.
    fn means(&self, copy: usize, planes: &Planes<'_>) -> [[f64; CODE_COLS]; CODE_ROWS] {
        let mut means = [[f64::NAN; CODE_COLS]; CODE_ROWS];
        for (row, cells) in means.iter_mut().zip(&self.cells[copy]) {
            for (mean, cell) in row.iter_mut().zip(cells) {
                *mean = cell.mean(planes).unwrap_or(f64::NAN);
            }
        }
        means
    }
}

/// What one frame shows.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Reading {
    /// The barcode, if a copy reads (when both read, they agree).
    pub code: Option<Barcode>,
    /// Both copies read but name different frames (24-bit numbers): a torn frame.
    pub torn: Option<(u32, u32)>,
    /// Why barcode A didn't read, when neither copy did.
    pub error: Option<BarcodeError>,
    /// The flash patch's mean luma.
    pub flash_luma: Option<f64>,
}

impl Reading {
    /// The level the flash is compared with: the barcode's midpoint, else the nominal one.
    pub fn midpoint(&self) -> f64 {
        self.code.map_or(DEFAULT_MIDPOINT, |c| c.midpoint())
    }
}

/// Reads frame `planes`' two barcodes and its flash (README §4.6 steps 2–5).
pub fn read(geometry: &Geometry, planes: &Planes<'_>) -> Reading {
    let a = testsig::read_barcode(&geometry.means(0, planes));
    let b = testsig::read_barcode(&geometry.means(1, planes));
    let flash_luma = geometry.flash.mean(planes);
    let (code, torn, error) = match (a, b) {
        (Ok(a), Ok(b)) if a.frame24 != b.frame24 => (None, Some((a.frame24, b.frame24)), None),
        (Ok(a), Ok(b)) => (Some(if b.margin > a.margin { b } else { a }), None, None),
        (Ok(code), Err(_)) | (Err(_), Ok(code)) => (Some(code), None, None),
        (Err(error), Err(_)) => (None, None, Some(error)),
    };
    Reading {
        code,
        torn,
        error,
        flash_luma,
    }
}

/// Each frame's full testbed frame number, unwrapped from the 24-bit barcodes in order
/// (README §4.6 "Range"); `None` where no barcode read.
pub fn frame_numbers(readings: &[Reading]) -> Vec<Option<i64>> {
    let mut previous: Option<i64> = None;
    readings
        .iter()
        .map(|r| {
            let frame24 = r.code?.frame24;
            let n = match previous {
                Some(p) => testsig::unwrap_frame(p, frame24),
                None => i64::from(frame24),
            };
            previous = Some(n);
            Some(n)
        })
        .collect()
}

/// The barcode line: clip time = `alpha + beta × N`, in media ticks, from a robust fit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Line {
    /// Clip time at testbed frame 0.
    pub alpha: f64,
    /// Ticks per testbed frame.
    pub beta: f64,
    /// The frames the final fit kept.
    pub inliers: usize,
}

impl Line {
    /// The testbed's frame rate this line shows, in frames per second.
    pub fn rate(&self, timescale: u64) -> f64 {
        timescale as f64 / self.beta
    }
}

/// How far, in ticks, a frame may sit from the median residual and stay in the fit.
const LINE_REJECT_TICKS: f64 = 2.0;

/// Below this many testbed frames per output frame, the slope comes from least squares rather
/// than from the one-tick steps: under 1.5, an even delivery mixes steps whose lengths round
/// differently at the end-to-end slope (one and two frames at 75 Hz to 60 fps), and keeping
/// only the one-tick ones would measure the output's rate.
const STEP_SLOPE_MIN_RATIO: f64 = 1.5;

/// Why no barcode line fits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoLine {
    /// Fewer than two readable frames after frame 0.
    TooFew,
    /// Every readable frame after frame 0 shows this one testbed frame.
    Frozen(i64),
    /// No readable frame shows a later testbed frame than the one before it.
    NoAdvance,
}

impl std::fmt::Display for NoLine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooFew => f.write_str("fewer than two readable frames after frame 0"),
            Self::Frozen(n) => write!(
                f,
                "every readable frame after frame 0 shows testbed frame {n}"
            ),
            Self::NoAdvance => f.write_str("the testbed frame number never advances"),
        }
    }
}

/// Fits clip time against testbed frame number over frames 1 on (frame 0 is the held first
/// frame; README §6.3 `cadence.drops_dups`). The first slope:
/// - with at least 1.5 testbed frames per output frame, from the steps alone: the end-to-end
///   slope, then the time over the frames of the steps that look like one tick at that slope,
///   so drops and repeats don't bend it while judder's uneven steps average out;
/// - with fewer, from least squares over every readable frame;
/// - when the clip ends at or before the testbed frame it starts on, from the median slope of
///   the steps that advance.
///
/// The line then goes through the median residual, and least squares is refitted three times
/// without the frames ≥2 ticks from it.
pub fn barcode_line(pts: &[i64], numbers: &[Option<i64>]) -> Result<Line, NoLine> {
    // (output frame, testbed frame, clip time).
    let points: Vec<(usize, f64, f64)> = pts
        .iter()
        .zip(numbers)
        .enumerate()
        .skip(1)
        .filter_map(|(i, (&t, n))| Some((i, n.as_ref().copied()? as f64, t as f64)))
        .collect();
    let (Some(&first), Some(&last)) = (points.first(), points.last()) else {
        return Err(NoLine::TooFew);
    };
    if points.len() < 2 {
        return Err(NoLine::TooFew);
    }
    if points.iter().all(|p| p.1 == first.1) {
        return Err(NoLine::Frozen(first.1 as i64));
    }
    let pairs: Vec<(f64, f64)> = points.iter().map(|&(_, n, t)| (n, t)).collect();
    let ratio = (last.1 - first.1) / (last.0 - first.0) as f64;
    let end_to_end = (last.2 - first.2) / (last.1 - first.1);
    let advancing = end_to_end.is_finite() && end_to_end > 0.0;
    let seed = if advancing && ratio >= STEP_SLOPE_MIN_RATIO {
        Some(step_slope(&pairs, end_to_end))
    } else if advancing {
        fit(&pairs).map(|l| l.beta)
    } else {
        None
    };
    let beta = seed
        .filter(|b| b.is_finite() && *b > 0.0)
        .or_else(|| median_step_slope(&pairs))
        .ok_or(NoLine::NoAdvance)?;
    let mut offsets: Vec<f64> = pairs.iter().map(|&(n, t)| t - beta * n).collect();
    offsets.sort_by(f64::total_cmp);
    let mut line = Line {
        alpha: offsets[offsets.len() / 2],
        beta,
        inliers: pairs.len(),
    };
    for _ in 0..3 {
        let mut residuals: Vec<f64> = pairs
            .iter()
            .map(|&(n, t)| t - (line.alpha + line.beta * n))
            .collect();
        residuals.sort_by(f64::total_cmp);
        let median = residuals[residuals.len() / 2];
        let kept: Vec<(f64, f64)> = pairs
            .iter()
            .copied()
            .filter(|&(n, t)| (t - (line.alpha + line.beta * n) - median).abs() < LINE_REJECT_TICKS)
            .collect();
        match fit(&kept) {
            Some(refit) => line = refit,
            None => break,
        }
    }
    Ok(line)
}

/// The slope from the one-tick steps: starting at `beta`, the time over the frames of the
/// steps that are one tick at the current slope, four times.
fn step_slope(points: &[(f64, f64)], mut beta: f64) -> f64 {
    for _ in 0..4 {
        let (mut time, mut frames) = (0.0, 0.0);
        for pair in points.windows(2) {
            let (dn, dt) = (pair[1].0 - pair[0].0, pair[1].1 - pair[0].1);
            if dt > 0.0 && (dn * beta / dt).round() == 1.0 {
                time += dt;
                frames += dn;
            }
        }
        if frames <= 0.0 {
            break;
        }
        beta = time / frames;
    }
    beta
}

/// The median of `dt / dn` over the steps that advance in both, if any.
fn median_step_slope(points: &[(f64, f64)]) -> Option<f64> {
    let mut slopes: Vec<f64> = points
        .windows(2)
        .filter_map(|pair| {
            let (dn, dt) = (pair[1].0 - pair[0].0, pair[1].1 - pair[0].1);
            (dn > 0.0 && dt > 0.0).then_some(dt / dn)
        })
        .collect();
    if slopes.is_empty() {
        return None;
    }
    slopes.sort_by(f64::total_cmp);
    Some(slopes[slopes.len() / 2])
}

/// Ordinary least squares of t on n.
fn fit(points: &[(f64, f64)]) -> Option<Line> {
    let count = points.len() as f64;
    if points.len() < 2 {
        return None;
    }
    let (mean_n, mean_t) = points
        .iter()
        .fold((0.0, 0.0), |(a, b), &(n, t)| (a + n / count, b + t / count));
    let (mut snn, mut snt) = (0.0, 0.0);
    for &(n, t) in points {
        snn += (n - mean_n) * (n - mean_n);
        snt += (n - mean_n) * (t - mean_t);
    }
    if snn <= 0.0 {
        return None;
    }
    let beta = snt / snn;
    (beta > 0.0).then_some(Line {
        alpha: mean_t - beta * mean_n,
        beta,
        inliers: points.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decode::synthetic::{Damage, Synthetic};

    fn options(tag: u8) -> Options {
        Options {
            pattern: Pattern::Full,
            nominal_fps: 180,
            run_tag: tag,
            load: Load::Scroll,
            patches: Patches::Sdr,
        }
    }

    fn readings(size: (u32, u32), content: Vec<u64>, damage: Damage) -> Vec<Reading> {
        let synthetic = Synthetic::new(
            Size::new(size.0, size.1).unwrap(),
            options(9),
            content.clone(),
            damage,
        );
        let geometry = Geometry::new(size.0, size.1).unwrap();
        (0..content.len())
            .map(|i| read(&geometry, &synthetic.frame(i).unwrap().planes().unwrap()))
            .collect()
    }

    #[test]
    fn barcodes_read_at_every_size_through_noise_and_scaling() {
        for (size, damage) in [
            ((320, 180), Damage::default()),
            ((640, 360), Damage::default()),
            (
                (320, 180),
                Damage {
                    noise: 12,
                    ..Damage::default()
                },
            ),
            (
                (320, 180),
                Damage {
                    scale_from: Some(Size::new(480, 270).unwrap()),
                    ..Damage::default()
                },
            ),
            (
                (320, 180),
                Damage {
                    shift: 0.5,
                    quantise: 8,
                    noise: 4,
                    ..Damage::default()
                },
            ),
        ] {
            let content: Vec<u64> = (0..8).map(|k| 0xFF_FFF0 + 3 * k).collect();
            for (k, r) in readings(size, content.clone(), damage.clone())
                .iter()
                .enumerate()
            {
                let code = r
                    .code
                    .unwrap_or_else(|| panic!("{size:?} {damage:?} frame {k}: {r:?}"));
                assert_eq!(u64::from(code.frame24), content[k] & 0xFF_FFFF);
                assert_eq!(code.tag, 9);
                assert!(!code.hdr);
            }
        }
    }

    #[test]
    fn garbled_and_torn_frames_are_told_apart() {
        let damage = Damage {
            garble: vec![1],
            tear: vec![2],
            ..Damage::default()
        };
        let r = readings((320, 180), vec![30, 33, 36, 39], damage);
        assert!(r[0].code.is_some() && r[3].code.is_some());
        assert_eq!((r[1].code, r[1].torn), (None, None));
        assert!(r[1].error.is_some());
        assert_eq!((r[2].code, r[2].torn), (None, Some((36, 39))));
        // The flash is lit on frame 30 of 180 fps (30 mod 180 < 18 is false): unlit, and the
        // first frame's midpoint is the barcode's.
        assert!(r[0].flash_luma.unwrap() < r[0].midpoint());
    }

    #[test]
    fn frame_numbers_unwrap_across_the_24_bit_wrap() {
        let reading = |frame24: Option<u32>| Reading {
            code: frame24.map(|f| Barcode {
                frame24: f,
                hdr: false,
                tag: 0,
                margin: 1.0,
                white: 235.0,
                black: 16.0,
            }),
            torn: None,
            error: None,
            flash_luma: None,
        };
        let r: Vec<Reading> = [Some(0xFF_FFFD), None, Some(0x00_0003), Some(0x00_0006)]
            .into_iter()
            .map(reading)
            .collect();
        assert_eq!(
            frame_numbers(&r),
            [Some(0xFF_FFFD), None, Some(0x100_0003), Some(0x100_0006)]
        );
    }

    #[test]
    fn the_barcode_line_ignores_the_held_first_frame_and_outliers() {
        // 60 fps at 1/15360 showing 180 fps content: 256 ticks per output frame, 3 testbed
        // frames per output frame. Frame 0 is held 584 ticks; frame 40 arrives 30 ticks late.
        let mut pts: Vec<i64> = (0..120).map(|i| 584 - 256 + 256 * i).collect();
        pts[0] = 0;
        pts[40] += 30;
        let numbers: Vec<Option<i64>> = (0..120).map(|i| Some(3 * i)).collect();
        let line = barcode_line(&pts, &numbers).unwrap();
        assert!((line.beta - 256.0 / 3.0).abs() < 1e-9, "{line:?}");
        assert!((line.alpha - 328.0).abs() < 1e-6, "{line:?}");
        assert_eq!(line.inliers, 118);
        assert!((line.rate(15_360) - 180.0).abs() < 1e-9);
        assert_eq!(barcode_line(&pts[..2], &numbers[..2]), Err(NoLine::TooFew));
        assert_eq!(
            barcode_line(&[0, 1, 2], &[None, Some(5), Some(5)]),
            Err(NoLine::Frozen(5))
        );
        assert_eq!(
            barcode_line(&[0, 1, 2, 3], &[None, Some(9), Some(7), Some(5)]),
            Err(NoLine::NoAdvance)
        );
    }

    #[test]
    fn rates_near_the_output_rate_keep_their_own_slope() {
        // Every vblank of a display near the clip's rate, 30 s: the steps mix 0, 1 and 2
        // frames, which the one-tick filter alone would read as the output rate (review F1).
        for (source, fps, ticks) in [
            (75.0, 60.0, 256),
            (50.0, 60.0, 256),
            (144.0, 120.0, 128),
            (100.0, 120.0, 128),
            (90.0, 120.0, 128),
            (72.0, 60.0, 256),
            (45.0, 60.0, 256),
            (180.0, 60.0, 256),
            (144.0, 60.0, 256),
            (60.0, 60.0, 256),
        ] {
            let frames = (30.0 * fps) as i64;
            let pts: Vec<i64> = (0..frames).map(|i| 328 + ticks * i).collect();
            let numbers: Vec<Option<i64>> = (0..frames)
                .map(|k| Some(1_000 + (k as f64 * source / fps + 0.37).floor() as i64))
                .collect();
            let line = barcode_line(&pts, &numbers).unwrap();
            let rate = line.rate(15_360);
            assert!((rate - source).abs() < 0.01, "{source} at {fps}: {rate}");
        }
    }

    #[test]
    fn clips_that_stall_or_go_back_still_get_a_slope() {
        let pts: Vec<i64> = (0..120).map(|i| 328 + 256 * i).collect();
        // The newer half first, as a ring buffer read from its start would give; a testbed
        // that restarts; a last frame that shows frame 1's picture again.
        type Numbering = fn(i64) -> i64;
        let cases: [(&str, Numbering); 3] = [
            ("swapped halves", |k| {
                if k < 60 {
                    1_180 + 3 * k
                } else {
                    1_000 + 3 * (k - 60)
                }
            }),
            ("a restart", |k| {
                if k < 60 { 1_000 + 3 * k } else { 3 * (k - 60) }
            }),
            ("frame 1 again at the end", |k| {
                if k == 119 { 1_003 } else { 1_000 + 3 * k }
            }),
        ];
        for (name, n) in cases {
            let numbers: Vec<Option<i64>> = (0..120).map(|k| Some(n(k))).collect();
            let line = barcode_line(&pts, &numbers).unwrap();
            assert!((line.rate(15_360) - 180.0).abs() < 0.01, "{name}: {line:?}");
        }
    }

    #[test]
    fn drops_and_judder_dont_bend_the_slope() {
        let pts: Vec<i64> = (0..120).map(|i| 328 + 256 * i).collect();
        // Each output frame's testbed frame, from its index.
        type Numbering = fn(i64) -> i64;
        let cases: [(&str, Numbering); 3] = [
            ("a drop at 60", |k| 3 * (k + i64::from(k >= 60))),
            ("a repeat at 60", |k| 3 * (k - i64::from(k >= 60))),
            ("judder 2, 4, 2, 4", |k| 3 * k - (k % 2)),
        ];
        for (name, n) in cases {
            let numbers: Vec<Option<i64>> = (0..120).map(|k| Some(1_000 + n(k))).collect();
            let line = barcode_line(&pts, &numbers).unwrap();
            assert!((line.rate(15_360) - 180.0).abs() < 0.01, "{name}: {line:?}");
        }
    }
}
