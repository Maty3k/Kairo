// SPDX-License-Identifier: GPL-3.0-or-later
//! The Win32 calls M1's instruments share (docs/research/17-loadgen-bench/README.md §3.1, §4.3;
//! written for the testbed, research 16 §4.1–4.3): QPC time, the EcoQoS opt-out, the display
//! request, the session's idle time and the foreground lock time-out, a window and its messages,
//! taking the foreground, and the black [`Cover`] over another display. Every call is on this
//! process, its own threads or its own windows: nothing here synthesises input, hooks anything,
//! attaches to another thread's input or opens another process (CLAUDE.md house rules 1 and 4).

use std::fmt;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use kairo_core::timeline;
use kairo_display::Rect;
use windows::Win32::Foundation::{
    CloseHandle, ERROR_CLASS_ALREADY_EXISTS, HANDLE, HMODULE, HWND, LPARAM, LRESULT, POINT, RECT,
    WAIT_OBJECT_0, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    BLACK_BRUSH, ClientToScreen, GetStockObject, HBRUSH, UpdateWindow,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows::Win32::System::Power::{ES_CONTINUOUS, ES_DISPLAY_REQUIRED, SetThreadExecutionState};
use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::System::Threading::{
    CreateEventW, GetCurrentProcess, INFINITE, PROCESS_POWER_THROTTLING_CURRENT_VERSION,
    PROCESS_POWER_THROTTLING_EXECUTION_SPEED, PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION,
    PROCESS_POWER_THROTTLING_STATE, ProcessPowerThrottling, SetEvent, SetProcessInformation,
};
use windows::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForWindow};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect,
    GetForegroundWindow, GetWindowDisplayAffinity, GetWindowRect, HTCLIENT, MA_NOACTIVATE, MSG,
    MWMO_INPUTAVAILABLE, MsgWaitForMultipleObjectsEx, PM_REMOVE, PeekMessageW, QS_ALLINPUT,
    RegisterClassExW, SPI_GETFOREGROUNDLOCKTIMEOUT, SW_SHOW, SW_SHOWNOACTIVATE, SWP_NOACTIVATE,
    SWP_NOZORDER, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SetCursor, SetForegroundWindow,
    SetWindowDisplayAffinity, SetWindowPos, ShowWindow, SystemParametersInfoW, TranslateMessage,
    WDA_EXCLUDEFROMCAPTURE, WINDOW_EX_STYLE, WINDOW_STYLE, WM_CLOSE, WM_ERASEBKGND,
    WM_MOUSEACTIVATE, WM_SETCURSOR, WNDCLASSEXW, WNDPROC, WS_CAPTION, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_MINIMIZEBOX, WS_OVERLAPPED, WS_POPUP, WS_SYSMENU,
};
use windows::core::{HSTRING, PCWSTR};

/// Set when the window gets `WM_CLOSE`.
static CLOSE: AtomicBool = AtomicBool::new(false);

/// QPC's frequency in counts per second, read once: the logs' `qpc_hz`.
pub fn qpc_frequency() -> i64 {
    static FREQUENCY: OnceLock<i64> = OnceLock::new();
    *FREQUENCY.get_or_init(|| {
        let mut f = 0_i64;
        // SAFETY: writes one i64 we own; it can't fail on Windows XP or later.
        let _ = unsafe { QueryPerformanceFrequency(&mut f) };
        f
    })
}

