// SPDX-License-Identifier: GPL-3.0-or-later
//! Decoded pictures, in presentation order (docs/research/16-verify-testbed/README.md §3.3,
//! architecture.md §2.3).
//!
//! A [`Decoder`] hands each picture to a callback as borrowed 8-bit 4:2:0 planes, so nothing is
//! copied per frame. Timing never comes from a decoder: the checks take every picture's time
//! from Kairo's own sample tables, and `decode.clean` compares the counts. The backends:
//! [`synthetic::Synthetic`] renders pattern v1 straight from `kairo_core::testsig` (tests and
//! `synth`); [`cli::Cli`] (feature `dev-tools`) spawns an FFmpeg it is given and reads raw
//! frames from its pipe; the linked backend comes with research 16's PR 6.

#[cfg(feature = "dev-tools")]
pub mod cli;
pub mod synthetic;

use std::fmt;
use std::ops::ControlFlow;

/// One picture's planes: 8-bit Y′CbCr 4:2:0, borrowed.
#[derive(Debug, Clone, Copy)]
pub struct Planes<'a> {
    width: usize,
    height: usize,
    y: &'a [u8],
    y_stride: usize,
    cb: &'a [u8],
    cr: &'a [u8],
    c_stride: usize,
}

impl<'a> Planes<'a> {
    /// The planes of a `width` × `height` picture, or an error if a plane is too short for its
    /// stride. Chroma is ⌈width/2⌉ × ⌈height/2⌉.
    pub fn new(
        (width, height): (usize, usize),
        (y, y_stride): (&'a [u8], usize),
        (cb, cr, c_stride): (&'a [u8], &'a [u8], usize),
    ) -> Result<Self, DecodeError> {
        let (cw, ch) = (width.div_ceil(2), height.div_ceil(2));
        let fits = |plane: &[u8], stride: usize, w: usize, h: usize| {
            stride >= w
                && h.checked_sub(1)
                    .map_or(Some(0), |rows| rows.checked_mul(stride)?.checked_add(w))
                    .is_some_and(|need| plane.len() >= need)
        };
        if width == 0
            || height == 0
            || !fits(y, y_stride, width, height)
            || !fits(cb, c_stride, cw, ch)
            || !fits(cr, c_stride, cw, ch)
        {
            return Err(DecodeError::Planes { width, height });
        }
        Ok(Self {
            width,
            height,
            y,
            y_stride,
            cb,
            cr,
            c_stride,
        })
    }

    /// The width in pixels.
    pub fn width(&self) -> usize {
        self.width
    }

    /// The height in pixels.
    pub fn height(&self) -> usize {
        self.height
    }

    /// Row `row` of the luma plane, `width` samples.
    pub fn y_row(&self, row: usize) -> &'a [u8] {
        let start = row * self.y_stride;
        &self.y[start..start + self.width]
    }

    /// Row `row` of the Cb plane, ⌈width/2⌉ samples.
    pub fn cb_row(&self, row: usize) -> &'a [u8] {
        let start = row * self.c_stride;
        &self.cb[start..start + self.width.div_ceil(2)]
    }

    /// Row `row` of the Cr plane, ⌈width/2⌉ samples.
    pub fn cr_row(&self, row: usize) -> &'a [u8] {
        let start = row * self.c_stride;
        &self.cr[start..start + self.width.div_ceil(2)]
    }

    /// The mean luma of the rectangle `x0..x1` × `y0..y1`, clipped to the picture; `None` if
    /// nothing is left.
    pub fn luma_mean(&self, (x0, y0): (i64, i64), (x1, y1): (i64, i64)) -> Option<f64> {
        let clip = |v: i64, max: usize| v.clamp(0, max as i64) as usize;
        let (x0, x1) = (clip(x0, self.width), clip(x1, self.width));
        let (y0, y1) = (clip(y0, self.height), clip(y1, self.height));
        if x0 >= x1 || y0 >= y1 {
            return None;
        }
        let sum: u64 = (y0..y1)
            .map(|row| {
                self.y_row(row)[x0..x1]
                    .iter()
                    .map(|&v| u64::from(v))
                    .sum::<u64>()
            })
            .sum();
        Some(sum as f64 / ((x1 - x0) * (y1 - y0)) as f64)
    }
}

/// A picture that owns its planes, tightly packed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picture {
    /// The width in pixels.
    pub width: usize,
    /// The height in pixels.
    pub height: usize,
    /// Luma, `width` × `height`.
    pub y: Vec<u8>,
    /// Cb, ⌈width/2⌉ × ⌈height/2⌉.
    pub cb: Vec<u8>,
    /// Cr, ⌈width/2⌉ × ⌈height/2⌉.
    pub cr: Vec<u8>,
}

