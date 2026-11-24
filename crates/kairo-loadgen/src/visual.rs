// SPDX-License-Identifier: GPL-3.0-or-later
//! The picture (docs/research/17-loadgen-bench/README.md §3.5, §3.11): the CPU reference of
//! `visual()` in `loadgen.hlsl`, and its photosensitivity contract as tests.
//!
//! The picture is a dim, slowly drifting colour field with faint grey grain: sRGB codes 58 to 95,
//! a colour cycle every 20 s, and value noise on a 32 px lattice that scrolls at 120 px/s. It is a
//! pure function of the pixel and the frame's time ([`FrameConstants`]), so the ramp, the run and
//! its end draw the same picture and nothing in it is a step in time. Both workloads add a term of
//! at most ±0.5 code that depends only on the pixel, the count and the seed, which keeps the
//! compiler from removing their work and never changes from frame to frame ([`WORK_TERM`]).
//!
//! The reference follows the shader operation for operation in `f32`, so Windows CI's WARP test
//! can hold the GPU's read-back to it within ±2 codes (research 17 §6, W tier; transcendentals
//! differ between WARP and the CPU). Its tests hold the picture to Xbox Accessibility Guideline
//! 118's thresholds at half or less (README §3.11) over a grid and a full colour cycle:
//!
//! - its relative luminance spans less than 0.08 of white;
//! - within any 1/3 s, no pixel changes by 0.05 or more and the screen average by 0.005 or more;
//! - the red ratio R ÷ (R + G + B), on linear values, stays below 0.5;
//! - no window of 20% of the screen holds a luminance difference of 0.08 or more.

use std::f32::consts::TAU;

use crate::workload::FrameConstants;

/// `mix32`'s multipliers: lowbias32's, from Chris Wellons' hash-prospector (public domain).
pub const MIX_MULTIPLIERS: [u32; 2] = [0x7feb_352d, 0x846c_a68b];
/// The salt `hash01` mixes into a cell's y coordinate.
pub const HASH_SALT: u32 = 0x9e37_79b9;
/// The grain's lattice: 32 px cells.
pub const CELL_PX: f32 = 32.0;
/// The lattice wraps every 256 cells (8,192 px), where the scroll wraps too.
pub const CELL_MASK: u32 = 255;
/// The colour field's middle, in sRGB: code 76.5.
pub const BASE: f32 = 0.30;
/// Its swing either side: 12.75 codes.
pub const AMPLITUDE: f32 = 0.05;
/// Green's phase behind red: the f32 nearest 2π/3, which the HLSL writes `2.0943951`.
pub const PHASE_GREEN: f32 = 2.094_395_2;
/// Blue's: the f32 nearest 4π/3, which the HLSL writes `4.1887902`.
pub const PHASE_BLUE: f32 = 4.188_790_3;
/// The grain's full range: 12 codes, ±6.
pub const GRAIN: f32 = 12.0 / 255.0;
/// The largest work term either workload adds: half a code.
pub const WORK_TERM: f32 = 0.5 / 255.0;

/// Two xorshift-multiply rounds (lowbias32): every input bit flips each output bit about half the
/// time. The shaders hash pixels, cells and texel addresses with it.
pub fn mix32(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(MIX_MULTIPLIERS[0]);
    x ^= x >> 15;
    x = x.wrapping_mul(MIX_MULTIPLIERS[1]);
    x ^= x >> 16;
    x
}

/// A cell's value in [0, 1), from the top 24 bits of its hash (exact in `f32`).
pub fn hash01(cell: [u32; 2]) -> f32 {
    (mix32(cell[0] ^ mix32(cell[1] ^ HASH_SALT)) >> 8) as f32 * (1.0 / 16_777_216.0)
}

/// HLSL's `lerp`.
fn lerp(x: f32, y: f32, s: f32) -> f32 {
    x + s * (y - x)
}

/// Smoothstep-interpolated value noise at `p`, in [0, 1): the grain.
pub fn value_noise(p: [f32; 2]) -> f32 {
    let g = [p[0] / CELL_PX, p[1] / CELL_PX];
    let f = [g[0] - g[0].floor(), g[1] - g[1].floor()];
    let s = f.map(|f| f * f * (3.0 - 2.0 * f));
    let c = [
        (g[0].floor() as u32) & CELL_MASK,
        (g[1].floor() as u32) & CELL_MASK,
    ];
    let right = (c[0] + 1) & CELL_MASK;
    let below = (c[1] + 1) & CELL_MASK;
    let a = hash01(c);
    let b = hash01([right, c[1]]);
    let d = hash01([c[0], below]);
    let e = hash01([right, below]);
    lerp(lerp(a, b, s[0]), lerp(d, e, s[0]), s[1])
}

