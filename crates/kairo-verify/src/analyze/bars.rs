// SPDX-License-Identifier: GPL-3.0-or-later
//! The moving bars and the ramp (docs/research/16-verify-testbed/README.md §2 item 14, §6.3;
//! signals.md §2): PSNR against the analytic reference, the colour-matrix test, and row and
//! column line errors.
//!
//! One pass over each frame keeps, per band, each column's sum and sum of squares and each
//! row's sum (README §6.7). The reference is constant down a band's rows, so the PSNR against
//! any reference and both line errors follow from those sums in O(width), and the four
//! wrong-colour references cost almost nothing. The reference is the testbed's own formula,
//! unrounded, at the clip's pixel centres, converted to 4:2:0 with chroma co-sited with the even
//! columns and filtered [1 2 1]/4 (README §4.5).

use kairo_core::testsig::{self, BARS_U, Matrix, RAMP_U, Range};

use crate::decode::Planes;

/// Rows trimmed off each band's top and bottom, clear of its hard horizontal edges.
pub const MARGIN_PX: i64 = 2;

/// The PSNR reported when a plane matches its reference exactly.
pub const PERFECT_DB: f64 = 100.0;

/// The rows a band is measured over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Rows {
    /// Luma rows `y0..y1`.
    y0: usize,
    y1: usize,
    /// Chroma rows `c0..c1`: those whose two luma rows are both inside.
    c0: usize,
    c1: usize,
}

/// Where the bars and the ramp sit in a picture of one size, and each column's centre in layout
/// units.
#[derive(Debug, Clone, PartialEq)]
pub struct Bands {
    width: usize,
    height: usize,
    bars: Rows,
    ramp: Rows,
    x_u: Vec<f64>,
}

impl Bands {
    /// The bands of a `width` × `height` picture; `None` if a band has no rows left inside its
    /// margins.
    pub fn new(width: usize, height: usize) -> Option<Self> {
        let h = u32::try_from(height).ok()?;
        let rows = |(top, bottom): (i64, i64)| -> Option<Rows> {
            let y0 = usize::try_from(testsig::px_y(top, h) + MARGIN_PX).ok()?;
            let y1 = usize::try_from(testsig::px_y(bottom, h) - MARGIN_PX)
                .ok()?
                .min(height);
            (y1 > y0).then_some(Rows {
                y0,
                y1,
                c0: y0.div_ceil(2),
                c1: (y1 / 2).max(y0.div_ceil(2)),
            })
        };
        Some(Self {
            width,
            height,
            bars: rows(BARS_U)?,
            ramp: rows(RAMP_U)?,
            x_u: (0..width)
                .map(|x| testsig::unit_of_pixel(x as i64, h))
                .collect(),
        })
    }

    /// The size these bands are for.
    pub fn size(&self) -> (usize, usize) {
        (self.width, self.height)
    }

    /// The sums of `planes` over the bands, or `None` if the picture is another size.
    pub fn sums(&self, planes: &Planes<'_>) -> Option<BandSums> {
        if (planes.width(), planes.height()) != (self.width, self.height) {
            return None;
        }
        let cw = self.width.div_ceil(2);
        let luma = |r: Rows| sum_rows((r.y0..r.y1).map(|y| planes.y_row(y)), self.width);
        let cb = |r: Rows| sum_rows((r.c0..r.c1).map(|j| planes.cb_row(j)), cw);
        let cr = |r: Rows| sum_rows((r.c0..r.c1).map(|j| planes.cr_row(j)), cw);
        Some(BandSums {
            bars_y: luma(self.bars),
            ramp_y: luma(self.ramp),
            bars_cb: cb(self.bars),
            bars_cr: cr(self.bars),
            ramp_cb: cb(self.ramp),
            ramp_cr: cr(self.ramp),
        })
    }
}

/// One plane's sums over a band's rows.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Sums {
    /// How many rows.
    rows: usize,
    /// Per column: Σv and Σv².
    col: Vec<f64>,
    col2: Vec<f64>,
    /// Per row: Σv.
    row: Vec<f64>,
}

fn sum_rows<'a>(rows: impl Iterator<Item = &'a [u8]>, width: usize) -> Sums {
    let mut s = Sums {
        rows: 0,
        col: vec![0.0; width],
        col2: vec![0.0; width],
        row: Vec::new(),
    };
    for row in rows {
        let mut total = 0_u64;
        for ((c, c2), &v) in s.col.iter_mut().zip(s.col2.iter_mut()).zip(row) {
            let v64 = f64::from(v);
            *c += v64;
            *c2 += v64 * v64;
            total += u64::from(v);
        }
        s.row.push(total as f64);
        s.rows += 1;
    }
    s
}

