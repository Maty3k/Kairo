# Kairo research 11: the Rust engine shell (threads, tray, hotkeys, IPC, game detection, logging, crashes, dev loop)

*Researched 2026-10-21 for the implementation cookbook. Scope: everything in `kairo-engine.exe` that is not capture, encode or audio. Builds on the brief (00), report 03 (triggers, detection) and report 05 (stack, workflow), and on the parallel plan draft (`plan/draft-B-daily-driver-asap.md`).*

**Method.** Each crate version below is from the crates.io API on 2026-10-21. Every crate whose API I quote was downloaded from static.crates.io, its SHA-256 checked against the crates.io index, and the source read. Windows API facts come from the Microsoft Learn source repos (`MicrosoftDocs/sdk-api`, `MicrosoftDocs/win32`, `MicrosoftDocs/windowsserverdocs`, `MicrosoftDocs/windows-powershell-docs`). Function signatures were checked in the `windows` 0.62.2 crate source.

**Tags**
- **[V]**: verified today against a primary source (crate source, Microsoft docs, vendor README, spec).
- **[Inference]**: reasoned from verified facts, not tested.
- **[Unverified]**: could not be confirmed. Test it on the PC before relying on it.
- **[Estimate]**: a number to measure.

---

## 0. Decisions this report proposes, and corrections

| # | Topic | Recommendation | Changes to the brief / plan draft |
|---|---|---|---|
| E1 | Process topology | Keep the plan's split. A small **control** process owns tray, hotkeys, IPC, detection, DB and supervision. A **media** worker (`--media`) owns D3D, FFmpeg, WASAPI and the ring. Control hosts the minidump server for media. | Confirms plan §2.1. Adds the crash server, a stdio link and the job flags (§2, §5) |
| E2 | Event loop | A plain `windows` 0.62.2 Win32 loop (`GetMessageW`) on the control process's main thread, with one hidden **top-level** window. No winit, no tao. | — |
| E3 | Tray | `tray-icon` 0.26.1 + `muda` 0.21.2 on that thread, with `with_menu_on_left_click(false)` and a fixed `with_guid`. | — |
| E4 | Hotkeys | Call `RegisterHotKey` directly (`MOD_NOREPEAT`) on the shell window. Don't depend on `global-hotkey`. | Plan lists `global-hotkey` 0.8; current is 0.9.0, and it is unnecessary (§3.2) |
| E5 | IPC transport | tokio 1.54.0 named pipes (`ServerOptions`/`ClientOptions`) with an explicit DACL, plus `UnixListener` for the Mac mock. | Plan lists `interprocess` 2.4.6. Acceptable for the server, but its client opens pipes without `SECURITY_SQOS_PRESENT` (§4.1) |
| E6 | IPC protocol | Hand-rolled JSON-RPC 2.0 over serde_json, framed with `LengthDelimitedCodec` (u32 little-endian, 1 MiB max). Types in `kairo-rpc` → **ts-rs 12.0.1** + **schemars 1.2.3**. | Confirms plan; adds exact codec, error codes, method list (§4.6) |
| E7 | Native crashes | **`sentry-rust-minidump` is deprecated.** Its README says "This crate will get no more updates", superseded by the official `sentry-minidump` (the `minidump` feature of `sentry` 0.49.3). Both upload at crash time, so neither alone gives "Ask each time". Use `minidumper` 0.11.0 + `crash-handler` 0.8.1 with control as the dump server, and a **disk outbox `Transport`** that holds Sentry envelopes until consent. | Replaces plan §2.9 and brief §8.4 crate choice (§5.3–5.5) |
| E8 | Data folder | **Don't use `%LOCALAPPDATA%\Kairo` for data.** Tauri 2.12.4's per-user NSIS installs binaries to `$LOCALAPPDATA\${PRODUCTNAME}`, i.e. that same folder. Its uninstaller's "delete app data" option removes `$LOCALAPPDATA\${BUNDLEID}`. Put data in `%LOCALAPPDATA%\<bundle identifier>\`. | Changes the brief §7.1 and plan §2.6 paths (§6.5) |
| E9 | Log rotation | `tracing-appender` 0.2.5 rotates by **time only**. For the plan's "10 × 10 MB" use `file-rotate` 0.8.0 behind `tracing_appender::non_blocking`. | Plan §2.9 needs this crate (§5.1) |
| E10 | Panics | `std::process::abort` on Windows is `__fastfail`, which skips every in-process handler, so it leaves no dump. The panic hook must request a dump first, and no panic may cross an `extern "system"` callback. | New (§5.3) |
| E11 | Background throttling | Opt the media process out of EcoQoS and timer-resolution throttling with `SetProcessInformation(ProcessPowerThrottling)`. An invisible process isn't guaranteed timer resolution on Windows 11. | New (§2.6) |
| E12 | Autostart | Write `HKCU\…\Run\Kairo` yourself, with a **quoted** path. `auto-launch` 0.6.0 writes the path unquoted, tries HKLM first in `Dynamic` mode, and re-enables the user's Task Manager "disabled" flag on `enable()`. | Plan says `auto-launch` 0.6 (§6.1) |
| E13 | Dev loop | Sync with a **snapshot commit pushed to a non-bare clone** using `receive.denyCurrentBranch=updateInstead`. Launch with `Register-ScheduledTask` (Interactive, Limited, **Priority 4**). `schtasks /create` leaves tasks at priority 7 = `BELOW_NORMAL_PRIORITY_CLASS`. Drive the engine from SSH through `kairo-cli` over the same pipe. | Refines plan §3.1 (§8) |
| E14 | PresentMon over SSH | It works from the SSH session itself: ETW is system-wide, and an admin's OpenSSH logon gets the full (elevated) token. Otherwise add the user to Performance Log Users and open a new SSH session. | Confirms the plan's "verify in M0" (§8.8) |

---

## 1. Verified versions (crates.io, 2026-10-21)

Rust stable is **1.99.1** (channel manifest dated 2026-10-15), so every MSRV below is satisfied [V].

| Crate | Version (date) | MSRV | Use / note |
|---|---|---|---|
| `windows` | **0.62.2** (2025-10-06) | 1.82 | Pin `=0.62.2`. The 0.100 family (`windows-core` / `-registry` / `-result` / `-strings` 0.100.0, 2026-09-03) needs Rust 1.95 and changes signatures. `wasapi` 0.26.0 and `windows-capture` 2.0.1 both depend on `windows` 0.62.2 [V] |
| `windows-registry` | **0.6.1** (2025-10-06) | 1.82 | The 0.62-family version. `cargo add` would pick 0.100.0, so write `"0.6"` explicitly |
| `windows-sys` | 0.61.2 | 1.71 | Pulled in by tokio, interprocess, tray-icon, muda. `global-hotkey` 0.9.0 pulls **0.59** |
| `tray-icon` | **0.26.1** (2026-10-07) | 1.90 | windows-sys `>=0.60,<=0.61` |
| `muda` | **0.21.2** (2026-10-08) | 1.90 | Menus for the tray |
| `global-hotkey` | 0.9.0 (2026-10-16) | 1.77 | Not recommended (§3.2) |
| `winit` / `tao` | 0.30.13 / 0.37.1 | — | Not recommended (§2.3) |
| `tokio` | **1.54.0** (2026-10-19) | 1.85 | `net`, `rt-multi-thread`, `process`, `io-util`, `sync`, `time`, `macros` |
| `tokio-util` | **0.7.20** (2026-10-09) | 1.85 | `codec` (`LengthDelimitedCodec`) |
| `interprocess` | 2.4.6 (2026-10-16) | 1.75 | Alternative transport (§4.1) |
| `serde` / `serde_json` | 1.0.229 / 1.0.154 | — | |
| `ts-rs` | **12.0.1** (2026-01-31) | 1.78 | `TS_RS_LARGE_INT` defaults to `bigint`; set it to `number` (§4.5) |
| `schemars` | 1.2.3 (2026-10-12) | 1.74 | JSON Schema for contract tests |
| `specta` / `tauri-specta` | 2.0.0-**rc.25** (2026-05-07/08) | — | In RC since 2023-07-17 (specta) and 2023-10-04 (tauri-specta). Avoid |
| `jsonrpsee` | 0.26.1 (2026-09-30) | 1.87 | Not recommended (§4.4) |
| `tracing` / `tracing-subscriber` / `tracing-appender` | 0.1.44 / 0.3.23 / **0.2.5** | — | `tracing-appender`: `Rotation::{MINUTELY,HOURLY,DAILY,WEEKLY,NEVER}` only |
| `file-rotate` | 0.8.0 (2025-02-27) | — | Size-based rotation |
| `sentry` (+`-core`, `-types`, `-contexts`, `-panic`, `-tracing`) | **0.49.3** (2026-09-21) | 1.88 | Feature `minidump` now exists, backed by `sentry-minidump` 0.49.3 (first published 2026-09-21) |
| `sentry-rust-minidump` | 0.18.1 (2026-09-28) | — | **Deprecated** in its README |
| `crash-handler` | **0.8.1** (2026-09-25) | — | Embark |
| `minidumper` | **0.11.0** (2026-07-20) | — | Embark. AF_UNIX socket IPC on Windows |
| `minidump-writer` | 0.13.0 (2026-07-20) | 1.88 | Used by minidumper. Direct use for hang dumps |
| `minidumper-child` | 0.6.1 (2026-10-08) | — | Used by `sentry-minidump` |
| `minidump-stackwalk` / `dump_syms` | 0.27.0 / 2.3.9 | — | Offline crash analysis on the Mac |
| `rusqlite` | **0.40.2** (2026-08-08) | — | `bundled` = **SQLite 3.53.2** (`libsqlite3-sys` 0.38.2): FTS5, JSON1, `SQLITE_DEFAULT_FOREIGN_KEYS=1`, `THREADSAFE=1` [V] |
| `rusqlite_migration` | 2.6.0 (2026-05-28) | 1.95 | Uses `PRAGMA user_version` |
| `steamlocate` | **2.1.1** (2026-08-13) | 1.83 | Uses `winreg` 0.56 + `keyvalues-serde` |
| `keyvalues-parser` | 0.2.4 (2026-05-17) | 1.81 | Text VDF v1 only. Entry point `keyvalues_parser::parse(&str)` |
| `roxmltree` | 0.21.1 | 1.60 | `MicrosoftGame.config` |
| `crossbeam-channel` | 0.5.17 | 1.60 | Sync-thread channels with `select!` |
| `widestring` | 1.2.1 | — | Only if you use interprocess's SDDL API |
| `auto-launch` | 0.6.0 (2026-01-10) | — | Not recommended (§6.1) |
| `embed-resource` | 3.0.12 (2026-10-06) | 1.76 | Manifest + icon. Uses `llvm-rc` on non-Windows hosts |
| `tauri` / `tauri-plugin-single-instance` | 2.12.4 / 2.5.4 | 1.90 | UI side |
| `cargo-xwin` | 0.23.1 (2026-08-13) | 1.89 | Mac compile checks |
| `dirs` | 7.0.0 (2026-09-05) | — | `data_local_dir()`, `video_dir()` |
| `ulid` / `semver` / `anyhow` / `thiserror` | 3.0.0 / 1.0.28 / 1.0.104 / 2.0.21 | — | |

Tools: PresentMon **2.6.0** (2026-09-21), PowerShell **7.6.7** (2026-10-13), Git for Windows **2.56.0.windows.2** (2026-10-05), sentry-cli 3.8.0 (2026-09-16), Mutagen 0.18.1 (2025-02-24, no release since) [V].

---

## 2. Processes and threads

### 2.1 Topology

```
HKCU Run "Kairo" ─► kairo-engine.exe                      CONTROL  (medium IL, interactive session)
                     ├ T0 shell (main thread, COM STA): hidden top-level window · tray · RegisterHotKey
                     │    · SetWinEventHook(FOREGROUND) · WTS/power/endsession · sound cue · pop-up
                     ├ tokio runtime (2 workers): core actor · UI pipe server · media link · jobs
                     ├ detect (MTA)  ├ mouse (opt-in)  ├ db (rusqlite owner)  ├ crashsrv (minidumper::Server)
                     └ spawns, in a Job Object (KILL_ON_JOB_CLOSE | DIE_ON_UNHANDLED_EXCEPTION | BREAKAWAY_OK):
                         kairo-engine.exe --media --crash-socket=<path>          MEDIA
                           capture (WinRT MTA + DispatcherQueue loop) · video pacer/convert/encode (MTA)
                           audio capture ×N (MTA, MMCSS "Audio") · audio encode · ring · save/mux (MTA)
                           link: JSON-RPC over its stdin/stdout ⇄ control
Kairo.exe (Tauri) ── \\.\pipe\kairo-<user-SID> ──► control      (starts on demand, exits fully)
kairo-cli.exe (dev, over SSH) ── same pipe ─────► control      (same user, other session: allowed)
```

Why this split works [Inference from verified facts]:
- GPU-driver, FFmpeg and COM crashes stay in media. Tray, hotkeys and the UI connection survive, and control restarts media with a cue.
- QPC is one system-wide counter. Control can stamp the hotkey time and media can cut the ring at that timestamp.
- The audio stem that excludes Kairo's own sounds should exclude **control's** process tree. Media is control's child, so one include-tree exclusion covers both. That is an input for the audio report.

### 2.2 Thread map

| Process | Thread | COM | Priority / QoS | Owns | Talks via |
|---|---|---|---|---|---|
| control | **T0 shell** (main) | STA | normal | Hidden top-level window, tray-icon + muda, `RegisterHotKey`, WinEvent hook, WTS + power notifications, pop-up window, `PlaySoundW(SND_ASYNC\|SND_MEMORY)` cue | In: `PostMessageW(hwnd, WM_APP_WAKE)` + crossbeam queue. Out: tokio `UnboundedSender<CoreEvent>` (never blocks) |
| control | tokio workers ×2 | none (no COM here) | normal | Core actor (single owner of state), UI pipe sessions, media link, ffmpeg.exe export jobs | mpsc channels |
| control | detect | MTA | below normal | Foreground → PID → path → elevation; Toolhelp poll every 3 s; launcher scans; Xbox `PackageManager`; icon → PNG | crossbeam in, core channel out |
| control | db | none | normal | The single `rusqlite::Connection` | crossbeam command queue + oneshot replies |
| control | mouse (only if bound) | none | normal | `GetAsyncKeyState(VK_XBUTTON1/2)` poll on a high-resolution waitable timer | core channel |
| control | crashsrv | none | normal | `minidumper::Server::run` for media (and future helpers) | file system + core channel |
| media | link | none | normal | stdin/stdout JSON-RPC to control | — |
| media | capture | WinRT MTA | normal + MMCSS "Capture" [Unverified fit] | `windows-capture` `start_free_threaded` (its own `DispatcherQueue` + `GetMessageW` loop) or DDA loop | "latest frame" slot |
| media | video | **MTA** | above normal [Estimate] | QPC pacer, HLSL convert, libavcodec encode | packets → ring |
| media | audio-cap-* | MTA | MMCSS "Audio" (`AvSetMmThreadCharacteristicsW(w!("Audio"), …)`) | WASAPI event-driven capture | PCM → mixer |
| media | audio-enc | MTA | normal | Resample, mix, AAC | packets → ring |
| media | save | MTA | below normal | Mux snapshot → MP4 → fsync → rename | link notification |
| supervisor-free | — | — | — | No third process. Control is the supervisor | — |

