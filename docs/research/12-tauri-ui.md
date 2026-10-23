# 12 · Tauri 2 + React UI: library, player, trimmer, exports

*Code-level pre-implementation research for the Kairo implementation cookbook. Prepared 2026-10-21. Nothing is built yet.*

**How this was checked.** Versions were read from the crates.io and npm registries on 2026-10-21. API claims were checked against the source of the exact released versions: tag `tauri-v2.12.4` (tauri, tauri-runtime-wry, the JS API), `wry-v0.57.0`, the `windows` 0.62.2 crate source, interprocess 2.4.6, ts-rs 12.0.1, arboard 3.7.0, wavesurfer.js 8.0.3, @radix-ui/react-slider 1.6.1, the plugins-workspace `v2` branch and drag-rs `v2`. Microsoft Learn and Chromium source were used for WebView2 behaviour.

**Updated 2026-10-23.** After the Tauri 2.12.5 review (cookbook P20, §12 item 22), the pins below are `tauri` =2.12.5, `tauri-build` =2.7.4 and `@tauri-apps/cli` 2.12.4. The files cited under Sources are unchanged at tag `tauri-v2.12.5`, apart from version numbers, the changelogs and a capabilities-folder watch in `tauri-build/src/acl.rs`, so the notes still hold. The updater and process plugins were taken out of the snippets on the same day, because updates live in control (PLAN.md §2.1), and updater artifacts moved to a release-only overlay (§1.5).

**Tags.** Untagged means verified in a primary source or in source code today. **[Unverified]**: I could not confirm it. **[Estimate]**: arithmetic or judgement; measure it. **[Decision]**: a recommendation the plan should adopt or reject explicitly.

---

## 0. What the coding agent must know first

1. **Versions (2026-10-21; Tauri moved to 2.12.5 on 2026-10-23).** Use Tauri **=2.12.5** with `tauri-build` **=2.7.4** (crates), `@tauri-apps/cli` **2.12.4**, `@tauri-apps/api` **2.12.4** (MSRV 1.90, edition 2024), React **19.3.1**, Vite **8.3.5** with `@vitejs/plugin-react` **6.1.2**, and Node **24.22.1 LTS**.
   - Use TypeScript **6.0.3**, not 7.0.2. `typescript-eslint` 8.72.0 peers `typescript >=4.8.4 <6.1.0`, and TS 7.0 shipped without a stable programmatic API.
   - Tauri **3.0.0-alpha.5** is on the npm `next` tag. Never install `@next` versions.
2. **Window lifecycle works as Kairo needs by default.** This was verified in source:
   - When the last window is destroyed, Tauri raises `RunEvent::ExitRequested { code: None }` and leaves the event loop unless you call `api.prevent_exit()`.
   - Dropping a wry WebView calls `ICoreWebView2Controller::Close()`. Microsoft documents `Close` as the way "to safely shut down associated browser and renderer processes".
   - So: one window, no hidden windows, no tray in the UI, never call `prevent_exit`. Then measure on the PC with the script in §1.8.
3. **Playback [Decision]: serve clips through Kairo's own async URI scheme, `kairo-media`.** It is addressed by clip ID and answers HTTP `206` with responses of at most 2 MiB. Don't use the built-in asset protocol or a loopback HTTP server. Why:
   - tauri#4133 is **still open**.
   - In 2.12.4 the asset protocol is async, and multi-range responses were fixed in 2.12.0. But each range is capped at 1000 KiB, and a GET without `Range` reads the **whole file** into RAM.
   - wry copies every response body into an in-memory `IStream`.
   - Chromium's media loader always sends `Range: bytes=N-`.
   - A Microsoft WebView2 engineer confirmed that partial responses work only on `http(s)` URLs. Tauri maps custom schemes to `http://<scheme>.localhost` on Windows, which is why this works.
4. **Engine link.** The Tauri Rust shell owns one `EngineClient`:
   - `interprocess` 2.4.6 local socket: a named pipe on Windows, a Unix socket on the Mac.
   - JSON-RPC 2.0 framed with `tokio_util::codec::LengthDelimitedCodec`, a 4-byte big-endian length prefix. That is the same framing as StreamJsonRpc's `LengthHeaderMessageHandler`, so the C# fallback stays compatible.
   - React calls one allow-listed passthrough command, `engine_call(method, params)`.
   - Engine notifications are re-emitted as **one** Tauri event, `engine:notification`. Tauri event names may contain only alphanumerics and `- / : _`, so `clip.saved` cannot be an event name.