/// What a frame's bands hold: enough to score it against any reference and any frame number.
#[derive(Debug, Clone, PartialEq)]
pub struct BandSums {
    bars_y: Sums,
    ramp_y: Sums,
    bars_cb: Sums,
    bars_cr: Sums,
    ramp_cb: Sums,
    ramp_cr: Sums,
}

/// A colour reference (README §6.3 `content.colour_matrix`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reference {
    /// What Kairo writes: BT.709, limited range.
    Bt709Limited,
    /// BT.601, limited range.
    Bt601Limited,
    /// BT.709, full range.
    Bt709Full,
    /// BT.601, full range.
    Bt601Full,
    /// BT.709 limited with Cb and Cr swapped.
    Bt709Swapped,
}

impl Reference {
    /// The wrong ones the colour-matrix test tries.
    pub const OTHERS: [Reference; 4] = [
        Reference::Bt601Limited,
        Reference::Bt709Full,
        Reference::Bt601Full,
        Reference::Bt709Swapped,
    ];

    /// Its name in reports.
    pub fn name(self) -> &'static str {
        match self {
            Self::Bt709Limited => "BT.709 limited",
            Self::Bt601Limited => "matrix BT.601",
            Self::Bt709Full => "full range",
            Self::Bt601Full => "matrix BT.601, full range",
            Self::Bt709Swapped => "Cb and Cr swapped",
        }
    }

    fn colour(self) -> (Matrix, Range) {
        match self {
            Self::Bt709Limited | Self::Bt709Swapped => (Matrix::Bt709, Range::Limited),
            Self::Bt601Limited => (Matrix::Bt601, Range::Limited),
            Self::Bt709Full => (Matrix::Bt709, Range::Full),
            Self::Bt601Full => (Matrix::Bt601, Range::Full),
        }
    }
}

/// One band's reference on one frame: Y per column, Cb and Cr per chroma column.
struct Line {
    y: Vec<f64>,
    cb: Vec<f64>,
    cr: Vec<f64>,
}

fn reference(bands: &Bands, frame: u64, r: Reference, ramp: bool) -> Line {
    let (matrix, range) = r.colour();
    let full: Vec<[f64; 3]> = bands
        .x_u
        .iter()
        .map(|&x_u| {
            let rgb = if ramp {
                [testsig::ramp_code(x_u, frame); 3]
            } else {
                testsig::bars_codes(x_u, frame)
            };
            testsig::ycbcr(rgb, matrix, range)
        })
        .collect();
    let w = full.len();
    // Chroma sample i sits on luma column 2i: [1 2 1]/4 across it and its neighbours, the
    // edges repeated. Down the band nothing changes, so the vertical [1 1]/2 does nothing.
    let chroma = |c: usize| -> Vec<f64> {
        (0..w.div_ceil(2))
            .map(|i| {
                let x = 2 * i;
                let (l, r) = (x.saturating_sub(1), (x + 1).min(w - 1));
                (full[l][c] + 2.0 * full[x][c] + full[r][c]) / 4.0
            })
            .collect()
    };
    let (mut cb, mut cr) = (chroma(1), chroma(2));
    if r == Reference::Bt709Swapped {
        std::mem::swap(&mut cb, &mut cr);
    }
    Line {
        y: full.iter().map(|c| c[0]).collect(),
        cb,
        cr,
    }
}

/// The squared error of `sums` against a reference of one value per column.
fn sse(sums: &Sums, r: &[f64]) -> f64 {
    let n = sums.rows as f64;
    sums.col
        .iter()
        .zip(&sums.col2)
        .zip(r)
        .map(|((&s, &s2), &r)| s2 - 2.0 * r * s + n * r * r)
        .sum::<f64>()
        .max(0.0)
}

fn psnr(sse: f64, samples: usize) -> f64 {
    if samples == 0 {
        return f64::NAN;
    }
    let mse = sse / samples as f64;
    if mse < 1e-10 {
        PERFECT_DB
    } else {
        (10.0 * (255.0 * 255.0 / mse).log10()).min(PERFECT_DB)
    }
}

/// A frame's PSNR per plane: Y over the bars and the ramp, Cb and Cr over the bars.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Score {
    /// Y.
    pub y: f64,
    /// Cb.
    pub cb: f64,
    /// Cr.
    pub cr: f64,
}

impl Score {
    /// The worst plane, which is the frame's PSNR (signals.md §2.2: never averaged).
    pub fn min(&self) -> f64 {
        self.y.min(self.cb).min(self.cr)
    }
}

