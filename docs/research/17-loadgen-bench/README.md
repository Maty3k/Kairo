# kairo-loadgen, `pc bench` and the bench scripts: design (M1 gate 4)

> **About this folder.** M1's bench half is built from this design: `kairo-loadgen`, `pc bench`, `pm-stats.ps1`, `ab-stats.ps1`, `encoder-usage.ps1`, `env.json` and the AMD settings diff script (M1:13, M1:17, M1:19, M1:29). It merges the four research notes beside it (`requirements.md`, `loadgen.md`, `presentmon.md`, `pc-tooling.md`) and settles where they disagree. The **[Mac]** numbers come from Python scripts in the Mac's scratch folder, which are **not** in the repo; §6's test vectors are their record. For this merge, the ab-stats and A/A vectors, the t table, §4.7's CV table and §3.11's luminance, red-ratio and per-pixel figures were recomputed independently, and the pm-stats vectors were checked by hand. Three critiques (feasibility, completeness, rules) then reviewed it; the last section lists each issue, whether it held, and what changed.

*2026-11-11 · Written against `main` at ddec2c9. The testbed is PR #31 read at f88ed79 (its branch has two later commits, in pace.rs and after line 522 of run.rs, which move no line cited here); the harness is PRs #34–#35's `m1/harness-run` at 1cbfa9f. Both were unmerged and were read in scratch copies. Nothing in the repo was changed, built or run, and the PC was not touched. The design started on 2026-11-11, when PresentMon's and .NET's sources were downloaded; the four notes and this merge were written on 2026-11-14, the date of every live check. The downloads were read as data and never run: all 36 PresentMon files hash to the git blobs of tag v2.6.0 (commit e13fce6, a 1,103-entry tree, not truncated).*

*Revised on 2026-11-22 after the critiques. The scratch folder's cleanup had emptied the downloads by then, so the critique fetched PresentMon again at v2.6.0, and PowerShell's and .NET's files, which hash to the git blobs of PowerShell v7.6.7 and dotnet/runtime v10.0.0 [Live, 2026-11-22]. That evening #31 merged as 0c6018b: its testbed files equal f88ed79's except run.rs after line 522 and pace.rs, so every tb/ citation holds on `main`. The harness branch was rebased onto it as 7c994e8, where only `kairo-core`'s testsig.rs differs from 1cbfa9f, so every hn/ citation holds too. M1 and R16 line numbers stay at ddec2c9: #31 added a line to M1 after line 89, so M1:90 and later read one higher on `main`, and R16's lines moved; both get re-based when this document is committed. Claude implements from it; the owner can skim §1, §2 and §8.*

**Citation keys.**