COM rules [V]:
- FFmpeg 9.0 `h264_mf`/`aac_mf` call `CoInitializeEx(NULL, COINIT_MULTITHREADED)` themselves and fail with "COM must not be in STA mode" on an STA thread (`libavcodec/mf_utils.c`). **Never open an encoder on T0.**
- `CoInitializeEx` with a different concurrency flag on an already-initialised thread returns `RPC_E_CHANGED_MODE`. Each successful call, including `S_FALSE`, needs a matching `CoUninitialize`. Use an RAII guard per thread.
- `windows-capture` 2.0.1 `start()` calls `RoInitialize(RO_INIT_MULTITHREADED)`, creates a `DispatcherQueueController` (`DQTYPE_THREAD_CURRENT`) and runs `GetMessageW` on the calling thread. `start_free_threaded()` spawns that thread. Its frame pool is `Direct3D11CaptureFramePool::Create(…, 1, …)`, not `CreateFreeThreaded`, so frames arrive on that thread [V source]. Capture therefore never runs on T0.
- `wasapi` 0.26.0 exposes `initialize_mta()` / `initialize_sta()` / `deinitialize()` [V].

### 2.3 Event loop: plain `windows` loop vs winit / tao

| | Plain `windows` 0.62.2 loop | winit 0.30.13 / tao 0.37.1 |
|---|---|---|
| Raw messages (`WM_HOTKEY`, `WM_POWERBROADCAST`, `WM_WTSSESSION_CHANGE`, `WM_ENDSESSION`, TaskbarCreated) | Direct in your `WNDPROC` | Through `EventLoopBuilderExtWindows::with_msg_hook` (both have it [V]) |
| Windows needed | None (hidden window) | Built around windows |
| tray-icon / muda | Need "a win32 event loop running on the thread", created on that thread [V tray-icon lib.rs] | Same, plus `set_event_handler` → `EventLoopProxy` |
| Dependencies | None extra | Large; macOS and Linux backends compiled for the mock |

**Recommendation: the plain loop.** The engine has no visible windows. The messages it needs are raw. And tray-icon only needs any Win32 loop on its thread: it creates its own hidden window, class `tray_icon_app` [V].

### 2.4 Cross-thread rules (each one is a bug you'll otherwise write)

1. **Wake T0 with `PostMessageW` to its window, never `PostThreadMessageW`.** Thread messages aren't tied to a window, so "if the recipient thread is in a modal loop (as used by MessageBox or DialogBox), the messages will be lost" [V]. `TrackPopupMenu` (the tray menu) runs a modal loop. Each queue also has a limit of 10,000 posted messages [V]. So post one `WM_APP_WAKE` per batch, and keep the payload in a crossbeam queue.
2. **The shell window must be top-level, not `HWND_MESSAGE`.** Message-only windows don't receive broadcast messages [V], and TaskbarCreated, `WM_ENDSESSION`, `WM_POWERBROADCAST` and `WM_DISPLAYCHANGE` are broadcasts. Use `WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE` and never show it. tray-icon and global-hotkey do the same (`WS_OVERLAPPED` + `WS_EX_TOOLWINDOW`, with a comment about taskbar ghosts after Explorer restarts) [V].
3. **`HWND` and `HANDLE` are `!Send`** in windows 0.62.2 (`pub struct HWND(pub *mut c_void)`, no `Send` impl) [V]. Pass a `SendHwnd(isize)` newtype; `PostMessageW` itself is thread-safe.
4. **`RegisterHotKey` and `ShutdownBlockReasonCreate` only work for windows created by the calling thread** [V both docs]. All shell calls stay on T0.
5. **No panic may unwind out of a `WNDPROC` or `WINEVENTPROC`.** Wrap each body in `catch_unwind` (§5.3). Use `RefCell::try_borrow_mut` on thread-local state, because Win32 re-enters window procedures.
6. **WinEvent callbacks run on T0 while it pumps messages**, and they can re-enter: "additional events may be triggered, which may cause the hook function to reenter" [V]. Do no work in the callback beyond `try_send` to the detect thread.

### 2.5 OS lifecycle messages (all on T0)

| Message / API | Registration | Action |
|---|---|---|
| `WM_HOTKEY` | `RegisterHotKey(Some(hwnd), id, MOD_ALT \| MOD_NOREPEAT, vk)` | Stamp QPC immediately, send `CoreEvent::Hotkey{id,qpc}` |
| TaskbarCreated | tray-icon registers it and re-adds the icon [V source]. Also "broadcasts this message when the DPI of the primary display changes" [V] | Re-render tray icon for the new DPI |
| `WM_WTSSESSION_CHANGE` (`WTS_SESSION_LOCK`=7 / `UNLOCK`=8) | `WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION)`. It can fail with `RPC_S_INVALID_BINDING` before Terminal Services is ready (wait for `Global\TermSrvReadyEvent`) [V] | Pause the buffer while locked |
| `WM_POWERBROADCAST` `PBT_APMSUSPEND` / `PBT_APMRESUMEAUTOMATIC` | Broadcast. `RegisterSuspendResumeNotification(HANDLE(hwnd.0), DEVICE_NOTIFY_WINDOW_HANDLE)` for Modern Standby [Inference] | Stop / restart media |
| `PBT_POWERSETTINGCHANGE` + `GUID_CONSOLE_DISPLAY_STATE` | `RegisterPowerSettingNotification(HANDLE(hwnd.0), &GUID_CONSOLE_DISPLAY_STATE, DEVICE_NOTIFY_WINDOW_HANDLE)` [V signatures] | Display off → pause capture. **Read the `POWERBROADCAST_SETTING` in `lParam` synchronously**; the pointer is invalid after you return [Inference] |
| `WM_QUERYENDSESSION` / `WM_ENDSESSION` | Broadcast | If a save is in flight, `ShutdownBlockReasonCreate(hwnd, w!("Saving a clip…"))`. Flush logs. Destroy the block reason when done |
| `WM_DISPLAYCHANGE` | Broadcast | Tell media to re-evaluate monitors and HDR |

### 2.6 Timers, EcoQoS and priorities

- **Timer resolution.** Since Windows 10 2004 `timeBeginPeriod` is per-process. "Starting with Windows 11, if a window-owning process becomes fully occluded, minimized, or otherwise invisible or inaudible to the end user, Windows does not guarantee a higher resolution" [V]. Use `CreateWaitableTimerExW(None, PCWSTR::null(), CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, TIMER_ALL_ACCESS.0)` (Windows 10 1803+) for the pacer and the mouse poller [V].
- **EcoQoS.** "If an application does not explicitly enable `PROCESS_POWER_THROTTLING_EXECUTION_SPEED`, the system will use its own heuristics to automatically infer a Quality of Service level" [V]. While buffering, media should opt **out**: `ControlMask = EXECUTION_SPEED | IGNORE_TIMER_RESOLUTION`, `StateMask = 0`. Control may stay on the heuristics.
- **Don't raise process priority classes.** Use MMCSS for audio and capture threads. Note that Task Scheduler starts tasks at priority **7 → `BELOW_NORMAL_PRIORITY_CLASS`** by default [V]. That matters for the dev launcher (§8.5).

### 2.7 Skeletons

```rust
// crates/kairo-engine/src/main.rs — one binary, several roles
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // dev builds: add --console → AllocConsole
fn main() -> std::process::ExitCode {
    let args = cli::Args::from_env();
    match args.role {
        cli::Role::Control  => control::run(args),   // default: Run key, Start menu, UI relaunch
        cli::Role::Media    => media::run(args),     // only ever spawned by control
        cli::Role::Ensure   => lifecycle::ensure(),  // scheduled probe: start control if its mutex is free
        cli::Role::Shutdown => ipc::cli_shutdown(),  // NSIS PREINSTALL hook / dev loop
    }
}
```

```rust
// control::run — sketch
pub fn run(args: Args) -> ExitCode {
    let dirs = paths::AppDirs::resolve();                 // %LOCALAPPDATA%\<identifier>\{logs,crashes,outbox,state,cache}
    let _one = match lifecycle::SingleInstance::acquire() {
        Ok(g) => g,
        Err(lifecycle::AlreadyRunning) => { let _ = ipc::forward_args(&args); return ExitCode::SUCCESS; }
    };
    let _log = logging::init(&dirs, "control");
    crash::install_in_process(&dirs);                     // control crashes are rare: in-process dump
    let settings = settings::load_or_default(&dirs);
    let _telemetry = telemetry::init(&settings, &dirs);   // None unless opted in (§5.4)
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2).thread_name("kairo-rt").enable_all().build().expect("tokio");
    let (core_tx, core_rx) = tokio::sync::mpsc::unbounded_channel();
    let (shell_tx, shell_rx) = crossbeam_channel::unbounded();
    let hwnd_slot = std::sync::Arc::new(std::sync::OnceLock::new());
    rt.spawn(core::run(core_rx, shell_tx, hwnd_slot.clone(), dirs.clone(), settings));
    rt.spawn(ipc::serve_ui(core_tx.clone()));
    detect::spawn(core_tx.clone());
    crash::spawn_server(&dirs, core_tx.clone());          // minidumper::Server for media
    let code = shell::run(core_tx, shell_rx, hwnd_slot);  // blocks: Win32 loop on the main thread
    rt.shutdown_timeout(std::time::Duration::from_secs(3));
    code
}
```

```rust
// shell.rs — T0 (windows 0.62.2; features: Win32_Foundation, Win32_Graphics_Gdi,
// Win32_UI_WindowsAndMessaging, Win32_System_LibraryLoader, Win32_System_Com, Win32_System_RemoteDesktop)
use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::RemoteDesktop::{WTSRegisterSessionNotification, NOTIFY_FOR_THIS_SESSION};
use windows::Win32::UI::WindowsAndMessaging::*;

pub const WM_APP_WAKE: u32 = WM_APP + 1;

#[derive(Clone, Copy)]
pub struct SendHwnd(isize);
unsafe impl Send for SendHwnd {}
unsafe impl Sync for SendHwnd {}
impl SendHwnd {
    pub fn wake(self) {
        let _ = unsafe { PostMessageW(Some(HWND(self.0 as *mut _)), WM_APP_WAKE, WPARAM(0), LPARAM(0)) };
    }
}

thread_local! { static STATE: std::cell::RefCell<Option<ShellState>> = const { std::cell::RefCell::new(None) }; }

pub fn run(core: CoreSender, cmds: crossbeam_channel::Receiver<ShellCmd>,
           slot: std::sync::Arc<std::sync::OnceLock<SendHwnd>>) -> std::process::ExitCode {
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.ok().expect("COM STA");
    let hinst = unsafe { GetModuleHandleW(None) }.expect("module handle");
    let class = w!("KairoControlShell");
    let wc = WNDCLASSEXW {
        cbSize: size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: Some(wndproc),
        hInstance: hinst.into(),
        lpszClassName: class,
        ..Default::default()
    };
    assert_ne!(unsafe { RegisterClassExW(&wc) }, 0);
    // Top-level and never shown: message-only windows don't receive broadcasts.
    let hwnd = unsafe {
        CreateWindowExW(WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE, class, w!("Kairo"), WS_OVERLAPPED,
                        0, 0, 0, 0, None, None, Some(hinst.into()), None)
    }.expect("shell window");
    let _ = slot.set(SendHwnd(hwnd.0 as isize));
    STATE.with(|s| *s.borrow_mut() = Some(ShellState::new(hwnd, core, cmds))); // tray, hotkeys, WinEvent hook
    let _ = unsafe { WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION) }; // retry later on RPC_S_INVALID_BINDING
    let mut msg = MSG::default();
    loop {
        match unsafe { GetMessageW(&mut msg, None, 0, 0) }.0 {
            0 => break,                                    // WM_QUIT
            -1 => { tracing::error!("GetMessageW failed"); break; }
            _ => unsafe { let _ = TranslateMessage(&msg); DispatchMessageW(&msg); },
        }
        // tray-icon and muda emit on their global channels from inside DispatchMessageW on this thread.
        STATE.with(|s| if let Ok(mut g) = s.try_borrow_mut() { if let Some(st) = g.as_mut() { st.drain_tray_events(); } });
    }
    std::process::ExitCode::SUCCESS
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    // A panic must not unwind out of an extern "system" fn: Rust would abort via __fastfail (no dump).
    let handled = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        STATE.with(|s| match s.try_borrow_mut() {
            Ok(mut g) => g.as_mut().and_then(|st| st.on_message(hwnd, msg, wp, lp)),
            Err(_) => None, // re-entrant: fall through to DefWindowProc
        })
    }));
    match handled { Ok(Some(r)) => r, _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) } }
}

impl ShellState {
    fn on_message(&mut self, _hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        match msg {
            WM_HOTKEY => { self.core.send(CoreEvent::Hotkey { id: wp.0 as i32, qpc: qpc_now() }); Some(LRESULT(0)) }
            WM_APP_WAKE => { while let Ok(c) = self.cmds.try_recv() { self.apply(c); } Some(LRESULT(0)) }
            WM_WTSSESSION_CHANGE => { self.core.send(CoreEvent::Session(wp.0 as u32)); Some(LRESULT(0)) }
            WM_POWERBROADCAST => { self.core.send(power::decode(wp, lp)); Some(LRESULT(1)) } // decode lParam now
            WM_DISPLAYCHANGE => { self.core.send(CoreEvent::DisplaysChanged); None }
            WM_QUERYENDSESSION => Some(LRESULT(1)),
            WM_ENDSESSION => { if wp.0 != 0 { self.end_session_flush(); } Some(LRESULT(0)) }
            _ => None,
        }
    }
}
```

