// SPDX-License-Identifier: GPL-3.0-or-later
//! The testbed's `ready.json` (docs/research/16-verify-testbed/README.md §4.10), read once it
//! appears: the window to capture and where it is, checked against what the harness asked for.
//! It names the window handle and the adapter, so it stays on the PC (`C:\kairo\logs`).

use serde::Deserialize;

use crate::phases::exit;

/// The client area in desktop pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct Client {
    /// Left edge.
    pub left: i32,
    /// Top edge.
    pub top: i32,
    /// Width.
    pub width: i32,
    /// Height.
    pub height: i32,
}

/// What the ready file says.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Ready {
    /// The format's version: 1.
    pub kairo_testbed_ready: u32,
    /// The testbed's process id.
    pub pid: u32,
    /// The window handle.
    pub hwnd: isize,
    /// The client area.
    pub client: Client,
    /// The monitor's refresh rate, in mHz.
    pub refresh_mhz: u64,
    /// The adapter's LUID, `0x` and 16 hex digits.
    pub adapter_luid: String,
    /// `bgra8` or `fp16`.
    pub format: String,
    /// `borderless`, `windowed` or `exclusive`.
    pub mode: String,
    /// Frames per second.
    pub fps: u32,
}

/// What the harness expects of the testbed it started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expected<'a> {
    /// The child's process id.
    pub pid: u32,
    /// The LUID of the adapter that drives the monitor, as `kairo_display::Luid` prints it.
    pub luid: &'a str,
    /// The monitor's refresh rate, in mHz.
    pub refresh_mhz: u64,
    /// `borderless`.
    pub mode: &'a str,
    /// The testbed's rate.
    pub fps: u32,
}

/// Why a ready file is refused: the exit code and a reason with no PC detail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    /// The harness's exit code.
    pub code: u8,
    /// Why.
    pub reason: &'static str,
}

impl Ready {
    /// Reads a ready file's text.
    pub fn parse(text: &str) -> Result<Ready, Refusal> {
        serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|_| Refusal {
            code: exit::FAILED,
            reason: "the testbed's ready file doesn't read",
        })
    }

    /// Checks it against `expected`.
    pub fn check(&self, expected: &Expected<'_>) -> Result<(), Refusal> {
        let refuse = |code, reason| Err(Refusal { code, reason });
        if self.kairo_testbed_ready != 1 {
            return refuse(exit::FAILED, "the testbed's ready file is another version");
        }
        if self.pid != expected.pid {
            return refuse(exit::FAILED, "the ready file is another testbed's");
        }
        if !self.adapter_luid.eq_ignore_ascii_case(expected.luid) {
            return refuse(
                exit::READBACK,
                "the testbed drew on another adapter than the one that drives the monitor",
            );
        }
        if self.format != "bgra8" {
            return refuse(
                exit::REFUSED,
                "the testbed drew in HDR: the monitor is in HDR",
            );
        }
        if self.refresh_mhz != expected.refresh_mhz {
            return refuse(exit::REFUSED, "the monitor's refresh rate changed");
        }
        if self.mode != expected.mode || self.fps != expected.fps {
            return refuse(
                exit::INTERNAL,
                "the testbed runs another mode or rate than asked",
            );
        }
        if self.client.width <= 0 || self.client.height <= 0 {
            return refuse(exit::FAILED, "the testbed's window has no size");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str = r#"{"kairo_testbed_ready":1,"pid":4242,"hwnd":263330,"client":{"left":2560,"top":0,"width":1920,"height":1080},"refresh_mhz":179960,"adapter_luid":"0x00000000000abcde","dxgi_adapter_index":1,"dxgi_output_index":0,"format":"bgra8","mode":"borderless","fps":180,"start_hns":1234}"#;

    fn expected() -> Expected<'static> {
        Expected {
            pid: 4242,
            luid: "0x00000000000ABCDE",
            refresh_mhz: 179_960,
            mode: "borderless",
            fps: 180,
        }
    }

    #[test]
    fn a_ready_file_reads_and_checks() {
        let ready = Ready::parse(TEXT).unwrap();
        assert_eq!(ready.hwnd, 263_330);
        assert_eq!(
            ready.client,
            Client {
                left: 2560,
                top: 0,
                width: 1920,
                height: 1080
            }
        );
        assert_eq!(ready.check(&expected()), Ok(()));
        // With a byte-order mark, as some writers add.
        assert!(Ready::parse(&format!("\u{feff}{TEXT}")).is_ok());
    }

    #[test]
    fn each_mismatch_has_its_code() {
        let ready = Ready::parse(TEXT).unwrap();
        let code = |change: &dyn Fn(&mut Ready)| {
            let mut r = ready.clone();
            change(&mut r);
            r.check(&expected()).map_err(|e| e.code)
        };
        assert_eq!(code(&|r| r.kairo_testbed_ready = 2), Err(exit::FAILED));
        assert_eq!(code(&|r| r.pid = 1), Err(exit::FAILED));
        assert_eq!(
            code(&|r| r.adapter_luid = "0x0000000000000001".to_owned()),
            Err(exit::READBACK)
        );
        assert_eq!(code(&|r| r.format = "fp16".to_owned()), Err(exit::REFUSED));
        assert_eq!(code(&|r| r.refresh_mhz = 60_000), Err(exit::REFUSED));
        assert_eq!(
            code(&|r| r.mode = "windowed".to_owned()),
            Err(exit::INTERNAL)
        );
        assert_eq!(code(&|r| r.fps = 90), Err(exit::INTERNAL));
        assert_eq!(code(&|r| r.client.width = 0), Err(exit::FAILED));
        assert_eq!(Ready::parse("{").map_err(|e| e.code), Err(exit::FAILED));
    }

    #[test]
    fn reasons_name_no_pc_detail() {
        let ready = Ready::parse(TEXT).unwrap();
        let mut r = ready.clone();
        r.adapter_luid = "0x0000000000000001".to_owned();
        let reason = r.check(&expected()).unwrap_err().reason;
        assert!(
            !reason.contains("0x") && !reason.contains("abcde"),
            "{reason}"
        );
    }
}