impl Picture {
    /// Its planes.
    pub fn planes(&self) -> Result<Planes<'_>, DecodeError> {
        Planes::new(
            (self.width, self.height),
            (&self.y, self.width),
            (&self.cb, &self.cr, self.width.div_ceil(2)),
        )
    }

    /// A packed yuv420p frame's size in bytes, as FFmpeg's rawvideo writes it.
    pub fn packed_len(width: usize, height: usize) -> usize {
        width * height + 2 * width.div_ceil(2) * height.div_ceil(2)
    }

    /// Splits one packed yuv420p frame into planes, or `None` if `bytes` isn't one frame long.
    pub fn from_packed(width: usize, height: usize, bytes: &[u8]) -> Option<Self> {
        if bytes.len() != Self::packed_len(width, height) {
            return None;
        }
        let chroma = width.div_ceil(2) * height.div_ceil(2);
        let (y, rest) = bytes.split_at(width * height);
        let (cb, cr) = rest.split_at(chroma);
        Some(Self {
            width,
            height,
            y: y.to_vec(),
            cb: cb.to_vec(),
            cr: cr.to_vec(),
        })
    }
}

/// What a decode saw.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DecodeStats {
    /// Pictures handed to the callback.
    pub frames: usize,
    /// Decode errors the decoder reported (it went on where it could).
    pub errors: usize,
    /// The last error, in a few words, with no path.
    pub last_error: Option<String>,
}

/// Why a decode couldn't run or finish.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    /// The decoder couldn't start, e.g. the FFmpeg program is missing.
    Start(String),
    /// Reading its output failed part way.
    Read(String),
    /// A picture's planes don't fit its size.
    Planes {
        /// The width it claimed.
        width: usize,
        /// The height it claimed.
        height: usize,
    },
    /// The decoder can't decode this clip, e.g. a picture size it doesn't know.
    Unsupported(String),
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Start(what) => write!(f, "the decoder didn't start: {what}"),
            Self::Read(what) => write!(f, "reading decoded frames failed: {what}"),
            Self::Planes { width, height } => {
                write!(f, "a decoded picture's planes don't fit {width}x{height}")
            }
            Self::Unsupported(what) => write!(f, "can't decode this clip: {what}"),
        }
    }
}

impl std::error::Error for DecodeError {}

/// A source of decoded pictures.
pub trait Decoder {
    /// Its kind for the report: `synthetic`, `cli` or `linked`.
    fn kind(&self) -> &'static str;

    /// The licence of a decoder that is spawned rather than linked, for the report (README §2
    /// item 18): `GPL` or `LGPL`, if it says.
    fn licence(&mut self) -> Option<&'static str> {
        None
    }

    /// Decodes the clip's H.264 track, handing each picture, in presentation order, with its
    /// index to `each`, which may stop the decode by returning [`ControlFlow::Break`].
    fn video(
        &mut self,
        each: &mut dyn FnMut(usize, &Planes<'_>) -> ControlFlow<()>,
    ) -> Result<DecodeStats, DecodeError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn planes_check_their_sizes() {
        let picture = Picture {
            width: 5,
            height: 3,
            y: (0..15).collect(),
            cb: vec![128; 6],
            cr: vec![128; 6],
        };
        let planes = picture.planes().unwrap();
        assert_eq!(planes.y_row(2), &[10, 11, 12, 13, 14]);
        assert_eq!(planes.cb_row(1).len(), 3);
        assert_eq!(planes.luma_mean((1, 1), (3, 3)), Some(9.0));
        // Clipped to the picture; nothing left is None.
        assert_eq!(planes.luma_mean((-5, 0), (1, 1)), Some(0.0));
        assert_eq!(planes.luma_mean((5, 0), (9, 3)), None);
        // Short planes and a zero size are refused.
        assert!(Planes::new((5, 3), (&picture.y[..14], 5), (&picture.cb, &picture.cr, 3)).is_err());
        assert!(Planes::new((5, 3), (&picture.y, 5), (&picture.cb[..5], &picture.cr, 3)).is_err());
        assert!(Planes::new((0, 3), (&picture.y, 5), (&picture.cb, &picture.cr, 3)).is_err());
        assert!(Planes::new((5, 3), (&picture.y, 4), (&picture.cb, &picture.cr, 3)).is_err());
    }

    #[test]
    fn packed_frames_split_into_planes() {
        assert_eq!(Picture::packed_len(5, 3), 15 + 2 * 6);
        let bytes: Vec<u8> = (0..27).collect();
        let picture = Picture::from_packed(5, 3, &bytes).unwrap();
        assert_eq!(picture.cb, (15..21).collect::<Vec<u8>>());
        assert_eq!(picture.cr, (21..27).collect::<Vec<u8>>());
        assert!(Picture::from_packed(5, 3, &bytes[..26]).is_none());
    }
}
