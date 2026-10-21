use windows::core::{w, Interface, PCWSTR};
use windows::Win32::Devices::Display::*;
use windows::Win32::Graphics::Gdi::DISPLAYCONFIG_COLOR_ENCODING;
use windows::Win32::Foundation::{GlobalFree, HANDLE, HWND, POINT};
use windows::Win32::Graphics::Dxgi::{Common::DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020, IDXGIFactory1, IDXGIOutput, IDXGIOutput6};
use windows::Win32::System::Registry::{RegGetValueW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD};
use windows::Win32::UI::Shell::{SHQueryUserNotificationState, QUNS_RUNNING_D3D_FULL_SCREEN};
use windows::Win32::UI::WindowsAndMessaging::{SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE};

// ---- HDR per output ----
pub fn hdr_active(output: &IDXGIOutput) -> windows::core::Result<(bool, f32, u32)> {
    let d = unsafe { output.cast::<IDXGIOutput6>()?.GetDesc1()? };
    Ok((d.ColorSpace == DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020, d.MaxLuminance, d.BitsPerColor))
}
/// Poll ~1 Hz; when false, create a NEW factory and re-read outputs (descriptors are cached per factory).
pub fn factory_stale(f: &IDXGIFactory1) -> bool { !unsafe { f.IsCurrent() }.as_bool() }

pub fn sdr_white_nits(p: &DISPLAYCONFIG_PATH_INFO) -> Option<f32> {
    let mut wl = DISPLAYCONFIG_SDR_WHITE_LEVEL {
        header: DISPLAYCONFIG_DEVICE_INFO_HEADER { r#type: DISPLAYCONFIG_DEVICE_INFO_GET_SDR_WHITE_LEVEL,
            size: size_of::<DISPLAYCONFIG_SDR_WHITE_LEVEL>() as u32, adapterId: p.targetInfo.adapterId, id: p.targetInfo.id },
        SDRWhiteLevel: 1000 };
    (unsafe { DisplayConfigGetDeviceInfo(&mut wl.header) } == 0).then(|| wl.SDRWhiteLevel as f32 / 1000.0 * 80.0)
}
/// Not in windows 0.62.2: hand-declared from SDK 26100 wingdi.h (request type 15). Non-zero return before 24H2.
#[repr(C)] #[derive(Clone, Copy, Default)]
pub struct AdvancedColorInfo2 {
    pub header: DISPLAYCONFIG_DEVICE_INFO_HEADER,
    pub bits: u32,                 // b0 supported, b1 active, b3 limitedByPolicy, b4 hdrSupported, b5 hdrUserEnabled, ...
    pub color_encoding: DISPLAYCONFIG_COLOR_ENCODING,
    pub bits_per_color_channel: u32,
    pub active_color_mode: i32,    // 0 SDR, 1 WCG (ACM), 2 HDR
}
pub const GET_ADVANCED_COLOR_INFO_2: DISPLAYCONFIG_DEVICE_INFO_TYPE = DISPLAYCONFIG_DEVICE_INFO_TYPE(15);
pub fn color_mode(p: &DISPLAYCONFIG_PATH_INFO) -> Option<i32> {
    let mut v = AdvancedColorInfo2 { header: DISPLAYCONFIG_DEVICE_INFO_HEADER { r#type: GET_ADVANCED_COLOR_INFO_2,
        size: size_of::<AdvancedColorInfo2>() as u32, adapterId: p.targetInfo.adapterId, id: p.targetInfo.id }, ..Default::default() };
    (unsafe { DisplayConfigGetDeviceInfo(&mut v.header) } == 0).then_some(v.active_color_mode)
}

// ---- Exclusive fullscreen hint (no change notification: poll 1 Hz + on foreground change) ----
pub fn d3d_exclusive_fullscreen() -> bool { matches!(unsafe { SHQueryUserNotificationState() }, Ok(s) if s == QUNS_RUNNING_D3D_FULL_SCREEN) }

