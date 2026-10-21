//! Monitor list for "screen mode": friendly names, stable ids, refresh, HDR state, SDR white level.
use std::mem::size_of;

use windows::Win32::Devices::Display::{
    DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO, DISPLAYCONFIG_DEVICE_INFO_GET_SDR_WHITE_LEVEL,
    DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
    DISPLAYCONFIG_DEVICE_INFO_HEADER, DISPLAYCONFIG_DEVICE_INFO_TYPE,
    DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO, DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO,
    DISPLAYCONFIG_SDR_WHITE_LEVEL, DISPLAYCONFIG_SOURCE_DEVICE_NAME, DISPLAYCONFIG_TARGET_DEVICE_NAME,
    DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes, QDC_ONLY_ACTIVE_PATHS, QueryDisplayConfig,
};
use windows::Win32::Foundation::{LPARAM, RECT, TRUE};
use windows::Win32::Graphics::Dxgi::Common::DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020;
use windows::Win32::Graphics::Dxgi::{DXGI_ERROR_NOT_FOUND, IDXGIFactory1, IDXGIOutput6};
use windows::Win32::Graphics::Gdi::{
    DISPLAYCONFIG_COLOR_ENCODING, EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO,
    MONITORINFOEXW,
};
use windows::core::{BOOL, Interface, Result};

/// Not in windows 0.62.2 (SDK 26100 wingdi.h): DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO_2, type 15.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct AdvancedColorInfo2 {
    header: DISPLAYCONFIG_DEVICE_INFO_HEADER,
    bits: u32, // b0 advancedColorSupported, b1 advancedColorActive, b3 limitedByPolicy,
    //            b4 highDynamicRangeSupported, b5 highDynamicRangeUserEnabled,
    //            b6 wideColorSupported, b7 wideColorUserEnabled
    color_encoding: DISPLAYCONFIG_COLOR_ENCODING,
    bits_per_color_channel: u32,
    active_color_mode: i32, // DISPLAYCONFIG_ADVANCED_COLOR_MODE: 0 SDR, 1 WCG, 2 HDR
}
const GET_ADVANCED_COLOR_INFO_2: DISPLAYCONFIG_DEVICE_INFO_TYPE = DISPLAYCONFIG_DEVICE_INFO_TYPE(15);

#[derive(Debug, Clone)]
pub struct MonitorInfo {
    pub hmonitor: HMONITOR,
    pub gdi_name: String,      // \\.\DISPLAY1 (changes across reboots/re-plugs)
    pub friendly_name: String, // EDID name, e.g. the OLED's model
    pub device_path: String,   // stable id: persist this in settings
    pub rect: RECT,
    pub primary: bool,
    pub refresh_hz: f64,
    pub hdr_active: bool,       // DXGI_OUTPUT_DESC1.ColorSpace == G2084/P2020
    pub hdr_supported: bool,    // advanced colour supported (legacy struct)
    pub color_mode: Option<i32>, // 24H2+: 0 SDR, 1 WCG (ACM on SDR), 2 HDR
    pub sdr_white_nits: f32,    // SDRWhiteLevel / 1000 * 80
    pub max_luminance: f32,
    pub bits_per_color: u32,
}

fn wide(s: &[u16]) -> String {
    let n = s.iter().position(|&c| c == 0).unwrap_or(s.len());
    String::from_utf16_lossy(&s[..n])
}

unsafe extern "system" fn push_monitor(h: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
    let v = unsafe { &mut *(data.0 as *mut Vec<HMONITOR>) };
    v.push(h);
    TRUE
}

fn header<T>(kind: DISPLAYCONFIG_DEVICE_INFO_TYPE, path: &DISPLAYCONFIG_PATH_INFO, target: bool) -> DISPLAYCONFIG_DEVICE_INFO_HEADER {
    DISPLAYCONFIG_DEVICE_INFO_HEADER {
        r#type: kind,
        size: size_of::<T>() as u32,
        adapterId: if target { path.targetInfo.adapterId } else { path.sourceInfo.adapterId },
        id: if target { path.targetInfo.id } else { path.sourceInfo.id },
    }
}

