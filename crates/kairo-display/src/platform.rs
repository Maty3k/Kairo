// SPDX-License-Identifier: GPL-3.0-or-later
//! Every Win32 and DXGI call `kairo-display` makes; the only module with `unsafe`.
//!
//! The display configuration API (`QueryDisplayConfig`, `DisplayConfigGetDeviceInfo`) maps a
//! GDI name to its device path, friendly name, refresh rate, SDR white level and advanced
//! colour state (research 08 §7.2–7.3, adapted from docs/research/prototypes/kcapcheck); DXGI
//! maps each HMONITOR to the adapter that owns its output and tells whether it is in HDR.
//! Nothing here touches another process. Signatures follow `windows` 0.62.2.

use std::mem::size_of;

use windows::Win32::Devices::Display::{
    DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO,
    DISPLAYCONFIG_DEVICE_INFO_GET_SDR_WHITE_LEVEL, DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
    DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME, DISPLAYCONFIG_DEVICE_INFO_HEADER,
    DISPLAYCONFIG_DEVICE_INFO_TYPE, DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO, DISPLAYCONFIG_MODE_INFO,
    DISPLAYCONFIG_PATH_INFO, DISPLAYCONFIG_SDR_WHITE_LEVEL, DISPLAYCONFIG_SOURCE_DEVICE_NAME,
    DISPLAYCONFIG_TARGET_DEVICE_NAME, DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes,
    QDC_ONLY_ACTIVE_PATHS, QueryDisplayConfig,
};
use windows::Win32::Foundation::{
    ERROR_INSUFFICIENT_BUFFER, LPARAM, LUID, RECT, TRUE, WIN32_ERROR,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020;
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory2, DXGI_CREATE_FACTORY_FLAGS, DXGI_ERROR_NOT_FOUND, IDXGIAdapter1,
    IDXGIFactory1, IDXGIOutput6,
};
use windows::Win32::Graphics::Gdi::{
    DISPLAYCONFIG_COLOR_ENCODING, EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO,
    MONITORINFOEXW,
};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows::core::{BOOL, Interface};

use crate::{
    AdapterInfo, ColorMode, DisplayError, DisplayTarget, HdrState, Luid, Rect, Refresh,
    sdr_white_nits,
};

/// `MONITORINFOF_PRIMARY`.
const MONITOR_PRIMARY: u32 = 1;

/// `DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO_2` (Windows 11 24H2, SDK 26100 wingdi.h), which
/// `windows` 0.62.2 doesn't have yet.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct AdvancedColorInfo2 {
    header: DISPLAYCONFIG_DEVICE_INFO_HEADER,
    /// Bit 0 advanced colour supported, 1 active, 4 HDR supported, 5 HDR user enabled.
    bits: u32,
    color_encoding: DISPLAYCONFIG_COLOR_ENCODING,
    bits_per_color_channel: u32,
    /// `DISPLAYCONFIG_ADVANCED_COLOR_MODE`: 0 SDR, 1 WCG, 2 HDR.
    active_color_mode: i32,
}

/// `DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO_2`.
const GET_ADVANCED_COLOR_INFO_2: DISPLAYCONFIG_DEVICE_INFO_TYPE =
    DISPLAYCONFIG_DEVICE_INFO_TYPE(15);

/// Makes this process per-monitor-v2 DPI aware, so rectangles and window sizes are physical
/// pixels and a swap chain maps 1:1 to the screen (research 08:767). Call it first thing,
/// before any window exists. Fails if the awareness was already set, which is harmless.
pub fn make_dpi_aware() -> bool {
    // SAFETY: takes a constant context handle and changes only this process's DPI mode.
    unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) }.is_ok()
}

fn wide(text: &[u16]) -> String {
    let end = text.iter().position(|&c| c == 0).unwrap_or(text.len());
    String::from_utf16_lossy(&text[..end])
}

fn luid(raw: LUID) -> Luid {
    Luid {
        high: raw.HighPart,
        low: raw.LowPart,
    }
}

fn config_error(code: WIN32_ERROR) -> DisplayError {
    DisplayError::Config(code.0)
}

fn dxgi_error(error: windows::core::Error) -> DisplayError {
    DisplayError::Dxgi(error.code().0)
}

