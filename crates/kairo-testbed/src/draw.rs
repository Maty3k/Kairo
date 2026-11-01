// SPDX-License-Identifier: GPL-3.0-or-later
//! The draw plan (docs/research/16-verify-testbed/README.md §4.4): how one frame of pattern v1
//! becomes D3D11 operations, worked out without D3D so that it can be tested anywhere.
//!
//! Every element is a solid rectangle or a horizontal band whose rows repeat, so no shader and
//! no pattern maths run on the GPU (CLAUDE.md: nothing from OBS):
//! - the background is cleared black, so OLED pixels are off;
//! - each run of identical band rows ([`Scene::band_runs`]) is one row, evaluated by `testsig`
//!   at the device's pixel centres and uploaded once into a row texture. It is copied into the
//!   run's first row, then doubled: the rows done so far go into a scratch texture and back below
//!   themselves. Plain `CopySubresourceRegion` can't copy within one subresource
//!   (`CopySubresourceRegion1` can, where the driver reports `CopyWithOverlap`), so the scratch
//!   texture keeps to the D3D11.0 call;
//! - the rectangles (patches, flash, barcode cells) are `ClearView` calls, one per colour, each
//!   with its list of rectangles. They come after the bands, as in `Scene::raster_srgb`.
//!
//! In SDR the swap chain is BGRA8 and holds sRGB codes. In HDR it is FP16 scRGB, where 1.0 is
//! 80 nits: SDR elements are their sRGB-decoded value × the OS's SDR white / 80, as Windows
//! composes SDR apps, and the HDR patches are their nits / 80 (README §4.8). Colours given to
//! `ClearView` are rounded to the format first, so the GPU's conversion is exact. The 256 codes'
//! FP16 values are worked out once per frame, into a palette: an sRGB decode for every band
//! pixel would cost milliseconds.

use std::fmt;

use kairo_core::testsig::{self, Fill, Scene};

/// The rows of the row texture: the most band runs one frame can have, with room to spare
/// (bars and ramp up to 2 each where the motion pattern wraps them, the load band's 12 block
/// rows plus the one split at the wrap).
pub const ROW_TEXTURE_ROWS: u32 = 32;

/// The swap chain's pixel format.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Format {
    /// `B8G8R8A8_UNORM`: sRGB codes.
    Bgra8,
    /// `R16G16B16A16_FLOAT`: scRGB, linear, 1.0 = 80 nits; SDR elements at `sdr_white_nits`.
    Fp16 {
        /// The OS's SDR white, in nits.
        sdr_white_nits: f64,
    },
}

impl Format {
    /// Bytes per pixel.
    pub fn bytes_per_pixel(self) -> usize {
        match self {
            Format::Bgra8 => 4,
            Format::Fp16 { .. } => 8,
        }
    }

    /// The `ClearView` colour (RGBA) of `fill`, already exact in this format.
    pub fn colour(self, fill: Fill) -> Result<[f32; 4], DrawError> {
        match (self, fill) {
            (Format::Bgra8, Fill::Srgb(codes)) => {
                let [r, g, b] = codes.map(|c| f32::from(c) / 255.0);
                Ok([r, g, b, 1.0])
            }
            (Format::Bgra8, Fill::Nits(_)) => Err(DrawError::NitsInSdr),
            (Format::Fp16 { sdr_white_nits }, Fill::Srgb(codes)) => {
                let [r, g, b] = codes.map(|c| sdr_linear(c, sdr_white_nits));
                Ok([r, g, b, 1.0])
            }
            (Format::Fp16 { .. }, Fill::Nits(nits)) => {
                let v = exact_f16(nits / 80.0);
                Ok([v, v, v, 1.0])
            }
        }
    }
}

/// The pixel bytes of each sRGB code in a format.
struct Palette {
    format: Format,
    /// FP16 only: each code's half, little-endian.
    halves: [[u8; 2]; 256],
}

impl Palette {
    fn new(format: Format) -> Palette {
        let halves = match format {
            Format::Bgra8 => [[0; 2]; 256],
            Format::Fp16 { sdr_white_nits } => std::array::from_fn(|code| {
                f16_bits(sdr_linear(code as u8, sdr_white_nits)).to_le_bytes()
            }),
        };
        Palette { format, halves }
    }

