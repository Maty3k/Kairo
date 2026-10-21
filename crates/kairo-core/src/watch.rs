// SPDX-License-Identifier: GPL-3.0-or-later
//! The watch agent's status: `C:\kairo\state\watch.json` (docs/PLAN.md §3.1).
//!
//! `kairo-devrun watch` runs in the owner's session and rewrites the file every second, so
//! Claude can tell over SSH whether the owner is at the PC before starting anything visible.
//! The games list comes later; in M0 a fullscreen foreground window stands in for "playing".

use serde::{Deserialize, Serialize};

/// Input within this many milliseconds makes the PC busy: 10 minutes.
pub const BUSY_INPUT_WINDOW_MS: u64 = 600_000;
/// How often the watch agent rewrites `watch.json`.
pub const WATCH_INTERVAL_MS: u64 = 1_000;
/// A `watch.json` older than this means the watch agent is not running: treat the PC as busy.
pub const WATCH_STALE_AFTER_MS: u64 = 5_000;

/// One sample of the owner's activity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WatchStatus {
    /// When the sample was taken, RFC 3339 UTC.
    pub ts_utc: String,
    /// Milliseconds since the last keyboard or mouse input in the session; null if unknown.
    pub last_input_ms_ago: Option<u64>,
    /// The foreground window covers its whole monitor (a game, a video, a presentation).
    pub foreground_fullscreen: bool,
    /// File name of the foreground window's program while that window is fullscreen, e.g.
    /// `VALORANT-Win64-Shipping.exe`; null otherwise or if unknown. Never a full path, which
    /// could contain the user name, and never the name of an ordinary (non-fullscreen) window.
    pub foreground_exe: Option<String>,
    /// Input within the last 10 minutes (or unknown), or a fullscreen foreground window.
    pub busy: bool,
}

impl WatchStatus {
    /// A sample; `busy` follows from the other fields.
    pub fn new(
        ts_utc: String,
        last_input_ms_ago: Option<u64>,
        foreground_fullscreen: bool,
        foreground_exe: Option<String>,
    ) -> Self {
        Self {
            ts_utc,
            last_input_ms_ago,
            foreground_fullscreen,
            foreground_exe,
            busy: is_busy(last_input_ms_ago, foreground_fullscreen),
        }
    }
}

/// Busy means input within the last 10 minutes, an unknown input time, or a fullscreen
/// foreground window.
pub fn is_busy(last_input_ms_ago: Option<u64>, foreground_fullscreen: bool) -> bool {
    foreground_fullscreen || last_input_ms_ago.is_none_or(|ms| ms < BUSY_INPUT_WINDOW_MS)
}

/// Milliseconds between two `GetTickCount` readings, correct across its 49.7-day wrap.
pub fn ticks_between(earlier: u32, later: u32) -> u64 {
    u64::from(later.wrapping_sub(earlier))
}

/// A rectangle in screen pixels (right and bottom exclusive), like Win32's `RECT`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenRect {
    /// Left edge.
    pub left: i32,
    /// Top edge.
    pub top: i32,
    /// Right edge.
    pub right: i32,
    /// Bottom edge.
    pub bottom: i32,
}

/// What the fullscreen test needs to know about the foreground window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowFacts {
    /// The window's rectangle.
    pub window: ScreenRect,
    /// The full rectangle of the monitor the window is on (not just its work area).
    pub monitor: ScreenRect,
    /// Maximized with a title bar: an ordinary window, even if the taskbar auto-hides.
    pub maximized_with_caption: bool,
    /// The desktop or a taskbar, which cover the screen without being a fullscreen app.
    pub shell: bool,
}

/// True for a window that covers its whole monitor and is not the desktop or a maximized
/// ordinary window.
pub fn is_fullscreen(facts: &WindowFacts) -> bool {
    !facts.shell && !facts.maximized_with_caption && covers(facts.window, facts.monitor)
}

/// True if `window` covers all of `monitor`.
pub fn covers(window: ScreenRect, monitor: ScreenRect) -> bool {
    window.left <= monitor.left
        && window.top <= monitor.top
        && window.right >= monitor.right
        && window.bottom >= monitor.bottom
}

