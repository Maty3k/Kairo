// SPDX-License-Identifier: GPL-3.0-or-later
//! The grey patches (docs/research/16-verify-testbed/README.md §6.3 `content.levels` and
//! `hdr.curve_levels`; signals.md §4): the central 50% mean luma of each patch wholly in the
//! picture against the level its grey should come out at.

use kairo_core::testsig::{
    self, Curve, Load, Options, PATCH_SIZE_U, Part, Patches, Pattern, SDR_PATCH_CODES, Scene, Size,
};

use super::Window;
use crate::decode::Planes;

/// What the patches should show.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Expected {
    /// SDR codes: Y = 16 + 219 × code / 255.
    Sdr,
    /// HDR greys after `curve`, from a display peaking at `peak_nits` with SDR white at
    /// `white_nits` (signals.md §4.2).
    Hdr {
        /// The tone-mapping curve.
        curve: Curve,
        /// The display's peak.
        peak_nits: f64,
        /// The OS's SDR white.
        white_nits: f64,
    },
}

impl Expected {
    /// Patch `k`'s expected luma code.
    pub fn luma(&self, k: usize) -> Option<f64> {
        match *self {
            Self::Sdr => SDR_PATCH_CODES
                .get(k)
                .map(|&c| 16.0 + 219.0 * f64::from(c) / 255.0),
            Self::Hdr {
                curve,
                peak_nits,
                white_nits,
            } => testsig::sdr_luma(
                testsig::hdr_patch_nits(k, white_nits),
                curve,
                peak_nits,
                white_nits,
            )
            .ok(),
        }
    }
}

/// One patch on one frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Patch {
    /// Which patch, 0 to 11.
    pub k: usize,
    /// The level it should show.
    pub expected: f64,
    /// The level it shows.
    pub measured: f64,
}

/// The patches wholly in frame `frame`'s picture, measured. The row drifts 1 u a frame and wraps,
/// so which patches are whole changes from frame to frame.
pub fn patches(planes: &Planes<'_>, frame: u64, expected: &Expected) -> Vec<Patch> {
    let (Ok(w), Ok(h)) = (
        u32::try_from(planes.width()),
        u32::try_from(planes.height()),
    ) else {
        return Vec::new();
    };
    let Ok(size) = Size::new(w, h) else {
        return Vec::new();
    };
    // Where the patches are doesn't depend on what they show.
    let options = Options {
        pattern: Pattern::Full,
        nominal_fps: 60,
        run_tag: 0,
        load: Load::None,
        patches: Patches::Sdr,
    };
    let Ok(scene) = Scene::new(size, frame, &options) else {
        return Vec::new();
    };
    let whole = testsig::px_y(PATCH_SIZE_U, h) - 1;
    scene
        .rects()
        .iter()
        .filter_map(|rect| {
            let Part::Patch(k) = rect.part else {
                return None;
            };
            if rect.x1 - rect.x0 < whole {
                return None;
            }
            Some(Patch {
                k,
                expected: expected.luma(k)?,
                measured: Window::central(rect).mean(planes)?,
            })
        })
        .collect()
}

/// Whether a patch is within `tolerance` codes of its level. With `one_sided_top` (HDR patches,
/// PLAN:430), one at the top of the range (235) only has to reach it, since nothing above it is
/// wrong for that patch (signals.md §4.3); SDR patches are judged both ways.
pub fn within(patch: &Patch, tolerance: f64, one_sided_top: bool) -> bool {
    if one_sided_top && patch.expected >= 235.0 - 1e-9 {
        patch.measured >= patch.expected - tolerance
    } else {
        (patch.measured - patch.expected).abs() <= tolerance
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decode::synthetic::{Damage, Synthetic};

    fn measured(patches: Patches, expected: Expected, frame: u64) -> Vec<Patch> {
        let options = Options {
            pattern: Pattern::Full,
            nominal_fps: 180,
            run_tag: 0,
            load: Load::Scroll,
            patches,
        };
        let synthetic = Synthetic::new(
            Size::new(640, 360).unwrap(),
            options,
            vec![frame],
            Damage::default(),
        );
        let picture = synthetic.frame(0).unwrap();
        self::patches(&picture.planes().unwrap(), frame, &expected)
    }

    #[test]
    fn sdr_patches_measure_their_codes() {
        for frame in [0, 400, 1_919] {
            let found = measured(Patches::Sdr, Expected::Sdr, frame);
            // A 640-wide picture holds 3 to 4 whole patches; the row moves, so they change.
            assert!(!found.is_empty(), "frame {frame}");
            for p in &found {
                assert!(
                    (p.measured - p.expected).abs() < 1.0,
                    "frame {frame}: {p:?}"
                );
                assert!(within(p, 2.0, false));
            }
        }
    }

    #[test]
    fn hdr_patches_measure_their_curve() {
        let curve = Curve::Natural;
        let expected = Expected::Hdr {
            curve,
            peak_nits: 1_000.0,
            white_nits: 240.0,
        };
        // signals.md §4.2's table: Natural at W = 240 nits is Y 215.
        assert!((expected.luma(7).unwrap() - 215.0).abs() < 0.5);
        let shown = Patches::ExpectedSdr {
            curve,
            peak_nits: 1_000.0,
            sdr_white_nits: 240.0,
        };
        let mut seen = 0;
        for frame in (0..1_920).step_by(160) {
            for p in measured(shown, expected, frame) {
                assert!(within(&p, 2.0, true), "frame {frame}: {p:?}");
                seen += 1;
            }
            // The other curve is told apart above SDR's middle greys.
            let other = Expected::Hdr {
                curve: Curve::MatchSdrApps,
                peak_nits: 1_000.0,
                white_nits: 240.0,
            };
            for p in measured(shown, other, frame) {
                if p.k == 7 {
                    assert!(!within(&p, 2.0, true), "frame {frame}: {p:?}");
                }
            }
        }
        assert!(seen > 20);
    }

    #[test]
    fn the_top_of_the_range_is_one_sided_for_hdr_only() {
        let top = |measured| Patch {
            k: 11,
            expected: 235.0,
            measured,
        };
        assert!(within(&top(240.0), 2.0, true));
        assert!(within(&top(233.5), 2.0, true));
        assert!(!within(&top(232.0), 2.0, true));
        // SDR's ±2 holds both ways (README §6.3 content.levels).
        assert!(!within(&top(240.0), 2.0, false));
        assert!(within(&top(236.5), 2.0, false));
        let mid = |measured| Patch {
            k: 5,
            expected: 150.0,
            measured,
        };
        for hdr in [false, true] {
            assert!(within(&mid(151.9), 2.0, hdr) && !within(&mid(152.5), 2.0, hdr));
        }
    }
}
