use std::sync::atomic::{AtomicBool, Ordering};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND};
use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
use windows::Win32::System::Threading::*;
use windows::Win32::UI::Accessibility::{SetWinEventHook, HWINEVENTHOOK};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VIRTUAL_KEY};
use windows::Win32::UI::WindowsAndMessaging::{EVENT_SYSTEM_FOREGROUND, GetWindowThreadProcessId, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS};

/// Opt-in, only while bound. High bit only; returns 0 under UIPI (elevated foreground) or on another desktop.
pub fn mouse_poll(stop: &AtomicBool, buttons: [VIRTUAL_KEY; 2], mut on_press: impl FnMut(u8, i64)) -> windows::core::Result<()> {
    let timer = unsafe { CreateWaitableTimerExW(None, PCWSTR::null(), CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, TIMER_ALL_ACCESS.0) }?;
    let due: i64 = -80_000; // 8 ms, relative, 100 ns units
    unsafe { SetWaitableTimer(timer, &due, 8, None, None, false) }?;
    let mut prev = [false; 2];
    while !stop.load(Ordering::Relaxed) {
        unsafe { WaitForSingleObject(timer, 100) };
        for (i, vk) in buttons.iter().enumerate() {
            let down = (unsafe { GetAsyncKeyState(vk.0 as i32) } as u16 & 0x8000) != 0;
            if down && !prev[i] { on_press(i as u8, crate::qpc::now_hns()); }
            prev[i] = down;
        }
    }
    unsafe { CloseHandle(timer) }
}

/// Foreground game elevated? Some(true/false), None = unknown (protected / anti-cheat handle filtering).
pub fn is_elevated(hwnd: HWND) -> Option<bool> {
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut tok = HANDLE::default();
        let ok = OpenProcessToken(h, TOKEN_QUERY, &mut tok).is_ok();
        let _ = CloseHandle(h);
        if !ok { return None; }
        let mut e = TOKEN_ELEVATION::default();
        let mut len = 0u32;
        let r = GetTokenInformation(tok, TokenElevation, Some(&mut e as *mut _ as *mut _), size_of::<TOKEN_ELEVATION>() as u32, &mut len);
        let _ = CloseHandle(tok);
        r.ok().map(|_| e.TokenIsElevated != 0)
    }
}

/// T0 only (needs the message loop). Callback: try_send the HWND (as isize) to the detect thread, nothing else.
pub unsafe extern "system" fn on_foreground(_h: HWINEVENTHOOK, _ev: u32, hwnd: HWND, id_obj: i32, _id_child: i32, _thread: u32, _time: u32) {
    let _ = std::panic::catch_unwind(|| { if id_obj == 0 { let _raw = hwnd.0 as isize; /* tx.try_send(raw) */ } });
}
pub fn hook_foreground() -> HWINEVENTHOOK {
    unsafe { SetWinEventHook(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND, None, Some(on_foreground), 0, 0,
                             WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS) }
}