unsafe extern "system" fn push_monitor(
    monitor: HMONITOR,
    _: HDC,
    _: *mut RECT,
    data: LPARAM,
) -> BOOL {
    // SAFETY: `data` is the `&mut Vec<HMONITOR>` that `monitors` passes, alive for the whole
    // EnumDisplayMonitors call, which calls back on the same thread.
    let list = unsafe { &mut *(data.0 as *mut Vec<HMONITOR>) };
    list.push(monitor);
    TRUE
}

/// Every HMONITOR on the desktop.
fn monitors() -> Vec<HMONITOR> {
    let mut list: Vec<HMONITOR> = Vec::new();
    // SAFETY: the callback only pushes into `list`, which outlives the call.
    let _ = unsafe {
        EnumDisplayMonitors(
            None,
            None,
            Some(push_monitor),
            LPARAM(&raw mut list as isize),
        )
    };
    list
}

/// The active display paths.
fn active_paths() -> Result<Vec<DISPLAYCONFIG_PATH_INFO>, DisplayError> {
    // The configuration can change between the two calls; retry a few times if it grew.
    for _ in 0..4 {
        let (mut paths_len, mut modes_len) = (0_u32, 0_u32);
        // SAFETY: two u32 out-parameters.
        let sizes = unsafe {
            GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut paths_len, &mut modes_len)
        };
        if sizes.0 != 0 {
            return Err(config_error(sizes));
        }
        let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); paths_len as usize];
        let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); modes_len as usize];
        // SAFETY: the buffers hold the element counts passed, which the call may lower.
        let status = unsafe {
            QueryDisplayConfig(
                QDC_ONLY_ACTIVE_PATHS,
                &mut paths_len,
                paths.as_mut_ptr(),
                &mut modes_len,
                modes.as_mut_ptr(),
                None,
            )
        };
        if status == ERROR_INSUFFICIENT_BUFFER {
            continue;
        }
        if status.0 != 0 {
            return Err(config_error(status));
        }
        paths.truncate(paths_len as usize);
        return Ok(paths);
    }
    Err(config_error(ERROR_INSUFFICIENT_BUFFER))
}

fn header<T>(
    kind: DISPLAYCONFIG_DEVICE_INFO_TYPE,
    path: &DISPLAYCONFIG_PATH_INFO,
    target: bool,
) -> DISPLAYCONFIG_DEVICE_INFO_HEADER {
    let (adapter, id) = if target {
        (path.targetInfo.adapterId, path.targetInfo.id)
    } else {
        (path.sourceInfo.adapterId, path.sourceInfo.id)
    };
    DISPLAYCONFIG_DEVICE_INFO_HEADER {
        r#type: kind,
        size: size_of::<T>() as u32,
        adapterId: adapter,
        id,
    }
}

/// What the display configuration says about one path.
struct PathFacts {
    device_path: String,
    friendly_name: String,
    refresh: Refresh,
    source_adapter: Luid,
    supported: bool,
    mode: Option<ColorMode>,
    sdr_white_nits: f64,
}

/// Asks the display configuration for one packet; true if it answered.
///
/// # Safety
///
/// `T` is `#[repr(C)]`, starts with its [`DISPLAYCONFIG_DEVICE_INFO_HEADER`], and that
/// header's size is `size_of::<T>()`, as [`header`] sets it.
unsafe fn device_info<T>(packet: &mut T) -> bool {
    let pointer = std::ptr::from_mut(packet).cast::<DISPLAYCONFIG_DEVICE_INFO_HEADER>();
    // SAFETY: the pointer comes from all of `packet`, so it covers every byte the call writes
    // (the header's size); the caller vouches for the layout.
    unsafe { DisplayConfigGetDeviceInfo(pointer) == 0 }
}

/// The GDI name of a path's source, e.g. `\\.\DISPLAY1`. In Duplicate mode several active
/// paths share one source, and so its GDI name and HMONITOR.
fn source_name(path: &DISPLAYCONFIG_PATH_INFO) -> Option<String> {
    let mut source = DISPLAYCONFIG_SOURCE_DEVICE_NAME {
        header: header::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>(
            DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
            path,
            false,
        ),
        viewGdiDeviceName: [0; 32],
    };
    // SAFETY: a repr(C) packet headed by its own header.
    let answered = unsafe { device_info(&mut source) };
    answered.then(|| wide(&source.viewGdiDeviceName))
}

