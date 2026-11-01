// SPDX-License-Identifier: GPL-3.0-or-later
//! The testbed's Win32 calls (docs/research/16-verify-testbed/README.md §4.1–4.3): QPC time,
//! the high-resolution timer, the power and display requests, the window and its messages, and
//! taking the foreground. Nothing here synthesises input, hooks anything, attaches to another
//! thread's input or opens another process (CLAUDE.md house rules 1 and 4).

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use kairo_core::timeline;
use windows::Win32::Foundation::{
    CloseHandle, HANDLE, HWND, LPARAM, LRESULT, POINT, RECT, WAIT_OBJECT_0, WPARAM,
};
use windows::Win32::Graphics::Dwm::{DWM_TIMING_INFO, DwmGetCompositionTimingInfo};
use windows::Win32::Graphics::Gdi::{BLACK_BRUSH, ClientToScreen, GetStockObject, HBRUSH};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows::Win32::System::Power::{ES_CONTINUOUS, ES_DISPLAY_REQUIRED, SetThreadExecutionState};
use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::System::Threading::{
    CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, CreateWaitableTimerExW, GetCurrentProcess,
    PROCESS_POWER_THROTTLING_CURRENT_VERSION, PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
    PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION, PROCESS_POWER_THROTTLING_STATE,
    ProcessPowerThrottling, SetProcessInformation, SetWaitableTimer, TIMER_ALL_ACCESS,
    WaitForSingleObject,
};
use windows::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForWindow};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect,
    GetForegroundWindow, HTCLIENT, MSG, PM_REMOVE, PeekMessageW, RegisterClassExW,
    SPI_GETFOREGROUNDLOCKTIMEOUT, SW_SHOW, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOZORDER,
    SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SetCursor, SetForegroundWindow, SetWindowPos, ShowWindow,
    SystemParametersInfoW, TranslateMessage, WINDOW_EX_STYLE, WINDOW_STYLE, WM_CLOSE,
    WM_ERASEBKGND, WM_SETCURSOR, WNDCLASSEXW, WS_CAPTION, WS_MINIMIZEBOX, WS_OVERLAPPED, WS_POPUP,
    WS_SYSMENU,
};
use windows::core::{PCWSTR, w};

/// Set when the window gets `WM_CLOSE`.
static CLOSE: AtomicBool = AtomicBool::new(false);

/// QPC's frequency, read once.
fn frequency() -> i64 {
    static FREQUENCY: OnceLock<i64> = OnceLock::new();
    *FREQUENCY.get_or_init(|| {
        let mut f = 0_i64;
        // SAFETY: writes one i64 we own; it can't fail on Windows XP or later.
        let _ = unsafe { QueryPerformanceFrequency(&mut f) };
        f
    })
}

/// A QPC reading in 100 ns, the unit of every time in the log.
pub fn qpc_to_hns(counter: i64) -> i64 {
    timeline::hns_from_qpc(counter, frequency()).unwrap_or(0)
}

/// Now, in QPC 100 ns.
pub fn now_hns() -> i64 {
    let mut counter = 0_i64;
    // SAFETY: writes one i64 we own.
    let _ = unsafe { QueryPerformanceCounter(&mut counter) };
    qpc_to_hns(counter)
}

/// Opts the process out of EcoQoS and timer-resolution throttling (cookbook P16), so pacing
/// isn't slowed when the window isn't in front.
pub fn opt_out_of_power_throttling() -> bool {
    let state = PROCESS_POWER_THROTTLING_STATE {
        Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        ControlMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED
            | PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION,
        StateMask: 0,
    };
    // SAFETY: `state` outlives the call and its size is passed; the handle is this process's
    // pseudo-handle.
    unsafe {
        SetProcessInformation(
            GetCurrentProcess(),
            ProcessPowerThrottling,
            (&raw const state).cast(),
            size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
        )
    }
    .is_ok()
}

/// Keeps the screens on while it lives (`ES_DISPLAY_REQUIRED`, README §4.1, §8.1).
pub struct KeepDisplayOn;

impl KeepDisplayOn {
    /// Asks for the displays to stay on.
    pub fn new() -> KeepDisplayOn {
        // SAFETY: changes only this thread's execution-state request.
        unsafe { SetThreadExecutionState(ES_CONTINUOUS | ES_DISPLAY_REQUIRED) };
        KeepDisplayOn
    }
}

impl Drop for KeepDisplayOn {
    fn drop(&mut self) {
        // SAFETY: clears this thread's request.
        unsafe { SetThreadExecutionState(ES_CONTINUOUS) };
    }
}

/// A high-resolution waitable timer.
pub struct Timer(HANDLE);