// ---- Engine-owned toast: top-level, own process, created once, hidden when idle ----
pub fn exclude_from_capture(hwnd: HWND) -> windows::core::Result<()> { unsafe { SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE) } }

// ---- AMD hints (registry names [U]; PDH is the ground truth) ----
fn reg_dword(root: HKEY, sub: PCWSTR, val: PCWSTR) -> Option<u32> {
    let (mut v, mut len) = (0u32, 4u32);
    unsafe { RegGetValueW(root, sub, val, RRF_RT_REG_DWORD, None, Some(&mut v as *mut u32 as *mut _), Some(&mut len)) }.is_ok().then_some(v)
}
pub fn amd_instant_replay_hint() -> Option<bool> {
    reg_dword(HKEY_LOCAL_MACHINE, w!("SOFTWARE\\AMD\\DVR"), w!("InstantReplayEnabled"))
        .or_else(|| reg_dword(HKEY_CURRENT_USER, w!("Software\\AMD\\DVR"), w!("InstantReplayEnabled"))).map(|v| v != 0)
}

// ---- CF_HDROP + Preferred DropEffect (call on the UI thread with the main window as owner) ----
pub fn set_file_drop(hwnd: HWND, paths: &[std::path::PathBuf]) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::System::{DataExchange::*, Memory::*, Ole::{CF_HDROP, DROPEFFECT_COPY}};
    use windows::Win32::UI::Shell::{CFSTR_PREFERREDDROPEFFECT, DROPFILES};
    let mut list: Vec<u16> = Vec::new();
    for p in paths { list.extend(p.as_os_str().encode_wide()); list.push(0); } // absolute, no \\?\ prefix
    list.push(0);
    let hdr = size_of::<DROPFILES>();
    unsafe {
        let hdrop = GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, hdr + list.len() * 2).map_err(|e| e.to_string())?;
        let base = GlobalLock(hdrop) as *mut u8;
        if base.is_null() { let _ = GlobalFree(Some(hdrop)); return Err("GlobalLock".into()); }
        std::ptr::write_unaligned(base as *mut DROPFILES, DROPFILES { pFiles: hdr as u32, pt: POINT { x: 0, y: 0 }, fNC: false.into(), fWide: true.into() });
        std::ptr::copy_nonoverlapping(list.as_ptr() as *const u8, base.add(hdr), list.len() * 2);
        let _ = GlobalUnlock(hdrop); // Err with code 0 = success (lock count reached 0)
        let effect = GlobalAlloc(GMEM_MOVEABLE, 4).map_err(|e| e.to_string())?;
        *(GlobalLock(effect) as *mut u32) = DROPEFFECT_COPY.0;
        let _ = GlobalUnlock(effect);
        let cf_effect = RegisterClipboardFormatW(CFSTR_PREFERREDDROPEFFECT);
        let mut opened = false;
        for _ in 0..10 { if OpenClipboard(Some(hwnd)).is_ok() { opened = true; break; } std::thread::sleep(std::time::Duration::from_millis(20)); }
        if !opened { let _ = GlobalFree(Some(hdrop)); let _ = GlobalFree(Some(effect)); return Err("clipboard busy".into()); }
        let r = (|| -> Result<(), String> {
            EmptyClipboard().map_err(|e| e.to_string())?; // owner = hwnd (a NULL owner makes SetClipboardData fail)
            if SetClipboardData(CF_HDROP.0 as u32, Some(HANDLE(hdrop.0))).is_err() { let _ = GlobalFree(Some(hdrop)); return Err("SetClipboardData".into()); }
            if SetClipboardData(cf_effect, Some(HANDLE(effect.0))).is_err() { let _ = GlobalFree(Some(effect)); }
            Ok(()) // on success the system owns the memory
        })();
        let _ = CloseClipboard();
        r
    }
}
