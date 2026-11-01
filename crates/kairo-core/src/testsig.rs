// SPDX-License-Identifier: GPL-3.0-or-later
//! Pattern v1 (docs/research/16-verify-testbed/README.md §4.5–4.8): what the testbed draws and
//! plays, defined so that the clip generator renders the same frames and `kairo-verify` can
//! regenerate any frame and any tone from the frame number alone.
//!
//! Geometry is in layout units: 1 u is the client height / 1080, measured from the client's top
//! and, for centred elements, from its horizontal centre. Every u → pixel conversion rounds half
//! away from zero in exact integers ([`px_y`], [`px_centred`]), so the testbed, the generator and
//! the verifier place every edge on the same pixel. The bands (bars, ramp, load) are functions
//! of x alone (and of the row, for the load band), evaluated at pixel centres.
//!
//! Colours are sRGB code values, what an SDR swap chain stores; the published BT.709 matrix
//! turns them into Y′CbCr ([`ycbcr`]), and [`subsample_chroma`] makes 4:2:0 with
//! `chroma_sample_loc_type` 0 (H.273). HDR expectations use only the published SMPTE ST 2084
//! (PQ), BT.2390 (EETF) and IEC 61966-2-1 (sRGB) formulas. Nothing here comes from OBS.

use std::fmt;

/// The pattern's version, drawn into nothing but checked by every barcode's CRC.
pub const PATTERN_VERSION: u8 = 1;
/// Layout units per client height.
pub const UNITS_PER_HEIGHT: i64 = 1_080;

/// Barcode cell size, in u.
pub const CELL_U: i64 = 24;
/// Barcode columns.
pub const CODE_COLS: usize = 16;
/// Barcode rows: one of references, three of data.
pub const CODE_ROWS: usize = 4;
/// Top of barcode copy A, in u.
pub const CODE_A_TOP_U: i64 = 32;
/// Top of barcode copy B, in u (its bottom is at 1,048 u).
pub const CODE_B_TOP_U: i64 = 952;
/// The bars band, in u.
pub const BARS_U: (i64, i64) = (160, 352);
/// The ramp band, in u.
pub const RAMP_U: (i64, i64) = (352, 416);
/// Top of the flash square, in u.
pub const FLASH_TOP_U: i64 = 448;
/// Side of the flash square, in u.
pub const FLASH_SIZE_U: i64 = 192;
/// Top of the patch row, in u.
pub const PATCHES_TOP_U: i64 = 672;
/// Side of a patch, in u.
pub const PATCH_SIZE_U: i64 = 128;
/// Distance between the left edges of neighbouring patches, in u.
pub const PATCH_PITCH_U: i64 = 160;
/// The load band, in u.
pub const LOAD_U: (i64, i64) = (832, 928);

/// Width of one bar, in u.
pub const BAR_WIDTH_U: i64 = 160;
/// Width of the raised-cosine blend centred on each bar boundary, in u.
pub const BAR_EDGE_U: i64 = 32;
/// How far the bars move right each frame, in u.
pub const BARS_SCROLL_U: i64 = 3;
/// Period of the grey ramp's triangle, in u.
pub const RAMP_PERIOD_U: i64 = 1_024;
/// How far the ramp moves left each frame, in u.
pub const RAMP_SCROLL_U: i64 = 2;
/// How far the patch row moves right each frame, in u.
pub const PATCH_SCROLL_U: i64 = 1;
/// Side of a load block, in u.
pub const LOAD_BLOCK_U: i64 = 8;
/// Load blocks per period of the scrolling texture.
pub const LOAD_PERIOD_BLOCKS: i64 = 512;
/// How far the load texture moves right each frame, in u.
pub const LOAD_SCROLL_U: i64 = 5;
/// The load texture's seed.
pub const LOAD_SEED: u64 = 0x4B41_4952_4F00_0001;
/// How far the motion pattern moves down each frame, in u.
pub const MOTION_SCROLL_U: i64 = 1;

/// The eight bars, as sRGB codes: SMPTE's order at 75% (code 191), then black.
pub const BARS: [[u8; 3]; 8] = [
    [191, 191, 191],
    [191, 191, 0],
    [0, 191, 191],
    [0, 191, 0],
    [191, 0, 191],
    [191, 0, 0],
    [0, 0, 191],
    [0, 0, 0],
];
/// The SDR patches' grey sRGB codes, left to right.
pub const SDR_PATCH_CODES: [u8; 12] = [0, 8, 16, 32, 64, 96, 128, 160, 191, 224, 240, 255];
/// The HDR patches' grey levels in nits, left to right; patch 7 shows the OS's SDR white.
pub const HDR_PATCH_NITS: [f64; 12] = [
    0.0, 10.0, 40.0, 80.0, 120.0, 160.0, 203.0, 0.0, 400.0, 600.0, 1_000.0, 4_000.0,
];
/// The patch that shows the OS's SDR white in HDR mode.
pub const SDR_WHITE_PATCH: usize = 7;

/// Peak level of every tone, in dBFS (CLAUDE.md: tones at −30 dBFS).
pub const BEEP_PEAK_DBFS: f64 = -30.0;
/// Length of a beep, in ms.
pub const BEEP_MS: u32 = 100;
/// Length of each raised-cosine ramp at a tone's ends, in ms.
pub const BEEP_RAMP_MS: f64 = 2.0;
/// Where in each second the placeholder cue starts in synthetic clips, in ms: 100 ms after the
/// voice beep ends, and ending 80 ms before the next second's game beep, already playing when
/// the search for that beep opens 250 ms before it.
pub const CUE_OFFSET_MS: u32 = 700;

/// Why a frame or a tone could not be made.
#[derive(Debug, Clone, PartialEq)]
pub enum PatternError {
    /// The client must be 1 × 1 to [`MAX_SIDE`] × [`MAX_SIDE`] pixels.
    Size {
        /// The width asked for.
        width: u32,
        /// The height asked for.
        height: u32,
    },
    /// The nominal frame rate must be at least 1.
    Fps(u32),
    /// The run tag has 7 bits.
    Tag(u8),
    /// A light level must be a finite number of nits above zero where it divides.
    Nits(f64),
    /// The CPU raster draws SDR codes only; this scene has HDR patches in nits.
    NotSdr,
    /// The sample rate must be at least 1 Hz.
    SampleRate(u32),
    /// A plane must hold exactly width × height samples.
    Plane {
        /// The samples it holds.
        len: usize,
        /// The width given.
        width: usize,
        /// The height given.
        height: usize,
    },
}

impl fmt::Display for PatternError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Size { width, height } => {
                write!(
                    f,
                    "the client must be 1x1 to {MAX_SIDE}x{MAX_SIDE} pixels, not {width}x{height}"
                )
            }
            Self::Fps(fps) => write!(f, "the nominal frame rate must be at least 1, not {fps}"),
            Self::Tag(tag) => write!(f, "the run tag must be 0 to 127, not {tag}"),
            Self::Nits(nits) => write!(f, "a light level must be finite and positive, not {nits}"),
            Self::NotSdr => write!(f, "the CPU raster draws SDR codes only, not HDR patches"),
            Self::SampleRate(rate) => write!(f, "the sample rate must be at least 1, not {rate}"),
            Self::Plane { len, width, height } => {
                write!(f, "a {width}x{height} plane holds {len} samples")
            }
        }
    }
}

impl std::error::Error for PatternError {}

// ---------------------------------------------------------------------------------------------
// Pixels
// ---------------------------------------------------------------------------------------------

/// The largest client side: D3D11's largest texture side, which no testbed client or clip
/// exceeds. It bounds a CPU raster at 768 MiB.
pub const MAX_SIDE: u32 = 16_384;

/// The client area a frame is drawn for, in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Size {
    width: u32,
    height: u32,
}

impl Size {
    /// A client of `width` × `height` pixels, each 1 to [`MAX_SIDE`].
    pub fn new(width: u32, height: u32) -> Result<Self, PatternError> {
        let side = 1..=MAX_SIDE;
        if !side.contains(&width) || !side.contains(&height) {
            return Err(PatternError::Size { width, height });
        }
        Ok(Self { width, height })
    }

    /// Width in pixels.
    pub const fn width(self) -> u32 {
        self.width
    }

    /// Height in pixels.
    pub const fn height(self) -> u32 {
        self.height
    }
}

/// `num / den` rounded half away from zero; `den` must be positive.
fn round_half_away(num: i128, den: i128) -> i64 {
    let rounded = if num >= 0 {
        (2 * num + den).div_euclid(2 * den)
    } else {
        -(2 * -num + den).div_euclid(2 * den)
    };
    i64::try_from(rounded).unwrap_or(if rounded < 0 { i64::MIN } else { i64::MAX })
}

/// The pixel row (or column, from the left) of `y_u` layout units.
pub fn px_y(y_u: i64, height: u32) -> i64 {
    round_half_away(i128::from(y_u) * i128::from(height), 1_080)
}

/// The pixel column of the point `offset_u` layout units right of the client's centre.
pub fn px_centred(offset_u: i64, size: Size) -> i64 {
    round_half_away(
        540 * i128::from(size.width) + i128::from(offset_u) * i128::from(size.height),
        1_080,
    )
}

/// Where the centre of pixel column (or row) `px` lies, in layout units.
pub fn unit_of_pixel(px: i64, height: u32) -> f64 {
    (px as f64 + 0.5) * 1_080.0 / f64::from(height)
}

// ---------------------------------------------------------------------------------------------
// Barcode
// ---------------------------------------------------------------------------------------------

/// Which of the two identical barcodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeCopy {
    /// The top one.
    A,
    /// The bottom one.
    B,
}

impl CodeCopy {
    /// Its top edge, in u.
    pub const fn top_u(self) -> i64 {
        match self {
            Self::A => CODE_A_TOP_U,
            Self::B => CODE_B_TOP_U,
        }
    }
}

/// What a barcode says.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Barcode {
    /// The frame number modulo 2²⁴.
    pub frame24: u32,
    /// Whether the testbed showed HDR patches.
    pub hdr: bool,
    /// The run's tag, 0 to 127.
    pub tag: u8,
    /// How clearly the data cells sat on their side of the threshold: the smallest distance
    /// from it, over half the black-to-white span. 1 is perfect.
    pub margin: f64,
    /// The white reference level: the median of the four white reference cells.
    pub white: f64,
    /// The black reference level: the median of the four black reference cells.
    pub black: f64,
}

