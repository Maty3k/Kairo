// SPDX-License-Identifier: GPL-3.0-or-later
//! Pattern v1 straight from `kairo_core::testsig`, converted to BT.709 limited 4:2:0 the way
//! `synth` will hand it to an encoder, with optional damage for the tests: noise, scaling from
//! another size, garbled and torn barcodes (architecture.md §2.3). Audio tracks hold testsig's
//! beeps and cue at exact times, with optional noise.

use std::ops::ControlFlow;

use kairo_core::testsig::{
    self, CodeCopy, Fill, Matrix, Options, Part, PatternError, Range, Scene, Size,
};

use super::{DecodeError, DecodeStats, Decoder, Picture, Planes};

/// Pattern v1 as limited-range BT.709 4:2:0: Y and full-resolution Cb and Cr from the sRGB
/// raster, then chroma subsampled with `chroma_sample_loc_type` 0 (README §4.5).
pub fn render(scene: &Scene) -> Result<Picture, PatternError> {
    render_as(scene, Matrix::Bt709, Range::Limited)
}

/// [`render`] with another matrix or range, for the colour defects.
fn render_as(scene: &Scene, matrix: Matrix, range: Range) -> Result<Picture, PatternError> {
    let size = scene.size();
    let (w, h) = (size.width() as usize, size.height() as usize);
    let raster = scene.raster_srgb()?;
    let (luma, cb, cr) = planes_of(&raster, matrix, range);
    let round = |v: f64| v.round().clamp(0.0, 255.0) as u8;
    Ok(Picture {
        width: w,
        height: h,
        y: luma.into_iter().map(round).collect(),
        cb: testsig::subsample_chroma(&cb, w, h)?
            .into_iter()
            .map(round)
            .collect(),
        cr: testsig::subsample_chroma(&cr, w, h)?
            .into_iter()
            .map(round)
            .collect(),
    })
}

/// Full-resolution Y, Cb and Cr of an sRGB raster, unrounded.
fn planes_of(raster: &[[u8; 3]], matrix: Matrix, range: Range) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    let mut planes = (
        Vec::with_capacity(raster.len()),
        Vec::with_capacity(raster.len()),
        Vec::with_capacity(raster.len()),
    );
    for rgb in raster {
        let [y, cb, cr] = testsig::ycbcr(rgb.map(f64::from), matrix, range);
        planes.0.push(y);
        planes.1.push(cb);
        planes.2.push(cr);
    }
    planes
}

/// Bilinear resampling of a `from_w` × `from_h` plane to `to_w` × `to_h`, pixel centres
/// aligned, as a scaler with no sharpening would do.
fn resample(
    plane: &[f64],
    (from_w, from_h): (usize, usize),
    (to_w, to_h): (usize, usize),
) -> Vec<f64> {
    let at = |x: usize, y: usize| plane[y * from_w + x];
    let source = |dst: usize, from: usize, to: usize| {
        let s = ((dst as f64 + 0.5) * from as f64 / to as f64 - 0.5).clamp(0.0, (from - 1) as f64);
        let i = s.floor() as usize;
        (i, (i + 1).min(from - 1), s - i as f64)
    };
    let mut out = Vec::with_capacity(to_w * to_h);
    for y in 0..to_h {
        let (y0, y1, fy) = source(y, from_h, to_h);
        for x in 0..to_w {
            let (x0, x1, fx) = source(x, from_w, to_w);
            let top = at(x0, y0) * (1.0 - fx) + at(x1, y0) * fx;
            let bottom = at(x0, y1) * (1.0 - fx) + at(x1, y1) * fx;
            out.push(top * (1.0 - fy) + bottom * fy);
        }
    }
    out
}

/// What the synthetic frames get wrong on purpose.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Damage {
    /// Uniform noise of up to ± this many codes on every luma sample, from a fixed seed.
    pub noise: u8,
    /// Render at this size and scale to the clip's, bilinearly.
    pub scale_from: Option<Size>,
    /// Output frames whose barcodes are garbled: both copies' data rows inverted.
    pub garble: Vec<usize>,
    /// Output frames torn between two testbed frames: barcode B shows the frame 3 later.
    pub tear: Vec<usize>,
    /// A horizontal shift of the whole picture by this many pixels (under one), linearly
    /// interpolated, as a scaler's sub-pixel phase would give.
    pub shift: f64,
    /// Luma rounded to multiples of this many codes, as coarse quantisation would leave it.
    pub quantise: u8,
    /// Another matrix or range than BT.709 limited (signals.md §2.4's colour faults).
    pub colour: Option<(Matrix, Range)>,
    /// Cb and Cr swapped.
    pub swap_chroma: bool,
    /// 1 px luma lines every this many pixels, both ways, inside the bars band, this many codes
    /// up (README §7.3's `bars` and `bars-sparse` defects).
    pub gridlines: Option<(usize, i16)>,
    /// Only these output frames get the gridlines; all of them when empty.
    pub gridline_frames: Vec<usize>,
    /// Cb this many codes off grey inside the ramp band, as a low bitrate can leave it.
    pub ramp_chroma: i16,
}