/// A QPC reading in 100 ns, the unit of every time in the logs.
pub fn qpc_to_hns(counter: i64) -> i64 {
    timeline::hns_from_qpc(counter, qpc_frequency()).unwrap_or(0)
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

/// Keeps the screens on while it lives (`ES_DISPLAY_REQUIRED`, research 16 §4.1, §8.1). It never
/// wakes a dark display with input.
pub struct KeepDisplayOn;

// A guard whose making asks Windows for something: `Default` would hide that.
#[allow(clippy::new_without_default)]
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

/// Registers a black window class of this program's module, named `name`, with `proc`. A class
/// this process registered already (an earlier window of the same kind) is used as it is.
fn register_class(module: HMODULE, name: &HSTRING, proc: WNDPROC) -> Result<(), String> {
    // SAFETY: a stock object, which must not be deleted (and isn't).
    let black = HBRUSH(unsafe { GetStockObject(BLACK_BRUSH) }.0);
    let class = WNDCLASSEXW {
        cbSize: size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: proc,
        hInstance: module.into(),
        hbrBackground: black,
        lpszClassName: PCWSTR(name.as_ptr()),
        ..Default::default()
    };
    // SAFETY: `class` is complete, and `name` outlives the call, which copies it.
    if unsafe { RegisterClassExW(&class) } == 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() != Some(ERROR_CLASS_ALREADY_EXISTS.0 as i32) {
            return Err(format!("window class: {error}"));
        }
    }
    Ok(())
}

/// The window's kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Frame {
    /// A popup with no frame (borderless and exclusive).
    Popup,
    /// A framed window that can't be resized.
    Framed,
}

/// A program's window: the testbed's "Kairo testbed", loadgen's "Kairo loadgen".
pub struct Window {
    hwnd: HWND,
}

