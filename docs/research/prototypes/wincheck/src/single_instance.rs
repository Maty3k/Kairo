use windows::core::HSTRING;
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS, HANDLE};
use windows::Win32::System::Threading::CreateMutexW;
/// Ok(Some(h)) = we own it; Ok(None) = another control runs in this session (forward args over the pipe).
pub fn acquire(sid: &str) -> windows::core::Result<Option<HANDLE>> {
    let name = HSTRING::from(format!(r"Local\Kairo.Control.{sid}"));
    let h = unsafe { CreateMutexW(None, true, &name)? };
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS { return Ok(None); } // nothing called in between
    Ok(Some(h))
}
