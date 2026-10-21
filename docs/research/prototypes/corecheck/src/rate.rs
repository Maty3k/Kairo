/// WGC MinUpdateInterval start value in seconds (see wincheck::wgc_rate).
pub fn min_update_interval_s(refresh_hz: f64, clip_fps: f64) -> f64 {
    let k = (refresh_hz / (1.2 * clip_fps)).floor().max(1.0);
    (k - 0.5) / refresh_hz
}
#[cfg(test)]
mod t {
    use super::*;
    fn deliveries(refresh: f64, fps: f64) -> f64 { // deliveries/s if each lands on the first vblank >= interval
        let iv = min_update_interval_s(refresh, fps); refresh / (iv * refresh).ceil()
    }
    #[test] fn table() {
        for (hz, fps, want) in [(360.0, 60.0, 72.0), (180.0, 60.0, 90.0), (165.0, 60.0, 82.5), (144.0, 60.0, 72.0),
                                (120.0, 60.0, 120.0), (60.0, 60.0, 60.0), (360.0, 120.0, 180.0), (240.0, 60.0, 80.0)] {
            assert!((deliveries(hz, fps) - want).abs() < 1e-9, "{hz} Hz {fps} fps -> {}", deliveries(hz, fps));
            assert!(deliveries(hz, fps) >= fps);
        }
    }
}