```rust
// media: opt out of EcoQoS + timer-resolution throttling while buffering
use windows::Win32::System::Threading::*;
fn opt_out_of_power_throttling() -> windows::core::Result<()> {
    let state = PROCESS_POWER_THROTTLING_STATE {
        Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        ControlMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED | PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION,
        StateMask: 0, // 0 = mechanism off
    };
    unsafe { SetProcessInformation(GetCurrentProcess(), ProcessPowerThrottling,
        &state as *const _ as *const core::ffi::c_void, size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32) }
}
```

---

## 3. Tray, hotkeys, mouse buttons, elevated games

### 3.1 tray-icon 0.26.1 + muda 0.21.2 facts [V source]

- "On Windows … an event loop must be running on the thread"; it "doesn't need to be the main thread but you have to create the tray icon on the same thread as the event loop".
- It creates its own hidden window. It handles TaskbarCreated (remove, then re-register). It calls `ChangeWindowMessageFilterEx(…, MSGFLT_ALLOW)` for TaskbarCreated, so an elevated Kairo would also recover. If Explorer isn't ready at startup it "keep[s] the window alive and wait[s] for TaskbarCreated". That matters for Run-key launches.
- Events: `TrayIconEvent::{Click, DoubleClick, Enter, Move, Leave}` carrying `button`, `button_state`, `rect` and `position`. They arrive on `TrayIconEvent::receiver()` (crossbeam) or `set_event_handler`. Menu clicks arrive on `MenuEvent::receiver()`.
- `menu_on_left_click` defaults to **true**. For "left click = open Kairo, right click = menu", set `with_menu_on_left_click(false)`.
- `with_guid(u128)` gives the icon a stable identity. Windows keys "pinned vs overflow" on it, and with a GUID it "survives" binary moves **if** the exe is Authenticode-signed by the same publisher. Without a GUID the identity is path + counter.
- Icons: `Icon::from_rgba(rgba, w, h)`, `Icon::from_path`, or `Icon::from_resource(ordinal, Some((w, h)))` from the exe's own `.ico` resource. The resource route lets Windows pick the DPI-correct frame. Embed the icons with `embed-resource` (§9).
- muda: `Menu::new()`, `MenuItem::with_id(id, text, enabled, None)`, `CheckMenuItem::with_id(id, text, enabled, checked, None)`, `PredefinedMenuItem::separator()`, `menu.append_items(&[…])`.

Tray states from the plan: waiting, REC game, REC screen, paused, warning. Add **safe mode** and **elevated game: hotkeys may not work**.

### 3.2 Hotkeys: call `RegisterHotKey` yourself

Facts [V]:
- **Ids** must be in 0x0000–0xBFFF.
- **Same id twice:** registering the same `(hWnd, id)` again doesn't replace the first. "It is maintained along with the new hot key. The application must explicitly call UnregisterHotKey." So always unregister before re-registering.
- **Taken combinations:** the call "typically … fails if the keystrokes … have already been registered". Some OS defaults (e.g. PrintScreen) can be overridden while your window is in the foreground.
- **F12** "is reserved for use by the debugger at all times".
- **`MOD_NOREPEAT` (0x4000)** means auto-repeat doesn't yield more notifications. The next `WM_HOTKEY` comes only after the key is released and pressed again.
- **Failure code:** `ERROR_HOTKEY_ALREADY_REGISTERED` = 1409, i.e. `WIN32_ERROR(1409)` in windows 0.62.2. Compare with `e.code() == ERROR_HOTKEY_ALREADY_REGISTERED.to_hresult()`; `to_hresult` lives in `extensions/Win32/Foundation/WIN32_ERROR.rs`.

`global-hotkey` 0.9.0 does call `RegisterHotKey` with `MOD_NOREPEAT` always, and maps 1409 to `Error::AlreadyRegistered(hotkey)` [V source]. But:
- (a) For each `WM_HOTKEY` it spawns a new OS thread that polls `GetAsyncKeyState` every 50 ms to synthesise `Released`. Under UIPI that poll returns 0 at once.
- (b) It pins windows-sys 0.59, a third copy in the build.
- (c) It adds a hidden window per manager.

Direct use costs about 60 lines and gives exact error codes. **Recommend direct.**

Each binding reports a status to the UI: `Registered | InUse | Reserved (F12, Win+) | Invalid | Failed{hresult}`. When a change fails, re-register the old binding and show "Alt+F8 is used by another app".

```rust
use windows::Win32::Foundation::{HWND, ERROR_HOTKEY_ALREADY_REGISTERED};
use windows::Win32::UI::Input::KeyboardAndMouse::*;

pub fn set(&mut self, id: i32, b: Binding) -> HotkeyStatus {
    if !(0..=0xBFFF).contains(&id) { return HotkeyStatus::Invalid; }
    if b.vk == VK_F12.0 || b.win { return HotkeyStatus::Reserved; }
    if self.active.remove(&id).is_some() {
        let _ = unsafe { UnregisterHotKey(Some(self.hwnd), id) }; // same (hwnd,id) would otherwise stack
    }
    let mut mods = MOD_NOREPEAT;
    if b.ctrl { mods |= MOD_CONTROL; }
    if b.alt { mods |= MOD_ALT; }
    if b.shift { mods |= MOD_SHIFT; }
    match unsafe { RegisterHotKey(Some(self.hwnd), id, mods, b.vk as u32) } {
        Ok(()) => { self.active.insert(id, b); HotkeyStatus::Registered }
        Err(e) if e.code() == ERROR_HOTKEY_ALREADY_REGISTERED.to_hresult() => HotkeyStatus::InUse,
        Err(e) => HotkeyStatus::Failed { hresult: e.code().0 },
    }
}
```

Display names come from `MapVirtualKeyW(vk, MAPVK_VK_TO_VSC)` → `GetKeyNameTextW((scan << 16) as i32, &mut buf)` [V signatures]. Store bindings as VK + modifiers, not as text. Report 03 covers which defaults survive UIPI and `RIDEV_NOHOTKEYS` (spike S4).

### 3.3 Mouse side buttons (opt-in, only while bound)

- The return value is zero when "UI Privilege Isolation (UIPI) prevents the calling thread from accessing the foreground thread", or when the current desktop isn't active [V]. So mouse triggers can't work while an elevated game has focus or on the lock screen.
- Use only the **high bit** (`0x8000`). The low "pressed since last call" bit "should not be relied upon" because another app can consume it [V].
- `GetAsyncKeyState` reads physical buttons, not logical (swapped) ones [V].
- An 8–10 ms poll catches normal clicks (≈60–120 ms long) [Estimate]. OBS polls every 25 ms (report 03).

```rust
fn mouse_poll(stop: &AtomicBool, buttons: [VIRTUAL_KEY; 2], tx: &CoreSender) -> windows::core::Result<()> {
    let timer = unsafe { CreateWaitableTimerExW(None, PCWSTR::null(), CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, TIMER_ALL_ACCESS.0) }?;
    let due: i64 = -80_000; // 8 ms relative, 100 ns units
    unsafe { SetWaitableTimer(timer, &due, 8, None, None, false) }?;
    let mut prev = [false; 2];
    while !stop.load(Ordering::Relaxed) {
        unsafe { WaitForSingleObject(timer, 100) };
        for (i, vk) in buttons.iter().enumerate() {
            let down = (unsafe { GetAsyncKeyState(vk.0 as i32) } as u16 & 0x8000) != 0;
            if down && !prev[i] { tx.send(CoreEvent::MouseTrigger { button: i as u8, qpc: qpc_now() }); }
            prev[i] = down;
        }
    }
    unsafe { CloseHandle(timer) }
}
```

### 3.4 Is the foreground game elevated?

- `OpenProcessToken` needs a process handle with `PROCESS_QUERY_LIMITED_INFORMATION`. "To get a handle to an elevated process from within a non-elevated process, both processes must be started from the same account"; a different account needs `SE_DEBUG_NAME` [V, OpenProcessToken remarks].
- `QueryFullProcessImageNameW` is one of the functions `PROCESS_QUERY_LIMITED_INFORMATION` grants [V].

So the house rule ("PQLI only") holds:

1. `GetWindowThreadProcessId(hwnd, Some(&mut pid))`.
2. `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid)`.
3. `QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, …)`.
4. `OpenProcessToken(h, TOKEN_QUERY, &mut tok)`.
5. `GetTokenInformation(tok, TokenElevation, …)` → `TOKEN_ELEVATION { TokenIsElevated }`.
6. Optionally `TokenIntegrityLevel` → `TOKEN_MANDATORY_LABEL`. The RID comes from `GetSidSubAuthority(sid, count - 1)` after `GetSidSubAuthorityCount`. Compare with `SECURITY_MANDATORY_HIGH_RID` = 0x3000 and `SECURITY_MANDATORY_MEDIUM_RID` = 0x2000 [V constants].

Treat `ERROR_ACCESS_DENIED` at step 4 as **Unknown**, not Elevated: protected processes and anti-cheat handle filtering also deny it [Inference]. Do this once per foreground change, cache it per PID, and close every handle (RAII guard).

UX: "This game runs as administrator, so Kairo's hotkeys and mouse buttons may not work." Offer the v1.1 elevated-mode option from the brief.

---

## 4. IPC

### 4.1 Transport: tokio vs interprocess [V source of both]

| | tokio 1.54.0 `net::windows::named_pipe` | interprocess 2.4.6 |
|---|---|---|
| Explicit DACL | `unsafe ServerOptions::create_with_security_attributes_raw(addr, *mut c_void)` | `ListenerOptionsExt::security_descriptor(SecurityDescriptor::deserialize(&U16CStr))` (SDDL) |
| `PIPE_REJECT_REMOTE_CLIENTS` | `reject_remote_clients`, **default `true`** | `accept_remote: false` default → flag set |
| First-instance guard | `first_pipe_instance(true)` | First instance always sets `FILE_FLAG_FIRST_PIPE_INSTANCE` |
| Client impersonation level | `ClientOptions::new()` defaults to `SECURITY_IDENTIFICATION \| SECURITY_SQOS_PRESENT` | Client `CreateFileW(…, FILE_FLAG_OVERLAPPED, …)`, **no SQOS flags**. A squatting server could impersonate the client, which matters if the UI is ever elevated [Inference] |
| Busy pipe | Docs show a retry loop on `ERROR_PIPE_BUSY` | `WaitNamedPipeW` internally |
| Mac | Use `tokio::net::UnixListener` / `UnixStream` behind a `cfg` | Unified local sockets. On macOS, `GenericNamespaced` maps to `/tmp/<name>` via the **deprecated** `SpecialDirUdSocket`; use `GenericFilePath` |
| Peer PID | `AsRawHandle` → `GetNamedPipeClientProcessId` | `peer_creds()`, or `client_process_id()` |

**Recommendation: tokio direct**, with a ~80-line `transport` module (`cfg(windows)` pipes, `cfg(unix)` Unix socket in `std::env::temp_dir()`, which is per-user on macOS). It needs no extra crate, gives the client its SQOS default, and gives full control. interprocess remains acceptable for the server if the team prefers.

### 4.2 Pipe security

- **The default DACL is wrong for Kairo.** "The ACLs in the default security descriptor for a named pipe grant full control to the LocalSystem account, administrators, and the creator owner. They also grant read access to members of the Everyone group and the anonymous account" [V].
- **SDDL:** `D:P(A;;GA;;;SY)(A;;GA;;;<current-user-SID>)`. Get the SID with `GetTokenInformation(TokenUser)` → `TOKEN_USER.User.Sid` → `ConvertSidToStringSidW`. Then `ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl, SDDL_REVISION_1, &mut psd, None)` and `LocalFree` when done [V signatures]. Give every instance the same attributes.
- **User SID, not logon SID, on purpose.** Microsoft: "To prevent remote users or users on a different terminal services session from accessing a named pipe, use the logon SID on the DACL" [V]. Kairo *wants* the same user's SSH session (session 0) to reach the engine for `kairo-cli`. Pipe names are not per-session (named kernel objects are; pipes aren't listed) [V].
  - Don't add a deny ACE for `NU` (NETWORK). OpenSSH logons may carry that SID and it would block the dev CLI [Unverified; check `whoami /groups` over SSH].
- **Instance mismatches fail.** All instances must use the same type, access, instance count and timeout, or creation fails with `ERROR_ACCESS_DENIED`. With `FILE_FLAG_FIRST_PIPE_INSTANCE`, a pre-existing pipe of that name also gives `ERROR_ACCESS_DENIED` [V]. Treat that at startup as "another engine (or a squatter) owns the name": log it, show a tray error, exit.
- **Name:** `\\.\pipe\kairo-<user-SID>`. It's unique per user, and a second session of the same user collides on it, which acts as a natural single-engine-per-user guard.
- **Optional hardening:**
  - The client checks `GetNamedPipeServerProcessId` → image path equals the installed `kairo-engine.exe`.
  - The server checks client image paths (`Kairo.exe`, `kairo-cli.exe`) in release builds.
  - It's defence in depth only: same-user processes are already trusted [Inference].

### 4.3 Framing

`tokio_util::codec::LengthDelimitedCodec` defaults to a **4-byte big-endian** length and an **8 MiB** max frame [V source]. Use:

```rust
pub fn codec() -> tokio_util::codec::LengthDelimitedCodec {
    tokio_util::codec::LengthDelimitedCodec::builder()
        .length_field_type::<u32>()
        .little_endian()
        .max_frame_length(1 << 20)      // 1 MiB; anything bigger is paginated or a file path
        .new_codec()
}
```

Each frame holds exactly one UTF-8 JSON object (no batches). If you ever want hand-typed debugging, `LinesCodec::new_with_max_length(1 << 20)` is the NDJSON alternative. Its error is `LinesCodecError::MaxLineLengthExceeded` [V]. Pick one and use it everywhere. The plan chose length-prefixed.

### 4.4 JSON-RPC: hand-rolled vs jsonrpsee

- jsonrpsee 0.26.1 can dispatch over any transport via `Methods::raw_json_request(&self, request: &str, buf_size: usize) -> Result<(Box<RawValue>, mpsc::Receiver<Box<RawValue>>), serde_json::Error>` [V].
- But its server stack is built around HTTP/WebSocket, and subscriptions follow its own id/subscription protocol.
- It provides no TypeScript generation.

**Hand-rolled is smaller and fits the ts-rs flow:** a `Request { jsonrpc, id, method, params: Option<Box<RawValue>> }` envelope, a `match` on the method, and `serde_json::from_str::<P>(params)`. JSON-RPC 2.0 explicitly allows one peer to be both client and server ("One implementation of this specification could easily fill both of those roles") [V]. So engine→UI notifications are valid. Reply to batch arrays with `-32600` and document it.

