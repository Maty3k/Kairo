# Research 17 requirements: kairo-loadgen, pc bench, PresentMon and the bench scripts (M1 gate 4)

*2026-11-14 · Requirements digest for research 17. Nothing in the repo was changed, built or run, and the PC was not touched. Line numbers are `main` at ddec2c9; the testbed (PR #31) and the harness (PRs #34–35) are read from their unmerged worktrees.*

## How to read this

**Citation keys.** Each key is a file and a line number.

| Key | File |
|---|---|
| `CL:n` | CLAUDE.md |
| `PLAN:n` | docs/PLAN.md |
| `M0:n`, `M1:n` | docs/milestones/M0.md, M1.md |
| `R08:n`, `R13:n` | docs/research/08-capture-rust.md, 13-amd-performance.md |
| `CB:n` | docs/research/15-implementation-cookbook.md |
| `R16:n` | docs/research/16-verify-testbed/README.md |
| `job.rs:n` | crates/kairo-core/src/job.rs |
| `display:n`, `display/platform.rs:n` | crates/kairo-display/src/lib.rs, platform.rs |
| `pc:n`, `lib.ps1:n`, `fetch.ps1:n` | tools/mac/pc, tools/mac/remote/lib.ps1, tools/mac/remote/harness-fetch.ps1 |
| `boot:n`, `doctor:n` | tools/pc/bootstrap.ps1, tools/pc/doctor.ps1 |
| `ci:n` | .github/workflows/ci.yml |
| `TB:file:n` | testbed worktree (PR #31, `m1/testbed` at f88ed79), crates/kairo-testbed/src/ |
| `HN:path:n` | harness worktree (PRs #34–35, `m1/harness-run` at 1cbfa9f) |
| `PM:path:n` | PresentMon source at tag v2.6.0 (copies in `research17/pm-src`); files in its `PresentMon/` folder may drop the folder name |
| `NET:file:n` | .NET runtime and PowerShell sources (copies in `research17/dl`) |

**The downloads** (untrusted data: read, never run).
- `pm-src` is PresentMon **v2.6.0**. `tagref.json` gives `refs/tags/v2.6.0` → commit `e13fce6acdb55a808fd8318175a56863e532d95f`; `tree.json` is that commit's tree (1,103 entries, not truncated), and all 36 copied files hash to its blob SHA-1s (recomputed here). `release.json`: published 2026-09-21T17:30:36Z from `release/2.6.0`; asset `PresentMon-2.6.0-x64.exe`, 980,320 bytes, `sha256:b2a706bc6ad475749e3b7e3409263aa1e6906d45bdcf993f6dbc0f660188f1af`, the same pin as boot:169–174 and doctor:75–77. `PM:` citations are primary.
- `dl` holds `Process.cs`, `Process.Windows.cs`, `ProcessManager.Windows.cs` (dotnet/runtime) and `TypeTable_Types_Ps1Xml.cs` (PowerShell), all MIT, with no tag or commit recorded. `NET:` citations show the mechanism; that PowerShell 7.6.7 (PLAN:308) ships exactly this code is [Unverified].

**Tags.**
- **[Rec]** my recommendation where the sources are silent or disagree, with the reason.
- **[Conflict]** sources disagree. Precedence: CLAUDE.md; the owner's answers and M1's contract, whose names "replace the research's launch sketches" (M1:48); then PLAN, the cookbook, research.
- **[Gap]** no source says. **[Owner]** needs the owner's answer or hands (§11).
- **[Unverified]**, **[Estimate]**, **[proposed]** as in R16:23–27.
- **M1** = build now. **M2+** = design for now, build later (M2's ABBA ×3 reuses these tools).

---

## 0. Summary

- **Gate 4** (M1:29, M1:140): `pc bench` runs a 2-minute warm-up and the A/A noise floor (6 runs per workload; CV >1% → more runs) on `kairo-loadgen` with PresentMon 2.6.0, and `pm-stats.ps1` turns each CSV into one line of statistics. The check is that the runs "produce data" (PLAN:417), not CV ≤1%. Loadgen and bench sit outside the exit-rule clock (M1:7, M1:110, PLAN:415). Games, ABBA and the gate clauses wait for M2 (M1:29, PLAN:427).
- **Other M1 deliverables** (M1:17, M1:19, M1:131–132): `ab-stats.ps1`; `encoder-usage.ps1`, which PR 9c also needs before every harness job (R16:865; HN:tools/mac/pc:1120 prints "encoder use: not measured yet (PR 9c)"); `env.json` (M1:87); the AMD settings diff script (M1:19, M1:88, M1:142).
- **What changes the research's sketches:**
  1. [Conflict] PresentMon "may run from SSH" (M1:97), but the elevated session never creates a file in `logs\` (M1:81, lib.ps1:95–98), and CSVs belong in `logs\` (M1:75). [Rec] run PresentMon as the owner, as a child of the bench job (§4).
  2. Research 13's `encoder-usage.ps1` prints `$p.Path` (R13:631). In PowerShell that property is `$this.Mainmodule.FileName` (NET:TypeTable_Types_Ps1Xml.cs:1136–1142), which opens the process with `PROCESS_QUERY_INFORMATION | PROCESS_VM_READ` and calls `EnumProcessModulesEx` (NET:ProcessManager.Windows.cs:794–808): a module list and a memory-read handle on every process that uses a video engine, which can be the game itself (on AMD its video decode counts as `VideoCodec`, R13:618). CL:9 bans that, and the path it prints is a raw path (CL:16, M1:86). CI's banned-API scan (ci:58–76) reads only Rust under `crates/`, so nothing would catch it (M1:102).
  3. PresentMon's `--scroll_indicator` presses Scroll Lock with `SendInput` (PM:PresentMon/MainThread.cpp:42–77), banned by CL:10 and M1:101; `--restart_as_admin` restarts it with the `runas` verb, a UAC prompt (PM:MainThread.cpp:266–270, PM:PresentMon/Privilege.cpp:124–134). The bench never passes either.
  4. `ab-stats.ps1`'s t-table (R13:600) has no entry for 1, 11, 13, 14, 15 or more than 16 pairs; `$t975[$n]` is then `$null`, the interval collapses to the mean, and PASS tests the mean alone.
  5. The research's diff script takes all three toggles behind one `Read-Host` (R13:485). M1 needs two calls (M1:88), and one diff over three toggles can't say which value belongs to which setting.
  6. A bench session takes ≈25–45 min [Estimate]. Holding the displays on (as the testbed and harness do: TB:platform.rs:88–105, R16:775) keeps the OLED on a static desktop that long, which M1:35 warns against; letting Windows' screen-off timer run can blank the 180 Hz monitor mid-run. [Owner]
  7. [Conflict] Research 13 publishes `env.json` with the bench summary (R13:843); M1:87 and CL:18 say never. M1 wins.
- **Owner items** (§11): the attended bench window; the OLED during it; Instant Replay and AMD's frame-rate features during the M1 A/A; the diff-script toggles; and two already open (test days, M1:35; the SSH account, M1:172, which would move the diff script's HKCU).

---

## 1. Constraints that bind every piece

| Area | Requirement | Source |
|---|---|---|
| Clock | Loadgen, bench and the AMD diff script are outside the exit-rule clock | M1:7, M1:110, PLAN:415 |
| Before | `pc status` first and again before each long step: BUSY (owner input in the last 10 min, or a fullscreen window in front) or UNKNOWN → Mac work. `pc doctor` each session; it checks PresentMon's hash and Performance Log Users (doctor:674–693) | CL:23, M1:96, M0:74 |
| Before | Owner signed in and unlocked, displays on, no Remote Desktop | M1:96, CB:1175 |
| Who is home | Until `kairo-devrun` stops on a game start or another fullscreen window, restores displays and audio, keeps the PC awake and covers the OLED, no job runs while the owner is away: only short jobs while the owner is home and has said in chat that the PC is free, with `abort_on_input` on | M1:103, M1:135, M1:213 |
| Who is home | Test days are unanswered, so no away runs yet; nothing overnight unless the owner says so | M1:35, M1:37, M1:153, CL:25 |
| Who is home | Today a game started with a controller neither counts as input nor stops a job | M1:103 |
| Builds | Deploy only between sessions; keep 3 builds. [Rec] a bench session records its build id and stops if `bin\current` changes | CL:24, M1:96 |
| Updates | After a Windows, GPU-driver or Adrenalin update: a harness smoke at the next idle window; env.json keeps data from before and after apart | PLAN:333, M1:217 |
| Limits | Every job has `max_seconds` (1–86,400) and `abort_on_input` on (the default); one job at a time; `pc` exit 6 = stopped by `kairo-devrun` (time limit or input), which stops the whole job tree | M1:98, job.rs:15, job.rs:57–61, pc:330–332, R16:788 |
| Launch | GUI, capture and audio programs start only through `\Kairo\Run`: owner's session, priority 4, no console window; devrun refuses jobs from the SSH session | CL:24, M1:97, M0:63–67 |
| Launch | Never benchmark through a `schtasks /create /it` task (below-normal priority); bench jobs keep `"priority": "normal"`, the default | CB:54, PLAN:332, M1:48, M1:81, job.rs:33–40 |
| Jobs | Job file `{"id"` (`^[A-Za-z0-9_-]{1,64}$`), `"cmd"`, `"cwd"`, `"max_seconds"`, `"abort_on_input"`, `"window"`, `"priority"}`; result `C:\kairo\logs\<id>.result.json`, output `<id>.out.log` and `<id>.err.log`; a job that names a monitor is kept in `.dogfood/` | M0:72–73, M1:76, M1:81 |
| Files | Everything a job writes goes to `C:\kairo\logs\` (jobs write nowhere else but `state\`); the file names, and any new program name, join M1's contract as they land | M1:50, M1:75, M1:98 |
| Files | Jobs run programs from `bin\current`; `pc deploy` copies only `kairo*.exe` and `testbed.exe` with their `.pdb` and `.dll` files, so loadgen's HLSL lives inside the exe and no script can be deployed to `bin\` | M1:72, M1:81 |
| Elevated | Claude's elevated sessions never create, rename or delete anything in `logs\`, `state\` or `build\`; they read there only through a handle checked to be an owner-owned regular file; nothing elevated runs a program from `build\` or `bin\` | M1:81, lib.ps1:95–98, lib.ps1:116–161 |
| Elevated | Only the SSH bridge is elevated; Kairo's programs never are (the elevated test task isn't built); no UAC prompt without asking. [Rec] no `Add-Type` in bench scripts, since the bootstrap fix drops it from elevated sessions | CL:10–11, CL:42–43, M1:171, M1:185 |
| Scripts | `pc bench` sends its PowerShell on stdin to `pwsh -NoProfile` (PowerShell 7.6.7), so a `Read-Host` can't get a key press | M1:88, M1:92, M0:82, PLAN:308 |
| Scripts | PowerShell keeps the Never list and the privacy rules unaided: CI checks banned APIs and sockets only in Rust and `Cargo.toml` files under `crates/` | M1:102, ci:58–76 |
| Privacy | Never log clips, audio, typed keys, non-game window titles or raw paths; process lists use file names | CL:16, M1:86, M1:102 |
| Privacy | Never commit logs, PC details or raw CSVs; raw results, env.json included, stay in `.dogfood/`; docs and the gate report get aggregates and hardware class only | CL:18, M1:78, M1:87, M1:221, PLAN:377 |
| Privacy | Monitor device paths and friendly names are PC details: `.dogfood/` only, never in a log that might be shared (the trace rule: no monitor, adapter or PC names) | display:17–18, M1:77, M1:89 |
| Monitors | M1: loadgen borderless on the 180 Hz monitor (answered 2026-10-24); a window picks its monitor by device path, never "the main screen"; patterns go on the 180 Hz monitor | M1:38, M1:40, CL:26 |
| OLED | Moving content only, ≤10 min per run, a black capture-excluded cover when unattended; M2's SP1 puts loadgen there in chunks of ≤10 min with the cover in between | CL:26, M1:39–40, M1:99 |
| Adapters | Two adapters (the Ryzen's iGPU and the RX 9000-series card): every D3D11 program uses the adapter that drives its monitor, by LUID, and logs the LUID | M1:66, R13:872 |
| Anti-cheat | No synthetic input; no game in M1; any anti-cheat warning is P0. Game handles ≤ `PROCESS_QUERY_LIMITED_INFORMATION`, no memory reads, no module lists; capture only through WGC or DDA, never `vsrc_amf` | CL:7–12, M1:100–101 |
| AMD | Kairo never writes AMD's registry or settings; the owner toggles them | R13:465, R13:811, CB:1066 |
| Encoding (M2+) | AMF: NV12 only, every option set and read back, H.264 peak ≤50 Mbps; one H.264 session at a time, each released ≥1 s before the next | CL:31, M1:100 |
| P0 | A display-driver reset (System event 4101, provider Display) or an unexpected reboot (41) during a job: stop all PC work and report | M1:214, R16:866, HN:tools/mac/remote/harness-events.ps1:1–6 |
| Ask | Before physical actions, PC changes outside Kairo's folders, security or privacy settings, UAC prompts | CL:42–43, PLAN:382 |
| Code | `windows` =0.62.2; `unsafe` only in gpu and platform modules, with `// SAFETY:`; no `unwrap` on runtime paths; no per-frame allocation; no GPL code, no OBS shaders | CL:17, CL:30, CL:32, PLAN:316 |
| Process | Branches `m1/<slug>`, Conventional Commits with DCO, squash after CI; "done" includes docs and CHANGELOG. [Rec] loadgen and bench PRs aren't media PRs (no capture, encode or audio), so no harness smoke; M2's B-run code is | CL:34, CL:44, PLAN:383, M1:204 |

Plan decisions that touch measuring: quality gates, no deadline (PLAN:648); the AMD RX 7000/9000 rig with a 360 Hz HDR OLED and a 180 Hz panel (PLAN:629); replacing Instant Replay (PLAN:631); instruments before spikes (PLAN:659); the exit-rule counting (PLAN:646, PLAN:657); RAM read from the first bench's env.json (PLAN:663); default quality from M2's numbers (PLAN:667).

---

## 2. `kairo-loadgen` (crate `kairo-loadgen`, PLAN:297)

| # | Requirement | When | Source |
|---|---|---|---|
| L1 | A deterministic GPU load with an **ALU** mode and a **bandwidth** mode; scriptable | M1 | M1:13, PLAN:352, R13:704–709 |
| L2 | Flip model with tearing: `DXGI_SWAP_EFFECT_FLIP_DISCARD`, `DXGI_SWAP_CHAIN_FLAG_ALLOW_TEARING`, `Present(0, DXGI_PRESENT_ALLOW_TEARING)`; uncapped; fixed duration | M1 | M1:57, R13:705, R13:709, CB:1079 |
| L3 | Borderless fullscreen on the 180 Hz monitor in M1 | M1 | M1:40 (C5) |
| L4 | ALU mode: research 13's pixel shader (`cbuffer P { float t; uint iters; float2 res; }`, a `[loop]` of `iters`), `iters` tuned to about 8 ms per frame | M1 | R13:707, R13:713–723 |
| L5 | Bandwidth mode: random reads from a 256 MB texture, because "Kairo's cost is partly bandwidth" | M1 | R13:708 |
| L6 | Deterministic, time-based animation | M1 | R13:709 |
| L7 | Monitor by `monitorDevicePath` through `kairo-display` (`targets()`, `find_by_path`, `adapter_for_luid`); device on that monitor's adapter by LUID; log the LUID | M1 | M1:64, M1:66, display:1–20, display:239, display/platform.rs:350, display/platform.rs:411 |
| L8 | Started only as a `\Kairo\Run` job at normal priority, never from SSH (research: "fully automatable over SSH", C15) | M1 | M1:97, R13:704 |
| L9 | Its fullscreen window makes `watch.json` report BUSY today; the planned fullscreen stop must exempt "the job's own testbed or loadgen window" | M1 | M1:103 |
| L10 | HLSL compiled at run time with `D3DCompile`, as the engine does; Windows CI compiles every entry point | M1 | PLAN:304, CB:353, CB:377 |
| L11 | On the OLED in M2's SP1: jobs of ≤600 s, moving content only, the cover between chunks | M2+ | M1:40, CL:26, M1:99 |
| L12 | First of the bench's workloads; games, an HDR run, video on the 180 Hz screen and alt-tab follow in M2 | M2+ | PLAN:353, R13:710 |
| L13 | Outside the exit-rule clock | M1 | M1:7, M1:110 |

**Gaps and recommendations.**
- [Gap] CLI, exit codes and log format: Claude decides and adds them to M1's contract (M1:50). [Rec] mirror the testbed: `--monitor <path> --mode alu|bandwidth --seconds S [--iters N] [--ready FILE] [--stop-on-stdin-eof] [--log FILE]`; exit 0 ok, 2 usage, 3 monitor, 4 device or swap chain (TB:cli.rs:10–36).
- [Gap] "Tuned to about 8 ms per frame" (R13:707) has no procedure, and tuning between runs would break determinism. [Rec] a separate calibration that measures GPU time per frame with D3D11 timestamp queries (CB:410–418; poll through the vtable, since windows-rs turns `S_FALSE` into `Ok`, R13:685–693) and prints `iters`; bench runs pass a fixed `--iters`, recorded in env.json. Give the bandwidth mode the same ≈8 ms target, so neither mode runs at thousands of fps: PresentMon warns of overflowed present events at high fps (default buffer 2048, PM:MainThread.cpp:455–460), and CSVs grow.
- [Gap] How a run shows it was deterministic. [Rec] loadgen logs frames presented, its own mean frame time and the QPC of its first and last present, to cross-check PresentMon's `MsBetweenPresents` and `CPUStartQPC` (`--qpc_time`, R13:551).
- [Rec] Native resolution of the monitor; an SDR BGRA8 swap chain in M1; check `DXGI_FEATURE_PRESENT_ALLOW_TEARING` and log the answer; no static text or HUD and a moving picture in both modes (L11); no device path or monitor name in its log (display:17–18).
- [Rec] Hold `ES_CONTINUOUS | ES_DISPLAY_REQUIRED` while running, as the testbed does (TB:platform.rs:88–105), subject to owner item O2.
- [Rec] Reuse the testbed's ready file and `--stop-on-stdin-eof` (TB:cli.rs:10–21): PresentMon's 5 s delay then starts after the window is up, and the bench can stop loadgen cleanly.
- [Gap] M2's "one HDR run" (PLAN:353) needs an HDR screen; on the OLED that means ≤10-min jobs (M1:40, M1:203). Whether an SDR flip-model swap chain keeps Independent Flip on an HDR desktop is [Unverified]. M2+.
- Reuse note: the testbed already does the borderless window on a monitor by path, the LUID device, a flip-discard swap chain, `KeepDisplayOn`, the ready file and stdin stop (TB:gpu.rs:1–9, TB:gpu.rs:131–146, TB:platform.rs:88–105). It runs no shader and sets no tearing flag; loadgen adds both.

---

## 3. `pc bench`

| # | Requirement | When | Source |
|---|---|---|---|
| B1 | A `tools/mac/pc` verb `bench`, research 13's "AB runner"; its PowerShell goes to the PC on stdin | M1 | M1:17, M1:92, PLAN:416, R13:775 |
| B2 | A 2-minute warm-up, discarded | M1 | M1:29, R13:830, CB:1078 |
| B3 | A/A noise floor: Kairo off, 6 runs per workload; run-to-run CV of average FPS; CV >1% → more runs, or fix the noise first (thermals, background tasks, power plan) | M1 | M1:29, PLAN:353, R13:730, R13:831, CB:1078 |
| B4 | Workload: `kairo-loadgen`; game benchmarks need the owner and wait for M2 | M1 | M1:29 |
| B5 | PresentMon 2.6.0 for each run; `pm-stats.ps1` turns each CSV into one line | M1 | M1:29, M1:74, M1:85 |
| B6 | The environment frozen and recorded in `env.json` (§8) | M1 | PLAN:353, R13:819–822, CB:1078, M1:87 |
| B7 | 60 s per run with `--delay 5`; 30 s idle between runs (stated for the A/B gate) | M1 (A/A: Gap) | R13:833, R13:547 |
| B8 | Tools: PresentMon, the four PowerShell scripts, `kairo-cli` (starts and stops the buffer), `kairo-loadgen` | M1, M2+ | R13:829 |
| B9 | AMD features off globally and in the game's profile: Instant Replay, Record Desktop, In-Game Replay, Metrics Overlay and logging, AFMF, Chill, Boost, FRTC, Enhanced Sync, RSR; VSync off; Game Bar capture off; no OBS, Medal, Discord streaming or Steam recording; second monitor a static desktop without video | M2+ (M1 needs none: M1:201) | PLAN:353, R13:823–828, R13:866, CB:1078 |
| B10 | A/B: ABBA ×3 = 12 × 60 s, 6 pairs per workload; A = engine not running, B = default config with the buffer armed; in SP1, B is FFmpeg `gfxcapture` → `h264_amf` with research 13's options and bitrates | M2+ | PLAN:353, PLAN:427, R13:731, R13:802–806, R13:833–834 |
| B10a | SP1's protocol: Instant Replay off, AMD features off, PresentMon with the four flags, PDH video-engine logging, `kairo-loadgen`; §5.6 tests 1–3, 5, 7 and 10 | M2+ | R13:749–754 |
| B11 | `encoder-usage.ps1` in parallel during one B run | M2+ | R13:835 |
| B12 | Analysis: `pm-stats.ps1` per CSV, then `ab-stats.ps1` per workload; `present_modes` and `gpu_wait_ms` deltas; Kairo's own tracing (duplicate ticks, backlog, shader GPU time) | M2+ | R13:837–839 |
| B13 | Variations: 3 pairs, 6 when within 1% of a threshold | M2+ | R13:840, PLAN:427 |
| B14 | Coexistence: Instant Replay alone, Kairo alone, both; the numbers feed the onboarding warning | M2+ | R13:841, PLAN:353, PLAN:427, CB:306 |
| B15 | Gate, Instant Replay off: mean FPS loss ≤3% with the upper 95% CI ≤4%; 1%-low loss ≤5%; PresentMode unchanged; no worse than Instant Replay alone + 1 pp | M2+ | PLAN:353, R13:731, CB:36, CB:58, CB:1078 |
| B16 | Workloads: loadgen; built-in benchmarks of 2 of the owner's games (the owner starts each "when the beep says so", ≈30 min per game); one HDR run; video on the 180 Hz screen; alt-tab between monitors; a Copy for Discord export during play; live play only for soaks | M2+ | PLAN:353, PLAN:440, R13:710–711 |
| B17 | Reused in M2 (SP1), M3 (untimed 3-pair A/B), M5 (G3 with the HDR run, video on the second screen and an export; the owner's 30-minute bench session), M8 and releases; a >1 pp regression blocks; PresentMode in every bench | M2+ | PLAN:346, PLAN:427, PLAN:474, PLAN:482, PLAN:595, PLAN:610, CB:1195, R08:805 |
| B18 | Gate 4's evidence line: aggregates (CV, n, run count), hardware class only | M1 | M1:221, PLAN:377 |
| B19 | Q1's RAM comes from the first bench's env.json | M1 | PLAN:663 |

**Gaps and recommendations.**
- [Gap] What a "workload" is in M1. [Rec] each loadgen mode is one: ALU and bandwidth, each with its own warm-up and 6 runs.
- [Gap] CV and the "more runs" rule. [Rec] CV = sample standard deviation (n−1) ÷ mean of the per-run `avg_fps`. If CV >1% after 6 runs, run 6 more once and stop; report CV, n and the mean's 95% CI. More runs narrow the mean's interval, which the A/B needs; they don't lower the true run-to-run CV, so a CV that stays above 1% is a setup finding (R13:831), never a reason to loop. Gate 4 passes on data either way (M1:29, PLAN:417). CB:1078 writes "CV ≤1%" as if it were a pass rule (C12).
- [Gap] Warm-up per workload or per session; the A/A's spacing. [Rec] per workload ("run the workload once", R13:830), and the A/B's timing for the A/A too (60 s, `--delay 5`, 30 s idle), with a fresh loadgen process per run, like a benchmark pass.
- [Estimate] One run ≈68 s (start ≈2 s, `--delay 5`, 60 s, exit ≈1 s). One M1 workload ≈120 + 6 × 68 + 5 × 30 ≈ 680 s ≈ 11 min; the M1 session (two workloads) ≈23–25 min, up to ≈45 min with extra runs. One M2 ABBA ×3 workload ≈120 + 12 × 68 + 11 × 30 ≈ 21 min. That is longer than M1:103's "short jobs" (O1).
- [Gap] Job shape. One job at a time (M1:98) and PresentMon (§4) point to one runner job per session that starts loadgen and PresentMon as children, like `kairo-harness run`: one `max_seconds`, `abort_on_input` on, children killed and reaped, everything in one run folder in `logs\`, older folders pruned by the job as the owner (HN:crates/kairo-harness/src/run.rs:1–7, HN:crates/kairo-harness/src/prune.rs:1–6). [Rec]; its name (a `kairo-harness` subcommand or a new program) is Claude's to pick and add to the contract (M1:50). A PC-side job also survives an SSH drop, which a capture streamed over SSH doesn't.
- [Gap] BUSY from its own window and the 10-minute IDLE rule. `pc status` reads BUSY while loadgen is fullscreen (M1:103, M0:74), and IDLE needs 10 min without input (M0:74). [Rec] the attended pattern of R16:773–777: the owner says "go" in chat, Claude starts at once with `--force` (the owner's OK for that session, pc:286–288); inside the job, devrun handles input, and the runner stops when `watch.json`'s fullscreen `foreground_exe` is anything but `kairo-loadgen.exe`.
- [Gap] Driver caps. With no input, Radeon Chill lowers fps (R13:366), FRTC caps it and a forced VSync caps it; none is readable without ADLX, which is deferred (R13:428–433, R13:776). [Rec] record registry hints in env.json and flag a run whose average sits within 1% of the refresh rate, or whose `SyncInterval` isn't 0 or `AllowsTearing` isn't 1 [proposed].
- [Gap] Mixed present modes: a toast or overlay over loadgen demotes Independent Flip. [Rec] flag a run whose dominant `PresentMode` share is below 99% [proposed].
- [Gap] Storage and comparison. [Rec] `.dogfood/bench/<stamp>/` with the job file, env.json, every CSV, the pm-stats lines and a summary; regression checks (PLAN:346) compare only sessions whose env.json versions match (M1:217).
- [Gap] Fetching CSVs: main's fetch refuses files over 1 MiB (fetch.ps1:28); HN adds a 48 KiB-block mode with size and SHA-256 for clips (HN:tools/mac/remote/harness-fetch.ps1:1–11). A 60 s CSV at ≈125 fps is ≈7,500 rows, ≈2–3 MB [Estimate].
- [Gap] Exit codes. [Rec] those of `pc run` (pc:330–332) plus 7 for a 4101 or 41 event, as `pc harness` does (R16:868), with HN's `harness-events.ps1` run after every bench job.
- [Gap] M2's beep for the owner (PLAN:440) plays at −30 dBFS (CL:26).
- Research 13 §7 items the bench answers later: whether capture demotes a game from Independent Flip, which "PresentMon decides" (R13:784); window vs monitor and copy- vs convert-on-arrival cost (R13:785); `SetGPUThreadPriority(-2)` under HAGS (R13:790); AFMF visibility is SP1's §3.2 test (R13:783, R13:379–388). M1's scripts start on R13:788 (the diff script, §9 of this note) and R13:789 (PDH visibility, through `encoder-usage.ps1`, §7 of this note).

---

## 4. PresentMon 2.6.0

| # | Requirement | Source |
|---|---|---|
| P1 | Version 2.6.0, pinned in the toolchain table | PLAN:308, R13:538 |
| P2 | `C:\kairo\tools\presentmon\PresentMon-2.6.0-x64.exe`, SHA-256 `b2a706bc6ad475749e3b7e3409263aa1e6906d45bdcf993f6dbc0f660188f1af` (the v2.6.0 release asset, 980,320 bytes); the bootstrap installs it, the doctor checks the hash; `tools\` is admin-owned and the owner can read and run it | M1:74, boot:169–174, boot:1447–1456, doctor:75–77, doctor:674–681, M0:58, M0:61, SETUP.md:100 |
| P3 | `pc bench` always passes `--session_name KairoBench --qpc_time --track_frame_type --track_gpu_video` | M1:74, CB:58, PLAN:353 |
| P4 | Per run: `--process_name <exe> --output_file <csv> --stop_existing_session --delay 5 --timed 60 --terminate_after_timed --no_console_stats` | R13:542–548, CB:1072–1074 |
| P5 | Never the default session name "PresentMon"; `--stop_existing_session` only with a unique name, because Adrenalin ships its own PresentMon; the doctor's 2 s test uses `KairoDoctor` | R13:540, R13:863, M1:74, doctor:460 |
| P6 | Needs admin or Performance Log Users; the bootstrap added the owner, and a limited token recorded 2 s through the doctor's temporary S4U task | R13:539, CB:1076, M0:11, M0:27, M0:166, doctor:438–505, SETUP.md:103 |
| P7 | May run from SSH, since ETW tracing is system-wide; after a dropped connection `--stop_existing_session` with the unique name recovers the session | M1:97, CB:1174 |
| P8 | Read columns by header name; the README lags the code | R13:559, R13:865 |
| P9 | `MsVideoBusy` is the target process's video work, not Kairo's: Kairo's comes from PDH | R13:552, R13:864 |
| P10 | `FrameType` (needs driver instrumentation) labels AFMF frames | R13:553, R13:364 |

**Checked in the v2.6.0 source.**
- With no `--v1_metrics` or `--v2_metrics`, the CSV comes from the unified writer (PM:PresentMon/OutputThread.cpp:527–536, PM:PresentMon/CsvOutput.cpp:531–672). Display, input and GPU tracking are on by default (PM:PresentMon/CommandLine.cpp:392–394), so with P3 and P4 the header is:

  ```
  Application,ProcessID,SwapChainAddress,PresentRuntime,SyncInterval,PresentFlags,AllowsTearing,PresentMode,FrameType,TimeInQPC,MsBetweenSimulationStart,MsBetweenPresents,MsBetweenDisplayChange,MsInPresentAPI,MsRenderPresentLatency,MsUntilDisplayed,CPUStartQPC,MsBetweenAppStart,MsCPUBusy,MsCPUWait,MsGPULatency,MsGPUTime,MsGPUBusy,MsGPUWait,MsVideoBusy,MsAnimationError,AnimationTime,MsFlipDelay,MsAllInputToPhotonLatency,MsClickToPhotonLatency
  ```

  R13:557 lists `TimeInMs`; under `--qpc_time` it is `TimeInQPC` (PM:CsvOutput.cpp:556–560, PM:CommandLine.cpp:633–636). The README calls the video column `VideoBusy` (PM:README-ConsoleApplication.md:104, 127); that name appears only with `--v2_metrics` (PM:CsvOutput.cpp:598–600). R13:552's `MsVideoBusy` is right.
- `FrameType` prints `Application`, `Intel XeSS-FG` or `AMD AFMF`; unset and repeated frames print as `Application` (PM:CsvOutput.cpp:54–70).
- `Application` is a file name, never a path: process-start events are cut to the file name (PM:PresentData/PresentMonTraceConsumer.cpp:2854–2858), and processes already running are named from the last part of `QueryFullProcessImageNameW` (PM:OutputThread.cpp:294–318). `--process_name` ignores case, folder and extension (PM:CommandLine.cpp:618–631).
- A swap chain's first present has `MsBetweenPresents` = 0 (PM:IntelPresentMon/CommonUtilities/mc/MetricsCalculatorAnimation.cpp:150–156); missing values print `NA` (PM:CsvOutput.cpp:1180–1198).
- Handles: PresentMon opens processes with `PROCESS_QUERY_LIMITED_INFORMATION` only (PM:OutputThread.cpp:303), within CL:9. Elevated, it also turns on `SeDebugPrivilege` (PM:MainThread.cpp:255–277); unelevated, processes "started on another account" or short-lived show as `<unknown>` (PM:README.md:62–65), which doesn't affect the owner's own loadgen.
- Exit codes: 0 done; 1 bad arguments or manifest (PM:MainThread.cpp:229–240); 3 and 4 window, 5 hotkey (PM:MainThread.cpp:283–301); 6 session not started, either the name is in use without `--stop_existing_session` or access is denied (PM:MainThread.cpp:336–383); 7 `--terminate_existing_session` failed (PM:MainThread.cpp:244–253).
- Lost data shows only on stderr at exit: "ETW buffers were lost", "ETW events were lost", "overflowed present events" (PM:MainThread.cpp:448–462; warnings and errors go to stderr, PM:Console.cpp:309–352).
- Never pass `--scroll_indicator` (`SendInput`, PM:MainThread.cpp:42–77, 397–399), `--restart_as_admin` (`runas`, PM:Privilege.cpp:124–134) or `--hotkey`; never run without `--process_name`: the CSV would list every program that drew frames, which is why the doctor deletes its own (doctor:456–461).

**[Conflict] C6: where PresentMon runs and writes.** M1:97 and CB:1174 allow SSH; M1:81 and lib.ps1:95–98 forbid the elevated session to create files in `logs\`; M1:75 puts CSVs there.
- (a) [Rec] As the owner, a child of the bench job, writing into the run folder in `logs\`; the elevated side only reads through checked handles. It fits M1:75, M1:81 and M1:98, follows the harness's one-job pattern, survives an SSH drop, and gives PresentMon no `SeDebugPrivilege`.
- (b) Elevated from SSH with `--output_stdout` (PM:CommandLine.cpp:441, 553–557), the CSV kept in memory and sent to the Mac; no file on the PC. Each 65 s run needs the SSH session alive; whether a dropped session kills PresentMon is [Unverified] (a leftover `KairoBench` session is cleared by P7).
- (c) Elevated into an admin-only folder: a contract change, and M1:48 retired `C:\kairo\bench\`.
- The doctor's temporary S4U task (doctor:438–505) is another way to run it as the owner, outside `\Kairo\Run`'s one-job slot.

**Gaps.**
- [Unverified] Process naming with a limited token in practice: the doctor's "csv-ok" checks only that the file isn't empty (doctor:458–461), which a header passes. [Rec] the first bench run checks that rows with `Application` = `kairo-loadgen.exe` exist.
- [Rec] Keep PresentMon's stderr per run; any lost-data line marks the run invalid.
- [Gap] A run stopped by devrun leaves a cut last CSV row and maybe a live `KairoBench` session. [Rec] pm-stats ignores a partial final row; the next run's `--stop_existing_session` clears the session.

---

## 5. `pm-stats.ps1`

| # | Requirement | Source |
|---|---|---|
| S1 | `pm-stats.ps1 -Csv -Exe -Tag` prints one JSON line per CSV: `tag`, `frames`, `avg_fps`, `low1_fps` (1000 ÷ P99, nearest rank), `low01_fps`, `gpu_busy_ms`, `gpu_wait_ms`, `present_modes`, `frame_types` | M1:85, R13:572–595 |
| S2 | Average FPS = frames ÷ Σ frame time × 1000, over the application presents of the main swap chain | R13:563, CB:1076 |
| S3 | 1% low = 1000 ÷ P99(`MsBetweenPresents`), nearest rank, and the definition is stated in every report (some tools average the slowest 1%); 0.1% low = 1000 ÷ P99.9 | R13:564–565, CB:1076 |
| S4 | Present-mode share = % of rows per value | R13:566 |
| S5 | GPU contention: `MsGPUBusy`, `MsGPUWait`, `MsGPUTime`, mean and P99 ("Kairo's or DWM's work ... shows up as higher MsGPUWait and MsGPUTime at the same MsGPUBusy") | R13:567 |
| S6 | `MsBetweenDisplayChange` as a secondary cadence | R13:568 |
| S7 | Main swap chain = the `SwapChainAddress` with the most rows; invariant-culture numbers; frame times > 0; fail when the CSV has no rows for `-Exe` | R13:575–581 |
| S8 | Columns read by header | R13:559 |
| S9 | In the elevated session, files in `logs\` are read only through a checked handle | M1:81, lib.ps1:116–161 |

**Gaps.**
- [Conflict] C7: R13:578 reads with `Import-Csv -LiteralPath`. [Rec] read through `Read-SharedText` (lib.ps1:139–161) and parse the text with `ConvertFrom-Csv`, or run the script as the owner.
- [Gap] M1:85's field list leaves out S5's `MsGPUTime` and P99s and S6. [Rec] add `gpu_time_ms` and the P99s after M1's keys.
- [Gap] Application presents only (S2): R13's sketch counts every row of the main swap chain, AFMF frames included. [Rec] fps and lows from `FrameType == Application`; other types appear in `frame_types`.
- [Gap] S3's "state the definition". [Rec] a fixed key such as `"low_def":"p99_nearest_rank"`.
- [Rec] Also `seconds` (Σ frame time) and `rows_skipped`; refuse a CSV without `TimeInQPC`, `FrameType` and `MsVideoBusy`, so one made without P3's flags can't pass.

---

## 6. `ab-stats.ps1`

| # | Requirement | Source |
|---|---|---|
| A1 | `ab-stats.ps1 -A -B` prints `n pairs: loss m% (95% CI lo..hi%) PASS=…` | M1:85, R13:598–606 |
| A2 | Pairs by order index; per-pair loss = 100 × (A − B) ÷ A, with A = Kairo off | R13:598, R13:601 |
| A3 | 95% CI from Student's t: R13:600's values are t(0.975, n−1) keyed by n | R13:600–604 |
| A4 | Research's PASS tests only the mean (≤3%) and the upper bound (≤4%); the plan's other clauses join before SP1 | M1:85, PLAN:353 |

**Gaps.**
- [Gap] The t-table covers n = 2–10, 12 and 16 only (R13:600). For n = 11, 13–15 or >16, `$t975[$n]` is `$null`, the half-width is 0 and PASS rests on the mean; n = 1 divides by n − 1 = 0 (R13:603). [Rec] a full table (or a computed t) for n = 2–30; refuse n < 2, unequal lengths and values ≤ 0.
- [Gap] Operational meaning of the plan's clauses [proposed]: 1%-low loss = the same paired mean on `low1_fps`, ≤5%; "PresentMode unchanged" = the same dominant mode in every run, its share within 1 pp between A and B; "Instant Replay alone + 1 pp" = loss(Kairo) − loss(IR) ≤ 1 pp, both against the nothing-running baseline (B14).
- [Rec] Take pm-stats JSON lines per role once lows and present modes join, instead of bare arrays.
- [Rec] M1 self-test on A/A data: pairs (1,2), (3,4), (5,6) give 3 pairs whose interval should include 0. This exercises ab-stats in M1 without Kairo.
- [Gap] Where it runs: plain arithmetic, but PowerShell isn't in the Mac's install list (PLAN:327), so it runs on the PC through stdin. [Rec] run it, and pm-stats, at the end of the bench session while the owner's "go" still holds; a later re-analysis waits for an idle PC like any other PC step (CL:23). The exception for harness-pre and harness-events (HN:tools/mac/pc:9–10) covers steps inside a running flow, not later analysis.

---

## 7. `encoder-usage.ps1`

| # | Requirement | When | Source |
|---|---|---|---|
| E1 | `encoder-usage.ps1 -Seconds`: video-engine use per process, adapter LUID and engine type, with file names only (research's sketch prints full paths and merges the two adapters) | M1 | M1:86, R13:623–635 |
| E2 | Counter `\GPU Engine(*)\Utilization Percentage`; instances `pid_<PID>_luid_0x<HI>_0x<LO>_phys_<N>_eng_<N>_engtype_<Type>`, lower-cased by `Get-Counter` | M1 | R13:611–613, R13:862 |
| E3 | Video engines: `^Video(Encode\|Codec)$` (R13:618), `(video\w*)` (R13:627) or "lower-cased, space-stripped name starts with `video`" (CB:1077, CB:58) (C9) | M1 | [Rec] CB:1077 |
| E4 | Log every engine type seen, once ("encoder-usage.ps1 logs every type") | M1 | R13:618, CB:1077, M1:194 |
| E5 | Rate counters need two collections; instances come and go, so wildcard and filter | M1 | R13:620 |
| E6 | Settle cookbook item 6: AMD's engine-type name and what the owner's account can see (`typeperf -q "GPU Engine"`) | M1 | M1:194, CB:1077, CB:1212, R13:789 |
| E7 | On AMD `VideoCodec` counts decode too: playback and recording look alike; the RX 9000 series has one VCN for both | M1 | R13:618, R13:859, R13:862, M1:195 |
| E8 | Before every harness job, read-only, `-Seconds 3`, shown next to the verdict (PR 9c) | M1 | R16:865, HN:tools/mac/pc:1056–1060, HN:tools/mac/pc:1120 |
| E9 | In parallel with one B run; Instant Replay detection = a non-Kairo process using a video engine >1% for 5 s; SP9 wants 10/10 toggles and 0 false positives | M2+ | R13:835, R13:471, CB:1066, PLAN:435 |
| E10 | Never `.Path`, `.MainModule`, `.Modules`, `Company`, `FileVersion`, `ProductVersion`, `Description` or `Product`: all go through `MainModule` (NET:TypeTable_Types_Ps1Xml.cs:1136–1244) | M1 | CL:9, CL:16 |

**Gaps and recommendations.**
- [Rec] PID → name from `[Diagnostics.Process]::GetProcesses()`, once per sample set: one `NtQuerySystemInformation` snapshot, no process handle (NET:Process.cs:1092–1098, NET:ProcessManager.Windows.cs:1005–1034); read `.Id` and `.ProcessName` only. `Get-Process -Id` goes through `GetProcessById`, which opens `PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE` (NET:Process.cs:1011–1019, NET:ProcessManager.Windows.cs:60–66), more than the letter of CL:9; whether the cmdlet takes that path is [Unverified], since its source isn't in `dl`.
- [Gap] Machine-readable output for PR 9c and the bench. [Rec] one JSON line: `seconds`, `samples`, `types_seen` (every engine type), and per process, pid, LUID and type `avg` and `max`; a table only on request.
- [Unverified] Whether `Get-Counter`'s first sample of a rate counter is valid. [Rec] drop it, so `-Seconds 3` yields 2 samples (or collect 4).
- [Unverified] `Get-Counter` takes localized counter names; the PC's display language isn't recorded here (it's a PC detail). The Rust sampler avoids this with `PdhAddEnglishCounterW` (R13:649).
- [Rec] Run it once elevated and once as the owner (a short job) to answer "what your account can see" (M1:194).
- Its output names every program that uses a video engine (file names): raw output stays in `.dogfood/` (CL:18).

---

## 8. `env.json`

| # | Requirement | Source |
|---|---|---|
| V1 | Freeze the environment and record it: Adrenalin and driver store versions, Windows build; HAGS, HDR, VRR, refresh rates; game settings; Kairo commit and config | R13:819–822, PLAN:353, CB:1078 |
| V2 | Whether AMD Software is installed | M1:87 |
| V3 | RAM (Q1) | PLAN:663 |
| V4 | Driver store version from `IDXGIAdapter1::CheckInterfaceSupport(&IDXGIDevice::IID)` (equal to AMD's "Driver Store Version" is [M]); HAGS from `D3DKMTQueryAdapterInfo(KMTQAITYPE_WDDM_2_7_CAPS)`, diagnostics only; TDRs (4101, LiveKernelReports) | CB:1080, R13:394–424, R13:682, R13:869 |
| V5 | Keeps data from before and after a Windows, driver or Adrenalin update apart | M1:217 |
| V6 | Holds PC details: never published, never committed, `.dogfood/` only | M1:78, M1:87, CL:18 |
| V7 | Related, from the capability-report list: AMF runtime version, VCN count, HDR and BitsPerColor per output, Instant Replay / Record Desktop / Metrics Overlay hints, video-engine users, TDR count | R13:763–771 |

**Gaps.**
- [Conflict] C3: R13:843 publishes it; M1:87 forbids that.
- [Gap] No allowed API reads VRR/FreeSync or the Chill, Boost, AFMF, FRTC, Enhanced Sync and RSR states: they need ADLX, which waits for a licence review (R13:428–433, R13:776). [Rec] registry hints where research names them ([Unverified]: R13:472, R13:374), the owner's checklist answer, and each value's source.
- [Gap] Where to read Adrenalin's version and "installed". [Rec] its uninstall entry, read-only; the exact entry is [Unverified].
- [Gap] Collecting the DXGI and D3DKMT values without `Add-Type` (M1:171). [Rec] reuse `kairo-harness displays` (device paths, refresh, HDR state, adapters' LUIDs, PCI ids and model names: M1:77) and add the driver version and HAGS there or in the bench runner, as the owner; the elevated script reads the rest (registry, CIM) read-only.
- [Rec] Also record: loadgen mode, `iters`, resolution, swap-chain format, tearing support; the monitor alias (the path stays in the job file); build id and commit (lib.ps1:447, lib.ps1:463); PresentMon's hash; the scripts' commit; the power plan GUID (R13:831); Instant Replay hints (HN:tools/mac/remote/harness-pre.ps1:25–28) and `encoder-usage.ps1` before the session; a snapshot at the start and one at the end (a change mid-session invalidates it, V5).
- [Rec] Leave out the PC name, account names, IP and MAC addresses and serial numbers even though the file stays local (M1:221, PLAN:377); CIM's `Win32_ComputerSystem` carries `Name` and `UserName`.

---

## 9. AMD settings diff script

| # | Requirement | Source |
|---|---|---|
| D1 | An M1 deliverable, from research 13 §4.2 | M1:19, M1:131, PLAN:416 |
| D2 | Two calls, before and after the owner toggles, because scripts reach the PC on stdin; prints `path\name: before -> after`; only reads | M1:88, R13:487–488 |
| D3 | Keys: `HKCU:\Software\AMD\DVR`, `HKLM:\SOFTWARE\AMD\DVR`, `HKCU:\Software\AMD\CN`, `HKCU:\Software\AMD\CN\Performance` | R13:479 |
| D4 | Toggles: Instant Replay, Record Desktop, Metrics Overlay | M1:42, R13:485 |
| D5 | Purpose: confirm the Instant Replay values (`InstantReplayEnabled`, `InGameReplayEnabled`, `DvrEnabled`, `DesktopRecordingEnabled`, `ShowRSOverlay`), the Metrics Overlay's `EnableMetricsOverlay`, and the hotkeys (`HKLM\SOFTWARE\AMD\DVR\*Hotkey`, `HotkeysDisabled`) | R13:374, R13:472, R13:509, R13:526, R13:788, CB:59, CB:1066, CB:1213 |
| D6 | Never writes AMD's settings | R13:465, R13:811 |
| D7 | Runs in M1; the owner confirms it in SP9 (10/10 toggles, 0 false positives); the names then go into the bootstrap's probes | M1:142, PLAN:435, M1:184 |
| D8 | HKCU is the SSH account's, the owner's today | R13:476, HN:tools/mac/remote/harness-pre.ps1:27 |
| D9 | Outside the exit-rule clock; AMD Software is installed again (2026-10-24), which the script needs (without it, its check waits and M1 still closes) | M1:110, M1:42, M1:216 |

**Gaps.**
- [Conflict] C10: one `Read-Host` for three toggles (R13:485). [Rec] one before/after pair per toggle, and each setting switched back with its own pair: both directions confirm the value, and the owner's settings end as they began.
- [Gap] False positives. [Rec] start with a null pair (two snapshots, no toggle) to list values that change by themselves.
- [Gap] Value types: `"$($_.Value)"` (R13:483) prints `System.Byte[]` for binary values, so a binary change is invisible. [Rec] hex for binary, joined text for multi-string, with the type.
- [Gap] Subkeys aren't read (R13:479–483). [Rec] recurse under `AMD\DVR` and `AMD\CN`, bounded in depth and count.
- [Gap] Privacy: values can hold paths (Instant Replay records to the media folder, R13:451) or names. [Rec] print a path-like value as `<path>` plus a short hash, so a change still shows; raw snapshots stay in `.dogfood/` (CL:16, CL:18).
- [Gap] Where the "before" snapshot lives: the elevated session can't write `logs\` (M1:81). [Rec] the Mac keeps it in `.dogfood/` and computes the diff.
- [Gap] D8 breaks if Claude's SSH sessions move to a separate admin account, as M1:172 recommends: then read the owner's hive under `HKU\<owner SID>`, or run as an owner job.

---

## 10. Conflicts and gaps

| # | Sources | Conflict | Resolution |
|---|---|---|---|
| C1 | R13:545, CB:1072 (`C:\kairo\bench\$Tag.csv`); PLAN:353, R13:819, CB:1078 (`bench/env.json`); R13:843 (`bench/<date>-amd.md`) vs M1:48, M1:75, M1:78 | Where outputs go | M1: job output in `C:\kairo\logs\`, raw results in `.dogfood/`, the summary as aggregates in the gate report (M1:221) or `docs/spikes/M2-report.md` (PLAN:423) |
| C2 | R13:543, CB:1072 vs M1:74, boot:174 | PresentMon's path | `C:\kairo\tools\presentmon\` |
| C3 | R13:843 vs M1:87, CL:18 | Publishing env.json | Never |
| C4 | R13:829 ("the `schtasks /it` task") vs CB:54, PLAN:332, M1:48, M1:97 | Launcher | `\Kairo\Run`, normal priority |
| C5 | R13:705 ("the 360 Hz OLED"), PLAN:352 and M1:13 ("borderless on the OLED") vs M1:40 (answered) | Loadgen's monitor | The 180 Hz monitor in M1; M1:13 and PLAN:352 still carry the old text |
| C6 | M1:97, CB:1174 vs M1:81, lib.ps1:95–98, M1:75 | Elevated PresentMon writing into `logs\` | §4; [Rec] option (a) |
| C7 | R13:578 vs M1:81, lib.ps1:116–131 | Elevated `Import-Csv` of an owner-writable file | Checked handle, or run as the owner |
| C8 | R13:631 vs M1:86, CL:9, CL:16 | `$p.Path`: a raw path and a module list | Names from `GetProcesses()` |
| C9 | R13:618 vs R13:627 vs CB:1077 | Engine-type match | CB:1077, and log every type |
| C10 | R13:485 vs M1:88 | One `Read-Host` for three toggles | A pair of calls per toggle |
| C11 | R13:605 vs PLAN:353 | PASS clauses | The plan's clauses join before SP1 (M1:85) |
| C12 | CB:1078 ("CV ≤1%") vs M1:29, PLAN:353 ("CV >1% → more runs"), R13:831 ("fix ... or increase the run count") | What the A/A requires | Gate 4 = data; stopping rule [Rec] in §3 |
| C13 | PLAN:353 vs R13:823–828, CB:1078 | Which AMD and recording features are off; "Instant Replay alone + 1 pp" | Reconcile before SP1 (M1:201) |
| C14 | R13:743 (test 14, `vsrc_amf`) vs CL:8, M1:100 | Capture path | Never run it |
| C15 | R13:704 ("fully automatable over SSH") vs M1:97 | Loadgen's launch | `pc bench` → `\Kairo\Run` |
| C16 | CB:1144 (`C:\kairo\tools\bench.ps1` on the PC) vs M0:82, M1:92 | Where scripts live | Sent on stdin from `tools/mac/remote/` |
| C17 | R13:557 (`TimeInMs`) vs PM:CsvOutput.cpp:556–560 | Column name under `--qpc_time` | `TimeInQPC`; parse by header |
| C18 | R13:750 ("§7 checklist") vs R13:823–828 | Cross-reference | The AMD checklist is §9 step 2 |
| C19 | M1:35 (leave the screen-off timer; a static OLED desktop for hours) vs TB:platform.rs:88–105 and R16:775 (hold `ES_DISPLAY_REQUIRED`), at a 25–45-min session | Displays during a bench session | O2 |

| Gap | Section | [Rec] in brief |
|---|---|---|
| Loadgen CLI, exit codes, log | §2 | Testbed's conventions |
| `iters` tuning; bandwidth cost; determinism check | §2 | Calibrate once, pass fixed `--iters`; ≈8 ms both modes; self-log vs PresentMon |
| "Workload" in M1 | §3 | Each loadgen mode |
| CV definition and stopping rule | §3 | Sample SD; 6 + 6 once; report |
| Warm-up scope; A/A spacing | §3 | Per workload; A/B timing; fresh process per run |
| Job shape; one job at a time | §3 | One runner job per session |
| Own window reads BUSY; 10-min IDLE | §3 | Owner's "go", `--force`, runner watches `foreground_exe` |
| Driver caps; mixed present modes | §3 | Flag runs from the data [proposed] |
| Results layout; CSV fetch size | §3 | `.dogfood/bench/<stamp>/`; block fetch |
| `pc bench` exit codes; P0 check | §3 | `pc run`'s + 7 |
| PresentMon naming with a limited token; stderr; cut rows | §4 | Check first run; keep stderr; ignore partial row |
| pm-stats fields, frame types, definition key | §5 | Extra keys; `Application` only; `low_def` |
| t-table holes; clause definitions; input form | §6 | Full table; [proposed] definitions; JSONL |
| encoder-usage output form, first sample, localization, visibility | §7 | One JSON line; drop sample 1; check once; run both ways |
| env.json unreadable fields; versions; collection without `Add-Type` | §8 | Hints with sources; reuse `kairo-harness displays` |
| Diff script types, subkeys, volatility, privacy, storage, HKCU | §9 | See §9 |
| HDR loadgen for M2's HDR run | §2 | M2 |

---

## 11. Owner items

Each comes with a recommendation (CL:42).

- **O1. An attended bench window** [Owner]. The M1 session is ≈25 min, up to ≈45 min with extra runs [Estimate], longer than M1:103's "short jobs". The owner is home, says "go" in chat and stays off keyboard and mouse; any touch stops the job (exit 6), and no game with a controller meanwhile, since that neither counts as input nor stops the job (M1:103). Claude starts with `--force` on that OK (pc:286–288, R16:774). Recommendation: one evening window, shared with other attended M1 runs if convenient (R16:949–953).
- **O2. The OLED during the session** [Owner]. Holding the displays on keeps the OLED on a static desktop for the whole session (M1:35); not holding them lets the timer blank the 180 Hz monitor mid-run. Options: (a) the owner switches the OLED off at its button for the session, a physical action; whether Windows then removes it from the desktop is [Unverified] (M1:206); (b) the bench runner puts a black, capture-excluded window on the OLED only, never taking focus, for the session; this is the runner's planned cover (M1:103, M1:135, PLAN:334) brought forward; (c) wait for the runner's cover. Recommendation: (b), with (a) for any session before (b) exists. Whether a black window on the other monitor changes loadgen's present mode is [Unverified]; the first run's `present_modes` shows it.
- **O3. Instant Replay and AMD's frame-rate features during the M1 A/A** [Owner]. M1 needs none of it (M1:201), but Instant Replay encodes and writes to disk all the time (R13:451), the M2 gate is measured with it off (PLAN:353), and Chill or FRTC would cap an unattended loadgen (R13:366). Recommendation: the owner switches Instant Replay off for the session and back on afterwards (Kairo never toggles it, R13:465), and confirms once that Chill, Boost, FRTC, AFMF, Enhanced Sync, RSR and the Metrics Overlay are off globally. Otherwise env.json records the hints and the floor is labelled "Instant Replay on".
- **O4. The diff-script toggles** [Owner]. About 10 minutes at the PC: Claude snapshots, the owner flips one setting in AMD Software and says so in chat, Claude snapshots again; Instant Replay, Record Desktop and the Metrics Overlay each on and back off, after a null pair. Recommendation: in the same visit as O1; the Metrics Overlay only once each way, given Adrenalin 26.10.1's overlay toggle timeouts (R13:374, R13:740).
- **O5. Noise fixes.** If CV stays above 1%, research suggests the power plan and background tasks (R13:831): PC changes outside Kairo's folders (CL:43). Recommendation: change nothing in M1; report the CV.
- **Already open elsewhere:** test days (M1:35) before any away run; Claude's separate SSH account (M1:172), which changes where the diff script finds the owner's HKCU (§9).

---

## 12. Already built and reusable

| Piece | What the bench takes from it | Source |
|---|---|---|
| `kairo-display` | Monitor by device path, adapter by LUID, PCI ids; the privacy note on paths and names | display:1–20, display:239, display/platform.rs:350, display/platform.rs:411 |
| Job contract and `kairo-devrun` | Job fields, `priority`, input and time stops, results, whole-tree stop | job.rs:15–67, M0:72–74, M1:81 |
| `tools/mac/pc` | Verb plumbing over stdin, job writing, `--force`, exit codes | pc:286–332 |
| `remote/lib.ps1` | `Open-OwnedFile`, `Read-SharedText`, `Write-TextFileAtomic` (admin-only folders), `Get-KairoWatch`, `Assert-PcIdle`, `Start-KairoJob`, `Wait-KairoJob`, build id and commit | lib.ps1:95–170, lib.ps1:283, lib.ps1:354, lib.ps1:447, lib.ps1:463, lib.ps1:561, lib.ps1:617 |
| Testbed (PR #31) | Borderless window by path, LUID device, flip-discard swap chain, `KeepDisplayOn`, ready file, `--stop-on-stdin-eof`, exit codes | TB:cli.rs:10–36, TB:gpu.rs:1–9, TB:gpu.rs:131–146, TB:platform.rs:88–105 |
| Harness (PRs #34–35) | One-job runner with children and a run folder; pruning (newest 5); Instant Replay hints before a job; the 4101/41 check (exit 7); block fetch | HN:crates/kairo-harness/src/run.rs:1–7, HN:crates/kairo-harness/src/prune.rs:1–6, HN:tools/mac/remote/harness-pre.ps1:1–28, HN:tools/mac/remote/harness-events.ps1:1–6, HN:tools/mac/remote/harness-fetch.ps1:1–11 |
| Doctor | PresentMon hash and group checks; running PresentMon as the owner (S4U) | doctor:438–505, doctor:674–693 |