impl Barcode {
    /// Halfway between the reference levels: the threshold the cells were read against, and
    /// the level `content.flash` compares the flash with (README §6.3).
    pub fn midpoint(&self) -> f64 {
        (self.white + self.black) / 2.0
    }
}

/// Why a barcode could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarcodeError {
    /// White and black references less than 110 codes apart.
    LowContrast,
    /// A reference cell on the wrong side of the threshold.
    References,
    /// The CRC did not match.
    Crc,
    /// The frame number's parity disagreed with the reference row's phase.
    Parity,
}

impl fmt::Display for BarcodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LowContrast => write!(f, "the barcode's references are too close together"),
            Self::References => write!(f, "a barcode reference cell is on the wrong side"),
            Self::Crc => write!(f, "the barcode's CRC does not match"),
            Self::Parity => write!(
                f,
                "the barcode's frame parity disagrees with its references"
            ),
        }
    }
}

impl std::error::Error for BarcodeError {}

/// The smallest white-minus-black reference span a readable barcode has, in luma codes: half
/// the nominal 219.
pub const MIN_CONTRAST: f64 = 110.0;

const FRAME_MASK: u32 = 0x00FF_FFFF;
const MASK40: u64 = (1 << 40) - 1;

/// SplitMix64's output function for one input.
fn splitmix64(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// CRC-16/CCITT-FALSE: polynomial 0x1021, initial value 0xFFFF, no reflection, no final XOR.
fn crc16(bytes: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for &byte in bytes {
        crc ^= u16::from(byte) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}

/// The whitening mask for bits 8–47, from the frame number's low byte.
fn mask40(low: u8) -> u64 {
    (splitmix64(0x4B01_0000_0000_0000 | u64::from(low)) >> 24) & MASK40
}

/// The CRC over the format prefix ("K", version) and the payload.
fn code_crc(frame24: u32, hdr: bool, tag: u8) -> u16 {
    let [_, high, mid, low] = frame24.to_be_bytes();
    crc16(&[
        0x4B,
        PATTERN_VERSION,
        high,
        mid,
        low,
        (u8::from(hdr) << 7) | (tag & 0x7F),
    ])
}

/// The barcode for frame `frame` (taken modulo 2²⁴): row 0 holds the references, white where
/// the column plus the frame number is even; rows 1–3 hold 48 bits, MSB first, row by row, 1
/// is white. Bits 0–7 are the frame number's low byte; bits 8–47 are its high 16 bits, the HDR
/// flag, the 7-bit tag and the CRC, XORed with a mask chosen by the low byte, so no cell keeps
/// a level for long (OLED care).
pub fn barcode_cells(frame: u64, hdr: bool, tag: u8) -> [[bool; CODE_COLS]; CODE_ROWS] {
    let frame24 = (frame & u64::from(FRAME_MASK)) as u32;
    let low = (frame24 & 0xFF) as u8;
    let tag = tag & 0x7F;
    let word40 = (u64::from(frame24 >> 8) << 24)
        | (u64::from(hdr) << 23)
        | (u64::from(tag) << 16)
        | u64::from(code_crc(frame24, hdr, tag));
    let word48 = (u64::from(low) << 40) | (word40 ^ mask40(low));
    let mut cells = [[false; CODE_COLS]; CODE_ROWS];
    for (col, cell) in cells[0].iter_mut().enumerate() {
        // Even sum: the column and the frame number have the same parity.
        *cell = (col as u64 ^ frame) & 1 == 0;
    }
    for (bit, cell) in cells[1..].iter_mut().flatten().enumerate() {
        *cell = (word48 >> (47 - bit)) & 1 == 1;
    }
    cells
}

/// The median of eight values.
fn median8(mut values: [f64; 8]) -> f64 {
    values.sort_by(f64::total_cmp);
    (values[3] + values[4]) / 2.0
}

/// Reads a barcode from its cells' mean luma, row by row as [`barcode_cells`] draws them.
///
/// The CRC catches every one-cell error but not every two-cell one (the low byte that whitens
/// the rest isn't covered), so a caller compares [`Barcode::tag`] with the run's tag, as
/// kairo-verify does on every frame (README §6.3).
pub fn read_barcode(means: &[[f64; CODE_COLS]; CODE_ROWS]) -> Result<Barcode, BarcodeError> {
    let refs = &means[0];
    let (mut even, mut odd) = ([0.0; 8], [0.0; 8]);
    for (i, [e, o]) in refs.as_chunks::<2>().0.iter().enumerate() {
        even[i] = *e;
        odd[i] = *o;
    }
    // Phase 0: the even columns are white, so the frame number is even.
    let phase = if even.iter().sum::<f64>() > odd.iter().sum::<f64>() {
        0
    } else {
        1
    };
    let (whites, blacks) = if phase == 0 { (even, odd) } else { (odd, even) };
    let (white, black) = (median8(whites), median8(blacks));
    let span = white - black;
    if span.is_nan() || span < MIN_CONTRAST {
        return Err(BarcodeError::LowContrast);
    }
    let threshold = (white + black) / 2.0;
    for (col, &mean) in refs.iter().enumerate() {
        if (mean > threshold) != ((col + phase) % 2 == 0) {
            return Err(BarcodeError::References);
        }
    }
    let mut word48 = 0_u64;
    let mut nearest = f64::INFINITY;
    for &mean in means[1..].iter().flatten() {
        word48 = (word48 << 1) | u64::from(mean > threshold);
        nearest = nearest.min((mean - threshold).abs());
    }
    let low = (word48 >> 40) as u8;
    let word40 = (word48 & MASK40) ^ mask40(low);
    let frame24 = (((word40 >> 24) as u32) << 8) | u32::from(low);
    let hdr = (word40 >> 23) & 1 == 1;
    let tag = ((word40 >> 16) & 0x7F) as u8;
    if code_crc(frame24, hdr, tag) != (word40 & 0xFFFF) as u16 {
        return Err(BarcodeError::Crc);
    }
    if frame24 % 2 != phase as u32 {
        return Err(BarcodeError::Parity);
    }
    Ok(Barcode {
        frame24,
        hdr,
        tag,
        margin: nearest / ((white - black) / 2.0),
        white,
        black,
    })
}

/// The full frame number a 24-bit barcode names, taking the one nearest `previous` (within
/// ±2²³ frames, which is 6.5 hours at 360 fps).
pub fn unwrap_frame(previous: i64, frame24: u32) -> i64 {
    let span = 1_i64 << 24;
    let delta = (i64::from(frame24 & FRAME_MASK) - previous.rem_euclid(span) + span / 2)
        .rem_euclid(span)
        - span / 2;
    previous.saturating_add(delta)
}

// ---------------------------------------------------------------------------------------------
// Elements
// ---------------------------------------------------------------------------------------------

/// Whether the flash square is lit on frame `frame` at nominal rate `fps`: for the first
/// tenth of every nominal second, `round(0.1 × fps)` frames.
pub fn flash_on(frame: u64, fps: u32) -> bool {
    let fps = u64::from(fps.max(1));
    frame % fps < (fps + 5) / 10
}

/// The colour of the moving bars at `x_u` on frame `frame`, as sRGB codes (fractional across
/// a blend). The pattern repeats every 1,280 u and moves right 3 u a frame; each boundary blends
/// its two bars over 32 u with a raised cosine.
pub fn bars_codes(x_u: f64, frame: u64) -> [f64; 3] {
    let period = (BAR_WIDTH_U * BARS.len() as i64) as f64;
    let shift = ((frame % 1_280) * BARS_SCROLL_U as u64 % 1_280) as f64;
    let p = (x_u - shift).rem_euclid(period);
    let half = (BAR_EDGE_U / 2) as f64;
    let width = BAR_WIDTH_U as f64;
    // The nearest boundary at or after p − 16 u, as a bar index (8 is the period's end).
    let boundary = ((p + half) / width).floor();
    let blend_from = boundary * width - half;
    let index = |i: f64| (i.rem_euclid(BARS.len() as f64)) as usize;
    if p >= blend_from && p < blend_from + 2.0 * half {
        let t = (p - blend_from) / (2.0 * half);
        let w = 0.5 - 0.5 * (std::f64::consts::PI * t).cos();
        let (a, b) = (BARS[index(boundary - 1.0)], BARS[index(boundary)]);
        return [0, 1, 2].map(|c| (1.0 - w) * f64::from(a[c]) + w * f64::from(b[c]));
    }
    BARS[index((p / width).floor())].map(f64::from)
}

/// The grey ramp's sRGB code at `x_u` on frame `frame`: a triangle from 0 up to 255 and back
/// every 1,024 u, moving left 2 u a frame.
pub fn ramp_code(x_u: f64, frame: u64) -> f64 {
    let shift = ((frame % 1_024) * RAMP_SCROLL_U as u64 % 1_024) as f64;
    let v = (x_u + shift).rem_euclid(RAMP_PERIOD_U as f64) / RAMP_PERIOD_U as f64;
    255.0 * if v < 0.5 { 2.0 * v } else { 2.0 - 2.0 * v }
}

/// What the load band shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Load {
    /// Seeded random 8 u blocks moving right 5 u a frame (the default).
    Scroll,
    /// New random blocks every frame (stress runs only).
    Churn,
    /// Nothing: background.
    None,
}

/// The load band's sRGB codes at `x_u`, `y_u` layout units below the band's top, on frame
/// `frame`.
pub fn load_codes(x_u: f64, y_u_in_band: f64, frame: u64, load: Load) -> [u8; 3] {
    let key = match load {
        Load::None => return [0, 0, 0],
        Load::Scroll | Load::Churn => {
            let period_u = LOAD_PERIOD_BLOCKS * LOAD_BLOCK_U;
            let shift = ((frame % period_u as u64) * LOAD_SCROLL_U as u64 % period_u as u64) as f64;
            let col = ((x_u - shift) / LOAD_BLOCK_U as f64)
                .floor()
                .rem_euclid(LOAD_PERIOD_BLOCKS as f64) as u64;
            let rows = (LOAD_U.1 - LOAD_U.0) / LOAD_BLOCK_U;
            let row =
                ((y_u_in_band / LOAD_BLOCK_U as f64).floor() as i64).clamp(0, rows - 1) as u64;
            let key = (row << 16) | col;
            if load == Load::Churn {
                key ^ (frame << 32)
            } else {
                key
            }
        }
    };
    let z = splitmix64(LOAD_SEED ^ key);
    [z as u8, (z >> 8) as u8, (z >> 16) as u8]
}

// ---------------------------------------------------------------------------------------------
// Scenes
// ---------------------------------------------------------------------------------------------

/// Which scene the testbed draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pattern {
    /// Everything: barcodes, bars, ramp, flash, patches and load.
    Full,
    /// For the OLED: no barcodes and no flash, and the whole scene moves down 1 u a frame,
    /// wrapping, so no row keeps a band.
    Motion,
}

