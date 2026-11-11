// SPDX-License-Identifier: GPL-3.0-or-later
//! What the capture's FFmpeg log must say (docs/research/16-verify-testbed/README.md §8.5 step
//! 2): the d3d11va device on the adapter that drives the monitor, shared by the capture and AMF;
//! NV12 BT.709 limited into `h264_amf`; and no option left unused. A missing line means nothing
//! the encoder did can be trusted (exit 4). Problems are fixed ids: the vendor:device and indices
//! they compare go to `harness.json`, which stays on the PC, never to a message.

use kairo_core::ffmpeg_cli::{UNUSED_OPTION, adapter_device, mapped_to, output_video_line};

/// One problem with the log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Problem {
    /// No `Selecting d3d11va adapter N` for the adapter asked for.
    NoAdapterSelected,
    /// No `Using device vvvv:dddd`: FFmpeg didn't find the adapter and fell back to the default.
    NoAdapterDevice,
    /// The adapter FFmpeg opened isn't the one that drives the monitor.
    AdapterMismatch,
    /// AMF didn't take the d3d11va device.
    NoAmfDevice,
    /// AMF didn't run on D3D11.
    NoAmfD3d11,
    /// `gfxcapture` made its own device instead of using the shared one.
    CaptureDeviceNotShared,
    /// No stream went to `h264_amf`.
    NotMappedToAmf,
    /// The output's video isn't NV12 BT.709 limited.
    NotNv12Bt709,
    /// An option no stream used.
    UnusedOption,
}

impl Problem {
    /// Its id in `harness.json`.
    pub fn id(self) -> &'static str {
        match self {
            Self::NoAdapterSelected => "no_adapter_selected",
            Self::NoAdapterDevice => "no_adapter_device",
            Self::AdapterMismatch => "adapter_mismatch",
            Self::NoAmfDevice => "no_amf_device",
            Self::NoAmfD3d11 => "no_amf_d3d11",
            Self::CaptureDeviceNotShared => "capture_device_not_shared",
            Self::NotMappedToAmf => "not_mapped_to_amf",
            Self::NotNv12Bt709 => "not_nv12_bt709",
            Self::UnusedOption => "unused_option",
        }
    }
}

/// What the log said.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LogCheck {
    /// What's wrong, in the order checked.
    pub problems: Vec<Problem>,
    /// The vendor:device FFmpeg named, lowercase: a PC detail, for `harness.json` only.
    pub log_device: Option<String>,
    /// AMF's lines about B-frames: recorded, never fatal at `bf=0` (README §7.2).
    pub bframe_warnings: usize,
    /// `gfxcapture` couldn't turn off the yellow border (Windows' borderless-capture
    /// permission): recorded, not fatal.
    pub border_warning: bool,
}

impl LogCheck {
    /// Whether the log says what it must.
    pub fn ok(&self) -> bool {
        self.problems.is_empty()
    }
}