/// A decoder that renders pattern v1: output frame `i` shows testbed frame `content[i]`.
#[derive(Debug, Clone)]
pub struct Synthetic {
    size: Size,
    options: Options,
    content: Vec<u64>,
    damage: Damage,
    audio: Vec<SyntheticAudio>,
}

/// What sounds in a synthetic audio track.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Sound {
    /// One of testsig's beeps, at this frequency.
    Beep(f64),
    /// testsig's placeholder cue.
    Cue,
}

/// A synthetic audio track at 48 kHz: sounds at given times and optional noise, the same on
/// every channel.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SyntheticAudio {
    /// Its length, in sample frames.
    pub frames: usize,
    /// Each sound: what, when it starts in seconds (any time, between samples included), and
    /// its gain in dB on testsig's level.
    pub sounds: Vec<(Sound, f64, f64)>,
    /// Gaussian white noise at this RMS, in dBFS, from a fixed seed.
    pub noise_db: Option<f64>,
}

impl SyntheticAudio {
    /// One sample per frame.
    pub fn render(&self) -> Vec<f32> {
        let rate = crate::analyze::audio::RATE;
        let mut x = vec![0.0_f64; self.frames];
        for &(sound, at, gain_db) in &self.sounds {
            let gain = 10_f64.powf(gain_db / 20.0);
            let ms = match sound {
                Sound::Beep(_) => testsig::BEEP_MS,
                Sound::Cue => testsig::CUE_MS,
            };
            let first = (at * rate).floor().max(0.0) as usize;
            let last = ((((at + f64::from(ms) / 1_000.0) * rate).ceil().max(0.0)) as usize + 1)
                .min(self.frames);
            for (n, v) in x.iter_mut().enumerate().take(last).skip(first) {
                let t = n as f64 / rate - at;
                *v += gain
                    * match sound {
                        Sound::Beep(hz) => testsig::tone_burst_at(hz, ms, t),
                        Sound::Cue => testsig::cue_placeholder_at(t),
                    };
            }
        }
        if let Some(db) = self.noise_db {
            // Box–Muller from SplitMix64.
            let sigma = 10_f64.powf(db / 20.0);
            let mut state = 0x4B41_4952_4F4E_4F49_u64;
            let mut uniform = || {
                state = splitmix(state);
                ((state >> 11) as f64 + 0.5) / (1_u64 << 53) as f64
            };
            for v in &mut x {
                let (u, w) = (uniform(), uniform());
                *v += sigma * (-2.0 * u.ln()).sqrt() * (std::f64::consts::TAU * w).cos();
            }
        }
        x.into_iter().map(|v| v as f32).collect()
    }
}

impl Synthetic {
    /// Frames of `size` showing `content` with these pattern `options`.
    pub fn new(size: Size, options: Options, content: Vec<u64>, damage: Damage) -> Self {
        Self {
            size,
            options,
            content,
            damage,
            audio: Vec::new(),
        }
    }

    /// The same, with these audio tracks, in file order.
    pub fn with_audio(mut self, audio: Vec<SyntheticAudio>) -> Self {
        self.audio = audio;
        self
    }

