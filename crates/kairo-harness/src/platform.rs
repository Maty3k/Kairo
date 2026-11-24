// SPDX-License-Identifier: GPL-3.0-or-later
//! The Windows calls `run` needs (docs/research/16-verify-testbed/README.md §8.3 phase 0): the
//! displays kept on, no error dialog from a child that can't load a DLL, the free space in the
//! logs folder, and a count of running processes by image name, to refuse a second FFmpeg (one
//! hardware encoder session at a time, M1:94). The snapshot lists processes only and reads
//! their image names; no other process is ever opened (CLAUDE.md).

use std::io;
use std::path::Path;

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
use windows::Win32::System::Diagnostics::Debug::{
    SEM_FAILCRITICALERRORS, SEM_NOGPFAULTERRORBOX, SetErrorMode,
};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Power::{ES_CONTINUOUS, ES_DISPLAY_REQUIRED, SetThreadExecutionState};
use windows::core::HSTRING;

/// `CreateProcess`'s flag for a console program with no console window: FFmpeg, kairo-verify
/// and kairo-tone, started from a hidden job, must never show one on the owner's screen.
pub const CREATE_NO_WINDOW: u32 = windows::Win32::System::Threading::CREATE_NO_WINDOW.0;

/// Keeps the screens on while it lives (`ES_DISPLAY_REQUIRED`, README §8.1). It never wakes a
/// dark display with input.
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

/// No critical-error or fault dialog for this process and the children it starts, which
/// inherit the mode: a child that can't find a DLL then exits with an error code instead of
/// leaving a dialog on the owner's screen.
pub fn no_error_dialogs() {
    // SAFETY: sets this process's error mode; no pointer is passed.
    unsafe { SetErrorMode(SEM_FAILCRITICALERRORS | SEM_NOGPFAULTERRORBOX) };
}

/// The bytes free to this account on the volume of `dir`.
pub fn free_bytes(dir: &Path) -> io::Result<u64> {
    let mut free = 0_u64;
    let name = HSTRING::from(dir.as_os_str());
    // SAFETY: `name` lives across the call; the one out pointer is to a local u64 and the
    // others are null, which the call accepts.
    unsafe { GetDiskFreeSpaceExW(&name, Some(&raw mut free), None, None) }
        .map_err(io::Error::other)?;
    Ok(free)
}

/// A snapshot handle, closed on drop.
struct Snapshot(HANDLE);

impl Drop for Snapshot {
    fn drop(&mut self) {
        // SAFETY: the handle came from CreateToolhelp32Snapshot and is closed once.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

/// How many running processes have the image name `image` (any case). Only the names are read,
/// and none is kept or logged.
pub fn count_processes_named(image: &str) -> io::Result<usize> {
    // SAFETY: a process snapshot of the whole system; the handle is closed by `Snapshot`.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }
        .map(Snapshot)
        .map_err(io::Error::other)?;
    let mut entry = PROCESSENTRY32W {
        dwSize: size_of::<PROCESSENTRY32W>() as u32,
        ..PROCESSENTRY32W::default()
    };
    let mut count = 0;
    // SAFETY: `entry` is a PROCESSENTRY32W with dwSize set, as the call requires.
    let mut more = unsafe { Process32FirstW(snapshot.0, &raw mut entry) }.is_ok();
    while more {
        let len = entry
            .szExeFile
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(entry.szExeFile.len());
        let name = String::from_utf16_lossy(&entry.szExeFile[..len]);
        if name.eq_ignore_ascii_case(image) {
            count += 1;
        }
        // SAFETY: as above, the same entry.
        more = unsafe { Process32NextW(snapshot.0, &raw mut entry) }.is_ok();
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_temp_folder_has_free_space() {
        assert!(free_bytes(&std::env::temp_dir()).unwrap() > 0);
    }

    #[test]
    fn this_test_is_a_running_process() {
        let exe = std::env::current_exe().unwrap();
        let name = exe.file_name().unwrap().to_str().unwrap().to_owned();
        assert!(count_processes_named(&name).unwrap() >= 1);
        assert!(count_processes_named(&name.to_ascii_uppercase()).unwrap() >= 1);
        assert_eq!(count_processes_named("no-such-image-kairo.exe").unwrap(), 0);
    }

    #[test]
    fn the_display_request_comes_and_goes() {
        let keep = KeepDisplayOn::new();
        drop(keep);
        no_error_dialogs();
    }
}