/// Whether two paths show one source: Windows' Duplicate mode.
fn same_source(a: &DISPLAYCONFIG_PATH_INFO, b: &DISPLAYCONFIG_PATH_INFO) -> bool {
    a.sourceInfo.id == b.sourceInfo.id
        && luid(a.sourceInfo.adapterId) == luid(b.sourceInfo.adapterId)
}

/// What the display configuration says about one path's target.
fn path_facts(path: &DISPLAYCONFIG_PATH_INFO) -> PathFacts {
    let rate = path.targetInfo.refreshRate;
    let mut facts = PathFacts {
        device_path: String::new(),
        friendly_name: String::new(),
        refresh: Refresh {
            numerator: rate.Numerator,
            denominator: rate.Denominator,
        },
        source_adapter: luid(path.sourceInfo.adapterId),
        supported: false,
        mode: None,
        sdr_white_nits: sdr_white_nits(1_000),
    };
    let mut name = DISPLAYCONFIG_TARGET_DEVICE_NAME {
        header: header::<DISPLAYCONFIG_TARGET_DEVICE_NAME>(
            DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
            path,
            true,
        ),
        ..Default::default()
    };
    // SAFETY: as above, for the target name.
    if unsafe { device_info(&mut name) } {
        facts.friendly_name = wide(&name.monitorFriendlyDeviceName);
        facts.device_path = wide(&name.monitorDevicePath);
    }
    let mut white = DISPLAYCONFIG_SDR_WHITE_LEVEL {
        header: header::<DISPLAYCONFIG_SDR_WHITE_LEVEL>(
            DISPLAYCONFIG_DEVICE_INFO_GET_SDR_WHITE_LEVEL,
            path,
            true,
        ),
        SDRWhiteLevel: 1_000,
    };
    // SAFETY: as above, for the SDR white level.
    if unsafe { device_info(&mut white) } {
        facts.sdr_white_nits = sdr_white_nits(white.SDRWhiteLevel);
    }
    let mut colour = DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO {
        header: header::<DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO>(
            DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO,
            path,
            true,
        ),
        ..Default::default()
    };
    // SAFETY: as above, for the advanced colour info.
    if unsafe { device_info(&mut colour) } {
        // SAFETY: the union's `value` view of the bit field is always initialised.
        let bits = unsafe { colour.Anonymous.value };
        facts.supported = bits & 1 != 0;
    }
    let mut colour2 = AdvancedColorInfo2 {
        header: header::<AdvancedColorInfo2>(GET_ADVANCED_COLOR_INFO_2, path, true),
        ..Default::default()
    };
    // SAFETY: as above; the struct is repr(C) with SDK 26100's layout. Before 24H2 the call
    // fails and `mode` stays unknown.
    if unsafe { device_info(&mut colour2) } {
        facts.mode = ColorMode::from_raw(colour2.active_color_mode);
    }
    facts
}

/// What DXGI says about one output.
struct OutputFacts {
    monitor: isize,
    adapter: Luid,
    hdr_active: bool,
    max_luminance: f64,
    bits_per_color: u32,
}