    /// Output frame `index`.
    pub fn frame(&self, index: usize) -> Result<Picture, PatternError> {
        let frame = self.content.get(index).copied().unwrap_or(0);
        let mut picture = match self.damage.scale_from {
            Some(from) if from != self.size => {
                let scene = Scene::new(from, frame, &self.options)?;
                let (fw, fh) = (from.width() as usize, from.height() as usize);
                let (w, h) = (self.size.width() as usize, self.size.height() as usize);
                let (matrix, range) = self
                    .damage
                    .colour
                    .unwrap_or((Matrix::Bt709, Range::Limited));
                let (luma, cb, cr) = planes_of(&scene.raster_srgb()?, matrix, range);
                let round = |v: f64| v.round().clamp(0.0, 255.0) as u8;
                let chroma = |plane: Vec<f64>| -> Result<Vec<u8>, PatternError> {
                    let scaled = resample(&plane, (fw, fh), (w, h));
                    Ok(testsig::subsample_chroma(&scaled, w, h)?
                        .into_iter()
                        .map(round)
                        .collect())
                };
                Picture {
                    width: w,
                    height: h,
                    y: resample(&luma, (fw, fh), (w, h))
                        .into_iter()
                        .map(round)
                        .collect(),
                    cb: chroma(cb)?,
                    cr: chroma(cr)?,
                }
            }
            _ => {
                let (matrix, range) = self
                    .damage
                    .colour
                    .unwrap_or((Matrix::Bt709, Range::Limited));
                render_as(&Scene::new(self.size, frame, &self.options)?, matrix, range)?
            }
        };
        let scene = Scene::new(self.size, frame, &self.options)?;
        if self.damage.swap_chroma {
            std::mem::swap(&mut picture.cb, &mut picture.cr);
        }
        let gridded =
            self.damage.gridline_frames.is_empty() || self.damage.gridline_frames.contains(&index);
        if let Some((step, delta)) = self.damage.gridlines.filter(|_| gridded) {
            let h = self.size.height();
            let top = testsig::px_y(testsig::BARS_U.0, h).max(0) as usize;
            let bottom = (testsig::px_y(testsig::BARS_U.1, h).max(0) as usize).min(picture.height);
            let w = picture.width;
            for y in top..bottom {
                for x in 0..w {
                    if step > 0 && (y % step == 0 || x % step == 0) {
                        let v = &mut picture.y[y * w + x];
                        *v = (i16::from(*v) + delta).clamp(0, 255) as u8;
                    }
                }
            }
        }
        if self.damage.ramp_chroma != 0 {
            // Chroma rows are half the luma's: the ramp band's, in chroma rows.
            let h = self.size.height();
            let cw = picture.width.div_ceil(2);
            let top = (testsig::px_y(testsig::RAMP_U.0, h).max(0) as usize) / 2;
            let bottom = ((testsig::px_y(testsig::RAMP_U.1, h).max(0) as usize) / 2)
                .min(picture.height.div_ceil(2));
            for v in &mut picture.cb[top * cw..bottom * cw] {
                *v = (i16::from(*v) + self.damage.ramp_chroma).clamp(0, 255) as u8;
            }
        }
        if self.damage.garble.contains(&index) {
            paint_cells(
                &mut picture,
                &scene,
                |_, row, _, white| {
                    if row == 0 { white } else { !white }
                },
            );
        }
        if self.damage.tear.contains(&index) {
            let later =
                testsig::barcode_cells(frame + 3, scene_hdr(&self.options), self.options.run_tag);
            paint_cells(&mut picture, &scene, |copy, row, col, white| {
                if copy == CodeCopy::B {
                    later[row][col]
                } else {
                    white
                }
            });
        }
        if self.damage.shift != 0.0 {
            let (w, cw) = (picture.width, picture.width.div_ceil(2));
            shift_rows(&mut picture.y, w, self.damage.shift);
            shift_rows(&mut picture.cb, cw, self.damage.shift / 2.0);
            shift_rows(&mut picture.cr, cw, self.damage.shift / 2.0);
        }
        if self.damage.quantise > 1 {
            let step = f64::from(self.damage.quantise);
            for v in &mut picture.y {
                *v = ((f64::from(*v) / step).round() * step).clamp(0.0, 255.0) as u8;
            }
        }
        if self.damage.noise > 0 {
            let amplitude = u64::from(self.damage.noise);
            let mut state = 0x4B56_0000_0000_0001_u64 ^ frame;
            for v in &mut picture.y {
                state = splitmix(state);
                let offset = (state % (2 * amplitude + 1)) as i64 - amplitude as i64;
                *v = (i64::from(*v) + offset).clamp(0, 255) as u8;
            }
        }
        Ok(picture)
    }
}

/// Moves every row of a plane `width` wide right by `shift` pixels, linearly interpolated, with
/// the edges repeated.
fn shift_rows(plane: &mut [u8], width: usize, shift: f64) {
    let mut row = vec![0.0; width];
    for line in plane.chunks_exact_mut(width) {
        for (x, out) in row.iter_mut().enumerate() {
            let source = (x as f64 - shift).clamp(0.0, (width - 1) as f64);
            let (i, f) = (source.floor() as usize, source.fract());
            let next = (i + 1).min(width - 1);
            *out = f64::from(line[i]) * (1.0 - f) + f64::from(line[next]) * f;
        }
        for (v, &r) in line.iter_mut().zip(&row) {
            *v = r.round().clamp(0.0, 255.0) as u8;
        }
    }
}