/// What one frame's picture depends on: the constant buffer's drift, scroll and size, and whether
/// the shaders were compiled with the grain (`--detail`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Picture {
    /// The colour drift, in cycles.
    pub drift: f32,
    /// The grain's scroll, in pixels.
    pub scroll_px: f32,
    /// 1 ÷ the target's width and height.
    pub inv_size: [f32; 2],
    /// The grain (`KAIRO_DETAIL` 1).
    pub detail: bool,
}

impl Picture {
    /// The picture a frame with `constants` draws.
    pub fn new(constants: &FrameConstants, detail: bool) -> Picture {
        Picture {
            drift: constants.drift,
            scroll_px: constants.scroll_px,
            inv_size: constants.inv_size,
            detail,
        }
    }

    /// The picture at `t_hns` after the anchor on a `size` target.
    pub fn at(t_hns: i64, size: (u32, u32), detail: bool) -> Picture {
        Picture::new(&FrameConstants::new(t_hns, 0, None, size), detail)
    }

    /// The colour at `px`, an `SV_Position` (a pixel's centre is x + 0.5, y + 0.5): the sRGB
    /// values the shader writes into the UNORM target, before the workload's work term.
    pub fn colour(&self, px: [f32; 2]) -> [f32; 3] {
        let uv = [px[0] * self.inv_size[0], px[1] * self.inv_size[1]];
        let a = TAU * (self.drift + 0.5 * uv[0] + 0.25 * uv[1]);
        let field = [
            BASE + AMPLITUDE * a.cos(),
            BASE + AMPLITUDE * (a + PHASE_GREEN).cos(),
            BASE + AMPLITUDE * (a + PHASE_BLUE).cos(),
        ];
        let grain = if self.detail {
            (value_noise([px[0] + self.scroll_px, px[1]]) - 0.5) * GRAIN
        } else {
            0.0
        };
        field.map(|c| c + grain)
    }

    /// The colour of pixel (`x`, `y`), at its centre.
    pub fn pixel(&self, x: u32, y: u32) -> [f32; 3] {
        self.colour([x as f32 + 0.5, y as f32 + 0.5])
    }

    /// Pixel (`x`, `y`) as 8-bit codes, red first (the swap chain's BGRA bytes are blue first).
    pub fn codes(&self, x: u32, y: u32) -> [u8; 3] {
        self.pixel(x, y).map(to_unorm8)
    }
}

