// SPDX-License-Identifier: GPL-3.0-or-later
//! `kairo-display`: the monitor by its device path, the adapter by its LUID
//! (docs/research/16-verify-testbed/architecture.md §1.5, README §4.2; research 08 §7.2–7.3).
//!
//! M1's testbed, harness, loadgen and `wgc-probe` all have to put a window on a chosen monitor
//! and create their D3D11 device on the adapter that drives it (M1 contract: "never primary";
//! the adapter by its LUID, not assumed to be adapter 0). This crate does that once:
//! `targets()` (Windows) lists every active display with its `monitorDevicePath` (the one id that
//! survives reboots and re-plugs), GDI name, HMONITOR, rectangle, refresh rate, HDR state,
//! whether it mirrors another display, and the LUID of the adapter to create its device on;
//! `adapter_for_luid()` turns a LUID into the DXGI adapter index and PCI ids.
//!
//! The DXGI index is valid only in the process that asked: DXGI may order adapters
//! differently per process (unverified), so a program passes its own index on (to FFmpeg, for
//! example) and confirms the adapter from the other side's log.
//!
//! Friendly names and device paths are details of the owner's PC: they belong in the local
//! `displays.json` (`.dogfood/` on the Mac), never in a log that might be shared (CLAUDE.md).
//! The types and their tests are pure and build everywhere; the Win32 and DXGI calls are in
//! the Windows-only `platform` module.

#![deny(unsafe_code)]

use std::fmt;

use serde::{Deserialize, Serialize};

#[cfg(windows)]
#[allow(unsafe_code)]
mod platform;

#[cfg(windows)]
pub use platform::{adapter_for_luid, adapters, make_dpi_aware, targets};

/// An adapter's locally unique id, as DXGI and the display configuration API report it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct Luid {
    /// The high 32 bits.
    pub high: i32,
    /// The low 32 bits.
    pub low: u32,
}

impl Luid {
    /// The 64-bit value, high part first.
    pub fn value(self) -> u64 {
        (u64::from(self.high as u32) << 32) | u64::from(self.low)
    }
}

impl fmt::Display for Luid {
    /// `0x` and 16 hex digits, the way logs print it (M1 contract: "logs the LUID").
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "0x{:016x}", self.value())
    }
}

/// A refresh rate as the display configuration reports it: an exact fraction of Hz.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Refresh {
    /// The numerator.
    pub numerator: u32,
    /// The denominator; 0 means unknown.
    pub denominator: u32,
}

impl Refresh {
    /// The rate in mHz, rounded to nearest (the testbed log's `refresh_mhz`), or `None` if
    /// unknown.
    pub fn millihertz(self) -> Option<u64> {
        (self.denominator > 0 && self.numerator > 0).then(|| {
            let (n, d) = (u64::from(self.numerator), u64::from(self.denominator));
            (2 * 1_000 * n + d) / (2 * d)
        })
    }

    /// The rate in Hz.
    pub fn hertz(self) -> Option<f64> {
        (self.denominator > 0).then(|| f64::from(self.numerator) / f64::from(self.denominator))
    }

    /// Whether `fps` frames a second divide this rate evenly within 0.5%: one frame every k
    /// vblanks for some k from 1 to 4, the SyncInterval range (README §4.3).
    pub fn fits_vsync(self, fps: u32) -> Option<u32> {
        let mhz = self.millihertz()?;
        (1..=4_u32).find(|&k| {
            let want = u64::from(fps) * u64::from(k) * 1_000;
            want.abs_diff(mhz) * 200 <= mhz
        })
    }
}

/// A rectangle in desktop pixels (physical pixels when the process is per-monitor-v2 DPI
/// aware; see `make_dpi_aware()`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Rect {
    /// Left edge.
    pub left: i32,
    /// Top edge.
    pub top: i32,
    /// Right edge (exclusive).
    pub right: i32,
    /// Bottom edge (exclusive).
    pub bottom: i32,
}

impl Rect {
    /// Width in pixels (0 if inverted).
    pub fn width(&self) -> u32 {
        u32::try_from(i64::from(self.right) - i64::from(self.left)).unwrap_or(0)
    }

    /// Height in pixels (0 if inverted).
    pub fn height(&self) -> u32 {
        u32::try_from(i64::from(self.bottom) - i64::from(self.top)).unwrap_or(0)
    }
}

/// The colour mode Windows 11 24H2 and later report (`DISPLAYCONFIG_ADVANCED_COLOR_MODE`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColorMode {
    /// Plain SDR.
    Sdr,
    /// SDR with Advanced Colour (wide colour gamut, ACM).
    Wcg,
    /// HDR.
    Hdr,
}

impl ColorMode {
    /// The mode for the API's value, if it is one of the three.
    pub fn from_raw(value: i32) -> Option<ColorMode> {
        match value {
            0 => Some(ColorMode::Sdr),
            1 => Some(ColorMode::Wcg),
            2 => Some(ColorMode::Hdr),
            _ => None,
        }
    }
}

