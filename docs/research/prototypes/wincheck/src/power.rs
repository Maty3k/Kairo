use windows::Win32::System::Threading::*;
/// Media process: opt out of EcoQoS and timer-resolution throttling while buffering.
pub fn opt_out_of_power_throttling() -> windows::core::Result<()> {
    let state = PROCESS_POWER_THROTTLING_STATE {
        Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        ControlMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED | PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION,
        StateMask: 0, // 0 = mechanism off
    };
    unsafe {
        SetProcessInformation(GetCurrentProcess(), ProcessPowerThrottling,
            &state as *const _ as *const core::ffi::c_void, size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32)
    }
}
