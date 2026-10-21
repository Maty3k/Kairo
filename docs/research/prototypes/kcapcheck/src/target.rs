//! Picking what to capture: the foreground game window (window mode) or its monitor (screen mode).
use std::mem::size_of;

use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
use windows::Win32::Graphics::Gdi::{HMONITOR, MONITOR_DEFAULTTONEAREST, MonitorFromWindow};
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::WindowsAndMessaging::{
    EVENT_SYSTEM_FOREGROUND, GA_ROOT, GWL_EXSTYLE, GetAncestor, GetForegroundWindow, GetWindowLongPtrW,
    GetWindowRect, GetWindowThreadProcessId, IsIconic, IsWindowVisible, WINEVENT_OUTOFCONTEXT,
    WINEVENT_SKIPOWNPROCESS, WS_EX_TOOLWINDOW,
};

pub fn is_capturable(hwnd: HWND) -> bool {
    unsafe {
        if hwnd.0.is_null() || !IsWindowVisible(hwnd).as_bool() {
            return false;
        }
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        if ex & WS_EX_TOOLWINDOW.0 != 0 {
            return false;
        }
        let mut cloaked = 0u32; // UWP on another virtual desktop, suspended apps, ...
        if DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, &mut cloaked as *mut u32 as *mut _, size_of::<u32>() as u32)
            .is_ok()
            && cloaked != 0
        {
            return false;
        }
    }
    true
}

/// (top-level hwnd, pid) of the foreground window, or None if it is ours / not capturable.
pub fn foreground_candidate(own_pid: u32) -> Option<(HWND, u32)> {
    let fg = unsafe { GetForegroundWindow() };
    if fg.0.is_null() {
        return None;
    }
    let root = unsafe { GetAncestor(fg, GA_ROOT) };
    if !is_capturable(root) {
        return None;
    }
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(root, Some(&mut pid)) };
    if pid == own_pid { None } else { Some((root, pid)) }
}

pub fn monitor_of(hwnd: HWND) -> HMONITOR {
    unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) }
}

pub fn is_minimized(hwnd: HWND) -> bool {
    unsafe { IsIconic(hwnd).as_bool() }
}

/// Borderless-fullscreen heuristic: window rect == monitor rect.
pub fn covers_monitor(hwnd: HWND, monitor_rect: &RECT) -> bool {
    let mut r = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut r) }.is_ok()
        && r.left <= monitor_rect.left
        && r.top <= monitor_rect.top
        && r.right >= monitor_rect.right
        && r.bottom >= monitor_rect.bottom
}

unsafe extern "system" fn on_foreground(
    _hook: HWINEVENTHOOK,
    _event: u32,
    hwnd: HWND,
    _id_object: i32,
    _id_child: i32,
    _thread: u32,
    _time: u32,
) {
    // Runs on the thread that installed the hook (needs a message loop). Post to the engine:
    // re-evaluate target, game DB match, capture mode.
    let _ = hwnd;
}

/// Out-of-context foreground hook (no DLL is mapped into games). Requires a GetMessageW loop.
pub fn install_foreground_hook() -> HWINEVENTHOOK {
    unsafe {
        SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            None,
            Some(on_foreground),
            0,
            0,
            WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
        )
    }
}

pub fn remove_hook(h: HWINEVENTHOOK) {
    unsafe {
        let _ = UnhookWinEvent(h);
    }
}
