use std::sync::OnceLock;
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
static QPF: OnceLock<i64> = OnceLock::new();
/// Raw QPC ticks -> 100 ns (same unit/epoch as WGC SystemRelativeTime and WASAPI QPC positions).
pub fn ticks_to_hns(ticks: i64) -> i64 {
    let f = *QPF.get_or_init(|| { let mut f = 0; unsafe { let _ = QueryPerformanceFrequency(&mut f); } f });
    (ticks as i128 * 10_000_000 / f as i128) as i64
}
pub fn now_hns() -> i64 { let mut c = 0; unsafe { let _ = QueryPerformanceCounter(&mut c); } ticks_to_hns(c) }
