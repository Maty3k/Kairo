//! Keep Kairo's own windows out of its captures.
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowDisplayAffinity, SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE, WDA_NONE,
};
use windows::core::Result;

/// Only for top-level windows owned by the calling process (MS docs). The engine can do this for
/// its own toast/overlay windows; the Tauri UI process must do it for its windows itself.
pub fn exclude_from_capture(hwnd: HWND, exclude: bool) -> Result<()> {
    unsafe { SetWindowDisplayAffinity(hwnd, if exclude { WDA_EXCLUDEFROMCAPTURE } else { WDA_NONE }) }
}

pub fn is_excluded(hwnd: HWND) -> bool {
    let mut a = 0u32;
    unsafe { GetWindowDisplayAffinity(hwnd, &mut a) }.is_ok() && a == WDA_EXCLUDEFROMCAPTURE.0
}