5. **Type sharing [Decision]: ts-rs 12.0.1**, which is stable, plus a tiny method-table macro. Not tauri-specta, which is still `2.0.0-rc.25`. Set `TS_RS_LARGE_INT=number`; the default `bigint` doesn't match what `JSON.parse` returns.
6. **Plugins.**
   - `tauri-plugin-single-instance` 2.5.4: Rust only, no npm package; register it first.
   - `window-state` 2.5.0: exclude `VISIBLE`.
   - `opener` 2.7.2: call `reveal_item_in_dir` from Rust.
   - `dialog` 2.8.3.
   - No `updater` or `process` plugin: updates live in control (PLAN.md §2.1; `deny.toml` bans `tauri-plugin-updater`), and `exit()` is in `@tauri-apps/api/app` since 2.12.0 (permission `core:app:allow-exit`).
   - `tauri-plugin-drag` 2.1.1 (crate) with `@crabnebula/tauri-plugin-drag` 2.1.0 (npm). It works, but its Windows backend uses `windows` 0.52, and it has open crash and hang issues (#72, #76, #94). Guard the paths you pass and keep the ~15-line fallback in §6.2.
   - `clipboard-manager` 2.5.0 can't put files on the clipboard; use the `CF_HDROP` code in §6.1.
7. **CSP.**
   - `connect-src ipc: http://ipc.localhost` is required for IPC.
   - Media needs both `kairo-media:` (macOS) and `http://kairo-media.localhost` (Windows).
   - Tauri adds nonces to `style-src`, which makes browsers ignore `'unsafe-inline'`. wavesurfer.js (Shadow DOM `<style>`) and sonner inject `<style>` at runtime, so set `dangerousDisableAssetCspModification: ["style-src"]`.
8. **UI stack.**
   - Data, routing and state: TanStack Query 5.104.5, TanStack Virtual 3.14.21, TanStack Router 1.170.45 (hash history), Zustand 5.0.15.
   - Styling: Tailwind CSS 4.3.3, and shadcn CLI 4.22.1 initialised with **`-b radix`**. Base UI became shadcn's default in July 2026.
   - lucide-react 1.61.0, motion 14.5.0, and Inter Variable (OFL) bundled locally.
   - Editor: wavesurfer.js **8.0.3** (BSD-3-Clause, v8 since 2026-09-23) fed precomputed peaks; react-image-crop 11.1.2 (ISC) for the 9:16 rectangle; Radix Slider 1.6.1, a range slider with the new `preserveThumbOrder` prop.
9. **Tests.**
   - Vitest **5.0.5**: it clears mocks before each test by default and fails un-awaited assertions.
   - Testing Library 16.3.3, jsdom 30.1.3, and `@tauri-apps/api/mocks` (`mockIPC(cb, { shouldMockEvents: true })`).
   - E2E: WebdriverIO **9.32.0** + `@wdio/tauri-service` 1.5.0 + `tauri-plugin-wdio-webdriver` 1.5.0, an embedded WebDriver server that works on macOS **and** Windows. Compile it only behind an `e2e` feature, never ship it.
   - `tauri-driver` 2.1.0 + msedgedriver also works on Windows, as does Playwright over CDP.

---

## 1. Project setup

### 1.1 Versions (read 2026-10-21)

| Component | Version (date) | Notes |
|---|---|---|
| `tauri` | 2.12.5 (2026-10-21), MSRV 1.90 | 2.12.5 changes only a Linux doc comment in `tauri` itself (cookbook §12 item 22). 2.12.0 (2026-09-26): async `asset://`, multipart fix, `Webview::convert_file_src`, `appDirectoriesOverride`, `exit()` in `@tauri-apps/api/app`, MSRV 1.90, edition 2024. It also **updated `windows` to 0.62 and `webview2-com` to 0.39** (drops Windows 7). |
| `tauri-build` / `tauri-cli` | 2.7.4 / 2.12.4 | npm `@tauri-apps/cli` 2.12.4 (bundler 2.10.4) |
| `@tauri-apps/api` | 2.12.4 | `next` tag = 3.0.0-alpha.3. Don't use it. |
| wry / tao / webview2-com | 0.57.0 / 0.37.1 / 0.39.1 | Pulled in by tauri-runtime-wry 2.12.3 |
| `create-tauri-app` | 4.7.5 | `react-ts` template pins `typescript ~6.0.3`, `vite ^8.0.16`, `@vitejs/plugin-react ^6.0.2`, `react ^19.1.0` |
| React / react-dom | 19.3.1 | |
| Vite / plugin-react | 8.3.5 / 6.1.2 | Vite 8 needs Node `^20.19 \|\| >=22.12` |
| TypeScript | **6.0.3** | 7.0.2 is "latest" but has no stable API yet (TS blog 2026-07-08). typescript-eslint 8.72.0 peers `<6.1.0`. |
| Node / pnpm | 24.22.1 LTS "Krypton" / 12.14.0 | jsdom 30 needs Node `^22.22.2 \|\| ^24.15` |
| `windows` crate | **0.62.2** | Same version Tauri 2.12 uses, so there is one copy in the binary (except drag-rs; see §6.2) |

### 1.2 Scaffold (on the Mac)

```sh
pnpm create tauri-app@4.7.5 ui --template react-ts --manager pnpm --identifier <final.reverse.dns> --yes
cd ui && pnpm add -D typescript@6.0.3 @types/node@^24
pnpm dlx shadcn@4.22.1 init -b radix        # -b base|radix|aria; Base UI is the default since 2026-07
pnpm add @tanstack/react-query@5.104.5 @tanstack/react-virtual@3.14.21 @tanstack/react-router@1.170.45 \
  zustand@5.0.15 lucide-react@1.61.0 motion@14.5.0 @fontsource-variable/inter@5.3.0 \
  wavesurfer.js@8.0.3 react-image-crop@11.1.2 i18next@26.5.4 react-i18next@17.1.0 \
  react-hotkeys-hook@5.3.3 react-error-boundary@6.1.6 sonner@2.0.8 \
  @tauri-apps/plugin-dialog@2.8.3 @crabnebula/tauri-plugin-drag@2.1.0
```

**Choose the bundle identifier once, before the beta.** It is baked into several places:
- the single-instance mutex, window class and window name (`<id>-sim`, `-sic`, `-siw`; see the plugin source);
- the app directories and the WebView2 user data folder, which holds localStorage;
- the updater.

Changing it later orphans user data and lets the old and new builds run side by side. The name check in report 14 is still open, so use a placeholder until then.

### 1.3 Repository layout

```
Kairo/
├─ Cargo.toml                 # workspace: crates/*, apps/ui/src-tauri
├─ .cargo/config.toml         # [env] TS_RS_EXPORT_DIR, TS_RS_LARGE_INT (see §2.2)
├─ crates/
│  ├─ kairo-protocol/         # serde + ts-rs types, method table, PROTOCOL_VERSION
│  ├─ kairo-engine/           # Windows engine (other reports)
│  └─ kairo-mock-engine/      # same protocol; Unix socket on macOS, pipe on Windows
└─ apps/ui/
   ├─ index.html  src/  src/gen/ (ts-rs output, committed; CI fails if stale)
   └─ src-tauri/  (crate kairo-ui: lib.rs, engine_client.rs, media_protocol.rs,
                   clipboard_win.rs, engine_launch.rs, capabilities/, build.rs, tauri.conf.json)
```

### 1.4 `src-tauri/Cargo.toml` (UI shell)

```toml
[dependencies]
tauri = { version = "=2.12.5", features = [] }   # no "protocol-asset", no "unstable", no "tray-icon"
tauri-plugin-single-instance = "2.5.4"
tauri-plugin-window-state   = "2.5.0"
tauri-plugin-opener         = "2.7.2"
tauri-plugin-dialog         = "2.8.3"
tauri-plugin-drag           = "2.1.1"
tauri-plugin-wdio-webdriver = { version = "1.5.0", optional = true }   # E2E only
kairo-protocol = { path = "../../../crates/kairo-protocol" }
interprocess = { version = "2.4.6", features = ["tokio"] }
tokio      = { version = "1.54", features = ["sync", "time", "fs", "io-util", "macros"] }
tokio-util = { version = "0.7.20", features = ["codec"] }
futures-util = { version = "0.3.34", features = ["sink"] }
bytes = "1.12.1"
serde = { version = "1.0.229", features = ["derive"] }
serde_json = "1.0.154"
thiserror = "2.0.21"
anyhow = "1.0.104"
http-range = "0.1.6"          # same crate Tauri's asset protocol uses
percent-encoding = "2.3.2"
tracing = "0.1.44"

[target.'cfg(windows)'.dependencies]
windows = { version = "0.62.2", features = ["Win32_Foundation", "Win32_System_DataExchange",
  "Win32_System_Memory", "Win32_System_Ole", "Win32_System_Com", "Win32_UI_Shell",
  "Win32_System_Threading"] }

[target.'cfg(target_os = "macos")'.dependencies]
arboard = { version = "3.7.0", default-features = false }   # Set::file_list() for Mac dev parity

[features]
e2e = ["dep:tauri-plugin-wdio-webdriver"]

[build-dependencies]
tauri-build = { version = "=2.7.4", features = [] }   # moves with tauri: 2.12.5 needs ^2.7.4
```

- Tauri's defaults (`wry`, `compression`, `common-controls-v6`, `dynamic-acl`) are fine.
- Leave `protocol-asset` off: Kairo doesn't use the asset protocol, so no filesystem scope exists for a compromised webview to abuse.
- Leave `unstable` (multi-webview) off. The accessibility focus bug tauri#15624 (focus lost after Alt+Tab, which breaks screen readers and keyboard use) involved that path and was fixed in 2.12.0. Stay single-webview anyway.

### 1.5 `tauri.conf.json` (UI-relevant parts)

```jsonc
{
  "$schema": "https://schema.tauri.app/config/2",
  "productName": "Kairo",
  "identifier": "<final.reverse.dns>",
  "build": { "beforeDevCommand": "pnpm dev", "devUrl": "http://localhost:1420",
             "beforeBuildCommand": "pnpm build", "frontendDist": "../dist" },
  "app": {
    "appDirectoriesOverride": "$LOCALDATA/Kairo/ui",   // 2.12.0+: config/data/localData, ui/caches, ui/logs; WebView2 UDF → …\Kairo\ui\EBWebView
    "withGlobalTauri": false,
    "windows": [{
      "label": "main", "create": false,               // created in setup() so we can add an init script
      "title": "Kairo", "width": 1280, "height": 800, "minWidth": 960, "minHeight": 600,
      "center": true, "visible": false,                // shown after first paint (no white flash)
      "backgroundColor": "#0B0C10", "theme": "Dark",
      "dragDropEnabled": true,                         // needed for "drop files to import"; see gotcha G21
      "zoomHotkeysEnabled": true,                      // default false; Ctrl +/- helps low-vision users
      "scrollBarStyle": "fluentOverlay"                // needs WebView2 ≥125.0.2535.41
    }],
    "security": {
      "csp": {
        "default-src": "'self'",
        "script-src": "'self'",
        "style-src": "'self' 'unsafe-inline'",
        "img-src": "'self' data: blob: kairo-media: http://kairo-media.localhost",
        "media-src": "'self' blob: kairo-media: http://kairo-media.localhost",
        "font-src": "'self' data:",
        "connect-src": "ipc: http://ipc.localhost"
      },
      // devCsp: omitting it (or null) means `csp` is ALSO used in dev. Copy `csp` and add
      // "ws://localhost:1420" to connect-src for Vite HMR. [Unverified] whether Tauri enforces CSP on
      // devUrl pages on desktop, so always test the real CSP with `tauri build --debug`.
      "dangerousDisableAssetCspModification": ["style-src"]
    }
  },
  "bundle": { "targets": ["nsis"], "externalBin": ["binaries/kairo-engine"] }
}
```

- **Updater artifacts** (updated 2026-10-23) come only from the release build, through an overlay: `tauri build --config tauri.release.conf.json` with `{"bundle":{"createUpdaterArtifacts":true},"plugins":{"updater":{"pubkey":"<tauri signer public key>"}}}`. No updater plugin is compiled in, but tauri-cli 2.12.4 stops every bundling build that has `createUpdaterArtifacts` on and no `plugins > updater`. It signs with `TAURI_SIGNING_PRIVATE_KEY` and fails without it unless given `--no-sign`. Control verifies the `.sig` (PLAN.md §2.1).

- **`appDirectoriesOverride`** is documented to move every `app_*_dir`, "including Tauri itself (the default webview data directory on Windows and Linux) and plugins". The window `dataDirectory` option is not affected.
  - With the override, all of Kairo's per-user state lives under `%LOCALAPPDATA%\Kairo`.
  - The NSIS "delete app data" option then needs a hook, because it deletes the identifier folders.
- **`contentProtected: true`** would call `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)` (tao source). That hides the library from every capture tool, Discord screen share included. Leave it off by default.

### 1.6 Capabilities (ACL)

Tauri 2 checks plugin commands against capabilities. **App commands are open to every local window unless an app manifest exists.** In `webview/mod.rs`, the ACL check runs only when `plugin_command.is_some() || has_app_acl_manifest || !is_local`. So declare the app commands:

```rust
// src-tauri/build.rs
fn main() {
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "engine_call", "engine_state", "engine_start",
            "clip_copy", "clip_reveal", "clip_drag_win",
        ]),
    )).expect("tauri-build failed");
}
```

Generated identifiers turn `_` into `-`: `allow-engine-call`. The source is `tauri-utils/src/acl/build.rs`, `autogenerate_command_permissions`.

```jsonc
// src-tauri/capabilities/main.json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "main",
  "windows": ["main"],
  "permissions": [
    "core:default",
    "core:window:allow-show", "core:window:allow-set-focus", "core:window:allow-set-progress-bar",
    "dialog:allow-open", "dialog:allow-save", "dialog:allow-message",
    "drag:allow-start-drag",
    { "identifier": "opener:allow-open-url",
      "allow": [{ "url": "https://github.com/Maty3k/Kairo/*" }, { "url": "https://apps.microsoft.com/*" }] },
    "allow-engine-call", "allow-engine-state", "allow-engine-start",
    "allow-clip-copy", "allow-clip-reveal", "allow-clip-drag-win"
  ]
}
```

- `core:window:default` is read-only (getters only). `show`, `set-focus` and `set-progress-bar` must be listed explicitly.
- `dialog:default` = `allow-message, allow-save, allow-open`.
- `drag:default` = `allow-start-drag`.
- **E2E**: `wdio-webdriver:default` must not appear in `main.json`, because the plugin isn't compiled into normal builds. Put it inline in an E2E-only overlay: `tauri build --features e2e --config tauri.e2e.conf.json` with `{"app":{"security":{"capabilities":["main",{"identifier":"e2e","windows":["main"],"permissions":["wdio-webdriver:default"]}]}}}`.

### 1.7 `lib.rs` skeleton

```rust
use tauri::{Emitter, Manager};

pub fn run() {
    let builder = tauri::Builder::default()
        // 1st: a second instance forwards argv to us and exits before anything else runs
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.unminimize(); let _ = w.show(); let _ = w.set_focus();
            }
            if let Some(route) = route_from_args(&args) { let _ = app.emit("ui:navigate", route); }
        }))
        .plugin(tauri_plugin_window_state::Builder::default()
            .with_state_flags(tauri_plugin_window_state::StateFlags::all()
                .difference(tauri_plugin_window_state::StateFlags::VISIBLE))   // we show the window ourselves
            .build())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_drag::init())
        .register_asynchronous_uri_scheme_protocol("kairo-media", media_protocol::handle)
        .invoke_handler(tauri::generate_handler![
            engine_client::engine_call, engine_client::engine_state, engine_launch::engine_start,
            shell::clip_copy, shell::clip_reveal, shell::clip_drag_win,
        ])
        .setup(|app| {
            app.manage(engine_client::EngineClient::spawn(app.handle().clone()));
            app.manage(media_protocol::MediaResolver::default());
            let cfg = app.config().app.windows.iter().find(|w| w.label == "main").unwrap().clone();
            let boot = serde_json::json!({ "route": route_from_args(&std::env::args().collect::<Vec<_>>()) });
            tauri::WebviewWindowBuilder::from_config(app.handle(), &cfg)?
                .initialization_script(format!("window.__KAIRO_BOOT__ = {boot};"))
                .build()?;
            // Safety net: never leave an invisible window if the JS bundle fails to boot.
            let h = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                if let Some(w) = h.get_webview_window("main") { let _ = w.show(); }
            });
            Ok(())
        });
    #[cfg(feature = "e2e")]
    let builder = builder.plugin(tauri_plugin_wdio_webdriver::init());
    builder.run(tauri::generate_context!()).expect("error while running the Kairo UI");
}
```

Verified signatures (tauri 2.12.4):
- `Builder::register_asynchronous_uri_scheme_protocol<N, H: Fn(UriSchemeContext<'_, R>, http::Request<Vec<u8>>, UriSchemeResponder) + Send + Sync + 'static>`
- `WebviewWindowBuilder::from_config(&M, &WindowConfig)`
- `.initialization_script(impl Into<String>)`
- the single-instance callback `FnMut(&AppHandle<R>, Vec<String>, String)`; the README says to register it first.

The window is created in Rust, so the first route reaches React through `window.__KAIRO_BOOT__`. That is more reliable than putting a `#fragment` into `WebviewUrl::App`.

### 1.8 Lifecycle: "close = exit", and do the WebView2 processes exit?

**What the source shows** (tag `tauri-v2.12.4` / `wry-v0.57.0`):
1. `CloseRequested` → `on_window_close` drops the tao window. In tauri-runtime-wry `lib.rs`, `TaoWindowEvent::Destroyed` removes the `WindowWrapper`, dropping its webviews. When the store is empty, it raises `RunEvent::ExitRequested { code: None, tx }`, and if nobody calls `prevent_exit()` it sets `ControlFlow::Exit`.
2. wry's `impl Drop for InnerWebView` calls `self.controller.Close()`.
3. Microsoft:
   - "To safely shut down associated browser and renderer processes, use the `Close` method of `CoreWebView2Controller`" (process model page).
   - `BrowserProcessExited` fires "when all associated WebViews are closed, after all resources have been released (including the user data folder)".
   - "Files in UDFs might still be in use after the WebView2 app is closed … wait for the browser process and all child processes to exit." Exit is therefore **asynchronous, but expected**.
4. A hidden window keeps the app alive (tauri#5611; the maintainers call this expected behaviour). So: no hidden helper windows, no tray icon in the UI (the engine owns the tray), and no `prevent_exit`.

**What to measure on the PC** (spike S3). Run it *inside* the interactive session with `schtasks /it`, because window messages and `CloseMainWindow` cannot cross session boundaries:

```powershell
# measure-webview2-exit.ps1  (Windows PowerShell 5.1 or 7)
param([string]$Exe = "$env:LOCALAPPDATA\Kairo\Kairo.exe", [string]$Udf = "Kairo\ui", [int]$WaitSec = 20)
$app = Start-Process -FilePath $Exe -PassThru
Start-Sleep -Seconds 8
function Get-Wv2 { Get-CimInstance Win32_Process -Filter "Name='msedgewebview2.exe'" |
                   Where-Object { $_.CommandLine -like "*$Udf*" } }   # every child gets --user-data-dir
$before = @(Get-Wv2)
$priv = ($before | ForEach-Object { (Get-Process -Id $_.ProcessId -EA SilentlyContinue).PrivateMemorySize64 } |
         Measure-Object -Sum).Sum
$null = $app.CloseMainWindow()                       # WM_CLOSE → CloseRequested → exit
$sw = [Diagnostics.Stopwatch]::StartNew(); $null = $app.WaitForExit(10000); $hostMs = $sw.ElapsedMilliseconds
do { Start-Sleep -Milliseconds 250; $left = @(Get-Wv2) } while ($left.Count -and $sw.Elapsed.TotalSeconds -lt $WaitSec)
[pscustomobject]@{ wv2Before = $before.Count; wv2PrivateMB = [math]::Round($priv/1MB)
  hostExitMs = $hostMs; allWv2GoneMs = $sw.ElapsedMilliseconds; leftovers = $left.Count; engineAlive = [bool](Get-Process kairo-engine -EA SilentlyContinue)
} | ConvertTo-Json | Set-Content "$env:TEMP\kairo-wv2-exit.json"
```

- **Pass criteria [Estimate]:**
  - host exit < 1 s;
  - every `msedgewebview2.exe` with Kairo's `--user-data-dir` gone within ~5 s;
  - `engineAlive = true`;
  - no `Kairo.exe` left in Task Manager → *Background processes*.
- **Variants to run:**
  - close with the X button;
  - close while a clip plays;
  - close while an export runs (the engine keeps exporting);
  - close a UI that started the engine itself (the engine must survive; §2.6).
- **[Unverified]** The browser command line also carries `--webview-exe-name=<host exe>`. Only a user-posted dump shows this, so filter on the UDF path, as the script does.

### 1.9 Building and checking from the Mac

- `cargo xwin check --target x86_64-pc-windows-msvc -p kairo-ui` needs `llvm-rc` on `PATH`: `brew install llvm`, then add `/opt/homebrew/opt/llvm/bin`. `tauri-build` compiles the Windows resource file (icon, manifest) with it, according to Tauri's cross-compile docs.
- **[Unverified]** whether `generate_context!` needs `frontendDist` to exist for a `check`. Keep a placeholder `apps/ui/dist/index.html` in the repo, or run `pnpm build` first.
- On the 8 GB M2, set `CARGO_BUILD_JOBS=4` and `[profile.dev] debug = "line-tables-only"`. **[Estimate]** The first `tauri dev` build takes about 5–10 minutes.
- On the PC, `pnpm tauri dev` must also run in the interactive session (a scheduled task with `/it`, as in brief §3.5). Otherwise the window opens on an invisible desktop.

---

## 2. Engine connection

### 2.1 Shape

```
React (WebView2) ─ invoke("engine_call",{method,params}) ─► kairo-ui shell (Rust) ─ JSON-RPC 2.0 ─► kairo-engine
        ▲                                                   │  \\.\pipe\kairo-<SID> (macOS mock: /tmp/kairo-<uid>.sock)
        ├─ listen("engine:notification")  ◄─────────────────┤  4-byte big-endian length-prefixed frames
        └─ listen("engine:state")  connecting|connected|notRunning|accessDenied|incompatible
<video src="http://kairo-media.localhost/v%2F01J…"> ─► kairo-ui media protocol ─(id→path, cached; asks the engine once)─► file
```

- The webview cannot open pipes. The shell is the only client. The pipe name, DACL and `PIPE_REJECT_REMOTE_CLIENTS` belong to the engine IPC report.
- interprocess's `GenericNamespaced` maps to `\\.\pipe\<name>` on Windows and to `/tmp/<name>` on other Unices (macOS). Its Windows client already waits on `ERROR_PIPE_BUSY` (`WaitNamedPipeW` in `c_wrappers.rs`).
- **Framing** follows whatever the IPC report decides. This sketch uses `LengthDelimitedCodec`: a u32 big-endian length, 8 MiB default max (raise to 16 MiB). StreamJsonRpc documents `LengthHeaderMessageHandler` as "a big endian, 4 byte integer" prefix, so the C# fallback can speak the same wire format.

### 2.2 Types: ts-rs vs tauri-specta

| | **ts-rs 12.0.1** (2026-01-31, MIT) | tauri-specta 2.0.0-rc.25 (2026-05-08) + specta 2.0.0-rc.25 |
|---|---|---|
| Maturity | Stable major line; serde-compat by default | Release candidate for about two years |
| Generates | `.ts` for any serde type (structs, tagged enums) | Typed `commands.*` and events for Tauri commands |
| Fit | The contract is engine↔UI and must survive a C# engine | Would only type the ~6 shell commands |

**[Decision]** Use ts-rs in `kairo-protocol`, plus a 30-line hand-written `commands.ts` for the shell commands. If the engine moves to C#, add JSON Schema export (for example `schemars` 1.2.3) from the same types and generate the C# types from it.

```toml
# .cargo/config.toml (workspace root)
[env]
TS_RS_EXPORT_DIR = { value = "apps/ui/src/gen", relative = true }
TS_RS_LARGE_INT  = "number"     # default "bigint" ≠ JSON.parse; keep ints < 2^53 (bytes, ms) or send strings
```

```rust
// crates/kairo-protocol/src/lib.rs (excerpt)
use serde::{Deserialize, Serialize};
use ts_rs::TS;
pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")] #[ts(export)]
pub struct ClipSummary {
    pub id: String, pub kind: ClipKind, pub title: String, pub game: Option<String>,
    pub captured_at_ms: i64, pub duration_ms: u32, pub size_bytes: u64,
    pub width: u32, pub height: u32, pub fps: f32, pub video_codec: String,
    pub favorite: bool, pub poster_v: String,           // content hash, used as a cache-busting query
    pub sprite: Option<SpriteInfo>,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")] #[ts(export)]
pub struct SpriteInfo { pub cols: u32, pub rows: u32, pub count: u32, pub tile_w: u32, pub tile_h: u32, pub interval_ms: u32, pub v: String }

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "method", content = "params")] #[ts(export)]
pub enum Notification {                                  // → TS discriminated union on `method`
    #[serde(rename = "buffer.status")]   BufferStatus(BufferStatus),
    #[serde(rename = "clip.saved")]      ClipSaved(ClipSummary),
    #[serde(rename = "library.changed")] LibraryChanged(ChangedIds),
    #[serde(rename = "export.progress")] ExportProgress(ExportProgress),
    #[serde(rename = "storage.status")]  StorageStatus(StorageStatus),
}

/// One named Params struct and one named Result struct per method, so the TS map imports cleanly.
macro_rules! rpc_methods { ($( $name:literal => $p:ty => $r:ty ),* $(,)?) => {
    pub const METHODS: &[&str] = &[$($name),*];
    pub fn ts_rpc_methods(cfg: &ts_rs::Config) -> String {
        use ts_rs::TS; let mut imports = std::collections::BTreeSet::new();
        let mut body = String::from("export interface RpcMethods {\n");
        $( imports.insert(<$p as TS>::ident(cfg)); imports.insert(<$r as TS>::ident(cfg));
           body += &format!("  \"{}\": {{ params: {}; result: {} }};\n", $name, <$p as TS>::ident(cfg), <$r as TS>::ident(cfg)); )*
        let mut out = String::from("// @generated by kairo-protocol. Do not edit.\n");
        for i in imports { out += &format!("import type {{ {i} }} from \"./{i}\";\n"); }
        out + &body + "}\n"
    }
}}
rpc_methods! {
    "engine.hello"        => HelloParams       => HelloResult,
    "library.list"        => LibraryListParams => LibraryPage,
    "library.resolvePath" => ResolvePathParams => ResolvedPath,
    "clip.update"         => ClipUpdateParams  => ClipSummary,
    "clip.delete"         => ClipIds           => Empty,
    "export.estimate"     => ExportSpec        => ExportEstimate,
    "export.start"        => ExportStart       => JobRef,      // ExportStart carries a client requestId (idempotent retry)
    "export.cancel"       => JobRef            => Empty,
    "jobs.list"           => Empty             => JobList,
    "storage.status"      => Empty             => StorageStatus,
    "waveform.peaks"      => PeaksParams       => Peaks,
    "clip.keyframes"      => ClipRef           => Keyframes,
}
#[cfg(test)] #[test] fn export_rpc_map() {
    let cfg = ts_rs::Config::from_env();
    std::fs::write(cfg.out_dir().join("RpcMethods.ts"), ts_rpc_methods(&cfg)).unwrap();
}
```

- ts-rs 12's trait methods take `&Config`: `TS::name(cfg)`, `TS::ident(cfg)`, `TS::export_all(cfg)`, `Config::from_env()`, `Config::out_dir()`.
- `#[ts(export)]` types are written when `cargo test` runs. CI runs `cargo test -p kairo-protocol && git diff --exit-code apps/ui/src/gen`.
- For `serde_json::Value` fields, enable ts-rs's `serde-json-impl` feature.

### 2.3 `EngineClient` (shell side, sketch)

```rust
use futures_util::{SinkExt, StreamExt};
use interprocess::local_socket::tokio::{prelude::*, Stream};
use std::{collections::HashMap, sync::{Arc, atomic::{AtomicU64, Ordering}}, time::Duration};
use tauri::{AppHandle, Emitter};
use tokio::sync::{mpsc, oneshot, watch, Mutex};
use tokio_util::codec::{Framed, LengthDelimitedCodec};

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<serde_json::Value, EngineError>>>>>;
pub struct EngineClient { out: mpsc::Sender<bytes::Bytes>, pending: Pending, next: AtomicU64, state: watch::Receiver<EngineState> }

impl EngineClient {
    pub fn spawn(app: AppHandle) -> Self {
        let (out, mut out_rx) = mpsc::channel::<bytes::Bytes>(256);
        let (state_tx, state) = watch::channel(EngineState::Connecting);
        let pending: Pending = Default::default();
        let p = pending.clone();
        tauri::async_runtime::spawn(async move {
            let mut backoff = Duration::from_millis(250);
            loop {
                match connect().await {
                    Ok(stream) => {
                        backoff = Duration::from_millis(250);
                        let codec = LengthDelimitedCodec::builder().max_frame_length(16 << 20).new_codec();
                        let (mut sink, mut source) = Framed::new(stream, codec).split();
                        // 1) handshake: send engine.hello first and compare PROTOCOL_VERSION
                        //    (Connected | Incompatible{..}); drop frames queued while disconnected
                        // 2) pump until EOF or error
                        loop { tokio::select! {
                            Some(frame) = out_rx.recv() => if sink.send(frame).await.is_err() { break },
                            msg = source.next() => match msg { Some(Ok(buf)) => dispatch(&app, &p, &buf).await, _ => break },
                        }}
                        for (_, tx) in p.lock().await.drain() { let _ = tx.send(Err(EngineError::Disconnected)); }
                        set_state(&app, &state_tx, EngineState::Connecting);
                    }
                    Err(e) => set_state(&app, &state_tx, match e.kind() {
                        std::io::ErrorKind::NotFound => EngineState::NotRunning,         // pipe doesn't exist
                        std::io::ErrorKind::PermissionDenied => EngineState::AccessDenied, // e.g. elevated engine / DACL
                        _ => EngineState::Connecting }),
                }
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(Duration::from_secs(5));
            }
        });
        Self { out, pending, next: AtomicU64::new(1), state }
    }

    pub async fn call(&self, method: &str, params: serde_json::Value, timeout: Duration) -> Result<serde_json::Value, EngineError> {
        if !matches!(*self.state.borrow(), EngineState::Connected { .. }) { return Err(EngineError::NotConnected); }
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);
        let frame = serde_json::to_vec(&serde_json::json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})).unwrap();
        self.out.send(frame.into()).await.map_err(|_| EngineError::Disconnected)?;
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(r)) => r,
            Ok(Err(_)) => Err(EngineError::Disconnected),
            Err(_) => { self.pending.lock().await.remove(&id); Err(EngineError::Timeout) }
        }
    }
}

async fn dispatch(app: &AppHandle, pending: &Pending, buf: &[u8]) {
    let Ok(msg) = serde_json::from_slice::<serde_json::Value>(buf) else { tracing::warn!("bad frame"); return };
    if let Some(id) = msg.get("id").and_then(|v| v.as_u64()) {
        if let Some(tx) = pending.lock().await.remove(&id) {
            let _ = tx.send(match msg.get("error") { Some(e) => Err(EngineError::from_rpc(e)),
                                                     None => Ok(msg.get("result").cloned().unwrap_or_default()) });
        }
    } else if let (Some(method), params) = (msg.get("method"), msg.get("params")) {
        // One event; the JSON-RPC method stays in the payload (Tauri event names can't contain '.')
        let _ = app.emit("engine:notification", serde_json::json!({ "method": method, "params": params }));
    }
}

#[tauri::command]   // async + State<'_> ⇒ must return Result (Tauri rule)
pub async fn engine_call(client: tauri::State<'_, EngineClient>, method: String, params: serde_json::Value)
    -> Result<serde_json::Value, EngineError> {
    if !kairo_protocol::METHODS.contains(&method.as_str()) { return Err(EngineError::UnknownMethod { method }); }
    let t = if method.starts_with("export.") { 30 } else { 10 };
    client.call(&method, params, Duration::from_secs(t)).await
}
```

- `EngineError` is `#[derive(Serialize, thiserror::Error, TS)] #[serde(tag = "kind", rename_all = "camelCase")]`, with the variants `NotConnected | Disconnected | Timeout | UnknownMethod{method} | Rpc{code, message, data}`.
- Tauri requires `Result<T, E: Serialize>` for fallible commands. The rejected `invoke` promise receives the serialized object.
- `Emitter::emit<S: Serialize + Clone>(&self, event: &str, payload: S)` is the 2.12 signature.

### 2.4 React side

```ts
// src/lib/engine.ts
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type { RpcMethods } from '@/gen/RpcMethods';
import type { Notification } from '@/gen/Notification';
import type { EngineState } from '@/gen/EngineState';

export const call = <M extends keyof RpcMethods>(method: M, params: RpcMethods[M]['params']) =>
  invoke<RpcMethods[M]['result']>('engine_call', { method, params });
export const onNotification = (cb: (n: Notification) => void): Promise<UnlistenFn> =>
  listen<Notification>('engine:notification', (e) => cb(e.payload));
export const onEngineState = (cb: (s: EngineState) => void) => listen<EngineState>('engine:state', (e) => cb(e.payload));
export const getEngineState = () => invoke<EngineState>('engine_state');   // initial value: avoids missing an early event
```

```ts
// src/lib/useEngineBridge.ts (mounted once, under QueryClientProvider)
export function useEngineBridge() {
  const qc = useQueryClient();
  useEffect(() => {
    let dead = false; const offs: UnlistenFn[] = [];
    const keep = (p: Promise<UnlistenFn>) => p.then((off) => (dead ? off() : offs.push(off)));  // StrictMode-safe
    keep(onNotification((n) => {
      switch (n.method) {
        case 'clip.saved':      qc.invalidateQueries({ queryKey: ['clips'] }); toast(t('toast.clipSaved')); break;
        case 'library.changed': qc.invalidateQueries({ queryKey: ['clips'] }); break;
        case 'export.progress': useJobs.getState().upsert(n.params); break;
        case 'storage.status':  qc.setQueryData(['storage'], n.params); break;
        case 'buffer.status':   useEngine.getState().setBuffer(n.params); break;
      }
    }));
    keep(onEngineState((s) => { useEngine.getState().setState(s); if (s.state === 'connected') qc.invalidateQueries(); }));
    getEngineState().then((s) => useEngine.getState().setState(s));
    return () => { dead = true; offs.forEach((off) => off()); };
  }, [qc]);
}
```

- **Why events and not one long-lived `Channel`.** Notifications arrive at 1–10 Hz. Events survive a webview reload: the new page just listens again. They can also be faked in tests with `emit()` under `mockIPC(…, { shouldMockEvents: true })`.
- Use a `Channel` only for per-call streams. The drag plugin already uses one.
- Note that 2.12.3 fixed channels stalling when an `onmessage` handler throws.

### 2.5 Connection states in the UI

| `EngineState` | Cause | UI |
|---|---|---|
| `connecting` | Startup, or reconnecting with 0.25 s → 5 s backoff | Thin top bar "Connecting to Kairo…". Show the cached library greyed, disable mutations |
| `connected{engineVersion}` | Handshake OK | Normal. Every query is invalidated once after a reconnect |
| `notRunning` | The pipe doesn't exist (`NotFound`) | Banner "Kairo isn't recording" + **Start** (`engine_start`) + "Start with Windows" hint |
| `accessDenied` | `PermissionDenied`: an elevated engine or another user | Explain; offer "Restart Kairo normally" |
| `incompatible{engineProtocol}` | UI and engine `PROTOCOL_VERSION` differ (e.g. after an update) | "Finish updating: restart the engine" → `engine.shutdown`, then relaunch |

Rules:
- Calls fail fast while not connected.
- TanStack Query retries queries once and never retries mutations.
- `export.start` carries a client `requestId` (ULID) so a retry cannot start two exports.
- After a reconnect, `jobs.list` resynchronises progress. Jobs the engine no longer knows become "interrupted".

### 2.6 Starting the engine from the UI

```rust
#[cfg(windows)]
#[tauri::command]
pub async fn engine_start() -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    use windows::Win32::System::Threading::{CREATE_BREAKAWAY_FROM_JOB, CREATE_NEW_PROCESS_GROUP, DETACHED_PROCESS};
    let exe = std::env::current_exe().map_err(|e| e.to_string())?.with_file_name("kairo-engine.exe"); // bundle.externalBin lands next to Kairo.exe
    let base = (DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP).0;
    let spawn = |flags: u32| std::process::Command::new(&exe).arg("--started-by-ui")
        .stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null())
        .creation_flags(flags).spawn();
    match spawn(base | CREATE_BREAKAWAY_FROM_JOB.0) {
        Ok(_) => Ok(()),
        Err(e) if e.raw_os_error() == Some(5) => spawn(base).map(|_| ()).map_err(|e| e.to_string()), // job forbids breakaway
        Err(e) => Err(e.to_string()),
    }
}
```

- The flag values were verified in windows 0.62.2: `DETACHED_PROCESS` = 0x8, `CREATE_NEW_PROCESS_GROUP` = 0x200, `CREATE_BREAKAWAY_FROM_JOB` = 0x0100_0000.
- Dropping `std::process::Child` does not kill the process.
- Breakaway matters when the UI itself runs inside a kill-on-close job, as some launchers and dev tools use. Without it, the engine would die with the UI.

---

## 3. Local video playback in WebView2

### 3.1 Verified facts

1. **tauri#4133** ("streaming playback without loading the entire file into memory") is **open**. It was opened 2022-05-15; the last comment is from 2026-01-05.
2. **Asset protocol in 2.12.4** (`crates/tauri/src/protocol/asset.rs`):
   - It now runs on `tauri::async_runtime` and replies through a responder. The 2.12.0 changelog says: "Load `asset://` files asynchronously instead of on the event loop thread."
   - The multipart fix is also in 2.12.0.
   - A single range is capped at `MAX_LEN = 1000 * 1024` bytes.
   - **A GET without `Range` reads the whole file** (`Vec::with_capacity(len)` + `read_to_end`).
3. **Chromium** (`resource_multi_buffer_data_provider.cc`):
   - Every media request sets `Range: bytes=<pos>-`, including position 0.
   - It checks `206`/`Content-Range` only for URLs in the HTTP family.
   - It accepts a short `206` and continues with a follow-up `Range` request at the new position.
4. **wry 0.57 on WebView2** (`src/webview2/mod.rs`):
   - Custom schemes are registered as `http(s)://<scheme>.localhost/*` filters with `AddWebResourceRequestedFilterWithRequestSourceKinds(…ALL…)`.
   - The handler takes a deferral. The async responder `PostMessage`s back to the UI thread.
   - Each body becomes `SHCreateMemStream(content)`. So every response is fully buffered and copied: **no streaming, so cap the chunk size.**
5. **Microsoft WebView2 engineer** on WebView2Feedback #3230 (2023): "the custom scheme does not understand partial content responses … Chromium does not handle certain HTTP protocol specific scenarios for non-HTTP schemes"; the workaround is an `http` URL. On #2679 the same team wrote that the right approach is to pass "a smaller stream that contains the part of the file that is requested"; WebView2 ignores the response's range headers and reads the whole stream. → Tauri's `http://<scheme>.localhost` mapping is what makes 206 work; never switch to a true WebView2 custom scheme.
6. **The memory leak in `put_Response` streams** (#2171) was fixed in runtime 104 (2022).
7. **Local Network Access.** The WebView2 announcement #126 (2026-02-12): LNA is currently **off** in WebView2 and flag-gated from 145. It will become "API-controlled" later, when requests from pages to localhost "will be blocked unless explicitly permitted". That is a future risk for any loopback-server design.
8. **`SetVirtualHostNameToFolderMapping`.** Microsoft's docs still warn that media through a virtual host "can be slow to load". One user on #2679 (2025-12-05) reports a 4 GB file now loads and seeks instantly with SDK 1.0.3595.46 **[Unverified]**. Tauri doesn't expose it, but `webview.with_webview(|w| w.controller())` returns the `ICoreWebView2Controller` (webview2-com 0.39).

### 3.2 Options

| Option | Seek in 500 MB+ | RAM per request | Security | Mac dev | Verdict |
|---|---|---|---|---|---|
| Built-in `asset://` + `convertFileSrc` | OK for `<video>` (Range is always sent) | ≤1000 KiB ×2 copies; **whole file** for a Range-less GET (`fetch`, `<a download>`) | Exposes a filesystem scope to any script in the webview | Works | No |
| **`kairo-media` async URI scheme (ID-addressed, 206 chunks)** | Same transport; you set the chunk size | ≤2 MiB ×2 copies **[Estimate: tune 1–4 MiB]** | Only known clip IDs; no path in the URL; GET/HEAD only | Works (`kairo-media://localhost/…`) | **Recommended** |
| Loopback HTTP (axum 0.8.9 / tower-http 0.7.1) | Real streaming | Small | Needs a random port, a per-session token, Origin checks and a CSP entry; **LNA will gate it**; a listening socket in a capture app looks bad to AV | Works | Fallback of last resort |
| WebView2 virtual host mapping via `with_webview` | Native streaming (per the 2025 user report) | None in our process | Maps whole folders (library, cache) into the webview | Windows only | Measured fallback if the chunked scheme fails the S3 numbers |

### 3.3 `media_protocol.rs` (sketch)

```rust
use tauri::{http::{header::*, Method, Request, Response, StatusCode}, Manager, Runtime, UriSchemeContext, UriSchemeResponder};
use tokio::io::{AsyncReadExt, AsyncSeekExt};

const CHUNK: u64 = 2 * 1024 * 1024;        // ≤ this per 206; Chromium re-requests from the next byte
const SMALL: u64 = 8 * 1024 * 1024;        // posters/sprites may be sent whole on a Range-less GET

pub fn handle<R: Runtime>(ctx: UriSchemeContext<'_, R>, req: Request<Vec<u8>>, responder: UriSchemeResponder) {
    if ctx.webview_label() != "main" { return responder.respond(status(StatusCode::FORBIDDEN)); }
    let app = ctx.app_handle().clone();
    tauri::async_runtime::spawn(async move {
        let res = serve(&app, req).await.unwrap_or_else(|e| { tracing::warn!(error = %e, "kairo-media"); status(StatusCode::INTERNAL_SERVER_ERROR) });
        responder.respond(res);
    });
}

async fn serve<R: Runtime>(app: &tauri::AppHandle<R>, req: Request<Vec<u8>>) -> anyhow::Result<Response<Vec<u8>>> {
    if req.method() != Method::GET && req.method() != Method::HEAD { return Ok(status(StatusCode::METHOD_NOT_ALLOWED)); }
    // convertFileSrc() percent-encodes '/', so "/v%2F01J…" → "v/01J…"
    let path = percent_encoding::percent_decode_str(req.uri().path().trim_start_matches('/')).decode_utf8()?;
    let (kind, id) = parse(&path).ok_or_else(|| anyhow::anyhow!("bad path"))?;   // kind ∈ v|poster|sprite|shot, id = 26-char ULID
    let file = app.state::<MediaResolver>().resolve(app, kind, id).await?;          // cache; else engine "library.resolvePath"
    let mut f = tokio::fs::File::open(&file).await?;                                // opened per request ⇒ no lingering lock
    let len = f.metadata().await?.len();
    let b = Response::builder().header(CONTENT_TYPE, kind.mime()).header(ACCEPT_RANGES, "bytes")
        .header(CACHE_CONTROL, kind.cache_control());                               // video: no-store; images: immutable (+ ?v=hash)
    let head = req.method() == Method::HEAD;
    match req.headers().get(RANGE).and_then(|v| v.to_str().ok()) {
        Some(r) => {
            let Ok(ranges) = http_range::HttpRange::parse(r, len) else {
                return Ok(Response::builder().status(StatusCode::RANGE_NOT_SATISFIABLE)
                    .header(CONTENT_RANGE, format!("bytes */{len}")).body(vec![])?);
            };
            let Some(first) = ranges.first() else { return Ok(status(StatusCode::RANGE_NOT_SATISFIABLE)) };   // media sends one open range
            let (start, n) = (first.start, first.length.min(CHUNK));
            let mut buf = vec![0u8; if head { 0 } else { n as usize }];
            if !head { f.seek(std::io::SeekFrom::Start(start)).await?; f.read_exact(&mut buf).await?; }
            Ok(b.status(StatusCode::PARTIAL_CONTENT)
                .header(CONTENT_RANGE, format!("bytes {start}-{}/{len}", start + n - 1))
                .header(CONTENT_LENGTH, n).body(buf)?)
        }
        None if len <= SMALL => { let mut buf = Vec::new(); if !head { f.read_to_end(&mut buf).await?; }
                                  Ok(b.status(StatusCode::OK).header(CONTENT_LENGTH, len).body(buf)?) }
        None => Ok(status(StatusCode::RANGE_NOT_SATISFIABLE)),   // never load a whole clip into RAM
    }
}
fn status(s: StatusCode) -> Response<Vec<u8>> { Response::builder().status(s).body(vec![]).unwrap() }
```

- `http_range::HttpRange { start, length }` and `HttpRange::parse(&str, u64)` are the same calls Tauri's asset protocol makes.
- `MediaResolver` is a `Mutex<HashMap<(Kind, Id), PathBuf>>`. Clear it on `library.changed` for the affected IDs, because renames move files.
- On Windows, the page origin is `http://tauri.localhost`, so a `<video>` from `kairo-media.localhost` is cross-origin. That's fine without `crossorigin`. Drawing it to a canvas works but taints the canvas: never call `getImageData`; let the engine make posters.

```ts
// src/lib/media.ts
import { convertFileSrc, isTauri } from '@tauri-apps/api/core';
export type MediaKind = 'v' | 'poster' | 'sprite' | 'shot';
export const mediaUrl = (kind: MediaKind, id: string, v?: string) =>
  isTauri() ? convertFileSrc(`${kind}/${id}`, 'kairo-media') + (v ? `?v=${encodeURIComponent(v)}` : '')
            : `/fixtures/${kind}/${id}${kind === 'v' ? '.mp4' : '.jpg'}`;      // browser-only mock mode
```

The injected `convertFileSrc` (in `scripts/core.js`) returns:
- on Windows and Android: `${protocolScheme}://${protocol}.localhost/${encodeURIComponent(path)}`;
- elsewhere: `${protocol}://localhost/…`.

So one helper works on the PC and on the Mac.

### 3.4 Codecs

- **Edge/WebView2 codec support** (Microsoft's support article, 2026-03-31):
  - H.264 is "Supported". The decoder order is Media Foundation (DXVA) → D3D11 → Chromium's FFmpeg software decoder.
  - HEVC is "conditionally supported": it needs a valid HEVC Video Extension (error `0xC00DB3B3` otherwise).
  - AV1 is "Supported".
  - To debug, use the DevTools Media panel (`edge://media-internals` equivalent) and `edge://gpu`.
- **At startup, probe** `navigator.mediaCapabilities.decodingInfo({ type: 'file', video: { contentType: 'video/mp4; codecs="hvc1.1.6.L123.B0"', width: 2560, height: 1440, bitrate: 40e6, framerate: 60 } })`, and the same for `avc1.640033` and AV1.
  - For a clip the webview can't decode, show "Can't preview HEVC here — make an H.264 copy / get the extension" with the Store link `https://apps.microsoft.com/detail/9n4wgh0z6vhq`.
- **Windows N/KN editions**: they lack Media Foundation codecs. Because Edge falls back to FFmpeg for H.264, playback probably still works **[Unverified]**. The same probe catches it.
- **Multi-track audio**: Chromium plays only the first/default track (brief §2.6), so the player plays T1, the mix. Stem soloing in the preview is post-v1.

### 3.5 Tests on the PC

1. Make a 600 MB fixture: 120 s at 40 Mbps, CBR with `h264_amf -rc cbr -b:v 40M`, plus a `noise` filter so the encoder actually spends the bits. Use `-g 60 -movflags +faststart`.
2. **Seek benchmark** (E2E, §7.4): 10 random seeks, measuring `currentTime=` → `seeked`. **Pass [Estimate]**: p95 < 150 ms.
3. **Steady playback at 1440p60 40 Mbps**: `video.getVideoPlaybackQuality().droppedVideoFrames` stays at 0 over 60 s, and Kairo.exe private bytes grow by less than 50 MB.
4. Play the same clip 10 times, then compare `kairo-ui` and `msedgewebview2` private bytes (leak check).
5. Variants: 512 KiB, 1 MiB, 2 MiB and 4 MiB chunks; an imported non-faststart MP4 (moov at the end); delete a clip while it plays, which must succeed because there is no lingering handle.

---

## 4. UI building blocks

### 4.1 Packages

| Need | Package (version, licence) | Notes |
|---|---|---|
| Server state | `@tanstack/react-query` 5.104.5 (MIT) | Data is pushed by the engine, so `staleTime: Infinity`, `refetchOnWindowFocus: false`, `retry: 1`, mutations `retry: 0` |
| Client state | `zustand` 5.0.15 (MIT) | `useEngine` (state, buffer), `useJobs`, `useSelection` |
| Virtual grid | `@tanstack/react-virtual` 3.14.21 (core 3.19.6, MIT) | Options verified: `count, getScrollElement, estimateSize, overscan, gap, lanes, getItemKey, measureElement` |
| Routing | `@tanstack/react-router` 1.170.45 (MIT) | `createHashHistory()` + code-based routes; typed search params for filters. (React Router 8.5.0 also works, but needs Node ≥22.22) |
| Primitives / UI kit | shadcn CLI 4.22.1 with `-b radix`; `radix-ui` 1.8.1 (MIT) | Base UI 1.9.0 is shadcn's default since 2026-07. Radix is "not being deprecated" and has far more examples the agent has seen |
| Styling | `tailwindcss` + `@tailwindcss/vite` 4.3.3, `tw-animate-css` 1.4.0, `class-variance-authority` 0.7.1, `clsx` 2.1.1, `tailwind-merge` 3.7.0 | Dark tokens; `<meta name="color-scheme" content="dark">` so native scrollbars and controls are dark |
| Icons | `lucide-react` 1.61.0 (ISC) | No brand logos. Use text labels or Simple Icons (CC0; trademark rules still apply) for Discord/TikTok |
| Font | `@fontsource-variable/inter` 5.3.0 (OFL-1.1) | Bundled, no network. `font-variant-numeric: tabular-nums` for times and sizes |
| Motion | `motion` 14.5.0 (MIT), `import { motion, MotionConfig } from 'motion/react'` | `<MotionConfig reducedMotion="user">` |
| Toasts | `sonner` 2.0.8 | Injects `<style>`, so it needs the CSP change in §1.5 |
| Shortcuts | `react-hotkeys-hook` 5.3.3 | Ignores form fields by default |
| Waveform | `wavesurfer.js` 8.0.3 (BSD-3-Clause) | §5.2 |
| Crop box | `react-image-crop` 11.1.2 (ISC, <5 KB, "Fully keyboard accessible", "Crop anything") | §5.3 |
| i18n | `i18next` 26.5.4 + `react-i18next` 17.1.0 (MIT) | §8 |

All of these are compatible with Kairo's GPL-3.0. Ship the OFL text for Inter.

### 4.2 Routes and data

- **Routes:**
  - `/`: library, with search params `{ q?, game?, kind?: 'clip'|'shot', fav?: boolean, sort?: 'new'|'old'|'size'|'len' }`;
  - `/clip/$id`: player, trim and export;
  - `/clip/$id/vertical`: the 9:16 editor;
  - `/settings/*`, `/onboarding`, `/diagnostics` (capability report plus "Report a problem", which opens a prefilled GitHub issue through `opener` with no attachments).
- **Hash history.** Use hash history so the WebView2 back and forward gestures (mouse XButton1/2) work, and boot or single-instance routes are simple hashes.
  - The engine may also bind XButtons as clip triggers. Decide whether the engine ignores them while the Kairo UI is in the foreground (product question).
- **Query for the grid:**

```ts
export const clipsQuery = (f: LibraryFilter) => infiniteQueryOptions({
  queryKey: ['clips', f],
  queryFn: ({ pageParam }) => call('library.list', { ...f, cursor: pageParam, limit: 200 }),
  initialPageParam: null as string | null,
  getNextPageParam: (last) => last.nextCursor ?? null,
  placeholderData: keepPreviousData,                 // filter changes don't flash an empty grid
});
```

- **Search** runs in the engine: SQLite FTS5 over title, game and tags (brief §7.1). The UI debounces with `useDeferredValue` plus ~150 ms.
- **Filter chips**: game, clips/screenshots, favourites, date range. **Sort**: newest, oldest, largest, longest. Show counts per chip from the engine's facet query.

### 4.3 Virtualized grid (row virtualization over N columns)

```tsx
const MIN_W = 280, GAP = 16, ROW_H = 158 + 56;           // 16:9 thumbnail + meta row
export function ClipGrid({ filter }: { filter: LibraryFilter }) {
  const ref = useRef<HTMLDivElement>(null);
  const [w, setW] = useState(0);
  useLayoutEffect(() => { const ro = new ResizeObserver(([e]) => setW(e.contentRect.width)); ro.observe(ref.current!); return () => ro.disconnect(); }, []);
  const cols = Math.max(1, Math.floor((w + GAP) / (MIN_W + GAP)));
  const q = useInfiniteQuery(clipsQuery(filter));
  const clips = q.data?.pages.flatMap((p) => p.items) ?? [];
  const rows = Math.ceil(clips.length / cols);
  const v = useVirtualizer({ count: rows, getScrollElement: () => ref.current, estimateSize: () => ROW_H + GAP, overscan: 3 });
  const vis = v.getVirtualItems();
  useEffect(() => { const last = vis.at(-1); if (last && last.index >= rows - 2 && q.hasNextPage && !q.isFetchingNextPage) void q.fetchNextPage(); }, [vis, rows, q]);
  const total = q.data?.pages[0]?.total ?? clips.length;
  return (
    <div ref={ref} className="h-full overflow-auto" role="list" aria-label={t('library.grid')}>
      <div style={{ height: v.getTotalSize(), position: 'relative' }}>
        {vis.map((row) => (
          <div key={row.key} className="absolute inset-x-0 grid" style={{ transform: `translateY(${row.start}px)`, gridTemplateColumns: `repeat(${cols}, minmax(0,1fr))`, gap: GAP }}>
            {clips.slice(row.index * cols, (row.index + 1) * cols).map((c, i) =>
              <ClipCard key={c.id} clip={c} posInSet={row.index * cols + i + 1} setSize={total} />)}
          </div>
        ))}
      </div>
    </div>
  );
}
```

- Give each card `role="listitem"`, `aria-posinset` and `aria-setsize`, with a real `<button>` inside to open the clip.
- Arrow keys move a roving focus index: `v.scrollToIndex(row)`, then focus after render.
- Right-click opens a Radix `ContextMenu`. For keyboard users, add a visible "More" `DropdownMenu` button, because a context menu is not reachable from the keyboard.

### 4.4 Hover-scrub sprites (no `<video>` on hover)

The engine writes one keyframe tile per second (1 s GOP; brief §7.1) and returns `SpriteInfo { cols, rows, count, tileW, tileH, intervalMs, v }`.

```tsx
function useScrub(s?: SpriteInfo) {
  const [i, setI] = useState<number | null>(null);
  const onPointerMove = (e: React.PointerEvent<HTMLElement>) => {
    if (!s) return; const r = e.currentTarget.getBoundingClientRect();
    setI(Math.min(s.count - 1, Math.max(0, Math.floor(((e.clientX - r.left) / r.width) * s.count))));
  };
  const style = s && i != null ? {
    backgroundImage: `url("${mediaUrl('sprite', clipId, s.v)}")`,
    backgroundSize: `${s.cols * 100}% ${s.rows * 100}%`,
    backgroundPosition: `${s.cols > 1 ? ((i % s.cols) / (s.cols - 1)) * 100 : 0}% ${s.rows > 1 ? (Math.floor(i / s.cols) / (s.rows - 1)) * 100 : 0}%`,
  } : undefined;
  return { style, onPointerMove, onPointerLeave: () => setI(null), timeMs: i != null && s ? i * s.intervalMs : null };
}
```

- Preload the sprite on `pointerenter` with `new Image().src = …`.
- The state changes only when the tile index changes, so there is no rAF loop. That matters on the 360 Hz panel.
- Show a thin progress line and the time under the cursor.

### 4.5 Storage meter

- The engine pushes `storage.status`: library, exports, cache, other-used and free bytes for the library drive, the cleanup cap, and the minimum free space.
- Render it as a segmented bar with `role="meter"`, `aria-valuenow`, `aria-valuemax` and `aria-valuetext="120 GB of 500 GB used"`. Add markers for the cap and the minimum-free threshold.
- **Units:** Explorer shows 1024-based sizes labelled "GB". Match it: compute with 1024³ and format with `Intl.NumberFormat(locale, { maximumFractionDigits: 1 })` plus the localized unit string.
- Export presets still compare **exact bytes**: Discord ≤ 19,000,000.

### 4.6 Theme, chrome, motion

- **Window chrome.** Keep the native title bar (`theme: "Dark"`), so Snap Layouts, Narrator and keyboard handling keep working. A custom title bar would need `data-tauri-drag-region` plus `core:window:allow-start-dragging`, and loses the Snap Layout flyout. Defer it.
- **No white flash.** Set `<html class="dark" style="background:#0B0C10">` and the same `backgroundColor` in the config. The window starts hidden; call `getCurrentWindow().show()` after the first paint:

```ts
// src/main.tsx (end)
root.render(<App />);
if (isTauri()) requestAnimationFrame(() => requestAnimationFrame(() => { void getCurrentWindow().show(); }));
```

- **Tailwind v4.** Use `@import "tailwindcss"; @import "tw-animate-css"; @custom-variant dark (&:where(.dark, .dark *));`, shadcn's generated tokens, and `@theme { --font-sans: "Inter Variable", "Segoe UI Variable", system-ui, sans-serif; }`.
  - Canvas libraries can't read CSS variables. Resolve them first: `getComputedStyle(document.documentElement).getPropertyValue('--wave')`.
- **Motion.** Keep it subtle: card hover lift, dialog fade, the saved-clip toast. Under `MotionConfig reducedMotion="user"` it follows the Windows "Animation effects" setting through `prefers-reduced-motion`.

---

## 5. Player, trimmer, 9:16 editor, exports

### 5.1 Player

- Use a native `<video preload="metadata" playsInline poster={mediaUrl('poster', id, v)}>` with custom controls built from shadcn/Radix parts. Video.js 10 and Vidstack aren't needed for one local file.
- **Shortcuts** (react-hotkeys-hook):
  - Space/K: play or pause;
  - J/L: ±5 s;
  - ←/→: ±1 frame when paused, ±1 s when playing;
  - I/O: set in and out points;
  - Home/End;
  - M: mute;
  - `,`/`.`: frame step.
- Drive the playhead with `video.requestVideoFrameCallback`. It fires per *video* frame, not per 360 Hz display refresh. Draw once more on `seeked`, because rVFC doesn't fire while paused.
- Before deleting the current clip, stop playback: `video.removeAttribute('src'); video.load()`.

### 5.2 Trim timeline: filmstrip + waveform + range slider

```tsx
import { Slider } from 'radix-ui';
import WaveSurfer from 'wavesurfer.js';

function Waveform({ media, peaks, durationSec, colors }: { media: HTMLVideoElement | null; peaks: number[]; durationSec: number; colors: { wave: string; played: string } }) {
  const el = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!media || !el.current) return;
    const ws = WaveSurfer.create({ container: el.current, media, peaks: [peaks], duration: durationSec,
      height: 40, normalize: true, interact: true, dragToSeek: true, cursorWidth: 0,
      waveColor: colors.wave, progressColor: colors.played });        // resolved colours, not var(--x)
    return () => ws.destroy();                                          // v8: destroy() is final; remount = new instance
  }, [media, peaks, durationSec, colors]);
  return <div ref={el} aria-hidden="true" />;                           // the slider is the accessible control
}

export function TrimBar({ durationMs, fps, value, onChange, keyframesMs, mode }: TrimProps) {
  const snapIn = (ms: number) => mode === 'quick' ? Math.max(0, ...keyframesMs.filter((k) => k <= ms)) : ms;
  return (
    <div className="relative h-24 select-none">
      <Filmstrip /> {/* the same sprite, N tiles across the width */}
      <Slider.Root className="absolute inset-0" min={0} max={durationMs}
        step={mode === 'quick' ? 1000 : Math.max(1, Math.round(1000 / fps))}
        minStepsBetweenThumbs={1} preserveThumbOrder value={value}
        onValueChange={onChange} onValueCommit={([a, b]) => onChange([snapIn(a), b])}>
        <Slider.Track><Slider.Range /></Slider.Track>
        <Slider.Thumb aria-label={t('trim.start')} aria-valuetext={fmtTime(value[0])} />
        <Slider.Thumb aria-label={t('trim.end')} aria-valuetext={fmtTime(value[1])} />
      </Slider.Root>
    </div>
  );
}
```

- **wavesurfer.js 8.0.3** (BSD-3-Clause; v8.0.0 shipped 2026-09-23):
  - Pass `peaks` + `duration`. In `loadAudio`, with `channelData` present it skips `Fetcher.fetchBlob`. Without peaks it would fetch the whole 500 MB MP4 and decode it in Web Audio.
  - With `media` passed, the initial URL is `media.currentSrc || media.src`, and `setSrc` returns early when the URL is unchanged, so the video doesn't reload.
  - v8 breaking changes that affect Kairo: `destroy()` is final; a superseded `load()` rejects with `AbortError`; deep imports need `.js` (`wavesurfer.js/dist/plugins/regions.js`).
  - It renders inside a Shadow DOM with a `<style>` element. Either set its `cspNonce` option or use the `style-src` change in §1.5.
- **Peaks come from the engine** [Decision]: `waveform.peaks { clipId, track: 'mix', perSecond: 50 }`. For a 120 s clip that is 6,000 floats, computed once and cached next to the sprite.
- **Radix Slider 1.6.1:**
  - `value: number[]`, `onValueChange`, `onValueCommit`, `minStepsBetweenThumbs`, `step`, and the new `preserveThumbOrder` (thumbs can't cross).
  - With two thumbs, the default thumb labels are the English strings "Minimum" and "Maximum" (`getLabel`). Always pass translated `aria-label` and `aria-valuetext`; the thumb spreads `...thumbProps`.
- **Quick vs Precise.**
  - Quick (lossless) snaps the in-point to the last keyframe at or before it (`clip.keyframes`; 1 s GOP) and shows a "lossless" badge.
  - Precise re-encodes and is frame-accurate. It is the default for exports (brief §7.2).
  - The engine has the final say: show the in/out it reports back from `export.estimate`.

### 5.3 9:16 editor

- **Presets** (filter graphs in report 06 §2.2):
  - *Centre crop*: one 9:16 rectangle.
  - *Fit + blurred background*: no rectangle; optional zoom.
  - *Facecam on top*: two rectangles. The facecam slot is 1080×608, gameplay 1080×1312, so aspects are `1080/608` and `1080/1312`. Remember the facecam rectangle per game.
- **Crop box.** Use `react-image-crop` 11.1.2 wrapping the `<video>`: `aspect`, `ruleOfThirds`, `locked`, and `onChange(pixelCrop, percentCrop)`; `centerCrop` and `makeAspectCrop` set the defaults. It edits one rectangle at a time, so for the facecam preset add tabs: "Facecam / Gameplay".
  - Store **percent** crops, which survive resizing.
  - Convert to source pixels in the engine and round to **even** values, because 4:2:0 needs even width, height and offsets.
- **Live preview.** A 9:16 `<canvas>` (about 360×640 CSS px × DPR) is drawn from the same `<video>` with `requestVideoFrameCallback`:
  - blurred background: `ctx.filter = 'blur(16px)'`, drawn as cover, then `ctx.filter = 'none'` and the foreground drawn as fit;
  - facecam: two `drawImage(video, sx, sy, sw, sh, dx, dy, dw, dh)` calls.
  - One decode serves both the editor and the preview. The canvas is tainted, which is fine for display.

### 5.4 Exports and progress

1. Call `export.estimate(spec)` and show the predicted size and resolution. Presets live in the engine: Discord 19 MB, 50 MB, 500 MB; WhatsApp; TikTok/Shorts/Reels; Original.
2. Call `export.start({ ...spec, requestId })` to get `{ jobId }`.
3. The engine sends `export.progress { jobId, stage: queued|encoding|muxing|verifying|done|failed|cancelled, progress 0..1, etaMs?, outputPath?, bytes? }`, at most about 10 Hz. Upsert it into `useJobs`.
4. While a job runs, set the taskbar progress: `getCurrentWindow().setProgressBar({ status: ProgressBarStatus.Normal, progress: Math.round(p * 100) })`, then `ProgressBarStatus.None` when it ends. This needs `core:window:allow-set-progress-bar` and is verified in 2.12.4 (`Window::set_progress_bar`).
5. When it's done, offer **Copy** (§6.1), **Show in folder** (Rust `app.opener().reveal_item_in_dir(path)`), and a **drag handle** (§6.2).
   - Audio defaults for share copies: mix only, voice chat excluded.
   - Closing the UI mid-export is fine: the engine keeps working, and `jobs.list` repopulates the UI on reopen.

---

## 6. Clipboard and drag-out

### 6.1 Put the video file on the clipboard (CF_HDROP + Preferred DropEffect)

`tauri-plugin-clipboard-manager` 2.5.0 only does text, image and HTML. The CrossCopy community plugin was last pushed 2024-10-17. So write it yourself with `windows` 0.62.2. Every signature below was verified in the crate source:
- `OpenClipboard(Option<HWND>) -> Result<()>`
- `SetClipboardData(u32, Option<HANDLE>) -> Result<HANDLE>`
- `GlobalAlloc(GLOBAL_ALLOC_FLAGS, usize) -> Result<HGLOBAL>`
- `GlobalLock(HGLOBAL) -> *mut c_void`
- `GlobalUnlock(HGLOBAL) -> Result<()>`
- `GlobalFree(Option<HGLOBAL>)`
- `CF_HDROP: CLIPBOARD_FORMAT(15)` in `Win32::System::Ole`
- `DROPFILES` is `#[repr(C, packed(1))]` in `Win32::UI::Shell`
- `CFSTR_PREFERREDDROPEFFECT = w!("Preferred DropEffect")`
- `DROPEFFECT_COPY = DROPEFFECT(1)`

```rust
#[cfg(windows)]
pub fn set_file_drop(hwnd: windows::Win32::Foundation::HWND, paths: &[std::path::PathBuf]) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::{Foundation::{GlobalFree, HANDLE, POINT},          // GlobalFree lives in Foundation in 0.62
        System::{DataExchange::*, Memory::*, Ole::{CF_HDROP, DROPEFFECT_COPY}}, UI::Shell::{CFSTR_PREFERREDDROPEFFECT, DROPFILES}};
    let mut list: Vec<u16> = Vec::new();
    for p in paths { list.extend(p.as_os_str().encode_wide()); list.push(0); }  // absolute, no \\?\ prefix
    list.push(0);                                                               // double NUL terminator
    let hdr = std::mem::size_of::<DROPFILES>();                                 // 20
    unsafe {
        let hdrop = GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, hdr + list.len() * 2).map_err(|e| e.to_string())?;
        let base = GlobalLock(hdrop) as *mut u8;
        if base.is_null() { let _ = GlobalFree(Some(hdrop)); return Err("GlobalLock".into()); }
        std::ptr::write_unaligned(base as *mut DROPFILES,
            DROPFILES { pFiles: hdr as u32, pt: POINT { x: 0, y: 0 }, fNC: false.into(), fWide: true.into() });
        std::ptr::copy_nonoverlapping(list.as_ptr() as *const u8, base.add(hdr), list.len() * 2);
        let _ = GlobalUnlock(hdrop);   // ← returns Err(code 0) when the lock count reaches zero: that's success

        let effect = GlobalAlloc(GMEM_MOVEABLE, 4).map_err(|e| e.to_string())?;
        *(GlobalLock(effect) as *mut u32) = DROPEFFECT_COPY.0;
        let _ = GlobalUnlock(effect);
        let cf_effect = RegisterClipboardFormatW(CFSTR_PREFERREDDROPEFFECT);

        let mut opened = false;                                   // another app may hold the clipboard
        for _ in 0..10 { if OpenClipboard(Some(hwnd)).is_ok() { opened = true; break } std::thread::sleep(std::time::Duration::from_millis(20)); }
        if !opened { let _ = GlobalFree(Some(hdrop)); let _ = GlobalFree(Some(effect)); return Err("clipboard busy".into()); }
        let r = (|| -> Result<(), String> {
            EmptyClipboard().map_err(|e| e.to_string())?;           // makes hwnd the owner
            if SetClipboardData(CF_HDROP.0 as u32, Some(HANDLE(hdrop.0))).is_err() { let _ = GlobalFree(Some(hdrop)); return Err("SetClipboardData(CF_HDROP)".into()); }
            if SetClipboardData(cf_effect, Some(HANDLE(effect.0))).is_err() { let _ = GlobalFree(Some(effect)); }
            Ok(())                                                  // after success the system owns the memory: never free it
        })();
        let _ = CloseClipboard();
        r
    }
}
```

The command marshals the call to the UI thread. The window must own the clipboard, and `HWND` is `!Send`, so call `hwnd()` inside the closure:

```rust
#[tauri::command]
pub async fn clip_copy(app: tauri::AppHandle, clip_id: String) -> Result<(), String> {
    let path = app.state::<MediaResolver>().resolve(&app, Kind::Video, &clip_id).await.map_err(|e| e.to_string())?;
    let win = app.get_webview_window("main").ok_or("no window")?;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.run_on_main_thread(move || {
        let r = win.hwnd().map_err(|e| e.to_string()).and_then(|h| set_file_drop(h, &[path]));
        let _ = tx.send(r);
    }).map_err(|e| e.to_string())?;
    rx.await.map_err(|e| e.to_string())?
}
```

Pitfalls:
- **Pass an owner window.** Microsoft documents that with a NULL owner, `EmptyClipboard` sets the owner to NULL "and causes `SetClipboardData` to fail". `window.hwnd()` returns the windows 0.62 `HWND` in Tauri 2.12, so the types match.
- **The data survives the UI exiting.** It is rendered immediately, not delay-rendered. Important, because the UI closes completely.
- **Never put a temp file on the clipboard.** "Copy for Discord" first exports a ≤19,000,000-byte file into `Videos\Kairo\_Exports` and puts *that* path on the clipboard. The file must still exist when the user pastes.
- **No `\\?\` paths.** Don't `canonicalize()` without `dunce::simplified`. arboard does this right with `GetFinalPathNameByHandleW` + `PathCchStripPrefix`.
- **Will Discord and WhatsApp accept it?** Chromium reads `CF_HDROP` first (report 06), and Discord and the current WhatsApp for Windows are Chromium-based. **[Unverified]** Test Ctrl+V into Discord, WhatsApp desktop and Explorer on the PC.
  - Cloud clipboard and Win+V history don't sync file lists.
- **Alternative without unsafe code:** arboard 3.7.0 (released 2026-10-15) added `Clipboard::new()?.set().file_list(&[path])`. It writes `CF_HDROP` on Windows and file URLs on macOS, but **no** Preferred DropEffect. Use it for the Mac dev build, and on Windows if the hand-written version gives trouble.
- **Test on the PC:** run `powershell.exe -NoProfile -Command "Get-Clipboard -Format FileDropList | % FullName"` *in the interactive session*.
  - `-Format FileDropList` exists only in Windows PowerShell 5.1; PowerShell 7's `Get-Clipboard` is text-only.
  - Read Preferred DropEffect with `[Windows.Forms.Clipboard]::GetData('Preferred DropEffect')` (STA).

### 6.2 Drag-out

**Status of `tauri-plugin-drag`** (CrabNebula drag-rs):
- Versions: crate 2.1.1 (2026-05-01), npm `@crabnebula/tauri-plugin-drag` 2.1.0 (2025-02-24). The repo was last pushed 2026-10-08, with 21 open issues.
- API: `startDrag({ item: string[], icon: string /* path or data:image/png;base64 */, mode?: 'copy'|'move' }, onEvent?)` invokes `plugin:drag|start_drag`. The Rust side runs `drag::start_drag` inside `app.run_on_main_thread`. On Windows that means `OleInitialize` + `DoDragDrop` with a shell-item data object, which provides CF_HDROP, and a WIC-decoded drag image.
- Permission: `drag:allow-start-drag`.
- It works on macOS too, which helps UI development on the Mac.
- Known problems:
  - The Windows backend depends on **`windows` 0.52** and **`windows-core` 0.58**, so two extra copies get compiled.
  - Open issues:
    - #72: panic on UNC/NAS paths;
    - #76: crash on paths over 260 characters (`ILCreateFromPathW`);
    - #94: a `DoDragDrop` that never returns wedges the whole app; PR #95 adds a timeout and is still open;
    - #41: the drag image is shown at native size;
    - #77: self-drops raise events late.

**Usage rules:**

```tsx
// Start the OS drag on movement with the button held; Chromium's own HTML5 drag must not start.
function DragHandle({ path, thumbPath }: { path: string; thumbPath: string }) {
  const start = useRef<{ x: number; y: number } | null>(null);
  return (
    <button draggable={false} onDragStart={(e) => e.preventDefault()}
      onPointerDown={(e) => { start.current = { x: e.clientX, y: e.clientY }; }}
      onPointerMove={(e) => {
        const s = start.current; if (!s || e.buttons !== 1 || Math.hypot(e.clientX - s.x, e.clientY - s.y) < 6) return;
        start.current = null; useUi.getState().setDraggingOut(true);              // the import overlay ignores our own drag
        void startDrag({ item: [path], icon: thumbPath }).finally(() => useUi.getState().setDraggingOut(false));
      }}
      onPointerUp={() => { start.current = null; }}>{t('clip.drag')}</button>);
}
```

- Pass a **small** thumbnail (160×90 JPEG; WIC decodes it) as `icon`.
- Only pass paths that are local, absolute, under 260 characters and that exist. Kairo's naming keeps game folder names short (≤64 characters).
- `DoDragDrop` is modal on the main thread. Engine events queue up until the drop.

**Fallback (Windows, about 15 lines, on the shared `windows` 0.62).** `SHDoDragDrop` is documented: if `pdsrc` is NULL "the Shell creates a drop source object for you", and it provides a generic drag image when none is set.

```rust
#[cfg(windows)]
unsafe fn drag_file(hwnd: windows::Win32::Foundation::HWND, path: &std::path::Path) -> windows::core::Result<windows::Win32::System::Ole::DROPEFFECT> {
    use windows::{core::HSTRING, Win32::{System::{Com::{IBindCtx, IDataObject}, Ole::{IDropSource, DROPEFFECT_COPY}},
        UI::Shell::{BHID_DataObject, IShellItem, SHCreateItemFromParsingName, SHDoDragDrop}}};
    let item: IShellItem = SHCreateItemFromParsingName(&HSTRING::from(path.as_os_str()), None::<&IBindCtx>)?;
    let data: IDataObject = item.BindToHandler(None::<&IBindCtx>, &BHID_DataObject)?;
    SHDoDragDrop(Some(hwnd), &data, None::<&IDropSource>, DROPEFFECT_COPY)   // modal; run inside run_on_main_thread
}
```

- The signatures come from the 0.62.2 source: `SHDoDragDrop(Option<HWND>, P1: Param<IDataObject>, P2: Param<IDropSource>, DROPEFFECT) -> Result<DROPEFFECT>` and `IShellItem::BindToHandler<P0, T>(&self, pbc, bhid: *const GUID) -> Result<T>`.
- `None::<&IDropSource>` works: windows-core 0.62.2 `param.rs` has `impl<T> Param<T> for Option<&T>`, and `&HSTRING: Param<PCWSTR>` and `From<&OsStr> for HSTRING` exist too. Compile-check the sketch with `cargo xwin check` anyway.
- `DRAGDROP_S_CANCEL` is a success HRESULT, so check the returned effect: `DROPEFFECT_NONE` means cancelled.

**Other drag-and-drop notes:**
- `dragDropEnabled: true` (the default) is what delivers "drop files here to import" through `getCurrentWebview().onDragDropEvent(e => e.payload.type === 'drop' && e.payload.paths)`. It also **disables HTML5 drag-and-drop inside the page on Windows**: the config schema says Tauri "replace[s] the drag drop handler of WebView2". Use pointer-based DnD (for example dnd-kit's pointer sensor) for any in-app dragging.
- Keep the UI unelevated, because UIPI blocks dragging into elevated apps (report 06).

---

## 7. Mock engine and testing

### 7.1 Mock engine (Mac and Windows)

`crates/kairo-mock-engine` uses the same `kairo-protocol` crate, `interprocess` 2.4.6 and `LengthDelimitedCodec`:

```rust
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    use futures_util::{SinkExt, StreamExt};
    use interprocess::local_socket::{tokio::{prelude::*, Stream}, GenericFilePath, GenericNamespaced, ListenerOptions};
    let name = if cfg!(windows) { kairo_protocol::pipe_name()?.to_ns_name::<GenericNamespaced>()? }
               else { format!("/tmp/kairo-{}.sock", whoami_uid()).to_fs_name::<GenericFilePath>()? };
    let listener = ListenerOptions::new().name(name).try_overwrite(true).create_tokio()?;   // reclaims a stale socket
    let lib = Fixtures::load("fixtures")?;                  // *.mp4 + *.poster.jpg + *.sprite.jpg + peaks.json
    loop {
        let conn: Stream = listener.accept().await?;
        let lib = lib.clone();
        tokio::spawn(async move {
            let mut io = tokio_util::codec::Framed::new(conn, tokio_util::codec::LengthDelimitedCodec::new());
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(1));
            loop { tokio::select! {
                _ = tick.tick() => io.send(lib.notification_buffer_status()).await?,
                f = io.next() => match f { Some(Ok(b)) => io.send(lib.handle(&b).await).await?, _ => break },
            }}
            anyhow::Ok(())
        });
    }
}
```

- **Scenarios** are flags that exercise every UI state:
  - `--slow-ms 800`, `--crash-after 30s`, `--protocol 0` (incompatible), `--no-clips`, `--clips 5000` (virtualization), `--export-fails`;
  - `debug.emit { method, params }`, which exists in the mock only.
- **Browser-only mode** is the fastest loop on 8 GB: `VITE_MOCK=1 pnpm dev` calls `mockIPC(handler, { shouldMockEvents: true })`, `mockWindows('main')` and `mockConvertFileSrc('macos')`, and serves `/fixtures` from Vite.
- **Fixture script** (macOS, VideoToolbox; never shipped):

```sh
out=fixtures; mkdir -p "$out"
ffmpeg -y -f lavfi -i "testsrc2=size=2560x1440:rate=60" -f lavfi -i "sine=frequency=440:sample_rate=48000" \
  -f lavfi -i "sine=frequency=660:sample_rate=48000" -t 30 -map 0:v -map 1:a -map 1:a -map 2:a \
  -c:v h264_videotoolbox -b:v 25M -g 60 -pix_fmt yuv420p -c:a aac -b:a 160k -ar 48000 \
  -movflags +faststart "$out/clip-30s.mp4"                               # 1 s GOP, mix + game + mic tracks
ffmpeg -y -ss 1 -i "$out/clip-30s.mp4" -frames:v 1 -vf scale=640:-2 "$out/clip-30s.poster.jpg"
ffmpeg -y -skip_frame nokey -i "$out/clip-30s.mp4" -vf "scale=160:90,tile=10x3" -fps_mode vfr -frames:v 1 "$out/clip-30s.sprite.jpg"
```

### 7.2 Unit and component tests

- **Stack:**
  - Vitest **5.0.5**: needs Node ≥22 and Vite ≥6.4; mocks are cleared before each test; an un-awaited async assertion fails; config files are no longer looked up in parent directories.
  - `@testing-library/react` 16.3.3, `@testing-library/dom` 10.4.2, `@testing-library/user-event` 14.6.8, `@testing-library/jest-dom` 7.0.1 (`import '@testing-library/jest-dom/vitest'`).
  - `jsdom` 30.1.3 for logic.
  - **Browser mode** for layout-dependent components (virtual grid, slider drag, canvas): `@vitest/browser-playwright` 5.0.5 + `playwright` 1.64.1 + `vitest-browser-react` 2.3.0.
- **Tauri mocks** (`@tauri-apps/api/mocks`, 2.12.4): `mockIPC(cb, { shouldMockEvents: true })` (event mocking since 2.7.0; `emitTo` isn't supported), `mockWindows('main')`, `mockConvertFileSrc('windows')`, `clearMocks()`.

```ts
import { mockIPC, mockWindows, clearMocks } from '@tauri-apps/api/mocks';
import { emit } from '@tauri-apps/api/event';
afterEach(() => clearMocks());
test('clip.saved refreshes the grid and toasts', async () => {
  mockWindows('main');
  let list = page([clipA]);
  mockIPC((cmd, args) => {
    if (cmd === 'engine_state') return { state: 'connected', engineVersion: '0.1.0' };
    if (cmd === 'engine_call' && (args as any).method === 'library.list') return list;
  }, { shouldMockEvents: true });
  render(<App />);
  expect(await screen.findByText(clipA.title)).toBeInTheDocument();
  list = page([clipB, clipA]);
  await emit('engine:notification', { method: 'clip.saved', params: clipB });
  expect(await screen.findByText(clipB.title)).toBeInTheDocument();
});
```

- **Pure-logic tests:**
  - sprite index and background-position maths (cols = 1 and rows = 1 edge cases);
  - keyframe snapping;
  - even-pixel crop conversion;
  - size formatting against Explorer values;
  - i18n key parity across locales;
  - the export-size fit formula (brief §7.4).
- **Accessibility smoke test:** `axe-core` 4.14.0 (MPL-2.0, dev only) on the main screens in browser mode.

### 7.3 Rust tests

- **Media protocol** (build `http::Request`s and call `serve`):
  - `bytes=0-` → 206, `Content-Length` = 2 MiB, `Content-Range: bytes 0-2097151/N`;
  - `bytes=N-10-` → the tail;
  - beyond EOF → 416 with `bytes */N`;
  - no Range on a big file → 416, so RAM is never touched;
  - HEAD → no body;
  - a bad ULID → error;
  - a percent-encoded path.
- **EngineClient**, over `tokio::io::duplex`: request/response correlation, a timeout that removes the pending entry, a notification that emits an event (mock `AppHandle` via `tauri::test::mock_app`, which needs `tauri = { features = ["test"] }` in `[dev-dependencies]`), and "disconnect fails every pending call".
- **DROPFILES layout**, as a pure function that returns the buffer: `pFiles = 20`, `fWide = 1`, UTF-16LE, double NUL.
- **Contract:** `cargo test -p kairo-protocol` + `git diff --exit-code apps/ui/src/gen`.

### 7.4 E2E

- **Recommended:** WebdriverIO **9.32.0** (`@wdio/cli`, `local-runner`, `mocha-framework`, `spec-reporter` 9.32.0) + **`@wdio/tauri-service` 1.5.0** (peer `webdriverio ^9`) + **`tauri-plugin-wdio-webdriver` 1.5.0**.
  - The service starts the app with `TAURI_WEBDRIVER_PORT`, and the plugin serves W3C WebDriver from inside the app.
  - Its ADR-0002 (2026-08-03) makes this embedded provider the default. It works on macOS (WKWebView) and on WebView2.
  - It "stayed green" when a WebView2 bump to 150 broke the external-driver CDP path.
  - Its README warns: "Never include it in production builds". Hence the `e2e` feature and the inline capability in §1.6.

```ts
// wdio.conf.ts
export const config: WebdriverIO.Config = {
  runner: 'local', specs: ['./e2e/**/*.e2e.ts'], maxInstances: 1, framework: 'mocha',
  services: [['@wdio/tauri-service', { driverProvider: 'embedded' }]],
  capabilities: [{ browserName: 'tauri', 'tauri:options': { application: '../src-tauri/target/release/kairo-ui' } }],
};
// e2e/seek.e2e.ts (the PC, real engine)
const ms = await browser.execute(async () => {
  const v = document.querySelector('video')!; const out: number[] = [];
  for (const f of [0.9, 0.1, 0.5, 0.75, 0.25, 0.95, 0.05, 0.6, 0.3, 0.8]) {
    const t0 = performance.now(); v.currentTime = f * v.duration;
    await new Promise((r) => v.addEventListener('seeked', r, { once: true })); out.push(performance.now() - t0);
  }
  return out;
});
```

- **Alternatives:**
  - `tauri-driver` 2.1.0 (`cargo install tauri-driver --locked`): Windows (msedgedriver) and Linux only. msedgedriver must match the WebView2 version, which auto-updates.
  - Playwright 1.64.1 over CDP: launch Kairo with `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222`, then `chromium.connectOverCDP('http://localhost:9222')` → `browser.contexts()[0].pages()[0]`.
  - Setting that variable replaces wry's default `--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection`, which is fine in tests.
- **On the PC:** start E2E inside the interactive session (scheduled task with `/it`) and write JUnit to a file. Then run the PowerShell checks: WebView2 exit (§1.8), clipboard contents (§6.1), engine still alive.

### 7.5 Manual matrix on the PC (one pass per beta build)

| Area | Cases |
|---|---|
| Paste | Ctrl+V of a Copy into Discord DM/channel, WhatsApp desktop, Explorer (copy, not move), Telegram |
| Drag | To Discord, WhatsApp, Explorer, Premiere/Resolve bins; a drag cancelled with Esc; a drag onto Kairo itself |
| Playback | 30 s and 600 MB H.264; HEVC with and without the extension; AV1; on the 360 Hz HDR OLED with HDR on and off (SDR mapping, banding); on the 180 Hz second monitor |
| Lifecycle | Close during playback or export; open from the tray while open (single instance + focus); open via the toast `--open clip:<id>` |
| A11y | Keyboard only through library → trim → export; Narrator and NVDA; Windows contrast themes (`forced-colors`); 200% zoom; "Animation effects" off |
| States | Engine stopped, crashed mid-export, elevated, older protocol |

---

## 8. i18n and accessibility basics

- **i18next 26 + react-i18next 17:**

```ts
import i18n from 'i18next'; import { initReactI18next } from 'react-i18next'; import en from '@/locales/en/common.json';
void i18n.use(initReactI18next).init({ resources: { en: { common: en } }, lng: navigator.language, fallbackLng: 'en',
  defaultNS: 'common', interpolation: { escapeValue: false } });
declare module 'i18next' { interface CustomTypeOptions { defaultNS: 'common'; resources: { common: typeof en } } }
```

  - Plurals use `count` with `_one/_other` keys.
  - Never concatenate translated fragments; use `<Trans>` for embedded markup.
  - Dates and numbers go through `Intl.DateTimeFormat`, `RelativeTimeFormat` and `NumberFormat`. Durations are `m:ss` with tabular figures.
  - Use logical CSS (Tailwind `ms-*`, `me-*`, `ps-*`, `pe-*`) so RTL stays possible.
- **Lint:** ESLint **9.39.5**, the maintenance line. `eslint-plugin-jsx-a11y` 6.10.2 still peers ESLint ≤9; @eslint/js 10 needs ESLint 10. Add:
  - `typescript-eslint` 8.72.0;
  - `eslint-plugin-react-hooks` 7.1.1;
  - `eslint-plugin-react-refresh` 0.5.7;
  - `eslint-plugin-jsx-a11y` 6.10.2;
  - `eslint-plugin-i18next` 6.1.5 (`no-literal-string` in JSX);
  - Prettier 3.9.10 + `prettier-plugin-tailwindcss` 0.8.1.
  - Biome 2.6.1 would pair with TS 7, but it has no literal-string rule.
- **A11y checklist:**
  - Radix primitives for focus traps and ARIA;
  - visible `:focus-visible` rings;
  - 4.5:1 text contrast on the dark tokens;
  - `aria-live` for toasts (sonner) and export progress;
  - translated slider labels (§5.2);
  - meters with `role="meter"`;
  - every hover-only affordance (scrub, context menu) has a keyboard path;
  - `zoomHotkeysEnabled: true` with a layout that survives 200%;
  - test `@media (forced-colors: active)`.

---

## 9. Recommended packages (copy-paste)

**npm (apps/ui):**
- **Dependencies:** react@19.3.1 react-dom@19.3.1 @tauri-apps/api@2.12.4 @tauri-apps/plugin-dialog@2.8.3 @crabnebula/tauri-plugin-drag@2.1.0 @tanstack/react-query@5.104.5 @tanstack/react-virtual@3.14.21 @tanstack/react-router@1.170.45 zustand@5.0.15 radix-ui@1.8.1 class-variance-authority@0.7.1 clsx@2.1.1 tailwind-merge@3.7.0 lucide-react@1.61.0 motion@14.5.0 @fontsource-variable/inter@5.3.0 wavesurfer.js@8.0.3 react-image-crop@11.1.2 i18next@26.5.4 react-i18next@17.1.0 react-hotkeys-hook@5.3.3 react-error-boundary@6.1.6 sonner@2.0.8
- **Dev dependencies:** typescript@6.0.3 @types/react@19.3.0 @types/react-dom@19.3.0 @types/node@^24 vite@8.3.5 @vitejs/plugin-react@6.1.2 tailwindcss@4.3.3 @tailwindcss/vite@4.3.3 tw-animate-css@1.4.0 @tauri-apps/cli@2.12.4 vitest@5.0.5 @vitest/coverage-v8@5.0.5 @vitest/browser-playwright@5.0.5 playwright@1.64.1 vitest-browser-react@2.3.0 jsdom@30.1.3 @testing-library/react@16.3.3 @testing-library/dom@10.4.2 @testing-library/user-event@14.6.8 @testing-library/jest-dom@7.0.1 axe-core@4.14.0 eslint@9.39.5 @eslint/js@9.39.5 typescript-eslint@8.72.0 eslint-plugin-react-hooks@7.1.1 eslint-plugin-react-refresh@0.5.7 eslint-plugin-jsx-a11y@6.10.2 eslint-plugin-i18next@6.1.5 globals@17.13.0 prettier@3.9.10 prettier-plugin-tailwindcss@0.8.1 webdriverio@9.32.0 @wdio/cli@9.32.0 @wdio/local-runner@9.32.0 @wdio/mocha-framework@9.32.0 @wdio/spec-reporter@9.32.0 @wdio/tauri-service@1.5.0

**Cargo:**
- The shell crate (§1.4).
- **kairo-protocol:** serde 1.0.229, serde_json 1.0.154, thiserror 2.0.21, ts-rs 12.0.1 (`serde-json-impl`).
- **kairo-mock-engine:** tokio 1.54 (`rt-multi-thread`, `macros`, `time`), interprocess 2.4.6 (`tokio`), tokio-util 0.7.20 (`codec`), futures-util 0.3.34, anyhow 1.0.104.
- **Tool:** tauri-driver 2.1.0, optional.

---

## 10. Gotchas

1. **TypeScript 7.0.2 is npm's "latest"**, but typescript-eslint 8.72.0 needs `<6.1.0`, and 7.0 has no stable programmatic API. Pin 6.0.3, as create-tauri-app 4.7.5 does.
2. **Never install `@next`**: it resolves to Tauri 3.0 alphas for every `@tauri-apps/*` package. `cargo add` ignores pre-releases; pnpm does not when you name the tag.
3. **Single-instance** is Rust-only (`@tauri-apps/plugin-single-instance` returns 404). Register it first. Its mutex name comes from the identifier, so choose the identifier once.
4. **Exit discipline.** A hidden window keeps the process alive (tauri#5611). Don't call `prevent_exit`, don't add a tray icon to the UI, don't enable `unstable` (multi-webview).
5. **window-state + `visible: false`:** drop `StateFlags::VISIBLE`, then show after the first paint. Add a 3 s Rust fallback so a broken bundle never leaves an invisible window.
6. **Event names** may contain only `[A-Za-z0-9-/:_]`. Use `engine:notification` and keep the method in the payload.
7. **App commands** are reachable by the webview unless `AppManifest::commands` is set. Set it, and grant `allow-…` per command.
8. **Async commands taking `State<'_>` must return `Result`.** The error type must be `Serialize`.
9. **The asset protocol** reads whole files for Range-less GETs (tauri#4133 is open), and every custom-protocol body is copied into memory (`SHCreateMemStream`). Cap chunks, and never answer a Range-less request for a clip with the file.
10. **Partial content works only on `http(s)` URLs** in WebView2 (Microsoft, #3230). Tauri's `http://<scheme>.localhost` mapping is what makes it work. `useHttpsScheme` keeps HTTP semantics but changes the origin: pick one value before the beta, because localStorage is per-origin.
11. **`convertFileSrc` percent-encodes `/`.** Decode in the handler.
12. **CSP:**
    - Without `connect-src ipc: http://ipc.localhost`, IPC breaks.
    - Media needs both `kairo-media:` and `http://kairo-media.localhost`.
    - Tauri's style nonces silently disable `'unsafe-inline'`, which breaks wavesurfer's Shadow DOM `<style>` and sonner. Use `dangerousDisableAssetCspModification: ["style-src"]`.
13. **A loopback HTTP server** will be gated by WebView2 Local Network Access once Microsoft enables it ("API-controlled", date TBD).
14. **wavesurfer 8:**
    - Without `peaks` + `duration`, it downloads and decodes the whole MP4.
    - `destroy()` is final.
    - Deep imports need `.js`.
    - Canvas colours can't be `var(--x)`.
15. **Radix Slider** labels two thumbs "Minimum" and "Maximum" in English unless you pass `aria-label`.
16. **Crops:** store percentages and make pixel sizes and offsets even (4:2:0).
17. **A cross-origin `<video>` drawn to a canvas** taints it. Display only; posters come from the engine.
18. **drag plugin:**
    - It pulls in `windows` 0.52 and `windows-core` 0.58.
    - It panics on UNC/NAS paths and paths over 260 characters.
    - It can hang the main thread (#94).
    - The drag image is native size.
    - Your own window receives the drag: ignore it in the import overlay.
19. **`dragDropEnabled: true`** (needed for import) disables HTML5 DnD inside the page on Windows. Use pointer-based DnD.
20. **Clipboard:**
    - `GlobalUnlock` returns `Err` with code 0 when it succeeds, in windows 0.62: ignore it.
    - Don't free memory after `SetClipboardData` succeeds.
    - Open with an owner HWND, on the UI thread.
    - No `\\?\` paths, no temp files.
21. **PowerShell 7's `Get-Clipboard`** has no `-Format FileDropList`. Use `powershell.exe` 5.1 in tests, in the interactive session.
22. **ts-rs** maps `u64`/`i64` to `bigint` by default. Set `TS_RS_LARGE_INT=number`. ts-rs 12's API takes `&Config` everywhere.
23. **eslint-plugin-jsx-a11y** 6.10.2 peers ESLint ≤9, so stay on ESLint 9.39.5.
24. **Vitest 5** clears mocks before each test, fails un-awaited `expect(...).resolves`, and doesn't look up configs in parent directories. **jsdom 30** needs Node ≥22.22.2 or ≥24.15.
25. **`mockIPC` with `shouldMockEvents`** swallows every `plugin:event|*` invoke and doesn't support `emitTo`.
26. **An engine started by the UI** must break away from job objects: `CREATE_BREAKAWAY_FROM_JOB`, falling back on `ERROR_ACCESS_DENIED`.
27. **Sizes:** Explorer's "GB" is GiB. Match it in the meter, but compute presets in exact bytes (Discord ≤ 19,000,000).
28. **360 Hz panel:** rAF loops run 360 times a second. Use rVFC and state-change-driven updates, and stop work when `document.visibilityState === 'hidden'`.
29. **Mouse XButton1/2** navigate WebView2 history, and the engine may use them as clip triggers. Define the rule.
30. **The E2E WebDriver plugin** opens an HTTP automation server. Use a feature flag, an inline E2E capability and a CI check that release builds don't contain the string `wdio`.
31. **Changing the identifier, `appDirectoriesOverride` or `useHttpsScheme` after release** loses the WebView2 UDF: localStorage, zoom and permissions. Settle all three before the beta.
32. **Debug builds only:** `getCurrentWindow().close()` and panicking async commands send stray HTTP requests to `localhost:80` (tauri#14359, open). Harmless, but don't run a dev server on port 80.

---

## 11. Still unverified or to measure (feeds spike S3)

- **To measure:**
  - WebView2 process exit timing after `Kairo.exe` exits (§1.8 script). Expected within seconds **[Estimate]**.
  - Seek p95 and dropped frames for 600 MB clips at 512 KiB to 4 MiB chunks, and whether virtual host mapping beats the chunked scheme (Microsoft docs vs the 2025 user report).
  - RAM: UI open vs closed, the sum of `msedgewebview2` private bytes (brief S3).
- **To test on the PC:**
  - Ctrl+V of a CF_HDROP file into Discord and WhatsApp desktop (inferred from Chromium source only).
  - H.264/AAC playback on Windows N without the Media Feature Pack (an FFmpeg fallback is documented for H.264; AAC not checked).
  - WKWebView (Mac dev) Range behaviour with the `kairo-media` scheme. Expected to work like Tauri's asset protocol does on macOS; not tested.
- **Not confirmed:**
  - Whether `generate_context!` needs `frontendDist` present during `cargo xwin check`.
  - `--webview-exe-name` on WebView2 command lines (a user-posted dump only).
  - Whether `freezePrototype: true` (2.12 option) breaks any chosen library. It isn't in the default config; try it in beta hardening.

---

## Sources

**Registries (versions read 2026-10-21)**
- crates.io API: https://crates.io/api/v1/crates/{tauri, tauri-build, tauri-cli, tauri-plugin-single-instance, tauri-plugin-updater, tauri-plugin-opener, tauri-plugin-dialog, tauri-plugin-window-state, tauri-plugin-process, tauri-plugin-log, tauri-plugin-clipboard-manager, tauri-plugin-drag, drag, tauri-driver, wry, tao, webview2-com, interprocess, ts-rs, specta, tauri-specta, windows, windows-core, arboard, clipboard-win, tokio, tokio-util, http-range, axum, tower-http, tauri-plugin-wdio-webdriver, tauri-plugin-sentry, tauri-plugin-clipboard}
- npm registry: https://registry.npmjs.org/ (all npm packages named above, incl. typescript, typescript-eslint peer range, vitest, jsdom engines, @wdio/tauri-service peers)
- Node releases: https://nodejs.org/dist/index.json

**Tauri (tag `tauri-v2.12.4`)**
- https://github.com/tauri-apps/tauri/blob/tauri-v2.12.4/crates/tauri/src/protocol/asset.rs
- https://github.com/tauri-apps/tauri/blob/tauri-v2.12.4/crates/tauri/CHANGELOG.md
- https://github.com/tauri-apps/tauri/blob/tauri-v2.12.4/crates/tauri/src/app.rs (async URI scheme, UriSchemeResponder/Context, RunEvent)
- https://github.com/tauri-apps/tauri/blob/tauri-v2.12.4/crates/tauri-runtime-wry/src/lib.rs (Destroyed → ExitRequested)
- https://github.com/tauri-apps/tauri/blob/tauri-v2.12.4/crates/tauri/src/webview/mod.rs (ACL check; PlatformWebview::controller)
- https://github.com/tauri-apps/tauri/blob/tauri-v2.12.4/crates/tauri/src/event/event_name.rs
- https://github.com/tauri-apps/tauri/blob/tauri-v2.12.4/crates/tauri-utils/src/acl/build.rs
- https://github.com/tauri-apps/tauri/blob/tauri-v2.12.4/crates/tauri-build/src/acl.rs and src/lib.rs
- https://github.com/tauri-apps/tauri/blob/tauri-v2.12.4/crates/tauri-schema-generator/schemas/config.schema.json
- https://github.com/tauri-apps/tauri/blob/tauri-v2.12.4/crates/tauri/Cargo.toml (features; windows 0.62)
- https://github.com/tauri-apps/tauri/blob/tauri-v2.12.4/crates/tauri/scripts/core.js (convertFileSrc)
- https://github.com/tauri-apps/tauri/tree/tauri-v2.12.4/packages/api/src (core.ts, mocks.ts, event.ts, webview.ts, window.ts)
- https://github.com/tauri-apps/tauri/blob/tauri-v2.12.4/crates/tauri-driver/README.md and CHANGELOG.md
- Issues: https://github.com/tauri-apps/tauri/issues/4133 · /5611 · /15624 · /14359
- Docs: https://v2.tauri.app/security/csp/ · https://v2.tauri.app/distribute/windows-installer/ · https://v2.tauri.app/develop/tests/webdriver/
- Plugins: https://github.com/tauri-apps/plugins-workspace/tree/v2/plugins (single-instance src/lib.rs + platform_impl/windows.rs, window-state src/lib.rs, opener README/commands.rs/permissions, dialog/updater/process permissions, clipboard-manager guest-js)
- create-tauri-app: https://github.com/tauri-apps/create-tauri-app (templates/template-react-ts, src/args.rs)
- tao content protection: https://github.com/tauri-apps/tao/blob/tao-v0.37.1/src/platform_impl/windows/window.rs

**wry / WebView2 / Chromium**
- https://github.com/tauri-apps/wry/blob/wry-v0.57.0/src/webview2/mod.rs
- https://github.com/chromium/chromium/blob/main/third_party/blink/renderer/platform/media/resource_multi_buffer_data_provider.cc
- https://github.com/MicrosoftEdge/WebView2Feedback/issues/2679 · /3230 · /2171
- https://github.com/MicrosoftEdge/WebView2Announcements/issues/126 (Local Network Access)
- https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/process-model
- https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2environment5
- https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/process-related-events
- https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/user-data-folder
- https://learn.microsoft.com/en-us/troubleshoot/microsoft-edge/development/video-playback-issues
- https://learn.microsoft.com/en-us/microsoft-edge/webview2/how-to/playwright · https://playwright.dev/docs/webview2

**Windows APIs**
- windows 0.62.2 crate source (static.crates.io/crates/windows/windows-0.62.2.crate): Win32/System/DataExchange, Memory, Ole, Threading, UI/Shell
- https://learn.microsoft.com/en-us/windows/win32/api/shlobj_core/nf-shlobj_core-shdodragdrop
- https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-openclipboard (NULL owner ⇒ SetClipboardData fails)
- https://learn.microsoft.com/en-us/windows/win32/shell/clipboard (CF_HDROP, CFSTR_PREFERREDDROPEFFECT; via report 06)

**Libraries**
- interprocess 2.4.6 source: examples/local_socket/tokio/{stream,listener}.rs; src/local_socket/name/type.rs; src/os/windows/named_pipe/c_wrappers.rs
- ts-rs 12.0.1 source (src/lib.rs: Config, TS trait, env vars)
- arboard 3.7.0 source (src/lib.rs, src/platform/windows.rs)
- drag-rs: https://github.com/crabnebula-dev/drag-rs (README, crates/tauri-plugin-drag/src/commands.rs, permissions/default.toml, crates/drag/Cargo.toml, platform_impl/windows; issues #41, #72, #76, #77, #94, PR #95)
- wavesurfer.js: https://github.com/katspaugh/wavesurfer.js/releases/tag/8.0.0 · src/wavesurfer.ts, src/player.ts, src/renderer.ts, src/plugins/regions.ts at 8.0.3
- @radix-ui/react-slider 1.6.1 package (dist/index.d.ts, index.mjs)
- react-image-crop README: https://github.com/dominictobias/react-image-crop
- shadcn: https://ui.shadcn.com/docs/changelog/2026-07-base-ui-default · shadcn 4.22.1 CLI (`init -b base|radix|aria`, `-t vite`)
- TanStack virtual-core 3.19.6 and react-router 1.170.45 type definitions
- @vitejs/plugin-react 6.1.2 README
- TypeScript 7.0: https://devblogs.microsoft.com/typescript/?p=5246 (Announcing TypeScript 7.0, 2026-07-08)
- Vitest 5.0.0 release notes: https://github.com/vitest-dev/vitest/releases/tag/v5.0.0
- WebdriverIO Tauri: https://github.com/webdriverio/desktop-mobile (packages/tauri-plugin-webdriver/README.md, docs/adr/0002-tauri-embedded-webdriver-default.md)
- StreamJsonRpc LengthHeaderMessageHandler: https://microsoft.github.io/vs-streamjsonrpc/api/StreamJsonRpc.LengthHeaderMessageHandler.html

**Earlier Kairo research used:** 00-brief.md (§3.4, §3.6 S3, §7.1–7.4), 05-stack-workflow.md (§2a, §5, §6.5), 06-post-clip-sharing.md (§1.3, §2.2, §3.1–3.3), 07-shipping-legal.md (§3).
