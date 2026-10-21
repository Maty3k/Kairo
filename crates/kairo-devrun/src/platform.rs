// SPDX-License-Identifier: GPL-3.0-or-later
//! Every Win32 call `kairo-devrun` makes; the only module with `unsafe`.
//!
//! House rules (docs/PLAN.md Appendix A): other programs are only ever opened with
//! `PROCESS_QUERY_LIMITED_INFORMATION`, and nothing here reaches into another process.
//! Signatures follow `windows` 0.62.2.

use std::io;
use std::os::windows::io::AsRawHandle;
use std::process::Child;

use kairo_core::watch::{self, ScreenRect, WindowFacts};
use windows::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HWND, RECT,
};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTONULL, MONITORINFO, MonitorFromWindow,
};
use windows::Win32::Security::{GetTokenInformation, TOKEN_QUERY, TokenSessionId};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JOBOBJECT_BASIC_LIMIT_INFORMATION,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectBasicAccountingInformation,
    JobObjectExtendedLimitInformation, QueryInformationJobObject, SetInformationJobObject,
    TerminateJobObject,
};
use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::System::Threading::{
    CreateMutexW, GetCurrentProcess, OpenMutexW, OpenProcess, OpenProcessToken, OpenThread,
    PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
    ResumeThread, SYNCHRONIZATION_SYNCHRONIZE, THREAD_SUSPEND_RESUME,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
use windows::Win32::UI::WindowsAndMessaging::{
    GWL_STYLE, GetClassNameW, GetForegroundWindow, GetWindowLongPtrW, GetWindowRect,
    GetWindowThreadProcessId, IsZoomed, WS_CAPTION,
};
use windows::core::{HSTRING, Owned, PCWSTR, PWSTR};

/// The Windows session of this process. Session 0 is the services session, where SSH logins
/// run: it has no desktop and never sees the owner's input.
pub fn session_id() -> io::Result<u32> {
    let token = process_token()?;
    let mut session = 0u32;
    let mut written = 0u32;
    // SAFETY: TokenSessionId writes one u32; the buffer is a u32 and its size is passed.
    unsafe {
        GetTokenInformation(
            *token,
            TokenSessionId,
            Some((&raw mut session).cast()),
            size_of::<u32>() as u32,
            &mut written,
        )
    }?;
    Ok(session)
}

fn process_token() -> io::Result<Owned<HANDLE>> {
    let mut token = HANDLE::default();
    // SAFETY: GetCurrentProcess returns a pseudo-handle that needs no closing, and `token` is
    // a valid place for the new handle.
    unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }?;
    // SAFETY: OpenProcessToken succeeded, so `token` is a handle that only we own.
    Ok(unsafe { Owned::new(token) })
}

/// `GetTickCount` value of the last keyboard or mouse input in this session, if Windows says.
pub fn last_input_tick() -> Option<u32> {
    let mut info = LASTINPUTINFO {
        cbSize: size_of::<LASTINPUTINFO>() as u32,
        dwTime: 0,
    };
    // SAFETY: `info` is a LASTINPUTINFO with cbSize set, as GetLastInputInfo requires.
    let ok = unsafe { GetLastInputInfo(&mut info) }.as_bool();
    ok.then_some(info.dwTime)
}

/// Milliseconds since boot, wrapping every 49.7 days: the clock `GetLastInputInfo` uses.
pub fn tick_count() -> u32 {
    // SAFETY: GetTickCount has no preconditions.
    unsafe { GetTickCount() }
}

/// A named mutex held for this process's lifetime, so another instance can tell it runs.
pub struct NamedMutex {
    _handle: Owned<HANDLE>,
}

impl NamedMutex {
    /// Creates the mutex `name`; `Ok(None)` if it already exists (another instance holds it).
    pub fn create(name: &str) -> io::Result<Option<Self>> {
        let name = HSTRING::from(name);
        // SAFETY: `name` outlives the call; no security attributes means the default DACL.
        let handle = unsafe { CreateMutexW(None, false, &name) }?;
        // SAFETY: read right after CreateMutexW succeeded, with no other call in between.
        let existed = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        // SAFETY: CreateMutexW returned a handle that only we own.
        let handle = unsafe { Owned::new(handle) };
        Ok((!existed).then_some(Self { _handle: handle }))
    }
}