### 4.5 TypeScript (and schema) generation

- **ts-rs 12.0.1.** `#[derive(TS)] #[ts(export)]` exports during `cargo test` to `TS_RS_EXPORT_DIR` (default `./bindings`). Configure it in `.cargo/config.toml`:
  ```toml
  [env]
  TS_RS_EXPORT_DIR = { value = "ui/src/gen", relative = true }
  TS_RS_LARGE_INT = "number"   # default "bigint" is wrong for JSON.parse'd values
  ```
- The `serde-compat` default feature honours `rename`, `rename_all`, `tag`, `content`, `untagged`, `skip`, `flatten` and `default`. `#[ts(optional_fields)]` emits `t?: T`.
- Programmatic export: `TS::export_all(&Config)`, `TS::name(&Config)`, `Config::from_env()` [V source]. Keep IDs and timestamps below 2^53.
- A test in `kairo-rpc` writes `methods.ts` (`RpcMethods = { "clip.save": { params: SaveParams; result: JobRef }, … }`) from the method table, using `<T as TS>::name(&cfg)`. CI runs `cargo test -p kairo-rpc && git diff --exit-code ui/src/gen`.
- schemars 1.2.3 `#[derive(JsonSchema)]` on the same types feeds the contract tests from the plan.
- Don't use specta / tauri-specta (still RC after ~3 years). The UI calls one generic Tauri command, so command codegen adds nothing.

### 4.6 Protocol shape

**Handshake (must be the first request).**
```json
→ {"jsonrpc":"2.0","id":1,"method":"engine.hello","params":{"client":{"name":"kairo-ui","version":"0.4.0"},"protocol":{"major":1,"minor":3},"topics":["engine.state","clip.saved","library.changed"]}}
← {"jsonrpc":"2.0","id":1,"result":{"protocol":{"major":1,"minor":2},"engine":{"version":"0.4.0","build":"<git sha>","pid":1234,"startedAt":"…"},"features":["screenshots","hevc"],"safeMode":null,"session":"01JC…"}}
```

**Versioning rules**
- Same `major` is required. Otherwise reply `-32002 ProtocolMismatch`; the UI shows "updating" and calls `engine.shutdown` so the new engine starts.
- Within a major, changes are additive only: new methods, new optional params, new result fields, new notifications, new enum variants with `#[serde(other)] Unknown`.
- Never use `deny_unknown_fields`. Unknown notifications are ignored.
- Use `features` strings for optional capabilities rather than minor-version arithmetic.

**Methods** (namespace.verb). Long operations return `{ jobId }` at once and report through notifications.

| Method | Params → Result |
|---|---|
| `engine.hello` / `engine.status` / `engine.shutdown` / `engine.restartMedia` | see above / `EngineStatus` (state, buffer, capture, encoder, sources, game, `sessionLocked`, `displayOn`, warnings) / `{reason}` → `{}` / `{reason}` → `{}` |
| `events.subscribe` | `{topics}` → `{topics}`. Rate-limited topics (`buffer.status` ≤2 Hz, `levels` ≤10 Hz) are sent only while subscribed |
| `clip.save` / `screenshot.take` | `{preSecs?, postSecs?, origin}` → `{jobId}` / `{}` → `{jobId}` |
| `settings.get` / `settings.patch` | → `{revision, settings}` / `{baseRevision, patch}` → `{revision, settings, restartRequired[]}`. A stale revision returns `-32005 Conflict` |
| `hotkeys.list` / `hotkeys.set` | → `[{action, binding, status}]` / `{action, binding}` → `{status}` |
| `library.list` / `.get` / `.update` / `.delete` / `.reveal` / `.rescan` | `{cursor?, limit≤200, filter, sort}` → `{items, nextCursor?}` / … / `{ids, mode:"recycle"\|"permanent"}` / `{id}` / → `{jobId}` |
| `export.start` / `job.cancel` | `{clipId, preset, range?}` → `{jobId}` / `{jobId}` → `{}` |
| `games.list` / `games.mark` | → `Game[]` / `{exePath, isGame}` → `Game` |
| `diagnostics.capabilities` / `.bundle` / `.setLogLevel` / `.flag` | → report / `{includeCrashes}` → `{path, files[]}` / `{filter}` (dev/beta) / `{note}` (the plan's flag key) |
| `crash.pending` / `crash.send` / `crash.discard` | Consent flow (§5.4) |

**Notifications (engine → UI):**
- `engine.state`, `buffer.status`;
- `clip.saved` / `clip.failed`, `screenshot.saved`;
- `job.progress` / `job.done`;
- `library.changed {ids, kind}`, `game.changed`, `hotkeys.changed`, `settings.changed {revision}`;
- `levels`, `warning {code, message}`, `error`, `crash.pending`.

The plan's camelCase names (`bufferStatus`, `clipSaved`…) map one-to-one; just pick one style for both methods and events.

**Errors.**
- Standard: `-32700` parse, `-32600` invalid request, `-32601` method not found, `-32602` invalid params, `-32603` internal [V].
- `-32000..-32099` is "reserved for implementation-defined server-errors" [V]. Use it:
  - `-32001 NotReady`, `-32002 ProtocolMismatch`, `-32003 Busy`, `-32004 NotFound`, `-32005 Conflict`;
  - `-32006 HotkeyInUse`, `-32007 CaptureUnavailable`, `-32008 EncoderUnavailable`, `-32009 Denied`, `-32010 Cancelled`, `-32011 Unsupported`, `-32012 SafeMode`.
- `data: {kind, …}` is a ts-rs enum, so the UI can switch on it.

**Back-pressure.** Each session gets a bounded outbound queue (256). Responses always wait. `buffer.status` and `levels` keep only the latest value and are dropped when the queue is full. A slow UI can never stall control.

### 4.7 Who speaks the protocol

- **Tauri core (`ui/src-tauri`).** It is the only UI client:
  - one `#[tauri::command] async fn engine_call(method: String, params: serde_json::Value)`;
  - notifications re-emitted with `app.emit("engine://notification", …)`;
  - reconnect with backoff; on `NotFound`, start `kairo-engine.exe`.
- **`kairo-cli`** (dev): `kairo-cli call clip.save '{"preSecs":30}'`. It runs over SSH in session 0 as the same user and reaches control in session 1 (§4.2). This is how the harness triggers saves unattended.
- **`kairo-mock-engine`** on the Mac: the same `kairo-rpc` types, a Unix socket, and scenario data. Vitest uses `mockIPC` (plan).
- **Media link.** Control spawns media with `tokio::process::Command` (`stdin(Stdio::piped())`, `stdout(Stdio::piped())`, `kill_on_drop(true)`). It frames `tokio::io::join(child.stdout, child.stdin)` with the same codec, and assigns `child.raw_handle()` to the Job Object [V APIs exist].
  - Nothing to name, nothing to squat, and the link's lifetime is tied to the child.
  - Media must never write to stdout: its logs go to its own file.

```rust
// Windows server sketch (tokio 1.54.0 + windows 0.62.2)
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
use windows::core::HSTRING;
use windows::Win32::Foundation::{LocalFree, HLOCAL};
use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
use windows::Win32::Security::Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1};

pub struct PipeSecurity(PSECURITY_DESCRIPTOR);
unsafe impl Send for PipeSecurity {}   // immutable after creation
unsafe impl Sync for PipeSecurity {}
impl PipeSecurity {
    pub fn current_user_only() -> windows::core::Result<Self> {
        let sid = current_user_sid_string()?; // TokenUser + ConvertSidToStringSidW (LocalFree the string)
        let sddl = HSTRING::from(format!("D:P(A;;GA;;;SY)(A;;GA;;;{sid})"));
        let mut psd = PSECURITY_DESCRIPTOR::default();
        unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(&sddl, SDDL_REVISION_1, &mut psd, None)? };
        Ok(Self(psd))
    }
}
impl Drop for PipeSecurity { fn drop(&mut self) { unsafe { let _ = LocalFree(Some(HLOCAL(self.0.0))); } } }

fn instance(name: &str, sec: &PipeSecurity, first: bool) -> std::io::Result<NamedPipeServer> {
    let mut sa = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: sec.0.0,
        bInheritHandle: false.into(),
    };
    let mut o = ServerOptions::new();
    o.first_pipe_instance(first).reject_remote_clients(true).max_instances(8); // same count for every instance
    unsafe { o.create_with_security_attributes_raw(name, &mut sa as *mut _ as *mut core::ffi::c_void) }
}

pub async fn serve_ui(core: CoreSender) -> std::io::Result<()> {
    let name = pipe_name(); // \\.\pipe\kairo-<SID>
    let sec = PipeSecurity::current_user_only().map_err(std::io::Error::other)?;
    let mut server = instance(&name, &sec, true)?;
    loop {
        server.connect().await?;
        let connected = server;
        server = instance(&name, &sec, false)?; // create the next instance BEFORE handing off (tokio docs)
        let core = core.clone();
        tokio::spawn(async move {
            if let Err(e) = session::run(connected, core).await { tracing::debug!(%e, "ui session ended"); }
        });
    }
}
```

```rust
// Client (Tauri core / kairo-cli) — tokio's defaults include SECURITY_IDENTIFICATION
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient};
pub async fn connect(name: &str) -> std::io::Result<NamedPipeClient> {
    loop {
        match ClientOptions::new().open(name) {
            Ok(c) => return Ok(c),
            Err(e) if e.raw_os_error() == Some(windows::Win32::Foundation::ERROR_PIPE_BUSY.0 as i32) => {}
            Err(e) => return Err(e), // NotFound → engine not running → start it
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}
```

---

## 5. Logging, crash capture, Sentry

### 5.1 Logs

**Layout under `%LOCALAPPDATA%\<identifier>\logs\`:**
- one file family **per process** (`control.*`, `media.*`, `ui.*`): rolling appenders are not multi-process safe [Inference];
- JSON lines in beta, compact text in dev.

**Rotation:**
- Time-based (verified API): `RollingFileAppender::builder().rotation(Rotation::DAILY).filename_prefix("control").filename_suffix("log").max_log_files(14).build(dir)?`.
  - Dates are **UTC** (`OffsetDateTime::now_utc`).
  - `max_log_files` deletes the oldest matching files.
  - Don't use `latest_symlink` on Windows, where symlinks need privileges [Inference].
- Size-based, as in the plan ("10 × 10 MB"): `FileRotate::new(dir.join("control.log"), AppendCount::new(10), ContentLimit::BytesSurpassed(10 << 20), Compression::None, None)`. It implements `Write` [V].

**Non-blocking writer:** wrap either one with `tracing_appender::non_blocking(w)`, or the builder with `.lossy(true)` (the default) and `.buffered_lines_limit(128_000)` (the default). Lossy drops lines when the buffer is full instead of blocking a capture thread [V]. Keep the `WorkerGuard` alive until `WM_ENDSESSION` or shutdown.

**Filtering:** `EnvFilter` (feature `env-filter`), default `info` and `kairo=debug` in dev. Change it at runtime with `tracing_subscriber::reload::Layer` behind `diagnostics.setLogLevel`.

**Rate-limiting:** cap repeated warnings (capture errors at 60 Hz) to 1/s per call site, with a "suppressed N" counter.

### 5.2 Redaction

- The fmt layer formats each event into one buffer and calls `io::Write::write_all` **once per event** on `make_writer_for(meta)` [V `fmt_layer.rs`]. So a wrapping `MakeWriter` sees whole lines and can redact before bytes reach disk:
  - `C:\Users\<name>` → `%USERPROFILE%`;
  - the username → `<user>`;
  - `COMPUTERNAME` → `<host>`;
  - the library root → `<library>`;
  - SIDs (`S-1-5-21-…`) → `<sid>`.
- Content rules: never log window titles (Discord DM titles), clipboard, key codes other than bound hotkeys, or IPs. Game identity is logged as the exe name plus a hashed path.

```rust
struct Redact<M>(M, std::sync::Arc<Redactor>);
impl<'a, M: tracing_subscriber::fmt::MakeWriter<'a>> tracing_subscriber::fmt::MakeWriter<'a> for Redact<M> {
    type Writer = RedactWriter<M::Writer>;
    fn make_writer(&'a self) -> Self::Writer { RedactWriter(self.0.make_writer(), self.1.clone()) }
}
impl<W: std::io::Write> std::io::Write for RedactWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let line = String::from_utf8_lossy(buf);
        self.0.write_all(self.1.apply(&line).as_bytes())?;   // one event per call (verified)
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> { self.0.flush() }
}
```

### 5.3 Crash capture design

**Facts that shape it [V]:**
- **Rust aborts can't be caught.** Rust 1.99.1 `abort_internal()` on x86_64 Windows executes `int 0x29` with `FAST_FAIL_FATAL_APP_EXIT`, which "will terminate the process immediately without running any in-process exception handlers". Microsoft: with `__fastfail` "no exception handlers are invoked".
  - This covers `panic = "abort"`, `std::process::abort()`, double panics, and panics in `extern "system"` callbacks.
  - In-process handlers (`crash-handler`, Sentry) never see them.
- **What crash-handler 0.8.1 does catch** (Windows): `SetUnhandledExceptionFilter`, `AddVectoredExceptionHandler` (for `EXCEPTION_STACK_OVERFLOW`), `_set_invalid_parameter_handler`, `_set_purecall_handler`, and a `SIGABRT` handler.
  - API: `CrashHandler::attach(unsafe { make_crash_event(move |cc: &CrashContext| CrashEventResult::Handled(client.request_dump(cc).is_ok())) })`.
  - `handler.simulate_exception(Some(code))` writes a dump of the current context on demand.
- **minidumper 0.11.0:**
  - `Server::with_name(SocketName::Path(p))`;
  - `server.run(Box<dyn ServerHandler>, &AtomicBool, Option<Duration>)`;
  - handler methods `create_minidump_file() -> io::Result<(File, PathBuf)>`, `on_minidump_created(Result<MinidumpBinary, Error>) -> LoopAction`, `on_message(kind, Vec<u8>)`, `on_client_connected/disconnected`;
  - client `Client::with_name`, `request_dump(&CrashContext)`, `send_message(kind, buf)`, `ping()`.
  - On Windows it talks over **AF_UNIX** sockets, implemented in-crate.
- **minidump-writer 0.13.0** can dump another process: `MinidumpWriter::dump_crash_context(&CrashContext, …)` uses the external path when the PID isn't ours. `dump_local_context(…)` dumps the current process and is documented as "inherently unreliable".

**Design:**
1. **Media worker.** At startup it calls `crash_handler::CrashHandler::attach(...)` with a `minidumper::Client` connected to control's socket (`%LOCALAPPDATA%\<id>\run\crash-<controlpid>.sock`). Dumps are written **out of process** by control's `crashsrv` thread into `crashes\media-<ulid>.dmp`, with a JSON sidecar (build, uptime, safe-mode level, last log lines).
2. **Panics, everywhere.** `std::panic::set_hook` does three things:
   - `tracing::error!` with the backtrace;
   - `crash_handler.simulate_exception(Some(0xE0004B50))`, a Kairo-specific code, so a dump exists **before** any abort;
   - `std::process::exit(EXIT_PANIC)`, so control sees a classified exit code instead of the fast-fail `0xC0000409`.

   `catch_unwind` in every `extern "system"` callback stops a panic there turning into an un-dumped fast-fail.
3. **Hangs.** Media sends a heartbeat every 1 s over the link, with progress counters for capture, video and audio. No heartbeat for 10 s while the process is alive [Estimate] means:
   - control writes a dump of media from outside (`MiniDumpWriteDump(hproc, pid, hfile, MiniDumpWithThreadInfo | …, None, None, None)`, signature verified);
   - then `TerminateProcess` and restart.
4. **Control.** Same panic hook. For native crashes, an in-process `crash-handler` callback writes with `dump_local_context`. It's less reliable, but control has no drivers or FFmpeg. If control dies: the UI restarts it if open, otherwise the plan's `Kairo\Ensure` task (§6.4).
5. **Exit codes.** Read them with `WaitForSingleObject` + `GetExitCodeProcess`:
   - `0` clean;
   - `EXIT_RESTART`, `EXIT_PANIC`;
   - `0xC0000005` access violation, `0xC00000FD` stack overflow, `0xC0000409` fast-fail / stack buffer overrun;
   - anything else unknown.

   Feed them to the restart policy (§6.3).

### 5.4 Sentry: opt-in, EU, deferred upload

- **EU region.** The DSN host is `o<org>.ingest.de.sentry.io`. The region is fixed when the organisation is created [V Sentry docs].
- **Don't call `sentry::init` at all unless the user opted in.** Modes: Never (default) / Ask each time / Always.
- **The `sentry` 0.49.3 `ClientOptions` fields that matter** [V source]: `dsn`, `release`, `environment`, `send_default_pii`, `server_name`, `before_send: Option<Arc<dyn Fn(Event) -> Option<Event>>>`, `before_breadcrumb`, `transport`, `auto_session_tracking`, `enable_logs`, `attach_stacktrace`, `default_integrations`, `max_breadcrumbs`.
- **PII gotcha:** `sentry-contexts` `ContextIntegration::setup` sets `server_name` to the **hostname** whenever it is `None`, whatever `send_default_pii` says [V]. Windows hostnames often contain the owner's name. Set `server_name: Some("redacted".into())` and also clear it in `before_send`.
- **Features:** `default-features = false, features = ["backtrace", "contexts", "panic", "debug-images", "reqwest", "native-tls"]`. The default set also enables `logs`, `metrics` and `release-health`, which send sessions; that's analytics, and the beta has none.
- **`before_send` scrubbing:**
  - `server_name = None`, `user = None`, `request = None`;
  - redact `message`, exception values and breadcrumbs through the same `Redactor`;
  - for `debug_meta.images[*].code_file`, keep only the file name (`debug_id` / `code_id` still symbolicate);
  - drop `extra` keys not on an allowlist.
- **Deferred upload for "Ask each time"** [V APIs]:
  - `Transport` is a trait with `send_envelope(&self, Envelope)`, and `Arc<T: Transport>` is a valid `TransportFactory`.
  - Install an **`OutboxTransport`** that writes `envelope.to_writer(file)` to `outbox\<ulid>.envelope` and never touches the network.
  - At the next start, `crash.pending` lists the outbox plus any `crashes\*.dmp` not yet reported.
  - On "Send", create a client with the real transport, `Envelope::from_path(p)` → `client.send_envelope(env)`, then `client.flush(Some(5s))`, then delete the files.
  - Native dumps become an `Event { level: Level::Fatal, .. }` with `scope.add_attachment(Attachment { buffer, filename, ty: Some(AttachmentType::Minidump), .. })`. That is exactly what `sentry-minidump` does internally [V source].
- **In "Always" mode,** the same code uses the reqwest transport directly.
- **Project settings:**
  - Keep minidump storage off. Sentry drops dumps after processing and doesn't scrub attachments (brief §8.4).
  - Turn on IP scrubbing [Unverified setting name].
  - Upload PDBs from CI with sentry-cli 3.8.0.

```rust
struct OutboxTransport { dir: std::path::PathBuf }
impl sentry::Transport for OutboxTransport {
    fn send_envelope(&self, envelope: sentry::Envelope) {
        let path = self.dir.join(format!("{}.envelope", ulid::Ulid::new()));
        if let Ok(f) = std::fs::File::create(&path) { let _ = envelope.to_writer(f); }
    }
}
// ClientOptions { transport: Some(Arc::new(Arc::new(OutboxTransport { dir }))), .. }
```

### 5.5 Crash-crate comparison

| Option | Process model | Upload | Fits "Ask each time" | Status |
|---|---|---|---|---|
| `sentry` feature `minidump` (`sentry-minidump` 0.49.3, uses `minidumper-child` 0.5) | Spawns a crash-reporter child (same exe, env var `_SENTRY_CRASH_REPORTER_SERVER`). Code before `sentry::init` runs in both processes. Only the first `init` with a DSN starts it | `capture_event` + flush in the reporter at crash time | Only with a custom outbox transport. The reporter clones `ClientOptions`, so a custom transport travels with it [Inference] | Official, new (2026-09-21) |
| `sentry-rust-minidump` 0.18.1 | Same idea (timfish) | Immediate | Same caveat | **Deprecated, no more updates** |
| `minidumper` 0.11.0 + `crash-handler` 0.8.1, **control as server** | No extra process: control already supervises media | Ours (§5.4) | Yes | Maintained (Embark), recommended |

---

## 6. Lifecycle: autostart, single instance, watchdog, safe mode, updates

### 6.1 Autostart

- Write `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, value **`Kairo`** = `"C:\Users\…\AppData\Local\Kairo\kairo-engine.exe" --autostart`, **with quotes**. Use `windows-registry` 0.6.1 (`CURRENT_USER.create(path)?.set_string(name, value)?`).
- Matching Tauri's `productName` matters: the Tauri NSIS uninstaller runs `DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "${PRODUCTNAME}"` on uninstall (not on update) [V `installer.nsi` 2.12.4].
- Read `…\Explorer\StartupApproved\Run\Kairo` to show "disabled in Windows settings". **Never overwrite it**: it records the user's choice.
  - The format is undocumented: a 12-byte value with first DWORD 2 = enabled, 3 = disabled.
  - auto-launch treats a non-zero trailing 8-byte FILETIME as disabled [V source; Microsoft docs: none].