/// What the patch row shows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Patches {
    /// SDR grey codes ([`SDR_PATCH_CODES`]).
    Sdr,
    /// HDR grey levels in nits ([`HDR_PATCH_NITS`]), with the OS's SDR white as patch 7. The
    /// motion pattern caps them at SDR white unless `bright` is set.
    Hdr {
        /// The OS's SDR white, in nits.
        sdr_white_nits: f64,
        /// Show the levels above SDR white in the motion pattern too.
        bright: bool,
    },
    /// The SDR codes an HDR source should come out at after `curve`, for synthetic clips.
    ExpectedSdr {
        /// The tone-mapping curve.
        curve: Curve,
        /// The display's peak, in nits.
        peak_nits: f64,
        /// The OS's SDR white, in nits.
        sdr_white_nits: f64,
    },
}

/// How a frame is drawn.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Options {
    /// The scene.
    pub pattern: Pattern,
    /// The nominal frame rate that times the flash, at least 1.
    pub nominal_fps: u32,
    /// The run's tag, 0 to 127.
    pub run_tag: u8,
    /// The load band.
    pub load: Load,
    /// The patch row.
    pub patches: Patches,
}

/// A rectangle's colour.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Fill {
    /// sRGB codes.
    Srgb([u8; 3]),
    /// A grey at this many nits (HDR patches only).
    Nits(f64),
}

/// What a solid rectangle shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    /// A barcode cell: copy, row, column.
    Cell(CodeCopy, usize, usize),
    /// The flash square.
    Flash,
    /// Patch `k`.
    Patch(usize),
}

/// A solid rectangle, in pixels, half-open and clipped to the client.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    /// What it is.
    pub part: Part,
    /// Left edge.
    pub x0: i64,
    /// Top edge.
    pub y0: i64,
    /// Right edge (exclusive).
    pub x1: i64,
    /// Bottom edge (exclusive).
    pub y1: i64,
    /// Its colour.
    pub fill: Fill,
}

/// Which band a row range belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BandKind {
    /// The moving bars.
    Bars,
    /// The grey ramp.
    Ramp,
    /// The load texture.
    Load(Load),
}

/// Pixel rows that show a band across the whole width.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Band {
    /// Which band.
    pub kind: BandKind,
    /// First row.
    pub y0: i64,
    /// Last row (exclusive).
    pub y1: i64,
    /// Where the band's top sits on this frame, in u (shifted in the motion pattern).
    top_u: i64,
}

/// One frame of pattern v1, as rectangles and bands over a black background.
#[derive(Debug, Clone, PartialEq)]
pub struct Scene {
    size: Size,
    frame: u64,
    rects: Vec<Rect>,
    bands: Vec<Band>,
}

/// Row spans of the element between `top_u` and `bottom_u`, moved down `shift_u` and wrapped at
/// the client's bottom: one span, or two when it straddles the wrap.
fn row_spans(top_u: i64, bottom_u: i64, shift_u: i64, height: u32) -> Vec<(i64, i64, i64)> {
    let (top, bottom) = (top_u + shift_u, bottom_u + shift_u);
    let mut spans = Vec::with_capacity(2);
    for (from, to, origin) in [
        (top, bottom.min(UNITS_PER_HEIGHT), top),
        (
            top - UNITS_PER_HEIGHT,
            bottom - UNITS_PER_HEIGHT,
            top - UNITS_PER_HEIGHT,
        ),
    ] {
        let (from, to) = (from.max(0), to.min(UNITS_PER_HEIGHT));
        if from < to {
            spans.push((px_y(from, height), px_y(to, height), origin));
        }
    }
    spans
}

impl Scene {
    /// Frame `frame` of pattern v1 for a client of `size`.
    pub fn new(size: Size, frame: u64, options: &Options) -> Result<Self, PatternError> {
        if options.nominal_fps == 0 {
            return Err(PatternError::Fps(options.nominal_fps));
        }
        if options.run_tag > 0x7F {
            return Err(PatternError::Tag(options.run_tag));
        }
        let patch_fills = patch_fills(options)?;
        let (w, h) = (i64::from(size.width), size.height);
        let shift_u = match options.pattern {
            Pattern::Full => 0,
            Pattern::Motion => (frame % UNITS_PER_HEIGHT as u64) as i64 * MOTION_SCROLL_U,
        };
        let mut rects = Vec::new();
        let mut push = |part, x0: i64, x1: i64, top_u, bottom_u, fill| {
            for (y0, y1, _) in row_spans(top_u, bottom_u, shift_u, h) {
                let (x0, x1) = (x0.clamp(0, w), x1.clamp(0, w));
                let (y0, y1) = (y0.clamp(0, i64::from(h)), y1.clamp(0, i64::from(h)));
                if x0 < x1 && y0 < y1 {
                    rects.push(Rect {
                        part,
                        x0,
                        y0,
                        x1,
                        y1,
                        fill,
                    });
                }
            }
        };
        // Patches drift right 1 u a frame and wrap over max(client width, 12 patches + gaps).
        // Positions are kept in sub-units of u × height, so the wrap is exact.
        let sub = |u: i64| i128::from(u) * i128::from(h);
        let period = (i128::from(w) * 1_080).max(sub(1_920));
        for (k, fill) in patch_fills.iter().enumerate() {
            let start = (i128::from(frame) % period * i128::from(h)
                + sub(PATCH_PITCH_U * k as i64))
                % period;
            for left in [start, start - period] {
                let x0 = round_half_away(left, 1_080);
                let x1 = round_half_away(left + sub(PATCH_SIZE_U), 1_080);
                push(
                    Part::Patch(k),
                    x0,
                    x1,
                    PATCHES_TOP_U,
                    PATCHES_TOP_U + PATCH_SIZE_U,
                    *fill,
                );
            }
        }
        if options.pattern == Pattern::Full {
            let half = FLASH_SIZE_U / 2;
            let lit = flash_on(frame, options.nominal_fps);
            let grey = if lit { 255 } else { 0 };
            push(
                Part::Flash,
                px_centred(-half, size),
                px_centred(half, size),
                FLASH_TOP_U,
                FLASH_TOP_U + FLASH_SIZE_U,
                Fill::Srgb([grey; 3]),
            );
            let hdr = !matches!(options.patches, Patches::Sdr);
            let cells = barcode_cells(frame, hdr, options.run_tag);
            let left_u = -(CELL_U * CODE_COLS as i64) / 2;
            for copy in [CodeCopy::A, CodeCopy::B] {
                for (row, bits) in cells.iter().enumerate() {
                    for (col, &white) in bits.iter().enumerate() {
                        let x_u = left_u + CELL_U * col as i64;
                        let y_u = copy.top_u() + CELL_U * row as i64;
                        let grey = if white { 255 } else { 0 };
                        push(
                            Part::Cell(copy, row, col),
                            px_centred(x_u, size),
                            px_centred(x_u + CELL_U, size),
                            y_u,
                            y_u + CELL_U,
                            Fill::Srgb([grey; 3]),
                        );
                    }
                }
            }
        }
        let mut bands = Vec::new();
        let mut kinds = vec![(BandKind::Bars, BARS_U), (BandKind::Ramp, RAMP_U)];
        if options.load != Load::None {
            kinds.push((BandKind::Load(options.load), LOAD_U));
        }
        for (kind, (top_u, bottom_u)) in kinds {
            for (y0, y1, origin) in row_spans(top_u, bottom_u, shift_u, h) {
                let (y0, y1) = (y0.clamp(0, i64::from(h)), y1.clamp(0, i64::from(h)));
                if y0 < y1 {
                    bands.push(Band {
                        kind,
                        y0,
                        y1,
                        top_u: origin,
                    });
                }
            }
        }
        Ok(Self {
            size,
            frame,
            rects,
            bands,
        })
    }

    /// The client size.
    pub fn size(&self) -> Size {
        self.size
    }

    /// The frame number.
    pub fn frame(&self) -> u64 {
        self.frame
    }

    /// The solid rectangles: patches, then the flash, then barcode cells (black ones too, so
    /// the barcode never changes the list's length; only the patches do, as they cross a wrap
    /// or the client's edges).
    pub fn rects(&self) -> &[Rect] {
        &self.rects
    }

    /// The bands' row ranges.
    pub fn bands(&self) -> &[Band] {
        &self.bands
    }

    /// The sRGB codes of `band` on pixel row `y`, one per column.
    pub fn band_row(&self, band: &Band, y: i64) -> Vec<[u8; 3]> {
        let h = self.size.height;
        (0..i64::from(self.size.width))
            .map(|x| {
                let x_u = unit_of_pixel(x, h);
                match band.kind {
                    BandKind::Bars => bars_codes(x_u, self.frame).map(round_code),
                    BandKind::Ramp => [round_code(ramp_code(x_u, self.frame)); 3],
                    BandKind::Load(load) => {
                        let y_u = unit_of_pixel(y, h) - band.top_u as f64;
                        load_codes(x_u, y_u, self.frame, load)
                    }
                }
            })
            .collect()
    }

    /// The whole frame as sRGB codes, row by row, the way an SDR swap chain holds it. Bands are
    /// drawn first, then the rectangles.
    pub fn raster_srgb(&self) -> Result<Vec<[u8; 3]>, PatternError> {
        let (w, h) = (self.size.width as usize, self.size.height as usize);
        let mut pixels = vec![[0_u8; 3]; w * h];
        for band in &self.bands {
            for y in band.y0..band.y1 {
                let row = self.band_row(band, y);
                let start = y as usize * w;
                pixels[start..start + w].copy_from_slice(&row);
            }
        }
        for rect in &self.rects {
            let Fill::Srgb(codes) = rect.fill else {
                return Err(PatternError::NotSdr);
            };
            for y in rect.y0..rect.y1 {
                let start = y as usize * w;
                pixels[start + rect.x0 as usize..start + rect.x1 as usize].fill(codes);
            }
        }
        Ok(pixels)
    }
}