impl Window {
    /// A hidden window of class `class` with the title `title`, whose client area is `width` ×
    /// `height` at (`left`, `top`) in desktop pixels (the process is per-monitor-v2 DPI aware).
    pub fn create(
        class: &str,
        title: &str,
        frame: Frame,
        left: i32,
        top: i32,
        width: i32,
        height: i32,
    ) -> Result<Window, String> {
        // SAFETY: a null name asks for this program's own module handle.
        let module = unsafe { GetModuleHandleW(None) }.map_err(|e| format!("module: {e}"))?;
        let class = HSTRING::from(class);
        let title = HSTRING::from(title);
        register_class(module, &class, Some(window_proc))?;
        let style = match frame {
            Frame::Popup => WS_POPUP,
            Frame::Framed => WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX,
        };
        // SAFETY: the class is registered, and both strings outlive the call.
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                &class,
                &title,
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

    /// The whole window, frame included, in desktop pixels: left, top, right, bottom.
    pub fn outer_rect(&self) -> Option<(i32, i32, i32, i32)> {
        let mut rect = RECT::default();
        // SAFETY: our own window; `rect` is ours.
        unsafe { GetWindowRect(self.hwnd, &mut rect) }.ok()?;
        Some((rect.left, rect.top, rect.right, rect.bottom))
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
        // No cursor over the client area (research 16 §4.1).
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

/// How long [`take_foreground`] keeps asking for the foreground (`--activate`, research 16
/// §4.1).
pub const ACTIVATE_FOR: Duration = Duration::from_secs(10);

/// How a wait for the foreground lock ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockWait {
    /// The session's last input is old enough for Windows to let the window take the
    /// foreground.
    Ready,
    /// The window was asked to close (`WM_CLOSE`).
    Closed,
    /// Standard input ended (`--stop-on-stdin-eof`).
    StdinEof,
}

/// Waits, handling `window`'s messages, until the session's last input is older than the
/// foreground lock time-out plus 5 s (research 16 §4.1), or the window is asked to close, or
/// `stdin_eof` is set.
pub fn wait_for_foreground_lock(window: &Window, stdin_eof: &AtomicBool) -> LockWait {
    loop {
        window.pump();
        if close_requested() {
            return LockWait::Closed;
        }
        if stdin_eof.load(Ordering::Relaxed) {
            return LockWait::StdinEof;
        }
        let need = u64::from(foreground_lock_timeout_ms()) + 5_000;
        if u64::from(idle_ms()) >= need {
            return LockWait::Ready;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// Asks for the foreground for up to [`ACTIVATE_FOR`]; never with input.
pub fn take_foreground(window: &Window) -> bool {
    let until = Instant::now() + ACTIVATE_FOR;
    loop {
        if window.take_foreground() {
            return true;
        }
        window.pump();
        if Instant::now() >= until {
            return false;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// The covers' window class.
const COVER_CLASS: &str = "KairoCover";
/// The covers' window title.
const COVER_TITLE: &str = "Kairo cover";

/// Whether a [`Cover`] is hidden from screen capture (research 17 §4.3, Q4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureExclusion {
    /// Capture sees the cover as the black it is: the bench's covers (Q4's recommendation).
    Off,
    /// Hidden from capture, which then sees what is under it, for a run that captures a
    /// covered display (the runner's "capture-excluded" cover, PLAN §3.1 and M1):
    /// `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)` on the cover's own window, set and read
    /// back before it is shown (Windows 10 2004 and later).
    On,
}

/// Why a [`Cover`] couldn't be shown. A cover that fails is gone, its thread with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoverError {
    /// The rectangle asked for is empty.
    Empty,
    /// Its thread, its window class or its window couldn't be made.
    Window(String),
    /// Capture exclusion was refused, or read back as another affinity.
    Exclusion(String),
    /// Once shown, the window isn't where it was asked to be.
    Misplaced {
        /// The rectangle asked for.
        want: Rect,
        /// The window's rectangle.
        got: Rect,
    },
}

impl fmt::Display for CoverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "the cover's rectangle is empty"),
            Self::Window(why) => write!(f, "the cover's window: {why}"),
            Self::Exclusion(why) => write!(f, "the cover's capture exclusion: {why}"),
            Self::Misplaced { want, got } => {
                write!(f, "the cover's window is at {got:?}, not {want:?}")
            }
        }
    }
}

impl std::error::Error for CoverError {}

/// A black, topmost window over one display's rectangle that never takes focus (research 17
/// §4.3, "The covers"; the OLED cover of PLAN §3.1): `WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW` (no
/// activation, no taskbar button, not in Alt+Tab), shown without activation, and a click on it
/// activates nothing. It has its own thread and message pump, so it stays black and answers
/// Windows while its owner waits on children. It hides the cursor over it, so nothing stands
/// still on an OLED it covers, and only its owner closes it: dropping the `Cover` destroys the
/// window and joins the thread. It dies with the process, so a job stopped by input takes its
/// covers with it.
pub struct Cover {
    /// The window, as a number; it belongs to the cover's thread.
    hwnd: isize,
    /// The rectangle asked for.
    rect: Rect,
    /// The event that ends the cover's thread, as a number so the cover can move between
    /// threads.
    stop: isize,
    thread: Option<JoinHandle<()>>,
}

impl Cover {
    /// Shows a cover over `rect` (desktop pixels: the process should be per-monitor-v2 DPI
    /// aware, `kairo_display::make_dpi_aware()`) and returns once it is up: hidden from capture
    /// if `exclusion` says so, shown without activation, and its window's rectangle read back
    /// and found equal to `rect`.
    pub fn show(rect: Rect, exclusion: CaptureExclusion) -> Result<Cover, CoverError> {
        if rect.width() == 0 || rect.height() == 0 {
            return Err(CoverError::Empty);
        }
        // SAFETY: an unnamed manual-reset event, not set, with default security; the handle is
        // ours and closed in Drop.
        let stop = unsafe { CreateEventW(None, true, false, PCWSTR::null()) }
            .map_err(|e| CoverError::Window(format!("its stop event: {e}")))?;
        let mut cover = Cover {
            hwnd: 0,
            rect,
            stop: stop.0 as isize,
            thread: None,
        };
        let (sender, receiver) = mpsc::channel();
        let stop = cover.stop;
        let thread = std::thread::Builder::new()
            .name("cover".to_owned())
            .spawn(move || cover_thread(rect, exclusion, stop, &sender))
            .map_err(|e| CoverError::Window(format!("its thread: {e}")))?;
        cover.thread = Some(thread);
        match receiver.recv() {
            Ok(Ok(hwnd)) => {
                cover.hwnd = hwnd;
                Ok(cover)
            }
            // The thread has ended, or is ending: dropping `cover` joins it.
            Ok(Err(e)) => Err(e),
            Err(_) => Err(CoverError::Window("its thread ended".to_owned())),
        }
    }

    /// The window handle, as a number.
    pub fn hwnd(&self) -> isize {
        self.hwnd
    }

    /// The rectangle it was asked to cover.
    pub fn rect(&self) -> Rect {
        self.rect
    }

    /// The window's rectangle now, in desktop pixels.
    pub fn window_rect(&self) -> Option<Rect> {
        window_rect(HWND(self.hwnd as *mut _))
    }
}

impl Drop for Cover {
    fn drop(&mut self) {
        let stop = HANDLE(self.stop as *mut _);
        // SAFETY: our own event, open until the end of this function.
        let _ = unsafe { SetEvent(stop) };
        if let Some(thread) = self.thread.take() {
            // The thread destroys its window and ends as soon as it sees the event.
            let _ = thread.join();
        }
        // SAFETY: the event CreateEventW gave us, closed once, after the thread that waited on
        // it has ended.
        let _ = unsafe { CloseHandle(stop) };
    }
}

/// A window's rectangle in desktop pixels.
fn window_rect(hwnd: HWND) -> Option<Rect> {
    let mut rect = RECT::default();
    // SAFETY: `rect` is ours; GetWindowRect reads a window of any thread.
    unsafe { GetWindowRect(hwnd, &mut rect) }.ok()?;
    Some(Rect {
        left: rect.left,
        top: rect.top,
        right: rect.right,
        bottom: rect.bottom,
    })
}

/// The cover's thread: make and show the window, say how that went, then handle its messages
/// until the owner sets `stop`, and destroy it.
fn cover_thread(
    rect: Rect,
    exclusion: CaptureExclusion,
    stop: isize,
    shown: &mpsc::Sender<Result<isize, CoverError>>,
) {
    let window = match open_cover(rect, exclusion) {
        Ok(window) => window,
        Err(e) => {
            let _ = shown.send(Err(e));
            return;
        }
    };
    let _ = shown.send(Ok(window.0 as isize));
    pump_until(HANDLE(stop as *mut _));
    // SAFETY: destroys the cover's own window, once, on the thread that made it.
    let _ = unsafe { DestroyWindow(window) };
}

/// Makes the cover's window on this thread and shows it; destroys it again if that fails.
fn open_cover(rect: Rect, exclusion: CaptureExclusion) -> Result<HWND, CoverError> {
    let failed = |what: &str, e: windows::core::Error| CoverError::Window(format!("{what}: {e}"));
    let (Ok(width), Ok(height)) = (i32::try_from(rect.width()), i32::try_from(rect.height()))
    else {
        return Err(CoverError::Window("the rectangle is too large".to_owned()));
    };
    // SAFETY: a null name asks for this program's own module handle.
    let module = unsafe { GetModuleHandleW(None) }.map_err(|e| failed("module", e))?;
    let class = HSTRING::from(COVER_CLASS);
    let title = HSTRING::from(COVER_TITLE);
    register_class(module, &class, Some(cover_proc)).map_err(CoverError::Window)?;
    // SAFETY: the class is registered, and both strings outlive the call.
    let window = unsafe {
        CreateWindowExW(
            WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
            &class,
            &title,
            WS_POPUP,
            rect.left,
            rect.top,
            width,
            height,
            None,
            None,
            Some(module.into()),
            None,
        )
    }
    .map_err(|e| failed("window", e))?;
    match show_cover(window, rect, exclusion) {
        Ok(()) => Ok(window),
        Err(e) => {
            // SAFETY: the window made above on this thread, destroyed once.
            let _ = unsafe { DestroyWindow(window) };
            Err(e)
        }
    }
}

/// Excludes the new window from capture if asked, shows it without activation, paints it black
/// and checks where it is.
fn show_cover(window: HWND, rect: Rect, exclusion: CaptureExclusion) -> Result<(), CoverError> {
    if exclusion == CaptureExclusion::On {
        let refused = |e: windows::core::Error| CoverError::Exclusion(e.to_string());
        // SAFETY: our own top-level window, not yet shown.
        unsafe { SetWindowDisplayAffinity(window, WDA_EXCLUDEFROMCAPTURE) }.map_err(refused)?;
        let mut affinity = 0_u32;
        // SAFETY: our own window; `affinity` is ours.
        unsafe { GetWindowDisplayAffinity(window, &mut affinity) }.map_err(refused)?;
        if affinity != WDA_EXCLUDEFROMCAPTURE.0 {
            return Err(CoverError::Exclusion(format!(
                "the window's affinity reads {affinity:#x}"
            )));
        }
    }
    // SAFETY: shows our own window without activating it.
    let _ = unsafe { ShowWindow(window, SW_SHOWNOACTIVATE) };
    // SAFETY: paints our own window now (black, its class's brush), on the thread that made it.
    let _ = unsafe { UpdateWindow(window) };
    let got = window_rect(window)
        .ok_or_else(|| CoverError::Window("its rectangle can't be read".to_owned()))?;
    if got != rect {
        return Err(CoverError::Misplaced { want: rect, got });
    }
    Ok(())
}

/// Handles this thread's messages until `stop` is set.
fn pump_until(stop: HANDLE) {
    let mut message = MSG::default();
    loop {
        // SAFETY: this thread's own queue; `message` is ours.
        while unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE) }.as_bool() {
            // SAFETY: `message` was filled in by PeekMessageW.
            let _ = unsafe { TranslateMessage(&message) };
            // SAFETY: as above.
            unsafe { DispatchMessageW(&message) };
        }
        // SAFETY: waits for our own event, or for a message to this thread, even one already
        // seen.
        let woke = unsafe {
            MsgWaitForMultipleObjectsEx(Some(&[stop]), INFINITE, QS_ALLINPUT, MWMO_INPUTAVAILABLE)
        };
        // WAIT_OBJECT_0 + 1 is a message; the event, or a failed wait, ends the cover.
        if woke.0 != WAIT_OBJECT_0.0 + 1 {
            return;
        }
    }
}