- Why not `auto-launch` 0.6.0 [V source]:
  - it writes `format!("{} {}", app_path, args)` without quotes;
  - its `Dynamic` mode tries **HKLM** first;
  - `enable()` writes `02 00…` to StartupApproved, silently re-enabling what the user disabled.
- MSIX later: `windows.startupTask` (brief §8.2).

### 6.2 Single instance

- Control acquires `CreateMutexW(None, true, w!("Local\\Kairo.Control.<SID>"))`.
  - windows 0.62.2's wrapper returns `Result<HANDLE>` and calls no other API on success, so `GetLastError() == ERROR_ALREADY_EXISTS` straight afterwards is reliable [V source].
  - `Local\` is the session namespace [V].
- The pipe's first-instance flag is the second guard, across sessions.
- A second launch forwards its intent (`--show-library`, `--open <clip>`) over the pipe (`ui.open`) and exits 0.
- UI: `tauri-plugin-single-instance` 2.5.4 uses mutex `{id}-sim` plus `FindWindowW` + `WM_COPYDATA` [V source].

### 6.3 Restart policy and safe mode (control supervising media)

```
on media exit(code, uptime):
  Clean / UserQuit            → stop
  RestartRequested            → restart now (settings changed capture backend, etc.)
  Crash/Hang (classified)     → record {time, code, uptime, safe_level} in state\crashes.json
       backoff = [0.5, 1, 2, 5, 10, 30] s (reset after 10 min of healthy uptime)
       if ≥3 crashes within 10 min  → safe_level += 1   (plan: next encoder, then DDA)
           L1: alternate encoder (e.g. h264_mf)   L2: DDA + h264_mf   L3: no capture (tray + library only)
       if L3 reached → stop restarting; tray: "Recording paused after repeated crashes — Send report / Try again"
```

- Persist `safe_level` so it survives a reboot. Clear it after 24 h of healthy recording or "Try again".
- Expose it in `engine.hello` (`safeMode`) and in the capability report.
- **Job Object:** `CreateJobObjectW(None, PCWSTR::null())`, then `SetInformationJobObject(job, JobObjectExtendedLimitInformation, …)` with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION | JOB_OBJECT_LIMIT_BREAKAWAY_OK`. Kill-on-close requires `JOBOBJECT_EXTENDED_LIMIT_INFORMATION`. Die-on-unhandled-exception forces `SEM_NOGPFAULTERRORBOX`, so no WER dialog keeps a dead worker alive [V].
- **Don't count on `RegisterApplicationRestart`.** "The application is not automatically restarted without the user's consent", and only after 60 s of runtime [V].

### 6.4 Who restarts control

- The UI, if open: the pipe vanishes → it starts `kairo-engine.exe` and reconnects.
- Otherwise the plan's `\Kairo\Ensure` task (per-user, Interactive, a time trigger every 5 min plus at logon) runs `kairo-engine.exe --ensure`. That exits at once if the mutex is held.
  - [Unverified] whether a standard (non-admin) user can register a logon-triggered task for themselves without elevation. Test in M0. A repeating time trigger is the fallback.

### 6.5 Folders and updates

- **Tauri NSIS defaults [V `installer.nsi` 2.12.4]:**
  - per-user default install dir is `$LOCALAPPDATA\${PRODUCTNAME}` → `%LOCALAPPDATA%\Kairo`;
  - the optional "delete app data" step removes `$APPDATA\${BUNDLEID}` and `$LOCALAPPDATA\${BUNDLEID}`.
- **So data goes in `%LOCALAPPDATA%\<identifier>\`:** `settings.json`, `library.db`, `logs\`, `crashes\`, `outbox\`, `state\`, `cache\`, `run\`. The same value is hard-coded in the engine, which isn't a Tauri app.
- Library media stays under `Videos\Kairo`.
- **Updates:** the NSIS PREINSTALL hook runs `kairo-engine.exe --shutdown`. That sends `engine.shutdown`, waits ≤10 s, then the hook uses `taskkill /f /t`. POSTINSTALL starts the engine. Never update while a game or save is active (brief).
- **Launching the UI from the worker side:** with a kill-on-close job, give the UI `CREATE_BREAKAWAY_FROM_JOB` (`PROCESS_CREATION_FLAGS(0x0100_0000)`) so it survives. If that fails with `ERROR_ACCESS_DENIED` (an outer job without breakaway, e.g. under Task Scheduler in dev), retry without the flag [Inference]. Launching the UI from control avoids this, since control isn't in the job.

---

## 7. Game detection, icons, SQLite

### 7.1 Runtime pipeline

- **Hook.** On T0: `SetWinEventHook(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND, None, Some(on_event), 0, 0, WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS)`. It returns an `HWINEVENTHOOK`; check `.is_invalid()`.
  - Out-of-context events are "delivered on the same thread that called SetWinEventHook", and "the client thread … must have a message loop" [V].
  - Callback type: `unsafe extern "system" fn(HWINEVENTHOOK, u32 event, HWND, i32 idObject, i32 idChild, u32 idEventThread, u32 dwmsEventTime)` [V].
  - Filter `idObject == OBJID_WINDOW.0` (0) and `try_send` the HWND as `isize`.
  - Also hook `EVENT_SYSTEM_MINIMIZEEND` (23) for restored windows.
- **Detect thread.** Resolve PID → path → elevation (§3.4), then match against the index (§7.3).
  - Add a Toolhelp snapshot every 3 s (`CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)` + `Process32FirstW/NextW`). It gives exe names and parent PIDs without opening handles, which catches game exit and serves the audio report's parent walk.
  - Optionally, `OpenProcess(SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION)` + wait for an instant exit signal. [Unverified] whether anti-cheat drivers strip `SYNCHRONIZE`; keep the poll.
- **Cheap Steam hint.** `HKCU\Software\Valve\Steam\RunningAppID` (DWORD) is read by many open-source tools (GitHub code search: 155 hits, including NiceHashMiner, TcNo-Acc-Switcher and the "clipstems" clipper). Valve doesn't document it [V usage / Unverified semantics].

### 7.2 Installed-games index

| Source | How (verified reference) |
|---|---|
| Steam | `steamlocate` 2.1.1: `SteamDir::locate()?` (HKLM `SOFTWARE\Wow6432Node\Valve\Steam` or `SOFTWARE\Valve\Steam`, value `InstallPath`). Then `steam_dir.libraries()?` → `Result<Library>` → `library.apps()` → `Result<App>`, with `App { app_id, install_dir, name, last_user, state_flags, … }` (`#[non_exhaustive]`), and `library.resolve_app_dir(&app)` [V source] |
| Epic | `%PROGRAMDATA%\Epic\EpicGamesLauncher\Data\Manifests\*.item` (JSON: `DisplayName`, `InstallLocation`, `LaunchExecutable`, `AppName`, `CatalogItemId`, `MainGameAppName`, `ProcessNames`). The launcher sometimes writes empty manifests, so skip invalid JSON. Also `%PROGRAMDATA%\Epic\UnrealEngineLauncher\LauncherInstalled.dat` (`InstallationList[].InstallLocation/AppName`) [V Playnite] |
| GOG | `HKLM\SOFTWARE\WOW6432Node\GOG.com\Games\<id>`, value `path` [V ReShade source]. Also Uninstall keys matching `^(\d+)_is1` with Publisher `GOG.com` and `InstallLocation`, and `goggame-<id>.info` in the install dir [V Playnite] |
| Ubisoft Connect | `HKLM\SOFTWARE\WOW6432Node\Ubisoft\Launcher\Installs\<id>`, value `InstallDir`. Name from `…\Uninstall\Uplay Install <id>`, value `DisplayName` [V BoilR source] |
| Battle.net | Uninstall entries whose `UninstallString` matches `Battle\.net.*--uid=…`, plus `C:\ProgramData\Battle.net\Agent\product.db` as a fallback [V Playnite] |
| Xbox / Game Pass | `PackageManager::new()?.FindPackagesByUserSecurityIdWithPackageTypes(&HSTRING::new(), PackageTypes::Main)?`. windows-rs uses the ABI name; C# calls it `FindPackagesForUser`, `[Overload("FindPackagesByUserSecurityId")]`. An empty SID means the current user; admin is needed only for another user's SID [V MS Learn + windows 0.62.2 source]. For each package: skip `IsFramework()`, read `InstalledPath()`, and parse `MicrosoftGame.config` with roxmltree. `ExecutableList/Executable@Name` is a path relative to the game root, `@Id` (often "Game"), and `ShellVisuals@DefaultDisplayName`. Every Store-distributed GDK game must include it [V GDK docs]. BoilR does the same check in PowerShell [V] |
| EA app | No verified method (report 03). Uninstall entries + curated list in v1 |
| User | "Mark as game", plus a curated exe list |

