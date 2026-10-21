// SPDX-License-Identifier: GPL-3.0-or-later
//! Every Win32 call `kairo-hello` makes; the only module with `unsafe`.
//! Signatures follow `windows` 0.62.2.

pub mod pipe;
pub mod window;

use std::io;

use windows::Win32::Foundation::{HANDLE, HLOCAL};
use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows::Win32::Security::{
    GetTokenInformation, TOKEN_ELEVATION, TOKEN_INFORMATION_CLASS, TOKEN_QUERY, TOKEN_USER,
    TokenElevation, TokenSessionId, TokenUser,
};
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::core::{Owned, PWSTR};

/// The Windows session of this process (0 would be the services session).
pub fn session_id() -> io::Result<u32> {
    token_value::<u32>(TokenSessionId)
}

/// True if this process runs elevated (it should not).
pub fn is_elevated() -> io::Result<bool> {
    let elevation = token_value::<TOKEN_ELEVATION>(TokenElevation)?;
    Ok(elevation.TokenIsElevated != 0)
}

/// The current user's SID as a string, e.g. `S-1-5-21-…-1001`.
pub fn current_user_sid() -> io::Result<String> {
    let token = process_token()?;
    let mut needed = 0u32;
    // SAFETY: a size query: no buffer is passed, only the required length is written.
    let _ = unsafe { GetTokenInformation(*token, TokenUser, None, 0, &mut needed) };
    // u64 elements keep the buffer aligned for TOKEN_USER and the SID that follows it.
    let mut buffer = vec![0u64; (needed as usize).div_ceil(8)];
    // SAFETY: the buffer holds `needed` bytes and outlives the call.
    unsafe {
        GetTokenInformation(
            *token,
            TokenUser,
            Some(buffer.as_mut_ptr().cast()),
            needed,
            &mut needed,
        )
    }?;
    // SAFETY: GetTokenInformation filled the buffer with a TOKEN_USER, whose SID pointer points
    // into the same buffer, which lives until the end of this function.
    let user = unsafe { &*buffer.as_ptr().cast::<TOKEN_USER>() };
    let mut text = PWSTR::null();
    // SAFETY: the SID is valid; Windows allocates `text` with LocalAlloc.
    unsafe { ConvertSidToStringSidW(user.User.Sid, &mut text) }?;
    // SAFETY: `text` came from LocalAlloc and is freed only here.
    let _free = unsafe { Owned::new(HLOCAL(text.0.cast())) };
    // SAFETY: ConvertSidToStringSidW returned a NUL-terminated UTF-16 string.
    unsafe { text.to_string() }.map_err(io::Error::other)
}

/// The current `QueryPerformanceCounter` value.
pub fn qpc_now() -> i64 {
    let mut ticks = 0i64;
    // SAFETY: `ticks` is a valid place for the counter; QPC cannot fail on supported Windows.
    let _ = unsafe { QueryPerformanceCounter(&mut ticks) };
    ticks
}

/// `QueryPerformanceFrequency`: counter ticks per second.
pub fn qpc_frequency() -> i64 {
    let mut frequency = 0i64;
    // SAFETY: `frequency` is a valid place for the value; this cannot fail on supported Windows.
    let _ = unsafe { QueryPerformanceFrequency(&mut frequency) };
    frequency
}

fn process_token() -> io::Result<Owned<HANDLE>> {
    let mut token = HANDLE::default();
    // SAFETY: GetCurrentProcess returns a pseudo-handle that needs no closing, and `token` is
    // a valid place for the new handle.
    unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }?;
    // SAFETY: OpenProcessToken succeeded, so `token` is a handle that only we own.
    Ok(unsafe { Owned::new(token) })
}

/// Reads a fixed-size token information class. Callers pair each class with its type:
/// TokenSessionId is a u32, TokenElevation a TOKEN_ELEVATION.
fn token_value<T: Default>(class: TOKEN_INFORMATION_CLASS) -> io::Result<T> {
    let token = process_token()?;
    let mut value = T::default();
    let mut written = 0u32;
    // SAFETY: `value` is a T, the structure `class` writes (see above), and its size is passed.
    unsafe {
        GetTokenInformation(
            *token,
            class,
            Some((&raw mut value).cast()),
            size_of::<T>() as u32,
            &mut written,
        )
    }?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_queries_work() {
        session_id().unwrap();
        is_elevated().unwrap();
        let sid = current_user_sid().unwrap();
        assert!(sid.starts_with("S-1-"), "{sid}");
    }

    #[test]
    fn the_performance_counter_runs() {
        assert!(qpc_frequency() > 0);
        let first = qpc_now();
        assert!(qpc_now() >= first);
    }
}