/// Every adapter with its outputs, in DXGI order.
fn dxgi(factory: &IDXGIFactory1) -> Result<(Vec<AdapterInfo>, Vec<OutputFacts>), DisplayError> {
    let (mut adapters, mut outputs) = (Vec::new(), Vec::new());
    for index in 0_u32.. {
        // SAFETY: a plain COM call; DXGI_ERROR_NOT_FOUND ends the list.
        let adapter: IDXGIAdapter1 = match unsafe { factory.EnumAdapters1(index) } {
            Ok(adapter) => adapter,
            Err(e) if e.code() == DXGI_ERROR_NOT_FOUND => break,
            Err(e) => return Err(dxgi_error(e)),
        };
        // SAFETY: a plain COM call returning a struct by value.
        let desc = unsafe { adapter.GetDesc1() }.map_err(dxgi_error)?;
        let id = luid(desc.AdapterLuid);
        adapters.push(AdapterInfo {
            luid: id,
            dxgi_index: index,
            vendor_id: desc.VendorId,
            device_id: desc.DeviceId,
            description: wide(&desc.Description),
        });
        for output_index in 0_u32.. {
            // SAFETY: a plain COM call; DXGI_ERROR_NOT_FOUND ends the list.
            let output = match unsafe { adapter.EnumOutputs(output_index) } {
                Ok(output) => output,
                Err(e) if e.code() == DXGI_ERROR_NOT_FOUND => break,
                Err(e) => return Err(dxgi_error(e)),
            };
            let Ok(output6) = output.cast::<IDXGIOutput6>() else {
                continue;
            };
            // SAFETY: a plain COM call returning a struct by value.
            if let Ok(d) = unsafe { output6.GetDesc1() } {
                outputs.push(OutputFacts {
                    monitor: d.Monitor.0 as isize,
                    adapter: id,
                    hdr_active: d.ColorSpace == DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020,
                    max_luminance: f64::from(d.MaxLuminance),
                    bits_per_color: d.BitsPerColor,
                });
            }
        }
    }
    Ok((adapters, outputs))
}

fn factory() -> Result<IDXGIFactory1, DisplayError> {
    // SAFETY: creates a new DXGI factory; no flags. A new factory is never stale
    // (IDXGIFactory1::IsCurrent, research 08 §7.3).
    unsafe { CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0)) }.map_err(dxgi_error)
}

/// Every active display: one entry per active path, so a screen that mirrors another
/// (Duplicate mode) is listed too, with `cloned` set on both. Rectangles are physical pixels
/// only in a per-monitor-v2 DPI aware process ([`make_dpi_aware`]).
pub fn targets() -> Result<Vec<DisplayTarget>, DisplayError> {
    let paths = active_paths()?;
    let (_, outputs) = dxgi(&factory()?)?;
    // Each source's GDI name, HMONITOR and monitor info.
    let mut sources = Vec::new();
    for monitor in monitors() {
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
        // SAFETY: MONITORINFOEXW begins with MONITORINFO and cbSize says which one it is.
        if unsafe { GetMonitorInfoW(monitor, (&raw mut info).cast::<MONITORINFO>()) }.as_bool() {
            sources.push((wide(&info.szDevice), monitor, info.monitorInfo));
        }
    }
    let mut out = Vec::new();
    for path in &paths {
        let Some(gdi_name) = source_name(path) else {
            continue;
        };
        let Some(&(_, monitor, info)) = sources.iter().find(|(name, _, _)| *name == gdi_name)
        else {
            continue;
        };
        let facts = path_facts(path);
        let output = outputs.iter().find(|o| o.monitor == monitor.0 as isize);
        let r = info.rcMonitor;
        out.push(DisplayTarget {
            device_path: facts.device_path,
            gdi_name,
            friendly_name: facts.friendly_name,
            hmonitor: monitor.0 as isize,
            rect: Rect {
                left: r.left,
                top: r.top,
                right: r.right,
                bottom: r.bottom,
            },
            primary: info.dwFlags & MONITOR_PRIMARY != 0,
            refresh: facts.refresh,
            cloned: paths.iter().filter(|p| same_source(p, path)).count() > 1,
            // DXGI's owner of the output is the adapter to create the device on; the path's
            // source adapter is the fallback.
            adapter_luid: output.map_or(facts.source_adapter, |o| o.adapter),
            hdr: HdrState {
                supported: facts.supported,
                active: output.is_some_and(|o| o.hdr_active),
                mode: facts.mode,
                sdr_white_nits: facts.sdr_white_nits,
                max_luminance_nits: output.map_or(0.0, |o| o.max_luminance),
                bits_per_color: output.map_or(8, |o| o.bits_per_color),
            },
        });
    }
    Ok(out)
}

/// Every adapter, in this process's DXGI order.
pub fn adapters() -> Result<Vec<AdapterInfo>, DisplayError> {
    Ok(dxgi(&factory()?)?.0)
}

/// The adapter with LUID `luid`.
pub fn adapter_for_luid(luid: Luid) -> Result<AdapterInfo, DisplayError> {
    adapters()?
        .into_iter()
        .find(|a| a.luid == luid)
        .ok_or(DisplayError::NoSuchAdapter(luid))
}