Windows features: `Management_Deployment`, `ApplicationModel`, `Foundation_Collections`, `Storage`, `Win32_System_Registry`, `Win32_System_Diagnostics_ToolHelp`.

### 7.3 Matching rules

- Normalise paths: `\\?\` stripped, then lower-cased, with `/` → `\`.
- Match by **longest install-dir prefix**, then by exe name for GDK `ExecutableList` and Epic `ProcessNames`.
- Launchers (`steam.exe`, `EpicGamesLauncher.exe`, `GalaxyClient.exe`, `upc.exe`, `Battle.net.exe`, `XboxPcApp.exe`) are never games.
- Re-scan sources at startup, on `games.rescan`, and when `library*.vdf` / `Manifests` change (the `notify` crate, optional).

### 7.4 Exe icon → PNG (detect thread, MTA)

- `SHDefExtractIconW(path, 0, 0, Some(&mut large), None, 256 | (16 << 16))`: the large size goes in the LOWORD, the small in the HIWORD. `S_FALSE` means no icon. The caller must `DestroyIcon` [V].
- Then `IWICImagingFactory::CreateBitmapFromHICON(hicon)` (keeps alpha) → PNG encoder → `cache\icons\<sha1(path)>.png` [V signatures].
- Fallback: `SHCreateItemFromParsingName` + `IShellItemImageFactory::GetImage(SIZE{256,256}, SIIGBF_ICONONLY)` → `HBITMAP` (caller frees it with `DeleteObject`) → `CreateBitmapFromHBITMAP(…, WICBitmapUsePremultipliedAlpha)`. Run it on an STA helper if the MTA call misbehaves [Unverified apartment requirement].
- GDK games: use `ShellVisuals` logos from the package folder instead.

```rust
unsafe fn exe_icon_png(exe: &std::path::Path, out: &std::path::Path) -> windows::core::Result<()> { unsafe {
    let mut large = HICON::default();
    let hr = SHDefExtractIconW(&HSTRING::from(exe), 0, 0, Some(&mut large), None, 256 | (16 << 16));
    if hr != S_OK { return Err(hr.into()); }
    let _g = IconGuard(large); // DestroyIcon on drop
    let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
    let bmp = wic.CreateBitmapFromHICON(large)?;
    let stream = wic.CreateStream()?;
    stream.InitializeFromFilename(&HSTRING::from(out), GENERIC_WRITE.0)?;
    let enc = wic.CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null())?;
    enc.Initialize(&stream, WICBitmapEncoderNoCache)?;
    let (mut frame, mut props) = (None, None);
    enc.CreateNewFrame(&mut frame, &mut props)?;
    let frame = frame.unwrap();
    frame.Initialize(props.as_ref())?;
    frame.WriteSource(&bmp, std::ptr::null())?;
    frame.Commit()?;
    enc.Commit()
}}
```

### 7.5 SQLite ownership (engine ↔ UI)

**Recommendation for v1: control is the only process that opens `library.db`.** It owns the schema, migrations and every write. The UI reads through `library.list` (cursor pages ≤200) and is told about changes by `library.changed`.

Why:
- one schema owner;
- the Mac mock serves the same API;
- no version skew during updates;
- no WAL checkpoint starvation. "If another connection has a read transaction open, then the checkpoint cannot reset the WAL file" [V sqlite.org/wal.html], and a long-lived UI read could cause that.

Later optimisation: a read-only UI connection (`OpenFlags::SQLITE_OPEN_READ_ONLY`). It must check `PRAGMA user_version`, use short transactions, and still needs write access to `-shm` (or existing `-wal`/`-shm` files) [V].

**Other WAL facts [V]:**
- "All processes using a database must be on the same host computer; WAL does not work over a network filesystem". So the DB never goes in the library folder or OneDrive.
- The automatic checkpoint runs at 1000 pages.

**Setup:**
- Confirm WAL actually took effect: `pragma_update_and_check(None, "journal_mode", "WAL", |r| r.get::<_, String>(0))`. On a network or cloud folder it can come back as something else.
- rusqlite 0.40 takes `schema_name: Option<&str>` [V].

```rust
pub fn open(path: &std::path::Path) -> anyhow::Result<rusqlite::Connection> {
    let mut c = rusqlite::Connection::open(path)?;
    let mode: String = c.pragma_update_and_check(None, "journal_mode", "WAL", |r| r.get(0))?;
    anyhow::ensure!(mode.eq_ignore_ascii_case("wal"), "WAL unavailable: {mode}");
    c.pragma_update(None, "synchronous", "NORMAL")?;
    c.pragma_update(None, "foreign_keys", "ON")?;   // bundled default is ON anyway
    c.busy_timeout(std::time::Duration::from_secs(5))?;
    MIGRATIONS.to_latest(&mut c)?;                   // rusqlite_migration 2.6.0, user_version
    Ok(c)
}
const MIGRATION_LIST: &[rusqlite_migration::M<'static>] = &[rusqlite_migration::M::up(include_str!("../migrations/001_init.sql"))];
const MIGRATIONS: rusqlite_migration::Migrations<'static> = rusqlite_migration::Migrations::from_slice(MIGRATION_LIST);
```

The DB is an index. Files plus MP4 tags or sidecars are the truth, so `library.rescan` rebuilds it (brief §7.1).

---

## 8. Dev loop: Mac ↔ Windows 11 PC

### 8.1 One-time PC setup (owner, elevated PowerShell)

```powershell
# OpenSSH Server (Windows 11: optional feature) — firewall rule OpenSSH-Server-In-TCP is created by setup
Add-WindowsCapability -Online -Name OpenSSH.Server~~~~0.0.1.0
Start-Service sshd; Set-Service -Name sshd -StartupType Automatic
Set-NetFirewallRule -Name OpenSSH-Server-In-TCP -Profile Private          # LAN only (plan)
# PowerShell 7 as the SSH shell (&&, ||, UTF-8 by default)
winget install --id Microsoft.PowerShell -e
New-ItemProperty -Path HKLM:\SOFTWARE\OpenSSH -Name DefaultShell -Value "C:\Program Files\PowerShell\7\pwsh.exe" -PropertyType String -Force
# Admin account → keys go in administrators_authorized_keys, ACL exactly Administrators + SYSTEM
Add-Content C:\ProgramData\ssh\administrators_authorized_keys "ssh-ed25519 AAAA… claude@mac"
icacls.exe "C:\ProgramData\ssh\administrators_authorized_keys" /inheritance:r /grant "Administrators:F" /grant "SYSTEM:F"
# sshd_config: PasswordAuthentication no  → Restart-Service sshd
```

- Every command above is from Microsoft's OpenSSH docs (`Add-WindowsCapability … OpenSSH.Server~~~~0.0.1.0`, the `DefaultShell` value under `HKLM\SOFTWARE\OpenSSH`, the `administrators_authorized_keys` ACL) [V]. sshd rejects the keys file if inheritance is left on (Win32-OpenSSH #1942) [V].
- An admin's SSH session is **elevated**. A Win32-OpenSSH maintainer gave this as the reason for the separate keys file ("ssh localhost … would bypass Windows UAC", #1324) [V]. So nothing user-facing may be started directly from SSH.
- Toolchain on the PC: Git for Windows 2.56, VS Build Tools (C++ + Windows SDK), rustup `x86_64-pc-windows-msvc`, Node 22 + pnpm, FFmpeg 9.0.2 LGPL, PresentMon 2.6.0 (brief §3.5). [Unverified] exact winget IDs for VS Build Tools 2022/2026.
- Optional build speed-ups: a Dev Drive, or a Defender exclusion for `C:\kairo\src\target` [Unverified effect on this PC].

### 8.2 Mac setup

```bash
# ~/.ssh/config
Host kairo-pc
  HostName 192.168.1.50
  User owner
  IdentityFile ~/.ssh/kairo_ed25519
  ServerAliveInterval 30
# cargo-xwin (compile checks only)
brew install llvm && export PATH="/opt/homebrew/opt/llvm/bin:$PATH"   # clang-cl, lld-link, llvm-lib, llvm-rc
rustup target add x86_64-pc-windows-msvc && cargo install --locked cargo-xwin
export XWIN_ARCH=x86_64      # default is "x86_64,aarch64": halve the CRT/SDK download
cargo xwin check --target x86_64-pc-windows-msvc -p kairo-engine
cargo xwin clippy --target x86_64-pc-windows-msvc --all-targets
```

- Using cargo-xwin means accepting Microsoft's CRT/SDK licence (README) [V]. cargo-xwin sets `CC_*`, `AR_*` = `llvm-lib`, the linker `lld-link` and `RCFLAGS` [V source].
- `embed-resource` 3.0.12 uses `llvm-rc` for `*-pc-windows-msvc` on non-Windows hosts, overridable via `RC`/`RC_<target>` [V]. `check` runs build scripts, so `llvm-rc` must be on `PATH`.
- `cargo xwin test` needs wine, so tests run on the PC.

### 8.3 Code sync: snapshot push (recommended)

Push a **snapshot commit of the working tree** to a **non-bare** clone on the PC that has `receive.denyCurrentBranch updateInstead`.
- No daemon, exact SHAs in logs, deletions handled, and no commits on the user's branches.
- Git documents this mode for "developing inside a VM to test and fix code on different Operating Systems". It "will refuse the push if the working tree or the index have any difference from the HEAD" [V git docs]. So builds on the PC must not dirty the tree:
  - `cargo build --locked`;
  - `pnpm install --frozen-lockfile`;
  - `target/` and `node_modules/` ignored;
  - `.gitattributes`: `* text=auto eol=lf`, and `git config core.autocrlf false` on the PC.

```bash
# scripts/pc sync  — snapshot tracked + untracked (not ignored) files without touching HEAD, index or branches
idx=$(mktemp); cp .git/index "$idx"
GIT_INDEX_FILE="$idx" git add -A
tree=$(GIT_INDEX_FILE="$idx" git write-tree); rm -f "$idx"
sha=$(git commit-tree "$tree" -p HEAD -m "pc-sync $(date -u +%FT%TZ)")
git push -f pc "$sha:refs/heads/pc-sync" && echo "$sha"
# one-time: git remote add pc kairo-pc:C:/kairo/src
#           git config remote.pc.receivepack "git receive-pack"; git config remote.pc.uploadpack "git upload-pack"
# PC one-time: git init C:\kairo\src; git -C C:\kairo\src config receive.denyCurrentBranch updateInstead; git -C C:\kairo\src checkout -b pc-sync
```

- `git commit-tree` creates a dangling commit object, not a branch commit. The owner should still OK this once in CLAUDE.md ("Claude may push snapshots to the `pc` remote") [Inference on policy].
- [Unverified on this PC]:
  - the remote URL form (`kairo-pc:C:/kairo/src` vs `ssh://kairo-pc/C:/kairo/src`);
  - whether PowerShell (DefaultShell) finds `git-receive-pack`, hence the `git receive-pack` override;
  - the first push into an unborn checked-out branch. Fallback: seed with `git bundle` over `scp`.
- Git over Win32-OpenSSH has a long history of quoting trouble (#752, open since 2017) [V]. Test all of this in M0 (S0).
- Alternatives:
  - Mutagen one-way-replica with `mutagen sync flush` as a barrier. Good, but the last release was 2025-02-24.
  - `tar` + `scp`, which doesn't propagate deletions.

### 8.4 Build on the PC

`ssh kairo-pc 'cd C:\kairo\src; cargo build --locked -p kairo-engine 2>&1 | Select-Object -Last 80'`
- MSVC is found without a Developer Prompt [Inference].
- If the owner might be gaming, the plan's busy check comes first. Run builds at lower priority, e.g. `$p = Start-Process cargo -ArgumentList 'build','--locked' -NoNewWindow -PassThru; $p.PriorityClass='BelowNormal'; $p.WaitForExit()`.

### 8.5 Launch into the owner's session

- `TASK_LOGON_INTERACTIVE_TOKEN`: "User must already be logged on. The task will be run only in an existing interactive session" [V].
- S4U / `/np` runs "in a non-interactive desktop", so it's wrong here [V].
- Default task priority 7 = `BELOW_NORMAL_PRIORITY_CLASS`, and the default `ExecutionTimeLimit` is three days [V].
- `schtasks /create` can't set priority or a working directory, so register with PowerShell:

```powershell
# once, over SSH (no password needed for an Interactive principal) — two tasks: Engine and ad-hoc Run
$prin = New-ScheduledTaskPrincipal -UserId "$env:USERDOMAIN\$env:USERNAME" -LogonType Interactive -RunLevel Limited
$set  = New-ScheduledTaskSettingsSet -Priority 4 -ExecutionTimeLimit ([TimeSpan]::Zero) -MultipleInstances IgnoreNew `
          -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries
$act  = New-ScheduledTaskAction -Execute "C:\kairo\builds\current\kairo-engine.exe" -Argument "--dev" -WorkingDirectory "C:\kairo\builds\current"
Register-ScheduledTask -TaskPath "\Kairo\" -TaskName "Engine" -Action $act -Principal $prin -Settings $set -Force
$act2 = New-ScheduledTaskAction -Execute "C:\kairo\builds\current\kairo-devrun.exe" -Argument "C:\kairo\run\job.json" -WorkingDirectory "C:\kairo\run"
Register-ScheduledTask -TaskPath "\Kairo\" -TaskName "Run" -Action $act2 -Principal $prin -Settings $set -Force
# each time
Start-ScheduledTask -TaskPath "\Kairo\" -TaskName "Engine"     # schtasks /run /tn "Kairo\Engine" also works
Stop-ScheduledTask  -TaskPath "\Kairo\" -TaskName "Engine"     # schtasks /end stops only the task's own process [V]
```

- `-ExecutionTimeLimit ([TimeSpan]::Zero)` means "no limit" [Inference from the task schema].
- The one-liner equivalent (no priority control): `schtasks /create /tn "Kairo\Engine" /tr "C:\kairo\builds\current\kairo-engine.exe --dev" /sc once /st 00:00 /it /rl LIMITED /f`.
  - `/st` is required for ONCE.
  - `/sd` formats vary by locale [V].
- **Nothing that opens a console window may run in the owner's session while they play.** A focus-stealing window can knock a game out of fullscreen [Inference]. Hence:
  - `kairo-engine` is always GUI-subsystem (`--console` calls `AllocConsole` only when asked);
  - ad-hoc jobs go through a tiny GUI-subsystem `kairo-devrun.exe`, which spawns steps with `CREATE_NO_WINDOW` and logs to `C:\kairo\logs`;
  - avoid `pwsh -WindowStyle Hidden`, which still flashes, especially with Windows Terminal as the default console host [Unverified].
- Prerequisites for capture tests:
  - the owner is logged in and **unlocked**, and the displays are on;
  - no RDP (brief).
  - `kairo-cli call engine.status` reports `sessionLocked` and `displayOn` from the engine's own notifications.

### 8.6 Driving and inspecting the engine from SSH

- `kairo-cli.exe call clip.save '{"preSecs":10}'`. It works across sessions because the DACL names the user SID (§4.2) [Inference; test in M0].
- Logs: `Get-ChildItem "$env:LOCALAPPDATA\<identifier>\logs\control*" | Sort-Object LastWriteTime | Select-Object -Last 1 | Get-Content -Tail 200`. Or `scp` the folder to the Mac. SSH runs as the same user, so `%LOCALAPPDATA%` is the same.
- Kill, gracefully first:
  1. `kairo-cli call engine.shutdown`;
  2. `Stop-ScheduledTask …`;
  3. `Stop-Process -Name kairo-engine -Force` / `taskkill /im kairo-engine.exe /f /t`.
- **Don't rely on SSH session lifetime for child processes.** Win32-OpenSSH puts session processes in a job with `JOB_OBJECT_LIMIT_BREAKAWAY_OK`; children may survive a dropped connection, and `ssh -t` kills them (#1751, open) [V]. Anything long-running goes through a task.
- **Crash dumps:** `scp kairo-pc:AppData/Local/<identifier>/crashes/*.dmp`. Symbolicate on the Mac with `dump_syms` 2.3.9 (PDB → Breakpad `.sym`) + `minidump-stackwalk` 0.27.0 (`--json`). It reads Breakpad symbol files, not PDBs directly [V README]. Or run `cdb` on the PC [Unverified install path].

### 8.7 Elevated-game simulation for S4 without a UAC click

Register a second test task with `-RunLevel Highest` that launches a test window or game. Because an admin registered it, it starts elevated in the owner's session without a prompt [Inference; standard Task Scheduler behaviour]. Combined with `kairo-cli call engine.status` (foreground elevation), the hotkey and mouse matrix can be filled in unattended.

### 8.8 PresentMon over SSH

- "PresentMon needs to be run by a user who is a member of the 'Performance Log Users' user group". Otherwise you get "failed to start trace session (access denied)" (the README sentence implies "admin *or* Performance Log Users"). Without admin, other users' and short-lived processes show as `<unknown>` [V README].
- ETW sessions are system-wide, so PresentMon doesn't need the interactive session [Inference]. The admin's SSH token is elevated (§8.1).
- For a non-admin user: `Add-LocalGroupMember -Group "Performance Log Users" -Member $env:USERNAME`, then reconnect (group membership is read at logon).

```powershell
& C:\kairo\tools\PresentMon-2.6.0-x64.exe --process_name cs2.exe `
  --output_file "C:\kairo\bench\cs2-$(Get-Date -f yyyyMMdd-HHmmss).csv" `
  --timed 60 --terminate_after_timed --stop_existing_session --no_console_stats --track_gpu_video
```

`--stop_existing_session` recovers from a stuck "PresentMon" ETW session after a dropped SSH connection. `--track_gpu_video` adds the video-encode column [V console README].

---

## 9. Recommended crates and features

```toml
# Cargo.toml (workspace) — versions verified 2026-10-21
[workspace]
members = ["crates/kairo-rpc", "crates/kairo-engine", "crates/kairo-mock-engine", "crates/kairo-cli", "crates/kairo-devrun", "ui/src-tauri"]
resolver = "3"
[workspace.package]
edition = "2024"
rust-version = "1.95"          # rusqlite_migration 2.6; toolchain is 1.99.1

[workspace.dependencies]
serde = { version = "1.0.229", features = ["derive"] }
serde_json = { version = "1.0.154", features = ["raw_value"] }
ts-rs = { version = "12.0.1", features = ["serde-json-impl"] }
schemars = "1.2.3"
tokio = { version = "1.54.0", features = ["rt-multi-thread", "macros", "net", "io-util", "sync", "time", "process"] }
tokio-util = { version = "0.7.20", features = ["codec"] }
futures = "0.3.34"
bytes = "1.12.1"
crossbeam-channel = "0.5.17"
thiserror = "2.0.21"
anyhow = "1.0.104"
tracing = "0.1.44"
tracing-subscriber = { version = "0.3.23", features = ["env-filter", "fmt", "json", "registry"] }
tracing-appender = "0.2.5"
file-rotate = "0.8.0"
sentry = { version = "0.49.3", default-features = false, features = ["backtrace", "contexts", "panic", "debug-images", "reqwest", "native-tls"] }
crash-handler = "0.8.1"
minidumper = "0.11.0"
minidump-writer = "0.13.0"
rusqlite = { version = "0.40.2", features = ["bundled"] }
rusqlite_migration = "2.6.0"
steamlocate = "2.1.1"
roxmltree = "0.21.1"
ulid = "3.0.0"
semver = "1.0.28"
dirs = "7.0.0"
windows-registry = "0.6.1"     # NOT 0.100 (needs windows-core 0.100 / Rust 1.95)

# crates/kairo-engine/Cargo.toml (Windows-only deps)
[target.'cfg(windows)'.dependencies]
tray-icon = { version = "0.26.1", default-features = false }
muda = { version = "0.21.2", default-features = false }
windows = { version = "=0.62.2", features = [
  "Win32_Foundation", "Win32_Graphics_Gdi", "Win32_Graphics_Imaging", "Win32_Security", "Win32_Security_Authorization",
  "Win32_Storage_FileSystem", "Win32_System_Com", "Win32_System_Diagnostics_Debug", "Win32_System_Diagnostics_ToolHelp",
  "Win32_System_JobObjects", "Win32_System_Kernel", "Win32_System_LibraryLoader", "Win32_System_Memory",
  "Win32_System_Performance", "Win32_System_Pipes", "Win32_System_Power", "Win32_System_ProcessStatus",
  "Win32_System_Registry", "Win32_System_RemoteDesktop", "Win32_System_Shutdown", "Win32_System_SystemServices",
  "Win32_System_Threading", "Win32_Media_Audio", "Win32_UI_Accessibility", "Win32_UI_Input_KeyboardAndMouse",
  "Win32_UI_Shell", "Win32_UI_WindowsAndMessaging",
  "ApplicationModel", "Foundation_Collections", "Management_Deployment", "Storage" ] }
[build-dependencies]
embed-resource = "3.0.12"      # manifest (PerMonitorV2, asInvoker, supportedOS) + icons; guard with CARGO_CFG_TARGET_OS
```

- All 31 `windows` feature names above exist in 0.62.2's `Cargo.toml` [V].
- Some APIs are feature-gated on a second feature [V]: `RegisterClassExW` (`Win32_Graphics_Gdi`), `CreateMutexW` / `OpenProcessToken` / `CreateWaitableTimerExW` (`Win32_Security`), `MiniDumpWriteDump` (`Win32_Storage_FileSystem` + `Win32_System_Kernel` + `Win32_System_Memory`), `SHDefExtractIconW` (`Win32_UI_WindowsAndMessaging`), and `RegisterSuspendResumeNotification` / `RegisterPowerSettingNotification` (also `Win32_UI_WindowsAndMessaging`).
- Release profile: keep `panic = "unwind"` for the workspace, since the Tauri UI shares it. Rely on the panic hook's dump-then-exit for the engine (§5.3).

---

## 10. Gotchas (checklist for the coding agent)

1. `PostThreadMessageW` loses messages during the tray menu's modal loop. Post to the shell window.
2. `HWND_MESSAGE` windows miss broadcasts (TaskbarCreated, `WM_ENDSESSION`, power, display). Use a hidden top-level tool window.
3. `HWND` / `HANDLE` are `!Send` in windows 0.62.2. Use an `isize` newtype.
4. `RegisterHotKey` must be called on the thread that created the window, and the same `(hWnd, id)` stacks instead of replacing. Unregister first.
5. F12 is reserved. Win+ combinations belong to the OS. App hotkey ids are 0–0xBFFF.
6. `GetAsyncKeyState` returns 0 under UIPI (elevated foreground) and on another desktop. Use the high bit only.
7. A panic in a `WNDPROC` / `WINEVENTPROC`, or any `abort()`, is a `__fastfail`: no handler runs and no dump is written. Wrap callbacks in `catch_unwind`; the panic hook dumps first.
8. FFmpeg MF encoders fail on STA threads. Encoders never run on T0.
9. The default pipe DACL gives Everyone and anonymous read. Always pass the SDDL.
10. Don't deny the NETWORK SID on the pipe, or `kairo-cli` over SSH may break [Unverified].
11. interprocess's client lacks SQOS flags. Use tokio `ClientOptions` (`SECURITY_IDENTIFICATION`) for clients.
12. On macOS, interprocess `GenericNamespaced` maps to `/tmp/<name>` (deprecated mapping). Use an explicit path in `$TMPDIR`.
13. ts-rs maps `u64`/`i64` to `bigint` by default. Set `TS_RS_LARGE_INT=number`.
14. `sentry-contexts` sends the hostname as `server_name` unless you set it. The default `sentry` features include `release-health` (sessions) and `logs`/`metrics`. Turn them off.
15. `sentry-rust-minidump` is deprecated, and both it and `sentry-minidump` upload at crash time. Consent needs an outbox.
16. Tauri NSIS per-user installs into `%LOCALAPPDATA%\<productName>`. Don't put data there.
17. `auto-launch` 0.6.0 writes the Run path unquoted and overrides the user's StartupApproved choice.
18. `tracing-appender` has no size rotation. File dates are UTC. Never two processes on one file.
19. Task Scheduler's default task priority is BELOW_NORMAL and the default time limit is 3 days. Set `-Priority 4` and `-ExecutionTimeLimit 0`.
20. An updateInstead push fails if the PC's working tree is dirty. Use `--locked` / `--frozen-lockfile` and LF endings.
21. Processes started from SSH are elevated for admins and live in session 0. They can't capture or receive hotkeys, and their lifetime after a disconnect is unpredictable (#1751).
22. `WTSRegisterSessionNotification` can fail right after logon (`RPC_S_INVALID_BINDING`). Retry.
23. `CreateMutexW` + `GetLastError()`: call nothing in between. windows-rs adds no call on success [V].
24. With a kill-on-close Job Object, a UI launched from inside the job dies with it unless `BREAKAWAY_OK` + `CREATE_BREAKAWAY_FROM_JOB`. Launch the UI from control.
25. `PackageManager` in windows-rs is `FindPackagesByUserSecurityId*`, not `FindPackagesForUser`. Pass `&HSTRING::new()` for the current user.
26. `keyvalues-parser` 0.2.4's entry point is `keyvalues_parser::parse(text)` → `PartialVdf`, and it handles text VDF only. Binary `shortcuts.vdf` goes through `steamlocate`'s `shortcuts()`.
27. SQLite WAL is local-only. A long UI read transaction blocks checkpoints. Verify `journal_mode` actually returned `wal`.
28. `POWERBROADCAST_SETTING` (in `lParam`) is valid only during the message. Copy what you need.
29. tray-icon's `menu_on_left_click` defaults to true.
30. Over SSH, PresentMon after a dropped connection needs `--stop_existing_session`.

---

## 11. Tests

| Area | Test | Where / how |
|---|---|---|
| Protocol | serde round-trip for every request, response and notification; unknown-field tolerance; `#[serde(other)]` enum fallback; error `data.kind` | `cargo test -p kairo-rpc` (Mac + CI) |
| Codegen drift | ts-rs export + `methods.ts` + schemars, then `git diff --exit-code ui/src/gen` | CI |
| Framing | Frame >1 MiB rejected; split / partial frames; garbage → `-32700` and the session survives | `tokio::io::duplex` (Mac) |
| Dispatcher / back-pressure | 10k `buffer.status` against a slow reader: responses still delivered, telemetry coalesced | duplex + paused tokio clock |
| Pipe DACL | `GetSecurityInfo` → `ConvertSecurityDescriptorToStringSecurityDescriptorW` equals the expected SDDL | PC, `cargo test --features win-it` |
| Squatting | Helper pre-creates `\\.\pipe\kairo-<SID>` → control reports "name taken", exits non-zero | PC |
| Remote reject | Opening `\\127.0.0.1\pipe\kairo-<SID>` (SMB loopback) fails [Unverified that loopback counts as remote] | PC |
| Cross-session CLI | Over SSH, `kairo-cli call engine.status` reaches the task-launched engine | PC (M0) |
| Single instance | Second launch exits 0 and forwards `ui.open` | PC |
| Hotkeys | Helper registers Alt+F8 first → `hotkeys.set` returns `InUse`; re-binding restores the old key on failure; F12 → `Reserved` | PC |
| TaskbarCreated | `Stop-Process -Name explorer -Force; Start-Process explorer` → icon returns, GUID preserved | PC, manual / scripted |
| Session / power | Lock → `engine.state` paused; sleep/resume → media restarted; `WM_ENDSESSION` while saving → shutdown block reason visible | PC, manual |
| Elevation | Elevated test window via the `-RunLevel Highest` task (§8.7) → `game.changed` shows elevated; mouse trigger disabled with a warning | PC |
| Crash: native | `kairo-engine --media --debug-crash=av\|stackoverflow\|purecall` → dump in `crashes\`, exit classified, restart with backoff | PC |
| Crash: panic / abort | `--debug-crash=panic` (also inside `WNDPROC`) → dump exists before exit; `--debug-crash=abort` → exit `0xC0000409` recorded with "no dump" | PC |
| Hang | `--debug-crash=hang` → heartbeat timeout → external dump + restart | PC |
| Safe mode | 3 crashes in 10 min → L1 → L2 → L3 tray state; "Try again" clears | PC (fake clock in unit tests on the Mac) |
| Restart policy | Pure state machine with injected clock | Mac |
| Redaction | Paths, username, hostname and SIDs scrubbed in log lines **and** Sentry events (`before_send` unit tests) | Mac |
| Outbox | `OutboxTransport` writes envelopes; `Envelope::from_path` round-trip; send-then-delete | Mac (+ a local Sentry test project) |
| Autostart | Run value quoted; StartupApproved "disabled" detected and left alone; uninstall removes the value | PC |
| Detection | Fixtures: Steam `libraryfolders.vdf` / `appmanifest_*.acf`, Epic `.item` (including an empty one), GOG / Ubisoft registry exported to `.reg` and read through a trait, `MicrosoftGame.config` samples | Mac (parsers), PC (live scan) |
| Icons | `notepad.exe` → 256×256 PNG with non-trivial alpha | PC |
| SQLite | Migrations up from empty and from each previous version; WAL asserted; `library.list` pagination stable under inserts | Mac + PC |
| Soak | 8 h control + media: handles, private bytes and thread count flat (`Get-Process kairo-engine \| Select Handles,PM,Threads` every minute) | PC (plan S5) |

---

## 12. Unverified or open items

1. Pipe access from the SSH session: whether OpenSSH tokens carry `NT AUTHORITY\NETWORK`, and whether `kairo-cli` over SSH reaches the session-1 pipe with a user-SID DACL. Expected yes.
2. Git over Win32-OpenSSH with PowerShell as `DefaultShell`: URL form, `receive-pack` lookup, first push into an unborn checked-out branch.
3. Whether a standard user can register the `\Kairo\Ensure` logon/time-trigger task without elevation.
4. `CREATE_BREAKAWAY_FROM_JOB` behaviour under Task Scheduler's own job.
5. Whether `SHDefExtractIconW` and WIC behave on an MTA thread for all exes. STA fallback ready.
6. Anti-cheat effects on `PROCESS_QUERY_LIMITED_INFORMATION` / `SYNCHRONIZE` / `OpenProcessToken` against protected games (EAC, BattlEye, Vanguard). Treat failures as "Unknown".
7. Steam `RunningAppID` semantics (undocumented; widely used).
8. MMCSS task choice for capture/encode threads ("Capture" vs "Games"), and whether "above normal" for the video thread helps or hurts game frame times. Measure with PresentMon.
9. Exact Sentry project setting names for IP scrubbing.
10. Whether `-WindowStyle Hidden` still flashes a Windows Terminal window when launched by a task (hence `kairo-devrun`).
11. VS Build Tools winget IDs and the cdb path on the PC.

---

## Sources

All accessed 2026-10-21.

**Crates (crates.io API; sources from static.crates.io, SHA-256 checked against index.crates.io)**
- https://crates.io/crates/tray-icon (0.26.1: `src/lib.rs`, `src/platform_impl/windows/mod.rs`)
- https://crates.io/crates/muda (0.21.2)
- https://crates.io/crates/global-hotkey (0.9.0: `src/platform_impl/windows/mod.rs`)
- https://crates.io/crates/winit (0.30.13), https://crates.io/crates/tao (0.37.1) (`platform/windows.rs` `with_msg_hook`)
- https://crates.io/crates/windows (0.62.2: `src/Windows/Win32/**`, `src/extensions/Win32/Foundation/WIN32_ERROR.rs`, `Cargo.toml` features), https://crates.io/crates/windows-core (0.62.2 `param.rs`), https://crates.io/crates/windows-result (0.4.1), https://crates.io/crates/windows-strings (0.5.1), https://crates.io/crates/windows-registry (0.6.1 / 0.100.0), https://crates.io/crates/windows-sys
- https://crates.io/crates/tokio (1.54.0: `src/net/windows/named_pipe.rs`, `src/io/join.rs`, `src/process/mod.rs`), https://crates.io/crates/tokio-util (0.7.20: `codec/length_delimited.rs`, `codec/lines_codec.rs`)
- https://crates.io/crates/interprocess (2.4.6: `os/windows/named_pipe/listener/create_instance.rs`, `named_pipe/c_wrappers.rs`, `os/windows/local_socket.rs`, `local_socket/name/type.rs`, `os/unix/local_socket/name_type.rs`)
- https://crates.io/crates/jsonrpsee (0.26.1; `jsonrpsee-core` `server/rpc_module.rs`)
- https://crates.io/crates/ts-rs (12.0.1: `src/lib.rs`), https://crates.io/crates/schemars (1.2.3), https://crates.io/crates/specta, https://crates.io/crates/tauri-specta
- https://crates.io/crates/tracing-appender (0.2.5: `rolling.rs`, `rolling/builder.rs`, `non_blocking.rs`), https://crates.io/crates/tracing-subscriber (0.3.23: `fmt/fmt_layer.rs`), https://crates.io/crates/file-rotate (0.8.0)
- https://crates.io/crates/sentry, https://crates.io/crates/sentry-core, https://crates.io/crates/sentry-types, https://crates.io/crates/sentry-contexts (0.49.3: `Cargo.toml` features, `clientoptions.rs`, `transport/mod.rs`, `client/mod.rs`, `protocol/envelope.rs`, `integration.rs`, `utils.rs`)
- https://crates.io/crates/sentry-minidump (0.49.3: README, `src/lib.rs`, `examples/app.rs`), https://crates.io/crates/sentry-rust-minidump (0.18.1: README deprecation notice)
- https://crates.io/crates/crash-handler (0.8.1), https://crates.io/crates/minidumper (0.11.0: `lib.rs`, `ipc/server.rs`, `ipc/client.rs`, `ipc/windows.rs`, `examples/diskwrite.rs`), https://crates.io/crates/minidump-writer (0.13.0 `windows/minidump_writer.rs`), https://crates.io/crates/minidumper-child, https://crates.io/crates/minidump-stackwalk (0.27.0 README), https://crates.io/crates/dump_syms
- https://crates.io/crates/auto-launch (0.6.0 `src/windows.rs`)
- https://crates.io/crates/steamlocate (2.1.1: `lib.rs`, `library.rs`, `app.rs`, `locate/windows.rs`), https://crates.io/crates/keyvalues-parser (0.2.4)
- https://crates.io/crates/rusqlite (0.40.2 `pragma.rs`), https://crates.io/crates/libsqlite3-sys (0.38.2 `sqlite3/sqlite3.h`, `build.rs`), https://crates.io/crates/rusqlite_migration (2.6.0)
- https://crates.io/crates/windows-capture (2.0.1 `capture.rs`, `graphics_capture_api.rs`), https://crates.io/crates/wasapi (0.26.0 `api.rs`)
- https://crates.io/crates/embed-resource (3.0.12), https://crates.io/crates/cargo-xwin (0.23.1 README, `compiler/*.rs`)
- https://crates.io/crates/tauri-plugin-single-instance (2.5.4 `platform_impl/windows.rs`), https://crates.io/crates/tauri-plugin-autostart (2.7.0)
- Rust stable channel manifest: https://static.rust-lang.org/dist/channel-rust-stable.toml (1.99.1)

**Microsoft Learn (source markdown from MicrosoftDocs repos)**
- RegisterHotKey: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-registerhotkey
- SetWinEventHook: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwineventhook ; WINEVENTPROC: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nc-winuser-wineventproc
- PostThreadMessageW: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-postthreadmessagew
- GetAsyncKeyState: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getasynckeystate
- Window features (message-only windows): https://learn.microsoft.com/en-us/windows/win32/winmsg/window-features
- Shell_NotifyIcon (TaskbarCreated): https://learn.microsoft.com/en-us/windows/win32/api/shellapi/nf-shellapi-shell_notifyicona
- CreateNamedPipeW: https://learn.microsoft.com/en-us/windows/win32/api/namedpipeapi/nf-namedpipeapi-createnamedpipew ; Named pipe security: https://learn.microsoft.com/en-us/windows/win32/ipc/named-pipe-security-and-access-rights
- OpenProcessToken: https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-openprocesstoken ; OpenProcess: https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-openprocess ; Process security and access rights: https://learn.microsoft.com/en-us/windows/win32/procthread/process-security-and-access-rights ; Mandatory Integrity Control: https://learn.microsoft.com/en-us/windows/win32/secauthz/mandatory-integrity-control
- CreateMutexW: https://learn.microsoft.com/en-us/windows/win32/api/synchapi/nf-synchapi-createmutexw ; Kernel object namespaces: https://learn.microsoft.com/en-us/windows/win32/termserv/kernel-object-namespaces
- CreateWaitableTimerExW: https://learn.microsoft.com/en-us/windows/win32/api/synchapi/nf-synchapi-createwaitabletimerexw ; timeBeginPeriod: https://learn.microsoft.com/en-us/windows/win32/api/timeapi/nf-timeapi-timebeginperiod ; SetProcessInformation: https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-setprocessinformation
- CoInitializeEx: https://learn.microsoft.com/en-us/windows/win32/api/combaseapi/nf-combaseapi-coinitializeex
- RegisterApplicationRestart: https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-registerapplicationrestart ; ShutdownBlockReasonCreate: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-shutdownblockreasoncreate ; WTSRegisterSessionNotification: https://learn.microsoft.com/en-us/windows/win32/api/wtsapi32/nf-wtsapi32-wtsregistersessionnotification
- JOBOBJECT_BASIC_LIMIT_INFORMATION: https://learn.microsoft.com/en-us/windows/win32/api/winnt/ns-winnt-jobobject_basic_limit_information ; __fastfail: https://learn.microsoft.com/en-us/cpp/intrinsics/fastfail
- SHDefExtractIconW: https://learn.microsoft.com/en-us/windows/win32/api/shlobj_core/nf-shlobj_core-shdefextracticonw ; IWICImagingFactory::CreateBitmapFromHICON: https://learn.microsoft.com/en-us/windows/win32/api/wincodec/nf-wincodec-iwicimagingfactory-createbitmapfromhicon ; IShellItemImageFactory::GetImage: https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-ishellitemimagefactory-getimage
- PackageManager.FindPackagesForUser: https://learn.microsoft.com/en-us/uwp/api/windows.management.deployment.packagemanager.findpackagesforuser
- MicrosoftGame.config overview: https://learn.microsoft.com/en-us/gaming/gdk/docs/features/common/game-config/microsoftgameconfig-overview ; Executable element: https://learn.microsoft.com/en-us/gaming/gdk/docs/reference/system/microsoftgameconfig/elements/microsoftgameconfig-element-executable
- OpenSSH install: https://learn.microsoft.com/en-us/windows-server/administration/openssh/openssh_install_firstuse ; server configuration (DefaultShell, administrators_authorized_keys): https://learn.microsoft.com/en-us/windows-server/administration/openssh/openssh-server-configuration ; key management: https://learn.microsoft.com/en-us/windows-server/administration/openssh/openssh_keymanagement
- schtasks create / run / end: https://learn.microsoft.com/en-us/windows-server/administration/windows-commands/schtasks-create , …/schtasks-run , …/schtasks-end
- TASK_LOGON_TYPE: https://learn.microsoft.com/en-us/windows/win32/api/taskschd/ne-taskschd-task_logon_type ; TaskSettings.Priority: https://learn.microsoft.com/en-us/windows/win32/taskschd/tasksettings-priority
- New-ScheduledTaskPrincipal / New-ScheduledTaskSettingsSet / New-ScheduledTaskAction / Register-ScheduledTask: https://learn.microsoft.com/en-us/powershell/module/scheduledtasks/

**Other primary sources**
- FFmpeg 9.0 `libavcodec/mf_utils.c` (`init_com_mf`): https://github.com/FFmpeg/FFmpeg/blob/release/9.0/libavcodec/mf_utils.c
- Rust 1.99.1 `library/std/src/sys/pal/windows/mod.rs` (`abort_internal`): https://github.com/rust-lang/rust/blob/1.99.1/library/std/src/sys/pal/windows/mod.rs
- Tauri 2.12.4 NSIS template: https://github.com/tauri-apps/tauri/blob/tauri-v2.12.4/crates/tauri-bundler/src/bundle/windows/nsis/installer.nsi
- Win32-OpenSSH issues #1324 (admin keys / UAC), #1751 (child processes on disconnect), #752 (git server quoting), #1942 (keys file ACL): https://github.com/PowerShell/Win32-OpenSSH/issues ; wiki "Security protection of various files in Win32 OpenSSH"
- PresentMon README + README-ConsoleApplication (main branch), releases (v2.6.0): https://github.com/GameTechDev/PresentMon
- Sentry data storage location: https://docs.sentry.io/organization/data-storage-location/
- GitHub "Creating an issue from a URL query": https://docs.github.com/en/issues/tracking-your-work-with-issues/using-issues/creating-an-issue#creating-an-issue-from-a-url-query
- Git `receive.denyCurrentBranch` (`Documentation/config/receive.adoc`): https://git-scm.com/docs/git-config#Documentation/git-config.txt-receivedenyCurrentBranch
- SQLite WAL: https://www.sqlite.org/wal.html
- JSON-RPC 2.0 specification: https://www.jsonrpc.org/specification
- Playnite extensions (Epic `EpicLauncher.cs`, `Models/LauncherInstalled.cs`; GOG `GogLibrary.cs`, `Gog.cs`; Battle.net `BattleNetLibrary.cs`; Xbox `XboxLibrary.cs`): https://github.com/JosefNemec/PlayniteExtensions
- BoilR (Xbox `xbox_games.ps1`, `platform.rs`; Ubisoft `platform.rs`): https://github.com/PhilipK/BoilR
- ReShade setup `SelectAppPage.xaml.cs` (GOG registry `path`): https://github.com/crosire/reshade
- GitHub code search for `RunningAppID` + `Valve\Steam` (155 results)
- Mutagen releases: https://github.com/mutagen-io/mutagen/releases ; PowerShell releases: https://github.com/PowerShell/PowerShell/releases ; Git for Windows releases: https://github.com/git-for-windows/git/releases ; sentry-cli releases: https://github.com/getsentry/sentry-cli/releases