    /// The bytes of a pixel of sRGB codes.
    fn pixel(&self, codes: [u8; 3]) -> [u8; 8] {
        let [r, g, b] = codes;
        match self.format {
            Format::Bgra8 => [b, g, r, 255, 0, 0, 0, 0],
            Format::Fp16 { .. } => {
                let [r, g, b] = [r, g, b].map(|c| self.halves[usize::from(c)]);
                let [a0, a1] = ONE_F16.to_le_bytes();
                [r[0], r[1], g[0], g[1], b[0], b[1], a0, a1]
            }
        }
    }
}

/// 1.0 in half precision.
const ONE_F16: u16 = 0x3C00;

/// An SDR code's scRGB value in HDR: sRGB-decoded × SDR white / 80, rounded to FP16.
fn sdr_linear(code: u8, sdr_white_nits: f64) -> f32 {
    exact_f16(testsig::srgb_decode(f64::from(code) / 255.0) * sdr_white_nits / 80.0)
}

/// `value` rounded to the nearest FP16 and back, so it converts to FP16 exactly.
fn exact_f16(value: f64) -> f32 {
    f16_to_f32(f16_bits_f64(value))
}

/// An RGBA pixel's FP16 bytes, little-endian.
fn fp16_pixel(rgba: [f32; 4]) -> [u8; 8] {
    let mut out = [0_u8; 8];
    for (bytes, c) in out.as_chunks_mut::<2>().0.iter_mut().zip(rgba) {
        *bytes = f16_bits(c).to_le_bytes();
    }
    out
}

/// IEEE 754 half precision of `x`, rounded to nearest, ties to even.
pub fn f16_bits(x: f32) -> u16 {
    let bits = x.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exponent = ((bits >> 23) & 0xFF) as i32;
    let mantissa = bits & 0x7F_FFFF;
    if exponent == 0xFF {
        // Infinity, or NaN with a quiet bit set.
        return sign | 0x7C00 | if mantissa == 0 { 0 } else { 0x200 };
    }
    let e = exponent - 127 + 15;
    if e >= 0x1F {
        return sign | 0x7C00;
    }
    if e <= 0 {
        // A subnormal half, or zero: shift the full significand into 10 bits and round.
        if e < -10 {
            return sign;
        }
        let significand = mantissa | 0x80_0000;
        let shift = (14 - e) as u32;
        let half = 1_u32 << (shift - 1);
        let rest = significand & ((1 << shift) - 1);
        let mut value = significand >> shift;
        if rest > half || (rest == half && value & 1 == 1) {
            value += 1;
        }
        return sign | value as u16;
    }
    let mut value = ((e as u32) << 10) | (mantissa >> 13);
    let rest = mantissa & 0x1FFF;
    if rest > 0x1000 || (rest == 0x1000 && value & 1 == 1) {
        // A carry into the exponent is the right result, up to infinity.
        value += 1;
    }
    sign | value as u16
}

/// IEEE 754 half precision of `x`, rounded to nearest, ties to even, straight from f64: going
/// through f32 first can round twice and miss the nearest half.
pub fn f16_bits_f64(x: f64) -> u16 {
    let bits = x.to_bits();
    let sign = ((bits >> 48) & 0x8000) as u16;
    let exponent = ((bits >> 52) & 0x7FF) as i32;
    let mantissa = bits & 0xF_FFFF_FFFF_FFFF;
    if exponent == 0x7FF {
        // Infinity, or NaN with a quiet bit set.
        return sign | 0x7C00 | if mantissa == 0 { 0 } else { 0x200 };
    }
    let e = exponent - 1_023 + 15;
    if e >= 0x1F {
        return sign | 0x7C00;
    }
    if e <= 0 {
        // A subnormal half, or zero: shift the full significand into 10 bits and round.
        if e < -10 {
            return sign;
        }
        let significand = mantissa | (1 << 52);
        let shift = (43 - e) as u32;
        let half = 1_u64 << (shift - 1);
        let rest = significand & ((1 << shift) - 1);
        let mut value = significand >> shift;
        if rest > half || (rest == half && value & 1 == 1) {
            value += 1;
        }
        return sign | value as u16;
    }
    let mut value = ((e as u64) << 10) | (mantissa >> 42);
    let rest = mantissa & ((1 << 42) - 1);
    let half = 1 << 41;
    if rest > half || (rest == half && value & 1 == 1) {
        // A carry into the exponent is the right result, up to infinity.
        value += 1;
    }
    sign | value as u16
}