/// A display's HDR state (research 08 §7.3).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct HdrState {
    /// Advanced colour is supported (the legacy bit, Windows 10 1709 and later).
    pub supported: bool,
    /// DXGI says the output is in HDR: its colour space is PQ with BT.2020 primaries.
    pub active: bool,
    /// 24H2 and later: SDR, WCG or HDR. `None` before 24H2, where only `active` tells.
    pub mode: Option<ColorMode>,
    /// The SDR white level in nits (`SDRWhiteLevel` / 1000 × 80).
    pub sdr_white_nits: f64,
    /// DXGI's maximum luminance in nits.
    pub max_luminance_nits: f64,
    /// Bits per colour channel.
    pub bits_per_color: u32,
}

/// `SDRWhiteLevel` (thousandths of 80 nits) in nits.
pub fn sdr_white_nits(level: u32) -> f64 {
    f64::from(level) / 1_000.0 * 80.0
}

/// An active display.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DisplayTarget {
    /// `monitorDevicePath`: the stable id to keep (in settings, jobs and `monitors.json`).
    pub device_path: String,
    /// The GDI name, e.g. `\\.\DISPLAY1`; it can change across reboots and re-plugs.
    pub gdi_name: String,
    /// The monitor's EDID name. Local files only.
    pub friendly_name: String,
    /// The HMONITOR's raw value, valid while the display configuration stays the same.
    pub hmonitor: isize,
    /// The monitor's rectangle on the desktop.
    pub rect: Rect,
    /// Whether it is the primary monitor (reported, never used to choose one).
    pub primary: bool,
    /// The refresh rate.
    pub refresh: Refresh,
    /// Whether another active display shows the same picture (Windows' Duplicate mode): a
    /// window meant for this one would appear on both, so the testbed and harness refuse it.
    pub cloned: bool,
    /// The LUID of the adapter DXGI lists its output under in this process, the one to create
    /// the device on; on a PC without Microsoft Hybrid graphics, the adapter that scans it out.
    pub adapter_luid: Luid,
    /// Its HDR state.
    pub hdr: HdrState,
}

/// A DXGI adapter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdapterInfo {
    /// Its LUID.
    pub luid: Luid,
    /// Its index in this process's `IDXGIFactory1::EnumAdapters1` order.
    pub dxgi_index: u32,
    /// The PCI vendor id (0x1002 AMD, 0x10DE NVIDIA, 0x8086 Intel, 0x1414 Microsoft).
    pub vendor_id: u32,
    /// The PCI device id.
    pub device_id: u32,
    /// Its description, e.g. the GPU's model. Local files only.
    pub description: String,
}

/// Why the displays or adapters couldn't be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DisplayError {
    /// A display configuration call failed with this Win32 error.
    Config(u32),
    /// A DXGI call failed with this HRESULT.
    Dxgi(i32),
    /// No active display has this device path.
    NoSuchMonitor,
    /// No adapter has this LUID.
    NoSuchAdapter(Luid),
}

impl fmt::Display for DisplayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(code) => write!(
                f,
                "the display configuration call failed (Win32 error {code})"
            ),
            Self::Dxgi(hr) => write!(f, "a DXGI call failed (HRESULT 0x{:08x})", *hr as u32),
            // The path itself is a PC detail: it isn't repeated here.
            Self::NoSuchMonitor => write!(f, "no active monitor has that device path"),
            Self::NoSuchAdapter(luid) => write!(f, "no adapter has LUID {luid}"),
        }
    }
}

impl std::error::Error for DisplayError {}

/// The target whose device path is `path`. Windows device paths are case-insensitive, so the
/// comparison ignores ASCII case.
pub fn find_by_path<'a>(targets: &'a [DisplayTarget], path: &str) -> Option<&'a DisplayTarget> {
    if path.is_empty() {
        return None;
    }
    targets
        .iter()
        .find(|t| t.device_path.eq_ignore_ascii_case(path))
}

/// The version of the `displays.json` format.
pub const DISPLAYS_VERSION: u32 = 1;