/// A code value rounded half away from zero and kept in 0–255.
fn round_code(value: f64) -> u8 {
    value.round().clamp(0.0, 255.0) as u8
}

/// Each patch's fill for these options.
fn patch_fills(options: &Options) -> Result<[Fill; 12], PatternError> {
    let mut fills = [Fill::Srgb([0; 3]); 12];
    match options.patches {
        Patches::Sdr => {
            for (fill, code) in fills.iter_mut().zip(SDR_PATCH_CODES) {
                *fill = Fill::Srgb([code; 3]);
            }
        }
        Patches::Hdr {
            sdr_white_nits,
            bright,
        } => {
            check_nits(sdr_white_nits)?;
            let cap = options.pattern == Pattern::Motion && !bright;
            for (k, fill) in fills.iter_mut().enumerate() {
                let nits = hdr_patch_nits(k, sdr_white_nits);
                *fill = Fill::Nits(if cap { nits.min(sdr_white_nits) } else { nits });
            }
        }
        Patches::ExpectedSdr {
            curve,
            peak_nits,
            sdr_white_nits,
        } => {
            for (k, fill) in fills.iter_mut().enumerate() {
                let nits = hdr_patch_nits(k, sdr_white_nits);
                let y = sdr_luma(nits, curve, peak_nits, sdr_white_nits)?;
                *fill = Fill::Srgb([round_code((y - 16.0) * 255.0 / 219.0); 3]);
            }
        }
    }
    Ok(fills)
}

/// Patch `k`'s level in HDR mode, in nits.
pub fn hdr_patch_nits(k: usize, sdr_white_nits: f64) -> f64 {
    if k == SDR_WHITE_PATCH {
        sdr_white_nits
    } else {
        HDR_PATCH_NITS.get(k).copied().unwrap_or(0.0)
    }
}

fn check_nits(nits: f64) -> Result<f64, PatternError> {
    if nits.is_finite() && nits > 0.0 {
        Ok(nits)
    } else {
        Err(PatternError::Nits(nits))
    }
}

// ---------------------------------------------------------------------------------------------
// Colour
// ---------------------------------------------------------------------------------------------

/// The Y′CbCr matrix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Matrix {
    /// ITU-R BT.709: Kr 0.2126, Kb 0.0722.
    Bt709,
    /// ITU-R BT.601: Kr 0.299, Kb 0.114.
    Bt601,
}

/// The code range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Range {
    /// Limited ("TV"): Y 16–235, Cb and Cr 16–240.
    Limited,
    /// Full ("PC"): 0–255.
    Full,
}

/// sRGB codes (0–255, fractional allowed) to Y′CbCr codes, unrounded. The gamma-coded values
/// go straight into the matrix, as for any desktop recording; nothing is linearised.
pub fn ycbcr(rgb: [f64; 3], matrix: Matrix, range: Range) -> [f64; 3] {
    let (kr, kb) = match matrix {
        Matrix::Bt709 => (0.2126, 0.0722),
        Matrix::Bt601 => (0.299, 0.114),
    };
    let [r, g, b] = rgb.map(|c| c / 255.0);
    let y = kr * r + (1.0 - kr - kb) * g + kb * b;
    let cb = (b - y) / (2.0 * (1.0 - kb));
    let cr = (r - y) / (2.0 * (1.0 - kr));
    match range {
        Range::Limited => [16.0 + 219.0 * y, 128.0 + 224.0 * cb, 128.0 + 224.0 * cr],
        Range::Full => [255.0 * y, 128.0 + 255.0 * cb, 128.0 + 255.0 * cr],
    }
}