fn score(bands: &Bands, sums: &BandSums, frame: u64, r: Reference) -> Score {
    let (bars, ramp) = (
        reference(bands, frame, r, false),
        reference(bands, frame, r, true),
    );
    let cw = bands.width.div_ceil(2);
    Score {
        y: psnr(
            sse(&sums.bars_y, &bars.y) + sse(&sums.ramp_y, &ramp.y),
            (sums.bars_y.rows + sums.ramp_y.rows) * bands.width,
        ),
        cb: psnr(sse(&sums.bars_cb, &bars.cb), sums.bars_cb.rows * cw),
        cr: psnr(sse(&sums.bars_cr, &bars.cr), sums.bars_cr.rows * cw),
    }
}

/// What a frame's bands measured.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameBars {
    /// The testbed frame scored (for an unreadable frame, the best fitting one).
    pub frame: u64,
    /// Against BT.709 limited.
    pub score: Score,
    /// The wrong reference that fits best, with its PSNR (the worst plane).
    pub best_other: (Reference, f64),
    /// The largest |mean residual| of any row or column on Y, in codes.
    pub line: f64,
    /// The largest |column mean − 128| of the ramp's Cb and Cr, in codes.
    pub ramp_chroma: f64,
}

/// Scores a frame's band sums as testbed frame `frame`.
pub fn judge(bands: &Bands, sums: &BandSums, frame: u64) -> FrameBars {
    let nominal = score(bands, sums, frame, Reference::Bt709Limited);
    let best_other = Reference::OTHERS
        .into_iter()
        .map(|r| (r, score(bands, sums, frame, r).min()))
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .unwrap_or((Reference::Bt601Limited, f64::NAN));
    let mut line = 0.0_f64;
    // The bars scroll right and the ramp left, so new content enters at the picture's sides each
    // frame, where an encoder can't predict it: VideoToolbox's good 1080p clip reads 4.6 codes at
    // x = 0 and 2.3 inside. The columns leave one macroblock out at each side; rows keep the
    // full width.
    let edge = if bands.width > 64 { 16 } else { 0 };
    for (s, ramp) in [(&sums.bars_y, false), (&sums.ramp_y, true)] {
        let r = reference(bands, frame, Reference::Bt709Limited, ramp).y;
        let n = s.rows as f64;
        if s.rows > 0 {
            for (&c, &r) in s.col.iter().zip(&r).take(bands.width - edge).skip(edge) {
                line = line.max((c / n - r).abs());
            }
        }
        let mean_r = r.iter().sum::<f64>() / r.len().max(1) as f64;
        for &row in &s.row {
            line = line.max((row / bands.width as f64 - mean_r).abs());
        }
    }
    let mut ramp_chroma = 0.0_f64;
    for s in [&sums.ramp_cb, &sums.ramp_cr] {
        if s.rows > 0 {
            for &c in &s.col {
                ramp_chroma = ramp_chroma.max((c / s.rows as f64 - 128.0).abs());
            }
        }
    }
    FrameBars {
        frame,
        score: nominal,
        best_other,
        line,
        ramp_chroma,
    }
}

