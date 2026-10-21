//! One recursive lock for multi-call sequences on the D3D11 immediate context.
//! Shared with FFmpeg via AVD3D11VADeviceContext.lock / .unlock / .lock_ctx.
use core::{cell::UnsafeCell, ffi::c_void};
use windows::Win32::System::Threading::{
    DeleteCriticalSection, EnterCriticalSection, InitializeCriticalSection, LeaveCriticalSection, CRITICAL_SECTION,
};

pub struct GpuLock(Box<UnsafeCell<CRITICAL_SECTION>>); // boxed: a CRITICAL_SECTION must never move
// SAFETY: CRITICAL_SECTION is a cross-thread primitive; we only touch it through Win32 calls.
unsafe impl Send for GpuLock {}
unsafe impl Sync for GpuLock {}

impl GpuLock {
    pub fn new() -> Self {
        let cs = Box::new(UnsafeCell::new(CRITICAL_SECTION::default()));
        unsafe { InitializeCriticalSection(cs.get()) };
        Self(cs)
    }
    pub fn enter(&self) -> GpuGuard<'_> {
        unsafe { EnterCriticalSection(self.0.get()) };
        GpuGuard(self)
    }
    /// For AVD3D11VADeviceContext.lock_ctx. FFmpeg never frees it: outlive every FFmpeg object.
    pub fn ffmpeg_ctx(&self) -> *mut c_void { self.0.get().cast() }
}
impl Drop for GpuLock {
    fn drop(&mut self) { unsafe { DeleteCriticalSection(self.0.get()) } }
}
pub struct GpuGuard<'a>(&'a GpuLock);
impl Drop for GpuGuard<'_> {
    fn drop(&mut self) { unsafe { LeaveCriticalSection(self.0 .0.get()) } }
}
/// AVD3D11VADeviceContext.lock / .unlock ("must be recursive": a CRITICAL_SECTION is).
pub unsafe extern "C" fn ff_lock(ctx: *mut c_void) { unsafe { EnterCriticalSection(ctx.cast()) } }
pub unsafe extern "C" fn ff_unlock(ctx: *mut c_void) { unsafe { LeaveCriticalSection(ctx.cast()) } }

// Type check: the callback type bindgen emits for `void (*lock)(void *lock_ctx)`.
pub const _CB: Option<unsafe extern "C" fn(*mut c_void)> = Some(ff_lock);