| Key | File |
|---|---|
| `CL:n` | CLAUDE.md |
| `PLAN:n` | docs/PLAN.md |
| `M0:n`, `M1:n` | docs/milestones/M0.md, M1.md |
| `R01`, `R08`, `R13` `:n` | docs/research/NN-*.md |
| `CB:n` | docs/research/15-implementation-cookbook.md |
| `R16:n` | docs/research/16-verify-testbed/README.md |
| `core/x.rs:n`, `devrun/x.rs:n`, `display/x.rs:n` | crates/kairo-core/src, crates/kairo-devrun/src, crates/kairo-display/src (`main`) |
| `pc:n`, `lib:n`, `fetch:n` | tools/mac/pc, tools/mac/remote/lib.ps1, tools/mac/remote/harness-fetch.ps1 (`main`) |
| `boot:n`, `doctor:n` | tools/pc/bootstrap.ps1, tools/pc/doctor.ps1 |
| `ci:n`, `Cargo:n` | .github/workflows/ci.yml, the workspace Cargo.toml |
| `tb/x.rs:n` | crates/kairo-testbed/src, read on PR #31 at f88ed79; every cited line is the same on `main` (0c6018b) |
| `hn/x.rs:n`, `hn/pc:n`, `hn/lib:n`, `hn/remote/x.ps1:n`, `hn/devrun/run.rs:n` | the harness branch (1cbfa9f, rebased as 7c994e8 with the same files): crates/kairo-harness/src, tools/mac/pc, tools/mac/remote, crates/kairo-devrun/src |
| `PM/x:n` | PresentMon v2.6.0: `PM/MainThread.cpp` and its neighbours for the `PresentMon/` folder, `PM/PresentData/…`, `PM/mc/…` for `IntelPresentMon/CommonUtilities/mc/…`; `PMR:n` its README-ConsoleApplication.md, `PMREADME:n` its README.md |
| `NET/x.cs:n`, `PS/x.cs:n` | dotnet/runtime v10.0.0's `System.Diagnostics.Process` sources (Process.cs, Process.Windows.cs, ProcessManager.Windows.cs, ProcessManager.Win32.cs) and PowerShell v7.6.7's TypeTable_Types_Ps1Xml.cs, GetCounterCommand.cs and PdhHelper.cs. Each hashes to its tag's git blob [Live, 2026-11-22]. PowerShell 7.6.7 builds on the .NET SDK 10.0.402 (its global.json), whose runtime is a 10.0 servicing release [M: the same Process code] |
| `win/…:n`, `wlink:n` | `windows` 0.62.2 `src/Windows/…`, `windows-link` 0.2.1 `src/lib.rs` (the Mac's cargo registry) |
| `wincheck/…:n` | docs/research/prototypes/wincheck |
| `MS-…`, `PS-Measure`, `WALBOURN`, `DXC`, `GH` | the pages under Sources |

**Evidence labels.**
- **[MS]**: what a Microsoft page says (Sources).
- **[Live]**: checked against GitHub's API on 2026-11-14, or on the date given.
- **[Mac]**: computed on the Mac in Python with IEEE doubles, the arithmetic .NET uses.
- **[M]**: a source of unknown version, or well-known behaviour not re-read here.
- **[Unverified]**: not yet checked; §9 says how each one gets settled.
- **[proposed]**: Claude's choice, to confirm in review or with PC data.
- **[Estimate]**: a planning number, not a measurement.

---

## 1. What the owner gets, and what it needs from them

**What you get.**
- **`kairo-loadgen`**: a GPU load that is identical from run to run, for measuring what Kairo costs a game. It comes in two kinds: shader maths (ALU) and memory reads (bandwidth). Each takes about 8 ms of GPU work per frame, runs uncapped, and sits borderless on the 180 Hz screen (your answer, M1:40). It shows a dim, slowly drifting colour field with faint moving grain. Nothing flashes.
- **`pc bench`**: it warms the GPU up for 2 minutes, then runs the load 6 times per kind while PresentMon records every frame, and turns each recording into one line of numbers. M1 asks how much identical runs differ (the "noise floor"); M2 reuses the same tool to compare Kairo off and on.
- **The scripts M1 lists**:
  - `pm-stats.ps1`: one recording → one line.
  - `ab-stats.ps1`: the comparison with its 95% interval.
  - `encoder-usage.ps1`: which programs use the video encoder, also shown before every harness run.
  - `env.json`: what was installed and set during a session; it stays in `.dogfood/`.
  - The AMD settings diff script, which finds where AMD Software keeps Instant Replay's on/off.
- **Gate 4's evidence**: one line with aggregates and the hardware class only, for example "A/A on kairo-loadgen, RX 9000-series, 180 Hz SDR: ALU 6 runs, CV x%; bandwidth 6 runs, CV y%; Hardware: Independent Flip z%; PresentMon 2.6.0" (M1:221). It sits beside the line of gate 4's other half, the `audio-soak` dry run (M1:29).

**Not in this design**: gate 4's `audio-soak` dry run; `wgc-probe`; `kairo-devrun`'s safety stops (M1:135), which reuse two pieces of it (§7); M2's A/B runs and game benchmarks, which §4.12 only prepares; research 16's PR 9c, which reuses `encoder-usage.ps1`.

**What it needs from you** (details and recommendations in §8).
1. **One evening, about 1 to 1½ hours** (Q1, Q2):
   - about 10 minutes at the PC switching three AMD settings one at a time while Claude reads the registry;
   - about 20 minutes off the keyboard and mouse for loadgen's first checks, a test of the bench's stops among them;
   - then, if you agree, about 30 minutes (up to about an hour if the numbers are noisy) for the full noise-floor run. Stay home, off the keyboard and mouse, and start no game, whether with keyboard, mouse or controller, until Claude says it's done. The bench keeps the OLED black. It stops itself when another program comes to the front, a fullscreen window appears or the PC locks, and at any touch of keyboard or mouse. A game that never comes to the front wouldn't stop it.
   - For that evening, Dynamic Lock and any screen saver off if they're on (Q2): either would stop or spoil the run.
2. **AMD settings** (Q3): Instant Replay off for that run and back on after (Claude reminds you, whatever happens), and one look that the settings that change the load are off: Chill, Boost, FRTC, AFMF, Enhanced Sync, RSR, Anti-Lag, "Wait for Vertical Refresh" and the Metrics Overlay.
3. **The black cover** (Q4): whether the bench's cover over the OLED is also hidden from screen capture, as M1 describes the runner's; Claude recommends not.
4. Nothing to install, buy or sign in to. PresentMon is already on your PC (M0). Claude changes no Windows or AMD setting: the switches above are yours.

---

## 2. Decisions

Each item gives the decision, then its reason and sources. **Conflict** marks a place where the notes or the older sketches disagree and says which way this design goes.

### Code and tooling

1. **Homes.**
   - `crates/kairo-loadgen`, binary `kairo-loadgen.exe` (PLAN:297, M1:57), which `pc deploy` already copies (`kairo*.exe`, M1:81).
   - `pc bench`'s PC side is a new verb, `kairo-harness bench`: one `\Kairo\Run` job per session that starts loadgen and PresentMon as its children, the way `kairo-harness run` starts the testbed and FFmpeg (hn/run.rs:1–7).
   - `tools/pc/bench/`: `pm-stats.ps1`, `ab-stats.ps1`, `encoder-usage.ps1`, `bench-rules.ps1` (run validity, the extra-runs proposal and the before/after comparison, §4.7) and `regdiff.ps1` (the diff script's functions). Each is a library of functions with its own command line, and CI parses them with Windows PowerShell 5.1 and PowerShell 7 (ci:226–247). The `pc` verbs send their text inside the remote script on stdin (M1:92), where the elevated session dot-sources it, so nothing on the PC's disk is ever run elevated.
   - **Dot-sourcing safely** [proposed]. Dot-sourced text runs its top level inside the elevated session, under lib.ps1's `Set-StrictMode -Version 3.0` and `$ErrorActionPreference = 'Stop'` (lib:21–22), and an `exit` there would end the session without the `@@KAIRO-EXIT` line the Mac reads (lib:77–81). So the functions only return or throw, never `exit`; the command-line body (parameter checks, file reading, exit codes) runs only when `$MyInvocation.InvocationName -ne '.'`; and no parameter is `Mandatory`, since `-NonInteractive` can't prompt. run-doctor's `LocalB64` isn't the model: it writes a file in admin-only `C:\kairo\tools` and runs it with `&` after `Set-StrictMode -Off` (tools/mac/remote/run-doctor.ps1:10–19, :32–40).
   - `tools/mac/remote/`: `bench-pre.ps1`, `bench.ps1`, `bench-fetch.ps1`, `bench-cleanup.ps1` and `amd-diff.ps1`, the verb bodies that need `lib.ps1`.
   - **Conflict:** the notes put the scripts in `tools/bench/`, `tools/pc/bench/` and `tools/mac/remote/`. `tools/pc/` already holds the PC-side scripts (bootstrap, doctor), and `tools/mac/remote/` holds verb bodies.
   - Each name joins M1's contract in the PR that adds it (M1:50).
2. **A shared Windows crate**, extracted from the testbed [proposed; Claude names it at implementation, `kairo-window` here]. #31 merged on 2026-11-22 (0c6018b), so nothing waits for it.
   - It holds the window, the D3D11 device on a LUID, QPC, the EcoQoS opt-out, `KeepDisplayOn`, the foreground wait, the stdin-EOF watcher, the create-new ready writer, and the new cover (item 25).
   - Its shape is the testbed's: `#![deny(unsafe_code)]`, then `#[cfg(windows)] #[allow(unsafe_code)] mod gpu;` and `mod platform;` (tb/lib.rs:15–28), with the cover in `platform` (CL:32).
   - Why: about 600 lines would otherwise exist twice. `KeepDisplayOn` already does (tb/platform.rs:88–105, hn/platform.rs:26–44).
   - It is a pure move, so #31's WARP tests stay as they are (tb/gpu.rs:835–948).
3. **Shaders compile at run time** with `D3DCompile` from the inbox `d3dcompiler_47.dll`, as the engine does (CB:377): HLSL embedded with `include_str!`, `vs_5_0`/`ps_5_0`, optimisation level 3, strict, warnings as errors.
   - `windows-link` imports the DLL raw-dylib, so the Mac never needs an import library (wlink:17–26).
   - Rejected: `dxc` (it compiles shader model 6.0 and later to DXIL only, [DXC], [MS-TARGETS]); `fxc.exe` in `build.rs` (two build paths); committed `.cso` files (opaque binaries in a public repo).
4. **House rules carried into the code**:
   - `windows` =0.62.2 (CL:30);
   - `unsafe` only in loadgen's `gpu` module and the shared crate's `gpu` and `platform` modules, each block with `// SAFETY:` (CL:32; `undocumented_unsafe_blocks = "deny"`, Cargo:48);
   - no `unwrap` on runtime paths (CL:32) and no allocation per frame (PLAN:316): the `f` line is formatted into a buffer reused for the whole run;
   - no new third-party crate, so no deny.toml change;
   - no GPL or OBS shader code (CL:17).

### The load

5. **Borderless on the 180 Hz monitor**, picked by device path, never as "the main screen" (M1:38, M1:64).
   - **Conflict:** R13:705, PLAN:352 and M1:13 say "on the OLED". Your answer (M1:40) wins, and loadgen's PR rewrites M1:13 and PLAN:352.
   - In M1 `pc bench` refuses the `oled` alias for the load, as `pc harness` does for capture (R16:861).
6. **Swap chain**: `FLIP_DISCARD`, `B8G8R8A8_UNORM`, 3 buffers, `ALLOW_TEARING | FRAME_LATENCY_WAITABLE_OBJECT`, latency 2, `NO_ALT_ENTER`, and every frame `Present(0, DXGI_PRESENT_ALLOW_TEARING)` (M1:57, R13:705, [MS-PRESENTFLAGS]).
   - Without tearing support it exits 3. It never falls back silently.
   - Latency 2 keeps one frame queued, so the GPU never waits for the CPU. At latency 1, the gap between frames would move with CPU scheduling, which Kairo's own threads change (§3.4).
7. **`--activate` for bench runs** [proposed]: the window takes the foreground, so the shell hides the taskbar on that monitor [Unverified] and nothing sits over the window.
   - Microsoft doesn't say independent flip needs that: when other content comes on top, DWM "can either seamlessly transition back to composed mode, efficiently 'reverse compose' the contents on top of the application before flipping it, or leverage MPO to maintain the independent flip mode" [MS-FLIP]. What activation buys on this PC is for the rehearsal pair (§9 risk 3).
   - It first waits until the last input is older than the foreground lock time-out plus 5 s (tb/run.rs:410–444).
   - The job's input stop means no input can come later, so a session pays that wait (at most about 205 s) once.
   - **Conflict:** the PC-tooling note showed the window without activation, fearing a 205 s wait per run. The first attended rehearsal compares both (§9 risk 3).
8. **Two workloads with explicit counts**: `run --workload alu --iters N` or `run --workload bw --reads N`.
   - Both target about 8 ms of GPU per frame (R13:707 for ALU; the same for bandwidth [proposed]). Neither then runs at thousands of fps, where PresentMon's 2,048-present ring overflows (PM/MainThread.cpp:455–459) and CSVs balloon.
   - A separate `calibrate` finds the counts; nothing changes inside a run.
   - **Conflict:** the requirements note wrote `--mode alu|bandwidth`. `--mode` is the testbed's window mode and its ready file's `mode` field (tb/ready.rs:32–33), so the flag is `--workload`.
9. **16 scissored bands per frame** [proposed]: each draw takes about 0.5 ms at the target, 4,000× below the 2 s TDR limit whatever the GPU's preemption granularity ([MS-TDR], [MS-PREEMPT]). Other GPU work interleaves every half millisecond, as it does between a game's many draws.
10. **The picture** is a dim, slowly drifting colour field with low-contrast grain: a pure function of the pixel and QPC time.
    - R13:713–723's shader takes every pixel through saturated, full-range hues once a second.
    - Loadgen keeps that kind of work and replaces the output (§3.11).
11. **A fresh loadgen process for every run** (all three notes agree): the A/A then includes the launch-to-launch variation (present-mode negotiation, clocks) that separate A and B runs will have.

### Measuring

12. **PresentMon runs as you, inside the bench job, and writes its CSV in the run folder** [proposed].
    - M1:97 lets PresentMon run from SSH. But an elevated PresentMon with `--output_file` in `C:\kairo\logs\` creates or empties a file there with an administrator's token (`_wfopen_s(…, L"w,ccs=UTF-8")`, PM/CsvOutput.cpp:1421–1427; `w` mode destroys an existing file, [MS-fopen]), and a link planted at that name would redirect it [M]. M1:81 rules that out.
    - `C:\kairo\bench\` no longer exists (M1:48).
    - A sequencer running over SSH dies with the connection or a sleeping Mac.
    - Your account is in Performance Log Users, and PresentMon recorded with your limited token in M0 (M0:11, M0:166). Unelevated, PresentMon only warns that it lacks SeDebugPrivilege and carries on (PM/MainThread.cpp:255–277).
    - The bench PR rewords M1:97: "PresentMon may run from SSH only when it writes no file; a run that writes a CSV runs as you, inside the bench job."
    - Fallback, if the first rehearsal shows the job's token can't record: the elevated session with `--output_stdout`, which writes no file (§4.2). PresentMon then prints neither recording line, which need Simple console output that `--output_stdout` turns off (PM/MainThread.cpp:110, :145; PM/CommandLine.cpp:556–557), so those runs have their own rule 1 (§4.7), and each run's `route` is recorded.
13. **PresentMon targets loadgen by PID** (`--process_id`, the child's PID, checked against `ready.json`).
    - The PID is an early filter (PM/MainThread.cpp:322–325), names can't be ambiguous, and it matches even where a name reads `<unknown>`.
    - The PID is validated as a positive integer first. `--process_id 0` means no target, and PresentMon then records every program that presents (PM/OutputThread.cpp:127–130).
    - `--process_name` waits for M2's games.
14. **PresentMon's flags**: M1:74's four, plus:
    - `--process_id`, `--output_file`, `--stop_existing_session` and `--delay 5 --timed 60 --terminate_after_timed --no_console_stats` (R13:542–548);
    - `--no_track_input` [proposed]. Valid runs have no input, so its two columns would always read `NA`, and it keeps every trace of your keyboard and mouse out of the data (PM/PresentData/PresentMonTraceSession.cpp:889–893). M1:74 gains it.
    - **Never passed**: `--scroll_indicator` (it calls `SendInput`, PM/MainThread.cpp:42–77; CL:10), `--restart_as_admin` (a UAC prompt through `runas`, PM/Privilege.cpp:124–134), `--hotkey`, `--exclude_dropped`, `--multi_csv`, `--date_time`, `--v1_metrics`, `--v2_metrics` (§4.5).
15. **The protocol**, per workload:
    - a discarded 120 s warm-up, then 6 runs of 60 s with `--delay 5`, with 30 s of idle before every run (R13:830–833);
    - the warm-up is per workload [proposed], because ALU and bandwidth load reach different clocks and temperatures;
    - CV = sample SD ÷ mean of the runs' `avg_fps`. A CV above 1% gets 6 more runs once, with their own warm-up, in the same job after its one decision point (§4.6). Still above 1% at 12 runs, it is reported as a setup finding, and the bench never loops.
    - Gate 4 passes on data either way (M1:29, PLAN:417).
    - **Conflict:** CB:1078 writes "CV ≤1%" like a pass rule, and R13:831 says "fix the noise first ... or increase the run count". M1's "produce data" governs, and the extension is bounded.
16. **One implementation of the statistics.**
    - `pm-stats.ps1` and `ab-stats.ps1` run in the elevated session over bytes read through the owner check (`Open-ReadFile`, lib:133–137) and decoded by their BOM, never with `Import-Csv` on a path in `logs\` (R13:578 vs M1:81).
    - `bench-rules` proposes the extra runs from `ab-stats`' JSON and its own, and the Mac applies them at the job's decision point (§4.6). The logic lives in PowerShell that CI tests, not in the Mac's bash.
    - **Conflict:** the PC-tooling note had the Mac compute the CV itself.
17. **Validity is objective and decided before the statistics** (§4.7). A run counts only if:
    - PresentMon exited 0 after printing both of its recording lines, with no ETW-loss warning, and loadgen ran until PresentMon ended (the job sees these, and re-runs the slot);
    - the CSV parses with no bad row and one swap chain, and holds the timed length ± 1 s;
    - every frame is an `Application` frame, none is longer than 250 ms, and at most 1% of presents were dropped;
    - one present mode covers at least 99% of frames, and it is the same mode in every valid run of the workload in the session.

    Nothing is trimmed, and no outlier is removed.

### Scripts

18. **`pm-stats.ps1`** (§5.1):
    - dedupes rows to presents: frame generation gives one row per displayed instance (PM/mc/MetricsCalculator.cpp:181–234);
    - picks the main swap chain deterministically;
    - computes nearest-rank lows in integers;
    - writes present modes and frame types as count objects; R13:592's `{0:P1}` strings follow the PC's culture;
    - adds diagnostic keys.
19. **`ab-stats.ps1`** carries the full t table for df 1–30 and a series beyond (§5.2).
    - R13:600's table covers n = 2–10, 12 and 16 only. For any other n, `$t975[$n]` is `$null`, the interval collapses to the mean, and PASS rests on the mean alone.
    - It also takes pm-stats lines, has an A/A mode and formats invariantly.
20. **`encoder-usage.ps1`** names processes from one `[Diagnostics.Process]::GetProcesses()` snapshot, reading `Id` and `ProcessName` only, and samples with one `Get-Counter` call per second (§5.3).
    - **Conflict:** R13:631 prints `$p.Path`, which PowerShell defines as `$this.Mainmodule.FileName` (PS/TypeTable_Types_Ps1Xml.cs:1142).
    - `MainModule` opens the process with `PROCESS_QUERY_INFORMATION | PROCESS_VM_READ` and enumerates its modules (NET/Process.Windows.cs:191–200, NET/ProcessManager.Win32.cs:67–81). Against a game that is a module list with memory-read rights (CL:9), and it prints a raw path (CL:16).
    - A game can be in that list: on AMD, video decode counts as `VideoCodec` too (R13:618).
21. **`env.json`** is read elevated and read-only: named registry values, CIM properties and `powercfg`, never a whole key or object (§5.4). The display facts come from the bench job's own reads (§4.8). It is taken before and after each session and compared.
    - **Conflict:** R13:843 publishes it with the bench summary; M1:87 and CL:18 say never. It stays in `.dogfood/`.
22. **The AMD diff script**:
    - runs as two calls per toggle (M1:88), starting with a null pair;
    - switches every setting back afterwards;
    - reads whole AMD subtrees with value types, hashes path-like strings and key or value names, and never writes.
    - **Conflict:** R13:485 takes all three toggles behind one `Read-Host`. That can't work over stdin, and one diff can't say which value belongs to which setting.

### When it runs

23. **Attended runs on your "go".** Loadgen's first checks, the calibration, the stop test and the rehearsals take 2–6 minutes each. They run while you're home, have said "go" in chat, and keep off the keyboard and mouse. Claude starts each at once with `--force`, which needs that OK (pc:6–8), as for gate 2 (R16:773–777).
    - **What `--force` does for `pc bench`** [proposed]: it only lets a job start although your last input is less than 10 minutes old. Unlike `pc build`'s, which turns off both stops (tools/mac/remote/build.ps1:149, :161; M1:81), it never turns a stop off: the job file always has `abort_on_input: true`, the job's own stops (item 25) run whatever the flag says, and a fullscreen window in front refuses even with it (hn/lib:355).
    - The job's one decision point re-checks the busy state without `--force` (§4.6), as M1:96 asks before each long step.
    - A job stopped by your input (exit 6) spends your "go": the next job needs a new one.
24. **The full A/A** (about 26–30 min, up to about an hour with the extra runs, and a hard limit of about 97 min, §4.3) isn't one of the "short jobs" M1:103 allows while you're home.
    - It runs either in one attended exception you agree to (Q1), recorded in M1:103 with its date and scope like the 2026-10-31 build exception, or while you're away on agreed test days once `kairo-devrun`'s safety stops exist (M1:35, M1:135).
    - **Conflict:** the requirements note recommended an attended window with the OLED cover brought forward; the PC-tooling note recommended waiting for devrun's stops (pc-tooling.md:488). This design asks, and recommends the attended evening (Q1), because the bench brings its own game-start stop (item 25).
25. **The bench job's own safety stops** [proposed], all inside `kairo-harness bench`:
    - **Fullscreen and a stale file:** it reads `watch.json` every second and ends the session (exit 6) when another program's fullscreen window comes to the front (a fullscreen sample without a `foreground_exe` counts as another program's), or when the file goes stale.
    - **Foreground**, for what the other stops miss: a windowed game, which `watch.json` doesn't report (core/watch.rs:25–30, M1:96), and a game started with a controller, which devrun's input stop never sees (M1:103). Every second it reads `GetForegroundWindow` and `GetWindowThreadProcessId`, which open no process and read no title, and ends the session when a program other than loadgen comes to the front during a run, or one that had no window when the session began (§4.3).
    - **Lock and Remote Desktop:** setup refuses a Remote Desktop session (`GetSystemMetrics(SM_REMOTESESSION)`, [MS-SYSMET]) and a locked one (`LogonUI.exe` running, M1:166 [Unverified]), and the session ends if `LogonUI.exe` appears.
    - **Covers:** for its whole life it covers every other active display, the OLED included, with a black, non-activating window. That is the cover PLAN:334 plans for the runner; it is built in the shared crate, so devrun's safety-stop PR reuses it. Whether the bench's cover is also capture-excluded is Q4.
    - These stops hold after an SSH drop, unlike the Mac's `StopWhen` (lib:617–656).
    - Your input stays `kairo-devrun`'s stop (`abort_on_input`).

### What this replaces in the older sketches

| Sketch | Here | Why |
|---|---|---|
| `C:\kairo\bench\$Tag.csv`, `bench/env.json`, `bench/<date>-amd.md` (R13:545, R13:819, R13:843, CB:1072) | `C:\kairo\logs\bench-<stamp>\`; raw results in `.dogfood/bench/`; aggregates in the gate report | M1:48, M1:75, M1:78, M1:221 |
| `C:\kairo\tools\PresentMon-2.6.0-x64.exe` (R13:543) | `C:\kairo\tools\presentmon\…` | M1:74, boot:169–174 |
| Launch through the `schtasks /it` task (R13:829); "fully automatable over SSH" (R13:704) | `\Kairo\Run` at normal priority | CB:54, PLAN:332, M1:97 |
| PresentMon from SSH by `--process_name` (R13:542, CB:1174) | Inside the job, by PID | Items 12–13 |
| `TimeInMs` (R13:557) | `TimeInQPC` under `--qpc_time` (PM/CsvOutput.cpp:558) | Columns are read by header |
| `Import-Csv -LiteralPath` (R13:578) | Bytes read through the owner check, decoded by their BOM | M1:81 |
| `$p.Path` (R13:631) | `ProcessName` from `GetProcesses()` | Item 20 |
| One `Read-Host` for three toggles (R13:485) | Two calls per toggle | M1:88 |
| `env.json` published (R13:843) | `.dogfood/` only | M1:87 |
| Engine types `^Video(Encode\|Codec)$` (R13:618) or `video\w*` (R13:627) | Lower-cased, spaces removed, starting with `video`; every type counted | CB:1077, M1:194 |
| One `Get-Counter … -MaxSamples $Seconds` call over the wildcard (R13:620, R13:625) | One `-MaxSamples 1` call per second; samples with a non-zero `Status` dropped and counted | One call expands the wildcard once, so later users go unseen, and a program that exits mid-sample raises an error (§5.3) |
| One warm-up (R13:830) | One per workload | Item 15 |
| Test 14, `vsrc_amf` (R13:743) | Never run | CL:8, M1:100 |

---

## 3. `kairo-loadgen`

### 3.1 Crate layout and reuse

What loadgen takes from the testbed (PR #31), and what moves into the shared crate:

| Piece | Testbed source | In loadgen |
|---|---|---|
| Crate shape: `#![deny(unsafe_code)]`; `#[cfg(windows)] #[allow(unsafe_code)] mod gpu; mod platform;`; `windows_subsystem = "windows"`; a stub `main` off Windows | tb/lib.rs:15–28, tb/bin/testbed.rs:6, :39–48 | Same shape |
| The monitor by device path, mirrored displays refused, refresh as an exact fraction, adapter LUID, HDR state; per-monitor-v2 DPI awareness | display/lib.rs:165–190, :237–246; display/platform.rs:64–70, :347–403 | Used as is |
| D3D11 on the adapter with that LUID (`EnumAdapterByLuid`, FL 11.1/11.0, BGRA support), its DXGI index, the output that shows the HMONITOR | tb/gpu.rs:75–108, :211–224, :373–401 | Moves |
| HDR read and HDR watch through the factory's `IsCurrent` | tb/gpu.rs:110–113, :226–282 | Moves. Loadgen refuses HDR and ends the run on a display change |
| Back buffer 0 and its RTV; device-lost classification after `Present` | tb/gpu.rs:403–415, :471–481, :522–546 | Moves. The present flags become a parameter (`DXGI_PRESENT(0)` is hard-coded at :526) |
| Frame-latency wait, closed in `Drop` | tb/gpu.rs:506–513, :647–650 | Moves |
| QPC (`now_hns`, `qpc_to_hns`), the EcoQoS opt-out (CB:57), `KeepDisplayOn` | tb/platform.rs:42–105 | Moves. The harness's copy (hn/platform.rs:26–44) goes |
| The window: a rectangle, a black brush, no cursor, `SW_SHOWNOACTIVATE`, `SetForegroundWindow` with a check, a panic-safe window procedure | tb/platform.rs:185–388 | Moves. Class name and title become parameters (`KairoLoadgen`, `Kairo loadgen`) |
| Idle time, the foreground lock time-out, the `--activate` wait and its 10 s retry | tb/platform.rs:155–183, tb/run.rs:410–444 | Moves |
| The `--stop-on-stdin-eof` watcher (devrun gives every job a NUL stdin, devrun/run.rs:209) | tb/run.rs:396–408 | Moves |
| A ready file written whole and only if new (`<path>.<pid>.partial`, then a hard link) | tb/ready.rs:62–73 | Moves as a function; loadgen has its own JSON |
| Log discipline (one `write_all` per line, header deferred to frame 0, `e end reason=…`) and the CLI idiom | tb/run.rs:84–182, core/testbed_log.rs:1–30, tb/cli.rs:10–36 | Copied; loadgen's format is its own |
| New: `Cover`, a black, non-activating window over a display, with its own thread and message pump; capture exclusion is an option (Q4) | — | In the shared crate's `platform` module; used by `kairo-harness bench` (§4.3) and later by devrun |
| Testbed only: `Chain` (vsync, timer, exclusive, `ResizeBuffers`), pacing, `VblankClock`, `Renderer`, the waitable timer, audio | tb/gpu.rs:287–364, :484–735; tb/pace.rs | Stay in the testbed |

Where loadgen differs from the testbed:
- It is borderless only: `DXGI_PRESENT_ALLOW_TEARING` "can only be used in windowed mode" [MS-PRESENTFLAGS].
- It uses 3 buffers with the tearing flag.
- It runs shaders, a constant buffer, a 256 MiB shader resource and queries; it is the workspace's first HLSL.
- It counts frames by present, while its picture follows QPC time.
- It reads GPU timestamps instead of display times.
- It has no audio, so no `wasapi`, and it is SDR only.

```
crates/kairo-loadgen/
  Cargo.toml                 [[bin]] kairo-loadgen; kairo-core (timeline), kairo-display, the shared crate, serde_json
  src/bin/kairo-loadgen.rs   windows_subsystem = "windows" outside tests; a stub off Windows
  src/lib.rs                 #![deny(unsafe_code)]
  src/cli.rs                 pure: run | calibrate | --version | --help
  src/workload.rs            pure: band split, ramp schedule, guard predictions, calibration fit, frame constants
  src/visual.rs              pure: the picture's CPU reference and the photosensitivity contract (§3.11)
  src/log.rs, summary.rs     pure: log lines; ready, summary and calibration JSON
  src/loadgen.hlsl           every shader, include_str!
  src/gpu.rs, gpu/shader.rs, gpu/queries.rs, gpu/kmt.rs   #[allow(unsafe_code)], Windows only; kmt.rs: the D3DKMT read-outs (§3.10)
  src/run.rs                 Windows only, no unsafe
```

`windows` features:
- `Win32_Foundation`, `Win32_Graphics_Direct3D` and `Win32_Graphics_Direct3D_Fxc` (`D3DCompile`, win/Win32/Graphics/Direct3D/Fxc/mod.rs:2);
- `Win32_Graphics_Direct3D11`, `Win32_Graphics_Dxgi`, `Win32_Graphics_Dxgi_Common`, `Win32_Graphics_Gdi`;
- `Win32_System_LibraryLoader`, `Win32_System_Performance`, `Win32_System_Power`, `Win32_System_SystemInformation`, `Win32_System_Threading`;
- `Win32_UI_Input_KeyboardAndMouse` (for `GetLastInputInfo`) and `Win32_UI_WindowsAndMessaging`;
- `Wdk_Graphics_Direct3D` for the preemption and HAGS read-outs (§3.10), which wincheck already compiles (wincheck/Cargo.toml:31).

### 3.2 Command line

```
kairo-loadgen run --monitor <monitorDevicePath> (--workload alu --iters N | --workload bw --reads N)
                  [--texture-mib 256] [--seed N] [--bands 16] [--buffers 3] [--latency 2] [--detail on|off]
                  [--seconds 60] [--activate] [--log FILE] [--ready FILE] [--stop-on-stdin-eof]
kairo-loadgen calibrate --monitor <monitorDevicePath> --workload alu|bw [--target-ms 8] [--warmup 120]
                  [--texture-mib 256] [--seed N] [--bands 16] [--buffers 3] [--latency 2] [--detail on|off]
                  [--activate] [--out FILE] [--log FILE] [--stop-on-stdin-eof]
kairo-loadgen --version | --help
```

- As in the testbed (R16:402–403):
  - `--monitor` is required, with no default screen;
  - each flag may appear once, and unknown arguments are refused;
  - `--seconds` has no cap of its own (devrun's `max_seconds` bounds every job, M1:98) and counts from the first full-workload frame;
  - a run ends at `--seconds`, at `WM_CLOSE`, or at stdin EOF only with `--stop-on-stdin-eof`.
- Limits [proposed]: `--iters` ≤65,536; `--reads` a multiple of 4 up to 4,096 (§3.5); `--bands` 1–64; `--buffers` 2–4; `--latency` 1–3; `--texture-mib` a power of two from 1 to 1,024.
- `--detail on|off` (default `on` [proposed]) is the ±6-code grain of §3.5. `off` compiles the pixel shaders with the macro `KAIRO_DETAIL=0` (D3DCompile's `pDefines`, §3.7), so the picture keeps only the drift and the header's shader hashes show which it was.
- Not in M1: `--oled`, which caps a run at 600 s for M2's SP1 chunks (M1:40), and an HDR format (SP3).

### 3.3 Window and swap chain

- **Monitor.** `kairo_display::make_dpi_aware()`, then `targets()` and `find_by_path(--monitor)`. A mirrored display is refused (tb/run.rs:206–227).
- **Window.** A `WS_POPUP` at the monitor's rectangle, shown with `SW_SHOWNOACTIVATE`, or brought to the front with `--activate`.
  - The client rectangle must then equal the monitor's exactly, or loadgen exits 3. DirectFlip needs "swapchain buffers match the screen dimensions, and your window client region covers the screen" [MS-FLIP].
  - `KeepDisplayOn` and the EcoQoS opt-out hold for the process's life.
- **Never used:** `WS_EX_LAYERED`, `WS_EX_TOPMOST`, `SetWindowDisplayAffinity`, and the swap-chain flag `DXGI_SWAP_CHAIN_FLAG_DISPLAY_ONLY`, which hides content from desktop duplication [MS-SCFLAG]. Kairo's capture must see loadgen in M2.

```rust
// gpu.rs, windows =0.62.2; every call sits in an unsafe block with // SAFETY:.
let f5: IDXGIFactory5 = factory.cast()?;
let mut tearing = BOOL(0);
f5.CheckFeatureSupport(DXGI_FEATURE_PRESENT_ALLOW_TEARING, (&raw mut tearing).cast(), size_of::<BOOL>() as u32)?;
if !tearing.as_bool() { return Err(Refuse::NoTearing) }                  // exit 3
let desc = DXGI_SWAP_CHAIN_DESC1 {
    Width: w, Height: h, Format: DXGI_FORMAT_B8G8R8A8_UNORM, SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
    BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT, BufferCount: 3, Scaling: DXGI_SCALING_NONE,
    SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD, AlphaMode: DXGI_ALPHA_MODE_IGNORE,
    Flags: (DXGI_SWAP_CHAIN_FLAG_ALLOW_TEARING.0 | DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT.0) as u32,
    ..Default::default() };
let swap = factory.CreateSwapChainForHwnd(&device, hwnd, &desc, None, None)?;
factory.MakeWindowAssociation(hwnd, DXGI_MWA_NO_ALT_ENTER)?;
let swap2: IDXGISwapChain2 = swap.cast()?;
swap2.SetMaximumFrameLatency(2)?;                                           // the waitable flag's default is 1 [MS-LAT2]
let waitable = swap2.GetFrameLatencyWaitableObject();                       // CloseHandle in Drop
```

- **Microsoft's conditions** for `DXGI_PRESENT_ALLOW_TEARING` [MS-PRESENTFLAGS]: the swap chain was created with `ALLOW_TEARING`; the sync interval is 0; and a fullscreen Win32 app "should present to a fullscreen borderless window and disable automatic ALT+ENTER fullscreen switching". Breaking them returns `DXGI_ERROR_INVALID_CALL`.
- **No resizing.** `ResizeBuffers` can't add or remove the tearing flag [MS-SCFLAG]. Loadgen never resizes: a display change ends the run.
- **[Unverified]** `ALLOW_TEARING` together with `FRAME_LATENCY_WAITABLE_OBJECT` on a D3D11 flip swap chain. Nothing on Learn forbids it, and the first PC run checks it. Fallback: no waitable object, `IDXGIDevice1::SetMaximumFrameLatency(2)` and a blocking `Present` [MS-LAT1].

What independent flip needs, and what provides it:

| Condition | How |
|---|---|
| Flip model | `FLIP_DISCARD` |
| Buffers the screen's size, a client area covering the screen | A popup at the monitor's rectangle in a DPI-aware process, with the exact-match check |
| The device on the adapter that scans the monitor out | The LUID from `kairo-display` (M1:66); the bench checks the ready file's LUID against the monitor's |
| Nothing on top of it on that monitor [proposed: Microsoft says DWM may instead reverse-compose or use MPO, §2 item 7] | `--activate`, so the taskbar hides for a fullscreen foreground window [Unverified]; no overlays (Adrenalin's, Game Bar); the bench's covers sit on the other displays |
| An SDR output that isn't rotated | HDR refused (exit 3), like the harness's capture (R16:411); `DXGI_OUTPUT_DESC.Rotation` logged |

- **Tearing doesn't decide independent flip.** Tearing with sync interval 0 lets an independent-flip swap chain flip at once, and lets a composed one present uncapped, keeping only the newest frame. Sync interval 0 means "Cancel the remaining time on the previously presented frame and discard this frame if a newer frame is queued" [MS-PRESENT].
- **Loadgen can't learn its present mode from a documented D3D11 call.**
  - PresentMon decides it from DxgKrnl's events (PM/PresentData/PresentMonTraceConsumer.cpp:718–740, :936–957, :1145–1165), and its CSV is the truth: `SyncInterval` 0, `PresentFlags` 512 (`DXGI_PRESENT_ALLOW_TEARING`, win/Win32/Graphics/Dxgi/mod.rs:1336), `AllowsTearing` and `PresentMode`.
  - Loadgen logs only hints [proposed]: `media=` from `IDXGISwapChainMedia::GetFrameStatisticsMedia` (documented for decode swap chains [MS-MEDIA], so [Unverified] on an RGB one); `hwcomp=` from `IDXGIOutput6::CheckHardwareCompositionSupport` [MS-HWCOMP]; and `occluded=<n>`.

| `Present` returns | Loadgen |
|---|---|
| `S_OK` | Counts it |
| `DXGI_STATUS_OCCLUDED` | Counts `occluded` and keeps rendering, so the load doesn't change |
| `DXGI_ERROR_DEVICE_REMOVED`, `_RESET`, `D3DDDIERR_DEVICEREMOVED` (tb/gpu.rs:471–472, :531–541) | Logs `e device_lost reason=0x…` from `GetDeviceRemovedReason` and exits 4; never recreates the device (§3.10) |
| `DXGI_ERROR_INVALID_CALL` | A tearing condition broke: logs `e present_failed hr=0x…` and exits 4 |

### 3.4 One frame, and the queue

```
WaitForSingleObject(waitable, 1000)       on a time-out, check the stop flags and GetDeviceRemovedReason, then loop
t = (now_hns − anchor_hns) / 1e7          QPC time drives the picture, never the frame count
Map(cb, WRITE_DISCARD) ← FrameConstants; Unmap
Begin(disjoint[s]); End(ts_begin[s])      s = frame mod 8 (§3.8)
OMSetRenderTargets(RTV of buffer 0)       Present unbinds it every time [MS-PRESENT]
RSSetViewports; RSSetState(scissor on); IASetPrimitiveTopology(TRIANGLELIST); IASetInputLayout(None)
VSSetShader(vs_fullscreen); PSSetShader(ps_alu | ps_bw); PSSetConstantBuffers(0, cb); PSSetShaderResources(0, big)
for band in 0..16: RSSetScissorRects(band); Draw(3, 0)
End(ts_end[s]); End(disjoint[s])
hr = Present(0, DXGI_PRESENT_ALLOW_TEARING)    an HRESULT, not a Result, in 0.62.2 (win/Win32/Graphics/Dxgi/mod.rs:5234)
poll older query sets oldest first, and write their f lines in order
```

- **Latency 2 and 3 buffers.** One frame is always queued behind the one the GPU runs, so the GPU never waits for the CPU, and fps = 1 / GPU time, as for a GPU-bound game without a latency reducer.
  - At latency 1 (the waitable object's default), the GPU idles every frame for the wake-up and the recording [Estimate: 0.05–0.5 ms, up to about 6% at 8 ms], and that idle time moves with CPU scheduling.
  - The third buffer is meant to leave the GPU a free buffer while two wait to be shown [proposed]. Microsoft says only that the first GPU write to a back buffer stalls until that buffer is available, and to delay that write as far into the frame as possible [MS-FLIP]. The first rehearsal's `MsGPUWait`, expected near 0, shows whether 3 buffers suffice.
  - `--latency` and `--buffers` exist for M2's sensitivity checks; both are logged.
- **CPU cost.** A few dozen calls per frame [Estimate: tens of µs], with no allocation (§2 item 4).
- **What PresentMon should show for an A run** [Estimate]: `MsBetweenPresents` ≈ the GPU frame time, `MsGPUBusy` ≈ `MsBetweenPresents`, `MsGPUWait` ≈ 0. Kairo's own GPU work then shows up as more `MsGPUWait` and `MsGPUTime` at the same busy time (R13:567).

### 3.5 The two workloads

**Common to both.**
- One triangle covers the target and is drawn once per band: 16 horizontal bands of ⌈H/16⌉ rows, the last band taking the rest.
- The rasterizer state has `ScissorEnable = TRUE` and no culling. There is no vertex buffer; the vertices come from `SV_VertexID`.
- A 32-byte dynamic constant buffer is filled by `Map(WRITE_DISCARD)` each frame:

```rust
#[repr(C)]
struct FrameConstants { drift: f32, scroll_px: f32, count: u32, seed: u32, inv_size: [f32; 2], tex_mask: [u32; 2] }
// drift = frac(t / 20 s) and scroll_px = (t · 120 px/s) mod 8192 are computed in f64, so precision never degrades.
```

**ALU.**
- A per-pixel loop of `count` steps, each one transcendental and a few ALU operations.
- The loop variable differs per pixel, so no part of the work can become scalar.
- The trip count is uniform, so nothing diverges, and the cost is linear in `count` [Estimate: thousands of steps at 1440p for 8 ms].

**Bandwidth.**
- **The texture.** An `R32_UINT` 8192 × 8192 texture: 256 MiB.
  - It is filled once on the GPU from `--seed`. It is also a render target: `ps_fill` runs in 16 bands, then the render target is unbound before the shader resource is bound.
  - Limits: sides up to 16,384, and a resource up to min(max(128, 0.25 × dedicated VRAM), 2048) MB [MS-LIMITS]. So 256 MiB needs at least 1 GiB of VRAM. WARP has none, so CI uses 1 MiB.
  - 256 MiB is larger than any RDNA 4 Infinity Cache (at most 64 MB on the series [Unverified per model]). Most loads therefore go to DRAM, which is the point: Kairo's copies and conversions are partly bandwidth (R13:708).
- **The reads.** Each pixel makes `count` random loads, `count` a multiple of 4. Each address is an integer hash of the pixel, the step and the seed, never of a loaded value, and each pass of the loop issues 4 loads before it uses any, so each wave keeps 4 in flight [proposed]. With one load per pass, each wave would wait for every load before issuing the next.
  - Whether that makes the load bandwidth-bound rather than latency-bound on RDNA 4 is [Unverified]. `calibrate` logs the achieved GB/s (§3.6), and the first PC run compares it with the card's peak.
- **Traffic** ≈ W × H × `count` × 64 B [Estimate: 64 B per random load; at 2560 × 1440 and 8 reads, about 1.9 GB per frame]. Calibration finds `count` [Estimate: 4 to tens].

```hlsl
// loadgen.hlsl (sketch; Windows CI compiles it with the production flags). Written for Kairo; no third-party shader code (CL:17).
#ifndef KAIRO_DETAIL
#define KAIRO_DETAIL 1                                      // --detail off compiles with 0 (§3.2)
#endif
cbuffer Frame : register(b0) {
    float drift; float scroll_px; uint count; uint seed;   // drift = frac(t / 20 s); scroll_px = 120 px/s · t mod 8192
    float2 inv_size; uint2 tex_mask;                        // 1 / client size; texture side − 1
};
Texture2D<uint> big : register(t0);

uint mix32(uint x) {     // two xorshift-multiply rounds; lowbias32's constants [M: hash-prospector, public domain]
    x ^= x >> 16; x *= 0x7feb352du; x ^= x >> 15; x *= 0x846ca68bu; x ^= x >> 16; return x;
}
float hash01(uint2 c) { return float(mix32(c.x ^ mix32(c.y ^ 0x9e3779b9u)) >> 8) * (1.0 / 16777216.0); }
float value_noise(float2 p) {                               // 32 px cells; wraps every 256 cells (8192 px)
    float2 g = p / 32.0; float2 f = frac(g); float2 s = f * f * (3.0 - 2.0 * f);
    uint2 c = uint2(floor(g)) & 255u;
    float a = hash01(c), b = hash01((c + uint2(1, 0)) & 255u);
    float d = hash01((c + uint2(0, 1)) & 255u), e = hash01((c + 1u) & 255u);
    return lerp(lerp(a, b, s.x), lerp(d, e, s.x), s.y);
}
float3 visual(float2 px) {                                  // the picture (§3.11): sRGB codes into a UNORM target
    float2 uv = px * inv_size;
    float a = 6.2831853 * (drift + 0.5 * uv.x + 0.25 * uv.y);
    float3 c = 0.30 + 0.05 * cos(a + float3(0.0, 2.0943951, 4.1887902));             // codes 64–89, desaturated
    return c + KAIRO_DETAIL * (value_noise(float2(px.x + scroll_px, px.y)) - 0.5) * (12.0 / 255.0);   // ±6 codes of grey grain
}

float4 vs_fullscreen(uint id : SV_VertexID) : SV_Position {  // (0,0) (2,0) (0,2): one triangle over the target
    float2 p = float2((id << 1) & 2, id & 2);
    return float4(p.x * 2.0 - 1.0, 1.0 - p.y * 2.0, 0.0, 1.0);
}
float4 ps_alu(float4 pos : SV_Position) : SV_Target {
    float x = frac(dot(pos.xy, float2(0.6180339, 0.4142135))), acc = 0.0;
    [loop] for (uint k = 0; k < count; ++k) { x = frac(x * 1.0001 + 0.5 * sin(x * 7.31 + k * 0.01)); acc += x; }
    return float4(visual(pos.xy) + (acc / max(count, 1u) - 0.5) / 255.0, 1.0);   // ±0.5 code keeps the work alive
}
uint ps_fill(float4 pos : SV_Position) : SV_Target { uint2 p = uint2(pos.xy); return mix32(p.x ^ mix32(p.y ^ seed)); }
float4 ps_bw(float4 pos : SV_Position) : SV_Target {
    uint2 p = uint2(pos.xy);
    uint h = mix32(p.x ^ mix32(p.y ^ seed)), acc = 0;
    [loop] for (uint k = 0; k < count; k += 4) {            // count is a multiple of 4: four loads issued before any is used
        uint a = mix32(h + k), b = mix32(h + k + 1u), c = mix32(h + k + 2u), d = mix32(h + k + 3u);
        uint va = big.Load(int3(a & tex_mask.x, (a >> 16) & tex_mask.y, 0));
        uint vb = big.Load(int3(b & tex_mask.x, (b >> 16) & tex_mask.y, 0));
        uint vc = big.Load(int3(c & tex_mask.x, (c >> 16) & tex_mask.y, 0));
        uint vd = big.Load(int3(d & tex_mask.x, (d >> 16) & tex_mask.y, 0));
        acc += (va ^ vb) + (vc ^ vd);
    }
    return float4(visual(pos.xy) + (float(acc & 255u) / 255.0 - 0.5) / 255.0, 1.0);
}
```

- The two ±0.5-code work terms keep the compiler from removing the work. They depend only on the pixel, `count` and the seed, so they never flicker.
- R13:713–723 is the ALU model, with two changes: a per-pixel loop variable, and an output from `visual()` rather than from the loop. R13's output cycles the whole screen's colour once per second, which §3.11 rules out.
- A linear `ByteAddressBuffer` read in 64-byte bursts would move more bytes per transaction. It stays an M2 option, since R13:708 asks for a texture and games read tiled textures.

### 3.6 Determinism, and what a run records

- **Time-based.** The picture is a pure function of the pixel and t = (frame start − anchor) in QPC time, never of the frame count. Two runs show the same picture at the same t whatever their frame rates (R13:709), and an encoder sees comparable content in A and B.
- **Moving.** The picture changes every frame, which WGC needs, since it delivers only what DWM composes (R08:387).
- **Explicit counts.** `run` has no default `--iters` or `--reads`, so a run can't quietly differ from the last. Nothing changes during a run: the start-up ramp (§3.10) can only refuse.
- **The log header records:**
  - workload, count, texture size and seed, bands;
  - size, refresh, format, buffers, latency, present flags and sync interval;
  - detail, `qpc_hz`, preemption granularity, HAGS, hardware composition, HDR;
  - an FNV-1a 64 hash of each compiled bytecode, the hash the corpus manifests use (M1:90). It shows a Windows update that changed `d3dcompiler_47.dll`'s output.
- **`calibrate`**, never inside a measured run:
  1. The same window, swap chain, bands and picture as `run`.
  2. Doubling steps from `count` = 16 (ALU) or 4 (bandwidth), at least 30 frames each, taking the median GPU ms from the timestamps (§3.8). The TDR prediction (§3.10) runs before each step. It stops once the median passes half the target.
  3. It holds that count for `--warmup` s (default 120), so clocks and temperature match the runs (R13:830).
  4. A least-squares fit ms = a + b · count over the last three doubling steps, re-measured warm. It refuses (exit 3) if R² < 0.99 [proposed].
  5. It verifies the fitted count for 5 s and accepts it within ±3% of the target, else takes one correction step [proposed].
  6. It prints one JSON line and, with `--out`, writes the same line create-new: `{"kairo_loadgen_calibration":1,"workload":"alu","target_ms":8.0,"count":…,"gpu_ms_p50":…,"fit":{"a_ms":…,"b_ms":…,"r2":…},"bands":16,"size":"WxH","texture_mib":256,"shader":"fnv64:…","fingerprint":"fnv64:…","gbps":null}`. `gbps` is the bandwidth workload's achieved rate, W × H × `count` × 64 B ÷ the GPU time [Estimate: 64 B per load], and null for ALU. This is PC data: `logs\` and `.dogfood/` only (CL:18).
- **Calibrate again** when the shader hashes, the size, the refresh rate, the driver (`umd_version`), HAGS or the texture size change [proposed]. Loadgen puts an FNV-1a 64 `fingerprint` of those values in its calibration line and its ready file, and the bench refuses a session whose first ready file's fingerprint differs from `calibration.json`'s (§4.4: exit 3, "calibrate again"). The adapter's LUID isn't in it, since LUIDs are new after every restart [M]. A/B pairs in one session always share one count.

### 3.7 Shader compilation

- **The call.** `D3DCompile(SRC.as_ptr().cast(), SRC.len(), s!("loadgen.hlsl"), defines, None::<&ID3DInclude>, entry, target, D3DCOMPILE_OPTIMIZATION_LEVEL3 | D3DCOMPILE_ENABLE_STRICTNESS | D3DCOMPILE_WARNINGS_ARE_ERRORS, 0, &mut code, Some(&mut errors as *mut _))` for `vs_fullscreen` (`vs_5_0`) and for `ps_alu`, `ps_bw` and `ps_fill` (`ps_5_0`). `defines` is `None`, or with `--detail off` a `D3D_SHADER_MACRO` list holding `KAIRO_DETAIL=0` and a closing null entry. wincheck/src/shaders.rs:4–14 is the compiled pattern; the signature is at win/Win32/Graphics/Direct3D/Fxc/mod.rs:2.
- **Names and errors.** `pSourceName` is a bare name used only in error messages [MS-COMPILE], so compiler errors name no path (CL:16). With no `#include`, there is no include handler. A failed compile is exit 4, with the error blob's text.
- **The DLL.** `d3dcompiler_47.dll` "is now included with the Windows 8.1 / Windows 10 OS" [WALBOURN]. It is a Windows component, loaded like `d3d11.dll`, not a dependency to license (CL:17).
- **DLL search.** The raw-dylib import loads at process start, with the application's folder searched first. `bin\<build-id>\` is admin-owned and `pc deploy` copies only checked files (M1:81), so no foreign copy can land there.
  - A missing DLL would stop the process before `main` with an NTSTATUS exit code [M].
  - `LoadLibraryExW(…, LOAD_LIBRARY_SEARCH_SYSTEM32)` would turn that into exit 4, at the cost of more `unsafe`. Not needed on Windows 10/11.

### 3.8 Loadgen's own GPU time

- **Queries.** A ring of 8 query sets [proposed], each `{TIMESTAMP_DISJOINT, TIMESTAMP, TIMESTAMP}` around the frame's draws: one disjoint query per frame, as Learn advises [MS-QUERY].
- **Reading.** After each `Present`, the pending sets are polled oldest first with `GetData(…, D3D11_ASYNC_GETDATA_DONOTFLUSH)` through the vtable, comparing the result with `S_OK`.
  - The 0.62.2 wrapper ends in `.ok()`, which turns `S_FALSE` ("not yet available", [MS-GETDATA]) into success. The fix is at R13:684–695, CB:410–419 and wincheck/src/gpu_timer.rs:3–8.
  - Polling stops at the first set that isn't ready, so `f` lines stay in order.
- **Value.** `gpu_us = (end − begin) × 10⁶ / Frequency`, only when `Disjoint` is FALSE [MS-DISJOINT]; otherwise `-`, counted in `gpu_disjoint`. A set still pending when its slot comes round also gives `-`, counted in `gpu_unknown`.
- **Never blocking.** A measured run never waits on a query. `calibrate`, the ramp and the final read may flush, bounded at 2 s.
- **What it measures.** Wall time on the GPU between the two marks, including the time other processes' work takes from the frame. It rises with contention, like PresentMon's `MsGPUTime`. The `f` lines carry QPC times that line up with PresentMon's `CPUStartQPC` (`--qpc_time`).

### 3.9 Ready file, log, summary and exit codes

**`ready.json`.** Written create-new (tb/ready.rs:62–73) once the ramp has reached the full workload and that frame was presented. PresentMon starts after it, so it never records the ramp.

```json
{"kairo_loadgen_ready":1,"pid":4242,"hwnd":197794,"client":{"left":-2560,"top":0,"width":2560,"height":1440},
 "refresh_mhz":179960,"adapter_luid":"0x00000000000123ab","dxgi_adapter_index":1,"dxgi_output_index":0,"umd_version":"…",
 "format":"bgra8","hdr":false,"swap_effect":"flip_discard","buffers":3,"latency":2,"tearing":true,"present_flags":512,
 "sync_interval":0,"activated":true,"workload":"alu","count":3072,"bands":16,"detail":true,"qpc_hz":10000000,
 "hags":"on","fingerprint":"fnv64:…","start_hns":123450000000,"seconds":90}
```

- The values are examples. The file names this run's window and adapter, so it stays local, like the testbed's (tb/ready.rs:1–5).
- `umd_version` is the adapter's user-mode driver version from `IDXGIAdapter::CheckInterfaceSupport` (CB:1080) [proposed].
- `hags` (`on`, `off` or `unknown`) is loadgen's own read-out (§3.10); `fingerprint` is §3.6's.
- `pid` and `hwnd` let devrun's future fullscreen stop exempt the job's own window (M1:103).

**The log.** UTF-8, following `kairo_core::testbed_log`'s rules: one `write_all` per line, times in QPC 100 ns, a torn last line dropped by readers, setup events held until the header.

```
# kairo-loadgen 1
h workload=alu count=3072 texture_mib=- seed=- bands=16 size=2560x1440 refresh_mhz=179960 format=bgra8 buffers=3 latency=2 present=512 sync=0 detail=1 qpc_hz=10000000 shader=<fnv64 per entry> preempt=pixel hags=on hwcomp=fullscreen,windowed hdr=0 start_hns=<QPC of frame 0>
f <n> <start_hns> <present_hns> <gpu_us|->
e <name> [key=value …]
```

- **Events:**
  - `ramp step=<k> count=<c> gpu_us=<p50>`, `activate result=ok|failed`, `ready`;
  - `present_hint media=… occluded=<n>`, at ready and then every 10 s;
  - `too_heavy predicted_ms=<x>`, `display_changed`, `device_lost reason=0x…`, `present_failed hr=0x…`;
  - `end reason=seconds|stdin|close|error`.
- About 3.75 KB/s at 125 fps [Estimate].
- No device path, monitor, adapter or PC name, window title or wall-clock time (CL:16).

**Summary.** One JSON line on stdout at exit, after `ready` (the bench keeps it):
`{"kairo_loadgen":1,"workload":"alu","count":3072,"frames":N,"seconds":S,"fps":…,"gpu_ms_p50":…,"gpu_ms_p99":…,"gpu_ms_max":…,"gpu_unknown":k,"gpu_disjoint":d,"occluded":m,"end":"seconds"}`.
Loadgen's own `fps` is informational; PresentMon's CSV is the measurement.

| Exit | Meaning (the testbed's numbering, tb/cli.rs:24–36) |
|---|---|
| 0 | Ran to `--seconds`, or stopped by `WM_CLOSE` or stdin EOF; `calibrate` wrote its result |
| 2 | Bad command line; the log, ready or `--out` file exists or can't be made |
| 3 | Refused: the monitor isn't found or mirrors another, HDR, no tearing support, the client area isn't the monitor, a TDR guard, a display change, or a calibration that can't reach its target within the caps |
| 4 | Device, swap-chain or shader failure, or the device lost mid-run |
| 5 | `--activate` didn't get the foreground |

devrun's own stop gives a job exit code 1 (core/job.rs:17), so loadgen leaves 1 unused.

### 3.10 TDR limits

- **The risk.**
  - "The default timeout period in Windows is two seconds. If the GPU can't complete or preempt the current task within the TDR timeout period, the OS diagnoses that the GPU is frozen" [MS-TDR].
  - "the OS bug-checks the computer on the sixth (or more) GPU hang when it detects that five (5) or more GPU hangs ... occur within one (1) minute" [MS-TDR]. A loader that recreated its device and resubmitted the same frame could crash your PC.
  - At `PRIMITIVE_BOUNDARY` granularity the driver can't stop a running primitive [MS-PREEMPT], and a full-screen triangle is one primitive.

| Guard | Rule [proposed] |
|---|---|
| Bands | 16 draws per frame, about 0.5 ms each at the target |
| Caps | `--iters` ≤65,536, `--reads` ≤4,096 (a multiple of 4), `--bands` 1–64, texture ≤1,024 MiB |
| Start-up ramp in `run` | Steps of ×4 from `count` 16 (ALU) or 4 (bandwidth), at least 10 frames each, timed by the queries. A step runs only if 4 × the previous median predicts ≤25 ms per band and ≤100 ms per frame; otherwise exit 3 with `e too_heavy`. It reaches the requested count or refuses; it never changes it |
| `calibrate` | The same prediction before each doubling |
| At run time | Three frames in a row over 250 ms of GPU time end the run (exit 3) |
| Never | `D3D11_CREATE_DEVICE_DISABLE_GPU_TIMEOUT` [MS-DEVFLAG]; `TdrDelay` or `TdrLevel` registry changes (a PC change outside Kairo's folders, CL:43) |

- **A lost device** ends the run with exit 4 and is never recreated, as in the testbed (tb/run.rs:498–513). The bench then reads System events 4101 and 41, and either is a P0 (M1:214).
- **Read-outs** [proposed]. `D3DKMTOpenAdapterFromLuid` on loadgen's own LUID, then `D3DKMTQueryAdapterInfo` twice: `KMTQAITYPE_WDDM_1_2_CAPS` → `GraphicsPreemptionGranularity`, logged as `preempt=`, and `KMTQAITYPE_WDDM_2_7_CAPS` → whether HAGS is on, logged as `hags=` (win/Wdk/Graphics/Direct3D/mod.rs:13294, :13299, :10382; CB:1080).
  - Microsoft marks the 2.7 caps "reserved for system use", so they serve logging only, as OBS uses them (R13:869). §5.1 compares GPU columns only between runs with the same `hags`.
  - It opens loadgen's own adapter read-only; no process is involved. A failed read logs `unknown` and the run goes on.

### 3.11 Photosensitivity limits

**The thresholds** (Xbox Accessibility Guideline 118 [MS-XAG118]):
- A flash is "a 10% change in luminance". Flashes fail at "approximately more than three per second" over "approximately 20 percent or more" of the screen, and "Lower intensity flashing can also cause a failure if it's continued for an extended period of time".
- Red flashes involve "R/(R + G + B) >= 0.8".
- Spatial patterns fail when "The difference in contrast is greater than 10 percent".

| Property of the picture [Mac] | Value | Limit |
|---|---|---|
| sRGB codes | 58–95 | — |
| Relative luminance | 0.051–0.103 of white: the whole picture spans 5.2% | 10% |
| Largest change at one pixel within 1/3 s | 0.0016 of white from the drift; 0.023 when grain passes | 10% |
| Largest change of the screen average within 1/3 s | 0.0009 | 10% over ≥20% of the screen |
| Red ratio R/(R+G+B), maximum | 0.39 on sRGB values, 0.46 on linear ones | 0.8 |
| Screen average over the 20 s cycle | 0.066–0.083, varying smoothly at 0.05 Hz | No flash |

- **No step in time anywhere.** The ramp, the run and the end draw the same picture. The only transitions are the window appearing and leaving, one of each per run and at least 30 s apart: single changes, never a repeating flash.
- **No high-contrast element.** A tear line (with VRR off) is a one-pixel offset in faint grain.
- **A contract in CI.** `visual.rs` checks the picture in Mac and Linux CI, over a grid and a full cycle, at half XAG's limits or tighter [proposed]:
  - luminance span <0.08;
  - every change within 1/3 s <0.05 at a pixel and <0.005 for the screen average;
  - red ratio (on linear values) <0.5;
  - no luminance difference >0.08 inside any window of 20% of the screen.

  The WARP test (§6) checks that the GPU draws what `visual.rs` describes.
- **[Corrected] 2026-11-24** (PR #43's first review): not all of these limits are half of XAG's. The changes within 1/3 s are (0.05 and 0.005 against 0.10); the span and the window are 0.08 against 0.10, and the red ratio 0.5 against 0.8, since the picture itself spans 0.054 and reaches a red ratio of 0.46. Each limit is stated beside XAG's in `visual.rs` and the CHANGELOG.
- **The OLED** isn't involved in M1 (M1:40, M1:99). The picture is moving SDR content, so M2's OLED chunks of at most 10 minutes need no other picture (CL:26).

### 3.12 Rules, call by call

- **Own objects only.** Every call is on loadgen's own device, swap chain, window, process or thread:
  - DXGI and D3D11 creation and drawing;
  - `D3DCompile` (code compiled in its own process, no hook);
  - `CreateWindowExW`, and `SetForegroundWindow`, which sends no input (CL:10);
  - `GetLastInputInfo` (the session's idle time, as the testbed reads it);
  - `SetThreadExecutionState`, `SetProcessInformation(ProcessPowerThrottling)` on `GetCurrentProcess()`, and QPC.

  The D3DKMT read-outs open its own adapter, not a process.
- **Never used:** any capture (CL:8), another process's handle (CL:9), synthetic input, drivers, services, registry writes or elevation (CL:10–11).
- **CI's scan** reads every line of every `*.rs` under `crates/`, comments included (ci:58–76). No identifier here matches: `PROCESS_POWER_THROTTLING_*` isn't `PROCESS_VM_*`, and `D3D11_CREATE_DEVICE_DEBUG` doesn't contain `DEBUG_PROCESS`. Comments say "never synthesises input" rather than naming a banned call (as tb/platform.rs:4–5 does). The HLSL isn't scanned (M1:102).
- **The clock.** Loadgen, the shared crate and the bench are instrument work, outside the exit-rule clock (M1:7, M1:110). One edge, as with `kairo-display` (R16 §10): if `wgc-probe` later uses the shared crate, fixes made for it after the clock starts count as capture code.
  - That edge goes into the day log's day 1 for your veto (M1:115), as research 16's two did (R16:922).
  - PR 3 should land before the first `wgc-probe` commit [proposed], so the crate exists as instrument code when the clock starts.

---

## 4. `pc bench`

### 4.1 The protocol (M1)

| Step | What | Source |
|---|---|---|
| Before | `pc status` and `pc doctor`; every PC step checks the busy state first (§4.6); PresentMon's SHA-256 against the pin; `env.json` and a 3 s `encoder-usage`; no screen saver, inactivity lock or Dynamic Lock that could fire; no Remote Desktop or lock; no leftover Kairo PresentMon or loadgen | CL:23, M1:35, M1:96, doctor:674–693 |
| Warm-up | Per workload: loadgen at the calibrated count for 120 s, without PresentMon; discarded | R13:830, M1:29 |
| Runs | 6 per workload; each is 30 s of idle, a fresh loadgen, then PresentMon with `--delay 5 --timed 60` | R13:831–833 [proposed for the A/A] |
| Workloads | ALU, then bandwidth, each with its own warm-up | §2 item 15 |
| Statistics | `pm-stats` per CSV; validity (§4.7); `ab-stats` in A/A mode per workload | M1:85 |
| More runs | At the job's one decision point (§4.6), per workload: make-up runs up to 6 valid ones, or, with 6 valid and a CV above 1%, 6 more; each block with its own warm-up, in the same job; still above 1% at 12 → reported | M1:29, §4.7 |
| After | Events 4101 and 41; `env.json` again, compared; any leftover ETW session stopped; Instant Replay's state shown (Q3) | M1:214, M1:217 |

- **Rehearsal** (attended, about 2.5 min): one workload, a 20 s warm-up, 2 runs with `--timed 20`, 10 s gaps, the covers, and no decision point.
- **Calibration**: `kairo-loadgen calibrate --warmup 120` per workload, with a 30 s gap between them, under the covers.
- **Stop test** (attended, up to about 4 min): `--plan selftest` (§4.3) shows that the foreground and fullscreen stops end a session within 2 s, before Q1's run relies on them.

### 4.2 Where each piece runs

| Piece | Runs as | Why |
|---|---|---|
| `kairo-harness bench`, loadgen, PresentMon | You, in your session: one hidden `\Kairo\Run` job at normal priority | GUI programs start only through `\Kairo\Run` (CL:24, M1:97); timing needs normal priority (core/job.rs:31–40, CB:54); everything a job writes goes to `logs\` (M1:75); one job at a time (M1:98); it survives an SSH drop |
| `bench-pre.ps1`, `bench-fetch.ps1`, `env.json`, `encoder-usage`, `pm-stats`, `ab-stats`, `bench-rules`, the events check, the session cleanup (inbox `logman`), the decision file | The elevated SSH session. It reads files in `logs\` only through `Open-OwnedFile`, and writes one file: the decision, in admin-only `C:\kairo\jobs\` beside `next.json` | M1:81, lib:92–99, lib:116–131, lib:163–170 |
| Decisions (extra runs, pooling, a P0) and storage | The Mac, from `bench-rules`' proposal; `.dogfood/bench/` | M1:78 |

PresentMon in the job versus in the elevated session:

| | In the job, as you (chosen) | Elevated, from SSH (fallback) |
|---|---|---|
| CSV | `logs\bench-<stamp>\<w>-<nn>.csv`, made by you | No allowed folder: only `--output_stdout`, with the CSV relayed in memory to `.dogfood/` |
| Lining up with loadgen | Local: PresentMon starts right after `ready.json` | Across sessions, polling the ready file once a second |
| SSH drop or a sleeping Mac | No effect | The run, and the rest of the session, are lost; the ETW session is orphaned [Unverified: whether OpenSSH ends the children] |
| Your input | devrun kills loadgen and PresentMon together | PresentMon runs on to its `--timed` end |
| Privilege | A limited token without SeDebugPrivilege. Either way PresentMon opens processes with `PROCESS_QUERY_LIMITED_INFORMATION` only (PM/OutputThread.cpp:303) | SeDebugPrivilege enabled (PM/Privilege.cpp:27–68) |
| Evidence | M0 recorded 2 s with a limited token, from an S4U task (doctor:438–505, M0:166) | Sketches only |

### 4.3 The job

`.dogfood/jobs/bench-<stamp>.json` (it names monitors, so it lives in `.dogfood/`, M1:76):

```json
{"id":"bench-<yyyyMMdd-HHmmss>","cmd":["C:\\kairo\\bin\\current\\kairo-harness.exe","bench","--plan","aa",
 "--run-id","bench-<yyyyMMdd-HHmmss>","--monitor","<180 Hz device path>","--not-monitor","<OLED device path>",
 "--cover-others","--workloads","alu,bw","--iters","<from calibration>","--reads","<from calibration>",
 "--fingerprint","<from calibration>","--runs","6","--retries","3","--warmup","120","--delay","5",
 "--seconds","60","--gap","30","--activate","--decide-wait","300"],
 "cwd":"C:\\kairo\\logs","max_seconds":5830,"abort_on_input":true,"window":"hidden","priority":"normal",
 "etw_sessions":["KairoBench"]}
```

- `--run-id` repeats the job id and names the folder, as `run`'s does (R16:780–789).
- The Mac computes `max_seconds` from the plan's phase limits plus slack, as `phases::max_seconds` does (hn/phases.rs:87–91), and a test like `the_limits_fit_the_job` (hn/phases.rs:102–119) checks that the sum below, retry reserve and second pass included, stays under it. devrun's time stop reads exactly like a safety stop (exit 6), so the limits must never reach it.
- `--plan aa|calibrate|selftest` in M1; a rehearsal is `aa` with small numbers and `--decide-wait 0`. M2 adds `abba` (§4.12).
- `--not-monitor` names the `oled` alias's display. The job refuses (exit 3) a `--monitor` equal to it, a second guard beside the Mac's (hn/pc:880–888).
- `etw_sessions` is PR 5's new job field (§4.5 c).

| Phase | What | Limit | Typical [Estimate] |
|---|---|---|---|
| 0 setup | See below | 15 s | 2 s |
| 1 warm-up, per workload | loadgen `run --seconds 120 --activate --stop-on-stdin-eof --ready ready-<w>-warmup.json`; wait for ready, then for its exit | Ready 20 s (for the job's first loadgen, the lock time-out + 25 s); end 130 s after ready | 123 s (+ up to 205 s once) |
| Gap | Nothing runs | `--gap` | 30 s |
| 2 run n of N | §4.4 | Ready 20 s; PresentMon `--delay` + `--timed` + 25 s, then 5 s once its session is stopped; loadgen's end 10 s | About 69 s |
| Retry | A run slot again, for a failure the job sees itself (§4.7) | As a run; at most `--retries` per workload | — |
| 3 decision (`aa`) | The covers stay up, nothing runs; the job waits for `C:\kairo\jobs\<id>.decision.json` | `--decide-wait` | 1–2 min |
| 4 second pass | Per workload given extra runs: a warm-up, then those runs | As phases 1 and 2; at most 6 runs per workload | 0, or about 12 min per extended workload |
| 5 report | The final `bench.json`; one summary on stdout | — | <1 s |

Setup does, in order:
1. Makes `bench-<stamp>\`; if it exists, exits 2.
2. Prunes to the newest 5 real `bench-<stamp>` folders, never following a link (hn/prune.rs).
3. Calls `SetErrorMode`, holds `KeepDisplayOn`, and wants at least 2 GiB free.
4. Refuses (exit 3) in a Remote Desktop session (`GetSystemMetrics(SM_REMOTESESSION)` non-zero, [MS-SYSMET]), when `LogonUI.exe` runs (a locked session, M1:166 [Unverified]), or when `PresentMon-2.6.0-x64.exe` or `kairo-loadgen.exe` runs (a leftover). Any other `PresentMon*.exe`, such as Adrenalin's (R13:540), is recorded as `foreign_presentmon` and doesn't refuse: its own session name keeps it apart (§9 risk 30). Image names come from a Toolhelp snapshot only, as `count_processes_named` does (hn/platform.rs:77–103).
5. Checks that PresentMon and loadgen exist (and `testbed.exe` for `selftest`).
6. Resolves the monitor: refresh, HDR (exit 3 if on), adapter LUID; a `--monitor` equal to `--not-monitor` refuses (exit 3).
7. Puts a cover on every other active display (below).
8. Stops a leftover `KairoBench` session with `PresentMon … --session_name KairoBench --terminate_existing_session`. Exit 7 means two things (PM/MainThread.cpp:244–253), so stderr decides: "no existing sessions found" is fine, while "failed to terminate existing session" refuses (exit 3, "leftover KairoBench: run pc bench cleanup"), since each run's `--stop_existing_session` would then fail with exit 6 (:355–383).
9. Records every active display's role (`bench` or `cover`), refresh, HDR, size and adapter LUID as `displays_start`; collects the PIDs that own a top-level window (`EnumWindows` and `GetWindowThreadProcessId`: PIDs only, no titles) for the foreground stop; starts the watcher; and writes `bench.json` with the plan.

For the full A/A the limits add up to 5,770 s:
- setup 15 s; the first block's warm-up 355 s (ready at the default 200 s lock time-out [M] + 25, end 130) and the second's 150 s;
- 12 run slots of 155 s (gap 30 + ready 20 + PresentMon 90 + its stop 5 + loadgen's end 10) = 1,860 s;
- a retry reserve of 3 slots per workload, 930 s; the decision wait, 300 s;
- a second pass of at most a warm-up and 6 slots per workload, 2,160 s.

`max_seconds` is that sum plus 60 s for the kills' waits: 5,830 s, about 97 minutes. Only a run of failures comes near it; a typical session ends after about 25 minutes (§4.10).

**The decision point** [proposed]. After the first pass, an `aa` job with `--decide-wait` above 0 writes `phase: awaiting_decision` into `bench.json`, keeps the covers up and the displays on, runs no load, and reads `C:\kairo\jobs\<id>.decision.json` every second.
- The elevated session writes that file (§4.6 step 4) in the admin-only folder that holds `next.json`, so none of your programs can make one. The job uses it only if an administrator or SYSTEM owns it, its `run_id` is the job's, and each workload's extra runs are 0 to 6: `{"kairo_bench_decision":1,"run_id":"bench-<stamp>","extra":{"alu":0,"bw":6}}`.
- It then runs phase 4 and ends. One decision per job, so the bench never loops.
- No valid file within `--decide-wait` (an SSH drop, a sleeping Mac) ends the job with the first pass's data and `decision: timeout`.

**The watcher** [proposed]. Every second:
- **Fullscreen and a stale file.** It reads `C:\kairo\state\watch.json`, which your watch agent rewrites every second (core/watch.rs:10–15). It acts when `foreground_fullscreen` is true and `foreground_exe` is null or neither of the job's own programs, `kairo-loadgen.exe` and `kairo-harness.exe` (compared ignoring case), or when the file has been older than 5 s for 10 s.
- **Foreground.** It reads `GetForegroundWindow` and `GetWindowThreadProcessId` (win/Win32/UI/WindowsAndMessaging/mod.rs:866, :1184), which open no process and read no title, and acts when:
  - during an activated run, from its `ready.json` to its stop, the foreground PID is neither loadgen's nor the harness's;
  - at any time, the foreground PID owned no top-level window at setup: a program started since, such as a game started with a controller or a windowed game;
  - in a gap, the foreground changes again after the change Windows makes when loadgen closes (read 1 s after its exit), so a switch to an open game never meets the next loadgen's activation;
  - no window has been in front for 5 s in a row.
  `watch.json` never names an ordinary window in front (core/watch.rs:25–30, M0:74), and neither does this check: `bench.json` records the stop, never the program.
- **Lock and overlays.** A Toolhelp snapshot, names only: `LogonUI.exe` appearing, or a `PresentMon*.exe` that wasn't running at setup (Adrenalin's overlay switched on, §9 risk 33).
- **Acting** kills the children, stops the session's ETW session, finishes `bench.json` with `outcome` `stopped_fullscreen`, `stopped_foreground`, `stopped_locked`, `stopped_overlay` or `stopped_watch`, and exits 6. The covers go with the process.

**The covers** [proposed].
- A black, non-activating (`WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW`), topmost window over each active display other than `--monitor`, found with `kairo-display` at setup and again on `WM_DISPLAYCHANGE`. Each window's rectangle is checked against its monitor's once it is shown.
- Each has its own thread and message pump and never takes focus. They die with the job, so your first input removes them.
- Capture exclusion (`SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)` on its own window) is an option of the shared crate's `Cover`; whether the bench uses it is Q4.
- Every plan uses the covers, `calibrate` and `selftest` included, except the one rehearsal without them (§6).
- `bench.json` records `covers: not_needed` only when no other display is active. A stale `oled` alias can't mislead this, since the covers follow the active displays, not an alias (M1:77 keeps an alias whose display is off).
- The full A/A refuses to start (exit 3) when a cover can't be shown.

**The stop test** (`--plan selftest`) [proposed]. Setup, the covers, then one activated loadgen run on `--monitor`. Once it is ready, the harness starts `C:\kairo\bin\current\testbed.exe --monitor <the same path> --mode borderless --pattern motion --activate --seconds 20 --log selftest-testbed.log --stop-on-stdin-eof` as its own child (plus `--beep off` once research 16's PR 8b adds the beep, R16:404). The testbed's activation puts a program other than loadgen in front, fullscreen, and the session must end (`stopped_foreground` or `stopped_fullscreen`) within 2 s of the testbed's `e activate result=ok`. `pc bench selftest` passes only then.

### 4.4 One run, step by step

1. **loadgen.** `kairo-loadgen.exe run --monitor <path> --workload <w> --iters|--reads <n> --seconds 90 --activate --stop-on-stdin-eof --log <w>-<nn>.log --ready ready-<w>-<nn>.json`.
   - stdin is piped, the creation flag is `CREATE_NO_WINDOW`, stdout goes to `loadgen-<w>-<nn>.out` (its summary line) and stderr to `loadgen-<w>-<nn>.err`.
   - `--seconds 90` is only a cap: the bench ends loadgen through its stdin once PresentMon is done.
2. **The ready check**, polling every 50 ms:
   - `pid` = the child's;
   - `adapter_luid` = the monitor's adapter;
   - the client area = the monitor's rectangle;
   - `tearing` true, `present_flags` 512, `sync_interval` 0, `hdr` false;
   - the workload and count as asked;
   - `fingerprint` = `--fingerprint`, the calibration's (§3.6).

   A mismatch of the adapter, client area, swap chain or HDR stops the block, since a retry would meet it again. A fingerprint mismatch stops the job (exit 3, "calibrate again"). A ready file that doesn't come in time is retried (§4.7).
3. **PresentMon.** stdin NUL, `CREATE_NO_WINDOW`, stdout and stderr to `pm-<w>-<nn>.out` and `.err`. Given a disk file, PresentMon writes UTF-16LE with a BOM (PM/Console.cpp:13–43); the harness decodes by the BOM.

```
C:\kairo\tools\presentmon\PresentMon-2.6.0-x64.exe
  --process_id <loadgen pid> --output_file C:\kairo\logs\bench-<stamp>\<w>-<nn>.csv
  --session_name KairoBench --stop_existing_session
  --delay 5 --timed 60 --terminate_after_timed --no_console_stats
  --qpc_time --track_frame_type --track_gpu_video --no_track_input
```

   - The ETW session starts at once. Recording starts when the 5 s delay timer fires, and the 60 s timer starts with it (PM/MainThread.cpp:120–123, :176–188, :401–405).
   - The recording window follows present time, not ETW delivery (PM/OutputThread.cpp:42–62, :471–481).
   - `--stop_existing_session` is safe here: setup refused any other PresentMon, and devrun's mutex rules out a second job.
4. **Wait for PresentMon's exit**, at most delay + timed + 25 s.
   - Past that, the bench runs `--session_name KairoBench --terminate_existing_session`. Stopping the session makes `ProcessTrace` return, so the CSV still closes properly (PM/ConsumerThread.cpp:30–38).
   - After 5 more seconds it kills PresentMon and marks the run `stopped_early`.
5. **Stop loadgen**: close its stdin, wait 10 s, then kill it.
6. **Record the run in `bench.json`**, through a temporary file and a rename:
   - PresentMon's exit code;
   - whether its stdout held `Started recording.` and `Stopped recording.` (PM/MainThread.cpp:110–112, :145–147);
   - whether the CSV exists, its size, and whether its last byte is `\n` (a killed writer's cut tail isn't): PresentMon creates it only at the first recorded row (PM/CsvOutput.cpp:1421–1427);
   - the classified stderr: `ETW buffers were lost`, `ETW events were lost` and `overflowed present events` set `etw_loss` (PM/MainThread.cpp:448–462), while the expected "requires elevated privilege" warning (:272–276) is ignored;
   - loadgen's exit code and summary line, and whether it was still running when PresentMon ended;
   - whether the job retries the slot, and why (§4.7 rules 1–2).
7. **Retry**: a run that failed in a way the job sees is run again in its slot, after the gap, at most `--retries` times per workload.
8. **Gap**, then the next run.

The job computes no statistics: `pm-stats.ps1` is their one source.

### 4.5 PresentMon facts the bench relies on

| | v2.6.0 |
|---|---|
| Exit codes | 0: a normal end, **including** a run that recorded nothing (no CSV), a CSV that couldn't be opened (CsvOutput.cpp:1425–1427 returns quietly) and lost events (MainThread.cpp:474). 1: the embedded NVIDIA manifest failed to load, or a bad command line (:229–240). 3, 4, 5: window class, message window, hotkey (:283–301). 6: the session name is taken without `--stop_existing_session`, or `StartTrace` failed (e.g. access denied, followed by the Performance Log Users hint) (:336–383). 7: `--terminate_existing_session` found no session, or couldn't stop it (:244–253); only stderr's text tells them apart (§4.3 step 8) |
| Privileges | Admin or Performance Log Users (PMREADME:49–60, [MS-StartTrace]). Elevation adds SeDebugPrivilege and the Kernel-Process provider, whose access-denied error PresentMon tolerates (PresentData/PresentMonTraceSession.cpp:807–821). With `--process_id` on your own loadgen, neither matters |
| The session | Real-time; 64 KB buffers, 256–1,024 of them = 16–64 MiB (PresentMonTraceSession.cpp:496–500); stopped on a normal exit; it **outlives a killed PresentMon**, providers enabled, until something stops it or the PC restarts [MS-StartTrace] |
| What it writes | The CSV only: no log (a null log channel, LogSetup.cpp:4–7) and no ETL. `w` mode empties a file of that name [MS-fopen] |
| Other processes | `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)` and `QueryFullProcessImageNameW`, to name a presenting process it didn't see start (OutputThread.cpp:294–319): within CL:9 |

- **Session names.** `KairoBench` for the bench and `KairoDoctor` for the doctor's test (doctor:460). Never the default `PresentMon` (CommandLine.cpp:384), so Kairo can't stop another tool's session (R13:540). Adrenalin's own session name is [Unverified].
- **Orphans.** devrun's input or time stop kills PresentMon and leaves `KairoBench` running. Cleanup [proposed]:
  - (a) Right after any job that didn't end normally, `pc bench` runs the inbox `C:\Windows\System32\logman.exe stop KairoBench -ets` [M] from the elevated session, only when neither `\Kairo\Run` nor a `PresentMon-2.6.0-x64.exe` runs. It writes no file (M1:81), and no third-party program runs elevated for it (M1:171–172).
  - (b) The next setup stops it as you (§4.3 step 8), with PresentMon's own `--terminate_existing_session`, as does a run's time-out (§4.4 step 4).
  - (c) The robust fix, in PR 5: a job-file field `"etw_sessions": ["KairoBench"]` that `kairo-devrun` stops by name, as you, whenever it ends the job, for any reason. It takes only names matching `^Kairo[A-Za-z]{1,32}$`, so it can never stop another tool's session. Unlike (a), it works when Claude's session is gone, and it lands before Q1's run. devrun's safety-stop PR (M1:135) then inherits it.
  - (d) `bench-pre` and `pc doctor` list the running session names read-only (`logman query -ets` [M]) and report a leftover.
- **The pin.**
  - The binary: `C:\kairo\tools\presentmon\PresentMon-2.6.0-x64.exe`, 980,320 bytes, SHA-256 `b2a706bc6ad475749e3b7e3409263aa1e6906d45bdcf993f6dbc0f660188f1af` (boot:169–174, doctor:75–77). That equals GitHub's asset digest [Live], and v2.6.0 (2026-09-21) is still the latest release [Live].
  - `tools\` is admin-owned, and you can read and run it (M0:61).
  - `bench-pre` re-hashes it every session and refuses a mismatch. A missing file is re-installed between your sessions with the bootstrap's own steps (as for FFmpeg, M1:163). A wrong hash is a different matter: only an administrator can change `tools\` (M0:61), so bench work stops, and Claude reports the file's size and hash to you before anything replaces it (CL:42–43).
  - A new version changes PLAN:308, both pins and M1's contract.
  - MIT licence. Never shipped, committed or mirrored.

### 4.6 `pc bench` on the Mac

```
pc bench calibrate [--monitor 180hz] [--workloads alu,bw] [--force]
pc bench selftest [--monitor 180hz] [--force]
pc bench aa [--monitor 180hz] [--workloads alu,bw] [--runs 6] [--rehearsal] [--no-activate] [--no-cover]
            [--no-extend] [--fetch-csv] [--force]
pc bench fetch STAMP [--csv] [--force]     after a dropped connection or a stop; refuses while the job runs
pc bench cleanup                           stops an orphaned KairoBench session; never refuses
pc bench env [--force]                     env.json alone, read-only
pc amd-diff before|after [--label NAME] [--driver] [--force]
```

- `--no-cover` is for the one rehearsal without covers (§6); the full A/A refuses it.
- **Busy checks.** Every bench verb and `amd-diff` calls `Assert-PcIdle` first, as pc:7 requires: a fullscreen window refuses even with `--force` (hn/lib:355), and input in the last 10 minutes refuses unless `--force`. Only `pc bench cleanup` never refuses, written into pc's header beside harness's two exemptions (hn/pc:9–10): it is one inbox `logman` call that runs only when no job or PresentMon runs, and an orphaned ETW session would cost your next game.

**Steps of `pc bench aa`.**
1. **Mac.**
   - Take `.dogfood/bench/.lock` (with `mkdir`; a lock whose PID is gone is stale).
   - Run any pending checks first (the `tdr-pending` and `cleanup-pending` markers, as `harness_pending_checks` does, hn/pc:904–928).
   - Map `--monitor` and the `oled` alias to device paths through `.dogfood/pc/monitors.json`, refusing `oled` as the load's monitor (hn/pc:880–888).
   - Write the job and check it (`JQ_JOB_CHECK`, pc:603–653).
   - Take the counts and the fingerprint from `.dogfood/bench/calibration.json`, or refuse ("run pc bench calibrate first").
   - Keep the Mac awake for the wait (`caffeinate -i`): a sleeping Mac loses the progress and the decision point.
2. **PC, `bench-pre.ps1`** (read-only):
   - `Assert-PcIdle 'bench' $true` first, so nothing below runs while you play;
   - `env.json` (§5.4) and `encoder-usage -Seconds 3 -Json`;
   - PresentMon's SHA-256 (a mismatch stops bench work, §4.5);
   - any process named `PresentMon*`, `kairo-loadgen` or `LogonUI` (names only);
   - the screen saver, Dynamic Lock and inactivity-lock settings (§5.4). It refuses (exit 3) when a screen saver or an inactivity lock would start before the job's `max_seconds` runs out (its time-out ≤ the current idle time + `max_seconds`), for every plan, since `SetThreadExecutionState` "does not stop the screen saver from executing" [MS-STES]; and when Dynamic Lock is on, for the full A/A;
   - `EnableMetricsOverlay` at 1 refuses, once Q2 has confirmed the value's name (§9 risk 33);
   - the `logman query -ets` session names and the state of `\Kairo\Run`.

   Results come back as Base64 `@@KV` lines; the Mac keeps `env-before.json` and `encoder-before.json`.
3. **PC, `bench.ps1`**:
   - `Assert-PcIdle 'bench' $true`: a fullscreen window refuses even with `--force` (hn/lib:355);
   - `Start-KairoJob` and `Send-KV started`;
   - then a wait of up to `max_seconds` + 30 s, with progress read from `bench.json` about every 10 s ("alu run 3/6 recorded"). It returns when the job ends, or with `@@KV awaiting=1` when `bench.json`'s `phase` is `awaiting_decision`.
   - Its `StopWhen` is only a backstop. `Stop-KairoRun` kills devrun before it writes a result, so `bench.json` never gets its outcome (lib:600–604, :633–637). So it ends the job only when a fullscreen window other than the job's own has been in front for 10 s, by when the job's own watcher, which polls every second, should have acted; and it ignores `--force`.
4. **The decision point** (`aa` only):
   - the TDR check of step 5 on the events so far; exit 7 → the decision "no extra runs", then the P0 report;
   - `bench-fetch.ps1` (step 8) on the runs `bench.json` lists as recorded, while the job waits; meanwhile the job writes nothing but `bench.json`, through a rename;
   - `bench-rules -Decide` proposes the extra runs per workload (§4.7);
   - `bench.ps1` again, with the decision in the prelude. It reads the busy state as `Assert-PcIdle` does, but **without** `--force` (M1:96), and when the PC is busy, unknown or shows a fullscreen window it writes the decision "no extra runs" instead and says why. It writes the decision file through `Write-TextFileAtomic` (lib:163–170), keeps the newest 5 decision files, then waits as in step 3.
   - With `--no-extend`, or in a rehearsal, the job gets `--decide-wait 0` and goes straight on.
5. **TDR check.** The Mac writes `tdr-pending`; the PC runs `harness-events.ps1` with its id rule widened to `^(harness|bench)-[0-9]{8}-[0-9]{6}\z` (hn/remote/harness-events.ps1:9). Exit 7 is a P0: stop all PC work and report (M1:214).
6. **Cleanup** after any job that didn't end normally (§4.5).
7. **After a stop** (exit 6, whether your input, the time limit or the bench's own stops ended it), nothing else runs now. The fetch, the statistics and `env-after` wait for `pc bench fetch STAMP` on an idle PC, as `harness capture` returns without fetching unless its job exited 0 or 1 (hn/pc:1091).
8. **PC, `bench-fetch.ps1`** (`Assert-PcIdle` first):
   - `bench.json` (≤1 MiB) through `Read-SharedText`;
   - each CSV (≤64 MiB) read once as bytes through `Open-ReadFile`, as the clip fetch reads the clip (hn/remote/harness-fetch.ps1:61–96). Its SHA-256 and gzip come from those bytes, and `pm-stats` gets the same bytes decoded by their BOM. `Read-SharedText` drops PresentMon's BOM (lib:156), so a hash of its text wouldn't match the file;
   - `pm-stats` per CSV with `-ProcessId`;
   - `bench-rules` validity per run and per workload;
   - `ab-stats` in A/A mode per workload over the valid runs, plus its paired self-test over runs (1,2), (3,4), (5,6) [proposed];
   - with `--fetch-csv`, each CSV gzipped in memory and sent as 48 KiB `@@KV csv=` blocks, with the SHA-256 of the raw CSV and of the gzip, one call per CSV;
   - without a devrun result file (devrun was killed), freshness from `bench.json`'s `run_id` and `started_utc` instead, since harness-fetch refuses a job with no result (hn/remote/harness-fetch.ps1:17–18).

   The tool scripts travel as Base64 in the prelude (`PmStatsB64` and so on) and are dot-sourced from memory under §2 item 1's rules; the prelude goes on stdin, so cmd's 8,191-character limit (pc:52) doesn't apply.
9. **PC, `bench-pre.ps1` again** gives `env-after.json`. `bench-rules -CompareEnv` compares §5.4's fields; a difference means "the environment changed during the session, so it doesn't count" (M1:217).
10. **Mac, the report.**
   - Per workload: the valid runs, the mean `avg_fps`, the CV, the present-mode shares and the flags.
   - A CV still above 1%: "fix the setup", with `env.json`'s suspects (thermals, background CPU, the power plan; R13:831).
   - On every way out, this step included: Instant Replay's state from `env-after` (or `env-before`, after a stop). If it reads off, Claude's closing message asks you to switch it back on (Q3).

**`pc bench fetch STAMP`** collects a session after an SSH drop or a stop. It checks the busy state first (exits 3 and 4; `--force` with your OK), refuses while `\Kairo\Run` runs (exit 5), and runs the pending TDR check first. `env-after` is then taken late, but is still compared.

### 4.7 Validity, extra runs and the CV rule

**A run counts only if all of these hold** [proposed]. `bench-rules.ps1` applies them; M2's A/B reuses them, with rules 7 and 8 open for B runs (§4.12).

The job sees rules 1 and 2 itself and re-runs a failed slot, at most `--retries` (3) times per workload; one more failure stops that workload's block (`retries_exhausted`):
1. PresentMon exited 0 after printing both `Started recording.` and `Stopped recording.`, and the run isn't `stopped_early`. The CSV exists and its last byte is `\n`. Loadgen was still running when PresentMon ended, and its ready file came in time. PresentMon's exit 6 isn't retried: it is the access problem of §4.9. Runs through the elevated fallback (§4.2) print no recording lines, so their rule 1 is: exit 0, a header and at least one row on stdout, and the elevated session's own start and end times around the recording.
2. No `etw_loss`.

The elevated side judges rules 3 to 8 at the decision point and after the job:
3. `pm-stats` exits 0 with `bad_rows` 0 and `swapchains` 1.
4. `|duration_s − timed| ≤ 1.0`. A loadgen exit shortens the recording. A stall doesn't: PresentMon prunes a swap chain only when another target chain presents 4 s later (PM/OutputThread.cpp:247–292, :434, :555; UnifiedSwapChain.cpp:17–21), which a single loadgen chain never meets, so a stall stays in the CSV as one long frame (rule 7, §5.1).
5. `frame_types` holds only `Application`. `AMD AFMF` means AFMF is on (R13:866).
6. One `present_modes` entry holds at least 99% of frames, and it is the same entry in every valid run of the workload in the session, extra runs included.
7. `long_frames` is 0: no frame over 250 ms, loadgen's own stall threshold (§3.10) [proposed].
8. `dropped_pct` ≤ 1.0 [proposed]. Loadgen's ~125 fps is below the refresh rate, so nothing should drop; a display that is off, or a locked screen, should drop nearly everything [Unverified]. The first rehearsals show the real share.

**Extra runs**, at the job's one decision point [proposed]. `bench-rules -Decide` proposes, per workload:
- 6 valid runs and a CV of 1.00% or less: none;
- 6 valid runs and a CV above 1.00%: 6 more, the extension (n = 12);
- 3 to 5 valid runs: 6 − valid make-up runs. If the CV over the 6 is then above 1%, the report says so, and a later session extends it;
- fewer than 3: none, and "insufficient data: fix the setup", since something systematic failed and the rules name it.

Each extra block starts with its own warm-up. After the second pass, a workload with fewer than 6 valid runs is "insufficient data" (`pc bench aa` exit 8).

**Flags** are reported but don't invalidate a run:
- an average within 1% of the refresh rate (a cap: FRTC, forced VSync, Chill);
- `gpu_busy_ms × avg_fps / 1000 < 0.9`: the GPU isn't the limit, so something capped the run, such as Chill, or the CPU limited it;
- fewer than 1,000 frames;
- any present with `SyncInterval` other than 0, `AllowsTearing` other than 1 or `PresentFlags` other than 512 (§5.1's count objects);
- for the session: a program other than Kairo's with video-engine use above 1% in `encoder-before` or `encoder-after`, such as a recorder (R13:827).

**The CV rule.**
- CV = 100 · s / x̄ over the valid runs' `avg_fps`, with s the sample SD (n − 1), as `Measure-Object -StandardDeviation` computes it ([PS-Measure]: 1.58113883008419 for 1..5). The population SD reads about 9% low at n = 6.
- A CV of 1.00% or less is fine. Above 1.00%: one extension (n = 12) at the decision point, with the CV over all 12. Still above 1%: reported as high.
- Nothing is trimmed: a run leaves the set only by the rules above.
- Diagnostics, not gating: the CV of `low1_fps`, and the least-squares drift of `avg_fps` per run as a percentage of the mean. Thermal creep shows in the drift; ABBA cancels linear drift, but an A/A CV doesn't.
- Six runs estimate σ only loosely. Its 95% interval is [0.62 s, 2.45 s] at n = 6 and [0.71 s, 1.70 s] at n = 12 [Mac]. "CV ≤1%" screens the setup; the A/B's own interval decides M2's gate.

**What a run CV buys M2's ABBA** [Mac]. This assumes independent runs of equal variance, so the paired SD ≈ √2 · CV and h = t(0.975, n−1) · √2 · CV / √n:

| Run CV | Half-width at 6 pairs | Pairs for h ≤1.0 pp |
|---|---|---|
| 0.25% | 0.37 pp | 3 |
| 0.50% | 0.74 pp | 5 |
| 0.674% | 1.00 pp | 7 |
| 1.00% | 1.48 pp | 11 |
| 1.50% | 2.23 pp | 20 |
| 2.00% | 2.97 pp | 34 |

At a CV of 1%, a B whose true loss is above about 2.5% fails "upper ≤4%", though it passes "mean ≤3%". The plan's 1 pp between the two clauses needs a run CV of about 0.67% or less at 6 pairs. That is the number gate 4's data gives M2.

### 4.8 Outputs, and what comes back

| Where | What |
|---|---|
| `C:\kairo\logs\bench-<stamp>\` (made by you; the newest 5 are kept) | `bench.json`; `ready-<w>-<nn>.json` (and `-warmup`); `<w>-<nn>.log` (loadgen); `<w>-<nn>.csv`; `pm-<w>-<nn>.out` and `.err`; `loadgen-<w>-<nn>.out` and `.err`; `calibration-<w>.json` under `--plan calibrate`; `selftest-testbed.log` under `--plan selftest`. `<w>` is `alu` or `bw`; `<nn>` counts from 01, retries and extra runs included |
| Beside it | devrun's `bench-<stamp>.result.json`, `.out.log` and `.err.log`; devrun's `PRUNED` gains `("bench-", 5)` (hn/devrun/run.rs:130) |
| `C:\kairo\jobs\` (admin-only) | `bench-<stamp>.decision.json`, written by the elevated session; the newest 5 are kept |
| `.dogfood/jobs/` | `bench-<stamp>.json` |
| `.dogfood/bench/<stamp>/` | `env-before.json`, `env-after.json`, `encoder-before.json`, `encoder-after.json`, `bench.json`, `stats.jsonl`, `rules.json` (validity and the proposal), `decision.json`, `aa.json` (ab-stats per workload), `csv/<w>-<nn>.csv.gz` with `--fetch-csv`, and the `tdr-pending` and `cleanup-pending` markers |
| `.dogfood/bench/calibration.json` | The current counts and their history |
| `docs/` | The gate report's line (M1:221) and M1's contract rows. Never a CSV, `env.json`, a model name or a driver version (M1:87, CL:18) |

**`bench.json`** [proposed]. It is written as you and stays a local file.
- Top level: `kairo_bench: 1`, `run_id`, `started_utc`, `ended_utc`, `phase` (`setup`, `first_pass`, `awaiting_decision`, `second_pass`, `report`), `outcome` (`complete`, `partial`, `failed`, `stopped_fullscreen`, `stopped_foreground`, `stopped_locked`, `stopped_overlay` or `stopped_watch`), `exit_code`, `decision` (`none`, `applied` or `timeout`, with the extra runs) and `foreign_presentmon`.
- `plan`: workloads, runs, retries, `warmup_s`, `delay_s`, `timed_s`, `gap_s`, `decide_wait_s`, counts.
- `monitor`: `refresh_mhz`, `hdr`, `size`, `adapter_luid`, `umd_version`, `hags`, `fingerprint`.
- `displays_start` and `displays_end`: each active display's role (`bench` or `cover`), refresh, HDR, size and adapter LUID, read with `kairo-display` at setup and at the end. The before/after comparison and `encoder-usage`'s LUID mapping use these, not `.dogfood/pc/displays.json`, which no bench step refreshes and whose LUIDs are new after every restart [M].
- `presentmon`: file, session, `leftover_stopped`. `covers`: shown (how many) or `not_needed`.
- `blocks[]`: `{workload, pass, warmup: {ok, seconds}, runs: [...]}`. Each run holds `{n, tag, pid, csv, csv_bytes, csv_ends_newline, pm_exit, pm_started, pm_stopped, etw_loss, stopped_early, loadgen_exit, loadgen_alive_at_pm_end, retry_of, loadgen: {fps, gpu_ms_p50, gpu_ms_p99, gpu_ms_max, gpu_unknown, gpu_disjoint, occluded}, elapsed_s}`.
- No device path, name or foreground program. The job's stdout (`.out.log`) holds phase names, counts, exit codes and file names only.
- With the elevated fallback (§4.2), the Mac's copy records each run's `route` (`job` or `elevated_stdout`), since the elevated session writes nothing in `logs\`.

**What the CSV reveals.**
- Loadgen's file name, a PID, a swap-chain pointer, how loadgen presents, and frame timings.
- Raw QPC ticks since boot, so the uptime: a minor PC detail.
- Never a user, account, monitor, adapter or GPU name, or a path. `--process_id` keeps other programs out.
- Raw CSVs are never committed (CL:18).

**Sizes** [Estimate]:
- about 270 bytes a row [Mac], so about 2 MB per 60 s CSV at 125 fps and about 25 MB per session;
- about 5 MB to fetch after gzip [Estimate: ≈5×].

The Mac checks its free space first (2.5 × the size + 1 GiB, as the clip fetch does), since its disk is nearly full. Statistics always come back; CSVs only with `--fetch-csv`, which is on for the first full A/A as the gate's evidence.

### 4.9 Failures and exit codes

| Event | What happens | Recovery |
|---|---|---|
| Your input | devrun stops the whole job tree (exit 6). Finished runs' CSVs and `bench.json` stay; `KairoBench` is orphaned until devrun's `etw_sessions` (PR 5) or the cleanup stops it | Cleanup at once, and nothing else until the PC is idle again: `pc bench fetch STAMP` then. A complete workload block counts; the rest runs in a new session, on a new "go" |
| A fullscreen window in front (a game) | The job stops itself (exit 6, `stopped_fullscreen`). The Mac's `StopWhen` is a 10 s backstop while connected | As above |
| Another program comes to the front: a game started with a controller, a windowed game | The job stops itself (`stopped_foreground`) | As above |
| The PC locks (Dynamic Lock, a screen-saver lock), or a `PresentMon*` appears | The job stops itself (`stopped_locked`, `stopped_overlay`) | As above; `bench-pre` checks those settings first |
| `watch.json` goes stale | The job stops itself (`stopped_watch`) | The watch agent restarts with `\Kairo\Ensure` (M1:164) |
| An SSH drop, or the Mac asleep | The job runs on, and its own stops hold. At the decision point it waits `--decide-wait`, then ends with the first pass's data | `pc bench fetch STAMP` |
| `max_seconds` reached | devrun's `timeout`, exit 6 | Report it; cleanup. The limits stay below it (§4.3), so this means a hang |
| PresentMon exit 6, "access denied" | The block stops; `kairo-harness bench` exits 4 | The elevated `--output_stdout` fallback (§4.2). Signing out and back in would refresh your account's groups (PMREADME:60), but it is a login and closes your programs, so it is only ever a question to you, with that cost, and never done remotely (CL:42, §8) |
| Setup's PresentMon exit 7, "failed to terminate existing session" | Refused (exit 3) | `pc bench cleanup`, then again |
| No CSV, a cut CSV, a missing recording line, loadgen gone before PresentMon, a ready time-out | The job re-runs the slot (at most 3 per workload) | Repeated: the block stops, and the report names the rule |
| ETW loss | The job re-runs the slot | Repeated: `--set_circular_buffer_size 4096` for that workload, A and B alike (PM/MainThread.cpp:455–459) |
| A run fails rules 3 to 8 | Invalid; make-up runs at the decision point (§4.7) | Fewer than 6 valid after them: "insufficient data" (exit 8) |
| loadgen exits 3, 4 or 5 | The block stops | Exit 4 after `ready` → the events check. Exit 5 → re-run with `--no-activate` if the rehearsal showed that activation doesn't matter |
| The display off | Nearly every present dropped | Invalid (rule 8); start on "go" with the displays on |
| Event 4101 or 41 | P0 | Stop all PC work and report (M1:214) |
| An update during the session | `env-after` differs | The session doesn't count (M1:217) |

**`kairo-harness bench` exits** [proposed, following `run`'s, hn/phases.rs:6–20]:
- 0: complete; every planned run recorded a CSV (validity is judged on the elevated side);
- 1: a step failed or timed out (`bench.json` says which);
- 2: bad command line, or the folder exists;
- 3: refused (the monitor isn't found or is `--not-monitor`, HDR, disk space, Remote Desktop, a locked session, a leftover PresentMon, loadgen or `KairoBench` session, a missing program, a cover that can't be shown, a fingerprint that differs from the calibration's);
- 4: PresentMon couldn't start its session;
- 5: internal;
- 6: it stopped itself (a fullscreen window, another program in front, a lock, a new `PresentMon*`, a stale `watch.json`).

**`pc bench aa` exits**:
- 0: complete (data, whatever the CV);
- 1: failed;
- 2: usage;
- 3: busy, a fullscreen window, or a refusal at the start (a screen saver or lock that could fire, Dynamic Lock, a stale calibration, a cover that can't be shown);
- 4: busy state unknown;
- 5: still running when the wait ended;
- 6: stopped (input, the time limit, or one of the job's own stops);
- 7: P0;
- 8: insufficient data: a workload with fewer than 6 valid runs after its extra runs [proposed];
- 255: ssh couldn't connect.

**The other verbs' exits** [proposed]:
- `pc bench calibrate` and `selftest`: as `aa`'s, without 8. `selftest` exits 0 only when the session ended `stopped_foreground` or `stopped_fullscreen` within 2 s of the testbed's activation, and 1 otherwise.
- `pc bench fetch`: 0; 1; 2; 3 and 4 (busy, unknown); 5 (the job still runs); 7.
- `pc bench cleanup`: 0 (stopped, or nothing to stop); 1 (`logman` failed); 5 (refused: a job or a Kairo PresentMon runs).
- `pc bench env`: 0, 1, 2, 3, 4.
- `pc amd-diff`: §5.5.

### 4.10 Time budget [Estimate]

| | Job | Mac before and after | Total |
|---|---|---|---|
| One run | About 2 s to ready + 66 s of PresentMon + about 1 s to stop = about 69 s, after a 30 s gap | | |
| One workload block | 123 s of warm-up + 6 × (30 + 69) s ≈ 12 min | | |
| Rehearsal (1 workload, 20 s warm-up, 2 × 20 s runs, 10 s gaps) | About 1.7 min | About 0.75 min | About 2.5 min |
| Stop test (`selftest`) | Up to about 4 min, mostly the activation wait | About 0.5 min | Up to about 4.5 min |
| Calibration, both workloads | About 5.5 min | About 0.5 min | About 6 min |
| Full A/A, first pass, 2 workloads | About 24 min | About 2–3 min | About 26–27 min |
| The decision point | The job waits | About 1–2 min (TDR check, statistics, the decision) | About 1–2 min |
| One extension | About 12 min | About 1 min | About 13 min |
| k make-up runs | About 2 + 1.65 k min | | |
| Both workloads extended | | | About 55 min |
| M2's ABBA, one job | 123 + 4 × 99 s ≈ 8.6 min, plus the capture's or the engine's start and stop | | About 9–10 min |

The first loadgen of a job adds up to about 205 s when your last input was recent (the foreground lock time-out). With that wait, the full A/A takes about 26–30 minutes, and with both workloads extended about an hour. The job's hard limit is about 97 minutes (§4.3).

### 4.11 When it may run

| Run | Length | When | Conditions |
|---|---|---|---|
| Loadgen's first checks, calibration, the stop test, rehearsals | 2–6 min each | You're home, have said "go" in chat, and keep off the keyboard and mouse | `pc status` shows no fullscreen window; Claude starts at once with `--force` on your OK; the displays are on; `abort_on_input` is on; the covers keep the other displays black, except in the one rehearsal without them, when the OLED shows your desktop for about 2 minutes, as in gate 2's attended runs (R16:773–777) |
| The full A/A, with its extra runs | About 26 min to an hour | (a) one attended exception (Q1), recorded in M1:103, or (b) while you're away on agreed test days, once devrun's safety stops exist (M1:35, M1:135) | The bench's covers and its own stops (§2 item 25), after a passing stop test; no screen saver, inactivity lock or Dynamic Lock that could fire (`bench-pre` checks); daytime or evening, never overnight (M1:37) |
| M2's ABBA | About 9–10 min per job | Test days; game benchmarks with you there (PLAN:440) | §4.12 |

Always:
- `pc status` first, and `pc doctor` each session (CL:23, M1:96);
- after a stop for your input, a new "go" before the next job (§2 item 23);
- deploys only between your sessions (CL:24);
- one job at a time (M1:98);
- the load on the 180 Hz monitor only (CL:26);
- no game (M1:101).

An update between sessions is fine: `env.json` keeps the data from before and after apart (M1:217).

### 4.12 Designed for M2's A/B, built in M2

- **`--plan abba`**: A B B A ×3 per workload, 12 runs (R13:833, PLAN:353), with a `condition` per run in `bench.json`.
  - **What B is.** In SP1, B is the FFmpeg CLI capture, `gfxcapture` into `h264_amf` with research 13's options (PLAN:427), started by the bench job as a child the way `kairo-harness run` starts FFmpeg. The engine with its buffer armed (R13:834) is B from M3 on. B runs fall under M1:100's encoding rules (NV12, every option set and read back, peak ≤50 Mbps, one H.264 session at a time, at least 1 s apart), and the PR that adds them is a media PR, which needs a green harness smoke (CL:34).
  - **One A B B A per job** [proposed], about 8.6 minutes [Estimate: 123 s of warm-up + 4 × 99 s], so three jobs give a workload's six pairs. Every job then fits SP1's OLED limit if the load runs there (M1:40: chunks of at most 10 minutes, with the cover in between; M1:203 recommends that the job is the run). Its 30 s gaps would show the desktop on the load's own display, so M2 adds a cover there between runs [proposed].
- **Pairs by order.** In each job, A B B A pairs as (1,2) and (4,3), each pair adjacent in time; over a 12-run order, A at 1, 4, 5, 8, 9 and 12 and B at 2, 3, 6, 7, 10 and 11 pair the same way [Mac].
- **Open for M2, with their constraints:**
  - **Game benchmarks.** You start each one when the beep says so (PLAN:440), at −30 dBFS (CL:26), and it runs fullscreen. Such a job needs `abort_on_input` off, which M1:98 allows when "you're meant to use its window"; an exemption for the target game in the fullscreen and foreground stops; and the game's PID, found after you start it by image name in a Toolhelp snapshot (which opens no process) and passed as `--process_id`. Unelevated PresentMon holds no SeDebugPrivilege and opens only `PROCESS_QUERY_LIMITED_INFORMATION`. The first PresentMon run on an anti-cheat game is practice or unranked, with you there (CL:27).
  - **HDR.** The OLED is the HDR screen (PLAN:629). Loadgen refuses HDR, so SP1 on the OLED, or PLAN:353's "one HDR run", needs an HDR format first [Unverified: the OLED's HDR state when SP1 runs].
  - **The other workloads**: video on the 180 Hz screen, alt-tab between monitors, and a Copy for Discord export during play (PLAN:353; M5's G3, PLAN:474).
  - **Later reuse**: M3's untimed 3-pair A/B (CB:1195) and M5's 30-minute bench session with you (PLAN:482).
  - **Rules 7 and 8** (§4.7) drop a run with a hitch or drops. A hitch Kairo causes in B is part of what's measured, so M2 counts long frames and drops per arm instead of dropping those runs.
  - **The analysis**: `present_modes` and `gpu_wait_ms` deltas between A and B (R13:838), from pm-stats' existing keys.
- **The rest.** `encoder-usage` samples one B run, timed from `bench.json`'s progress (R13:835). `ab-stats -A -B` gains the SP1 clauses (§5.2).
- **The off-list.** PLAN:353's list of AMD features and R13:823–828's differ. Their union, plus Anti-Lag, goes into SP1's checklist (M1:201).
- **Questions the bench answers later**: whether capture demotes a game from independent flip (R13:784); window versus monitor capture cost (R13:785); `SetGPUThreadPriority(-2)` under HAGS (R13:790); whether AFMF frames are visible (R13:783).

---

## 5. The scripts

`tools/pc/bench/` holds `pm-stats.ps1`, `ab-stats.ps1`, `encoder-usage.ps1`, `bench-rules.ps1` (§4.7) and `regdiff.ps1` (§5.5). Each:
- is ASCII and parses under Windows PowerShell 5.1 (so no `??`, no ternary, no `&&`) and runs on PowerShell 7.6.7 (ci:216–247, PLAN:308);
- uses no `Add-Type`, since the bootstrap fix drops it from elevated sessions (M1:171);
- is a library of functions with a guarded command line, safe to dot-source (§2 item 1);
- keeps the Never list and the privacy rules unaided: CI's house-rules scan reads only Rust (M1:102), so §6 adds a scan of these files.

### 5.1 `pm-stats.ps1`

**Interface.** `pm-stats.ps1 -Csv <path> -Exe <file name> -Tag <tag> [-ProcessId <pid>] [-Strict]` (M1:85).
- It defines `Get-PmStats -Text <string> -Exe -Tag [-ProcessId] [-Strict]`; `-Csv` reads the file and calls it.
- The elevated script dot-sources the text it received and passes the CSV's bytes decoded by their BOM (§4.6 step 8).
- `-Exe` is compared with `Application` exactly, ignoring case.
- `-Tag` must match `^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$` [proposed], so no path or PC detail can ride in it.

**The CSV it reads.** PresentMon 2.6.0's unified writer (PM/CsvOutput.cpp:530–672), with the bench's flags: 28 columns in this order.

`Application, ProcessID, SwapChainAddress, PresentRuntime, SyncInterval, PresentFlags, AllowsTearing, PresentMode, FrameType, TimeInQPC, MsBetweenSimulationStart, MsBetweenPresents, MsBetweenDisplayChange, MsInPresentAPI, MsRenderPresentLatency, MsUntilDisplayed, CPUStartQPC, MsBetweenAppStart, MsCPUBusy, MsCPUWait, MsGPULatency, MsGPUTime, MsGPUBusy, MsGPUWait, MsVideoBusy, MsAnimationError, AnimationTime, MsFlipDelay`

Without `--no_track_input`, `MsAllInputToPhotonLatency` and `MsClickToPhotonLatency` follow.

- **Names.** Under `--qpc_time`, the first time column is `TimeInQPC` (R13:557's `TimeInMs` is the default), and both QPC columns hold raw ticks (CsvOutput.cpp:558, :620). The README's `VideoBusy` and `CPUStartTime` are the `--v2_metrics` names (CsvOutput.cpp:581–600; PMR:97, :104). So columns are read by header (R13:559).
- **Format.**
  - UTF-8 with a BOM, and CRLF line ends [MS-fopen];
  - comma-separated and never quoted, so a program name with a comma shifts its row;
  - numbers in the C locale, with `.`, and `NA` where a value is missing;
  - created at the first recorded row, and absent if there is none.
- **GPU columns.** `MsGPUTime` = `MsGPUBusy` + `MsGPUWait` (CsvOutput.cpp:1260–1263).
  - Busy is the time at least one of the process's packets ran on a non-video engine; wait is the gaps inside the frame's GPU span (PM/mc/MetricsCalculatorCpuGpu.cpp:83–99, :137–145).
  - Video-engine work goes to `MsVideoBusy`: the **target's** video work, not Kairo's (PM/PresentData/GpuTrace.cpp:205–227, R13:864).
- **`FrameType`** prints `Application` (also for unset and repeated frames), `Intel XeSS-FG`, `AMD AFMF` or `Unknown` (CsvOutput.cpp:54–70).

**Row semantics** that the statistics depend on:
- **One row per displayed instance.** A present shown several times (frame generation) gives several rows with the same `TimeInQPC` and `MsBetweenPresents`. Only the application instance carries GPU and CPU numbers (PM/mc/MetricsCalculator.cpp:181–234, MetricsCalculatorCpuGpu.cpp:14–117).
- **Dropped presents** keep their row, with `NA` display columns (MetricsCalculator.cpp:100–137).
- **Lost presents** leave no row, so the next interval spans the gap (OutputThread.cpp:437–440).
- **A swap chain's first present** only seeds it (OutputThread.cpp:355–360).
- **A stall stays in the CSV.** PresentMon prunes a swap chain whose last present is more than 4 s older than the batch's reference time (OutputThread.cpp:247–292; UnifiedSwapChain.cpp:17–21). That time starts at 0 in every batch (OutputThread.cpp:434), and only a target chain's own presents set it, apart from the first present of a process the output thread hasn't met yet (:333–337, :351–365); a batch that leaves it at 0 prunes nothing (:260–262, :555). During a loadgen stall the batches hold only DWM's presents, which always pass the PID filter (PresentMonTraceConsumer.cpp:3609–3612) but set nothing. So a single chain is never pruned, and a stall becomes one long `MsBetweenPresents` with `duration_s` unchanged. Pruning drops a stall only while another target chain keeps presenting. `max_frame_ms` and `long_frames` catch it (§4.7 rule 7).
- **HAGS.** With HAGS on, GPU busy may read about 0.5 ms large (PMREADME:75–84). GPU columns are compared only between runs with the same HAGS state, from loadgen's read-out (§3.10).

**Reading.**
1. Detect the BOM (UTF-8 expected, UTF-16 accepted), and split lines on CRLF or LF.
2. Split the header on `,`.
   - Required: `Application`, `ProcessID`, `SwapChainAddress`, `PresentMode`, `MsBetweenPresents`, `MsUntilDisplayed`, `MsGPUBusy`, `MsGPUWait`, and one time column (`TimeInQPC`, `TimeInMs`, `TimeInSeconds` or `TimeInDateTime`).
   - `FrameType` is optional; without it, `frame_types` is null and a warning prints.
   - A `--v2_metrics` header, which has no `MsBetweenPresents`, exits 3 with that reason.
   - `-Strict`, which the bench passes, also requires `TimeInQPC`, `FrameType` and `MsVideoBusy`, so a CSV made without M1:74's flags can't pass [proposed].
3. A row with a different field count is a bad row. So is a last line without a newline: a killed PresentMon's cut tail.
4. Numbers are parsed with `[double]::TryParse(s, NumberStyles.Float, CultureInfo.InvariantCulture)`. `NA`, empty and non-finite values are missing.

**Selection and formulas.**
1. **The process.** Rows where `Application -ieq $Exe` (and `ProcessID` = `-ProcessId`).
   - None → exit 3, with counts only on stderr, never another program's name.
   - Several PIDs without `-ProcessId` → exit 3.
2. **The main swap chain** is the `SwapChainAddress` with the most rows; a tie goes to the one seen first. R13:580's `Sort-Object` leaves ties to an unstable sort. `swapchains` is the number of chains.
3. **Presents**: the main chain's rows grouped by the time column. A present's frame time, GPU numbers and mode come from its first `FrameType = Application` row, else its first row.
4. **Frames**: presents whose `MsBetweenPresents` > 0; the others count in `bad_rows`. n = `frames`; F = their `MsBetweenPresents` values.
5. **`avg_fps`** = 1000 · n / ΣF. This is time-weighted and includes dropped presents, since the app made them (R13:563, CB:1076).
6. **Nearest rank.** Sort F ascending; P(p) = F[k − 1], with k = (99·n + 99) div 100 for P99 and (999·n + 999) div 1000 for P99.9.
   - That is ⌈p·n⌉. R13:577's floating-point form gives the same k for every n ≤ 2,000,000 [Mac].
   - `low1_fps` = 1000 / P99; `low01_fps` = 1000 / P99.9; `low_def` = `p99_nearest_rank` states the definition (R13:564–565).
   - Below 100 frames, P99 is the maximum. A warning prints below 1,000 frames.
7. **GPU means and P99s.** `gpu_busy_ms`, `gpu_wait_ms` and `gpu_time_ms` are means over the chosen rows; missing values are skipped and zeros kept. R13's sketch averages every row, which mixes in generated rows' zeros (vector P2). `gpu_busy_p99_ms`, `gpu_wait_p99_ms` and `gpu_time_p99_ms` are nearest-rank P99s over the same values (R13:567).
8. **Display cadence** (secondary, R13:568): `display_change_ms` and `display_change_p99_ms`, the mean and nearest-rank P99 of `MsBetweenDisplayChange` over the counted presents where it isn't `NA`.
9. **Counts.** `present_modes` counts per present. `frame_types` counts over all instance rows of the counted presents, so `{"AMD AFMF": …}` shows AFMF at work. `sync_interval`, `allows_tearing` and `present_flags` count per present, keyed by the column's text; a missing column gives null.
10. **Diagnostics** [proposed]:
   - `max_frame_ms` = the largest F; `long_frames` = the frames with F > 250 ms (§4.7 rule 7);
   - `duration_s` = ΣF / 1000;
   - `dropped_pct` = 100 · (presents with `MsUntilDisplayed` NA) / n;
   - `swapchains`;
   - `rows`: the process's well-formed rows across all its chains;
   - `bad_rows`.
11. **Nothing is trimmed by time.** The warm-up is a separate run, and `--delay 5` keeps PresentMon's own start-up out of the CSV.

**Output.** One compact JSON line: M1:85's keys first, then the additions that M1:85 gains in the same PR.
`{"pm_stats":1,"tag":"…","frames":n,"avg_fps":x.xx,"low1_fps":x.xx,"low01_fps":x.xx,"gpu_busy_ms":x.xxx,"gpu_wait_ms":x.xxx,"present_modes":{"<mode>":n,…},"frame_types":{"<type>":n,…},"low_def":"p99_nearest_rank","gpu_time_ms":x.xxx,"gpu_busy_p99_ms":x.xxx,"gpu_wait_p99_ms":x.xxx,"gpu_time_p99_ms":x.xxx,"display_change_ms":x.xxx,"display_change_p99_ms":x.xxx,"max_frame_ms":x.xxx,"long_frames":n,"sync_interval":{"<value>":n,…},"allows_tearing":{"<value>":n,…},"present_flags":{"<value>":n,…},"duration_s":x.xxx,"dropped_pct":x.xx,"swapchains":n,"rows":n,"bad_rows":n}`
- **Rounding**: fps to 2 decimals, ms and seconds to 3, percentages to 2, with `[math]::Round(x, d, [MidpointRounding]::AwayFromZero)`.
- **Serialisation**: `ConvertTo-Json -Compress` from an `[ordered]` dictionary, which writes numbers invariantly. R13:592's `-f` strings follow the culture ("99,5 %" on a German PC).
- **Warnings** go to stderr, one line each: `swapchains` above 1, `bad_rows` above 0, `frames` below 1,000, `long_frames` above 0, a frame type other than `Application`, `FrameType` missing.
- **Exits** [proposed; kairo-verify's convention, M1:90]:
  - 0: the line is printed;
  - 2: bad command line (a missing argument, a bad `-Tag`, a `-ProcessId` that isn't a positive integer);
  - 3: can't evaluate (no file; unreadable or empty; a required column missing; no row for the process; several processes; no valid frame).

  It never judges a run; `bench-rules` does.

### 5.2 `ab-stats.ps1`

**Interface** [proposed]. `ab-stats.ps1 -A <runs> [-B <runs>] [-Json] [-Reference <ab-stats JSON line>]`.
- `<runs>` is a `[string[]]` of pm-stats JSON lines, a file of them in run order, or bare numbers (`avg_fps`, R13:599's form). With bare numbers, only the mean and upper-bound clauses can be evaluated. One kind per call.
- `-A` alone is the A/A mode; `-A` and `-B` pair the runs by order index.

**Paired loss** (M1's scope, R13:597–606):
- dᵢ = 100 · (Aᵢ − Bᵢ) / Aᵢ from `avg_fps`. Positive is a loss (B = Kairo on).
- m = mean(d); s = √(Σ(dᵢ − m)² / (n − 1)); h = t(0.975, n − 1) · s / √n; CI = [m − h, m + h].
- `PASS = (m ≤ 3) and (m + h ≤ 4)` (R13:731, PLAN:353).
- Unequal counts or n < 2 → exit 2. Any Aᵢ ≤ 0 → exit 3.

**The t quantile** t(0.975, df).
- df 1–30 [Mac; these match the published values]: 12.7062, 4.3027, 3.1824, 2.7764, 2.5706, 2.4469, 2.3646, 2.3060, 2.2622, 2.2281, 2.2010, 2.1788, 2.1604, 2.1448, 2.1314, 2.1199, 2.1098, 2.1009, 2.0930, 2.0860, 2.0796, 2.0739, 2.0687, 2.0639, 2.0595, 2.0555, 2.0518, 2.0484, 2.0452, 2.0423.
- Beyond 30, the Cornish–Fisher series, with z = 1.959963984540054 and ν = df:
  t ≈ z + (z³ + z)/(4ν) + (5z⁵ + 16z³ + 3z)/(96ν²) + (3z⁷ + 19z⁵ + 17z³ − 15z)/(384ν³) + (79z⁹ + 776z⁷ + 1482z⁵ − 1920z³ − 945z)/(92160ν⁴).
  Its error is 3.1×10⁻⁸ at df 30, 7.3×10⁻⁹ at 40, and 3.0×10⁻¹¹ at 120 [Mac].

**A/A mode.** n, the mean, the sample SD and CV = 100 · s / mean; the CV of `low1_fps`; the drift per run (§4.7). `noise_ok` = CV ≤ 1.00.

**Output** (invariant culture, `F2`):
- paired: `6 pairs: loss 2.44% (95% CI 2.25..2.63%) PASS=True`. This is M1:85's form, with one space before `PASS`; R13:605 has two, and its `{1:N2}` follows the culture.
- A/A: `6 runs: avg_fps 120.33 sd 0.71 (CV 0.59%) NOISE=ok` [proposed].
- `-Json`, one line at full precision:
  - paired: `{"ab_stats":1,"mode":"ab","n":…,"loss_pct":…,"sd_pp":…,"t":…,"ci_lo":…,"ci_hi":…,"clauses":{"mean_le_3":…,"upper_le_4":…,"low1_le_5":null,"present_mode":null,"ir_plus_1pp":null},"pass":…}`;
  - A/A: `{"ab_stats":1,"mode":"aa","n":…,"mean_fps":…,"sd_fps":…,"cv_pct":…,"low1_cv_pct":…,"drift_pct_per_run":…,"noise_ok":…}`.

  `null` means not evaluated.
- **Exits**: 0 PASS, or noise ok; 1 FAIL, or noise high; 2 bad command line (unequal counts, n < 2, mixed kinds); 3 can't evaluate (an Aᵢ ≤ 0, an unparsable line).

**The clauses that join before SP1** (specified here; built then, M1:85, M1:201) [proposed]:

| Clause | Source | Formula |
|---|---|---|
| Mean loss ≤3% | PLAN:353 | As above (M1) |
| Upper 95% bound ≤4% | PLAN:353 | As above (M1) |
| 1%-low loss ≤5% | PLAN:353, R13:731 | The same pairs on `low1_fps`, mean ≤5; the plan asks for no interval here |
| PresentMode unchanged | PLAN:353, R13:731 | `present_modes` pooled per arm: the same dominant mode; B's dominant share ≥ A's − 1 pp; no mode new in B with a share of 1% or more |
| No worse than Instant Replay alone + 1 pp | PLAN:353, PLAN:427 | m(Kairo) − m(IR) ≤ 1.0 pp, both paired losses from the same session and workload; m(IR) from `-Reference` |
| A release regression above 1 pp blocks | PLAN:346 | m(new) − m(baseline) ≤ 1.0 pp, against the last accepted baseline for the same workload and `env.json` fingerprint |

R13:840's "3 pairs, expanding to 6 if the result is within 1% of a decision threshold" reads as: expand when |m − 3| ≤ 1 or |m + h − 4| ≤ 1. That is `pc bench`'s decision, not `ab-stats`'.

**In M1**, `ab-stats` runs in A/A mode for gate 4. It also runs once paired over A/A runs (1,2), (3,4) and (5,6) of each workload [proposed]: a self-test whose interval should include 0, and whose half-width shows what M2's 6 pairs can resolve.

### 5.3 `encoder-usage.ps1`

- **Interface.** `encoder-usage.ps1 -Seconds N [-Json]` (M1:86): video-engine use per process, adapter LUID and engine type, with file names only.
  - It only reads. The scripts that call it check the busy state first (§4.6).
  - `bench-pre` runs it before and after each bench job. From PR 9c, the harness flow runs it before each capture (R16:865); today hn/pc:1120 prints "encoder use: not measured yet (PR 9c)". `harness-pre` runs before `run.ps1`'s busy check (hn/pc:1058, :1066), so PR 9c samples in `run.ps1`, after that check [proposed].
- **Sampling** [proposed]. One `Get-Counter -Counter '\GPU Engine(*)\Utilization Percentage' -MaxSamples 1 -ErrorAction SilentlyContinue -ErrorVariable pdhErrors` per second, N times (R13:611, CB:1077), filtered in code:
  - Each call expands the wildcard once, before it samples (PS/GetCounterCommand.cs:444, :478). One call with `-MaxSamples N` would never see a program that starts using an engine during the window; one call per second sees it from the next second.
  - Get-Counter discards its own priming read (:488–511), so each sample it returns is a valid rate over its interval, and none is dropped. Failed reads count toward `-MaxSamples` (:513–524).
  - A program that exits mid-sample leaves a sample whose `Status` isn't 0, and Get-Counter writes a non-terminating error for that set (:611–625; PS/PdhHelper.cs:1234–1297). Under lib.ps1's `$ErrorActionPreference = 'Stop'` (lib:22) the error would throw, so the call sets `-ErrorAction SilentlyContinue`; the set still comes back, its invalid samples are dropped and counted (`invalid_samples`), and `samples` counts the sets returned.
  - An explicit `-Counter` path isn't translated to the PC's language; only the default counters are (:413–441). A localized Windows may therefore not find the set (exit 3, §9 risk 21). The Rust sampler of CB:1077, with `PdhAddEnglishCounterW` and `PdhGetFormattedCounterArrayW`, has neither problem, and is the route for engine-side sampling in M2.
- **Parsing.**
  - Each instance name, which `Get-Counter` lower-cases (PS/PdhHelper.cs:1160; R13:613), is matched against `^pid_(\d+)_luid_0x([0-9a-f]{8})_0x([0-9a-f]{8})_phys_(\d+)_eng_(\d+)_engtype_(.+)$`.
  - The engine type is lower-cased with spaces removed; video means it starts with `video` (CB:1077). AMD reports `videocodec`, which includes decode (R13:618, M1:195).
  - Every type is counted once, in a census (M1:194: "logs every type").
- **Names.**
  - One `[Diagnostics.Process]::GetProcesses()` call before sampling and one after, reading `Id` and `ProcessName` only, through the one helper that may call it (§6's scan). `GetProcesses` builds every object from one `NtQuerySystemInformation` snapshot (NET/Process.cs:1081–1107, NET/ProcessManager.Windows.cs:63–67), and `ProcessName` returns the snapshot's name without opening the process (NET/Process.Windows.cs:892–904). PowerShell 7.6.7 runs on .NET 10 (its global.json), so this is the code that runs [M: the shipped runtime is a 10.0 servicing release].
  - A PID gone by the second snapshot prints `(exited)`.
  - `Get-Process -Id` would open each PID with `PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE` (NET/Process.cs:1035–1043, NET/ProcessManager.Windows.cs:29–37): allowed, but needless.
  - Never `.Path`, `.MainModule`, `.Modules`, `.Company`, `.FileVersion`, `.ProductVersion`, `.Description` or `.Product`, which go through `MainModule` (PS/TypeTable_Types_Ps1Xml.cs:1136–1244), nor `.CommandLine`, a WMI query for another program's command line (:1148–1165). Nor `Format-List *`, `Select-Object *`, `ConvertTo-Json` or `Export-Csv` on a Process object, which read every property, `Path` included [M].
- **Output**: one JSON line. Without `-Json`, a table of the rows with `max_pct` above 0.

```json
{"kairo_encoder_usage":1,"seconds":3,"samples":3,"invalid_samples":0,"elevated":true,"types":{"3d":24,"copy":12,"videocodec":4},
 "video":[{"name":"AMDRSServ","pid":5120,"luid":"0x00000000000123ab","engtype":"videocodec","eng":4,"avg_pct":3.1,"max_pct":7.5,"seen":3}]}
```

  LUIDs are in kairo-display's form, `0x` and 16 hex digits with the high part first (display/lib.rs:51–55). The Mac maps each one to "the 180 Hz monitor's adapter" or "the other adapter" through a display record from the same boot: the session's `bench.json` (§4.8), since LUIDs are new after every restart [M] and `displays.json` may be older. Both adapters are AMD (M1:66), so vendor ids can't tell them apart.
- **Exits**:
  - 0: sampled;
  - 2: usage;
  - 3: the counter set can't be read: it is missing on a CI runner, or has a localized name [Unverified]. The Rust sampler avoids the latter with `PdhAddEnglishCounterW` (R13:649).
- **What your account can see** (cookbook item 6, M1:194).
  - Run it once elevated and once as you, over the same seconds. The second runs as a short `\Kairo\Run` job: `"C:\Program Files\PowerShell\7\pwsh.exe" -NoLogo -NoProfile -NonInteractive -File C:\kairo\src\tools\pc\bench\encoder-usage.ps1 -Seconds 5 -Json`. `C:\kairo\src` is admin-owned (M0:61), and the output lands in the job's `.out.log`.
  - M1:72 says jobs run programs from `bin\current`; a job that runs a script from `C:\kairo\src` is a new pattern, so PR 1 adds it to M1's contract.
  - Then compare the PID sets, and write the engine-type name and the result into M1 beside its item (M1:194). This is a one-off, in the attended evening.
- **Cost and privacy.** Sampling `Get-Counter` at 1 Hz costs CPU, so in M1 it never runs during a recorded run; M2 samples one B run (R13:835). The output names every program that uses a video engine (file names), so it stays in `.dogfood/` (CL:18).

### 5.4 `env.json`

It is read elevated and read-only, in `bench-pre.ps1`, as one JSON object [proposed]. It covers research 13's list (R13:819–822), whether AMD Software is installed (M1:87), and your RAM, which PLAN's Q1 still needs (PLAN:663).

| Field | Read from | Why |
|---|---|---|
| `windows`: `display_version`, `build` (`CurrentBuild.UBR`), `edition`; `reboot_pending`; `uptime_s` | The values `DisplayVersion`, `CurrentBuild`, `UBR` and `EditionID` of `HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion` by name, never the whole key, which holds `RegisteredOwner`, `ProductId` and `DigitalProductId` [M]; `…\WindowsUpdate\Auto Update\RebootRequired` [M]; `Win32_OperatingSystem`'s `LastBootUpTime` alone (`Get-CimInstance -Property`) | R13:820; an update during the milestone (M1:217) |
| `gpus[]`: `driver_version`, `driver_date`, `pci`, `radeon_software_version` | `Win32_VideoController` (`DriverVersion`, `DriverDate`, `PNPDeviceID`); the display class key `…\Class\{4d36e968-e325-11ce-bfc1-08002be10318}\00NN` whose `MatchingDeviceId` holds the adapter's `VEN_`/`DEV_`, value `RadeonSoftwareVersion` (boot:797–830 matches by `DriverDesc`, and its value names are TODO(verify)); matched to LUIDs through `bench.json`'s displays and their PCI ids | R13:820. That WMI's version equals DXGI's UMD version (loadgen's `umd_version`) is [M] |
| `amd_software`: `installed`, `version`, `basis` | Uninstall entries whose `DisplayName` starts with "AMD Software" [M]; `RadeonSoftware.exe` under `C:\Program Files\AMD\CNext\CNext\` [path Unverified, R13:471]; the class key | M1:87 |
| `amd_hints` | `HKLM:\SOFTWARE\AMD\DVR` `InstantReplayEnabled`, `InGameReplayEnabled`; your `Software\AMD\DVR` `DvrEnabled`, `DesktopRecordingEnabled`; your `Software\AMD\CN\Performance` `EnableMetricsOverlay`. Each an integer or `absent` (hn/remote/harness-pre.ps1:9–28). The names are [Unverified] until the diff script confirms them (R13:472, R13:374) | PLAN:353's off-list |
| `amd_processes` | Names only, from one `GetProcesses()`: `RadeonSoftware`, `AMDRSServ`, `AMDRSSrcExt`, `amdow`, any `PresentMon*` (R13:473, R13:540) | Coexistence; a second PresentMon |
| `hags`: `hw_sch_mode`, `effective` | `HKLM:\SYSTEM\CurrentControlSet\Control\GraphicsDrivers` `HwSchMode` (2 on, 1 off, absent = the driver's default [M]), the setting, applied at boot; `effective` from loadgen's own `D3DKMT_WDDM_2_7_CAPS` read-out in the session's `bench.json` (§3.10, CB:1080), so no wait for `wgc-probe` | R13:820; PresentMon's GPU columns (§5.1) |
| `dx_user` | Your `Software\Microsoft\DirectX\UserGpuPreferences` `DirectXUserGlobalSettings`: `VRROptimizeEnable`, `SwapEffectUpgradeEnable`, `AutoHDREnable` [M] | Windows' VRR and windowed-game settings |
| `vrr` | `unknown` until the diff script finds FreeSync's value [Unverified]. No read-only API was found without ADLX, which waits for a licence review (R13:428–433) | R13:820 |
| `displays[]`: `role`, `refresh_mhz`, `hdr`, `size` | The session's `bench.json` (`displays_start`, `displays_end`, §4.8), joined on the Mac; `.dogfood/pc/displays.json`, which no bench step refreshes, only names the aliases | R13:820 |
| `power`: `scheme`, `overlay_ac`, `display_off_ac_s` | The active scheme's GUID from `powercfg /getactivescheme`, with a label for Windows' own plans (Balanced, High performance, Power saver, Ultimate Performance) and `custom` otherwise, never a custom plan's name, which is yours [proposed]; `…\Control\Power\User\PowerSchemes` `ActiveOverlayAcPowerScheme` [M]; `powercfg /q SCHEME_CURRENT SUB_VIDEO VIDEOIDLE` [M] | A noise source (R13:831); the screen-off timer R16 left open (R16:773) |
| `lock`: `screen_saver` (active, time-out, secure, set), `dynamic_lock`, `inactivity_s` | Your `Control Panel\Desktop` `ScreenSaveActive`, `ScreenSaveTimeOut`, `ScreenSaverIsSecure`, and whether `SCRNSAVE.EXE` is set (never its path); your `Software\Microsoft\Windows NT\CurrentVersion\Winlogon` `EnableGoodbye`, Dynamic Lock [M: third-party documentation only]; `HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\System` `InactivityTimeoutSecs` [M] | `bench-pre`'s refusal (§4.6); M1:35. `SetThreadExecutionState` doesn't stop a screen saver [MS-STES] |
| `tdr` | The number of System events 4101 in the last 7 days, and of files in `C:\Windows\LiveKernelReports` (names not kept) | R13:771, CB:1080: a stability baseline beside the per-session P0 check |
| `game_mode`, `game_bar_capture` | Your `Software\Microsoft\GameBar` `AutoGameModeEnabled`; `System\GameConfigStore` `GameDVR_Enabled`; `…\CurrentVersion\GameDVR` `AppCaptureEnabled` [M] | R13:828 |
| `mpo` | `HKLM:\SOFTWARE\Microsoft\Windows\Dwm` `OverlayTestMode` (5 = MPO off) [M] | Present modes |
| `cpu`, `ram` | `Win32_Processor`'s name and cores; `Win32_ComputerSystem.TotalPhysicalMemory` (that property only) and `Win32_PhysicalMemory` capacity and speed (boot:772–795) | PLAN:663 |
| `background_cpu_pct` | `\Processor(_Total)\% Processor Time` over the encoder-usage seconds | R13:831 |
| `etw_sessions` | `logman query -ets` names [M] | Leftovers; Adrenalin's session name |
| `kairo` | `bin\current` → build id and commit (lib:447–468); PresentMon's SHA-256 against the pin; FFmpeg's `KAIRO-PIN.json` asset (hn/remote/harness-pre.ps1:30–36); the calibrated counts, shader hash and fingerprint; the scripts' commit, which the Mac adds: its `git rev-parse HEAD`, and whether `tools/pc/bench` or `tools/mac/remote` has uncommitted changes | R13:822; requirements note §8 |
| `game`, `kairo_engine` | `null` in M1, reserved for the game's settings and Kairo's commit and config (R13:821–822) | M2 |

- **Compared before and after** [proposed], by `bench-rules -CompareEnv`: `windows.build`, `reboot_pending`, every `driver_version`, `amd_software.version`, `hags`, the power scheme and overlay, both displays' refresh and HDR (from `bench.json`), `lock`, `amd_hints`, `mpo`, `dx_user` and `kairo.build_id`. A difference means the session doesn't count (M1:217). Sessions are pooled or compared only when these fields match.
- **"Your" hive.** The per-user values are read from `Registry::HKEY_USERS\<SID>`, with the SID of `\Kairo\Run`'s principal [proposed]. An administrator registered that task, so its principal can be trusted. The principal's account name is turned into a SID in memory and never printed, logged or sent. Today that hive is also the SSH account's own HKCU (R13:476), and the read stays right if Claude's SSH sessions move to a separate account (M1:172).
- **Privacy.** It holds PC details (model names, PCI ids, RAM, driver versions), so it lives in `.dogfood/bench/<stamp>/` only (M1:78, M1:87, CL:18).
  - It never holds the PC's name, account names, IP or MAC addresses, serial numbers or product keys, even locally (M1:221).
  - Every value is read by name (`Get-ItemPropertyValue -Name`, `Get-CimInstance -Property`) and copied into an `[ordered]` dictionary, as boot:657–672 and :782–795 project theirs. No whole registry key or CIM object is ever serialized: they carry `RegisteredOwner`, `RegisteredUser`, `PrimaryOwnerName`, `CSName`, `UserName`, `SerialNumber` and the like, which often hold your name or Microsoft account [M].
  - The terminal shows a three-line digest: Windows build, driver version, HAGS and power plan [proposed].

### 5.5 The AMD settings diff script

- **Files.** `tools/mac/remote/amd-diff.ps1`, with the snapshot, render and compare functions in `tools/pc/bench/regdiff.ps1` (tested in CI). The verb is `pc amd-diff before|after [--label NAME] [--driver] [--force]` [proposed].
- **Two calls per toggle** (M1:88):
  - `before` takes a snapshot and returns it as `@@KV snapshot=<Base64 JSON>`; the Mac keeps `.dogfood/amd-diff/<stamp>/before.json`.
  - You toggle one setting.
  - `after` receives the before snapshot back in the prelude (`BeforeB64`, on stdin). It takes a new snapshot, prints the differences, and returns its snapshot too.
- **It never writes.**
  - Keys are opened read-only with `[Microsoft.Win32.RegistryKey]::OpenBaseKey(…).OpenSubKey(path, $false)`. There is no `Set-*`, `New-*` or `Remove-*`, and AMD's settings are never written (R13:465, CB:1066); §6's scan checks it.
- **The busy check.** It calls `Assert-PcIdle 'amd-diff' $true` first: a fullscreen window refuses even with `--force`. You're at the keyboard by design, so every call passes `--force`, which your OK in that session gives (pc:6–8).
- **Scope.**
  - Whole subtrees: your `Software\AMD` (by SID, §5.4), `HKLM\SOFTWARE\AMD` and `HKLM\SOFTWARE\WOW6432Node\AMD`. They hold research 13's four keys (R13:479) and the hotkeys under `HKLM\SOFTWARE\AMD\DVR` (R13:509).
  - `--driver` adds every display-class key `…\Class\{4d36e968-…}\00NN` whose `ProviderName` is AMD, with its subkeys, each labelled by its `MatchingDeviceId`'s `VEN_`/`DEV_` part: both adapters are AMD (M1:66), and keys of removed devices stay behind [M]. Driver-level settings (Chill, Boost, AFMF, FRTC, Enhanced Sync, RSR, FreeSync) are said to live there [Unverified].
  - A subkey it may not read (a `SecurityException`) prints `<denied>`, and the walk goes on.
  - Limits: depth 8 and 20,000 values. Past them it stops and says so.
- **Names.** Every key and value name passes the plain-text test below (the pattern; no `\`, `/` or `@`; at most 64 characters; neither the account's nor the PC's name). A name that fails prints as `<name len=N sha256=8hex>`, in the lines and in the snapshots. Windows keys some per-app settings by a program's path [M], and how AMD keys its per-app profiles is [Unverified] (§9 risk 15).
- **Values.**
  - DWORD and QWORD values print as numbers.
  - `REG_SZ`, `REG_EXPAND_SZ` and `REG_MULTI_SZ` print only when short and plain: they match `^[A-Za-z0-9 _.,+=#:;()\[\]-]{0,64}$`, contain no `\`, `/` or `@`, and contain neither the account's nor the PC's name. `REG_EXPAND_SZ` is read with `DoNotExpandEnvironmentNames`, so `%USERPROFILE%` is never expanded into a path with your account's name.
  - Everything else prints as `<str len=N sha256=8hex>`, and `REG_BINARY` as `<bin len=N sha256=8hex>`.
  - So a change still shows, but a save folder such as `C:\Users\<name>\Videos\…` never leaves the PC (CL:16).
- **Lines** (M1:88): `HKCU\Software\AMD\DVR\DvrEnabled: 1 -> 0`, `… (absent) -> 1`, `… 0 -> (absent)`, type changes as `REG_DWORD 1 -> REG_SZ "1"`, then `N values changed`. Hives print as HKCU and HKLM, never as a SID.
- **Exits**: 0 printed; 1 a hive couldn't be read; 2 usage (a bad label, or `after` without a before snapshot); 3 busy (a fullscreen window, or input without `--force`); 4 busy state unknown.
- **The session** (about 10 minutes, Q2) [proposed]:
  1. A null pair first, two snapshots about 30 s apart with nothing touched, lists the values that change by themselves.
  2. Then Instant Replay, Record Desktop and the Metrics Overlay in turn (M1:42): one pair switching the setting and one switching it back. Both directions confirm the value, and your settings end as they began.
  3. The Metrics Overlay goes only once each way (R13:374, R13:740), switched from AMD Software's main window, kept on only briefly, with no other overlay (Game Bar, a game) opened meanwhile: Adrenalin 26.10.1 can crash or time out "when opening or closing other overlays ... while the ... Performance Metrics Overlay is enabled" (R13:257). The System log's 4101 and 41 check (`harness-events`) runs after the session, and either event is a P0 (M1:214).
  4. A toggle that changes nothing in scope is repeated with `--driver`. If that still finds nothing, the setting isn't in the registry, and SP9 notes it.
  5. With Q3, Instant Replay's two pairs are split: its "off" pair opens the evening, and its "on" pair closes it after the A/A, or comes first after any stop (Q3).
- **Outputs.** `.dogfood/amd-diff/<stamp>/{before,after-<label>}.json` and `diff-<label>.txt`.
  - Confirmed value names are AMD's, not PC details, so they may go into `docs/`: research 13's correction, and the bootstrap's and harness's probes (M1:184) [proposed].
  - M1:142's item ticks when the script runs; you confirm it in SP9 (PLAN:435).

---

## 6. Tests

| Tier | Where | What |
|---|---|---|
| **U** | Mac and Linux CI (`cargo test -p kairo-loadgen -p kairo-harness`; ci:50 gains loadgen) | **Loadgen**: CLI parse and refusals; `bands(h, n)` covers every row once at odd heights; the ramp schedule and guard predictions; the calibration fit (linear, noisy, non-linear → refused); frame constants in f64 (drift and scroll wrap seamlessly); log, ready, summary and calibration JSON; `mix32`'s avalanche; §3.11's photosensitivity contract on `visual.rs`. **`kairo-harness bench`**: the session plan (phases, limits, and a test like `the_limits_fit_the_job`, hn/phases.rs:102–119: every limit, the retry reserve and the second pass stay below `max_seconds`); PresentMon's command line word for word (a PID ≤0 refused; none of §2 item 14's banned flags); stderr classification of each warning and of exit 7's two texts, in UTF-16LE with and without a BOM; each ready-check mismatch, and which ones retry; `bench.json`; the watcher's decisions on synthetic `watch.json` files (loadgen in front, a game in front, a fullscreen sample with no name, stale); the foreground stop's decisions on synthetic samples (loadgen in front during a run, a known PID during a run, a new PID, a second change in a gap, no window for 5 s); the decision file's checks (owner, run id, counts 0 to 6, a timeout); the cover set from synthetic display lists, and `--not-monitor`; pruning; stdout names no device path, LUID, model or program |
| **X** | Mac | `cargo xwin check` and `clippy --target x86_64-pc-windows-msvc -p kairo-loadgen -p kairo-harness`. Loadgen needs no `FFMPEG_DIR`; the harness builds as today |
| **W** | Windows CI (ci:140–157) | (1) Every entry point compiles through `D3DCompile` with the production flags, and twice gives the same bytes (CB:1189). (2) WARP offscreen, 320 × 180: ALU at `count` 4 and bandwidth at `count` 4 over a 1 MiB texture, 4 bands, with `--detail` on and off. The read-back matches `visual.rs` within ±2 codes (transcendentals differ between WARP and the CPU), and the timestamp ring resolves [reported, not failed, if WARP gives no timestamps]. (3) A hidden window with a flip-discard swap chain on WARP presenting 30 frames, with the tearing flag only if WARP reports support [proposed; reported, not failed, if the runner has no usable desktop]. (4) `kairo-harness bench`'s Windows parts: a child killed and reaped at its deadline (as for `run`), and the cover window's creation |
| **PS** | Windows CI's `powershell` job (pwsh 7; the 5.1 parse; ASCII; ci:216–247), and Linux CI's preinstalled pwsh [M; proposed]. The Mac has no PowerShell (PLAN:327), so the [Mac] vectors come from the Python reference | Every tool runs both ways: dot-sourced under `Set-StrictMode -Version 3.0` and `$ErrorActionPreference = 'Stop'`, as the elevated session runs it, and through `pwsh -File` with its exit codes. pm-stats vectors P1–P13 and E1–E6; ab-stats vectors AB1–AB5, AE1–AE3, AA1–AA2, L1 and S1–S2, and the t table; `bench-rules` vectors V1–V8 and D1–D5 and the before/after comparison; encoder-usage's parser on synthetic instance names, the `(exited)` path, sets with a non-zero `Status`, and exit 3 where `GPU Engine` is missing; `regdiff`'s renderer and compare on synthetic snapshots (a path-named value, an account-named subkey, a `<denied>` subkey) and on a temporary key under the runner's HKCU, a test that refuses to run unless `GITHUB_ACTIONS` is set; `env.json`'s collector, run on the runner and parsed as JSON, holding none of `RegisteredOwner`, `RegisteredUser`, `PrimaryOwnerName`, `CSName`, `UserName` or `SerialNumber` and neither the runner's computer nor its account name (as ci:280–289 runs the doctor). **Scans** [proposed], since CI's house-rules scan reads only Rust (ci:58–76, M1:102), over the PowerShell AST, not a regular expression, which `[IO.Path]::` would trip (lib:103, stub.ps1:25): a member named `Path`, `MainModule`, `Modules`, `Company`, `FileVersion`, `ProductVersion`, `Description`, `Product` or `CommandLine` on anything but a type literal, in the bench scripts; `GetProcesses` and an unfiltered `Get-Process` anywhere in `tools/**/*.ps1` outside the one helper that returns `[pscustomobject]@{Id; Name}` (by-name lookups such as tools/mac/remote/status.ps1:11's `LogonUI` stay allowed); `Add-Type`, `SendInput` and `Import-Csv`; registry writes in `regdiff.ps1` and `amd-diff.ps1` (`Set-ItemProperty`, `New-Item`, `New-ItemProperty`, `Remove-Item`, `.SetValue(`, `.CreateSubKey(`, `.DeleteValue(`, `OpenSubKey(…, $true)`); and file writes in the elevated bench scripts other than `Write-TextFileAtomic`'s decision file. Fixtures are built in code, and no PC data is committed (CL:18) |
| **P** | The PC, attended (Q2) | The first-run checks below |

**Fixture rows.** They are built in code from §5.1's 28-column header, with these defaults:
- `Application` `kairo-loadgen.exe`, `ProcessID` 4242, `SwapChainAddress` `0x1A2B3C4D5E0`, `PresentRuntime` `DXGI`;
- `SyncInterval` 0, `PresentFlags` 512, `AllowsTearing` 1, `PresentMode` `Hardware: Independent Flip`, `FrameType` `Application`;
- `TimeInQPC` unique per present (1,000,000 + 80,000·i unless stated);
- `MsBetweenSimulationStart` `NA`; `MsBetweenDisplayChange` = `MsBetweenPresents` (`NA` when dropped); `MsUntilDisplayed` `2.5000`;
- `MsGPUBusy` `7.9000`, `MsGPUWait` `0.1000`, `MsGPUTime` their sum;
- `MsAnimationError`, `AnimationTime` and `MsFlipDelay` `NA`; every other column 0;
- `MsBetweenPresents` written with 14 decimals; a BOM and CRLF unless stated.

**pm-stats vectors** [Mac] (with `-Exe kairo-loadgen.exe` unless stated; compare numbers as numbers, since `ConvertTo-Json` may print `40.0` or `40`):

| # | Rows (differences from the defaults) | Expected |
|---|---|---|
| P1 | 10 presents; `MsBetweenPresents` 8 ×9, then 16; busy 7.0, 7.1 … 7.9; wait 0.0, 0.1 … 0.9 | `{"pm_stats":1,"tag":"aa-alu-1","frames":10,"avg_fps":113.64,"low1_fps":62.5,"low01_fps":62.5,"gpu_busy_ms":7.45,"gpu_wait_ms":0.45,"present_modes":{"Hardware: Independent Flip":10},"frame_types":{"Application":10},"low_def":"p99_nearest_rank","gpu_time_ms":7.9,"gpu_busy_p99_ms":7.9,"gpu_wait_p99_ms":0.9,"gpu_time_p99_ms":8.8,"display_change_ms":8.8,"display_change_p99_ms":16.0,"max_frame_ms":16.0,"long_frames":0,"sync_interval":{"0":10},"allows_tearing":{"1":10},"present_flags":{"512":10},"duration_s":0.088,"dropped_pct":0.0,"swapchains":1,"rows":10,"bad_rows":0}` |
| P2 | 6 presents, ms 10, 10, 10, 20, 20, 20. Each of the first 3 also has an `AMD AFMF` row before it (same `TimeInQPC` and ms, busy 0, wait 0); the app rows have busy 6.0, wait 0.5 | frames 6, avg_fps 66.67, low1 50.0, low01 50.0, busy 6.0, wait 0.5, `frame_types` `{"AMD AFMF":3,"Application":6}`, duration 0.09, rows 9. R13's sketch gives 9 frames, 75.00 fps and busy 4.0 |
| P3 | 8 presents, ms 5, mode `Composed: Flip`, busy 4.2, wait 0.3; rows 3 and 6 dropped (`MsUntilDisplayed` `NA`) | frames 8, avg_fps 200.0, busy 4.2, wait 0.3, `present_modes` `{"Composed: Flip":8}`, dropped_pct 25.0 |
| P4 | Chain `0xAAAA0` ×5 (ms 10), chain `0xCCCC0` ×3 (ms 30), and `other.exe` ×5; `-Exe Kairo-Loadgen.EXE` | frames 5, avg_fps 100.0, swapchains 2, rows 8, bad_rows 0 |
| P5 | 6 good presents (ms 10); one row with `Application` `kairo,loadgen.exe` (one field too many); one with `MsBetweenPresents` `NA`; a last line cut to 40 characters with no newline | frames 6, avg_fps 100.0, `frame_types` `{"Application":6}`, rows 7, bad_rows 3 |
| P6 | P1 without a BOM, LF line ends only | As P1 |
| P7 | P1 without the `FrameType` column | Without `-Strict`: P1's numbers, `frame_types` null, one warning. With `-Strict`: exit 3, "missing FrameType (-Strict)" |
| P8 | P1 written as UTF-16LE with a BOM, CRLF | As P1 |
| P9 | Chain `0xAAAA0` ×4 (ms 10) first, then chain `0xCCCC0` ×4 (ms 20): a tie | The first-seen chain: frames 4, avg_fps 100.0, duration 0.04, swapchains 2, rows 8 |
| P10 | 200 presents, ms 1, 2 … 200 | avg_fps 9.95, low1 5.05 (P99 = 198 ms, rank 198), low01 5.0, duration 20.1 |
| P11 | 7,500 presents, ms 8 + 0.25·(i mod 5), except where i mod 250 = 249 → 25 ms; busy 7.5 + 0.01·(i mod 10); wait 0.05·(i mod 3) | avg_fps 116.77, low1 111.11, low01 40.0, busy 7.545, wait 0.05, duration 64.23. The 30 hitches (0.4%) miss the 1% low and set the 0.1% low |
| P12 | A stall: 100 presents of 10 ms, one of 4,500 ms, 100 of 10 ms | frames 201, avg_fps 30.92, low1 100.0, low01 0.22, max_frame_ms 4500.0, long_frames 1, duration 6.5 (§5.1: the stall stays in) |
| P13 | P1 with presents 3–4 at `SyncInterval` 1, 5–6 at `AllowsTearing` 0, 7–8 at `PresentFlags` 0 | `sync_interval` `{"0":8,"1":2}`, `allows_tearing` `{"0":2,"1":8}`, `present_flags` `{"0":2,"512":8}`; the rest as P1 |

**pm-stats error vectors.**
- E1: no file → exit 3.
- E2: a header only → exit 3.
- E3: a `--v2_metrics` header → exit 3, "missing MsBetweenPresents (a --v2_metrics CSV?)".
- E4: rows only for `other.exe` → exit 3.
- E5: two PIDs for `kairo-loadgen.exe` → exit 3 without `-ProcessId`, and that PID's statistics with it.
- E6: `-Tag ../x` → exit 2.

**ab-stats vectors** [Mac; recomputed for this merge]:

| # | Input (`avg_fps`) | Expected |
|---|---|---|
| AB1 | A 125.40, 124.90, 125.10, 125.60, 124.80, 125.20; B 122.10, 121.95, 122.40, 122.30, 121.70, 122.25 | `6 pairs: loss 2.44% (95% CI 2.25..2.63%) PASS=True` (s 0.1823, t 2.5706, h 0.1913) |
| AB2 | A 100 ×11; B 97.0, 97.5, 96.8, 97.2, 97.1, 96.9, 97.3, 97.0, 96.7, 97.4, 97.1 | `11 pairs: loss 2.91% (95% CI 2.74..3.07%) PASS=True` (t 2.2281). R13's sketch prints `CI 2.91..2.91` |
| AB3 | A 100 ×6; B 98 ×6 | `6 pairs: loss 2.00% (95% CI 2.00..2.00%) PASS=True` (s 0) |
| AB4 | A 100 ×6; B 96.0, 99.0, 97.5, 95.5, 98.5, 97.0 | `6 pairs: loss 2.75% (95% CI 1.31..4.19%) PASS=False`: the mean passes and the bound fails |
| AB5 | A 100 ×3; B 101.0, 100.5, 101.5 | `3 pairs: loss -1.00% (95% CI -2.24..0.24%) PASS=True` (a gain) |
| AE1–AE3 | 6 A runs vs 5 B; n = 1; an Aᵢ of 0 | Exit 2; exit 2; exit 3 |
| AA1 | 120.0, 121.2, 119.4, 120.6, 119.8, 121.0 | `6 runs: avg_fps 120.33 sd 0.71 (CV 0.59%) NOISE=ok`; population CV 0.54%; drift +0.0475% per run |
| AA2 | 118.0, 120.4, 121.9, 119.1, 122.3, 117.6 | `6 runs: avg_fps 119.88 sd 1.98 (CV 1.65%) NOISE=high` → an extension |
| L1 | `low1_fps` A 111.1, 110.8, 111.3, 111.0, 110.9, 111.2; B 106.0, 105.2, 106.4, 105.9, 105.1, 106.2 | Mean 1%-low loss 4.73% → `low1_le_5` true (an SP1 clause) |
| S1 | AA1 paired as the self-test does: (1,2), (3,4), (5,6) | `3 pairs: loss -1.00% (95% CI -1.01..-1.00%) PASS=True` (t 4.3027) |
| S2 | AA2 paired the same way | `3 pairs: loss 1.37% (95% CI -6.20..8.94%) PASS=False` |

**bench-rules vectors** [proposed]. Each V vector is one synthetic run record that breaks one rule and nothing else; the expected result is "invalid" with that rule alone. The D vectors are `-Decide` over a workload's valid runs.

| # | Input | Expected |
|---|---|---|
| V1 | `pm_stopped` false | Invalid, rule 1 (a slot the job retries) |
| V2 | `etw_loss` true | Invalid, rule 2 |
| V3 | `swapchains` 2 | Invalid, rule 3 |
| V4 | `duration_s` 58.7 at `timed` 60 | Invalid, rule 4 |
| V5 | `frame_types` `{"Application":7400,"AMD AFMF":7400}` | Invalid, rule 5 |
| V6 | `present_modes` `{"Hardware: Independent Flip":7350,"Composed: Flip":150}` (98%) | Invalid, rule 6 |
| V7 | `long_frames` 1 | Invalid, rule 7 |
| V8 | `dropped_pct` 1.5 | Invalid, rule 8 |
| D1 | AA1's 6 runs (CV 0.59%) | No extra runs |
| D2 | AA2's 6 runs (CV 1.65%) | 6 extra runs |
| D3 | AA1's first 4 runs (CV 0.64%) | 2 make-up runs |
| D4 | AA2's first 4 runs (CV 1.40%) | 2 make-up runs; the report notes the CV |
| D5 | 2 valid runs | None; "insufficient data" |

**The PC's attended first run** (Q2). It runs on a stacked build (main plus the window, loadgen and bench PRs, as research 16's PR 9a plan did), so the evidence exists before the merges.
- **`bench-pre`'s lock checks** read your screen-saver, Dynamic Lock and inactivity settings; the full A/A starts only once nothing could fire (Q2).
- **`pc bench calibrate`**: both fits with R² ≥ 0.99, each verify step within ±3%, no `too_heavy`, and the bandwidth workload's GB/s against the card's peak (§3.5).
- **`pc bench selftest`**: the session ends `stopped_foreground` or `stopped_fullscreen` within 2 s of the testbed's activation, before Q1's run relies on the bench's stops.
- **Three rehearsals**: with `--activate` and the covers (the A/A's setup), without `--activate`, and without the covers. Each should show:
  - `PresentMode` (`Hardware: Independent Flip` expected), `AllowsTearing` 1, `SyncInterval` 0, `PresentFlags` 512, `FrameType` `Application`, from pm-stats' count objects;
  - rows whose `Application` is `kairo-loadgen.exe` (the doctor's "csv-ok" checked only that the file wasn't empty, doctor:458–461);
  - `MsGPUBusy` > 0, `MsGPUWait` near 0, no ETW loss, `dropped_pct` near 0, `long_frames` 0, and loadgen's median GPU ms within ±1% across runs [proposed].
- **Also settled**: PresentMon records inside the Interactive job (no exit 6); `ready.json`'s LUID is the 180 Hz monitor's adapter; `SetForegroundWindow` works from a task-launched process; the waitable object works with the tearing flag; whether the cover hides the OLED's taskbar; and loadgen's `hags` read-out against `HwSchMode`.
- **The rest**: `encoder-usage` elevated versus as you (§5.3); System events 4101 and 41 after the AMD session and after each job: none.

---

## 7. PR plan

- Branches are `m1/<slug>`, with Conventional Commits and DCO, squashed after CI (CL:44).
- Each PR adds its names and formats to M1's contract first (M1:50), plus a CHANGELOG line.
- Sizes are lines of code plus tests [Estimate].
- None is a media PR (no capture, encode or audio), so none needs a harness smoke (CL:34) [proposed]. M2's B runs will.

| # | Branch | Scope | Size | Tested on | PC and owner |
|---|---|---|---|---|---|
| 0 | `m1/research-17` | This design and its four notes in `docs/research/17-loadgen-bench/` (Claude names the folder); CHANGELOG; PLAN:300's research list, and PLAN Appendix B's (PLAN:740, which lacks 16 too); M1's pointer to it beside M1:125's; PLAN:353's "environment in `bench/env.json`" → `.dogfood/` (M1:78), as #17 (3733339) did for research 16 | S (docs) | — | None |
| 1 | `m1/bench-scripts` | `tools/pc/bench/pm-stats.ps1`, `ab-stats.ps1`, `encoder-usage.ps1` and `bench-rules.ps1`, with their CI tests (§6 PS) and the PowerShell scans. M1:85–86 gain the new keys, the A/A line and the JSON forms; M1:86 also gains the module-list reason against `$p.Path` (CL:9). Correction notes at R13 §5.3 (`$p.Path`, R13:631) and CB §9.6, whose CB:1078 sends implementers to R13 §5.2–5.3. M1:72 gains the owner job that runs a script from `C:\kairo\src` (§5.3). tools/pc/uninstall-dev.ps1:115–118 reads every process's `.Path`, a module list on any game running then, so it changes to Kairo's own image names from the snapshot, which the scan needs anyway | M ~800 | Windows CI | None |
| 2 | `m1/amd-diff` | `tools/pc/bench/regdiff.ps1`, `tools/mac/remote/amd-diff.ps1` and `pc amd-diff before\|after`, with CI tests. M1:88 gains its line format, M1:92 the verb, and the Files table `.dogfood/amd-diff/` | S ~400 | Windows CI | Q2's 10 minutes at the PC. No deploy: the scripts travel on stdin |
| 3 | `m1/window-lib` | The shared crate (§3.1), `Cover` included. The testbed moves onto it with its WARP tests unchanged; the harness's `KeepDisplayOn` copy goes if #35 has merged. M1:64's Libraries line | M ~700 (mostly moved) | Mac (xwin), Linux and Windows CI | The testbed's attended acceptance (R16 §4.12, still to come, M1:126) runs on a build with PR 3, so it also checks the moved activation, window and `KeepDisplayOn` code that WARP can't. If the acceptance came first, a short attended smoke repeats `testbed.exe --activate`, borderless and exclusive |
| 4 | `m1/loadgen` | The crate (§3); ci:50 adds it. M1's contract: the program row, `run` and `calibrate`, the log, ready, summary and calibration formats, the exit codes. M1:13 and PLAN:352 say "the 180 Hz monitor in M1; the OLED only in M2's SP1, in chunks of ≤10 minutes" (M1:40) | L ~1,600 | Mac, Linux CI, Windows CI | The attended evening's first-run checks (§6), on the stacked build, before it merges |
| 5 | `m1/bench` | See below | L ~1,800 | Mac, Linux and Windows CI; the PC | The attended evening (Q1–Q3): calibration, the stop test, rehearsals, the first-run checks (§6), the visibility check, then the full A/A |

PR 5's scope:
- `kairo-harness bench` (`aa`, `calibrate`, `selftest`, the decision point, the watcher with its foreground, lock and overlay stops, the covers);
- devrun's `PRUNED` gains `("bench-", 5)`, and devrun gains `etw_sessions` (§4.5 c), with `kairo-core`'s job format, `Test-KairoJob`'s field list (lib:513) and `JQ_JOB_CHECK` (pc:603–653);
- `pc bench calibrate|selftest|aa|fetch|cleanup|env`, and `remote/bench-pre.ps1`, `bench.ps1`, `bench-fetch.ps1` and `bench-cleanup.ps1`; pc's header gains the cleanup's exemption (§4.6);
- `harness-events.ps1`'s id rule;
- the doctor skips its PresentMon test while `\Kairo\Run` runs [proposed] and lists ETW sessions;
- `env.json`;
- M1's contract:
  - Programs: M1:62's `kairo-harness` row gains `bench`;
  - Files: `bench-<stamp>\` and its names, `.dogfood/bench/`, `C:\kairo\jobs\<id>.decision.json`; M1:81's pruning sentence gains bench's 5 folders;
  - Formats: `bench.json`, `env.json`, the decision file;
  - M1:74: `--no_track_input`, `--process_id`, inside the job;
  - M1:92: the verbs;
  - M1:97 reworded (§2 item 12);
  - M1:103: Q1's exception, if you agree, dated and scoped to one evening and that one job, beside the 2026-10-31 build exception.

**Progress** (M1:119). PR 4 ticks M1:127. PRs 1 and 2 together tick M1:131. PR 5 with research 16's PR 9 ticks M1:132. The evening ticks M1:142, and gate 4's bench half of M1:140, whose other half is the `audio-soak` dry run. The visibility check settles M1:194, written into M1 beside it (§5.3). PR 5's report reconciles M1:201's two off-lists for SP1 (§4.12). The first bench's `env.json` fills PLAN:663's RAM row.

**Order and dependencies.**
- PR 0 first. PRs 1 and 2 depend on nothing and can merge now, alongside #32–#35. Research 16's PR 9c (`kairo-harness probe`, and the encoder record before each capture) uses PR 1's `encoder-usage.ps1` rather than writing its own.
- #31 merged on 2026-11-22 (0c6018b), before its attended PC run, so PR 3 can start now.
- PR 4 needs PR 3.
- PR 5 needs PRs 1 and 4 and #34–#35, for three reasons:
  - it extends `kairo-harness`;
  - it reuses `run`'s child handling, pruning and `harness-events.ps1`;
  - its job's arguments name device paths. Main's devrun still logs a job's whole command line (devrun/run.rs:98–102), and #34 reduces that to the program's file name and argument count (hn/devrun/run.rs:98–122).
- PR 5's deploy happens between your sessions (CL:24), from a stacked snapshot if the merges lag.

**Separate, not research 17.** devrun's safety stops (M1:135) reuse PR 3's `Cover` and PR 5's `etw_sessions`. The full A/A waits for them only if Q1's answer is no.

**The clock.** All of this is instrument work, outside the clock (M1:7, M1:110). The shared crate's edge (§3.12) goes into the day log's day 1 for your veto (M1:115), and PR 3 lands before the first `wgc-probe` commit [proposed].

---

## 8. Owner questions (real decisions only)

1. **The full noise-floor run, before the "away" safety stops exist.**
   - It takes about 30 minutes, up to about an hour if the numbers are noisy. Its hard limit is about 97 minutes, which only a string of failures could approach. That is longer than the short jobs M1 allows while you're home (M1:103).
   - Option (a): one evening when you stay home but off the PC for that time. You say "go" in chat, keep off the keyboard and mouse, and start no game (fullscreen, windowed or with a controller) until Claude says the run is done.
     - It stops at once at any keyboard or mouse touch. It stops itself within about 2 seconds when another program comes to the front, a fullscreen window appears or the PC locks. It keeps the OLED black throughout.
     - A game that never comes to the front wouldn't stop it, which is why "start no game" matters.
     - Claude records your yes in M1, dated, for that one evening and that one job, like the build exception of 2026-10-31.
   - Option (b): wait until the runner's safety stops are built and you've agreed test days (M1:35), then run it while you're out.
   - Recommendation: (a), on the same evening as Q2, once the stop test has shown that the bench's stops work. It closes gate 4 without waiting for test days, and the bench brings its own game-start stop for this one job.
2. **One evening at the PC: about 30 minutes with you nearby**, before Q1's run.
   - About 10 minutes switching Instant Replay, Record Desktop and the Metrics Overlay in AMD Software, one at a time when Claude asks, each one off and back on (Instant Replay back on only after the full run, Q3). Switch the Metrics Overlay from AMD Software's main window, keep it on only briefly, and open no other overlay or game meanwhile: Adrenalin 26.10.1 can time out when overlays open and close while it is on (R13:257). Claude only reads the registry, changes nothing, and checks the System log afterwards.
   - About 20 minutes hands-off for loadgen's first checks: the calibration, a test of the bench's stops and three short rehearsals. The 180 Hz screen shows the dim, drifting load, and for a few seconds the testbed's pattern; the OLED stays black under the bench's cover, except for about 2 minutes in one rehearsal.
   - Windows lets the load come to the front only once your last touch is about 3½ minutes old, so the first check may wait that long.
   - **For that evening, Dynamic Lock and any screen saver off**, if they're on, in Windows' settings, and back on afterwards if you like. Either would lock or cover the screen during the long run, and the bench checks them before it starts. They're your security settings, so you change them, not Claude (CL:42).
   - Recommendation: yes, any evening. The AMD part also ticks M1:142's item.
3. **AMD settings during the full run.** Instant Replay records all the time (R13:451), and the M2 comparison is measured with it off (PLAN:353). Chill and FRTC would cap a load that nobody touches (R13:366).
   - The settings that change loadgen's runs (R13:823–828): Instant Replay, Record Desktop, In-Game Replay, the Metrics Overlay, AFMF, Chill, Boost, FRTC, Enhanced Sync, RSR, Anti-Lag and the global "Wait for Vertical Refresh"; Game Bar capture; and any other recorder (OBS, Medal, Discord streaming, Steam recording).
   - Recommendation: switch Instant Replay off as the evening's first AMD toggle, and back on as the last, after the full run. Claude's closing message shows its state whatever happens, and after any stop the "on" toggle comes first. Look once that the others are off; switch any that's on off for the evening and back on afterwards. If you like, Claude records each such switch with the diff script (two short calls each), which SP9 needs anyway.
   - If you'd rather keep Instant Replay on, the run is labelled "Instant Replay on" and still counts for M1. If you'd rather keep Chill or FRTC on, the runs are likely capped (flagged, §4.7), and their noise floor says little about M2's loads.
4. **The bench's black cover: capture-excluded or not.** M1:99 and PLAN:334 describe the runner's cover as "capture-excluded". A black, topmost window hidden from screen capture is also the usual shape of a cheat overlay hidden from anti-cheat screenshots [Unverified], and nothing captures during an M1 bench run, so the exclusion buys nothing there.
   - Option (a): the bench's cover isn't capture-excluded. The shared crate keeps the option for runs that capture, such as devrun's runner or gate 3 on the OLED (M1:203).
   - Option (b): as written, capture-excluded.
   - Recommendation: (a). Either way the cover ends with the session when a game comes to the front (§2 item 25).

**Already open elsewhere**: test days (M1:35); Claude's separate SSH account (M1:172), which this design already handles by reading your settings by account id (§5.4).

**Later, not now.**
- M2's built-in game benchmarks (about 30 minutes per game, PLAN:440).
- The first PresentMon run on an anti-cheat game: practice or unranked, with you there (CL:27).
- If the noise stays above 1%: whether to change the power plan or background tasks, which are PC changes outside Kairo's folders (CL:43). The recommendation will be to change nothing in M1 and report it.
- Only if PresentMon can't record inside the job and the elevated fallback fails too (§9 risk 1): whether you'd sign out and back in, which refreshes your account's groups but closes your programs. Claude would ask, with that cost, and never do it remotely (CL:42).

---

## 9. Risks, fallbacks and [Unverified] items

| # | Item | How it gets settled | Fallback |
|---|---|---|---|
| 1 | PresentMon records inside the Interactive `\Kairo\Run` job: Performance Log Users survives in its filtered token, and the GPU, `FrameType` and `MsVideoBusy` columns fill [Unverified; M0 proved an S4U token, M0:166] | The first rehearsal (exit 6, "access denied", if not) | The elevated `--output_stdout` route, with its own rule 1 (§4.2, §4.7). Signing out and in (PMREADME:60) only as a question to you (§8) |
| 2 | `SetForegroundWindow` from a task-launched process after the lock time-out [Unverified; R16 risk 22] | The testbed's attended run (#31), then the first rehearsal | `--no-activate` |
| 3 | Independent flip without activation, with the taskbar on that monitor [Unverified]. Microsoft says DWM can keep independent flip under other content by reverse composition or MPO [MS-FLIP] | The rehearsal pair with and without `--activate` | `--activate` always. "Hardware Composed: Independent Flip" is acceptable if it is the same in every run |
| 4 | `ALLOW_TEARING` with `FRAME_LATENCY_WAITABLE_OBJECT` on D3D11 [Unverified] | The first PC run | A blocking `Present` with `IDXGIDevice1::SetMaximumFrameLatency(2)` |
| 5 | `GetFrameStatisticsMedia` answers for an RGB swap chain [Unverified] | Logged beside PresentMon | `media=unavailable`; hints are never verdicts |
| 6 | An SDR swap chain on an HDR output is composed [Unverified] | — | HDR refused in M1 |
| 7 | WARP: 256 MiB textures (its 128 MB limit) and timestamp queries [Unverified] | PR 4's Windows CI | A 1 MiB texture; timing reported, not failed |
| 8 | `d3dcompiler_47.dll` on `windows-latest` [Unverified] | PR 4's Windows CI | `LoadLibraryExW(SYSTEM32)` |
| 9 | A Windows update changes the compiler's output | The bytecode hash in every header | Recalibrate; treat it like a driver update (M1:217) |
| 10 | Preemption granularity on the RX 9000-series [Unverified] | Loadgen's read-out (§3.10) | Bands keep draws at about 0.5 ms either way |
| 11 | The 180 Hz monitor driven by the iGPU, not the card [Unverified] | `displays.json` and the ready check | The bench refuses. A cable change is a physical action, so Claude asks (CL:43) |
| 12 | Chill, FRTC, Anti-Lag or Enhanced Sync change the caps or the queue [Unverified] | Q3's look; the `gpu_bound` and near-refresh flags; the diff script's `--driver` | Off globally (R13:823–828), with Anti-Lag added to that list |
| 13 | VRR (FreeSync) changes tearing, and how `AllowsTearing` reads [Unverified] | The first runs, with VRR as you have it | Record it; compare like with like |
| 14 | An A/A CV of 1% or less for a synthetic GPU-bound load [Estimate] | Gate 4 | The extension; then a setup finding |
| 15 | AMD Software keeps a per-app profile for `kairo-loadgen.exe` [Unverified] | The diff script with `--driver` | Global settings off |
| 16 | A TDR or device loss mid-bench | loadgen's exit 4 and events 4101 and 41 | P0: stop and report (M1:214) |
| 17 | An orphaned `KairoBench` session after a kill: providers on, 16–64 MiB of buffers, and a small cost on your next game [Unverified size] | devrun's `etw_sessions` (PR 5); the inbox `logman` cleanup in `pc bench`; the next setup; `pc doctor`'s listing | A reboot clears it |
| 18 | ETW loss or ring overflow at loadgen's frame rate [Unverified] | §4.7 rule 2 | `--set_circular_buffer_size 4096` for that workload, A and B alike |
| 19 | `MsGPUBusy` on the RX 9000-series at your HAGS state; HAGS adds about 0.5 ms (PMREADME:75–84) [Unverified] | The first run's `gpu_busy_ms` > 0; HAGS from loadgen's read-out and `env.json` | GPU columns reported as unavailable; the FPS clauses don't need them |
| 20 | What your account sees in PDH (CB:1212 item 6) [Unverified] | §5.3's one-off comparison | Registry hints for Instant Replay detection until it is decided |
| 21 | English counter names on a localized Windows: an explicit `-Counter` path isn't translated (PS/GetCounterCommand.cs:413–441) [Unverified for this PC]. Settled: Get-Counter's first returned sample is valid, since it discards its own priming read (:488–511) | The first run (exit 3 if the name fails) | Map the name through the Perflib index, or the Rust sampler with `PdhAddEnglishCounterW` (CB:1077) |
| 22 | Settled: `ProcessName` from `GetProcesses()` opens no process in dotnet/runtime v10.0.0 (NET/Process.Windows.cs:892–904, NET/ProcessManager.Windows.cs:63–67), which PowerShell 7.6.7 runs on [M: the shipped servicing release] | — | — |
| 23 | AMD's toggles live in the registry [Unverified, R13:472] | Q2's session | `--driver`; otherwise SP9 notes it |
| 24 | The cover hides the OLED's taskbar, and doesn't change loadgen's present mode [Unverified] | The rehearsal pair with and without the cover | If the taskbar shows over the cover, no unattended run with the OLED on: either you switch the OLED off at its button for the run (a physical action Claude asks for, CL:42–43), or the run waits for a cover that hides the taskbar. Making the cover the foreground window would hide it, but would take the foreground from loadgen |
| 25 | `ES_DISPLAY_REQUIRED` wakes a dark display [Unverified; R16 risk 23] | devrun's safety-stop PR | Start on "go" with the displays on |
| 26 | Windows throttles the windowless PresentMon with EcoQoS [Unverified] | ETW-loss flags | Opt the child out with `SetProcessInformation` on Kairo's own child |
| 27 | An update during a session | The `env.json` comparison | The session doesn't count (M1:217) |
| 28 | CSV size at the bandwidth workload's frame rate [Estimate] | The rehearsal | Statistics only, by default |
| 29 | The QPC frequency (10 MHz?), to line up `CPUStartQPC` with Kairo's 100 ns traces in M2 | Loadgen's `qpc_hz` | Convert with the logged frequency |
| 30 | Adrenalin runs its own `PresentMon*.exe` whenever AMD Software is up [Unverified; R13:374, R13:540] | `foreign_presentmon` in `bench.json`; `amd_processes` in `env.json` | Its own session name keeps it apart, so only Kairo's own file refuses (§4.3) |
| 31 | The black, topmost cover looks like a hidden cheat overlay to an anti-cheat that starts beside it [Unverified] | — (no game should run during a bench) | Q4: no capture exclusion for the bench's cover; the foreground stop ends the session, and the cover with it, when a game comes to the front |
| 32 | A screen saver, an inactivity lock or Dynamic Lock stops or spoils a long run; `SetThreadExecutionState` "does not stop the screen saver from executing" [MS-STES] | `bench-pre` reads the settings (§4.6, §5.4) | You switch them off for the evening (Q2) |
| 33 | Adrenalin 26.10.1's Metrics Overlay crashes or times out when other overlays open and close (R13:257); whether loadgen's windows or the covers count as such is [Unverified] | `EnableMetricsOverlay` in `bench-pre`, once Q2 confirms its name; the 4101 and 41 checks | Refuse with the overlay on; end the session when a `PresentMon*` appears (§4.3) |
| 34 | The bandwidth workload stays latency-bound, not bandwidth-bound [Unverified] | `calibrate`'s GB/s against the card's peak | More loads in flight per pass, or the `ByteAddressBuffer` of §3.5 |
| 35 | `LogonUI.exe` runs exactly while the session is locked or at sign-in [Unverified; M1:166] | M1:166's checks | Rule 8 drops runs a locked screen spoils anyway |
| 36 | A game the bench can't see: already open, never brought to the front, and played with a controller | — | Q1 asks you to start no game; the gap rule (§4.3) keeps loadgen's activation from taking the foreground back from one you switch to |

---

## Sources

- **PresentMon v2.6.0** (the tag resolves to e13fce6). Read: README-ConsoleApplication.md, README.md, LICENSE.txt, Version.props; `PresentMon/{CommandLine, Console, ConsumerThread, CsvOutput, LogSetup, MainThread, OutputThread, Privilege}.cpp`; `PresentData/{GpuTrace, PresentMonTraceConsumer, PresentMonTraceSession}.cpp` and headers; `IntelPresentMon/CommonUtilities/mc/*`.
  - https://github.com/GameTechDev/PresentMon/tree/v2.6.0
  - [GH] https://api.github.com/repos/GameTechDev/PresentMon/releases/tags/v2.6.0 and https://api.github.com/repos/GameTechDev/PresentMon/git/ref/tags/v2.6.0
  - The v1.9.2 README, for the 1.x option names: https://github.com/GameTechDev/PresentMon/blob/v1.9.2/README.md
- **Microsoft Learn**, read 2026-11-14:
  - [MS-VRR] https://learn.microsoft.com/en-us/windows/win32/direct3ddxgi/variable-refresh-rate-displays
  - [MS-PRESENTFLAGS] https://learn.microsoft.com/en-us/windows/win32/direct3ddxgi/dxgi-present
  - [MS-FLIP] https://learn.microsoft.com/en-us/windows/win32/direct3ddxgi/for-best-performance--use-dxgi-flip-model
  - [MS-PRESENT] https://learn.microsoft.com/en-us/windows/win32/api/dxgi/nf-dxgi-idxgiswapchain-present
  - [MS-SCFLAG] https://learn.microsoft.com/en-us/windows/win32/api/dxgi/ne-dxgi-dxgi_swap_chain_flag
  - [MS-LAT2] https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_3/nf-dxgi1_3-idxgiswapchain2-setmaximumframelatency
  - [MS-LAT1] https://learn.microsoft.com/en-us/windows/win32/api/dxgi/nf-dxgi-idxgidevice1-setmaximumframelatency
  - [MS-MEDIA] https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_3/nf-dxgi1_3-idxgiswapchainmedia-getframestatisticsmedia
  - [MS-HWCOMP] https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_6/ne-dxgi1_6-dxgi_hardware_composition_support_flags
  - [MS-ERR] https://learn.microsoft.com/en-us/windows/win32/direct3ddxgi/dxgi-error
  - [MS-QUERY] https://learn.microsoft.com/en-us/windows/win32/api/d3d11/ne-d3d11-d3d11_query
  - [MS-DISJOINT] https://learn.microsoft.com/en-us/windows/win32/api/d3d11/ns-d3d11-d3d11_query_data_timestamp_disjoint
  - [MS-GETDATA] https://learn.microsoft.com/en-us/windows/win32/api/d3d11/nf-d3d11-id3d11devicecontext-getdata
  - [MS-TDR] https://learn.microsoft.com/en-us/windows-hardware/drivers/display/timeout-detection-and-recovery
  - [MS-DEVFLAG] https://learn.microsoft.com/en-us/windows/win32/api/d3d11/ne-d3d11-d3d11_create_device_flag
  - [MS-PREEMPT] https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/d3dkmdt/ne-d3dkmdt-_d3dkmdt_graphics_preemption_granularity
  - [MS-COMPILE] https://learn.microsoft.com/en-us/windows/win32/api/d3dcompiler/nf-d3dcompiler-d3dcompile
  - [MS-TARGETS] https://learn.microsoft.com/en-us/windows/win32/direct3dhlsl/specifying-compiler-targets
  - [MS-LIMITS] https://learn.microsoft.com/en-us/windows/win32/direct3d11/overviews-direct3d-11-resources-limits
  - [MS-XAG118] https://learn.microsoft.com/en-us/gaming/accessibility/xbox-accessibility-guidelines/118
  - [MS-StartTrace] https://learn.microsoft.com/en-us/windows/win32/api/evntrace/nf-evntrace-starttracew
  - [MS-fopen] https://learn.microsoft.com/en-us/cpp/c-runtime-library/reference/fopen-wfopen
  - [PS-Measure] https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.utility/measure-object
- **Microsoft Learn**, read 2026-11-22 for the critique (MS-FLIP re-read the same day):
  - [MS-STES] https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-setthreadexecutionstate
  - [MS-SYSMET] https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getsystemmetrics
- **Third-party, for Dynamic Lock's value `EnableGoodbye` only** [M]:
  - https://winaero.com/enable-dynamic-lock-windows-10
  - https://www.winhelponline.com/blog/windows-goodbye-dynamic-lock-feature-windows-10/
  - https://www.elevenforum.com/t/turn-on-or-off-dynamic-lock-to-automatically-lock-windows-11.1940/post-42721
- **Other Microsoft-authored pages**:
  - [WALBOURN] https://walbourn.github.io/hlsl-fxc-and-d3dcompile
  - [DXC] https://github.com/microsoft/DirectXShaderCompiler
- **Local**:
  - `windows` 0.62.2, `windows-core` 0.62.2 (`src/param.rs`) and `windows-link` 0.2.1, from the Mac's cargo registry;
  - the testbed and harness branches, and the repo files in the citation keys;
  - dotnet/runtime v10.0.0's `Process.cs`, `Process.Windows.cs`, `ProcessManager.Windows.cs` and `ProcessManager.Win32.cs`, and PowerShell v7.6.7's `TypeTable_Types_Ps1Xml.cs`, `GetCounterCommand.cs`, `PdhHelper.cs` and `global.json` (MIT), each matched to its tag's git blob on 2026-11-22 [Live];
  - PresentMon v2.6.0's files again, fetched by the critique and pinned to e13fce6.

---

## Changes after critique

Each issue was checked against its source before anything changed: the repo at ddec2c9 (and at 0c6018b where #31's merge matters), the harness branch, PresentMon v2.6.0 as the critique fetched it again (e13fce6), PowerShell v7.6.7's and dotnet/runtime v10.0.0's sources (each matched to its tag's git blob), `windows` and `windows-core` 0.62.2 from the Mac's cargo registry, and three Microsoft pages read on 2026-11-22. F = feasibility, C = completeness, R = rules. One issue was rejected (F12). A few were fixed in another form than the one proposed, as their rows say; issues two critiques share are fixed once.

| Issue | Held? What changed |
|---|---|
| **F1, C1** (high) A run re-run "in its slot" needs rules the job can't see; `max_seconds` 2,400 leaves no room; no path for a workload below 6 valid runs; rule 6 undefined across jobs | Held: the limits sum to 2,320 s, plus 60 s of PresentMon stops, against 2,400. Fixed in another form. The job re-runs only failures it sees itself (PresentMon's exit and recording lines, ETW loss, a missing or cut CSV, loadgen gone, a ready time-out), at most 3 per workload. Rules 3–8 are judged at one decision point inside the job, which then runs make-up runs or the extension in the same job, so there is no second job. "Insufficient data" is `pc bench aa` exit 8, and rule 6 spans the session. `max_seconds` is 5,830 s from a listed sum, with a phase-sum test like `the_limits_fit_the_job` (§2 items 15–17, §4.3, §4.4, §4.6 step 4, §4.7, §4.9, §6) |
| **F2** (high) One `Get-Counter` call misses programs that start later, dies when a sampled program exits, and the design dropped a valid first sample | Held: PS/GetCounterCommand.cs:444, :478, :488–524, :611–625; PS/PdhHelper.cs:1234–1297; lib:22. Fixed: one `-MaxSamples 1` call per second with `-ErrorAction SilentlyContinue`, invalid samples dropped and counted, nothing else dropped; the Rust sampler named for M2; §5.3's sentence corrected; risk 21's first half settled (§5.3, §9) |
| **F3** (high) A screen saver or Dynamic Lock can stop or spoil the long run | Held: "This function does not stop the screen saver from executing" [MS-STES]; M1:35 asks for both off on away days, and neither Q1, Q2 nor `bench-pre` covered them. Fixed: Q2 asks you to switch them off for the evening; `bench-pre` reads them, read-only, from your hive and refuses when one could fire (Dynamic Lock for the full A/A); `env.json` records them (§1, §4.6 step 2, §5.4, §8, risk 32) |
| **F4** (high) A stall of 4 s or more isn't pruned, so rule 4 can't catch it | Held: OutputThread.cpp:260–264, :333–365, :434, :555, UnifiedSwapChain.cpp:17–21 and PresentMonTraceConsumer.cpp:3609–3612 leave a single chain unpruned. Fixed: `max_frame_ms` and `long_frames` in pm-stats, rule 7 (no frame over 250 ms), both texts corrected, vector P12 (§4.7, §5.1, §6) |
| **F5** Dot-sourcing standalone scripts runs their top level: `Mandatory` parameters, `exit`, StrictMode | Held (lib:21–22, :77–81); run-doctor uses a file and `&` (tools/mac/remote/run-doctor.ps1:10–19, :32–40). Fixed: functions only return or throw, a guarded command-line body, no `Mandatory`, both paths tested (§2 item 1, §5, §6) |
| **F6** PresentMon's exit 7 means "none found" or "couldn't stop" | Held (MainThread.cpp:244–253, :355–383). Fixed: stderr decides, and "failed to terminate" refuses with exit 3 and a pointer to `pc bench cleanup` (§4.3 step 8, §4.5, §4.9) |
| **F7** Two statements stronger than [MS-FLIP] | Held: re-read on 2026-11-22, the page allows reverse composition or MPO under other content, and says only that the first GPU write stalls. Both relabelled, with the quotes (§2 item 7, §3.3, §3.4, risk 3) |
| **F8** `--driver` picks "the" AMD class key; there are several | Held (M1:66; removed devices' keys stay [M]). Fixed: every AMD class key, labelled by `MatchingDeviceId`, and `<denied>` per unreadable subkey; `env.json` matches by `MatchingDeviceId` too (§5.4, §5.5) |
| **F9** The regex scan trips on `[IO.Path]::` | Held (lib:103, stub.ps1:25 and others). Fixed: a scan of the PowerShell AST (§6) |
| **F10** The fetched CSV's hash can't match the file | Held: `Read-SharedText` drops the BOM (lib:156). Fixed: the bytes are read once through `Open-ReadFile`, then hashed, gzipped and decoded from the same buffer (§2 item 16, §4.6 step 8) |
| **F11** "The memory system, not latency, is the limit" is unlabelled | Held. Fixed: 4 independent loads per pass (`--reads` a multiple of 4), the claim [Unverified], `calibrate` logs GB/s, risk 34 (§3.2, §3.5, §3.6, §3.10, §6) |
| **F12** `CreateSwapChainForHwnd(…, None, None)` won't compile | Rejected. The testbed on `main` makes exactly that call (tb/gpu.rs:151) and builds on Windows CI, and windows-core 0.62.2 has a single `Param` impl for `Option` (param.rs:16), so the type is inferred. The sketch stays |
| **C2** (high) A dark display, a locked session and Remote Desktop go undetected; §4.7 and §4.9 disagree | Held (M1:96, core/watch.rs:18–31). Fixed: setup refuses Remote Desktop ([MS-SYSMET]) and a lock (`LogonUI.exe`, [Unverified]); the watcher stops on a lock; rule 8 makes `dropped_pct` above 1% invalid, so §4.9 agrees (§2 item 25, §4.3, §4.7, §4.9, risk 35) |
| **C3, R4** Verbs that skip the busy check; work that runs before it or right after a stop | Held (pc:7; hn/pc:9–10, :1058, :1066, :1091; hn/remote/harness-fetch.ps1:13). Fixed: `bench-pre`, `bench env`, `bench fetch` and `amd-diff` call `Assert-PcIdle` first (`amd-diff` refuses a fullscreen window and takes your session's `--force`); `bench cleanup` keeps a written exemption; after a stop only the TDR check and the cleanup run; PR 9c samples after `run.ps1`'s check (§4.6, §5.3, §5.5) |
| **C4, R1** (high) No game-start stop for a long run while §1 and Q1 promised one; the controller caveat lost; the exception not recorded | Held (CL:25, M1:96, M1:103, core/watch.rs:25–30). Fixed: a foreground stop that opens no process (`GetForegroundWindow`, `GetWindowThreadProcessId`, PIDs from `EnumWindows`), with a gap rule; a fullscreen sample without a name counts as foreign; an attended stop test (`selftest`) before Q1's run; §1 and Q1 say plainly what stops the run and what doesn't, ask you to start no game, and give the real lengths; the exception goes into M1:103 in PR 5 (§1, §2 items 24–25, §4.3, §6, §7, §8, risk 36) |
| **C5** M2's hooks contradict M1 and PLAN | Held. Fixed as open M2 items with their constraints: in SP1, B is the FFmpeg CLI (PLAN:427), so a media PR (CL:34); one ABBA per job (~8.6 min) for SP1's OLED limit; game benchmarks need `abort_on_input` off, a target exemption and a PID found later; HDR; the other workloads; later reuse; rules 7–8 per arm; R13:838's deltas (§4.10, §4.12) |
| **C6** The `--output_stdout` fallback fails rule 1 on every run | Held (MainThread.cpp:110, :145; CommandLine.cpp:556–557). Fixed: its own rule 1, and a `route` per run (§2 item 12, §4.7, §4.8) |
| **C7** The Mac's `StopWhen` can kill the job before its own watcher, leaving no result | Held (lib:600–604, :633–637; hn/remote/harness-fetch.ps1:17–18). Fixed: the `StopWhen` waits 10 s and ignores `--force`; `bench-fetch` checks freshness from `bench.json` without a result file (§4.6 steps 3 and 8) |
| **C8** The banned `$p.Path` sketch is still where the cookbook points | Held (CB:1078, R13:631, M1:86). Fixed: PR 1 adds correction notes at R13 §5.3 and CB §9.6, and the module-list reason to M1:86 (§7) |
| **C9** Docs, progress and contract updates missing from the PR plan | Held. Fixed: PR 0 (this design, CHANGELOG, PLAN:300 and :740, M1's pointer, PLAN:353); each PR's contract lines (M1:62, :72, :81, :86, :92, :103; `.dogfood/amd-diff/` moved to PR 2); the progress ticks mapped (M1:127, :131, :132, :140, :142, :194, :201; PLAN:663) (§7) |
| **C10** pm-stats lacks R13's P99s, the display cadence and the tearing columns | Held (R13:567–568). Fixed: `gpu_*_p99_ms`, `display_change_ms` and its P99, count objects for `SyncInterval`, `AllowsTearing` and `PresentFlags`, a flag on them, vector P13, P1 recomputed (§4.7, §5.1, §6) |
| **C11** Holes in the test plan | Held. Fixed: P7–P9 defined, plus P12–P13, S1–S2, V1–V8 and D1–D5; `env.json` runs in CI with a privacy assertion; the decision and the before/after comparison moved from the Mac's bash into `bench-rules`, which CI tests; registry- and file-write scans; PR 3's and PR 4's PC needs stated (§2 item 16, §6, §7) |
| **C12** Stale calibration, displays and LUIDs between sessions | Held. Fixed: a calibration `fingerprint`, checked against the first ready file (exit 3, "calibrate again"); `bench.json` records the displays at setup and at the end, for the before/after check and the LUID mapping (§3.6, §3.9, §4.4, §4.8, §5.3, §5.4) |
| **C13** Adrenalin's own PresentMon could block every session | Held (R13:540; whether it always runs is [Unverified]). Fixed: only Kairo's own file refuses, others are recorded, and one appearing mid-session stops it (R6); risk 30 (§4.3) |
| **C14** `env.json` leaves out listed items | Held. Fixed: HAGS's effective state from loadgen's D3DKMT read-out (`KMTQAITYPE_WDDM_2_7_CAPS`, win/Wdk/Graphics/Direct3D/mod.rs:13299; logging only, R13:869); TDR history; the scripts' commit; a flag for other video-engine users; reserved `game` and `kairo_engine` (§3.10, §4.7, §5.4) |
| **C15** Q3's list and options incomplete | Held (R13:823–828). Fixed: the full list, "off for the evening and back on after" for each, optional diff-script records, and what keeping Chill or FRTC on costs (§1, §8) |
| **C16** `--detail` undefined, missing exit codes, two house rules not carried | Held. Fixed: `--detail` defined as a compile-time macro; every verb's exits; "no unwrap" and "no per-frame allocation" in §2 item 4 (§2, §3.2, §3.7, §4.9) |
| **C17** Gate 4's other half and the scope unstated | Held. Fixed: a "Not in this design" line, and the evidence line sits beside `audio-soak`'s (§1) |
| **C18** The clock edge isn't routed to the day log | Held (M1:115, R16:922). Fixed: day 1 of the day log, and PR 3 before the first `wgc-probe` commit [proposed] (§3.12, §7) |
| **R2** (high) `--force` has no stated limits, and the extension inherits it | Held (tools/mac/remote/build.ps1:149, :161; pc-tooling.md:263; hn/pc:1064, :1094, :1152). Fixed: for `pc bench`, `--force` only lets a job start despite recent input; `abort_on_input` is always true; the job's stops ignore the flag; the decision point re-checks without it, and no second job exists any more; a stop spends your "go" (§2 item 23, §4.6, §4.11) |
| **R3** OLED gaps: a stale alias, risk 24's fallback, uncovered attended plans, the gap between jobs | Held (M1:77, CL:26, PLAN:334). Fixed: covers on every other active display, found with `kairo-display`, checked against their monitors, and required for the full A/A; on every plan but the one no-cover rehearsal; risk 24's fallback rewritten; `--not-monitor` guards the PC side; the in-job decision point removes the gap between jobs (§4.3, §4.11, risk 24) |
| **R5** The AMD diff prints key and value names raw | Held in part: the cited Windows keys are outside the diff's scope, but AMD's per-app layout is [Unverified] and CL:16 holds. Fixed: names pass the same plain-text test, else `<name len=N sha256=8hex>`, with test vectors (§5.5, §6) |
| **R6** The 26.10.1 Metrics Overlay time-outs are only flagged | Held (R13:257). Fixed: `bench-pre` refuses with the overlay on once its value is confirmed; a `PresentMon*` appearing stops the session; Q2's overlay steps; a 4101/41 check after the AMD session; risk 33 (§4.3, §4.6, §5.5, §8) |
| **R7** Instant Replay can stay off after an interrupted evening | Held (M1:42). Fixed: every way out shows its state, the closing message asks, and the "on" toggle comes first after a stop (§4.6 step 10, §5.5, §8) |
| **R8** A wrong PresentMon hash is re-installed silently | Held (M0:61, CL:42–43). Fixed: a missing file is re-installed; a wrong hash stops bench work and is reported to you first (§4.5) |
| **R9** The elevated cleanup runs PresentMon as administrator | Held (M1:171–172). Fixed: the inbox `logman stop KairoBench -ets`, only when no job or Kairo PresentMon runs (§4.5) |
| **R10** Q1's run comes before the robust orphan fix | Held. Fixed: `etw_sessions` moved into PR 5, with a name pattern that can't reach another tool's session (§4.3, §4.5, §7) |
| **R11** The cover looks like a cheat overlay | Held as a risk [Unverified]. Fixed: risk 31; the cover ends with the foreground stop; Q4 asks whether the bench's cover needs capture exclusion (M1:99, PLAN:334), recommending not |
| **R12** `env.json`'s sources hold your name and PC identifiers | Held [M]. Fixed: named values only, projected as boot:657–672 does; the power plan as a GUID and a known label; the task's principal turned into a SID in memory only; the CI assertion (§5.4, §6) |
| **R13** The scan misses indirect routes to `MainModule` and to writes | Held (PS/TypeTable_Types_Ps1Xml.cs:1136–1244, and `CommandLine` at :1148–1165). Fixed: one process helper and a scan against any other; whole-object output of a Process banned; registry- and file-write scans; the temp-key test guarded (§5.3, §6) |
| **R14** No home for the shared crate's `unsafe` | Held (CL:32, tb/lib.rs:15–28). Fixed: the testbed's shape, with the cover in `platform`; §2 item 4 corrected (§2, §3.1) |
| **R15** "Sign out and in" listed as a recovery step | Held (CL:42). Fixed: the fallback first; signing out only as a question with its cost, never done remotely (§4.9, §8, risk 1) |
| **Settled by the checks** | Risk 22: `ProcessName` opens no process in .NET v10.0.0. `.Path` goes through `MainModule` in PowerShell v7.6.7 (:1142), which opens `PROCESS_QUERY_INFORMATION \| PROCESS_VM_READ` and lists modules (NET/ProcessManager.Win32.cs:67–81). `Get-Counter` lower-cases instance names (PS/PdhHelper.cs:1160). Risk 21's first half. The NET and PS keys now carry tags |
| **Found in this review** | tools/pc/uninstall-dev.ps1:115–118 reads `.Path` of every process, a module list with memory-read rights on any game running at the time (CL:9); PR 1 fixes it (§7). `main` moved to 0c6018b with #31's merge and the harness branch to 7c994e8; the header records what that changes, and §2 item 2 and §7 no longer wait for #31 |
