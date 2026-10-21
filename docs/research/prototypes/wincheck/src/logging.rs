use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};
/// One file family per process; size rotation 10 x 10 MB; non-blocking + lossy so capture threads never block.
pub fn init(dir: &std::path::Path, process: &str) -> tracing_appender::non_blocking::WorkerGuard {
    let rot = file_rotate::FileRotate::new(dir.join(format!("{process}.log")), file_rotate::suffix::AppendCount::new(10),
        file_rotate::ContentLimit::BytesSurpassed(10 << 20), file_rotate::compression::Compression::None, None);
    let (w, guard) = tracing_appender::non_blocking(rot);
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,kairo=debug,wasapi=warn")))
        .with(fmt::layer().json().with_writer(w))   // wrap `w` in the redacting MakeWriter (research 11 §5.2)
        .init();
    guard // keep alive until WM_ENDSESSION / shutdown
}