/// `displays.json`: what `targets()` and `adapters()` found, written by `kairo-harness
/// displays` and kept in `.dogfood/pc/` on the Mac.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Displays {
    /// The format version: [`DISPLAYS_VERSION`].
    pub kairo_displays: u32,
    /// The active displays.
    pub targets: Vec<DisplayTarget>,
    /// The adapters, in this process's DXGI order.
    pub adapters: Vec<AdapterInfo>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(path: &str) -> DisplayTarget {
        DisplayTarget {
            device_path: path.to_owned(),
            gdi_name: r"\\.\DISPLAY1".to_owned(),
            friendly_name: "Panel".to_owned(),
            hmonitor: 0x1_0001,
            rect: Rect {
                left: -2_560,
                top: 0,
                right: 0,
                bottom: 1_440,
            },
            primary: false,
            refresh: Refresh {
                numerator: 179_960,
                denominator: 1_000,
            },
            cloned: false,
            adapter_luid: Luid {
                high: 0,
                low: 0x0001_2345,
            },
            hdr: HdrState::default(),
        }
    }

    #[test]
    fn luids_print_as_one_hex_number() {
        let luid = Luid {
            high: 1,
            low: 0xDEAD_BEEF,
        };
        assert_eq!(luid.value(), 0x1_DEAD_BEEF);
        assert_eq!(luid.to_string(), "0x00000001deadbeef");
        // The high part is signed in the API; it prints as its bits.
        assert_eq!(Luid { high: -1, low: 0 }.to_string(), "0xffffffff00000000");
    }

    #[test]
    fn refresh_rates_in_millihertz() {
        let r = |numerator, denominator| Refresh {
            numerator,
            denominator,
        };
        assert_eq!(r(179_960, 1_000).millihertz(), Some(179_960));
        assert_eq!(r(60_000, 1_001).millihertz(), Some(59_940));
        assert_eq!(r(144, 1).millihertz(), Some(144_000));
        // Halves round up.
        assert_eq!(r(1, 2_000).millihertz(), Some(1));
        assert_eq!(r(0, 0).millihertz(), None);
        assert_eq!(r(170, 0).hertz(), None);
        assert_eq!(r(170, 1).hertz(), Some(170.0));
    }

    #[test]
    fn vsync_rates_divide_the_refresh_within_half_a_percent() {
        let at = |mhz: u32| Refresh {
            numerator: mhz,
            denominator: 1_000,
        };
        // README §4.3: 60, 90 and 180 on a 180 Hz panel (179.96 measured); not 72.
        assert_eq!(at(179_960).fits_vsync(180), Some(1));
        assert_eq!(at(179_960).fits_vsync(90), Some(2));
        assert_eq!(at(179_960).fits_vsync(60), Some(3));
        assert_eq!(at(179_960).fits_vsync(72), None);
        // 90, 120, 180 and 360 at 360 Hz; 60 would need k = 6.
        assert_eq!(at(360_000).fits_vsync(120), Some(3));
        assert_eq!(at(360_000).fits_vsync(90), Some(4));
        assert_eq!(at(360_000).fits_vsync(60), None);
        // At 170 Hz nothing near 60 fits.
        assert_eq!(at(170_000).fits_vsync(60), None);
        assert_eq!(at(170_000).fits_vsync(85), Some(2));
        assert_eq!(Refresh::default().fits_vsync(60), None);
    }

    #[test]
    fn rects_and_white_levels() {
        let t = target("x");
        assert_eq!((t.rect.width(), t.rect.height()), (2_560, 1_440));
        let inverted = Rect {
            left: 10,
            top: 10,
            right: 0,
            bottom: 0,
        };
        assert_eq!((inverted.width(), inverted.height()), (0, 0));
        assert_eq!(sdr_white_nits(1_000), 80.0);
        assert_eq!(sdr_white_nits(3_000), 240.0);
        assert_eq!(ColorMode::from_raw(2), Some(ColorMode::Hdr));
        assert_eq!(ColorMode::from_raw(3), None);
    }

    #[test]
    fn monitors_are_found_by_device_path_ignoring_case() {
        let path =
            r"\\?\DISPLAY#ABC1234#5&1a2b3c4d&0&UID4352#{e6f07b5f-ee97-4a90-b076-33f57bf4eaa7}";
        let targets = [target("other"), target(path)];
        assert!(find_by_path(&targets, path).is_some());
        assert!(find_by_path(&targets, &path.to_lowercase()).is_some());
        assert_eq!(find_by_path(&targets, "missing"), None);
        assert_eq!(find_by_path(&targets, ""), None);
        // Errors never repeat the path.
        assert!(!DisplayError::NoSuchMonitor.to_string().contains("DISPLAY"));
        assert_eq!(
            DisplayError::Dxgi(0x887A_0002_u32 as i32).to_string(),
            "a DXGI call failed (HRESULT 0x887a0002)"
        );
    }

    #[test]
    fn displays_json_round_trips() {
        let displays = Displays {
            kairo_displays: DISPLAYS_VERSION,
            targets: vec![target("p")],
            adapters: vec![AdapterInfo {
                luid: Luid {
                    high: 0,
                    low: 0x0001_2345,
                },
                dxgi_index: 0,
                vendor_id: 0x1002,
                device_id: 0x7550,
                description: "GPU".to_owned(),
            }],
        };
        let text = serde_json::to_string(&displays).unwrap();
        assert!(text.starts_with(r#"{"kairo_displays":1,"targets":[{"device_path":"p","#));
        assert_eq!(serde_json::from_str::<Displays>(&text).unwrap(), displays);
    }
}
