//! \GPU Engine(*)\Utilization Percentage sampler (video engines only). Match "video" case-insensitively:
//! the AMD engine-type spelling (VideoCodec vs VideoEncode, spaces?) is [U] - log every type seen.
use windows::{core::*, Win32::System::Performance::*};
pub struct EngineSampler { q: PDH_HQUERY, c: PDH_HCOUNTER, buf: Vec<PDH_FMT_COUNTERVALUE_ITEM_W> }
impl EngineSampler {
    pub fn new() -> Option<Self> { unsafe {
        let mut q = PDH_HQUERY::default();
        if PdhOpenQueryW(PCWSTR::null(), 0, &mut q) != 0 { return None; }
        let mut c = PDH_HCOUNTER::default();
        if PdhAddEnglishCounterW(q, w!("\\GPU Engine(*)\\Utilization Percentage"), 0, &mut c) != 0 { PdhCloseQuery(q); return None; }
        PdhCollectQueryData(q); // rate counter: first sample primes it
        Some(Self { q, c, buf: Vec::new() })
    } }
    /// (pid, engtype, %) for video engines; call ~1 Hz.
    pub fn video(&mut self) -> Vec<(u32, String, f64)> { unsafe {
        if PdhCollectQueryData(self.q) != 0 { return vec![]; }
        let (mut bytes, mut n) = (0u32, 0u32);
        if PdhGetFormattedCounterArrayW(self.c, PDH_FMT_DOUBLE, &mut bytes, &mut n, None) != PDH_MORE_DATA { return vec![]; }
        self.buf.resize((bytes as usize).div_ceil(size_of::<PDH_FMT_COUNTERVALUE_ITEM_W>()), Default::default());
        if PdhGetFormattedCounterArrayW(self.c, PDH_FMT_DOUBLE, &mut bytes, &mut n, Some(self.buf.as_mut_ptr())) != 0 { return vec![]; }
        self.buf[..n as usize].iter()
            .filter(|i| i.FmtValue.CStatus == PDH_CSTATUS_VALID_DATA || i.FmtValue.CStatus == PDH_CSTATUS_NEW_DATA)
            .filter_map(|i| {
                let name = i.szName.to_string().ok()?;                     // pid_123_luid_..._engtype_VideoCodec
                let pid = name.strip_prefix("pid_")?.split('_').next()?.parse().ok()?;
                let ty = name.rsplit_once("engtype_")?.1.to_string();
                ty.to_ascii_lowercase().replace(' ', "").starts_with("video").then(|| (pid, ty, i.FmtValue.Anonymous.doubleValue))
            }).collect()
    } }
}
impl Drop for EngineSampler { fn drop(&mut self) { unsafe { PdhCloseQuery(self.q); } } }