/// True if a mutex called `name` exists (including one we may not open).
pub fn mutex_exists(name: &str) -> bool {
    let name = HSTRING::from(name);
    // SAFETY: `name` outlives the call; SYNCHRONIZE is the least access OpenMutexW accepts.
    let opened = unsafe { OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, false, &name) };
    match opened {
        Ok(handle) => {
            // SAFETY: OpenMutexW returned a handle that only we own; dropping it closes it.
            drop(unsafe { Owned::new(handle) });
            true
        }
        Err(error) => error.code() == ERROR_ACCESS_DENIED.to_hresult(),
    }
}

/// A job object that stops every process in it when its last handle closes, so nothing a job
/// starts can outlive `kairo-devrun`.
pub struct Job {
    handle: Owned<HANDLE>,
}

impl Job {
    /// An anonymous job with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`.
    pub fn kill_on_close() -> io::Result<Self> {
        // SAFETY: an anonymous job object with default security.
        let handle = unsafe { CreateJobObjectW(None, PCWSTR::null()) }?;
        // SAFETY: CreateJobObjectW returned a handle that only we own.
        let handle = unsafe { Owned::new(handle) };
        let job = Self { handle };
        let limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION {
            BasicLimitInformation: JOBOBJECT_BASIC_LIMIT_INFORMATION {
                LimitFlags: JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                ..Default::default()
            },
            ..Default::default()
        };
        // SAFETY: `limits` is the structure JobObjectExtendedLimitInformation expects, and its
        // size is passed.
        unsafe {
            SetInformationJobObject(
                *job.handle,
                JobObjectExtendedLimitInformation,
                (&raw const limits).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        }?;
        Ok(job)
    }

    /// Adds `child` (still suspended) to the job; the processes it starts join it too.
    pub fn assign(&self, child: &Child) -> io::Result<()> {
        let process = HANDLE(child.as_raw_handle());
        // SAFETY: both handles stay open for the call; `child` keeps ownership of its handle.
        unsafe { AssignProcessToJobObject(*self.handle, process) }?;
        Ok(())
    }

    /// Stops every process in the job.
    pub fn terminate(&self, exit_code: u32) -> io::Result<()> {
        // SAFETY: the job handle is open, and the job only holds processes we started.
        unsafe { TerminateJobObject(*self.handle, exit_code) }?;
        Ok(())
    }

    /// How many processes of the job are still running.
    pub fn active_processes(&self) -> io::Result<u32> {
        let mut accounting = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
        // SAFETY: the buffer is the structure JobObjectBasicAccountingInformation fills, and
        // its size is passed.
        unsafe {
            QueryInformationJobObject(
                Some(*self.handle),
                JobObjectBasicAccountingInformation,
                (&raw mut accounting).cast(),
                size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                None,
            )
        }?;
        Ok(accounting.ActiveProcesses)
    }
}

/// Lets a process started with `CREATE_SUSPENDED` run. It has exactly one thread, still
/// suspended, which this finds with a thread snapshot (std does not hand out thread handles).
pub fn resume_process(pid: u32) -> io::Result<()> {
    // SAFETY: a snapshot of all threads; the handle is owned below.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) }?;
    // SAFETY: CreateToolhelp32Snapshot returned a handle that only we own.
    let snapshot = unsafe { Owned::new(snapshot) };
    let mut entry = THREADENTRY32 {
        dwSize: size_of::<THREADENTRY32>() as u32,
        ..Default::default()
    };
    let mut resumed = 0;
    // SAFETY: `entry` is a THREADENTRY32 with dwSize set, as Thread32First requires.
    let mut more = unsafe { Thread32First(*snapshot, &mut entry) }.is_ok();
    while more {
        if entry.th32OwnerProcessID == pid {
            resume_thread(entry.th32ThreadID)?;
            resumed += 1;
        }
        // SAFETY: the same snapshot and entry as above.
        more = unsafe { Thread32Next(*snapshot, &mut entry) }.is_ok();
    }
    if resumed == 0 {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "the new process has no thread to resume",
        ));
    }
    Ok(())
}