extern "system" fn cover_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // A panic must never unwind into Windows.
    std::panic::catch_unwind(|| cover_message(window, message, wparam, lparam))
        .unwrap_or(LRESULT(0))
}

fn cover_message(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        // A click on the cover activates nothing.
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        // Only its owner closes it (Drop).
        WM_CLOSE => LRESULT(0),
        // No cursor over it, so nothing stands still on an OLED it covers.
        WM_SETCURSOR if (lparam.0 & 0xFFFF) as u32 == HTCLIENT => {
            // SAFETY: hides the cursor while it is over our window.
            unsafe { SetCursor(None) };
            LRESULT(1)
        }
        // SAFETY: Windows' default processing of a message sent to our own window, which
        // erases and paints it with its class's black brush.
        _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::UI::WindowsAndMessaging::{
        GCLP_HBRBACKGROUND, GWL_EXSTYLE, GWL_STYLE, GetClassLongPtrW, GetWindowLongPtrW, IsWindow,
        WDA_NONE, WS_VISIBLE,
    };

    #[test]
    fn qpc_time_moves_forward_in_100_ns() {
        let a = now_hns();
        std::thread::sleep(Duration::from_millis(20));
        let b = now_hns();
        assert!(a > 0 && b > a, "{a} then {b}");
        // At least the 20 ms slept (a busy runner may take far longer).
        assert!((200_000..100_000_000).contains(&(b - a)), "{}", b - a);
        // One second of counts is 10,000,000 units of 100 ns.
        assert_eq!(qpc_to_hns(qpc_frequency()), 10_000_000);
    }

    // Moved from kairo-harness with its copy of KeepDisplayOn.
    #[test]
    fn the_display_request_comes_and_goes() {
        let keep = KeepDisplayOn::new();
        drop(keep);
    }

    /// The first active display's rectangle, or `None` when the runner has no usable desktop:
    /// research 17 §6's W row reports that rather than failing.
    fn a_display() -> Option<Rect> {
        kairo_display::make_dpi_aware();
        let rect = kairo_display::targets().ok()?.first()?.rect;
        (rect.width() > 0 && rect.height() > 0).then_some(rect)
    }

    /// Whether CI says it has a desktop (`KAIRO_CI_DESKTOP`, set by Windows CI's test step): there
    /// a missing display or a refused exclusion fails, since a passing test's report never shows.
    fn on_ci_desktop() -> bool {
        std::env::var_os("KAIRO_CI_DESKTOP").is_some()
    }

    /// Reports `why` and skips the test, or fails it on CI's desktop.
    fn skip(why: &str) {
        assert!(!on_ci_desktop(), "KAIRO_CI_DESKTOP is set: {why}");
        eprintln!("{why}: reported, not failed");
    }

    fn styles(cover: &Cover) -> (u32, u32) {
        let window = HWND(cover.hwnd() as *mut _);
        // SAFETY: reads the cover's own window's styles.
        let style = unsafe { GetWindowLongPtrW(window, GWL_STYLE) } as u32;
        // SAFETY: as above.
        let ex_style = unsafe { GetWindowLongPtrW(window, GWL_EXSTYLE) } as u32;
        (style, ex_style)
    }

    #[test]
    fn an_empty_rectangle_gets_no_cover() {
        let empty = Rect {
            left: 10,
            top: 10,
            right: 10,
            bottom: 500,
        };
        assert_eq!(
            Cover::show(empty, CaptureExclusion::Off).err(),
            Some(CoverError::Empty)
        );
    }

    #[test]
    fn a_cover_is_black_topmost_never_in_front_and_goes_when_dropped() {
        let Some(rect) = a_display() else {
            return skip("no usable desktop here");
        };
        let cover = Cover::show(rect, CaptureExclusion::Off).unwrap();
        let window = HWND(cover.hwnd() as *mut _);
        assert_eq!(cover.rect(), rect);
        assert_eq!(cover.window_rect(), Some(rect));
        let (style, ex_style) = styles(&cover);
        assert_eq!(
            style & (WS_POPUP.0 | WS_VISIBLE.0),
            WS_POPUP.0 | WS_VISIBLE.0
        );
        assert_eq!(style & WS_CAPTION.0, 0);
        let wanted = WS_EX_NOACTIVATE.0 | WS_EX_TOOLWINDOW.0 | WS_EX_TOPMOST.0;
        assert_eq!(ex_style & wanted, wanted);
        // Black by its class's brush; nothing reads the screen to check it.
        // SAFETY: reads the cover's own class's background brush, and a stock object.
        let (brush, black) = unsafe {
            (
                GetClassLongPtrW(window, GCLP_HBRBACKGROUND),
                GetStockObject(BLACK_BRUSH).0 as usize,
            )
        };
        assert_eq!(brush, black);
        // SAFETY: reads which window is in front.
        assert_ne!(unsafe { GetForegroundWindow() }, window);
        let mut affinity = u32::MAX;
        // SAFETY: reads the cover's own window's affinity into a local.
        unsafe { GetWindowDisplayAffinity(window, &mut affinity) }.unwrap();
        assert_eq!(affinity, WDA_NONE.0, "capture sees it");
        drop(cover);
        // SAFETY: asks whether the handle still names a window.
        assert!(!unsafe { IsWindow(Some(window)) }.as_bool());
    }

    #[test]
    fn a_cover_can_be_hidden_from_capture() {
        let Some(rect) = a_display() else {
            return skip("no usable desktop here");
        };
        match Cover::show(rect, CaptureExclusion::On) {
            Ok(cover) => {
                let mut affinity = 0_u32;
                // SAFETY: reads the cover's own window's affinity into a local.
                unsafe { GetWindowDisplayAffinity(HWND(cover.hwnd() as *mut _), &mut affinity) }
                    .unwrap();
                assert_eq!(affinity, WDA_EXCLUDEFROMCAPTURE.0);
                assert_eq!(cover.window_rect(), Some(rect));
            }
            // A desktop that DWM doesn't compose refuses it, which is reported, not failed; on CI's
            // desktop every exclusion error fails, a wrong read-back included.
            Err(CoverError::Exclusion(why)) => skip(&format!("capture exclusion failed ({why})")),
            Err(e) => panic!("{e}"),
        }
    }
}
