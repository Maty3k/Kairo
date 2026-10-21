# Research prototypes

These are throwaway sketches that the research agents wrote on 2026-10-21 to check that the code in reports 08–15 uses real APIs. They are **not** Kairo's engine. They were type-checked with `cargo check --target x86_64-pc-windows-msvc` on a Mac, and only `corecheck`'s unit tests were actually run.

| Folder | What it checks | Used by |
|---|---|---|
| `kcapcheck/` | WGC/DDA capture, device setup, monitors, fullscreen detection, display affinity | [08-capture-rust.md](../08-capture-rust.md) |
| `kwcapcheck/` | The `windows-capture` crate's API, for comparison | [08-capture-rust.md](../08-capture-rust.md) |
| `wincheck/` | Engine shell: GPU lock, shaders, WGC rate, audio, pipe, hotkeys, input, crash handling, logging, PDH, drag-out | [15-implementation-cookbook.md](../15-implementation-cookbook.md) |
| `ffcheck/` | FFmpeg bindings: D3D11 hardware frames, encoder options, MP4 muxing | [09-encode-mux-rust.md](../09-encode-mux-rust.md), cookbook |
| `corecheck/` | Replay-ring cutting, the audio aligner, the MinUpdateInterval rule; 5 unit tests | cookbook |
| `scripts/` | MP4 box inspector, hosting-cost model, drift-loop simulations, licence/table checkers | reports 06, 10, 15 |

Each `Cargo.toml` has an empty `[workspace]` table, so these crates stay outside Kairo's real workspace. Treat them as references to copy from; don't depend on them.