/// A full-resolution chroma plane subsampled to 4:2:0 with `chroma_sample_loc_type` 0 (H.273):
/// each sample sits on an even column, between two rows, so it is [1 2 1]/4 across the column
/// and its neighbours and the mean of the two rows. Edges repeat. The result is
/// ⌈width/2⌉ × ⌈height/2⌉, row by row. Errors unless `plane` holds exactly `width` × `height`
/// samples.
pub fn subsample_chroma(
    plane: &[f64],
    width: usize,
    height: usize,
) -> Result<Vec<f64>, PatternError> {
    if width.checked_mul(height) != Some(plane.len()) {
        return Err(PatternError::Plane {
            len: plane.len(),
            width,
            height,
        });
    }
    let (cw, ch) = (width.div_ceil(2), height.div_ceil(2));
    let mut out = Vec::with_capacity(cw * ch);
    let at = |x: usize, y: usize| plane[y * width + x];
    for j in 0..ch {
        let (y0, y1) = (2 * j, (2 * j + 1).min(height - 1));
        for i in 0..cw {
            let x = 2 * i;
            let (left, right) = (x.saturating_sub(1), (x + 1).min(width - 1));
            let column = |y| (at(left, y) + 2.0 * at(x, y) + at(right, y)) / 4.0;
            out.push((column(y0) + column(y1)) / 2.0);
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// HDR
// ---------------------------------------------------------------------------------------------

const PQ_M1: f64 = 2_610.0 / 16_384.0;
const PQ_M2: f64 = 2_523.0 / 4_096.0 * 128.0;
const PQ_C1: f64 = 3_424.0 / 4_096.0;
const PQ_C2: f64 = 2_413.0 / 4_096.0 * 32.0;
const PQ_C3: f64 = 2_392.0 / 4_096.0 * 32.0;

/// The PQ signal (0–1) for `nits` (SMPTE ST 2084's inverse EOTF); levels outside 0–10,000 are
/// clipped.
pub fn pq_from_nits(nits: f64) -> f64 {
    let y = (nits / 10_000.0).clamp(0.0, 1.0);
    let ym = y.powf(PQ_M1);
    ((PQ_C1 + PQ_C2 * ym) / (1.0 + PQ_C3 * ym)).powf(PQ_M2)
}

/// The nits of PQ signal `e` (SMPTE ST 2084's EOTF); `e` is clipped to 0–1.
pub fn nits_from_pq(e: f64) -> f64 {
    let p = e.clamp(0.0, 1.0).powf(1.0 / PQ_M2);
    10_000.0 * ((p - PQ_C1).max(0.0) / (PQ_C2 - PQ_C3 * p)).powf(1.0 / PQ_M1)
}

/// BT.2390's EETF in the PQ domain, from a source that peaks at `source_peak_nits` to a target
/// that peaks at `target_peak_nits`, with no black lift: a Hermite knee at
/// KS = 1.5·maxLum − 0.5. Levels above the source peak are clipped to it first. A target at or
/// above the source peak leaves the level alone.
pub fn bt2390_eetf(nits: f64, source_peak_nits: f64, target_peak_nits: f64) -> f64 {
    let pw = pq_from_nits(source_peak_nits);
    let e1 = (pq_from_nits(nits) / pw).clamp(0.0, 1.0);
    let max_lum = pq_from_nits(target_peak_nits) / pw;
    if max_lum.is_nan() || max_lum >= 1.0 {
        return nits_from_pq(e1 * pw);
    }
    let ks = 1.5 * max_lum - 0.5;
    let e2 = if e1 < ks {
        e1
    } else {
        let t = (e1 - ks) / (1.0 - ks);
        let (t2, t3) = (t * t, t * t * t);
        (2.0 * t3 - 3.0 * t2 + 1.0) * ks
            + (t3 - 2.0 * t2 + t) * (1.0 - ks)
            + (-2.0 * t3 + 3.0 * t2) * max_lum
    };
    nits_from_pq(e2 * pw)
}

/// sRGB's encoding of linear `v` (IEC 61966-2-1), clipped to 0–1.
pub fn srgb_encode(v: f64) -> f64 {
    let v = v.clamp(0.0, 1.0);
    if v <= 0.003_130_8 {
        12.92 * v
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}

/// sRGB's decoding of code value `v` (0–1) to linear, clipped to 0–1.
pub fn srgb_decode(v: f64) -> f64 {
    let v = v.clamp(0.0, 1.0);
    if v <= 0.040_45 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

/// How an HDR source becomes the SDR clip (SP3 picks the default; docs/PLAN.md §2.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Curve {
    /// BT.2390's EETF to SDR white ("Natural").
    Natural,
    /// A plain clip at SDR white ("Match SDR apps").
    MatchSdrApps,
}

/// The limited-range luma code an SDR clip should show for a grey of `nits` from a display
/// that peaks at `peak_nits` with SDR white at `sdr_white_nits`, after `curve`: 16 + 219 ×
/// sRGB(min(curve(L) / white, 1)).
pub fn sdr_luma(
    nits: f64,
    curve: Curve,
    peak_nits: f64,
    sdr_white_nits: f64,
) -> Result<f64, PatternError> {
    let white = check_nits(sdr_white_nits)?;
    let peak = check_nits(peak_nits)?;
    if !nits.is_finite() {
        return Err(PatternError::Nits(nits));
    }
    let relative = match curve {
        Curve::Natural => bt2390_eetf(nits, peak, white) / white,
        Curve::MatchSdrApps => nits / white,
    };
    Ok(16.0 + 219.0 * srgb_encode(relative.min(1.0)))
}

// ---------------------------------------------------------------------------------------------
// Tones
// ---------------------------------------------------------------------------------------------

/// The beeps, each on its own place in every second.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// 1 kHz on the second, with the flash: the testbed's own "game" sound.
    Game,
    /// 2 kHz at +500 ms, from `kairo-tone.exe`: "voice chat".
    Voice,
    /// 3 kHz at +250 ms, a marker in synthetic clips' Mic track only.
    Mic,
}

impl Tone {
    /// Its frequency, in Hz.
    pub const fn hz(self) -> u32 {
        match self {
            Self::Game => 1_000,
            Self::Voice => 2_000,
            Self::Mic => 3_000,
        }
    }

    /// Where it starts in each second, in ms.
    pub const fn offset_ms(self) -> u32 {
        match self {
            Self::Game => 0,
            Self::Voice => 500,
            Self::Mic => 250,
        }
    }
}

/// A sine of `hz` lasting `ms` at `sample_rate`, starting at phase 0, at [`BEEP_PEAK_DBFS`]
/// with [`BEEP_RAMP_MS`] raised-cosine ramps at both ends (each half the burst, if that is
/// shorter).
pub fn tone_burst(hz: f64, ms: u32, sample_rate: u32) -> Result<Vec<f32>, PatternError> {
    if sample_rate == 0 {
        return Err(PatternError::SampleRate(sample_rate));
    }
    let rate = f64::from(sample_rate);
    let samples = u64::from(sample_rate) * u64::from(ms) / 1_000;
    Ok((0..samples)
        .map(|i| tone_burst_at(hz, ms, i as f64 / rate) as f32)
        .collect())
}

/// [`tone_burst`]'s value `t` seconds after its onset, 0 outside it: for a burst that starts
/// between two samples.
pub fn tone_burst_at(hz: f64, ms: u32, t: f64) -> f64 {
    let length = f64::from(ms) / 1_000.0;
    if !(0.0..length).contains(&t) {
        return 0.0;
    }
    let amplitude = 10_f64.powf(BEEP_PEAK_DBFS / 20.0);
    let ramp = (BEEP_RAMP_MS / 1_000.0).min(length / 2.0);
    let envelope = if t < ramp {
        0.5 - 0.5 * (std::f64::consts::PI * t / ramp).cos()
    } else if t > length - ramp {
        0.5 - 0.5 * (std::f64::consts::PI * (length - t) / ramp).cos()
    } else {
        1.0
    };
    amplitude * envelope * (std::f64::consts::TAU * hz * t).sin()
}

/// One beep of `tone`: [`BEEP_MS`] long.
pub fn beep(tone: Tone, sample_rate: u32) -> Result<Vec<f32>, PatternError> {
    tone_burst(f64::from(tone.hz()), BEEP_MS, sample_rate)
}

/// The placeholder cue's length, in ms.
pub const CUE_MS: u32 = 220;

/// A stand-in for Kairo's save cue until M4 defines it: 600 Hz for 100 ms, then 1,500 Hz for
/// 120 ms, each with its own ramps. Its notes keep at least 200 Hz from every beep's frequency,
/// as the real cue must.
pub fn cue_placeholder(sample_rate: u32) -> Result<Vec<f32>, PatternError> {
    let mut samples = tone_burst(600.0, 100, sample_rate)?;
    samples.extend(tone_burst(1_500.0, 120, sample_rate)?);
    Ok(samples)
}

/// [`cue_placeholder`]'s value `t` seconds after its start, 0 outside it.
pub fn cue_placeholder_at(t: f64) -> f64 {
    if t < 0.1 {
        tone_burst_at(600.0, 100, t)
    } else {
        tone_burst_at(1_500.0, 120, t - 0.1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Expected values come from an independent Python model written from the design
    // (docs/research/16-verify-testbed/README.md §2 items 6–8, §4.5–4.8), with exact integers
    // for the geometry and the barcode.

    fn sdr(pattern: Pattern, nominal_fps: u32, run_tag: u8, load: Load) -> Options {
        Options {
            pattern,
            nominal_fps,
            run_tag,
            load,
            patches: Patches::Sdr,
        }
    }

    fn rows(cells: &[[bool; CODE_COLS]; CODE_ROWS]) -> [u16; CODE_ROWS] {
        cells.map(|row| {
            row.iter()
                .enumerate()
                .fold(0, |acc, (c, &white)| acc | (u16::from(white) << (15 - c)))
        })
    }

    fn levels(cells: &[[bool; CODE_COLS]; CODE_ROWS]) -> [[f64; CODE_COLS]; CODE_ROWS] {
        cells.map(|row| row.map(|white| if white { 235.0 } else { 16.0 }))
    }

    #[test]
    fn crc_and_whitening_follow_their_definitions() {
        // CRC-16/CCITT-FALSE's published check value.
        assert_eq!(crc16(b"123456789"), 0x29B1);
        assert_eq!(mask40(0), 0xdc_c5a4_717c);
        assert_eq!(mask40(1), 0x44_1cda_02ed);
        assert_eq!(mask40(0xAB), 0x32_3243_9981);
        assert_eq!(mask40(0xFF), 0x1e_e7ee_ce5d);
    }

    #[test]
    fn barcodes_match_the_model() {
        let cases: [(u64, bool, u8, [u16; 4]); 6] = [
            (0, false, 0, [0xaaaa, 0x00dc, 0xc5a4, 0x7c8f]),
            (1, false, 0, [0x5555, 0x0144, 0x1cda, 0x3c2f]),
            (0xAB_CDEF, true, 0x5A, [0x5555, 0xef77, 0x3fab, 0x385d]),
            (0xFF_FFFF, false, 127, [0x5555, 0xffe1, 0x1891, 0xcbe9]),
            ((1 << 24) + 6, true, 3, [0xaaaa, 0x0678, 0xcc1e, 0xdb18]),
            (5_397, false, 93, [0x5555, 0x1588, 0x35ad, 0xbb2a]),
        ];
        for (frame, hdr, tag, expected) in cases {
            assert_eq!(
                rows(&barcode_cells(frame, hdr, tag)),
                expected,
                "frame {frame}"
            );
        }
    }

    #[test]
    fn barcodes_read_back() {
        let frames = (0..5_000)
            .step_by(37)
            .chain([(1 << 24) - 1, 1 << 24, (1 << 30) + 12_345]);
        for frame in frames {
            let (hdr, tag) = (frame % 2 == 1, (frame % 128) as u8);
            let read = read_barcode(&levels(&barcode_cells(frame, hdr, tag))).unwrap();
            assert_eq!(
                (read.frame24, read.hdr, read.tag),
                ((frame % (1 << 24)) as u32, hdr, tag)
            );
            assert_eq!(read.margin, 1.0);
        }
        // Mild noise moves the margin, not the reading.
        let mut noisy = levels(&barcode_cells(0x12_3456, true, 77));
        for (k, cell) in noisy.iter_mut().flatten().enumerate() {
            *cell += 20.0 * (k as f64 * 1.7).sin();
        }
        let read = read_barcode(&noisy).unwrap();
        assert_eq!((read.frame24, read.hdr, read.tag), (0x12_3456, true, 77));
        assert!(read.margin > 0.8 && read.margin < 1.0, "{}", read.margin);
    }

    #[test]
    fn every_single_data_cell_flip_is_caught() {
        for frame in [0, 1, 0x12_3456, 0xFF_FFFF, (1 << 20) + 3] {
            for (hdr, tag) in [(false, 0), (true, 77), (false, 127)] {
                let clean = levels(&barcode_cells(frame, hdr, tag));
                for k in 0..48 {
                    let mut flipped = clean;
                    let cell = &mut flipped[1 + k / 16][k % 16];
                    *cell = 251.0 - *cell;
                    let read = read_barcode(&flipped);
                    assert!(
                        matches!(read, Err(BarcodeError::Crc | BarcodeError::Parity)),
                        "frame {frame} cell {k}: {read:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn barcode_faults_are_named() {
        let clean = levels(&barcode_cells(0x12_3456, true, 77));
        let mut flipped = clean;
        flipped[2][5] = 251.0 - flipped[2][5];
        assert_eq!(read_barcode(&flipped), Err(BarcodeError::Crc));
        let dim =
            barcode_cells(0x12_3456, true, 77).map(|r| r.map(|w| if w { 130.0 } else { 30.0 }));
        assert_eq!(read_barcode(&dim), Err(BarcodeError::LowContrast));
        let mut reference = clean;
        reference[0][3] = 251.0 - reference[0][3];
        assert_eq!(read_barcode(&reference), Err(BarcodeError::References));
        // An odd frame's references over an even frame's data.
        let mut phase = clean;
        phase[0] = levels(&barcode_cells(0x12_3457, true, 77))[0];
        assert_eq!(read_barcode(&phase), Err(BarcodeError::Parity));
        let blank = [[f64::NAN; CODE_COLS]; CODE_ROWS];
        assert_eq!(read_barcode(&blank), Err(BarcodeError::LowContrast));
    }

    #[test]
    fn the_reader_uses_medians_and_the_110_code_limit() {
        // One black reference at 120: the median ignores it (threshold 125.5), a mean wouldn't
        // (threshold 132, so white data at 130 would read as black).
        let cells = barcode_cells(0x12_3456, true, 77);
        let mut means = cells.map(|row| row.map(|white| if white { 130.0 } else { 16.0 }));
        means[0] = levels(&cells)[0];
        means[0][1] = 120.0;
        let read = read_barcode(&means).unwrap();
        assert_eq!((read.frame24, read.hdr, read.tag), (0x12_3456, true, 77));
        assert_eq!(
            (read.white, read.black, read.midpoint()),
            (235.0, 16.0, 125.5)
        );
        assert!((read.margin - 4.5 / 109.5).abs() < 1e-12, "{}", read.margin);
        // White and black references exactly 110 codes apart read; 109.9 don't.
        for (white, ok) in [(126.0, true), (125.9, false)] {
            let dim = cells.map(|row| row.map(|w| if w { white } else { 16.0 }));
            assert_eq!(read_barcode(&dim).is_ok(), ok, "{white}");
        }
        // The tag has 7 bits: the eighth never reaches the HDR bit.
        assert_eq!(
            barcode_cells(9, false, 0x80 | 5),
            barcode_cells(9, false, 5)
        );
    }

    #[test]
    fn frame_numbers_unwrap_to_the_nearest() {
        assert_eq!(unwrap_frame(0, 5), 5);
        assert_eq!(unwrap_frame((1 << 24) - 2, 3), (1 << 24) + 3);
        assert_eq!(unwrap_frame(1 << 24, 0xFF_FFFF), (1 << 24) - 1);
        assert_eq!(unwrap_frame(100, 0xFF_FFFF), -1);
    }

    #[test]
    fn negative_halves_round_away_from_zero() {
        // −0.5 px rounds to −1, not 0 (half up would give 0).
        assert_eq!(px_y(-540, 1), -1);
        assert_eq!(px_y(-1, 540), -1);
        // −96 u at 45 px high is −4 px exactly halfway: −3.5 → −4.
        assert_eq!(px_centred(-96, Size::new(1, 45).unwrap()), -4);
    }

    #[test]
    fn units_round_half_away_from_zero() {
        let ys = [0, 1, 24, 32, 160, 448, 952, 1_048, 1_080, -16, -24];
        let cases: [(u32, [i64; 11]); 8] = [
            (1_080, [0, 1, 24, 32, 160, 448, 952, 1_048, 1_080, -16, -24]),
            (
                1_440,
                [0, 1, 32, 43, 213, 597, 1_269, 1_397, 1_440, -21, -32],
            ),
            (720, [0, 1, 16, 21, 107, 299, 635, 699, 720, -11, -16]),
            (270, [0, 0, 6, 8, 40, 112, 238, 262, 270, -4, -6]),
            (768, [0, 1, 17, 23, 114, 319, 677, 745, 768, -11, -17]),
            (1, [0, 0, 0, 0, 0, 0, 1, 1, 1, 0, 0]),
            (1_081, [0, 1, 24, 32, 160, 448, 953, 1_049, 1_081, -16, -24]),
            (
                2_160,
                [0, 2, 48, 64, 320, 896, 1_904, 2_096, 2_160, -32, -48],
            ),
        ];
        for (height, expected) in cases {
            assert_eq!(ys.map(|y| px_y(y, height)), expected, "height {height}");
        }
        let offsets = [-192, -168, 0, 96, 192, -96];
        let centred: [((u32, u32), [i64; 6]); 7] = [
            ((1_920, 1_080), [768, 792, 960, 1_056, 1_152, 864]),
            ((2_560, 1_440), [1_024, 1_056, 1_280, 1_408, 1_536, 1_152]),
            ((1_280, 720), [512, 528, 640, 704, 768, 576]),
            ((480, 270), [192, 198, 240, 264, 288, 216]),
            ((1_366, 768), [546, 564, 683, 751, 820, 615]),
            ((3_440, 1_440), [1_464, 1_496, 1_720, 1_848, 1_976, 1_592]),
            ((1, 1), [0, 0, 1, 1, 1, 0]),
        ];
        for ((w, h), expected) in centred {
            let size = Size::new(w, h).unwrap();
            assert_eq!(offsets.map(|o| px_centred(o, size)), expected, "{w}x{h}");
        }
    }

    fn patch_spans(scene: &Scene, k: usize) -> Vec<(i64, i64)> {
        scene
            .rects()
            .iter()
            .filter(|r| r.part == Part::Patch(k))
            .map(|r| (r.x0, r.x1))
            .collect()
    }

    #[test]
    fn patches_drift_and_wrap_exactly() {
        // The model's left and right edges for patches 0, 5 and 11, clipped to the client.
        type Spans<'a> = [&'a [(i64, i64)]; 3];
        let cases: [((u32, u32), u64, Spans); 9] = [
            (
                (1_920, 1_080),
                0,
                [&[(0, 128)], &[(800, 928)], &[(1_760, 1_888)]],
            ),
            (
                (1_920, 1_080),
                1_000,
                [&[(1_000, 1_128)], &[(1_800, 1_920), (0, 8)], &[(840, 968)]],
            ),
            (
                (1_920, 1_080),
                1_000_000,
                [&[(1_600, 1_728)], &[(480, 608)], &[(1_440, 1_568)]],
            ),
            (
                (2_560, 1_440),
                1_000,
                [
                    &[(1_333, 1_504)],
                    &[(2_400, 2_560), (0, 11)],
                    &[(1_120, 1_291)],
                ],
            ),
            (
                (1_366, 768),
                1_000,
                [&[(711, 802)], &[(1_280, 1_366), (0, 5)], &[(597, 688)]],
            ),
            (
                (1_366, 768),
                1_000_000,
                [&[(791, 882)], &[(1_360, 1_366), (0, 85)], &[(677, 768)]],
            ),
            (
                (480, 270),
                1_000,
                [&[(250, 282)], &[(450, 480), (0, 2)], &[(210, 242)]],
            ),
            ((1_280, 1_024), 0, [&[(0, 121)], &[(759, 880)], &[]]),
            (
                (1_280, 1_024),
                1_000,
                [&[(948, 1_070)], &[(0, 8)], &[(796, 918)]],
            ),
        ];
        let options = sdr(Pattern::Full, 60, 0, Load::Scroll);
        for ((w, h), frame, expected) in cases {
            let scene = Scene::new(Size::new(w, h).unwrap(), frame, &options).unwrap();
            for (k, spans) in [0, 5, 11].into_iter().zip(expected) {
                assert_eq!(
                    patch_spans(&scene, k),
                    spans,
                    "{w}x{h} frame {frame} patch {k}"
                );
            }
        }
    }

    fn close(a: [f64; 3], b: [f64; 3]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-6)
    }

    #[test]
    fn bars_match_the_model() {
        let xs = [
            0.0, 7.5, 15.9, 16.0, 100.0, 150.0, 160.0, 175.5, 1_270.0, 1_279.9,
        ];
        let gray = |v: f64| [v; 3];
        let cases: [(u64, [[f64; 3]; 10]); 3] = [
            (
                0,
                [
                    gray(95.5),
                    gray(159.633880188),
                    gray(190.995397756),
                    gray(191.0),
                    gray(191.0),
                    [191.0, 191.0, 174.905347975],
                    [191.0, 191.0, 95.5],
                    [191.0, 191.0, 0.115033932],
                    gray(16.094652025),
                    gray(94.562446003),
                ],
            ),
            (
                1,
                [
                    gray(67.777813322),
                    gray(136.331511423),
                    gray(186.61124043),
                    gray(186.887802062),
                    gray(191.0),
                    [191.0, 191.0, 186.887802062],
                    [191.0, 191.0, 123.222186678],
                    [191.0, 191.0, 5.582541775],
                    gray(4.112197938),
                    gray(66.881966047),
                ],
            ),
            (
                (1 << 24) + 17,
                [
                    [0.0, 191.0, 191.0],
                    [0.0, 191.0, 181.830977493],
                    [0.0, 191.0, 124.118033953],
                    [0.0, 191.0, 123.222186678],
                    [0.0, 191.0, 0.0],
                    [0.0, 191.0, 0.0],
                    [0.0, 191.0, 0.0],
                    [63.327019001, 127.672980999, 63.327019001],
                    [0.0, 191.0, 191.0],
                    [0.0, 191.0, 191.0],
                ],
            ),
        ];
        for (frame, expected) in cases {
            for (x, want) in xs.into_iter().zip(expected) {
                let got = bars_codes(x, frame);
                assert!(close(got, want), "frame {frame} x {x}: {got:?} vs {want:?}");
            }
        }
    }

    #[test]
    fn the_ramp_and_load_match_the_model() {
        let xs = [0.0, 1.0, 256.0, 511.0, 512.0, 700.0, 1_023.5];
        let cases: [(u64, [f64; 7]); 3] = [
            (
                0,
                [
                    0.0,
                    0.498046875,
                    127.5,
                    254.501953125,
                    255.0,
                    161.3671875,
                    0.249023438,
                ],
            ),
            (
                1,
                [
                    0.99609375,
                    1.494140625,
                    128.49609375,
                    254.501953125,
                    254.00390625,
                    160.37109375,
                    0.747070312,
                ],
            ),
            (
                333,
                [
                    178.30078125,
                    177.802734375,
                    50.80078125,
                    76.201171875,
                    76.69921875,
                    170.33203125,
                    178.549804688,
                ],
            ),
        ];
        for (frame, expected) in cases {
            for (x, want) in xs.into_iter().zip(expected) {
                assert!(
                    (ramp_code(x, frame) - want).abs() < 1e-6,
                    "frame {frame} x {x}"
                );
            }
        }
        assert_eq!(ramp_code(256.0, 1 << 30), 127.5);
        let points = [
            (0.0, 0.0),
            (8.0, 0.0),
            (17.3, 8.1),
            (4_000.0, 95.9),
            (-3.0, 40.0),
        ];
        let cases: [(u64, Load, [[u8; 3]; 5]); 4] = [
            (
                0,
                Load::Scroll,
                [
                    [169, 122, 26],
                    [209, 213, 53],
                    [7, 253, 87],
                    [34, 43, 89],
                    [119, 140, 161],
                ],
            ),
            (
                7,
                Load::Scroll,
                [
                    [1, 232, 157],
                    [53, 108, 102],
                    [62, 42, 118],
                    [137, 209, 113],
                    [206, 134, 8],
                ],
            ),
            (
                7,
                Load::Churn,
                [
                    [83, 55, 194],
                    [168, 6, 2],
                    [35, 17, 176],
                    [34, 167, 68],
                    [50, 78, 251],
                ],
            ),
            (
                123_456,
                Load::Scroll,
                [
                    [98, 156, 179],
                    [48, 5, 183],
                    [162, 171, 40],
                    [233, 114, 143],
                    [224, 222, 241],
                ],
            ),
        ];
        for (frame, load, expected) in cases {
            for ((x, y), want) in points.into_iter().zip(expected) {
                assert_eq!(
                    load_codes(x, y, frame, load),
                    want,
                    "frame {frame} {load:?} ({x}, {y})"
                );
            }
        }
        assert_eq!(load_codes(5.0, 5.0, 3, Load::None), [0, 0, 0]);
    }

    #[test]
    fn colours_follow_the_published_matrices() {
        // Research 09 §3.2's 75% bars: e.g. yellow 168/44/136 after rounding.
        let expected: [[f64; 3]; 8] = [
            [180.035294, 128.0, 128.0],
            [168.191946, 44.109804, 135.692243],
            [145.161391, 147.222953, 44.109804],
            [133.318042, 63.332757, 51.802047],
            [62.717252, 192.667243, 204.197953],
            [50.873904, 108.777047, 211.890196],
            [27.843348, 211.890196, 120.307757],
            [16.0, 128.0, 128.0],
        ];
        for (bar, want) in BARS.iter().zip(expected) {
            let got = ycbcr(bar.map(f64::from), Matrix::Bt709, Range::Limited);
            assert!(close(got, want), "{bar:?}: {got:?}");
        }
        let yellow = BARS[1].map(f64::from);
        assert!(close(
            ycbcr(yellow, Matrix::Bt709, Range::Full),
            [177.2098, 32.5, 136.756795]
        ));
        assert!(close(
            ycbcr(yellow, Matrix::Bt601, Range::Limited),
            [161.335271, 44.109804, 141.642628]
        ));
        assert!(close(
            ycbcr([127.5; 3], Matrix::Bt709, Range::Limited),
            [125.5, 128.0, 128.0]
        ));
    }

    #[test]
    fn chroma_is_left_sited() {
        // A 5 × 3 plane holding 10 × row + column.
        let plane: Vec<f64> = (0..3)
            .flat_map(|r| (0..5).map(move |c| f64::from(10 * r + c)))
            .collect();
        assert_eq!(
            subsample_chroma(&plane, 5, 3),
            Ok(vec![5.25, 7.0, 8.75, 20.25, 22.0, 23.75])
        );
        for (width, height) in [(6, 3), (5, 2), (usize::MAX, 2), (2, usize::MAX)] {
            assert_eq!(
                subsample_chroma(&plane, width, height),
                Err(PatternError::Plane {
                    len: 15,
                    width,
                    height
                })
            );
        }
        assert_eq!(subsample_chroma(&[], 0, 0), Ok(vec![]));
    }

    #[test]
    fn hdr_levels_follow_pq_and_bt2390() {
        let pq = [0.0, 100.0, 203.0, 240.0, 1_000.0, 10_000.0, 20_000.0].map(pq_from_nits);
        let want = [1e-6, 0.508078, 0.580689, 0.598256, 0.751827, 1.0, 1.0];
        for (got, want) in pq.iter().zip(want) {
            assert!((got - want).abs() < 1e-6, "{got} vs {want}");
        }
        for nits in [0.0, 1.0, 100.0, 1_000.0, 9_999.0] {
            assert!((nits_from_pq(pq_from_nits(nits)) - nits).abs() < 1e-6 * (1.0 + nits));
        }
        // Research 09: the EETF puts 240 nits at about 193 when SDR white is 240.
        assert!((bt2390_eetf(240.0, 1_000.0, 240.0) - 193.1666).abs() < 1e-4);
        assert!((bt2390_eetf(1_000.0, 1_000.0, 240.0) - 240.0).abs() < 1e-9);
        assert!((bt2390_eetf(50.0, 1_000.0, 240.0) - 50.0).abs() < 1e-9);
        // Levels above the source peak are clipped to it first.
        assert_eq!(
            bt2390_eetf(4_000.0, 1_000.0, 240.0),
            bt2390_eetf(1_000.0, 1_000.0, 240.0)
        );
        let cases: [(f64, f64, Curve, [f64; 12]); 6] = [
            (
                1_000.0,
                240.0,
                Curve::Natural,
                [
                    16.0, 65.4173, 113.4683, 150.1377, 176.9662, 195.2572, 207.764, 215.0187,
                    229.4469, 234.0184, 235.0, 235.0,
                ],
            ),
            (
                1_000.0,
                240.0,
                Curve::MatchSdrApps,
                [
                    16.0, 65.4173, 113.4683, 150.1377, 177.0432, 199.0855, 219.4308, 235.0, 235.0,
                    235.0, 235.0, 235.0,
                ],
            ),
            (
                600.0,
                240.0,
                Curve::Natural,
                [
                    16.0, 65.4173, 113.4683, 150.1377, 177.0432, 198.8477, 214.1236, 222.0592,
                    233.8256, 235.0, 235.0, 235.0,
                ],
            ),
            (
                1_000.0,
                200.0,
                Curve::Natural,
                [
                    16.0, 70.2683, 122.1119, 161.6755, 187.5144, 202.759, 213.0291, 212.4513,
                    230.5621, 234.2171, 235.0, 235.0,
                ],
            ),
            (
                400.0,
                240.0,
                Curve::Natural,
                [
                    16.0, 65.4173, 113.4683, 150.1377, 177.0432, 199.0855, 218.5005, 227.7474,
                    235.0, 235.0, 235.0, 235.0,
                ],
            ),
            // SDR white above the source's peak: nothing to compress, only the clip.
            (
                400.0,
                480.0,
                Curve::Natural,
                [
                    16.0, 49.9997, 85.9973, 113.4683, 133.6246, 150.1377, 165.3795, 218.0982,
                    218.0982, 218.0982, 218.0982, 218.0982,
                ],
            ),
        ];
        for (peak, white, curve, expected) in cases {
            for (k, want) in expected.into_iter().enumerate() {
                let got = sdr_luma(hdr_patch_nits(k, white), curve, peak, white).unwrap();
                assert!(
                    (got - want).abs() < 1e-3,
                    "{curve:?} {peak}/{white} patch {k}: {got}"
                );
            }
        }
        assert_eq!(
            sdr_luma(10.0, Curve::Natural, 1_000.0, 0.0),
            Err(PatternError::Nits(0.0))
        );
        assert_eq!(
            sdr_luma(10.0, Curve::Natural, 0.0, 240.0),
            Err(PatternError::Nits(0.0))
        );
        assert!(sdr_luma(f64::NAN, Curve::Natural, 1_000.0, 240.0).is_err());
    }

    #[test]
    fn srgb_follows_iec_61966_2_1() {
        // Both segments of each curve, from the published formula.
        for (linear, code) in [
            (0.002, 0.02584),
            (0.003_130_8, 0.040_449_936),
            (0.18, 0.461_356_129_5),
            (1.0, 1.0),
        ] {
            assert!((srgb_encode(linear) - code).abs() < 1e-9, "{linear}");
        }
        for (code, linear) in [
            (0.02, 0.001_547_987_616),
            (0.040_45, 0.003_130_804_954),
            (0.5, 0.214_041_140_482),
            (1.0, 1.0),
        ] {
            assert!((srgb_decode(code) - linear).abs() < 1e-11, "{code}");
        }
        for c in 0..=255 {
            let v = f64::from(c) / 255.0;
            assert!((srgb_encode(srgb_decode(v)) - v).abs() < 1e-12, "{c}");
        }
    }

    #[test]
    fn short_bursts_keep_a_smooth_envelope() {
        // At 12 kHz and 48 kHz every odd sample sits on a sine peak, so it reads the envelope.
        let amplitude = 10_f64.powf(BEEP_PEAK_DBFS / 20.0);
        for ms in 1..=6 {
            let burst = tone_burst(12_000.0, ms, 48_000).unwrap();
            let envelope: Vec<f64> = burst
                .iter()
                .skip(1)
                .step_by(2)
                .map(|&x| f64::from(x).abs() / amplitude)
                .collect();
            // Ramps of at most half the burst: no step between neighbours above a slope of
            // pi/2 per ramp, and the middle reaches full level.
            let steepest = envelope
                .windows(2)
                .map(|w| (w[1] - w[0]).abs())
                .fold(0.0, f64::max);
            assert!(steepest < 0.2, "{ms} ms: a step of {steepest}");
            let peak = envelope.iter().copied().fold(0.0, f64::max);
            assert!(peak > 0.99, "{ms} ms: peak {peak}");
        }
    }

    #[test]
    fn bursts_between_samples_follow_the_sampled_ones() {
        // The per-time functions give the sampled bursts, and are 0 outside them.
        let beep = beep(Tone::Voice, 48_000).unwrap();
        for (i, &x) in beep.iter().enumerate() {
            assert_eq!(
                tone_burst_at(2_000.0, BEEP_MS, i as f64 / 48_000.0) as f32,
                x
            );
        }
        let cue = cue_placeholder(48_000).unwrap();
        assert_eq!(cue.len(), 48 * CUE_MS as usize);
        // Up to the samples' f32 rounding: the second note's time is t − 0.1 here and
        // (i − 4800) / 48000 there.
        for (i, &x) in cue.iter().enumerate() {
            let at = cue_placeholder_at(i as f64 / 48_000.0);
            assert!((at - f64::from(x)).abs() < 1e-8, "{i}");
        }
        for t in [-1e-9, 0.1, 0.5] {
            assert_eq!(tone_burst_at(1_000.0, BEEP_MS, t), 0.0);
        }
        assert_eq!(cue_placeholder_at(0.22), 0.0);
        assert_eq!(cue_placeholder_at(-0.001), 0.0);
        // Half a sample late: the same sine, shifted.
        let t = 0.05 + 0.5 / 48_000.0;
        let late = tone_burst_at(1_000.0, BEEP_MS, t - 0.5 / 48_000.0);
        assert!((late - tone_burst_at(1_000.0, BEEP_MS, 0.05)).abs() < 1e-15);
    }

    #[test]
    fn the_flash_lights_a_tenth_of_each_second() {
        // A rate of 0 counts as 1 instead of dividing by zero: (1 + 5) / 10 = 0 frames lit.
        assert!(!flash_on(0, 0));
        for fps in [60_u32, 144, 165, 175, 180] {
            let lit = u64::from((fps + 5) / 10);
            let f = u64::from(fps);
            let frames = [0, lit - 1, lit, f - 1, f, f + 1];
            assert_eq!(
                frames.map(|n| flash_on(n, fps)),
                [true, true, false, false, true, true],
                "{fps}"
            );
        }
        assert_eq!(
            [60, 144, 165, 175, 180].map(|f| (f + 5) / 10),
            [6, 14, 17, 18, 18]
        );
    }

    #[test]
    fn tones_start_at_phase_zero_and_peak_at_minus_30_dbfs() {
        let game = beep(Tone::Game, 48_000).unwrap();
        assert_eq!(game.len(), 4_800);
        let want = [
            (0, 0.0),
            (1, 1.105e-06),
            (48, 0.0),
            (95, -0.004_126_496),
            (96, 0.0),
            (2_400, 0.0),
            (4_799, -1.105e-06),
        ];
        for (i, v) in want {
            assert!(
                (f64::from(game[i]) - v).abs() < 1e-8,
                "sample {i}: {}",
                game[i]
            );
        }
        let peak = game.iter().fold(0.0_f32, |p, s| p.max(s.abs()));
        assert!((f64::from(peak) - 0.031_622_777).abs() < 1e-8);
        let voice = beep(Tone::Voice, 48_000).unwrap();
        for (i, v) in [(1, 2.191e-06), (50, 0.008_422_751), (1_000, -0.027_386_128)] {
            assert!((f64::from(voice[i]) - v).abs() < 1e-8, "sample {i}");
        }
        let cue = cue_placeholder(48_000).unwrap();
        assert_eq!(cue.len(), 10_560);
        for (i, v) in [
            (10, 0.000_593_339),
            (4_810, 0.000_775_235),
            (10_559, -1.652e-06),
        ] {
            assert!((f64::from(cue[i]) - v).abs() < 1e-8, "cue sample {i}");
        }
        assert_eq!(beep(Tone::Game, 44_100).unwrap().len(), 4_410);
        assert_eq!(beep(Tone::Game, 0), Err(PatternError::SampleRate(0)));
        assert_eq!(
            [Tone::Game, Tone::Voice, Tone::Mic].map(|t| (t.hz(), t.offset_ms())),
            [(1_000, 0), (2_000, 500), (3_000, 250)]
        );
    }

    fn pixel(scene: &Scene, raster: &[[u8; 3]], x: usize, y: usize) -> [u8; 3] {
        raster[y * scene.size().width() as usize + x]
    }

    #[test]
    fn rasters_match_the_model() {
        type Point = (usize, usize, [u8; 3]);
        let full: [Point; 14] = [
            (320, 20, [0; 3]),
            (250, 15, [0; 3]),
            (390, 39, [0; 3]),
            (320, 180, [0; 3]),
            (330, 190, [0; 3]),
            (100, 80, [191, 191, 0]),
            (101, 125, [159; 3]),
            (600, 130, [116; 3]),
            (5, 300, [78, 2, 216]),
            (300, 300, [77, 6, 163]),
            (639, 359, [0; 3]),
            (0, 0, [0; 3]),
            (400, 230, [160; 3]),
            (100, 240, [0; 3]),
        ];
        let motion: [Point; 17] = [
            (320, 20, [0; 3]),
            (250, 15, [0; 3]),
            (390, 39, [191, 0, 191]),
            (320, 180, [0; 3]),
            (330, 190, [0; 3]),
            (100, 80, [0; 3]),
            (101, 125, [0; 3]),
            (600, 130, [0; 3]),
            (5, 300, [0; 3]),
            (300, 300, [0; 3]),
            (639, 359, [0; 3]),
            (0, 0, [0; 3]),
            (400, 230, [8; 3]),
            (100, 240, [0; 3]),
            // Inside the load band where it wraps from the bottom to the top (rows 251–282).
            (100, 260, [148, 36, 217]),
            (300, 270, [229, 192, 209]),
            (600, 282, [12, 253, 66]),
        ];
        let small: [Point; 10] = [
            (320, 20, [0; 3]),
            (250, 15, [255; 3]),
            (390, 39, [0; 3]),
            (320, 180, [0; 3]),
            (330, 190, [191; 3]),
            (100, 80, [0, 191, 191]),
            (101, 125, [0; 3]),
            (0, 0, [0; 3]),
            (400, 230, [198, 110, 201]),
            (100, 240, [0; 3]),
        ];
        type Case<'a> = ((u32, u32), u64, Options, &'a [Point]);
        let cases: [Case; 3] = [
            (
                (640, 360),
                7,
                sdr(Pattern::Full, 60, 5, Load::Scroll),
                &full,
            ),
            (
                (640, 360),
                1_000,
                sdr(Pattern::Motion, 60, 0, Load::Scroll),
                &motion,
            ),
            (
                (480, 270),
                3,
                sdr(Pattern::Full, 180, 93, Load::Churn),
                &small,
            ),
        ];
        for ((w, h), frame, options, points) in cases {
            let scene = Scene::new(Size::new(w, h).unwrap(), frame, &options).unwrap();
            let raster = scene.raster_srgb().unwrap();
            for &(x, y, want) in points {
                assert_eq!(
                    pixel(&scene, &raster, x, y),
                    want,
                    "{w}x{h} frame {frame} ({x}, {y})"
                );
            }
        }
    }

    #[test]
    fn scenes_hold_the_expected_parts() {
        let size = Size::new(1_920, 1_080).unwrap();
        let full = Scene::new(size, 0, &sdr(Pattern::Full, 60, 0, Load::Scroll)).unwrap();
        // 12 patches, the flash and 2 × 64 barcode cells; no patch wraps on frame 0.
        assert_eq!(full.rects().len(), 12 + 1 + 128);
        assert_eq!(full.bands().len(), 3);
        let flash = full.rects().iter().find(|r| r.part == Part::Flash).unwrap();
        assert_eq!(
            (flash.x0, flash.y0, flash.x1, flash.y1),
            (864, 448, 1_056, 640)
        );
        assert_eq!(flash.fill, Fill::Srgb([255; 3]));
        let cell = full
            .rects()
            .iter()
            .find(|r| r.part == Part::Cell(CodeCopy::B, 3, 15))
            .unwrap();
        assert_eq!(
            (cell.x0, cell.y0, cell.x1, cell.y1),
            (1_128, 1_024, 1_152, 1_048)
        );
        // The motion pattern has no barcode and no flash, and a band can straddle the wrap.
        let motion = Scene::new(size, 900, &sdr(Pattern::Motion, 60, 0, Load::None)).unwrap();
        assert!(
            motion
                .rects()
                .iter()
                .all(|r| matches!(r.part, Part::Patch(_)))
        );
        let bars: Vec<(i64, i64)> = motion
            .bands()
            .iter()
            .filter(|b| b.kind == BandKind::Bars)
            .map(|b| (b.y0, b.y1))
            .collect();
        assert_eq!(bars, [(1_060, 1_080), (0, 172)]);
        assert!(
            motion
                .bands()
                .iter()
                .all(|b| b.kind != BandKind::Load(Load::None))
        );
    }

    #[test]
    fn hdr_patches_are_nits_or_their_sdr_codes() {
        let size = Size::new(1_920, 1_080).unwrap();
        let mut options = sdr(Pattern::Full, 60, 0, Load::Scroll);
        options.patches = Patches::Hdr {
            sdr_white_nits: 240.0,
            bright: false,
        };
        let scene = Scene::new(size, 0, &options).unwrap();
        let fills: Vec<Fill> = (0..12)
            .map(|k| {
                scene
                    .rects()
                    .iter()
                    .find(|r| r.part == Part::Patch(k))
                    .unwrap()
                    .fill
            })
            .collect();
        assert_eq!(fills[7], Fill::Nits(240.0));
        assert_eq!(fills[11], Fill::Nits(4_000.0));
        assert_eq!(scene.raster_srgb(), Err(PatternError::NotSdr));
        // The motion pattern caps them at SDR white unless asked.
        options.pattern = Pattern::Motion;
        let capped = Scene::new(size, 0, &options).unwrap();
        let top = capped.rects().iter().filter_map(|r| match r.fill {
            Fill::Nits(n) => Some(n),
            Fill::Srgb(_) => None,
        });
        assert!(top.fold(0.0, f64::max) <= 240.0);
        options.patches = Patches::Hdr {
            sdr_white_nits: 240.0,
            bright: true,
        };
        let bright = Scene::new(size, 0, &options).unwrap();
        let fill = |scene: &Scene, k| {
            scene
                .rects()
                .iter()
                .find(|r| r.part == Part::Patch(k))
                .map(|r| r.fill)
        };
        assert_eq!(fill(&bright, 11), Some(Fill::Nits(4_000.0)));
        options.patches = Patches::ExpectedSdr {
            curve: Curve::Natural,
            peak_nits: 1_000.0,
            sdr_white_nits: 240.0,
        };
        options.pattern = Pattern::Full;
        let expected = Scene::new(size, 0, &options).unwrap();
        let white = expected
            .rects()
            .iter()
            .find(|r| r.part == Part::Patch(7))
            .unwrap();
        // Natural puts SDR white at Y 215.02: sRGB code 232.
        assert_eq!(white.fill, Fill::Srgb([232; 3]));
        options.patches = Patches::Hdr {
            sdr_white_nits: f64::NAN,
            bright: false,
        };
        assert_eq!(
            Scene::new(size, 0, &options)
                .map(|_| ())
                .unwrap_err()
                .to_string(),
            "a light level must be finite and positive, not NaN"
        );
    }

    /// One barcode copy's cells as the scene draws them, row by row.
    fn scene_cells(scene: &Scene, copy: CodeCopy) -> [[bool; CODE_COLS]; CODE_ROWS] {
        let mut cells = [[false; CODE_COLS]; CODE_ROWS];
        for rect in scene.rects() {
            if let Part::Cell(c, r, col) = rect.part
                && c == copy
            {
                cells[r][col] = rect.fill == Fill::Srgb([255; 3]);
            }
        }
        cells
    }

    #[test]
    fn the_barcode_carries_the_hdr_bit_and_the_run_tag() {
        let size = Size::new(1_920, 1_080).unwrap();
        for (patches, hdr) in [
            (
                Patches::Hdr {
                    sdr_white_nits: 240.0,
                    bright: false,
                },
                true,
            ),
            (
                Patches::ExpectedSdr {
                    curve: Curve::Natural,
                    peak_nits: 1_000.0,
                    sdr_white_nits: 240.0,
                },
                true,
            ),
            (Patches::Sdr, false),
        ] {
            let options = Options {
                patches,
                ..sdr(Pattern::Full, 60, 93, Load::Scroll)
            };
            let scene = Scene::new(size, 7, &options).unwrap();
            let a = scene_cells(&scene, CodeCopy::A);
            assert_eq!(a, scene_cells(&scene, CodeCopy::B));
            assert_eq!(a, barcode_cells(7, hdr, 93), "{patches:?}");
            let read = read_barcode(&levels(&a)).unwrap();
            assert_eq!((read.frame24, read.hdr, read.tag), (7, hdr, 93));
        }
        assert_ne!(barcode_cells(7, true, 93), barcode_cells(7, false, 93));
    }

    #[test]
    fn bad_options_are_refused() {
        assert_eq!(
            Size::new(0, 1080),
            Err(PatternError::Size {
                width: 0,
                height: 1080
            })
        );
        for (width, height) in [(MAX_SIDE + 1, 1_080), (1_920, u32::MAX)] {
            assert_eq!(
                Size::new(width, height),
                Err(PatternError::Size { width, height })
            );
        }
        assert!(Size::new(MAX_SIDE, MAX_SIDE).is_ok());
        assert_eq!(
            PatternError::Size {
                width: 20_000,
                height: 1_080
            }
            .to_string(),
            "the client must be 1x1 to 16384x16384 pixels, not 20000x1080"
        );
        let size = Size::new(64, 36).unwrap();
        let mut options = sdr(Pattern::Full, 0, 0, Load::Scroll);
        assert_eq!(
            Scene::new(size, 0, &options).map(|_| ()),
            Err(PatternError::Fps(0))
        );
        options.nominal_fps = 60;
        options.run_tag = 128;
        assert_eq!(
            Scene::new(size, 0, &options).map(|_| ()),
            Err(PatternError::Tag(128))
        );
        // A 1 × 1 client still makes a scene.
        options.run_tag = 0;
        let tiny = Scene::new(Size::new(1, 1).unwrap(), u64::MAX, &options).unwrap();
        assert_eq!(tiny.raster_srgb().unwrap().len(), 1);
    }
}