fn resume_thread(thread_id: u32) -> io::Result<()> {
    // SAFETY: opens a thread of our own suspended child with the one right we need.
    let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, false, thread_id) }?;
    // SAFETY: OpenThread returned a handle that only we own.
    let thread = unsafe { Owned::new(thread) };
    // SAFETY: the handle has THREAD_SUSPEND_RESUME access.
    if unsafe { ResumeThread(*thread) } == u32::MAX {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// The foreground window, as the watch agent reports it.
pub struct ForegroundWindow {
    /// The window handle as a number; it changes when another window comes to the front.
    pub window_id: usize,
    /// The process that owns the window.
    pub pid: u32,
    /// Covers its monitor and is neither the desktop nor a maximized ordinary window.
    pub fullscreen: bool,
}

/// The foreground window; `None` when there is none (for example on the lock screen).
pub fn foreground_window() -> Option<ForegroundWindow> {
    // SAFETY: no preconditions; the result may be null.
    let window = unsafe { GetForegroundWindow() };
    if window.is_invalid() {
        return None;
    }
    let mut pid = 0u32;
    // SAFETY: `pid` is a valid place for the process id.
    unsafe { GetWindowThreadProcessId(window, Some(&mut pid)) };
    Some(ForegroundWindow {
        window_id: window.0 as usize,
        pid,
        fullscreen: window_facts(window).is_some_and(|facts| watch::is_fullscreen(&facts)),
    })
}

// TODO(verify) on the PC (M0/M2): the fullscreen test against a borderless game, an exclusive-
// fullscreen game, a fullscreen video, the desktop and a maximized window (auto-hide taskbar).
fn window_facts(window: HWND) -> Option<WindowFacts> {
    let mut rect = RECT::default();
    // SAFETY: `rect` is a valid place for the window rectangle.
    unsafe { GetWindowRect(window, &mut rect) }.ok()?;
    // SAFETY: no preconditions; MONITOR_DEFAULTTONULL gives null for an off-screen window.
    let monitor = unsafe { MonitorFromWindow(window, MONITOR_DEFAULTTONULL) };
    if monitor.is_invalid() {
        return None;
    }
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: `info` is a MONITORINFO with cbSize set.
    if !unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
        return None;
    }
    // SAFETY: reading a window's style has no side effects.
    let style = unsafe { GetWindowLongPtrW(window, GWL_STYLE) };
    // SAFETY: no side effects.
    let maximized = unsafe { IsZoomed(window) }.as_bool();
    let has_caption = (style as u32 & WS_CAPTION.0) == WS_CAPTION.0;
    Some(WindowFacts {
        window: screen_rect(rect),
        monitor: screen_rect(info.rcMonitor),
        maximized_with_caption: maximized && has_caption,
        shell: watch::is_shell_window_class(&class_name(window)),
    })
}

fn class_name(window: HWND) -> String {
    let mut buffer = [0u16; 64];
    // SAFETY: the buffer is valid for its whole length, which the wrapper passes.
    let length = unsafe { GetClassNameW(window, &mut buffer) };
    let length = usize::try_from(length).unwrap_or(0).min(buffer.len());
    String::from_utf16_lossy(&buffer[..length])
}

fn screen_rect(rect: RECT) -> ScreenRect {
    ScreenRect {
        left: rect.left,
        top: rect.top,
        right: rect.right,
        bottom: rect.bottom,
    }
}

/// File name of a process's program, e.g. `game.exe`; `None` if Windows refuses even
/// query-limited access (protected or anti-cheat processes).
pub fn process_exe_name(pid: u32) -> Option<String> {
    if pid == 0 {
        return None;
    }
    // SAFETY: query-only access, the most Kairo may ask of another program (house rule 3).
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
    // SAFETY: OpenProcess returned a handle that only we own.
    let process = unsafe { Owned::new(process) };
    let mut buffer = [0u16; 1024];
    let mut length = buffer.len() as u32;
    // SAFETY: `buffer` holds `length` UTF-16 units and Windows writes at most that many.
    unsafe {
        QueryFullProcessImageNameW(
            *process,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut length,
        )
    }
    .ok()?;
    let path = String::from_utf16_lossy(buffer.get(..length as usize)?);
    watch::exe_file_name(&path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_named_mutex_is_exclusive_and_released_on_drop() {
        let name = format!(r"Local\Kairo.Devrun.Test.{}", std::process::id());
        assert!(!mutex_exists(&name));
        let first = NamedMutex::create(&name).unwrap().unwrap();
        assert!(NamedMutex::create(&name).unwrap().is_none());
        assert!(mutex_exists(&name));
        drop(first);
        assert!(!mutex_exists(&name));
    }

    #[test]
    fn session_and_clock_queries_work() {
        session_id().unwrap();
        let _ = last_input_tick();
        let _ = tick_count();
        // A CI runner may or may not have a desktop; the query must simply not fail badly.
        let _ = foreground_window();
    }

    #[test]
    fn our_own_program_name_is_readable() {
        let name = process_exe_name(std::process::id()).unwrap();
        assert!(name.to_ascii_lowercase().ends_with(".exe"), "{name}");
        assert!(!name.contains('\\'));
        assert_eq!(process_exe_name(0), None);
    }
}