impl Timer {
    /// A new timer, or `None` if Windows has no high-resolution timers (before Windows 10 1803).
    pub fn new() -> Option<Timer> {
        // SAFETY: no attributes and no name; the handle is ours and closed in Drop.
        let handle = unsafe {
            CreateWaitableTimerExW(
                None,
                PCWSTR::null(),
                CREATE_WAITABLE_TIMER_HIGH_RESOLUTION,
                TIMER_ALL_ACCESS.0,
            )
        }
        .ok()?;
        Some(Timer(handle))
    }

    /// Waits until QPC time `t_hns`; returns at once if it has passed.
    pub fn wait_until(&self, t_hns: i64) {
        let left = t_hns - now_hns();
        if left <= 0 {
            return;
        }
        // Negative: relative to now, in 100 ns.
        let due = -left;
        // SAFETY: the handle is ours and `due` outlives the call; no completion routine.
        let set = unsafe { SetWaitableTimer(self.0, &due, 0, None, None, false) };
        if set.is_ok() {
            let ms = u32::try_from(left / 10_000 + 1_000).unwrap_or(u32::MAX);
            // SAFETY: waits on our own timer, bounded.
            let woke = unsafe { WaitForSingleObject(self.0, ms) };
            if woke == WAIT_OBJECT_0 {
                return;
            }
        }
        std::thread::sleep(std::time::Duration::from_nanos(left.max(0) as u64 * 100));
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        // SAFETY: closes the handle CreateWaitableTimerExW gave us, once.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

/// DWM's latest vblank: its count and its QPC time in 100 ns, for a swap chain whose frame
/// statistics say nothing.
pub fn dwm_vblank() -> Option<(u64, i64)> {
    let mut info = DWM_TIMING_INFO {
        cbSize: size_of::<DWM_TIMING_INFO>() as u32,
        ..Default::default()
    };
    // SAFETY: `info` is ours and sized; a null window asks for the desktop's timing.
    unsafe { DwmGetCompositionTimingInfo(HWND::default(), &mut info) }.ok()?;
    let counter = i64::try_from(info.qpcVBlank).ok()?;
    Some((info.cRefresh, qpc_to_hns(counter)))
}

/// Milliseconds since the session's last keyboard or mouse input.
pub fn idle_ms() -> u32 {
    let mut info = LASTINPUTINFO {
        cbSize: size_of::<LASTINPUTINFO>() as u32,
        dwTime: 0,
    };
    // SAFETY: `info` is ours and sized.
    if unsafe { GetLastInputInfo(&mut info) }.as_bool() {
        // SAFETY: reads the tick count.
        unsafe { GetTickCount() }.wrapping_sub(info.dwTime)
    } else {
        0
    }
}

/// The foreground lock time-out, in ms (`SPI_GETFOREGROUNDLOCKTIMEOUT`, 200,000 by default).
pub fn foreground_lock_timeout_ms() -> u32 {
    let mut ms = 200_000_u32;
    // SAFETY: this action writes one u32 into `ms`, which outlives the call.
    let read = unsafe {
        SystemParametersInfoW(
            SPI_GETFOREGROUNDLOCKTIMEOUT,
            0,
            Some((&raw mut ms).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    if read.is_ok() { ms } else { 200_000 }
}

/// Whether the window was asked to close.
pub fn close_requested() -> bool {
    CLOSE.load(Ordering::Relaxed)
}

/// The window's kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Frame {
    /// A popup with no frame (borderless and exclusive).
    Popup,
    /// A framed window that can't be resized.
    Framed,
}

/// The testbed's window, "Kairo testbed".
pub struct Window {
    hwnd: HWND,
}

impl Window {
    /// A hidden window whose client area is `width` × `height` at (`left`, `top`) in desktop
    /// pixels (the process is per-monitor-v2 DPI aware).
    pub fn create(
        frame: Frame,
        left: i32,
        top: i32,
        width: i32,
        height: i32,
    ) -> Result<Window, String> {
        // SAFETY: a null name asks for this program's own module handle.
        let module = unsafe { GetModuleHandleW(None) }.map_err(|e| format!("module: {e}"))?;
        // SAFETY: a stock object, which must not be deleted (and isn't).
        let black = HBRUSH(unsafe { GetStockObject(BLACK_BRUSH) }.0);
        let class_name = w!("KairoTestbed");
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(window_proc),
            hInstance: module.into(),
            hbrBackground: black,
            lpszClassName: class_name,
            ..Default::default()
        };
        // SAFETY: `class` is complete and its strings are static.
        if unsafe { RegisterClassExW(&class) } == 0 {
            return Err(format!("window class: {}", std::io::Error::last_os_error()));
        }
        let style = match frame {
            Frame::Popup => WS_POPUP,
            Frame::Framed => WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX,
        };
        // SAFETY: the class is registered and the strings are static.
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                class_name,
                w!("Kairo testbed"),
                style,
                left,
                top,
                width,
                height,
                None,
                None,
                Some(module.into()),
                None,
            )
        }
        .map_err(|e| format!("window: {e}"))?;
        let window = Window { hwnd };
        if frame == Frame::Framed {
            window.fit_client(style, left, top, width, height);
        }
        Ok(window)
    }

    /// Grows a framed window so its client area is `width` × `height` at (`left`, `top`), at
    /// the DPI of the monitor it is on.
    fn fit_client(&self, style: WINDOW_STYLE, left: i32, top: i32, width: i32, height: i32) {
        let mut rect = RECT {
            left,
            top,
            right: left + width,
            bottom: top + height,
        };
        // SAFETY: our own window.
        let dpi = unsafe { GetDpiForWindow(self.hwnd) };
        // SAFETY: `rect` is ours.
        let adjusted = unsafe {
            AdjustWindowRectExForDpi(&mut rect, style, false, WINDOW_EX_STYLE::default(), dpi)
        };
        if adjusted.is_ok() {
            // SAFETY: moves our own window; no z-order change and no activation.
            let _ = unsafe {
                SetWindowPos(
                    self.hwnd,
                    None,
                    rect.left,
                    rect.top,
                    rect.right - rect.left,
                    rect.bottom - rect.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                )
            };
        }
    }

    /// The window handle, as a number (for the ready file and DXGI).
    pub fn hwnd(&self) -> isize {
        self.hwnd.0 as isize
    }

    /// Shows the window, taking the foreground only if `activate`.
    pub fn show(&self, activate: bool) {
        let how = if activate { SW_SHOW } else { SW_SHOWNOACTIVATE };
        // SAFETY: our own window.
        let _ = unsafe { ShowWindow(self.hwnd, how) };
    }

    /// Asks for the foreground and says whether the window has it. Windows may refuse even
    /// when the foreground lock time-out has passed; nothing here sends input to get it.
    pub fn take_foreground(&self) -> bool {
        self.show(true);
        // SAFETY: asks for our own window to come to the front.
        let _ = unsafe { SetForegroundWindow(self.hwnd) };
        // SAFETY: reads which window is in front.
        (unsafe { GetForegroundWindow() }) == self.hwnd
    }

    /// The client area in desktop pixels: left, top, width, height.
    pub fn client_rect(&self) -> (i32, i32, i32, i32) {
        let mut rect = RECT::default();
        // SAFETY: our own window; `rect` is ours.
        if unsafe { GetClientRect(self.hwnd, &mut rect) }.is_err() {
            return (0, 0, 0, 0);
        }
        let mut origin = POINT { x: 0, y: 0 };
        // SAFETY: our own window; `origin` is ours.
        let _ = unsafe { ClientToScreen(self.hwnd, &mut origin) };
        (
            origin.x,
            origin.y,
            rect.right - rect.left,
            rect.bottom - rect.top,
        )
    }

    /// Handles every waiting message.
    pub fn pump(&self) {
        let mut message = MSG::default();
        // SAFETY: the message loop of the thread that made the window; `message` is ours.
        while unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE) }.as_bool() {
            // SAFETY: `message` was filled in by PeekMessageW.
            let _ = unsafe { TranslateMessage(&message) };
            // SAFETY: as above.
            unsafe { DispatchMessageW(&message) };
        }
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        // SAFETY: destroys our own window, once, on the thread that made it.
        let _ = unsafe { DestroyWindow(self.hwnd) };
    }
}

extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // A panic must never unwind into Windows.
    std::panic::catch_unwind(|| handle_message(window, message, wparam, lparam))
        .unwrap_or(LRESULT(0))
}

fn handle_message(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        // The run loop ends the run, writes its last records and then destroys the window.
        WM_CLOSE => {
            CLOSE.store(true, Ordering::Relaxed);
            LRESULT(0)
        }
        // No cursor over the client area (README §4.1).
        WM_SETCURSOR if (lparam.0 & 0xFFFF) as u32 == HTCLIENT => {
            // SAFETY: hides the cursor while it is over our window.
            unsafe { SetCursor(None) };
            LRESULT(1)
        }
        // The swap chain covers the client area: nothing to erase.
        WM_ERASEBKGND => LRESULT(1),
        // SAFETY: Windows' default processing of a message sent to our own window.
        _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
    }
}
