//! Media: out-of-process dumps via control's minidumper server. Every process: panic hook dumps first,
//! because abort / panic=abort / panics escaping extern "system" are __fastfail (no in-process handler runs).
use std::sync::{Arc, OnceLock};
use crash_handler::{make_crash_event, CrashContext, CrashEventResult, CrashHandler};

static HANDLER: OnceLock<CrashHandler> = OnceLock::new();
pub const KAIRO_PANIC: i32 = 0xE0004B50u32 as i32; // Kairo-specific exception code for panic dumps
pub const EXIT_PANIC: i32 = 70;

pub fn install_media(socket: &std::path::Path) -> anyhow::Result<()> {
    let client = Arc::new(minidumper::Client::with_name(minidumper::SocketName::Path(socket))?);
    let c = client.clone();
    let handler = CrashHandler::attach(unsafe {
        make_crash_event(move |cc: &CrashContext| CrashEventResult::Handled(c.request_dump(cc).is_ok()))
    })?;
    let _ = HANDLER.set(handler);
    std::panic::set_hook(Box::new(|info| {
        tracing::error!(target: "panic", "{info}\n{}", std::backtrace::Backtrace::force_capture());
        if let Some(h) = HANDLER.get() { let _ = h.simulate_exception(Some(KAIRO_PANIC)); } // dump BEFORE any abort
        std::process::exit(EXIT_PANIC); // control sees a classified exit code, not 0xC0000409
    }));
    Ok(())
}

/// "Ask each time": envelopes wait in an outbox until the user clicks Send.
pub struct OutboxTransport { pub dir: std::path::PathBuf }
impl sentry::Transport for OutboxTransport {
    fn send_envelope(&self, envelope: sentry::Envelope) {
        let path = self.dir.join(format!("{}.envelope", ulid::Ulid::generate()));
        if let Ok(f) = std::fs::File::create(&path) { let _ = envelope.to_writer(f); }
    }
}
pub fn sentry_options(dsn: &str, release: &'static str, outbox: Option<std::path::PathBuf>) -> sentry::ClientOptions {
    let mut o = sentry::ClientOptions::default(); // #[non_exhaustive]: no struct literal, set fields
    o.dsn = dsn.parse().ok();                     // EU org: https://<key>@o<org>.ingest.de.sentry.io/<project>
    o.release = Some(release.into());
    o.server_name = Some("redacted".into());      // else sentry-contexts fills in the hostname (PII)
    o.send_default_pii = false;
    o.before_send = Some(Arc::new(|mut e: sentry::protocol::Event<'static>| { e.server_name = None; e.user = None; e.request = None; Some(e) }));
    if let Some(dir) = outbox { o.transport = Some(Arc::new(Arc::new(OutboxTransport { dir }))); }
    o
}
