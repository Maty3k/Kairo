//! Exclusive-fullscreen detection and the capture freeze watchdog.
use std::collections::VecDeque;

use windows::Win32::UI::Shell::{
    QUERY_USER_NOTIFICATION_STATE, QUNS_BUSY, QUNS_RUNNING_D3D_FULL_SCREEN, SHQueryUserNotificationState,
};

/// Poll every ~1 s and on foreground change: the shell sends no notification when a
/// fullscreen app starts or stops (SHQueryUserNotificationState remarks).
pub fn notification_state() -> Option<QUERY_USER_NOTIFICATION_STATE> {
    unsafe { SHQueryUserNotificationState() }.ok()
}

pub fn d3d_exclusive_fullscreen() -> bool {
    notification_state() == Some(QUNS_RUNNING_D3D_FULL_SCREEN)
}

pub fn fullscreen_busy() -> bool {
    matches!(notification_state(), Some(s) if s == QUNS_BUSY || s == QUNS_RUNNING_D3D_FULL_SCREEN)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureHealth {
    Ok,
    /// No new frame for `stale_after` while a game is foreground.
    Stale,
    /// Stale + the shell reports exclusive D3D fullscreen: suggest borderless.
    LikelyExclusiveFullscreen,
    /// DDA re-creating many times per second (OBS #14021 pattern).
    AccessLostLoop,
    /// Frames arrive but are all black (DRM / WDA_MONITOR / protected content).
    Black,
}

pub struct FreezeWatchdog {
    pub stale_after_hns: i64,  // e.g. 3 s = 30_000_000
    last_new_frame_hns: i64,
    lost: VecDeque<i64>,
    pub last_luma_mean: f32,   // fed by a 1 Hz 16x9 downsample/readback of the latest frame
    pub same_hash_since_hns: Option<i64>,
}

impl FreezeWatchdog {
    pub fn new(now_hns: i64) -> Self {
        Self { stale_after_hns: 30_000_000, last_new_frame_hns: now_hns, lost: VecDeque::new(), last_luma_mean: 1.0, same_hash_since_hns: None }
    }

    pub fn on_new_frame(&mut self, ts_hns: i64) {
        self.last_new_frame_hns = self.last_new_frame_hns.max(ts_hns);
    }

    pub fn on_access_lost(&mut self, now_hns: i64) {
        self.lost.push_back(now_hns);
        while let Some(&t) = self.lost.front() {
            if now_hns - t > 30_000_000 { self.lost.pop_front(); } else { break; }
        }
    }

    pub fn evaluate(&self, now_hns: i64, game_foreground: bool) -> CaptureHealth {
        if self.lost.len() >= 15 {
            return CaptureHealth::AccessLostLoop; // >= 5 per second over the last 3 s
        }
        if self.last_luma_mean < 0.002 {
            return CaptureHealth::Black;
        }
        let stale = now_hns - self.last_new_frame_hns > self.stale_after_hns
            || self.same_hash_since_hns.is_some_and(|t| now_hns - t > self.stale_after_hns);
        if game_foreground && stale {
            if d3d_exclusive_fullscreen() { CaptureHealth::LikelyExclusiveFullscreen } else { CaptureHealth::Stale }
        } else {
            CaptureHealth::Ok
        }
    }
}