/// Window classes of the shell's desktop and taskbars.
pub fn is_shell_window_class(class: &str) -> bool {
    matches!(
        class,
        "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd"
    )
}

/// The file name of a program path, e.g. `C:\Games\x.exe` → `x.exe`. The folders are dropped
/// because they can contain the user name (docs/PLAN.md Appendix A: no raw paths).
pub fn exe_file_name(path: &str) -> Option<String> {
    let name = path.rsplit(['\\', '/']).next()?;
    (!name.is_empty()).then(|| name.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    const MONITOR: ScreenRect = ScreenRect {
        left: 0,
        top: 0,
        right: 2560,
        bottom: 1440,
    };

    fn facts(window: ScreenRect) -> WindowFacts {
        WindowFacts {
            window,
            monitor: MONITOR,
            maximized_with_caption: false,
            shell: false,
        }
    }

    #[test]
    fn busy_rules() {
        assert!(is_busy(Some(0), false));
        assert!(is_busy(Some(599_999), false));
        assert!(!is_busy(Some(600_000), false));
        assert!(!is_busy(Some(3_600_000), false));
        assert!(is_busy(Some(3_600_000), true));
        assert!(is_busy(None, false));
    }

    #[test]
    fn tick_differences_survive_the_wrap() {
        assert_eq!(ticks_between(1_000, 1_500), 500);
        assert_eq!(ticks_between(u32::MAX - 9, 10), 20);
    }

    #[test]
    fn fullscreen_rules() {
        assert!(is_fullscreen(&facts(MONITOR)));
        let bigger = ScreenRect {
            left: -8,
            top: -8,
            right: 2568,
            bottom: 1448,
        };
        assert!(is_fullscreen(&facts(bigger)));
        let above_taskbar = ScreenRect {
            bottom: 1392,
            ..MONITOR
        };
        assert!(!is_fullscreen(&facts(above_taskbar)));
        let maximized = WindowFacts {
            maximized_with_caption: true,
            ..facts(bigger)
        };
        assert!(!is_fullscreen(&maximized));
        let desktop = WindowFacts {
            shell: true,
            ..facts(MONITOR)
        };
        assert!(!is_fullscreen(&desktop));
        let second_monitor = WindowFacts {
            window: ScreenRect {
                left: 2560,
                top: 0,
                right: 4480,
                bottom: 1080,
            },
            monitor: ScreenRect {
                left: 2560,
                top: 0,
                right: 4480,
                bottom: 1080,
            },
            maximized_with_caption: false,
            shell: false,
        };
        assert!(is_fullscreen(&second_monitor));
    }

    #[test]
    fn shell_classes() {
        assert!(is_shell_window_class("WorkerW"));
        assert!(is_shell_window_class("Progman"));
        assert!(!is_shell_window_class("UnrealWindow"));
    }

    #[test]
    fn exe_names_never_include_folders() {
        assert_eq!(
            exe_file_name(r"C:\Users\someone\AppData\Local\game.exe"),
            Some("game.exe".to_owned())
        );
        assert_eq!(exe_file_name("x.exe"), Some("x.exe".to_owned()));
        assert_eq!(exe_file_name("C:/a/b.exe"), Some("b.exe".to_owned()));
        assert_eq!(exe_file_name(r"C:\dir\"), None);
        assert_eq!(exe_file_name(""), None);
    }

    #[test]
    fn status_json_has_exactly_the_contract_fields() {
        let status = WatchStatus::new(
            "2026-10-21T20:15:03.123Z".to_owned(),
            Some(1_234),
            false,
            Some("explorer.exe".to_owned()),
        );
        assert!(status.busy);
        let text = serde_json::to_string(&status).unwrap();
        assert_eq!(
            text,
            r#"{"ts_utc":"2026-10-21T20:15:03.123Z","last_input_ms_ago":1234,"foreground_fullscreen":false,"foreground_exe":"explorer.exe","busy":true}"#
        );
        let idle = WatchStatus::new(String::new(), Some(BUSY_INPUT_WINDOW_MS), false, None);
        assert!(!idle.busy);
        assert_eq!(
            serde_json::to_value(&idle).unwrap()["foreground_exe"],
            serde_json::Value::Null
        );
    }
}
