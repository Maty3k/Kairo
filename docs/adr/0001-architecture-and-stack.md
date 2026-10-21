# ADR-0001: Architecture and stack

- **Status:** Accepted
- **Date:** 2026-10-21
- **Decided by:** the owner (decisions 5, 9, 13, 14, 26, 27, 29, 32 and 35 in [PLAN §8](../PLAN.md#8-decision-log-and-open-questions)); written up by Claude Code
- **Sources:** [PLAN §2](../PLAN.md#2-architecture-and-technical-design), research [00](../research/00-brief.md) §3, [05](../research/05-stack-workflow.md), [08](../research/08-capture-rust.md), [09](../research/09-encode-mux-rust.md), [10](../research/10-audio-rust.md), [11](../research/11-engine-shell-rust.md), [12](../research/12-tauri-ui.md), [13](../research/13-amd-performance.md) and the [implementation cookbook](../research/15-implementation-cookbook.md)

## Context

Kairo is a Windows instant-replay clipper: it keeps the last moments of a game in memory and saves them as a video when you press a hotkey. Its constraints decide the stack more than taste does.

- **Never touch the game** (decision 14). Anti-cheat systems watch for code inside game processes, memory reads and synthetic input, and Kairo runs next to every game. Capture therefore goes through Windows' own APIs, from a separate process.
- **Never cost the game.** The always-on part must be small and native, and the clip browser must not run while you play. The performance gate is a mean FPS loss of at most 3% ([PLAN §3.2](../PLAN.md#32-testing-strategy)).
- **Never miss the play.** Hotkeys must work over fullscreen games, a save must turn into a verified file in under a second, and a crash in a GPU driver or in FFmpeg must not take the tray and hotkeys down with it.
- **AMD first** (decision 9). The owner's RX 7000/9000 PC encodes with AMF; NVIDIA (NVENC) and Intel (QSV) follow, with Windows' Media Foundation encoder as the fallback.
- **Licences** (decisions 5, 32, 35). The desktop app is GPL-3.0-or-later and takes no GPL dependencies, so FFmpeg is used as LGPL libraries, and libobs and OBS code are out.
- **How it is built** (decisions 27, 29). Claude Code writes all the code on an 8 GB M2 Mac with no Windows VM, and tests on the owner's Windows PC over SSH; the owner doesn't read code. The compiler, the tests and the capture harness have to catch what code review can't.
- **Windows facts.** Services run in session 0, which can't capture the desktop or receive hotkeys, so the engine is a per-user process in the interactive session (research 05 §1.1).

## Decision

Two programs that talk over one contract (research 05 §0 and §7; [PLAN §2.1](../PLAN.md#21-process-model)):

1. **Engine: `kairo-engine.exe`, in Rust.** Per-user, unelevated, in the interactive session, with no web view. One binary with two roles: *control* (tray, hotkeys, game detection, IPC, the library database, the crash server, updates) and *media* (capture, conversion, encoding, audio, the replay ring, saves), the latter in a kill-on-close Job Object so that driver or FFmpeg crashes never take down the tray or the hotkeys. M3 runs media in-process behind a `Pipeline` trait; M5 splits it out.
2. **Capture:** Windows.Graphics.Capture through Kairo's own small module on the `windows` crate (=0.62.2), not the `windows-capture` crate; DXGI Desktop Duplication as the fallback and as the Windows 10 path. Nothing is injected or hooked (research 08).
3. **Convert and encode:** one D3D11 device shared with FFmpeg under one lock; HLSL shaders render NV12, and AMF only ever receives NV12; hardware encoding through FFmpeg 9.0.2 (LGPL libraries) via `ffmpeg-next` =9.0.0 and `ffmpeg-sys-next` =9.0.1: `h264_amf` first, NVENC and QSV later, `h264_mf` as the fallback; MP4 with faststart and no edit lists (research 09 and 13; cookbook §3–4).
4. **Audio:** WASAPI via the `wasapi` crate (=0.26.0): process loopback for the game and for voice chat, plus the microphone; one track per source plus a mix, aligned on Windows' QueryPerformanceCounter clock (research 10).
5. **Replay ring** of encoded packets in RAM, at most 120 s in v1. Saves cut on keyframes and are verified before Kairo reports success (cookbook §5).
6. **UI: `Kairo.exe`, Tauri 2 with React and TypeScript** (research 12). It starts on demand and exits fully when its windows close, so no WebView2 runs during play. It is built and tested on the Mac against a mock engine.
7. **IPC: JSON-RPC 2.0 over a per-user named pipe** with a 4-byte big-endian length prefix (at most 1 MiB per message), a DACL that admits only SYSTEM and the user, and remote clients rejected; the Mac mock uses a Unix socket. The framing matches StreamJsonRpc's, so a C# engine could serve the same contract. One `rpc_methods!` registry in `kairo-rpc` generates the dispatcher, the TypeScript types, the JSON Schema and the docs; JSON fixtures bind the mock, the engine and any replacement engine (research 11 §4; cookbook §7.4).
8. **Network:** only the `kairo-net` crate opens sockets (update checks, opt-in crash reports), behind a host allow-list ([PLAN §2.10](../PLAN.md#210-safety-privacy-and-security-rules)).
9. **Development loop:** the Mac for editing, `kairo-core` tests and `cargo xwin check`; the PC, over SSH, for every build that runs, with GUI work launched into the owner's session through scheduled tasks; GitHub-hosted CI, never a self-hosted runner ([PLAN §3.1](../PLAN.md#31-machines-and-toolchains-the-ssh-bridge-interactive-launching-the-mock-engine), [§3.3](../PLAN.md#33-cicd)).
10. **Permanent app identifier: `io.github.maty3k.kairo`.** It is the Tauri identifier and names the app-data folder `%LOCALAPPDATA%\io.github.maty3k.kairo\`, so changing it later would strand settings and libraries.

Exact versions and the full dependency list live in [PLAN §2.11](../PLAN.md#211-repository-layout-languages-key-dependencies-coding-standards) and [cookbook §1](../research/15-implementation-cookbook.md#1-repo-and-crate-layout); this ADR doesn't repeat them.

### Alternatives considered

| Option | Why not, or not first |
|---|---|
| The same Tauri UI with a **C#/.NET 10 NativeAOT** engine | Easier for the owner to read, with mature libraries (research 05 ranks it #2). Kept as the exit route below. Rust comes first because one native toolchain covers the engine, the UI shell and the mock, and its compiler guards memory and threads in code nobody reviews line by line |
| A **libobs** engine (OBS) | The fastest route to proven capture and replay, but it is GPL code, heavier while always on (≈100–250 MB [Estimate], research 05 §3), and its game capture works through hooks. Decision 32 keeps GPL code out, which keeps a GPL §7 Store permission possible ([PLAN §5](../PLAN.md#5-shipping-and-operations)) |
| The **`windows-capture`** crate | It creates its own D3D11 device on the default adapter, without video support or multithread protection, so the device can't be shared with the encoder; it uses a one-buffer pool, never requests borderless capture, and a breaking move to windows-rs 0.100 is pending (research 08 §2) |
| **Electron** or NativePHP for anything always on; **WPF or WinUI 3** as the main UI; **C++/Qt** | An extra runtime and RAM during play; no way to build the UI on the Mac; the steepest learning curve and undefined-behaviour risk for AI-written code (research 05 §0 and §3) |
| The Tauri app as the always-on tray process, with the engine as its sidecar | The always-on process would host the web view, and it is unverified that WebView2 fully exits when the last window closes. With the engine owning the tray and hotkeys, Kairo works with zero UI processes running (research 05 §5.1) |
| A Windows service | Session 0 can't capture the desktop or receive hotkeys (research 05 §1.1) |

## Consequences

**Good**
- Nothing Chromium-based runs during play. The engine's always-on footprint is small (≈15–40 MB before the replay buffer [Estimate], research 05 §3); the budget is ≤60 MB plus the buffer.
- Once media runs in its own process (M5), a media crash loses only what was in the replay buffer at that moment. The tray, the hotkeys and the library stay up, and media restarts (target: under 3 s, M5 gate G6).
- The UI, the contract, the fixtures, the mock and the harness survive a change of engine language.
- The licence picture stays simple: permissive crates and LGPL FFmpeg under a GPL-3.0-or-later app.
- UI and pure-logic work happen on the Mac, without the PC.

**Costs**
- Kairo builds its own capture stack (≈250 lines of WGC and ≈200 of DDA, plus shaders and encoder glue) instead of reusing OBS.
- COM and FFI code needs `unsafe`. It is confined to the capture, gpu, encode, audio, platform and input modules, each block with a `// SAFETY:` comment ([PLAN §2.11](../PLAN.md#211-repository-layout-languages-key-dependencies-coding-standards)).
- Exact pins with churn ahead: windows-rs 0.100, wasapi 0.27 and Tauri 3 (PLAN risk R11); `ffmpeg-next` describes itself as being in maintenance mode.
- The Mac only compile-checks Windows code. Anything that runs needs the PC or CI, so the development loop depends on the SSH bridge and the scheduled-task launcher that M0 sets up.
- FFmpeg's LGPL brings duties: dynamic linking, libraries users can replace, and the exact source, changes and configure line published with every release; Kairo maintains its own minimal LGPL FFmpeg build in CI.
- WebView2 adds roughly 100–300 MB while the library window is open [Estimate].

## Exit rule (decision 26; [PLAN M3](../PLAN.md#4-milestones))

If Rust stalls, the engine switches to C#/.NET 10 NativeAOT. The UI, the contract, the fixtures, the mock and the harness stay.

- Every working day on Rust capture, encode or audio code counts from the first `wgc-probe` commit (at least 4 hours of engine work with the PC available for at least 2 hours). A day is excluded only if the PC was unavailable, or the blocker reproduces with the FFmpeg CLI or a C++ sample, logged that day in `docs/spikes/engine-log.md` with evidence.
- Loop signals: the same compile, borrow or COM error class unresolved after 3 attempts, or an `unsafe` crash unresolved after 1 day.
- From day 3, a one-line daily summary to the owner. Day 4 without an end-to-end hard path: Claude drafts the C# plan and installs the .NET 10 SDK on the Mac and the PC. Day 5: a burn-down note that lists every exclusion.
- At 7 counted days, or 14 calendar days after M3 starts, without M3's timed gate (20 of 20 harness saves pass `kairo-verify`; save to file under 1 s at p95; no capture stall during saves), Claude switches the engine to C# unless the owner replies "keep Rust".
- A missed target that isn't Rust-specific goes to its own ADR, not to the C# switch.

If the switch happens, a new ADR supersedes the engine-language parts of this one.