/// The barcode's HDR bit for these options, as `Scene` sets it.
fn scene_hdr(options: &Options) -> bool {
    !matches!(options.patches, testsig::Patches::Sdr)
}

/// Repaints every barcode cell of `scene` in `picture` as black or white, as `decide` says from
/// its copy, row, column and current colour.
fn paint_cells(
    picture: &mut Picture,
    scene: &Scene,
    decide: impl Fn(CodeCopy, usize, usize, bool) -> bool,
) {
    let w = picture.width;
    for rect in scene.rects() {
        let (Part::Cell(copy, row, col), Fill::Srgb(codes)) = (rect.part, rect.fill) else {
            continue;
        };
        let white = decide(copy, row, col, codes[0] > 127);
        let level = if white { 235 } else { 16 };
        for y in rect.y0.max(0) as usize..(rect.y1.max(0) as usize).min(picture.height) {
            let row = &mut picture.y[y * w..(y + 1) * w];
            for v in &mut row[(rect.x0.max(0) as usize).min(w)..(rect.x1.max(0) as usize).min(w)] {
                *v = level;
            }
        }
    }
}

/// SplitMix64's step.
fn splitmix(state: u64) -> u64 {
    let mut z = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

impl Decoder for Synthetic {
    fn kind(&self) -> &'static str {
        "synthetic"
    }

    fn video(
        &mut self,
        each: &mut dyn FnMut(usize, &Planes<'_>) -> ControlFlow<()>,
    ) -> Result<DecodeStats, DecodeError> {
        let mut stats = DecodeStats::default();
        for index in 0..self.content.len() {
            let picture = self
                .frame(index)
                .map_err(|e| DecodeError::Unsupported(e.to_string()))?;
            stats.frames += 1;
            if each(index, &picture.planes()?).is_break() {
                break;
            }
        }
        Ok(stats)
    }

    fn audio(
        &mut self,
        track: usize,
        channels: usize,
        each: &mut dyn FnMut(&[f32]) -> ControlFlow<()>,
    ) -> Result<DecodeStats, DecodeError> {
        let Some(spec) = self.audio.get(track) else {
            return Err(DecodeError::Unsupported(format!("no audio track {track}")));
        };
        let channels = channels.max(1);
        let mut stats = DecodeStats::default();
        let mut chunk = Vec::with_capacity(4_096 * channels);
        for block in spec.render().chunks(4_096) {
            chunk.clear();
            for &v in block {
                chunk.extend(std::iter::repeat_n(v, channels));
            }
            stats.frames += block.len();
            if each(&chunk).is_break() {
                break;
            }
        }
        Ok(stats)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kairo_core::testsig::{Load, Patches, Pattern};

    fn options() -> Options {
        Options {
            pattern: Pattern::Full,
            nominal_fps: 180,
            run_tag: 5,
            load: Load::Scroll,
            patches: Patches::Sdr,
        }
    }

    #[test]
    fn renders_bt709_limited_with_exact_black_and_white() {
        let size = Size::new(320, 180).unwrap();
        let picture = render(&Scene::new(size, 0, &options()).unwrap()).unwrap();
        // Outside every element the background is black: Y 16, Cb = Cr = 128.
        assert_eq!(picture.y[0], 16);
        assert_eq!((picture.cb[0], picture.cr[0]), (128, 128));
        // The flash is lit on frame 0: Y 235 in its middle.
        let (cx, cy) = (160, (448 + 96) * 180 / 1080);
        assert_eq!(picture.y[cy * 320 + cx], 235);
        assert_eq!(picture.planes().unwrap().width(), 320);
    }

    #[test]
    fn damage_is_deterministic_and_bounded() {
        let size = Size::new(320, 180).unwrap();
        let clean = Synthetic::new(size, options(), vec![7], Damage::default());
        let noisy = Synthetic::new(
            size,
            options(),
            vec![7],
            Damage {
                noise: 3,
                ..Damage::default()
            },
        );
        let (a, b) = (clean.frame(0).unwrap(), noisy.frame(0).unwrap());
        assert_eq!(noisy.frame(0).unwrap(), b);
        assert!(a.y.iter().zip(&b.y).all(|(x, y)| x.abs_diff(*y) <= 3));
        assert_ne!(a.y, b.y);
        // Scaling from another size keeps the size asked for.
        let scaled = Synthetic::new(
            size,
            options(),
            vec![7],
            Damage {
                scale_from: Some(Size::new(480, 270).unwrap()),
                ..Damage::default()
            },
        );
        let picture = scaled.frame(0).unwrap();
        assert_eq!(
            (picture.width, picture.height, picture.cb.len()),
            (320, 180, 160 * 90)
        );
    }
}