/// Checks FFmpeg's `log` of a capture asked to use d3d11va adapter `adapter_index`, which must be
/// `device` (vendor:device, any case); `window`: `gfxcapture`, which must take the shared device
/// (`ddagrab` names none).
pub fn check(log: &str, adapter_index: u32, device: &str, window: bool) -> LogCheck {
    let mut problems = Vec::new();
    if !log.contains(&format!("Selecting d3d11va adapter {adapter_index}")) {
        problems.push(Problem::NoAdapterSelected);
    }
    let log_device = adapter_device(log);
    match &log_device {
        None => problems.push(Problem::NoAdapterDevice),
        Some(found) if !found.eq_ignore_ascii_case(device) => {
            problems.push(Problem::AdapterMismatch);
        }
        Some(_) => {}
    }
    if !log.contains("Using device kd (type d3d11va) with h264_amf encoder.") {
        problems.push(Problem::NoAmfDevice);
    }
    if !log.contains("AMF via D3D11.") {
        problems.push(Problem::NoAmfD3d11);
    }
    if window
        && (!log.contains("Using provided hw_device_ctx")
            || log.contains("Created internal hw_device_ctx"))
    {
        problems.push(Problem::CaptureDeviceNotShared);
    }
    if !mapped_to(log, "h264_amf") {
        problems.push(Problem::NotMappedToAmf);
    }
    if !output_video_line(log).is_some_and(|line| line.contains("nv12(tv, bt709")) {
        problems.push(Problem::NotNv12Bt709);
    }
    if log.contains(UNUSED_OPTION) {
        problems.push(Problem::UnusedOption);
    }
    let bframe_warnings = log
        .lines()
        .filter(|line| line.contains("[h264_amf @"))
        .filter(|line| {
            let lower = line.to_ascii_lowercase();
            lower.contains("b-frame") || lower.contains("bframe")
        })
        .count();
    LogCheck {
        problems,
        log_device,
        bframe_warnings,
        border_warning: log.contains("Failed setting border drawing mode"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A capture's log with every line it must have, made-up ids throughout.
    pub(crate) const GOOD: &str = "\
[AVHWDeviceContext @ 0x10] Selecting d3d11va adapter 1
[AVHWDeviceContext @ 0x10] Using device abcd:1234 (Test GPU).
[gfxcapture @ 0x20] Using provided hw_device_ctx
Stream mapping:
  setparams:default -> Stream #0:0 (h264_amf)
[h264_amf @ 0x30] Using device kd (type d3d11va) with h264_amf encoder.
[h264_amf @ 0x30] AMF via D3D11.
Output #0, mp4, to 'capture.mp4':
  Stream #0:0: Video: h264 (High) (avc1 / 0x31637661), nv12(tv, bt709, progressive), 1920x1080, q=2-31, 25000 kb/s, 60 fps
";

    #[test]
    fn a_good_log_has_no_problem() {
        let c = check(GOOD, 1, "ABCD:1234", true);
        assert!(c.ok(), "{c:?}");
        assert_eq!(c.log_device.as_deref(), Some("abcd:1234"));
        assert_eq!((c.bframe_warnings, c.border_warning), (0, false));
    }

    #[test]
    fn each_missing_line_is_its_own_problem() {
        let cases = [
            ("Selecting d3d11va adapter 1", Problem::NoAdapterSelected),
            (
                "Using device kd (type d3d11va) with h264_amf encoder.",
                Problem::NoAmfDevice,
            ),
            ("AMF via D3D11.", Problem::NoAmfD3d11),
            (
                "Using provided hw_device_ctx",
                Problem::CaptureDeviceNotShared,
            ),
            (
                "setparams:default -> Stream #0:0 (h264_amf)",
                Problem::NotMappedToAmf,
            ),
            ("nv12(tv, bt709", Problem::NotNv12Bt709),
        ];
        for (line, problem) in cases {
            let log = GOOD.replace(line, "");
            assert_eq!(
                check(&log, 1, "abcd:1234", true).problems,
                [problem],
                "{line}"
            );
        }
        // The adapter: a fallback names no vendor:device, another adapter the wrong one.
        let fallback = GOOD.replace("Using device abcd:1234 (Test GPU).", "");
        assert_eq!(
            check(&fallback, 1, "abcd:1234", true).problems,
            [Problem::NoAdapterDevice]
        );
        assert_eq!(
            check(GOOD, 1, "abcd:9999", true).problems,
            [Problem::AdapterMismatch]
        );
        assert_eq!(
            check(GOOD, 0, "abcd:1234", true).problems,
            [Problem::NoAdapterSelected]
        );
        // gfxcapture's own device; ddagrab names none and isn't asked to.
        let internal = GOOD.replace("Using provided", "Created internal");
        assert_eq!(
            check(&internal, 1, "abcd:1234", true).problems,
            [Problem::CaptureDeviceNotShared]
        );
        assert!(
            check(
                &internal.replace("Created internal hw_device_ctx", ""),
                1,
                "abcd:1234",
                false
            )
            .ok()
        );
        let unused = format!("{GOOD}Codec AVOption preset (x) has not been used for any stream.\n");
        assert_eq!(
            check(&unused, 1, "abcd:1234", true).problems,
            [Problem::UnusedOption]
        );
    }

    #[test]
    fn warnings_are_recorded_not_fatal() {
        let log = format!(
            "{GOOD}[h264_amf @ 0x30] B-Frames are not supported by this encoder\n\
             [gfxcapture @ 0x20] Failed setting border drawing mode\n"
        );
        let c = check(&log, 1, "abcd:1234", true);
        assert!(c.ok());
        assert_eq!((c.bframe_warnings, c.border_warning), (1, true));
    }

    #[test]
    fn problem_ids_name_no_device() {
        for p in [
            Problem::NoAdapterSelected,
            Problem::NoAdapterDevice,
            Problem::AdapterMismatch,
            Problem::NoAmfDevice,
            Problem::NoAmfD3d11,
            Problem::CaptureDeviceNotShared,
            Problem::NotMappedToAmf,
            Problem::NotNv12Bt709,
            Problem::UnusedOption,
        ] {
            assert!(
                p.id()
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b == b'_' || b.is_ascii_digit()),
                "{}",
                p.id()
            );
        }
    }
}