/// The f32 value of half-precision `bits`.
pub fn f16_to_f32(bits: u16) -> f32 {
    let sign = if bits & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exponent = i32::from((bits >> 10) & 0x1F);
    let mantissa = f32::from(bits & 0x3FF);
    sign * match exponent {
        0 => mantissa * 2_f32.powi(-24),
        0x1F if mantissa == 0.0 => f32::INFINITY,
        0x1F => f32::NAN,
        e => (1.0 + mantissa / 1_024.0) * 2_f32.powi(e - 15),
    }
}

/// Why a frame can't be planned.
#[derive(Debug, Clone, PartialEq)]
pub enum DrawError {
    /// A patch in nits on an SDR swap chain.
    NitsInSdr,
    /// More band runs than the row texture holds.
    TooManyRows(usize),
    /// A run longer than the scratch texture.
    RunTooLong(u32),
}

impl fmt::Display for DrawError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DrawError::NitsInSdr => write!(f, "an HDR patch on an SDR swap chain"),
            DrawError::TooManyRows(n) => {
                write!(f, "{n} band rows; the row texture holds {ROW_TEXTURE_ROWS}")
            }
            DrawError::RunTooLong(n) => write!(f, "a band run of {n} rows is too long"),
        }
    }
}

impl std::error::Error for DrawError {}

/// A rectangle in pixels, half-open, as D3D11 takes one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PixelRect {
    /// Left edge.
    pub left: i32,
    /// Top edge.
    pub top: i32,
    /// Right edge (exclusive).
    pub right: i32,
    /// Bottom edge (exclusive).
    pub bottom: i32,
}

/// One `ClearView`: a colour and every rectangle that has it.
#[derive(Debug, Clone, PartialEq)]
pub struct Clear {
    /// RGBA, exact in the swap chain's format.
    pub colour: [f32; 4],
    /// The rectangles.
    pub rects: Vec<PixelRect>,
}

/// One `CopySubresourceRegion`, a whole width of rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Copy {
    /// Row `row` of the row texture into the target's row `y`.
    RowToTarget {
        /// The row texture's row.
        row: u32,
        /// The target's row.
        y: u32,
    },
    /// The target's rows `y..y + rows` into the scratch texture's rows `0..rows`.
    TargetToScratch {
        /// The target's first row.
        y: u32,
        /// How many.
        rows: u32,
    },
    /// The scratch texture's rows `0..rows` into the target's rows `y..y + rows`.
    ScratchToTarget {
        /// The target's first row.
        y: u32,
        /// How many.
        rows: u32,
    },
}

/// One frame's operations, in order: clear the target black, upload `rows` into the row
/// texture's first `row_count` rows, make the `copies`, then the `clears`.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    /// The distinct band rows, `row_count` × width pixels in the swap chain's format.
    pub rows: Vec<u8>,
    /// How many.
    pub row_count: u32,
    /// The band copies.
    pub copies: Vec<Copy>,
    /// The rectangles, by colour.
    pub clears: Vec<Clear>,
}

impl Plan {
    /// Nothing but the black background: the warm-up frames before frame 0.
    pub fn black() -> Plan {
        Plan {
            rows: Vec::new(),
            row_count: 0,
            copies: Vec::new(),
            clears: Vec::new(),
        }
    }
}

/// The scratch texture's height for a client `height` pixels high: the tallest band, the bars,
/// since a run is at most one band.
pub fn scratch_rows(height: u32) -> u32 {
    let (top, bottom) = testsig::BARS_U;
    let rows = testsig::px_y(bottom, height) - testsig::px_y(top, height);
    u32::try_from(rows.max(1)).unwrap_or(1)
}