/// Scores a frame whose barcode didn't read against each candidate frame number, keeping the
/// best fit (README §6.3: corruption that also hits the barcode can't hide as "unreadable").
pub fn judge_best(
    bands: &Bands,
    sums: &BandSums,
    candidates: impl IntoIterator<Item = u64>,
) -> Option<FrameBars> {
    candidates
        .into_iter()
        .map(|n| judge(bands, sums, n))
        .max_by(|a, b| a.score.min().total_cmp(&b.score.min()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decode::synthetic::{Damage, Synthetic};
    use kairo_core::testsig::{Load, Options, Patches, Pattern, Size};

    fn options() -> Options {
        Options {
            pattern: Pattern::Full,
            nominal_fps: 180,
            run_tag: 0,
            load: Load::Scroll,
            patches: Patches::Sdr,
        }
    }

    fn frame_bars(size: (u32, u32), n: u64, damage: Damage, scored_as: u64) -> FrameBars {
        let synthetic = Synthetic::new(
            Size::new(size.0, size.1).unwrap(),
            options(),
            vec![n],
            damage,
        );
        let picture = synthetic.frame(0).unwrap();
        let bands = Bands::new(picture.width, picture.height).unwrap();
        let sums = bands.sums(&picture.planes().unwrap()).unwrap();
        judge(&bands, &sums, scored_as)
    }

    #[test]
    fn a_clean_frame_scores_at_the_rounding_floor() {
        for size in [(320, 180), (640, 360), (1280, 720)] {
            let f = frame_bars(size, 1_234, Damage::default(), 1_234);
            // 8-bit rounding of an exact render: 55-62 dB (signals.md §2.1: 59-62 lossless).
            assert!(f.score.min() > 50.0, "{size:?}: {f:?}");
            assert!(f.best_other.1 < f.score.min() - 6.0, "{size:?}: {f:?}");
            assert!(f.line < 1.0 && f.ramp_chroma < 1.0, "{size:?}: {f:?}");
        }
    }

    #[test]
    fn the_wrong_frame_number_scores_badly() {
        // The bars move 3 u a frame: one frame off is a clear miss, and an unreadable frame is
        // matched to the frame number that fits.
        let wrong = frame_bars((640, 360), 1_234, Damage::default(), 1_244);
        assert!(wrong.score.min() < 35.0, "{wrong:?}");
        let synthetic = Synthetic::new(
            Size::new(640, 360).unwrap(),
            options(),
            vec![1_234],
            Damage::default(),
        );
        let picture = synthetic.frame(0).unwrap();
        let bands = Bands::new(640, 360).unwrap();
        let sums = bands.sums(&picture.planes().unwrap()).unwrap();
        let best = judge_best(&bands, &sums, 1_220..1_250).unwrap();
        assert_eq!(best.frame, 1_234);
    }

    #[test]
    fn colour_faults_fit_their_own_reference_best() {
        // signals.md §2.4: BT.601, full range and swapped chroma each fit their reference
        // better than BT.709 limited by far more than 6 dB.
        for (damage, expected) in [
            (
                Damage {
                    colour: Some((Matrix::Bt601, Range::Limited)),
                    ..Damage::default()
                },
                Reference::Bt601Limited,
            ),
            (
                Damage {
                    colour: Some((Matrix::Bt709, Range::Full)),
                    ..Damage::default()
                },
                Reference::Bt709Full,
            ),
            (
                Damage {
                    swap_chroma: true,
                    ..Damage::default()
                },
                Reference::Bt709Swapped,
            ),
        ] {
            let f = frame_bars((640, 360), 500, damage, 500);
            assert_eq!(f.best_other.0, expected, "{f:?}");
            assert!(f.best_other.1 > f.score.min() + 6.0, "{f:?}");
            assert!(f.score.min() < 40.0, "{f:?}");
        }
    }

    #[test]
    fn gridlines_fail_psnr_or_the_line_rule() {
        // README §7.3: ±48 every 16 px is far under 40 dB; +24 every 64 px shows as lines.
        let dense = frame_bars(
            (640, 360),
            77,
            Damage {
                gridlines: Some((16, 48)),
                ..Damage::default()
            },
            77,
        );
        assert!(dense.score.min() < 40.0, "{dense:?}");
        let sparse = frame_bars(
            (640, 360),
            77,
            Damage {
                gridlines: Some((64, 24)),
                ..Damage::default()
            },
            77,
        );
        assert!(sparse.line > 4.0, "{sparse:?}");
    }

    #[test]
    fn the_sides_where_new_content_enters_are_left_out_of_the_columns() {
        // A column 24 codes up through the bars band: at either side it is where the scrolling
        // brings new content in, and doesn't count; at x = 64 it does (review F1 of PR 4b).
        let synthetic = Synthetic::new(
            Size::new(640, 360).unwrap(),
            options(),
            vec![300],
            Damage::default(),
        );
        let clean = synthetic.frame(0).unwrap();
        let bands = Bands::new(640, 360).unwrap();
        let top = kairo_core::testsig::px_y(kairo_core::testsig::BARS_U.0, 360) as usize + 2;
        let bottom = kairo_core::testsig::px_y(kairo_core::testsig::BARS_U.1, 360) as usize - 2;
        let line_with = |x: usize| {
            let mut picture = clean.clone();
            for y in top..bottom {
                let v = &mut picture.y[y * 640 + x];
                *v = v.saturating_add(24);
            }
            let sums = bands.sums(&picture.planes().unwrap()).unwrap();
            judge(&bands, &sums, 300).line
        };
        for x in [0, 15, 624, 639] {
            assert!(line_with(x) < 1.0, "x = {x}: {}", line_with(x));
        }
        assert!(line_with(64) > 20.0);
    }

    #[test]
    fn bands_fit_small_pictures_or_say_so() {
        assert!(Bands::new(320, 180).is_some());
        // At 18 px high the ramp band is 1 px: nothing inside its margins.
        assert!(Bands::new(32, 18).is_none());
        let bands = Bands::new(320, 180).unwrap();
        let other = crate::decode::Picture {
            width: 2,
            height: 2,
            y: vec![0; 4],
            cb: vec![0; 1],
            cr: vec![0; 1],
        };
        assert!(bands.sums(&other.planes().unwrap()).is_none());
    }
}
