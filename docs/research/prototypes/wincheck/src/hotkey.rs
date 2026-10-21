use windows::Win32::Foundation::{ERROR_HOTKEY_ALREADY_REGISTERED, HWND};
use windows::Win32::UI::Input::KeyboardAndMouse::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyStatus { Registered, InUse, Reserved, Invalid, Failed { hresult: i32 } }
#[derive(Clone, Copy)]
pub struct Binding { pub vk: u16, pub ctrl: bool, pub alt: bool, pub shift: bool, pub win: bool }

/// Call on T0 (the thread that created `hwnd`). Same (hwnd,id) stacks instead of replacing: unregister first.
pub fn set(hwnd: HWND, id: i32, b: Binding, was_registered: bool) -> HotkeyStatus {
    if !(0..=0xBFFF).contains(&id) { return HotkeyStatus::Invalid; }
    if b.vk == VK_F12.0 || b.win { return HotkeyStatus::Reserved; }
    if was_registered { let _ = unsafe { UnregisterHotKey(Some(hwnd), id) }; }
    let mut mods = MOD_NOREPEAT;
    if b.ctrl { mods |= MOD_CONTROL; }
    if b.alt { mods |= MOD_ALT; }
    if b.shift { mods |= MOD_SHIFT; }
    match unsafe { RegisterHotKey(Some(hwnd), id, mods, b.vk as u32) } {
        Ok(()) => HotkeyStatus::Registered,
        Err(e) if e.code() == ERROR_HOTKEY_ALREADY_REGISTERED.to_hresult() => HotkeyStatus::InUse,
        Err(e) => HotkeyStatus::Failed { hresult: e.code().0 },
    }
}
pub const ALT_F8: Binding = Binding { vk: VK_F8.0, ctrl: false, alt: true, shift: false, win: false };