/// The plan for `scene` on a swap chain of `format`.
pub fn plan(scene: &Scene, format: Format) -> Result<Plan, DrawError> {
    let size = scene.size();
    let bpp = format.bytes_per_pixel();
    let scratch = scratch_rows(size.height());
    let palette = Palette::new(format);
    // A frame has 14 or 15 distinct band rows.
    let mut rows = Vec::with_capacity(16 * size.width() as usize * bpp);
    let mut row_count = 0_u32;
    let mut copies = Vec::new();
    for band in scene.bands() {
        for (y0, y1) in scene.band_runs(band) {
            if row_count == ROW_TEXTURE_ROWS {
                return Err(DrawError::TooManyRows(row_count as usize + 1));
            }
            for codes in scene.band_row(band, y0) {
                rows.extend_from_slice(&palette.pixel(codes)[..bpp]);
            }
            let (y0, n) = (y0 as u32, (y1 - y0) as u32);
            if n.div_ceil(2) > scratch {
                return Err(DrawError::RunTooLong(n));
            }
            copies.push(Copy::RowToTarget {
                row: row_count,
                y: y0,
            });
            let mut done = 1;
            while done < n {
                let more = done.min(n - done);
                copies.push(Copy::TargetToScratch { y: y0, rows: more });
                copies.push(Copy::ScratchToTarget {
                    y: y0 + done,
                    rows: more,
                });
                done += more;
            }
            row_count += 1;
        }
    }
    let mut clears: Vec<Clear> = Vec::new();
    for rect in scene.rects() {
        let colour = format.colour(rect.fill)?;
        let pixels = PixelRect {
            left: rect.x0 as i32,
            top: rect.y0 as i32,
            right: rect.x1 as i32,
            bottom: rect.y1 as i32,
        };
        match clears
            .iter_mut()
            .find(|c| c.colour.map(f32::to_bits) == colour.map(f32::to_bits))
        {
            Some(clear) => clear.rects.push(pixels),
            None => clears.push(Clear {
                colour,
                rects: vec![pixels],
            }),
        }
    }
    Ok(Plan {
        rows,
        row_count,
        copies,
        clears,
    })
}

/// The bytes of a `ClearView` colour in `format`, as the GPU writes them.
pub fn clear_bytes(format: Format, colour: [f32; 4]) -> [u8; 8] {
    match format {
        Format::Bgra8 => {
            let [r, g, b, a] = colour.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8);
            [b, g, r, a, 0, 0, 0, 0]
        }
        Format::Fp16 { .. } => fp16_pixel(colour),
    }
}

/// Carries out `plan` on a `width` × `height` frame in memory, as the GPU would: the reference
/// the WARP test and this module's tests compare against.
pub fn execute(plan: &Plan, format: Format, width: u32, height: u32) -> Vec<u8> {
    let bpp = format.bytes_per_pixel();
    let row_bytes = width as usize * bpp;
    let black = clear_bytes(format, [0.0, 0.0, 0.0, 1.0]);
    let mut target: Vec<u8> = (0..width as usize * height as usize)
        .flat_map(|_| black[..bpp].to_vec())
        .collect();
    let mut scratch = vec![0_u8; scratch_rows(height) as usize * row_bytes];
    let rows = |r: u32, n: u32| r as usize * row_bytes..(r + n) as usize * row_bytes;
    for copy in &plan.copies {
        match *copy {
            Copy::RowToTarget { row, y } => {
                target[rows(y, 1)].copy_from_slice(&plan.rows[rows(row, 1)]);
            }
            Copy::TargetToScratch { y, rows: n } => {
                scratch[rows(0, n)].copy_from_slice(&target[rows(y, n)]);
            }
            Copy::ScratchToTarget { y, rows: n } => {
                target[rows(y, n)].copy_from_slice(&scratch[rows(0, n)]);
            }
        }
    }
    for clear in &plan.clears {
        let pixel = clear_bytes(format, clear.colour);
        for r in &clear.rects {
            for y in r.top..r.bottom {
                for x in r.left..r.right {
                    let at = (y as usize * width as usize + x as usize) * bpp;
                    target[at..at + bpp].copy_from_slice(&pixel[..bpp]);
                }
            }
        }
    }
    target
}

/// `scene` in `format`, pixel by pixel straight from `testsig` (every band row evaluated, every
/// rectangle filled in order): what [`execute`] must give for [`plan`]'s output.
pub fn reference(scene: &Scene, format: Format) -> Result<Vec<u8>, DrawError> {
    let size = scene.size();
    let (w, h) = (size.width() as usize, size.height() as usize);
    let bpp = format.bytes_per_pixel();
    let black = clear_bytes(format, [0.0, 0.0, 0.0, 1.0]);
    let mut out: Vec<u8> = (0..w * h).flat_map(|_| black[..bpp].to_vec()).collect();
    for band in scene.bands() {
        for y in band.y0..band.y1 {
            for (x, codes) in scene.band_row(band, y).into_iter().enumerate() {
                let at = (y as usize * w + x) * bpp;
                let pixel = pixel_by_hand(format, codes);
                out[at..at + bpp].copy_from_slice(&pixel[..bpp]);
            }
        }
    }
    for rect in scene.rects() {
        let pixel = clear_bytes(format, format.colour(rect.fill)?);
        for y in rect.y0..rect.y1 {
            for x in rect.x0..rect.x1 {
                let at = (y as usize * w + x as usize) * bpp;
                out[at..at + bpp].copy_from_slice(&pixel[..bpp]);
            }
        }
    }
    Ok(out)
}

