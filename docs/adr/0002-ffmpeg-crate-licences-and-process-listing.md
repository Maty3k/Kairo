# ADR-0002: The FFmpeg crates' licence, and no process-listing crates

- **Status:** Accepted
- **Date:** 2026-11-01
- **Decided by:** Claude Code, within the plan: [ADR-0001](0001-architecture-and-stack.md) item 3 chose the crates, and this record adds no network host, house-rule change or elevated part ([README](README.md))
- **Sources:** [research 16](../research/16-verify-testbed/README.md) §2 item 3 and §3.1; [PLAN §2.11](../PLAN.md#211-repository-layout-languages-key-dependencies-coding-standards) (decision 32); `deny.toml`; house rule 3 in [CLAUDE.md](../../CLAUDE.md)

## Context

- ADR-0001 item 3 links FFmpeg's LGPL libraries through `ffmpeg-next` =9.0.0 and `ffmpeg-sys-next` =9.0.1. Both crates are licensed WTFPL, which `deny.toml`'s allow-list doesn't hold; its comment already expected per-crate exceptions "by ADR".
- The first workspace crate to link them is `kairo-verify`, for its linked decoder on Windows (research 16 §2 item 3), before M3's engine. That keeps the linking plumbing off M3's counted days (M1's item on `cargo xwin check` with `ffmpeg-sys-next`).
- House rule 3 allows no handle on a game beyond `PROCESS_QUERY_LIMITED_INFORMATION`. Crates that list processes, such as `sysinfo`, may open them with more rights, and CI's API scan reads only Kairo's own code, not its dependencies' (research 16 §3.1).

## Decision

1. `deny.toml` adds WTFPL as a **per-crate exception** for `ffmpeg-next` and `ffmpeg-sys-next` only. WTFPL stays off the general allow-list, so no other crate gains it.
2. Both crates are used with default features off: `ffmpeg-next` with `codec` and `format`, `ffmpeg-sys-next` with `avcodec` and `avformat`, plus avutil, which both always bring. Defaults would add avdevice, avfilter, swresample and swscale through feature unification. The engine adds `swresample` when it needs it, in M3.
3. The FFmpeg DLLs remain LGPL and aren't crates, so cargo-deny can't see them. At run time, `kairo-verify`'s linked decoder refuses to decode unless avutil, avcodec and avformat all report an LGPL licence, and `build.lgpl` fails a clip checked with a non-LGPL linked FFmpeg.
4. `deny.toml` bans `sysinfo`. A later need to list processes uses Kairo's own `platform` code (Toolhelp snapshots matched by image name, house rule 3), not a crate.

## Consequences

- cargo-deny passes with the FFmpeg crates in the graph, and a new WTFPL dependency still fails it.
- The licence guard covers what cargo-deny can't: a GPL FFmpeg build swapped in by mistake (Homebrew's, for one) can't be linked unnoticed.
- `ffmpeg-next` calls itself "in maintenance mode" (ADR-0001's consequences). Its `from_path` unwraps on a non-UTF-8 path, so `kairo-verify` refuses one first. Its safe accessors panic on a plane index out of range, so the decoder checks first.
- A process-listing need costs a small `platform` module instead of a dependency.
