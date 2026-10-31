// SPDX-License-Identifier: GPL-3.0-or-later
//! The flash (docs/research/16-verify-testbed/README.md §2 item 8, §6.3 `content.flash`): lit
//! while `N mod F < round(0.1·F)`. Each readable frame's flash patch, its central 50%, is lit
//! when brighter than the frame's barcode midpoint, and must be lit exactly when its testbed
//! frame number says so.
//!
//! The numbers are unwrapped from the barcode's 24 bits, starting from the first one read, so
//! this assumes the testbed hasn't passed 2^24 frames (12.9 h at 360 fps, 25.9 h at 180 fps):
//! past that, `N mod F` would be off by `2^24 mod F`. The testbed log's frame count will settle
//! it once the log is read (README §6.3).

use kairo_core::testsig;

use super::Reading;

/// What the flash patch showed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Flashes {
    /// Readable frames compared.
    pub frames: usize,
    /// Of those, lit.
    pub lit: usize,
    /// Onsets: a lit frame after an unlit one.
    pub onsets: usize,
    /// Frames lit when they shouldn't be, or dark when they should be lit: (output frame,
    /// testbed frame, lit).
    pub wrong: Vec<(usize, i64, bool)>,
}

/// Judges each readable frame's flash against `flash_on(N, source_fps)`.
pub fn flashes(readings: &[Reading], numbers: &[Option<i64>], source_fps: u32) -> Flashes {
    let mut out = Flashes::default();
    let mut previous: Option<bool> = None;
    for (index, (reading, number)) in readings.iter().zip(numbers).enumerate() {
        let (Some(n), Some(luma)) = (number, reading.flash_luma) else {
            previous = None;
            continue;
        };
        let lit = luma > reading.midpoint();
        out.frames += 1;
        if lit {
            out.lit += 1;
            if previous == Some(false) {
                out.onsets += 1;
            }
        }
        previous = Some(lit);
        let expected = u64::try_from(*n).is_ok_and(|n| testsig::flash_on(n, source_fps));
        if lit != expected {
            out.wrong.push((index, *n, lit));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use kairo_core::testsig::Barcode;

    fn reading(luma: f64) -> Reading {
        Reading {
            code: Some(Barcode {
                frame24: 0,
                hdr: false,
                tag: 0,
                margin: 1.0,
                white: 235.0,
                black: 16.0,
            }),
            torn: None,
            error: None,
            flash_luma: Some(luma),
        }
    }

    #[test]
    fn frames_must_be_lit_exactly_when_their_number_says() {
        // 180 fps: lit for N mod 180 < 18. Output frames show N = 3k.
        let numbers: Vec<Option<i64>> = (0..70).map(|k| Some(3 * k)).collect();
        let mut readings: Vec<Reading> = numbers
            .iter()
            .map(|n| reading(if n.unwrap() % 180 < 18 { 235.0 } else { 16.0 }))
            .collect();
        let good = flashes(&readings, &numbers, 180);
        assert_eq!((good.frames, good.lit, good.onsets), (70, 12, 1));
        assert!(good.wrong.is_empty());
        // One flash frame dark, one dark frame lit.
        readings[2] = reading(16.0);
        readings[30] = reading(200.0);
        let bad = flashes(&readings, &numbers, 180);
        assert_eq!(bad.wrong, [(2, 6, false), (30, 90, true)]);
        // Unreadable frames aren't judged.
        let partial = flashes(&readings, &[None; 70], 180);
        assert_eq!(partial.frames, 0);
    }
}
