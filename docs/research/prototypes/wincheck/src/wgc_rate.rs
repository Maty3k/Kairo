use std::time::Duration;
use windows::{core::Result, Foundation::TimeSpan, Graphics::Capture::GraphicsCaptureSession};

/// Start value for MinUpdateInterval (26100+): deliver at >= 1.2x the clip rate on a whole number k of
/// vblanks, asking for (k - 0.5) vblanks so jitter cannot push a delivery one vblank later. [E] SP2 decides.
pub fn min_update_interval(refresh_hz: f64, clip_fps: f64) -> Duration {
    let k = (refresh_hz / (1.2 * clip_fps)).floor().max(1.0);
    Duration::from_secs_f64((k - 0.5) / refresh_hz)
}

/// Returns (OS default read back, value set). Gate on ApiInformation::IsPropertyPresent first.
pub fn apply(session: &GraphicsCaptureSession, refresh_hz: f64, clip_fps: f64) -> Result<(Duration, Duration)> {
    let default: Duration = session.MinUpdateInterval()?.into();
    let want = min_update_interval(refresh_hz, clip_fps);
    session.SetMinUpdateInterval(TimeSpan::from(want))?;
    Ok((default, want))
}