/// A float in [0, 1] as an 8-bit UNORM code, rounded to nearest.
pub fn to_unorm8(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// An sRGB value as linear light (IEC 61966-2-1).
pub fn srgb_to_linear(value: f64) -> f64 {
    if value <= 0.040_45 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

/// The relative luminance of sRGB values, as a share of white (BT.709 weights on linear light).
pub fn luminance(rgb: [f32; 3]) -> f64 {
    let [r, g, b] = rgb.map(|c| srgb_to_linear(f64::from(c)));
    0.212_6 * r + 0.715_2 * g + 0.072_2 * b
}

/// XAG 118's red ratio R ÷ (R + G + B), on linear values.
pub fn red_ratio(rgb: [f32; 3]) -> f64 {
    let [r, g, b] = rgb.map(|c| srgb_to_linear(f64::from(c)));
    if r + g + b > 0.0 {
        r / (r + g + b)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workload::{DRIFT_PERIOD_HNS, HLSL};
    use kairo_core::timeline::HNS_PER_SECOND;

    #[test]
    fn mix32_avalanches() {
        // Flipping any input bit flips each output bit with probability ½ (lowbias32's bias is
        // about 0.0017); 4,096 inputs from a Weyl sequence give a sampling σ of about 0.008.
        const INPUTS: u32 = 4_096;
        let mut worst: f64 = 0.0;
        for bit in 0..32 {
            let mut flips = [0_u32; 32];
            for k in 0..INPUTS {
                let x = k.wrapping_mul(0x9e37_79b9) ^ 0x5bd1_e995;
                let diff = mix32(x) ^ mix32(x ^ (1 << bit));
                for (out, count) in flips.iter_mut().enumerate() {
                    *count += (diff >> out) & 1;
                }
            }
            for count in flips {
                let p = f64::from(count) / f64::from(INPUTS);
                worst = worst.max((p - 0.5).abs());
            }
        }
        assert!(worst < 0.05, "worst flip bias {worst}");
        // An xorshift-multiply hash fixes 0 and is a bijection.
        assert_eq!(mix32(0), 0);
        let mut seen: Vec<u32> = (0..65_536).map(mix32).collect();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), 65_536);
    }

    #[test]
    fn the_shader_carries_the_references_constants() {
        // The same numbers in loadgen.hlsl, so the GPU draws what this module describes.
        for text in [
            "x ^= x >> 16; x *= 0x7feb352du; x ^= x >> 15; x *= 0x846ca68bu; x ^= x >> 16;",
            "mix32(c.x ^ mix32(c.y ^ 0x9e3779b9u)) >> 8) * (1.0 / 16777216.0)",
            "float2 g = p / 32.0;",
            "& 255u",
            "f * f * (3.0 - 2.0 * f)",
            "6.2831853 * (drift + 0.5 * uv.x + 0.25 * uv.y)",
            "0.30 + 0.05 * cos(a + float3(0.0, 2.0943951, 4.1887902))",
            "value_noise(float2(px.x + scroll_px, px.y)) - 0.5) * (12.0 / 255.0)",
        ] {
            assert!(HLSL.contains(text), "loadgen.hlsl lacks {text:?}");
        }
        assert_eq!(MIX_MULTIPLIERS, [0x7feb_352d, 0x846c_a68b]);
        assert_eq!(HASH_SALT, 0x9e37_79b9);
        // The HLSL's literals are these f32s.
        assert_eq!("6.2831853".parse::<f32>().unwrap(), TAU);
        assert_eq!("2.0943951".parse::<f32>().unwrap(), PHASE_GREEN);
        assert_eq!("4.1887902".parse::<f32>().unwrap(), PHASE_BLUE);
        assert_eq!("0.30".parse::<f32>().unwrap(), BASE);
    }

    #[test]
    fn the_grain_is_value_noise_that_wraps_every_8192_px() {
        let mut low = f32::MAX;
        let mut high = f32::MIN;
        for y in (0..2_200).step_by(7) {
            for x in (0..8_192).step_by(61) {
                let p = [x as f32 + 0.5, y as f32 + 0.5];
                let v = value_noise(p);
                assert!((0.0..1.0).contains(&v), "{p:?}: {v}");
                assert_eq!(v, value_noise([p[0] + 8_192.0, p[1]]), "{p:?}");
                assert_eq!(v, value_noise([p[0], p[1] + 8_192.0]), "{p:?}");
                low = low.min(v);
                high = high.max(v);
            }
        }
        assert!(
            low < 0.05 && high > 0.95,
            "{low}..{high}: the grain uses its range"
        );
        // Continuous across a cell's edge.
        for y in [3.5_f32, 100.5, 1_000.5] {
            let left = value_noise([32.0 * 7.0 - 1e-3, y]);
            let right = value_noise([32.0 * 7.0 + 1e-3, y]);
            assert!((left - right).abs() < 1e-3, "{left} {right}");
        }
        for k in 0..1_000 {
            let v = hash01([k, k.wrapping_mul(31)]);
            assert!((0.0..1.0).contains(&v));
        }
    }

    #[test]
    fn the_picture_stays_within_its_codes() {
        for detail in [true, false] {
            let (mut low, mut high) = (u8::MAX, u8::MIN);
            for second in 0..20 {
                let picture = Picture::at(second * HNS_PER_SECOND, (2_560, 1_440), detail);
                for y in (0..1_440).step_by(37) {
                    for x in (0..2_560).step_by(37) {
                        for code in picture.codes(x, y) {
                            low = low.min(code);
                            high = high.max(code);
                        }
                    }
                }
            }
            // Codes 64–89 for the drift, 58–95 with the grain (README §3.11).
            let (floor, ceiling) = if detail { (58, 95) } else { (64, 89) };
            assert!(
                low >= floor && high <= ceiling,
                "detail {detail}: {low}..{high}"
            );
        }
    }

    /// The contract's limits (README §3.11): half of XAG 118's, or tighter.
    const SPAN_LIMIT: f64 = 0.08;
    const PIXEL_CHANGE_LIMIT: f64 = 0.05;
    const AVERAGE_CHANGE_LIMIT: f64 = 0.005;
    const RED_LIMIT: f64 = 0.5;
    const WINDOW_LIMIT: f64 = 0.08;

    /// The largest values a sweep found.
    #[derive(Debug, Default)]
    struct Worst {
        span: f64,
        pixel_change: f64,
        average_change: f64,
        red: f64,
        window: f64,
    }

    /// Samples the picture of a `width` × `height` target on a 48 × 27 grid every 1/12 s over
    /// a full 20 s colour cycle and 1/3 s more. A change within 1/3 s is the largest difference
    /// over five consecutive samples. Luminance and red bounds take the work term either way.
    fn sweep(width: u32, height: u32, detail: bool) -> Worst {
        const GX: usize = 48;
        const GY: usize = 27;
        const PER_SECOND: i64 = 12;
        const WINDOW: usize = 5; // samples spanning 1/3 s
        let mut worst = Worst::default();
        let (mut low, mut high) = (f64::MAX, f64::MIN);
        let mut history: Vec<[f64; WINDOW]> = vec![[0.0; WINDOW]; GX * GY];
        let mut averages = [0.0; WINDOW];
        let samples = 20 * PER_SECOND + 4;
        for k in 0..=samples {
            let picture = Picture::at(k * HNS_PER_SECOND / PER_SECOND, (width, height), detail);
            let slot = k as usize % WINDOW;
            let mut sum = 0.0;
            let mut grid_low = [[0.0; GX]; GY];
            let mut grid_high = [[0.0; GX]; GY];
            for gy in 0..GY {
                for gx in 0..GX {
                    let x = (gx as u32 * width) / GX as u32;
                    let y = (gy as u32 * height) / GY as u32;
                    let rgb = picture.pixel(x, y);
                    let y_mid = luminance(rgb);
                    let y_low = luminance(rgb.map(|c| c - WORK_TERM));
                    let y_high = luminance(rgb.map(|c| c + WORK_TERM));
                    low = low.min(y_low);
                    high = high.max(y_high);
                    grid_low[gy][gx] = y_low;
                    grid_high[gy][gx] = y_high;
                    worst.red = worst
                        .red
                        .max(red_ratio(rgb.map(|c| c - WORK_TERM)))
                        .max(red_ratio(rgb.map(|c| c + WORK_TERM)));
                    let past = &mut history[gy * GX + gx];
                    past[slot] = y_mid;
                    if k as usize >= WINDOW - 1 {
                        let (a, b) = past
                            .iter()
                            .fold((f64::MAX, f64::MIN), |(a, b), &v| (a.min(v), b.max(v)));
                        worst.pixel_change = worst.pixel_change.max(b - a);
                    }
                    sum += y_mid;
                }
            }
            averages[slot] = sum / (GX * GY) as f64;
            if k as usize >= WINDOW - 1 {
                let (a, b) = averages
                    .iter()
                    .fold((f64::MAX, f64::MIN), |(a, b), &v| (a.min(v), b.max(v)));
                worst.average_change = worst.average_change.max(b - a);
            }
            // Windows of a fifth of the screen, three shapes, every second.
            if k % PER_SECOND == 0 {
                for (wx, wy) in [(22, 12), (GX, 6), (10, GY)] {
                    for oy in 0..=GY - wy {
                        for ox in 0..=GX - wx {
                            let mut a = f64::MAX;
                            let mut b = f64::MIN;
                            for gy in oy..oy + wy {
                                for gx in ox..ox + wx {
                                    a = a.min(grid_low[gy][gx]);
                                    b = b.max(grid_high[gy][gx]);
                                }
                            }
                            worst.window = worst.window.max(b - a);
                        }
                    }
                }
            }
        }
        worst.span = high - low;
        worst
    }

    #[test]
    fn the_picture_keeps_the_photosensitivity_limits() {
        // The 180 Hz monitor's likely size, a 4K screen, and Windows CI's WARP target.
        for (width, height) in [(2_560, 1_440), (3_840, 2_160), (320, 180)] {
            for detail in [true, false] {
                let worst = sweep(width, height, detail);
                let at = format!("{width}x{height} detail {detail}: {worst:?}");
                assert!(worst.span < SPAN_LIMIT, "{at}");
                assert!(worst.pixel_change < PIXEL_CHANGE_LIMIT, "{at}");
                assert!(worst.average_change < AVERAGE_CHANGE_LIMIT, "{at}");
                assert!(worst.red < RED_LIMIT, "{at}");
                assert!(worst.window < WINDOW_LIMIT, "{at}");
                // The sweep reaches the figures research 17 §3.11 computed, so it isn't blind:
                // a span of 5.2% with the grain (5.4% with the work term) and 3.1% without it,
                // 0.023 at a pixel when the grain passes, and a red ratio of 0.46 at most.
                let (span, pixel) = if detail { (0.05, 0.02) } else { (0.03, 0.001) };
                assert!(worst.span > span && worst.pixel_change > pixel, "{at}");
                assert!(worst.red > 0.44 && worst.average_change > 0.0005, "{at}");
            }
        }
    }

    #[test]
    fn consecutive_frames_change_by_less_than_a_code() {
        // At 125 fps the drift moves a fraction of a code per frame; the grain moves about one
        // pixel, by less than a code where it changes most.
        let frame = HNS_PER_SECOND / 125;
        for detail in [false, true] {
            let limit = if detail { 1.0 } else { 0.1 };
            let mut largest: f32 = 0.0;
            for k in (0..2_500).step_by(7) {
                let t = k * frame;
                let a = Picture::at(t, (2_560, 1_440), detail);
                let b = Picture::at(t + frame, (2_560, 1_440), detail);
                for y in (0..1_440).step_by(53) {
                    for x in (0..2_560).step_by(53) {
                        let (p, q) = (a.pixel(x, y), b.pixel(x, y));
                        for i in 0..3 {
                            largest = largest.max((p[i] - q[i]).abs() * 255.0);
                        }
                    }
                }
            }
            assert!(largest < limit, "detail {detail}: {largest} codes");
        }
    }

    #[test]
    fn drift_and_scroll_wrap_seamlessly() {
        // Across the drift's wrap (every 20 s) and the scroll's (8,192 px at 120 px/s, every
        // 1,024/15 s), at the start and nearly a week into a run (118 of their common periods of
        // 5,120 s), the picture moves by a sliver of a code, never a jump.
        let cycles = 118 * 5_120 * HNS_PER_SECOND;
        for wrap in [
            DRIFT_PERIOD_HNS,
            682_666_667,
            cycles + DRIFT_PERIOD_HNS,
            cycles + 682_666_667,
        ] {
            let before = Picture::at(wrap - 10_000, (2_560, 1_440), true);
            let after = Picture::at(wrap + 10_000, (2_560, 1_440), true);
            let mut largest: f32 = 0.0;
            for y in (0..1_440).step_by(31) {
                for x in (0..2_560).step_by(31) {
                    let (p, q) = (before.pixel(x, y), after.pixel(x, y));
                    for i in 0..3 {
                        largest = largest.max((p[i] - q[i]).abs() * 255.0);
                    }
                }
            }
            assert!(largest < 0.25, "at {wrap}: {largest} codes");
        }
        // So nearly a week in, the picture is exactly the one at the start.
        let start = Picture::at(3 * HNS_PER_SECOND, (2_560, 1_440), true);
        let later = Picture::at(3 * HNS_PER_SECOND + cycles, (2_560, 1_440), true);
        assert_eq!(start, later);
        assert_eq!(start.pixel(1_234, 567), later.pixel(1_234, 567));
    }

    #[test]
    fn red_ratio_and_luminance_follow_srgb() {
        assert!((luminance([1.0, 1.0, 1.0]) - 1.0).abs() < 1e-12);
        assert_eq!(luminance([0.0, 0.0, 0.0]), 0.0);
        assert!((red_ratio([1.0, 0.0, 0.0]) - 1.0).abs() < 1e-12);
        assert!((red_ratio([0.5, 0.5, 0.5]) - 1.0 / 3.0).abs() < 1e-12);
        assert_eq!(red_ratio([0.0, 0.0, 0.0]), 0.0);
        assert!((srgb_to_linear(0.5) - 0.214_041).abs() < 1e-6);
        assert_eq!(to_unorm8(-0.1), 0);
        assert_eq!(to_unorm8(1.5), 255);
        assert_eq!(to_unorm8(76.5 / 255.0), 77);
    }
}