/// A pixel's bytes worked out on its own, without the palette: the reference's way.
fn pixel_by_hand(format: Format, codes: [u8; 3]) -> [u8; 8] {
    match format {
        Format::Bgra8 => {
            let [r, g, b] = codes;
            [b, g, r, 255, 0, 0, 0, 0]
        }
        Format::Fp16 { sdr_white_nits } => {
            let [r, g, b] = codes.map(|c| sdr_linear(c, sdr_white_nits));
            fp16_pixel([r, g, b, 1.0])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kairo_core::testsig::{Load, Options, Patches, Pattern, Size};

    fn options(pattern: Pattern, load: Load, patches: Patches) -> Options {
        Options {
            pattern,
            nominal_fps: 180,
            run_tag: 93,
            load,
            patches,
        }
    }

    #[test]
    fn halves_round_to_nearest_even() {
        assert_eq!(f16_bits(0.0), 0);
        assert_eq!(f16_bits(-0.0), 0x8000);
        assert_eq!(f16_bits(1.0), 0x3C00);
        assert_eq!(f16_bits(0.5), 0x3800);
        assert_eq!(f16_bits(65_504.0), 0x7BFF);
        assert_eq!(f16_bits(65_520.0), 0x7C00);
        assert_eq!(f16_bits(f32::INFINITY), 0x7C00);
        // The smallest subnormal and the largest one.
        assert_eq!(f16_bits(2_f32.powi(-24)), 1);
        assert_eq!(f16_bits(2_f32.powi(-14) - 2_f32.powi(-24)), 0x3FF);
        assert_eq!(f16_bits(2_f32.powi(-26)), 0);
        // Ties go to even: 1 + 2⁻¹¹ lies halfway between 1 and 1 + 2⁻¹⁰.
        assert_eq!(f16_bits(1.0 + 2_f32.powi(-11)), 0x3C00);
        assert_eq!(f16_bits(1.0 + 3.0 * 2_f32.powi(-11)), 0x3C02);
        // Every half converts back and forth exactly.
        for bits in (0..0x7C00_u16).step_by(7) {
            assert_eq!(f16_bits(f16_to_f32(bits)), bits, "{bits:#x}");
        }
    }

    #[test]
    fn halves_round_straight_from_f64() {
        // Exact in f32 too: both roundings agree.
        let mut x = 1.0e-9_f32;
        while x < 70_000.0 {
            for v in [x, -x, x * 1.000_7] {
                assert_eq!(f16_bits_f64(f64::from(v)), f16_bits(v), "{v}");
            }
            x *= 1.001_3;
        }
        assert_eq!(f16_bits_f64(f64::INFINITY), 0x7C00);
        assert_eq!(f16_bits_f64(f64::NAN) & 0x7E00, 0x7E00);
        assert_eq!(f16_bits_f64(2_f64.powi(-25)), 0);
        assert_eq!(f16_bits_f64(2_f64.powi(-25) * 1.000_001), 1);
        // Just above the midpoint between an even half and the odd one after it, by less than
        // half an f32 step: through f32 it lands on the midpoint and ties to the even one,
        // below; straight from f64 it goes up, to the nearest.
        let low = f64::from(f16_to_f32(0x2A02));
        let high = f64::from(f16_to_f32(0x2A03));
        let x = (low + high) / 2.0 + 1.0e-12;
        assert_eq!(f16_bits(x as f32), 0x2A02);
        assert_eq!(f16_bits_f64(x), 0x2A03);
        // Against the halves on either side: never farther than a neighbour, and even on a tie.
        let mut seed = 99_u64;
        for _ in 0..200_000 {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let x = 2_f64.powf((seed >> 11) as f64 / (1_u64 << 53) as f64 * 42.0 - 26.0);
            if x > 65_504.0 {
                continue;
            }
            let h = f16_bits_f64(x);
            let error = |bits: u16| (f64::from(f16_to_f32(bits)) - x).abs();
            for other in [h.saturating_sub(1), h + 1] {
                assert!(
                    error(h) < error(other) || (error(h) == error(other) && h & 1 == 0),
                    "{x}: {h:#x}"
                );
            }
        }
        // The palette's halves are the pixels worked out one by one.
        for white in [80.0, 100.24, 203.0, 240.0, 480.0] {
            let format = Format::Fp16 {
                sdr_white_nits: white,
            };
            let palette = Palette::new(format);
            for c in 0..=255_u8 {
                let codes = [c, 255 - c, c / 3];
                assert_eq!(
                    palette.pixel(codes),
                    pixel_by_hand(format, codes),
                    "{white} {c}"
                );
            }
        }
    }

    #[test]
    fn sdr_plans_draw_the_reference_raster() {
        let cases = [
            ((1_920, 1_080), 0, Pattern::Full, Load::Scroll),
            ((1_920, 1_080), 181, Pattern::Full, Load::Scroll),
            ((2_560, 1_440), 5_000, Pattern::Full, Load::Churn),
            ((1_280, 720), 179, Pattern::Motion, Load::Scroll),
            ((1_280, 720), 1_079, Pattern::Motion, Load::None),
            ((641, 361), 33, Pattern::Full, Load::Scroll),
        ];
        for ((w, h), frame, pattern, load) in cases {
            let size = Size::new(w, h).unwrap();
            let scene = Scene::new(size, frame, &options(pattern, load, Patches::Sdr)).unwrap();
            let plan = plan(&scene, Format::Bgra8).unwrap();
            let drawn = execute(&plan, Format::Bgra8, w, h);
            let want: Vec<u8> = scene
                .raster_srgb()
                .unwrap()
                .into_iter()
                .flat_map(|[r, g, b]| [b, g, r, 255])
                .collect();
            assert!(drawn == want, "{w}x{h} frame {frame} {pattern:?}");
            assert_eq!(reference(&scene, Format::Bgra8).unwrap(), want);
            // Few operations: a ClearView per colour and a few copies per run.
            assert!(plan.clears.len() <= 16, "{}", plan.clears.len());
            assert!(plan.row_count <= ROW_TEXTURE_ROWS);
        }
    }

    #[test]
    fn hdr_plans_draw_the_reference_raster() {
        let patches = Patches::Hdr {
            sdr_white_nits: 240.0,
            bright: false,
        };
        let format = Format::Fp16 {
            sdr_white_nits: 240.0,
        };
        for (pattern, frame) in [(Pattern::Full, 7), (Pattern::Motion, 600)] {
            let size = Size::new(1_920, 1_080).unwrap();
            let scene = Scene::new(size, frame, &options(pattern, Load::Scroll, patches)).unwrap();
            let plan = plan(&scene, format).unwrap();
            let drawn = execute(&plan, format, 1_920, 1_080);
            assert!(drawn == reference(&scene, format).unwrap(), "{pattern:?}");
        }
        // SDR white comes out at W / 80, and a 1,000-nit patch at 12.5.
        assert_eq!(
            format.colour(Fill::Srgb([255; 3])).unwrap(),
            [3.0, 3.0, 3.0, 1.0]
        );
        assert_eq!(format.colour(Fill::Nits(1_000.0)).unwrap()[0], 12.5);
        assert_eq!(
            Format::Bgra8.colour(Fill::Nits(80.0)),
            Err(DrawError::NitsInSdr)
        );
    }

    #[test]
    fn runs_double_through_the_scratch_texture() {
        let size = Size::new(1_920, 1_080).unwrap();
        let scene =
            Scene::new(size, 0, &options(Pattern::Full, Load::Scroll, Patches::Sdr)).unwrap();
        let plan = plan(&scene, Format::Bgra8).unwrap();
        // The bars' 192 rows: the first row, then 1, 2, 4 … 64 rows each way.
        let bars: Vec<&Copy> = plan
            .copies
            .iter()
            .take_while(|c| !matches!(c, Copy::RowToTarget { row: 1, .. }))
            .collect();
        assert_eq!(bars.len(), 1 + 2 * 8);
        assert_eq!(bars[0], &Copy::RowToTarget { row: 0, y: 160 });
        assert_eq!(bars[1], &Copy::TargetToScratch { y: 160, rows: 1 });
        assert_eq!(bars[2], &Copy::ScratchToTarget { y: 161, rows: 1 });
        assert_eq!(bars[16], &Copy::ScratchToTarget { y: 288, rows: 64 });
        // Bars, ramp and the load texture's 12 block rows.
        assert_eq!(plan.row_count, 14);
        assert_eq!(scratch_rows(1_080), 192);
        assert_eq!(scratch_rows(1_440), 256);
    }
}
