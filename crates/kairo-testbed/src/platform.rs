// SPDX-License-Identifier: GPL-3.0-or-later
//! The testbed's own Win32 calls (docs/research/16-verify-testbed/README.md §4.3): the
//! high-resolution timer that paces timer runs. QPC time, the power and display requests, the
//! window and taking the foreground are `kairo-window`'s (research 17 §3.1). Nothing here
//! synthesises input, hooks anything or opens another process (CLAUDE.md house rules 1 and 4).

use kairo_window::platform::now_hns;
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::System::Threading::{
    CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, CreateWaitableTimerExW, SetWaitableTimer,
    TIMER_ALL_ACCESS, WaitForSingleObject,
};
use windows::core::PCWSTR;

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
