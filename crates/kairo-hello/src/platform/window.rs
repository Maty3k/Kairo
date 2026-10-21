// SPDX-License-Identifier: GPL-3.0-or-later
//! The probe's window. It is centred on the primary monitor, shows short instructions, and
//! reports each key pressed while it has the keyboard focus. Keys come only from its own
//! WM_KEYDOWN and WM_SYSKEYDOWN messages: no hooks, nothing global.

use std::io;
use std::sync::OnceLock;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, COLOR_WINDOW, CreateFontW, DEFAULT_CHARSET,
    DEFAULT_PITCH, DT_EXPANDTABS, DT_LEFT, DT_NOPREFIX, DT_TOP, DT_WORDBREAK, DeleteObject,
    DrawTextW, EndPaint, FF_SWISS, FW_NORMAL, GetMonitorInfoW, GetSysColorBrush, HDC,
    InvalidateRect, MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromPoint, OUT_DEFAULT_PRECIS,
    PAINTSTRUCT, SelectObject, SetBkMode, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DispatchMessageW,
    GetClientRect, GetMessageW, IDC_ARROW, LoadCursorW, MSG, PostQuitMessage, RegisterClassW,
    SetForegroundWindow, TranslateMessage, WINDOW_EX_STYLE, WM_DESTROY, WM_KEYDOWN, WM_PAINT,
    WM_SYSKEYDOWN, WNDCLASSW, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
};
use windows::core::{HSTRING, w};

/// What the window calls back into.
pub struct Callbacks {
    /// Called with the virtual-key code of each new key press (auto-repeat is ignored).
    pub on_key: Box<dyn Fn(u16) + Send + Sync>,
    /// The text to show, asked for on every repaint.
    pub text: Box<dyn Fn() -> String + Send + Sync>,
}

static CALLBACKS: OnceLock<Callbacks> = OnceLock::new();

const WIDTH: i32 = 760;
const HEIGHT: i32 = 460;
const MARGIN: i32 = 24;
/// Bit 30 of a key message's lParam: the key was already down, so this is auto-repeat.
const KEY_WAS_DOWN: isize = 1 << 30;

/// Opens the window and runs its message loop until the window is closed.
pub fn run(title: &str, callbacks: Callbacks) -> io::Result<()> {
    if CALLBACKS.set(callbacks).is_err() {
        return Err(io::Error::other("the window can only be opened once"));
    }
    // SAFETY: a null name asks for this program's own module handle.
    let module = unsafe { GetModuleHandleW(None) }?;
    // SAFETY: loads a predefined system cursor.
    let cursor = unsafe { LoadCursorW(None, IDC_ARROW) }?;
    // SAFETY: returns a system colour brush, which must not be deleted (and is not).
    let background = unsafe { GetSysColorBrush(COLOR_WINDOW) };
    let class_name = w!("KairoHelloWindow");
    let class = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(window_proc),
        hInstance: module.into(),
        hCursor: cursor,
        hbrBackground: background,
        lpszClassName: class_name,
        ..Default::default()
    };
    // SAFETY: `class` is fully initialised and its strings are static.
    if unsafe { RegisterClassW(&class) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let (x, y) = centred_on_primary_monitor(WIDTH, HEIGHT);
    let title = HSTRING::from(title);
    // SAFETY: the class is registered and `title` outlives the call.
    let window = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class_name,
            &title,
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            x,
            y,
            WIDTH,
            HEIGHT,
            None,
            None,
            Some(module.into()),
            None,
        )
    }?;
    // SAFETY: best effort; Windows may keep the focus where it is (then the taskbar flashes).
    let _ = unsafe { SetForegroundWindow(window) };
    let mut message = MSG::default();
    loop {
        // SAFETY: the standard message loop, on the thread that created the window.
        let result = unsafe { GetMessageW(&mut message, None, 0, 0) };
        match result.0 {
            0 => return Ok(()),
            -1 => return Err(io::Error::last_os_error()),
            _ => {
                // SAFETY: `message` was filled in by GetMessageW.
                let _ = unsafe { TranslateMessage(&message) };
                // SAFETY: as above.
                unsafe { DispatchMessageW(&message) };
            }
        }
    }
}

extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // A panic must never unwind into Windows; treat it as "handled".
    std::panic::catch_unwind(|| handle_message(window, message, wparam, lparam))
        .unwrap_or(LRESULT(0))
}

fn handle_message(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            if lparam.0 & KEY_WAS_DOWN == 0 {
                if let Some(callbacks) = CALLBACKS.get() {
                    (callbacks.on_key)(wparam.0 as u16);
                }
                // SAFETY: asks Windows to repaint our own window, to show the new count.
                let _ = unsafe { InvalidateRect(Some(window), None, true) };
            }
            // Default handling keeps Alt+F4 and the other system keys working.
            default_handling(window, message, wparam, lparam)
        }
        WM_PAINT => {
            paint(window);
            LRESULT(0)
        }
        WM_DESTROY => {
            // SAFETY: ends this thread's message loop.
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => default_handling(window, message, wparam, lparam),
    }
}

fn default_handling(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: Windows' default processing of a message sent to our own window.
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}

fn paint(window: HWND) {
    let mut paint = PAINTSTRUCT::default();
    // SAFETY: called for WM_PAINT on our own window; EndPaint below pairs with it.
    let dc = unsafe { BeginPaint(window, &mut paint) };
    if !dc.is_invalid() {
        draw_text(window, dc);
    }
    // SAFETY: pairs with BeginPaint above.
    let _ = unsafe { EndPaint(window, &paint) };
}

fn draw_text(window: HWND, dc: HDC) {
    let mut area = RECT::default();
    // SAFETY: `area` is a valid place for the client rectangle.
    if unsafe { GetClientRect(window, &mut area) }.is_err() {
        return;
    }
    area.left += MARGIN;
    area.top += MARGIN;
    area.right -= MARGIN;
    area.bottom -= MARGIN;
    let text = CALLBACKS
        .get()
        .map(|callbacks| (callbacks.text)())
        .unwrap_or_default();
    let mut text: Vec<u16> = text.encode_utf16().collect();
    // SAFETY: creates a GDI font, deleted below.
    let font = unsafe {
        CreateFontW(
            -20,
            0,
            0,
            0,
            FW_NORMAL.0 as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            u32::from(DEFAULT_PITCH.0 | FF_SWISS.0),
            w!("Segoe UI"),
        )
    };
    // SAFETY: `dc` is the paint DC; selecting a null font just fails.
    let previous = unsafe { SelectObject(dc, font.into()) };
    // SAFETY: changes only this DC's state.
    unsafe { SetBkMode(dc, TRANSPARENT) };
    let format = DT_LEFT | DT_TOP | DT_WORDBREAK | DT_NOPREFIX | DT_EXPANDTABS;
    // SAFETY: `text` and `area` outlive the call.
    unsafe { DrawTextW(dc, &mut text, &mut area, format) };
    // SAFETY: puts the DC's original font back before ours is deleted.
    unsafe { SelectObject(dc, previous) };
    // SAFETY: the font is no longer selected into any DC.
    let _ = unsafe { DeleteObject(font.into()) };
}

fn centred_on_primary_monitor(width: i32, height: i32) -> (i32, i32) {
    // SAFETY: no preconditions; MONITOR_DEFAULTTOPRIMARY always yields a monitor.
    let monitor = unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) };
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: `info` is a MONITORINFO with cbSize set.
    if !unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
        return (CW_USEDEFAULT, CW_USEDEFAULT);
    }
    let work = info.rcWork;
    (
        work.left + (work.right - work.left - width).max(0) / 2,
        work.top + (work.bottom - work.top - height).max(0) / 2,
    )
}