pub fn enumerate(factory: &IDXGIFactory1) -> Result<Vec<MonitorInfo>> {
    let mut hmons: Vec<HMONITOR> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(push_monitor), LPARAM(&mut hmons as *mut _ as isize));
    }

    // Active display paths (one per active target).
    let (mut np, mut nm) = (0u32, 0u32);
    unsafe { GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut np, &mut nm) }.ok()?;
    let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); np as usize];
    let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); nm as usize];
    unsafe {
        QueryDisplayConfig(QDC_ONLY_ACTIVE_PATHS, &mut np, paths.as_mut_ptr(), &mut nm, modes.as_mut_ptr(), None)
    }
    .ok()?;
    paths.truncate(np as usize);

    let mut out = Vec::new();
    for h in hmons {
        let mut mi = MONITORINFOEXW::default();
        mi.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
        if !unsafe { GetMonitorInfoW(h, &mut mi as *mut MONITORINFOEXW as *mut MONITORINFO) }.as_bool() {
            continue;
        }
        let gdi_name = wide(&mi.szDevice);
        let mut m = MonitorInfo {
            hmonitor: h,
            gdi_name: gdi_name.clone(),
            friendly_name: String::new(),
            device_path: String::new(),
            rect: mi.monitorInfo.rcMonitor,
            primary: mi.monitorInfo.dwFlags & 1 != 0, // MONITORINFOF_PRIMARY
            refresh_hz: 0.0,
            hdr_active: false,
            hdr_supported: false,
            color_mode: None,
            sdr_white_nits: 80.0,
            max_luminance: 0.0,
            bits_per_color: 8,
        };

        for p in &paths {
            let mut src = DISPLAYCONFIG_SOURCE_DEVICE_NAME {
                header: header::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>(DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, p, false),
                viewGdiDeviceName: [0; 32],
            };
            if unsafe { DisplayConfigGetDeviceInfo(&mut src.header) } != 0 || wide(&src.viewGdiDeviceName) != gdi_name {
                continue;
            }
            m.refresh_hz = p.targetInfo.refreshRate.Numerator as f64 / p.targetInfo.refreshRate.Denominator.max(1) as f64;

            let mut tn = DISPLAYCONFIG_TARGET_DEVICE_NAME::default();
            tn.header = header::<DISPLAYCONFIG_TARGET_DEVICE_NAME>(DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME, p, true);
            if unsafe { DisplayConfigGetDeviceInfo(&mut tn.header) } == 0 {
                m.friendly_name = wide(&tn.monitorFriendlyDeviceName);
                m.device_path = wide(&tn.monitorDevicePath);
            }

            let mut wl = DISPLAYCONFIG_SDR_WHITE_LEVEL {
                header: header::<DISPLAYCONFIG_SDR_WHITE_LEVEL>(DISPLAYCONFIG_DEVICE_INFO_GET_SDR_WHITE_LEVEL, p, true),
                SDRWhiteLevel: 1000,
            };
            if unsafe { DisplayConfigGetDeviceInfo(&mut wl.header) } == 0 {
                m.sdr_white_nits = wl.SDRWhiteLevel as f32 / 1000.0 * 80.0;
            }

            let mut aci = DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO::default();
            aci.header = header::<DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO>(DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO, p, true);
            if unsafe { DisplayConfigGetDeviceInfo(&mut aci.header) } == 0 {
                let bits = unsafe { aci.Anonymous.value };
                m.hdr_supported = bits & 0b01 != 0; // advancedColorSupported
            }

            let mut aci2 = AdvancedColorInfo2 {
                header: header::<AdvancedColorInfo2>(GET_ADVANCED_COLOR_INFO_2, p, true),
                ..Default::default()
            };
            if unsafe { DisplayConfigGetDeviceInfo(&mut aci2.header) } == 0 {
                m.color_mode = Some(aci2.active_color_mode); // fails (non-zero) before 24H2: keep None
            }
            break;
        }

        // DXGI view of the same monitor (colour space + luminance). Re-create the factory when
        // IDXGIFactory1::IsCurrent() returns FALSE, otherwise these values are stale.
        let mut a = 0;
        'adapters: loop {
            let adapter = match unsafe { factory.EnumAdapters1(a) } {
                Ok(x) => x,
                Err(e) if e.code() == DXGI_ERROR_NOT_FOUND => break,
                Err(e) => return Err(e),
            };
            let mut o = 0;
            while let Ok(output) = unsafe { adapter.EnumOutputs(o) } {
                if let Ok(o6) = output.cast::<IDXGIOutput6>() {
                    if let Ok(d1) = unsafe { o6.GetDesc1() } {
                        if d1.Monitor.0 == h.0 {
                            m.hdr_active = d1.ColorSpace == DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020;
                            m.max_luminance = d1.MaxLuminance;
                            m.bits_per_color = d1.BitsPerColor;
                            break 'adapters;
                        }
                    }
                }
                o += 1;
            }
            a += 1;
        }
        out.push(m);
    }
    Ok(out)
}
