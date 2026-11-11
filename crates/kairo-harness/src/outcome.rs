// SPDX-License-Identifier: GPL-3.0-or-later
//! What a child's exit means for the harness (docs/research/16-verify-testbed/README.md §8.3,
//! §8.9): each component's exit code becomes the harness's exit code and a fixed reason, which
//! names no PC detail.

use serde_json::Value;

use crate::phases::exit;

/// The harness's exit code and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// The exit code.
    pub code: u8,
    /// Why, in plain words.
    pub reason: &'static str,
}

impl Outcome {
    /// An outcome.
    pub const fn new(code: u8, reason: &'static str) -> Outcome {
        Outcome { code, reason }
    }
}

/// Windows' "the program couldn't load a DLL it imports" (STATUS_DLL_NOT_FOUND).
pub const DLL_NOT_FOUND: i32 = 0xC000_0135_u32 as i32;

/// The testbed ended (`code`, `None` if killed) before writing its ready file.
pub fn testbed_before_ready(code: Option<i32>) -> Outcome {
    match code {
        Some(3) => Outcome::new(exit::REFUSED, "the testbed refused the monitor or its rate"),
        Some(2) => Outcome::new(exit::INTERNAL, "the testbed refused its command line"),
        Some(4) => Outcome::new(exit::FAILED, "the testbed's device or swap chain failed"),
        Some(5) => Outcome::new(exit::FAILED, "the testbed didn't get the foreground"),
        Some(DLL_NOT_FOUND) => Outcome::new(exit::FAILED, "the testbed couldn't load a DLL"),
        _ => Outcome::new(exit::FAILED, "the testbed ended before it was ready"),
    }
}

/// The testbed's log ended with `reason` during the warm-up or the capture.
pub fn testbed_ended(reason: &str) -> Outcome {
    match reason {
        "hdr_changed" => Outcome::new(exit::REFUSED, "the monitor's HDR changed"),
        _ => Outcome::new(exit::FAILED, "the testbed ended during the run"),
    }
}

/// kairo-verify exited with `code` (`None`: killed at its limit), leaving `report` if it wrote
/// one. Its 0 (pass) and 1 (fail) are reports; so is 3 when the report it wrote says `error`.
pub fn verify(code: Option<i32>, report: Option<&Value>) -> Outcome {
    let has_report =
        report.is_some_and(|r| r.get("kairo_verify").and_then(Value::as_u64) == Some(1));
    match code {
        Some(0 | 1) if has_report => Outcome::new(exit::OK, "the clip was judged"),
        Some(3)
            if has_report
                && report
                    .and_then(|r| r.get("verdict"))
                    .and_then(Value::as_str)
                    == Some("error") =>
        {
            Outcome::new(exit::OK, "the clip was judged: a check couldn't run")
        }
        Some(0 | 1 | 3) => Outcome::new(exit::FAILED, "kairo-verify wrote no report"),
        Some(2) => Outcome::new(
            exit::FAILED,
            "kairo-verify refused its command line: older than research 16's PR 8c?",
        ),
        Some(4) => Outcome::new(exit::FAILED, "kairo-verify couldn't write its report"),
        Some(DLL_NOT_FOUND) => Outcome::new(
            exit::FAILED,
            "kairo-verify couldn't load FFmpeg's DLLs: were they deployed (PR 6b)?",
        ),
        Some(_) => Outcome::new(exit::FAILED, "kairo-verify failed"),
        None => Outcome::new(exit::FAILED, "kairo-verify ran past its limit"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_testbeds_exits() {
        assert_eq!(testbed_before_ready(Some(3)).code, exit::REFUSED);
        assert_eq!(testbed_before_ready(Some(2)).code, exit::INTERNAL);
        for code in [
            Some(0),
            Some(1),
            Some(4),
            Some(5),
            Some(DLL_NOT_FOUND),
            None,
        ] {
            assert_eq!(testbed_before_ready(code).code, exit::FAILED, "{code:?}");
        }
        assert_eq!(testbed_ended("hdr_changed").code, exit::REFUSED);
        assert_eq!(testbed_ended("error").code, exit::FAILED);
    }

    #[test]
    fn kairo_verifys_exits() {
        let pass = json!({"kairo_verify": 1, "verdict": "pass"});
        let error = json!({"kairo_verify": 1, "verdict": "error"});
        assert_eq!(verify(Some(0), Some(&pass)).code, exit::OK);
        assert_eq!(verify(Some(1), Some(&pass)).code, exit::OK);
        assert_eq!(verify(Some(3), Some(&error)).code, exit::OK);
        assert_eq!(verify(Some(3), Some(&pass)).code, exit::FAILED);
        assert_eq!(verify(Some(0), None).code, exit::FAILED);
        assert_eq!(verify(Some(3), None).code, exit::FAILED);
        for code in [Some(2), Some(4), Some(DLL_NOT_FOUND), Some(-1), None] {
            assert_eq!(verify(code, Some(&pass)).code, exit::FAILED, "{code:?}");
        }
        assert!(verify(Some(DLL_NOT_FOUND), None).reason.contains("6b"));
    }
}
