use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};

/// Raw QPC ticks (same timeline as DDA `DXGI_OUTDUPL_FRAME_INFO::LastPresentTime`).
pub fn qpc_now() -> i64 {
    let mut t = 0i64;
    unsafe { QueryPerformanceCounter(&mut t) }.expect("QPC never fails on XP+");
    t
}

pub fn qpc_freq() -> i64 {
    let mut f = 0i64;
    unsafe { QueryPerformanceFrequency(&mut f) }.expect("QPF never fails on XP+");
    f
}

/// Raw QPC ticks -> 100 ns units (the unit of WGC `SystemRelativeTime` and WASAPI `pu64QPCPosition`).
pub fn qpc_to_hns(ticks: i64, freq: i64) -> i64 {
    ((ticks as i128 * 10_000_000) / freq as i128) as i64
}

pub fn now_hns() -> i64 {
    qpc_to_hns(qpc_now(), qpc_freq())
}
