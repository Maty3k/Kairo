# Research 17 note: PresentMon 2.6.0, pm-stats, ab-stats and the A/A rule (gate 4's bench half)

*2026-11-14 · Written against `main` at ddec2c9, read-only; nothing in the repo or on the PC was changed or run. One of research 17's notes: it covers PresentMon 2.6.0 (options, CSV columns, privileges, sessions, the pin) and the statistics (`pm-stats.ps1`, `ab-stats.ps1`, the A/A CV rule). Loadgen, `pc bench`'s orchestration, `encoder-usage.ps1`, `env.json` and the AMD diff script are other notes' parts; where PresentMon constrains them, this note says so.*

**Citation keys.**

| Key | Source |
|---|---|
| `PM:path:n` | PresentMon source at tag `v2.6.0` = commit `e13fce6acdb55a808fd8318175a56863e532d95f`, files in `research17/pm-src/` (paths as in the repo) |
| `PMR:n` | `README-ConsoleApplication.md` at that tag; `PMREADME:n` its `README.md` |
| `PM1` | PresentMon `v1.9.2` `README.md` (raw.githubusercontent.com, read 2026-11-14) |
| `GH` | GitHub API `repos/GameTechDev/PresentMon/releases/tags/v2.6.0` and `git/ref/tags/v2.6.0`, fetched live 2026-11-14 |
| `MS-fopen`, `MS-StartTrace`, `MS-Tdh`, `PS-Measure` | Microsoft Learn: `fopen, _wfopen` (CRT); `StartTraceW`; `TdhLoadManifestFromBinary`; `Measure-Object` (PowerShell 7.6), read 2026-11-14 |
| `CL:n`, `PLAN:n`, `M0:n`, `M1:n`, `CB:n`, `R13:n`, `R16:n` | CLAUDE.md, docs/PLAN.md, docs/milestones/M0.md and M1.md, docs/research/15-implementation-cookbook.md, 13-amd-performance.md, 16-verify-testbed/README.md |
| `boot:n`, `doc:n` | tools/pc/bootstrap.ps1, tools/pc/doctor.ps1 |
| `devrun/x.rs:n`, `ci:n` | crates/kairo-devrun/src/x.rs, .github/workflows/ci.yml |
| `lib:n` | tools/mac/remote/lib.ps1 on the harness branch (PRs #34–35, not merged; `scratchpad/kairo-harness`) |
| `ready.rs:n` | crates/kairo-testbed/src/ready.rs on the testbed branch (PR #31, not merged; `scratchpad/kairo-testbed`) |
| `dl/` | the earlier run's downloads in `research17/dl/` (.NET `Process*.cs`, PowerShell `TypeTable_Types_Ps1Xml.cs`; tag unknown) |
| `ref` | this note's Python reference, `scratchpad/research17-scripts/statref.py` and `vectors.py` (Mac scratch, not in the repo) |

**Evidence labels.**
- **[Code]**: read in the PresentMon (or Kairo) source at the cited line.
- **[Doc]**: Microsoft Learn or PresentMon's own README.
- **[Live]**: checked live today (GitHub API, raw files).
- **[Mac]**: computed on the Mac with `ref` (IEEE doubles, the arithmetic .NET uses).
- **[Unverified]**: not yet checked on the PC; §12 says how each one gets settled.
- **[proposed]**: Claude's choice, to confirm in review or with PC data.
- **[Estimate]**: a planning number, not a measurement.
- **[Inference]**: reasoned from code or docs, not observed.

---

## 1. Findings in brief

1. **The downloads are v2.6.0.** `pm-src/tagref.json` names `refs/tags/v2.6.0` → commit `e13fce6a`; all 36 downloaded files hash (git blob SHA-1) to the entries of that commit's `tree.json`, and 5 of them re-fetched live at `e13fce6a` match byte for byte [Live]. v2.6.0 (2026-09-21) is still the latest release [Live, GH].
2. **The pin already exists and is right:** `PresentMon-2.6.0-x64.exe`, 980,320 bytes, SHA-256 `b2a706bc6ad475749e3b7e3409263aa1e6906d45bdcf993f6dbc0f660188f1af` in boot:169–174 and doc:75–77, equal to GitHub's asset digest today [Live]. MIT licence; a dev tool that is never shipped or committed.
3. **Command line** (§3.1): research 13's flags are all valid in 2.6.0 [Code], with two changes: `--process_id <pid>` instead of `--process_name` for loadgen (an early PID filter, no name ambiguity, works even when the name can't be resolved), and the contract's paths (M1:74–75). Recommended addition: `--no_track_input` [proposed].
4. **`--qpc_time` renames two columns:** `TimeInMs` → `TimeInQPC` and `CPUStartTimeInMs` → `CPUStartQPC`, both raw QPC ticks of the same clock as Kairo's `QueryPerformanceCounter` (PM:PresentMon/CsvOutput.cpp:558, :620; PM:PresentData/PresentMonTraceSession.hpp:39). Research 13 (R13:557) lists only the second.
5. **The CSV pc bench gets has 30 columns in a fixed order** (§5.2). Things the statistics must handle that research 13's sketch doesn't [Code]: a present shown several times (frame generation) gets **one row per display instance**, all with the same `TimeInQPC` and `MsBetweenPresents`, and GPU/CPU numbers only on the application row; dropped presents keep their row with `NA` display columns; lost presents and ≥4 s stalls leave no row; nothing is quoted; the file is UTF-8 with a BOM and CRLF; it is created only when the first row is recorded and **truncates** a file of the same name.
6. **`MsGPUTime = MsGPUBusy + MsGPUWait`** (PM:PresentMon/CsvOutput.cpp:1260–1263): `MsGPUBusy` is the time at least one of the target's non-video GPU packets ran during the frame, `MsGPUWait` the idle gaps between its first packet start and last packet end; with `--track_gpu_video` video-engine work moves out of `MsGPUBusy` into `MsVideoBusy` (PM:PresentData/GpuTrace.cpp:205–227).
7. **Privileges:** admin **or** Performance Log Users, nothing else [Code, Doc]. Elevation adds only SeDebugPrivilege (names of other accounts' processes) and process start/stop events; neither is needed with `--process_id` on the owner's own loadgen.
8. **A contract conflict:** M1:97 lets PresentMon run from SSH, but an elevated PresentMon with `--output_file` in `C:\kairo\logs\` creates (and truncates, following any link) a file there, which M1:81 forbids. Recommendation [proposed]: PresentMon runs **as the owner, inside the bench job** (Performance Log Users, as M0's doctor test does), writes its CSV in `logs\`, and the job's end always stops the `KairoBench` ETW session (§4).
9. **Never pass** `--scroll_indicator` (it calls `SendInput`, banned by CL:10), `--restart_as_admin` (a UAC prompt), `--hotkey`, `--exclude_dropped` (breaks Σ `MsBetweenPresents`), `--multi_csv`, `--date_time`, `--v1_metrics`/`--v2_metrics` (§3.3).
10. **PresentMon exits 0 when it recorded nothing** (no CSV) and when it lost ETW events (warnings on stderr only). pc bench must check the CSV, stdout and stderr, not only the exit code (§3.5, §8.4).
11. **pm-stats** (§6): dedupe rows to presents, pick the main swap chain deterministically, nearest-rank percentiles, `present_modes` and `frame_types` as count objects (research 13's `{1:P1}` strings are culture-dependent), plus 6 diagnostic fields [proposed]. Research 13's float nearest-rank index is safe: it equals the integer formula for every n ≤ 2,000,000 [Mac].
12. **ab-stats** (§7): research 13's t table lacks n = 11, 13–15 and ≥17, so `$t975[$n]` is `$null`, the CI collapses to the mean and PASS rests on the mean alone (vector AB2); it also formats with the current culture. Fixed with a df 1–30 table plus a series accurate to 3×10⁻⁸ beyond. The plan's other clauses are specified for SP1 (§7.5).
13. **A/A rule** (§8): CV = sample SD (n − 1) ÷ mean of per-run `avg_fps`, 6 valid runs per workload after the discarded warm-up; CV > 1% → 6 more (n = 12), no trimming. At CV 1%, 6 ABBA pairs give a 95% half-width of about 1.48 pp, so a true loss above ≈2.5% fails the "upper ≤ 4%" clause [Estimate].
14. **No owner question** comes from this part (§11).

---

## 2. Provenance, pin and licence

### 2.1 Which files were read

- `pm-src/tagref.json`: `"ref": "refs/tags/v2.6.0"`, object `e13fce6acdb55a808fd8318175a56863e532d95f`, type `commit` (a lightweight tag). `pm-src/Version.props`: `PresentMonVersion` 2.6.0, `PresentMonFileVersion` 2.6.0.0.
- Each downloaded file's `git hash-object` equals its path's blob SHA in `pm-src/tree.json` (36 of 36). `PresentMon/CsvOutput.cpp`, `CommandLine.cpp`, `MainThread.cpp`, `OutputThread.cpp` and `IntelPresentMon/CommonUtilities/mc/MetricsCalculator.cpp` re-fetched from `raw.githubusercontent.com/GameTechDev/PresentMon/e13fce6a…/` hash the same [Live].
- Coverage of the side-effect scan (§3.6): all 8 `.cpp` files the console project compiles (`PresentMon.vcxproj` `ClCompile` items, fetched live) and all 8 `PresentData/*.cpp` (5 fetched live today for the scan); of `IntelPresentMon/CommonUtilities`, only `mc/`. The console's log channel is null (PM:PresentMon/LogSetup.cpp:4–7), so `pmlog_*` calls in the library write nothing.
- `research17/dl/` (from the earlier run) holds .NET and PowerShell sources of unknown tag; §4.5 uses two lines of them, labelled.

### 2.2 The release and the pin

| Asset (GH, v2.6.0, published 2026-09-21T17:30:36Z, target `release/2.6.0`) | Bytes | GitHub `digest` |
|---|---|---|
| `PresentMon-2.6.0-x64.exe` (the console application) | 980,320 | `sha256:b2a706bc6ad475749e3b7e3409263aa1e6906d45bdcf993f6dbc0f660188f1af` |
| `PresentMon-2.6.0.msi` (service + capture app; **not used**: it installs a service, CL:10) | 169,672,704 | `sha256:0cb43d26…f6c820` |
| `ReleaseSymbols.zip` | 54,167,987 | `sha256:93e9a6ff…0ccc` |

- Already pinned: boot:169–174 (`$PresentMonUrl`, `$PresentMonSha256`, `$PresentMonDir = 'C:\kairo\tools\presentmon'`); boot:1447–1459 (`Install-PresentMon`: download to `.download`, SHA-256 check, delete on mismatch, move); doc:75–77 and doc:674–693 (every doctor run re-hashes: `sha256_ok`, else "PresentMon does not match its pinned SHA-256"). M1:74 names the path `C:\kairo\tools\presentmon\PresentMon-2.6.0-x64.exe`.
- [proposed] `pc bench` re-hashes the exe once per bench session (1 MB, milliseconds) and refuses on a mismatch, so a tampered or replaced binary never records. The SHA-256 is PresentMon's identity in every run record: 2.6.0 prints its version only in its usage text (PM:PresentMon/CommandLine.cpp:255).
- A future re-pin follows boot:160–174's pattern: GitHub digest, a local hash, PLAN's tool table (PLAN:308) and doc:75–77 together.

### 2.3 Licence and use

- MIT, "Copyright (C) 2017-2024 Intel Corporation" (`pm-src/LICENSE.txt`; PMREADME:35–43). `THIRD_PARTY.txt` lists ADL (MIT), NVAPI (MIT) and CEF notices, which concern the capture app.
- Kairo only runs it on the dev PC: never linked, shipped, committed or mirrored (mirroring would be publishing; CL owner rules). No GPL code is involved (CL:17).

---

## 3. The command line

### 3.1 The invocation

```
C:\kairo\tools\presentmon\PresentMon-2.6.0-x64.exe
  --process_id <pid> --output_file C:\kairo\logs\<run-id>\<tag>.csv
  --session_name KairoBench --stop_existing_session
  --delay 5 --timed 60 --terminate_after_timed --no_console_stats
  --qpc_time --track_frame_type --track_gpu_video
  --no_track_input                                   (proposed)
```

- The run folder and file names follow M1:75 ("Claude lists the file names here as they land"); the folder must exist first, because PresentMon creates no folders and fails **silently** when it can't open the file (§3.5).
- One process per run; `--delay` and `--timed` take whole seconds, parsed with `wcstoul` (PM:PresentMon/CommandLine.cpp:244–250), so `--timed 0` or a non-number records nothing.
- **`<pid>` must be validated as a positive integer before the command is built** [proposed]: `wcstoul("abc")` is 0, and `--process_id 0` means no target at all, so PresentMon records **every** presenting process (PM:PresentMon/OutputThread.cpp:127–130, "--capture_all"), which would put the owner's other programs' exe names into the CSV.

### 3.2 What each option does in 2.6.0 [Code]

| Option | What the code does | Why pc bench wants it |
|---|---|---|
| `--process_id <pid>` | CommandLine.cpp:436; MainThread.cpp:322–325 sets `mFilteredProcessIds` and `AddTrackedProcessForFiltering(pid)`; PresentMonTraceConsumer.cpp:3609–3618 then passes only that PID and DWM's to the analysis; OutputThread.cpp:132–135 matches the PID before any name | Less PresentMon CPU, no clash between two `kairo-loadgen` processes, and a match even when the name reads `<unknown>` |
| `--process_name <exe>` (M2 games) | Repeatable; compared case-insensitively without folder or extension (CommandLine.cpp:618–631; OutputThread.cpp:96–108, 137–142); no early filter, so every process's presents are analysed | For games the owner starts (§4.5) |
| `--output_file <path>` | Folder, name and extension taken from the path (CsvOutput.cpp:111–116); suffixes only with `--multi_csv` or `--hotkey` (:125–136); opened on the first recorded row with `_wfopen_s(fp, path, L"w,ccs=UTF-8")` (:1421–1427) | One known file per run |
| `--session_name KairoBench` | Default `"PresentMon"` (CommandLine.cpp:384); names are case-insensitive and unique system-wide [Doc, MS-StartTrace] | Never collides with AMD's or anyone's session (R13:540, R13:863) |
| `--stop_existing_session` | On `ERROR_ALREADY_EXISTS`, prints a warning, stops the named session and starts again (MainThread.cpp:336–359); without it, exit 6 | Recovers a session a killed run left behind |
| `--delay 5` | The startup's simulated hotkey press arms a 5,000 ms timer (MainThread.cpp:401–405, 204–208); recording starts when it fires | Skips swap-chain seeding, the providers' start-up rundown and the move to Independent Flip |
| `--timed 60` | Sets `mStartTimer` (CommandLine.cpp:456); `StartRecording` arms a 60,000 ms timer (MainThread.cpp:120–123) that calls `StopRecording` (:183–188) | The run length |
| `--terminate_after_timed` | After the timed stop, `ExitMainThread()` (MainThread.cpp:185–187) → session stopped, CSV closed, exit 0. Without it PresentMon idles until Ctrl+C | A bounded process |
| `--no_console_stats` | `ConsoleOutput::Simple` (CommandLine.cpp:444): prints only `Started recording.` and `Stopped recording.` on stdout (MainThread.cpp:110–112, 145–147). Added anyway, with a warning, when stdout isn't a console (CommandLine.cpp:613–616) | Clean logs; the two lines prove the window ran |
| `--qpc_time` | `TimeUnit::QPC` (CommandLine.cpp:633); columns `TimeInQPC` and `CPUStartQPC` hold raw ticks (CsvOutput.cpp:558, 620, 1172–1173, 1231–1232). The session logs QPC timestamps (`TIMESTAMP_TYPE_QPC`, PresentMonTraceSession.hpp:39) read raw (`PROCESS_TRACE_MODE_RAW_TIMESTAMP`, PresentMonTraceSession.cpp:521–544). Only one of `--qpc_time`, `--qpc_time_ms`, `--date_time` (CommandLine.cpp:497–506, else exit 1) | Frames line up with Kairo's QPC traces in M2 (R13:551) |
| `--track_frame_type` | Enables the Intel-PresentMon provider's frame-type events and DxgKrnl `MMIOFlipMultiPlaneOverlay3_Info` (PresentMonTraceSession.cpp:862, 950–976); adds `FrameType` | Detects AFMF, which invalidates an A/B (R13:866) |
| `--track_gpu_video` | Enables DxgKrnl `NodeMetadata_Info` (PresentMonTraceSession.cpp:859); packets on nodes typed `VIDEO_DECODE`, `VIDEO_ENCODE` or `VIDEO_PROCESSING` go to a separate trace (GpuTrace.cpp:205–227); adds `MsVideoBusy`. Ignored with `--no_track_gpu` (CommandLine.cpp:565–569) | M1:74 and P17 (CB:58). The column is the **target's** video work, not Kairo's (R13:864) |
| `--no_track_input` [proposed] | Drops Win32k's input events `InputDeviceRead_Stop`, `RetrieveInputMessage_Info`, `OnInputXformUpdate_Info` (PresentMonTraceSession.cpp:889–892) and the two input-latency columns; the display events stay (:883–888) | Valid runs have no owner input (`abort_on_input`), so the columns are always `NA`. Fewer events, and PresentMon records nothing about the owner's keyboard and mouse. Decide before the first A/A run, so M1's and M2's runs use the same provider set; M1:74 then gains it |

### 3.3 Options pc bench never passes

| Option | Why |
|---|---|
| `--scroll_indicator` | Toggles Scroll Lock with **`SendInput`** at each start and stop (PM:PresentMon/MainThread.cpp:42–77, called at :113–115, 142–144, 397–399, 436–438). CL:10 bans `SendInput`; injected keys would also count as owner input and stop the job |
| `--restart_as_admin` | Re-launches through `ShellExecuteEx` with the `runas` verb (PM:PresentMon/Privilege.cpp:124–134): a UAC prompt, which needs the owner's OK (CLAUDE.md, Owner and git) |
| `--hotkey` | `RegisterHotKey` on the owner's desktop (MainThread.cpp:296); could take a key the owner or AMD uses; one CSV per press |
| `--exclude_dropped` | Skips dropped rows (CsvOutput.cpp:1410–1414) while the swap chain's history still advances, so the next row's `MsBetweenPresents` starts at the dropped present: Σ `MsBetweenPresents` no longer covers the window and `avg_fps` is wrong |
| `--multi_csv` | File names gain `-<process name>-<pid>` (CsvOutput.cpp:125–131) |
| `--date_time`, `--qpc_time_ms` | Local wall-clock time in every row (a PC detail) / exclusive with `--qpc_time` |
| `--v1_metrics`, `--v2_metrics` | Different column sets (§5.4) |
| `--output_stdout` | Only for an elevated run that must write no file (§4.3, option A) |
| `--write_display_metadata`; hidden `--write_frame_id`, `--write_display_time`, `--track_etw_status`, `--disable_offline_backpressure` (CommandLine.cpp:479–486) | Not needed; the hidden ones aren't in the README and may change |
| `--set_circular_buffer_size N` | Only if a workload reports overflowed presents; MainThread.cpp:455–459 suggests 4096. Then every run of that workload, A and B alike, uses it |

### 3.4 Names that changed since 1.x

- Prefixes: 2.6.0 documents `--name`, but `ParseArgPrefix` also accepts `-name` and `/name`, case-insensitively (PM:PresentMon/CommandLine.cpp:193–217). Research 13's and the cookbook's spellings are valid.

| 1.9.2 (PM1) | 2.6.0 |
|---|---|
| `-no_top` | `--no_console_stats` |
| `-terminate_existing` | `--terminate_existing_session` |
| `-qpc_time_s` (seconds) | `--qpc_time_ms` (milliseconds) |
| `-track_gpu`, `-track_input` (opt-in) | on by default; `--no_track_gpu`, `--no_track_input` |
| `-captureall` | removed: the default when no target is named |
| `-track_debug`, `-track_mixed_reality`, `-include_mixed_reality` | removed |
| unknown names shown as `<error>` | `<unknown>` (PMREADME:62–65) |
| column `QPCTime` | `TimeInQPC` and `CPUStartQPC` (§5.2) |

### 3.5 Exit codes, stdout and stderr [Code: PM:PresentMon/MainThread.cpp]

| Exit | When |
|---|---|
| 0 | Normal end (:474), **including** a run that recorded no row (no CSV), a CSV that couldn't be opened (CsvOutput.cpp:1425–1427 returns silently) and lost ETW events (warnings only, :448–462) |
| 1 | The NVIDIA manifest embedded in the exe failed to load (:229–231), or a bad command line (:238–240; usage printed) |
| 3 / 4 / 5 | Window class / message window / hotkey registration failed (:283–301) |
| 6 | Session name taken without `--stop_existing_session` (:342–353), or `StartTrace` failed, for example "access denied", followed by the Performance Log Users hint when the token lacks the group (:361–383) |
| 7 | `--terminate_existing_session` found no such session or couldn't stop it (:245–253); 0 when it stopped one |

- **stdout** (with `--no_console_stats` and a CSV): `Started recording.` and `Stopped recording.`
- **stderr** (`PrintWarning`/`PrintError` → `vfwprintf(stderr)`, PM:PresentMon/Console.cpp:340, 354–370) lines pc bench must classify:
  - expected when unelevated: `warning: PresentMon requires elevated privilege in order to query processes that are short-running or started on another account. …` (MainThread.cpp:272–276);
  - expected after a killed run: `warning: a trace session named "KairoBench" is already running and it will be stopped.` (:338–341);
  - **run invalid**: `warning: %lu ETW buffers were lost.`, `warning: %lu ETW events were lost.`, `warning: %lu overflowed present events detected. …` (:448–462);
  - errors: `error: …`.
- **Encoding.** When stdout or stderr is a **disk file**, PresentMon writes a UTF-16LE BOM (if the file is empty) and switches that stream to UTF-16 (`_O_U16TEXT`); a console also gets UTF-16; a **pipe** stays narrow text (Console.cpp:13–43). kairo-devrun gives a job's process disk files for stdout and stderr and a NUL stdin (devrun/run.rs:181–211), so if PresentMon is ever a job's direct command, its job logs are UTF-16LE. lib:140–157's `Read-SharedText` detects BOMs, so it reads either. A parent that spawns PresentMon should give it pipes [proposed].

### 3.6 What it writes and touches [Code, §2.1's coverage]

- **The CSV**, and nothing else on disk. Without `--output_file`, `PresentMon-<local time>.csv` in the current folder (CsvOutput.cpp:117–123). `w` mode empties an existing file of that name [Doc, MS-fopen: "If the given file exists, its contents are destroyed"]; it would also write through a link planted at that name [Inference: `CreateFile` follows links unless told not to].
- **A BOM** into a redirected, empty stdout or stderr file (Console.cpp:28–40).
- **No log file, no ETL file:** a null log channel (LogSetup.cpp:4–7); the session is `EVENT_TRACE_REAL_TIME_MODE`, "We have a realtime consumer, not writing to a log file" (PresentMonTraceSession.cpp:496).
- **A system-wide ETW session** named by `--session_name`: buffers of 64 KB, 256 to 1,024 of them, so 16–64 MB of kernel memory (PresentMonTraceSession.cpp:498–500). Stopped on a normal exit (`PMTraceSession::Stop`, :598–635). It **outlives a killed PresentMon** ("The session remains active until the session is stopped, the computer is restarted…" [Doc, MS-StartTrace]), with its providers still enabled (§4.4).
- **In-process only:** `TdhLoadManifestFromBinary` on its own exe (MainThread.cpp:24–40; Learn says only that "The GUIDs and BinaryPath string are cached" [Doc, MS-Tdh]); SeDebugPrivilege enabled in its own token when it can (Privilege.cpp:27–68); a message-only window (MainThread.cpp:280–293).
- **Other processes:** `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)` and `QueryFullProcessImageNameW` on each presenting process it hasn't seen start, the handle kept to poll `GetExitCodeProcess` (OutputThread.cpp:212–231, 294–319). No `PROCESS_VM_READ`, module lists, memory reads, hooks, injection, registry writes, sockets or child processes (except `--restart_as_admin`) in the scanned sources. `SendInput` and `RegisterHotKey` only behind §3.3's flags.

---

## 4. Privileges and where PresentMon runs

### 4.1 What the session needs

- `StartTrace` succeeds for "administrative privileges, users in the Performance Log Users group, and services running as LocalSystem, LocalService, NetworkService" [Doc, MS-StartTrace]. PresentMon checks the group (S-1-5-32-559, PM:PresentMon/Privilege.cpp:8–25) only to print its hint on "access denied" (MainThread.cpp:373–377); PMREADME:49–60 adds "Sign out and log back in for the changes to take effect".
- **Elevation adds two things:**
  - SeDebugPrivilege, "to open handles to processes started by other accounts" (MainThread.cpp:255–277). Without it: the start warning, and other accounts' or short-lived processes read `<unknown>` and can't be targeted by name (PMREADME:62–65);
  - the Kernel-Process provider's start/stop events: enabling it may fail with `ERROR_ACCESS_DENIED`, which PresentMon tolerates silently (PresentMonTraceSession.cpp:807–821). The console never asks for its rundown (`mTrackProcessState = false`, PresentMonTraceConsumer.hpp:341), so processes started before PresentMon are named through `OpenProcess` anyway.
- With `--process_id` on loadgen, which runs as the owner, neither matters: the PID matches first, and the owner's token can open the owner's process.
- **The owner's account is already set up:** the bootstrap added it to Performance Log Users "so PresentMon works unelevated" (M0:11); the doctor checks membership and the session token (doc:674–693); M0 gate 4 passed on 2026-10-23 with "PresentMon recorded 2 seconds with your limited token" (M0:166), from an S4U task in session 0 (doc:453–463). Sessions don't matter for tracing: ETW is system-wide (CB:1174), so session 0 sees the owner's session-1 presents.

### 4.2 The conflict

- M1:97: "PresentMon may run from SSH, because its tracing is system-wide (cookbook §10.4)."
- M1:81: "Claude's elevated sessions … never create, rename or delete anything in `logs\`, `state\` or `build\`, because a link you made there could point them at any file". They read there only through a checked handle (lib:102–137).
- An elevated PresentMon with `--output_file C:\kairo\logs\…` creates and truncates (§3.6) a file in `logs\` with an administrator's token. That is exactly the case M1:81 rules out. The research sketches' `C:\kairo\bench\` (R13:545, CB:1072) is gone too (M1:48), so the contract has no admin-only output folder for bench.

### 4.3 Options and the recommendation

| | Where PresentMon runs | CSV | Abort on owner input | Cost |
|---|---|---|---|---|
| **C (recommended)** [proposed] | A child of the bench job (`\Kairo\Run`, the owner's token, Performance Log Users), spawned with `CREATE_NO_WINDOW` and pipes | Written by the owner in `logs\<run-id>\`; the elevated side reads it with lib's checked handle | Yes: devrun kills the job tree at once (devrun/platform.rs:129–167, `TerminateJobObject`), as CL:25 and M1:98 want | A kill orphans the ETW session (§4.4): devrun or pc bench must stop it |
| B | A temporary S4U task as the owner, as the doctor does (doc:453–463), beside the loadgen job | As C | No: PresentMon runs to its `--timed` end (≤ 60 s more) after loadgen stops. A second task beside "one job at a time" (M1:98) | Task registration per run; no orphan session, since it ends cleanly |
| A | The elevated SSH session, with `--output_stdout` instead of `--output_file`; the remote script holds the CSV in memory and relays it as Base64 lines, as `harness-fetch.ps1` relays clips | No file on the PC at all; lands in `.dogfood/` (M1:78) | No; ends at `--timed` | Needs the SSH link for ~70 s per run (a drop kills PresentMon and orphans the session); elevated, with SeDebugPrivilege, which nothing here needs. In this mode the narrow-text stdout mangles non-ASCII exe names [Inference: CRT "C" locale] |

- **Recommendation: C.** It is the only option that meets CL:25 and M1:98 as written, it is unelevated, and the CSV lands where M1:75 puts everything a job writes.
- **Fallbacks:** B, if the bench orchestrator ends up outside the job. A, only if a later need for elevation appears (§4.5), and never with `--output_file`.
- **M1:97 should be reworded** in the bench PR: "PresentMon may run from SSH only when it writes no file (`--output_stdout`, `--terminate_existing_session`); a run that writes a CSV runs as you in the bench job."

### 4.4 Session names, orphans and cleanup

- **Names.**
  - `KairoBench`: pc bench.
  - `KairoDoctor`: the doctor's 2-second test (doc:460–461).
  - `PresentMon`: the default (CommandLine.cpp:384); never used, so Kairo can't stop another tool's session.
  - Adrenalin's own PresentMon session name is unknown [Unverified]; distinct names never collide.
  - The system holds about 64 sessions [Doc, MS-StartTrace].
- **An orphan** is a `KairoBench` session whose PresentMon was killed: by devrun's input stop or time limit under C, or by an SSH drop under A. Its DxgKrnl, DXGI, D3D9, DWM and Win32k providers stay enabled with no consumer until someone stops it or the PC restarts. That is a small, unmeasured cost on whatever the owner plays next [Estimate: small; Unverified]. Microsoft: "If your component's session is already running when your component starts, your component should clean up the orphaned session" [Doc, MS-StartTrace]; the next run's `--stop_existing_session` does that.
- **Cleanup** [proposed]:
  - **(a)** After any bench job that didn't end normally, pc bench runs `PresentMon-2.6.0-x64.exe --session_name KairoBench --terminate_existing_session` from the elevated session. It writes no file, so M1:81 holds. Exit 0 = stopped, 7 = none.
    - It always prints `warning: --terminate_existing_session exits without capturing anything; ignoring all other options.`, because its argument count check doesn't count `--session_name`'s value (CommandLine.cpp:579–585). The warning is harmless.
  - **(b)** The robust fix: a job-file field `"etw_sessions": ["KairoBench"]` that kairo-devrun stops by name (`ControlTraceW`, `EVENT_TRACE_CONTROL_STOP`) whenever it ends that job, for any reason. The owner's token may control sessions as a Performance Log User [Doc]. It belongs with devrun's safety stops (M1:103, M1:135); an unattended bench needs it, because (a) relies on the Mac staying connected.
  - **(c)** `pc doctor` lists running ETW session names read-only (`logman query -ets`) and reports a leftover `KairoBench`. The first bench session records that list in `.dogfood/` to answer the Adrenalin question (§12, item 1).

### 4.5 Handles on other processes, and where the PID comes from

- PresentMon itself never goes beyond `PROCESS_QUERY_LIMITED_INFORMATION` (§3.6). That is the level CL:9 allows Kairo, so even M2's game runs stay inside the rule [Code].
- Running it unelevated (option C) also keeps SeDebugPrivilege out of the process that holds a game handle [Inference: fewer reasons for an anti-cheat to object]. The first PresentMon run on an anti-cheat game is still practice or unranked with the owner present (CL:27).
- **M1:** the bench orchestrator starts loadgen itself, so it knows the PID from its own child, or from loadgen's ready file if loadgen copies the testbed's `ready.json` with `pid` (ready.rs:14–38; R16:403). No process lookup at all.
- **M2 (games the owner starts):** `--process_name <exe>`, PresentMon's own match.
  - If a game runs elevated or under another account, unelevated PresentMon may read `<unknown>` [Unverified]. Then the PID comes from `(Get-Process -Name X).Id`, which reads a system snapshot [Inference from `dl/ProcessManager.Windows.cs:47–49`].
  - **Never** read `.Path`, `.Company`, `.FileVersion`, `.ProductVersion`, `.Description`, `.Product` or `.MainModule` of a game's process object. PowerShell defines `Path` as `$this.Mainmodule.FileName` (dl/TypeTable_Types_Ps1Xml.cs:1142; the others at :1184–1244), and .NET's module enumeration opens `PROCESS_QUERY_INFORMATION | PROCESS_VM_READ` and calls `EnumProcessModulesEx` (dl/ProcessManager.Windows.cs:806–808). That is a module list with memory-read rights, banned by CL:9 [Code, downloaded copies of unknown tag].
  - Research 13's `encoder-usage.ps1` sketch does exactly that with `$p.Path` (R13:630–631); the encoder-usage note should drop it (M1:86 already asks for file names only).

---

## 5. The CSV PresentMon 2.6.0 writes for pc bench

### 5.1 File format [Code]

- **Encoding and layout:** UTF-8 **with a BOM** ("Files opened for writing in Unicode mode have a BOM written to them automatically" [Doc, MS-fopen]) and CRLF line ends (text mode). The header is line 1, written with the first row (CsvOutput.cpp:1421–1434).
- **No file** if no row was recorded.
- **Comma-separated, never quoted.** An exe name containing a comma shifts every field of its row; pm-stats counts fields.
- **Numbers** are printed by `fwprintf` in the C runtime's default "C" locale, so with `.`: no `setlocale` appears in the console, PresentData or `mc/` sources. `NA` marks a missing value. `MsBetweenPresents`, `MsBetweenDisplayChange`, `MsInPresentAPI` and `MsRenderPresentLatency` have 14 decimals (`DBL_DIG - 1`); other ms columns 4.
- **The recording window.** The main thread logs the QPC of each start and stop (OutputThread.cpp:42–62); the output thread compares each present's time with them (:471–481). So the window follows present time, not ETW delivery latency.
- **Edge effects.** Rows are emitted one displayed present late (the look-ahead, §5.3), so each end of the window can gain or lose a row or two.
- **Size:** about 278 bytes per row, 2.1 MB per 60 s at 125 fps and 17 MB at 1,000 fps [Mac].

### 5.2 The 30 columns, in order (pc bench's flags, neither `--v1_metrics` nor `--v2_metrics`)

Header: CsvOutput.cpp:530–672. Rows: :1121–1351. Formulas: `IntelPresentMon/CommonUtilities/mc/*` [Code].

| # | Column | Format | Meaning in 2.6.0 |
|---|---|---|---|
| 1 | `Application` | text | Exe file name, without folder: from Kernel-Process start events (trimmed at the last `\`, PresentMonTraceConsumer.cpp:2854–2858) or `QueryFullProcessImageNameW` (OutputThread.cpp:306–313); `<unknown>` if neither worked |
| 2 | `ProcessID` | `%d` | |
| 3 | `SwapChainAddress` | `0x%llX` (no zero padding; v1 pads) | |
| 4 | `PresentRuntime` | `DXGI`, `D3D9`, `Other` | |
| 5 | `SyncInterval` | `%d` | The app's value; loadgen's `Present(0, …)` → 0 |
| 6 | `PresentFlags` | `%d` | The DXGI present flags word (PresentMonTraceConsumer.cpp:2693): `DXGI_PRESENT_ALLOW_TEARING` = 512. `DXGI_PRESENT_TEST` presents are never counted (:2680–2683) |
| 7 | `AllowsTearing` | 0/1 | 1 if the frame could tear |
| 8 | `PresentMode` | text | `Hardware: Legacy Flip`, `Hardware: Legacy Copy to front buffer`, `Hardware: Independent Flip`, `Composed: Flip`, `Hardware Composed: Independent Flip`, `Composed: Copy with GPU GDI`, `Composed: Copy with CPU GDI`, `Other` (CsvOutput.cpp:25–37) |
| 9 | `FrameType` | text | `Application` (also for not-set and repeated frames), `Intel XeSS-FG`, `AMD AFMF`, `Unknown` (CsvOutput.cpp:54–70) |
| 10 | `TimeInQPC` | `%llu` | The present's start, raw QPC (`timeInSeconds = present.presentStartTime`, MetricsCalculatorAnimation.cpp:144) |
| 11 | `MsBetweenSimulationStart` | ms or `NA` | Needs app instrumentation; `NA` for loadgen |
| 12 | `MsBetweenPresents` | ms | This present's start − the previous present's start on the same swap chain (MetricsCalculatorAnimation.cpp:148–156) |
| 13 | `MsBetweenDisplayChange` | ms or `NA` | Screen time − the previous shown frame's screen time; `NA` when not shown (CsvOutput.cpp:1192–1199; MetricsCalculatorDisplay.cpp:27–36) |
| 14 | `MsInPresentAPI` | ms | Time inside `Present()` |
| 15 | `MsRenderPresentLatency` | ms | GPU completion (`ReadyTime`) − present start |
| 16 | `MsUntilDisplayed` | ms or `NA` | Screen time − present start; `NA` when not shown (CsvOutput.cpp:1208–1215) |
| 17 | `CPUStartQPC` | `%llu` | The frame's CPU start: the previous app present's start + its time in `Present()` (MetricsCalculator.cpp:332–353) |
| 18 | `MsBetweenAppStart` | ms (0 if missing) | `MsCPUBusy + MsCPUWait` (CsvOutput.cpp:1254–1257) |
| 19 | `MsCPUBusy` | ms (0 if missing) | Present start − CPU start; missing on non-app rows (MetricsCalculatorCpuGpu.cpp:14–36) |
| 20 | `MsCPUWait` | ms (0 if missing) | Time in `Present()` (:39–55) |
| 21 | `MsGPULatency` | ms | First GPU packet start − CPU start (:58–80) |
| 22 | `MsGPUTime` | ms | `MsGPUBusy + MsGPUWait` = last packet end − first packet start (CsvOutput.cpp:1260–1263; MetricsCalculatorCpuGpu.cpp:163) |
| 23 | `MsGPUBusy` | ms | Time during the frame when at least one of the process's packets ran on any non-video node (`mAccumulatedPacketTime`, GpuTrace.hpp:15–21; GpuTrace.cpp:536–593); 0 on non-app rows (MetricsCalculatorCpuGpu.cpp:83–99) |
| 24 | `MsGPUWait` | ms | `max(0, (ReadyTime − GPUStartTime) − MsGPUBusy)` (:137–145): gaps inside the frame's GPU span, for example waiting behind another process's work |
| 25 | `MsVideoBusy` | ms | The same, on video-typed nodes only (GpuTrace.cpp:205–227) |
| 26 | `MsAnimationError` | ms or `NA` | Change in the CPU-start delta vs the display delta (CPU-start source by default) |
| 27 | `AnimationTime` | ms or `NA` | From the first simulation start (or session start) to this frame's |
| 28 | `MsFlipDelay` | ms or `NA` | NVIDIA flip delay; `NA` elsewhere |
| 29–30 | `MsAllInputToPhotonLatency`, `MsClickToPhotonLatency` | ms or `NA` | Only with input tracking; **absent with `--no_track_input`**, which leaves 28 columns |

Expected for a healthy loadgen row [Inference from loadgen's swap chain, R13:705]:
- `PresentRuntime` `DXGI`, `SyncInterval` 0, `PresentFlags` 512;
- `AllowsTearing` 1 and `PresentMode` `Hardware: Independent Flip` (or `Hardware Composed: Independent Flip` on an overlay plane) [Unverified on the PC];
- `FrameType` `Application`, `MsVideoBusy` 0.0000.

### 5.3 Row semantics the statistics depend on [Code]

- **One row per display instance, not per present.** A shown present is processed once per entry of its `displayed` list (MetricsCalculator.cpp:181–227). Frame generation (AFMF, XeSS-FG) gives one present several instances.
  - Every row of one present has the same `TimeInQPC` and `MsBetweenPresents`: the swap chain's history updates only after the last instance (MetricsCalculator.cpp:229–234).
  - Only the application instance carries GPU and CPU numbers (`isAppFrame`, MetricsCalculatorCpuGpu.cpp:14–117).
  - Repeated flips next to an application flip are removed first (UnifiedSwapChain.cpp:23–45).
- **Dropped presents keep one row** (case 1, MetricsCalculator.cpp:100–137): `MsUntilDisplayed` and `MsBetweenDisplayChange` read `NA`, and GPU numbers are present. In Independent Flip with tearing almost nothing drops. In `Composed: Flip` above the refresh rate, DWM drops most presents.
- **Lost or failed presents leave no row** (OutputThread.cpp:437–440): the next row's interval spans the gap, so one frame is missing from the count. PresentMon reports such losses only as stderr warnings (§3.5).
- **The first present of a swap chain only seeds its history**: no row (OutputThread.cpp:355–360; UnifiedSwapChain.cpp:47–52).
- **A stall of 4 s or more removes the swap chain** (`PruneOldSwapChainData`, OutputThread.cpp:247–292, 4,000 ms at :264). The next present re-seeds without a row, so **the stall vanishes from Σ `MsBetweenPresents`**; `duration_s` catches it (§8.4).
- **Row order within a swap chain is present order**: dropped presents that arrive while a shown one waits for its successor are released right after it (UnifiedSwapChain.cpp:55–112). Other processes' rows may interleave in the file.
- **HAGS:** "When HWS is enabled, … *msGPUActive* … may be reported ~0.5ms larger than the true GPU work duration" (PMREADME:75–84). Compare GPU numbers only between runs with the same HAGS state (`env.json`, R13:821).

### 5.4 Where research 13, the cookbook and the README differ from the code

- R13:557 lists `TimeInMs`; with `--qpc_time` it is `TimeInQPC` (CsvOutput.cpp:558).
- PMR:97 and PMR:104 name the time and video columns `CPUStartTime` and `VideoBusy`. Those are the `--v2_metrics` names (CsvOutput.cpp:581–600); the default CSV uses `CPUStartTimeInMs`/`CPUStartQPC` and `MsVideoBusy` (:617–636). Research 13's "read columns by header name" (R13:559, R13:865) stands.
- R13:553 lists three `FrameType` values; there is a fourth, `Unknown`, and not-set or repeated frames print as `Application`.
- R13:563's "frames ÷ Σ frame time" over rows double-counts presents with several instances. Dedupe first (§6.3).
- R13:545 and CB:1072 write into `C:\kairo\bench\` and run `C:\kairo\tools\PresentMon-2.6.0-x64.exe`; the contract replaced both (M1:48, M1:74–75).

---

## 6. pm-stats.ps1

### 6.1 Interface

```
pm-stats.ps1 -Csv <path> -Exe <file name> -Tag <tag> [-ProcessId <pid>]
```
- `-Exe` is compared case-insensitively and exactly with `Application`, for example `kairo-loadgen.exe`.
- `-Tag` must match `^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$` [proposed], so no path or PC detail can ride in it; for example `alu-aa-r3`.
- `-ProcessId` narrows the selection when two processes share the name.
- **A function plus a thin CLI** [proposed]: the script defines `Get-PmStats -Text <string> -Exe -Tag [-ProcessId]`, and `-Csv` reads the file and calls it.
  - The elevated remote script dot-sources it and passes text it read with lib's `Read-SharedText` (lib:140–157). On a file in `logs\`, that opens the file once and checks through the same handle that the owner's account owns it (lib:111–131). Never `Import-Csv -LiteralPath` on `logs\` (M1:81).
  - CI calls the function on generated text.
- The Mac has no PowerShell (`which pwsh`: not found), so pm-stats and ab-stats run on the PC or in CI.
- They must also **parse under Windows PowerShell 5.1**: CI parses every `tools/**/*.ps1` with 5.1 and 7 (ci:226–247). So no `??`, no ternary, no `&&`; they run on pwsh 7.

### 6.2 Reading

1. Detect a BOM (UTF-8 expected; UTF-16 accepted); split on CRLF or LF.
2. Header = the first line, split on `,`.
   - Required: `Application`, `ProcessID`, `SwapChainAddress`, `PresentMode`, `MsBetweenPresents`, `MsUntilDisplayed`, `MsGPUBusy`, `MsGPUWait`, and one of `TimeInQPC`, `TimeInMs`, `TimeInSeconds`, `TimeInDateTime`.
   - Optional: `FrameType`; missing → `frame_types` null and a warning.
   - A `--v2_metrics` header (`FrameTime`, no `MsBetweenPresents`) fails with exit 3 and says so.
3. Rows: split on `,`. A row whose field count differs from the header's is a **bad row** and skipped. If the text doesn't end with a newline, its last line is a bad row: a killed PresentMon's truncated tail.
4. Numbers: `[double]::TryParse(s, NumberStyles.Float, CultureInfo.InvariantCulture)`; `NA`, empty or non-finite → missing.

### 6.3 Selection and formulas

1. **Process:** rows with `Application -ieq $Exe` (and `ProcessID -eq $ProcessId` when given). None → exit 3; stderr gives counts only, never other processes' names. More than one `ProcessID` without `-ProcessId` → exit 3.
2. **Main swap chain:** group by `SwapChainAddress`; the group with the most rows wins, ties going to the group seen first. Research 13's `Sort-Object Count -Descending` (R13:580) leaves ties to an unstable sort. `swapchains` = number of groups.
3. **Presents:** within the main chain, group rows by the time column. A present's row for frame time, GPU numbers and mode is its first `FrameType = Application` row, else its first row.
4. **Frames:** presents whose `MsBetweenPresents` is a number > 0. Others count in `bad_rows`: they don't occur in a healthy CSV. `frames` = n; F = their `MsBetweenPresents` values.
5. `avg_fps = 1000 · n / ΣF`, a time-weighted rate that includes dropped presents, because the app made them. Same definition as R13:563.
6. **Nearest rank:** sort F ascending; `P(p) = F[k − 1]` with `k = max(1, ⌈p · n⌉)`, computed in integers as `k = (99·n + 99) div 100` for P99 and `(999·n + 999) div 1000` for P99.9. `low1_fps = 1000 / P99`, `low01_fps = 1000 / P99.9`.
   - Research 13's `[math]::Ceiling($p * $s.Count)` (R13:577) gives the same k for every n ≤ 2,000,000 [Mac], so either form is right; the integer form states the intent.
   - With n < 100, P99 is the maximum; with n < 1,000, so is P99.9. Warn on n < 1,000.
7. `gpu_busy_ms`, `gpu_wait_ms`: arithmetic means of the chosen rows' `MsGPUBusy` and `MsGPUWait` (missing values skipped; zeros kept). Research 13 averages all rows, which with frame generation mixes in the generated rows' zeros (vector P2).
8. `present_modes`: per-present counts of `PresentMode` (one per present, not per instance).
9. `frame_types`: counts of `FrameType` over **all instance rows** of the counted presents. Generated frames are instances, so `{"AMD AFMF": …}` shows AFMF at work.
10. **Diagnostics** [proposed]:
    - `duration_s = ΣF / 1000`;
    - `dropped_pct = 100 · (presents with MsUntilDisplayed NA) / n`;
    - `swapchains`;
    - `rows` = rows selected for the process across all its chains;
    - `bad_rows`.
11. Nothing is trimmed by time. The 2-minute warm-up is a separate, discarded run (R13:830), and `--delay 5` already keeps PresentMon's own start-up out of the CSV.

### 6.4 Output

- One compact JSON line on stdout, keys in this order [proposed: `pm_stats` and the last 5 are additions that M1:85 gains in the same PR, contract first]:

```
{"pm_stats":1,"tag":"…","frames":n,"avg_fps":x.xx,"low1_fps":x.xx,"low01_fps":x.xx,"gpu_busy_ms":x.xxx,"gpu_wait_ms":x.xxx,
 "present_modes":{"<mode>":count,…},"frame_types":{"<type>":count,…},"duration_s":x.xxx,"dropped_pct":x.xx,"swapchains":n,"rows":n,"bad_rows":n}
```

- **Rounding:** fps to 2 decimals, ms and seconds to 3, percent to 2, `[math]::Round(x, d, [MidpointRounding]::AwayFromZero)`.
- **Serialisation:** `ConvertTo-Json -Compress` from an `[ordered]` dictionary writes numbers invariantly. Research 13's `'{0}={1:P1}' -f …` (R13:592) follows the current culture ("99,5 %" on a German PC) and loses the counts.
- **Warnings** go to stderr, one line each [proposed]: `swapchains > 1`, `bad_rows > 0`, `frames < 1000`, `FrameType` values other than `Application`, `FrameType` column missing.

### 6.5 Exit codes [proposed, kairo-verify's convention, M1:90]

- **0** — line printed.
- **2** — bad command line: a missing argument, a bad `-Tag`, or a `-ProcessId` that isn't a positive integer.
- **3** — the CSV can't be evaluated: no file (PresentMon recorded nothing), unreadable or empty, a required column missing, no row for the process, several processes, or no valid frame.
- pm-stats never judges; pc bench does (§8.4).

### 6.6 Changes from research 13's sketch (R13:572–595)

Dedupe by present; deterministic main chain; count objects instead of culture-formatted strings; exit codes instead of `throw` and division by zero; field-count checks instead of `Import-Csv`'s silent padding of short rows; checked-handle reading on the PC; the diagnostics above.

---

## 7. ab-stats.ps1

### 7.1 Interface and modes [proposed]

```
ab-stats.ps1 -A <runs> [-B <runs>] [-Json] [-Reference <ab-stats JSON line or file>]
```
- `<runs>` is a `[string[]]`: every item is a pm-stats JSON line, or a path to a file of such lines in run order, or a bare number (`avg_fps`, research 13's form, R13:599). One kind per call.
  - With bare numbers, only the mean and upper-bound clauses can be evaluated.
  - The remote script passes JSON lines straight from the Mac's payload, so no file is needed on the PC.
- **A only → the A/A noise floor** (§8). **A and B → paired A/B.**
- Pairs go by order index, so the i-th A run pairs with the i-th B run. With ABBA ×3 (positions A = 1, 4, 5, 8, 9, 12; B = 2, 3, 6, 7, 10, 11) every pair is adjacent in time: (1,2) (4,3) (5,6) (8,7) (9,10) (12,11) [Mac].

### 7.2 Paired loss (M1's scope, R13:597–606)

- `dᵢ = 100 · (Aᵢ − Bᵢ) / Aᵢ` from each run's `avg_fps`. Positive is a loss (B, Kairo on, slower).
- `m = mean(d)`, `s = sqrt(Σ(dᵢ − m)² / (n − 1))` (sample SD), `h = t(0.975, n − 1) · s / √n`, CI = `[m − h, m + h]`.
- `PASS = (m ≤ 3) and (m + h ≤ 4)`: research 13's two clauses (R13:731, PLAN:353).
- Requires equal counts and n ≥ 2 (else exit 2), and every `Aᵢ > 0` (else exit 3).

### 7.3 The t quantile

- A table for df 1–30 [Mac, exact to 4 decimals; matches the published values at df 1–15, 20, 30]:
  12.7062, 4.3027, 3.1824, 2.7764, 2.5706, 2.4469, 2.3646, 2.3060, 2.2622, 2.2281, 2.2010, 2.1788, 2.1604, 2.1448, 2.1314, 2.1199, 2.1098, 2.1009, 2.0930, 2.0860, 2.0796, 2.0739, 2.0687, 2.0639, 2.0595, 2.0555, 2.0518, 2.0484, 2.0452, 2.0423.
- **For df > 30**, the Cornish–Fisher series with `z = 1.959963984540054` and `ν = df`:
  `t ≈ z + (z³+z)/(4ν) + (5z⁵+16z³+3z)/(96ν²) + (3z⁷+19z⁵+17z³−15z)/(384ν³) + (79z⁹+776z⁷+1482z⁵−1920z³−945z)/(92160ν⁴)`.
  Error vs an exact inversion: 3.1×10⁻⁸ at df 30, 7.3×10⁻⁹ at 40, 3.0×10⁻¹¹ at 120 [Mac].
- Research 13's table (R13:600) has only n = 2–10, 12 and 16. For any other n, `$t975[$n]` is `$null`, `h` becomes 0 and PASS rests on the mean alone (vector AB2).

### 7.4 Output

- **Text (M1:85's form), invariant culture, `F2`:**
  - paired: `6 pairs: loss 2.44% (95% CI 2.25..2.63%) PASS=True`, with one space before `PASS`. R13:605 has two, and `{1:N2}` follows the current culture;
  - A/A: `6 runs: avg_fps 120.33 sd 0.71 (CV 0.59%) NOISE=ok` [proposed].
- **`-Json`** [proposed], one line at full precision:
  - `{"ab_stats":1,"mode":"ab","n":…,"loss_pct":…,"sd_pp":…,"t":…,"ci_lo":…,"ci_hi":…,"clauses":{"mean_le_3":…,"upper_le_4":…,"low1_le_5":null,"present_mode":null,"ir_plus_1pp":null},"pass":…}`;
  - for A/A: `{"ab_stats":1,"mode":"aa","n":…,"mean_fps":…,"sd_fps":…,"cv_pct":…,"low1_cv_pct":…,"drift_pct_per_run":…,"noise_ok":…}`.
  - `null` = not evaluated (M1).
- **Exit codes** [proposed]: 0 PASS / noise ok · 1 FAIL / noise high · 2 bad command line (unequal counts, n < 2, mixed kinds) · 3 can't evaluate (`Aᵢ ≤ 0`, unparsable line).

### 7.5 The clauses that join before SP1 (spec only; M1:85, M1:201, PLAN:346, PLAN:353, R13:731) [proposed]

| Clause | Source | Formula |
|---|---|---|
| Mean loss ≤ 3% | PLAN:353 | §7.2 (M1) |
| Upper 95% bound ≤ 4% | PLAN:353 | §7.2 (M1) |
| 1%-low loss ≤ 5% | PLAN:353, R13:731 | `d1ᵢ = 100·(A.low1ᵢ − B.low1ᵢ)/A.low1ᵢ`, same pairs; mean(d1) ≤ 5, a point estimate, since the plan asks for no CI here |
| PresentMode unchanged | PLAN:353, R13:731 | Pool `present_modes` counts per arm. Same dominant mode in A and B; B's dominant share ≥ A's − 1 pp; no mode new in B with ≥ 1% share |
| No worse than Instant Replay alone + 1 pp | PLAN:353; IR "as its own pair", PLAN:427 | `m_Kairo − m_IR ≤ 1.0` pp, both mean paired losses from the same session and workload; `m_IR` from `-Reference` |
| Release regression > 1 pp blocks | PLAN:346 | `m_new − m_baseline ≤ 1.0` pp against the last accepted baseline for the same workload and `env.json` fingerprint, through `-Reference` |

- With all of them, `PASS` = every evaluated clause, and the text line keeps M1's form.
- R13:840's "3 pairs, expanding to 6 if the result is within 1% of a decision threshold" reads as: expand when `|m − 3| ≤ 1` or `|m + h − 4| ≤ 1`. That is pc bench's decision, not ab-stats'.

### 7.6 Changes from research 13's sketch (R13:597–606)

The full t table plus the series; count and positivity checks (a shorter B made `$B[$i]` `$null`, read as 0, a 100% loss); n = 1 no longer divides by zero; invariant formatting; JSON lines as input, so the later clauses can see `low1_fps` and `present_modes`; the A/A mode.

---

## 8. The A/A noise-floor rule (M1:29, PLAN:353, R13:730, R13:831)

### 8.1 Rule [proposed where it goes beyond the plan]

1. Per workload (loadgen ALU and bandwidth separately), after one discarded warm-up of 120 s (R13:830). The warm-up runs loadgen without PresentMon: nothing is recorded, so nothing needs discarding.
2. 6 **valid** runs (§8.4), each `--delay 5 --timed 60`, spaced and restarted exactly as the A/B will be (R13:833: 30 s idle between runs). Otherwise the A/A measures a different noise than the one the A/B faces. An invalid run is re-run in its slot, at most 3 times per workload, then the session stops and reports.
3. Statistic: **CV = 100 · s / x̄ of per-run `avg_fps`**, with s the **sample** SD (n − 1). PowerShell's `Measure-Object -StandardDeviation` already computes it: Learn's example gives 1.58113883008419 for `1..5`, √2.5 rather than √2 [Doc, PS-Measure].
4. CV ≤ 1.00% → the noise floor is fine. CV > 1.00% → 6 more valid runs, CV over all 12. Still above 1% → report it as high, with §8.3's pair count, and look for the cause (thermals, background tasks, power plan; R13:831). **No trimming and no outlier removal:** a run leaves the set only by §8.4's objective rules.
5. Diagnostics, not gating: CV of `low1_fps`; the least-squares drift of `avg_fps` per run as % of the mean (thermal creep shows here; ABBA cancels linear drift, an A/A CV doesn't).
6. Gate 4 asks only that the A/A "produce data" (M1:29); a high CV is a finding, not a failed gate.

### 8.2 Why these choices

- **Sample SD:** the runs are a sample, and the A/B's CI uses the sample SD too (§7.2). The population SD reads about 9% low at n = 6 (vector AA1: 0.54% vs 0.59%).
- **`avg_fps`**, because the gate is on average FPS (PLAN:353; R13:730).
- **6 runs** = the plan's number (M1:29, PLAN:353). Six runs estimate σ only loosely: the 95% interval for σ is [0.62 s, 2.45 s] at n = 6 and [0.71 s, 1.70 s] at n = 12 (χ² with 5 and 11 df) [Mac]. So "CV ≤ 1%" screens the setup; it doesn't prove a floor. The A/B's own CI, from its own pairs, is what the gate uses.

### 8.3 What a CV buys the A/B [Estimate: independent runs of equal variance, so paired SD ≈ √2 · CV]

| Run CV | Half-width h, 6 pairs | Pairs for h ≤ 1.0 pp |
|---|---|---|
| 0.25% | 0.37 pp | 3 |
| 0.50% | 0.74 pp | 5 |
| 0.674% | 1.00 pp | 7 (6 gives 1.0003) |
| 1.00% | 1.48 pp | 11 |
| 1.50% | 2.23 pp | 20 |
| 2.00% | 2.97 pp | 34 |

- `h = t(0.975, n − 1) · √2 · CV / √n`; at 6 pairs `h = 1.484 · CV` [Mac].
- At CV 1%, a B whose true loss exceeds about 2.5% fails "upper ≤ 4%" even though it passes "mean ≤ 3%". The plan's 1 pp gap between the two clauses needs a run CV of about 0.67% or less at 6 pairs.
- Pairing adjacent runs removes slow drift, so a drift-dominated A/A overstates the A/B's noise.

### 8.4 When a run is invalid [proposed; pc bench applies it, A/A and A/B alike]

1. The bench job ended normally (no input stop, no time limit), and PresentMon exited 0 with both `Started recording.` and `Stopped recording.` on stdout.
2. No stderr line containing `ETW buffers were lost`, `ETW events were lost` or `overflowed present events` (MainThread.cpp:448–462).
3. The CSV exists (exit 0 alone proves nothing, §3.5), and pm-stats exits 0 with `bad_rows = 0` and `swapchains = 1`.
4. `|duration_s − 60| ≤ 1.0`. A loadgen exit shortens it; a stall ≥ 4 s is pruned out of the CSV and shortens it too (§5.3).
5. `frame_types` holds only `Application`. `AMD AFMF` means AFMF is on, which invalidates the measurement (R13:866).
6. One `present_modes` entry holds ≥ 99% of frames, and it is the same mode in every run of the workload. A run in another mode is data about the setup, not noise.

---

## 9. Test vectors

### 9.1 Fixture rows

- CI builds every CSV in code from the 30-column header of §5.2, with these defaults: `Application` `kairo-loadgen.exe`, `ProcessID` 4242, `SwapChainAddress` `0x1A2B3C4D5E0`, `PresentRuntime` `DXGI`, `SyncInterval` 0, `PresentFlags` 512, `AllowsTearing` 1, `PresentMode` `Hardware: Independent Flip`, `FrameType` `Application`.
- Numeric defaults: `MsBetweenSimulationStart` `NA`, `MsBetweenDisplayChange` = `MsBetweenPresents` (`NA` when the row is dropped), `MsUntilDisplayed` `2.5000`, `MsGPUBusy` `7.9000`, `MsGPUWait` `0.1000`; `MsAnimationError`, `AnimationTime`, `MsFlipDelay` and both input columns `NA`; every other column `0`.
- `MsBetweenPresents` is written with 14 decimals.
- Files: BOM + CRLF unless stated.
- Synthetic only: no PC data is committed (CL:18).

### 9.2 pm-stats vectors [Mac, `ref`]

| # | Rows (differences from the defaults) | Expected line (`-Exe kairo-loadgen.exe` unless stated) |
|---|---|---|
| P1 | 10 presents; `MsBetweenPresents` 8 ×9 then 16; `MsGPUBusy` 7.0, 7.1 … 7.9; `MsGPUWait` 0.0 … 0.9; `TimeInQPC` 1000000 + 80000·i | `{"pm_stats":1,"tag":"aa-alu-1","frames":10,"avg_fps":113.64,"low1_fps":62.5,"low01_fps":62.5,"gpu_busy_ms":7.45,"gpu_wait_ms":0.45,"present_modes":{"Hardware: Independent Flip":10},"frame_types":{"Application":10},"duration_s":0.088,"dropped_pct":0.0,"swapchains":1,"rows":10,"bad_rows":0}` |
| P6 | P1 without BOM, LF only | identical to P1 |
| P2 | 6 presents, ms 10,10,10,20,20,20; the first 3 also have an `AMD AFMF` row first (same `TimeInQPC` and ms, busy 0, wait 0); app rows busy 6.0, wait 0.5 | `…"frames":6,"avg_fps":66.67,"low1_fps":50.0,"low01_fps":50.0,"gpu_busy_ms":6.0,"gpu_wait_ms":0.5,…,"frame_types":{"AMD AFMF":3,"Application":6},"duration_s":0.09,…,"rows":9,…`. Research 13's sketch: frames 9, avg 75.00, busy 4.0 (wrong) |
| P3 | 8 presents, ms 5, mode `Composed: Flip`, rows 3 and 6 dropped (`MsUntilDisplayed` `NA`) | `…"frames":8,"avg_fps":200.0,…,"gpu_busy_ms":4.2,"gpu_wait_ms":0.3,"present_modes":{"Composed: Flip":8},…,"dropped_pct":25.0,…` |
| P4 | chain `0xAAAA0` ×5 (ms 10), chain `0xCCCC0` ×3 (ms 30), `other.exe` ×5; `-Exe Kairo-Loadgen.EXE` | `…"frames":5,"avg_fps":100.0,…,"swapchains":2,"rows":8,"bad_rows":0}` |
| P5 | 6 good presents (ms 10), one row with `Application` `kairo,loadgen.exe` (31 fields), one row with `MsBetweenPresents` `NA`, a last line cut to 40 characters without a newline | `…"frames":6,"avg_fps":100.0,…,"frame_types":{"Application":6},…,"rows":7,"bad_rows":3}` |
| P10 | 200 presents, ms 1, 2 … 200 | `…"frames":200,"avg_fps":9.95,"low1_fps":5.05,"low01_fps":5.0,…,"duration_s":20.1,…` (P99 = 198 ms, rank 198; P99.9 = 200 ms) |
| P11 | 7,500 presents, ms `8 + 0.25·(i mod 5)` except `i mod 250 = 249` → 25 ms; busy `7.5 + 0.01·(i mod 10)`; wait `0.05·(i mod 3)` | `…"frames":7500,"avg_fps":116.77,"low1_fps":111.11,"low01_fps":40.0,"gpu_busy_ms":7.545,"gpu_wait_ms":0.05,…,"duration_s":64.23,…`. The 30 hitches (0.4%) miss the 1% low but set the 0.1% low |

- Compare numbers as numbers, not text: `ConvertTo-Json` may print `40.0` or `40`.

### 9.3 pm-stats error vectors (exit codes)

- **E1** — no file: exit 3.
- **E2** — header only: exit 3.
- **E3** — a `--v2_metrics` header (`…,CPUStartTime,FrameTime,CPUBusy,…`): exit 3, "missing MsBetweenPresents (a --v2_metrics CSV?)".
- **E4** — rows only for `other.exe`: exit 3.
- **E5** — two PIDs for `kairo-loadgen.exe`: exit 3 without `-ProcessId`; with `-ProcessId 4242`, the stats of that PID.
- **E6** — `-Tag ../x`: exit 2.

### 9.4 ab-stats and A/A vectors [Mac, `ref`]

| # | Input (`avg_fps`) | Expected |
|---|---|---|
| AB1 | A = 125.40, 124.90, 125.10, 125.60, 124.80, 125.20; B = 122.10, 121.95, 122.40, 122.30, 121.70, 122.25 | `6 pairs: loss 2.44% (95% CI 2.25..2.63%) PASS=True` (s = 0.1823, t = 2.5706, h = 0.1913) |
| AB2 | A = 100 ×11; B = 97.0, 97.5, 96.8, 97.2, 97.1, 96.9, 97.3, 97.0, 96.7, 97.4, 97.1 | `11 pairs: loss 2.91% (95% CI 2.74..3.07%) PASS=True` (t = 2.2281). Research 13's sketch prints `CI 2.91..2.91` |
| AB3 | A = 100 ×6; B = 98 ×6 | `6 pairs: loss 2.00% (95% CI 2.00..2.00%) PASS=True` (s = 0) |
| AB4 | A = 100 ×6; B = 96.0, 99.0, 97.5, 95.5, 98.5, 97.0 | `6 pairs: loss 2.75% (95% CI 1.31..4.19%) PASS=False`: the mean passes, the upper bound fails |
| AB5 | A = 100 ×3; B = 101.0, 100.5, 101.5 | `3 pairs: loss -1.00% (95% CI -2.24..0.24%) PASS=True` (a gain) |
| AE1–3 | 6 A vs 5 B; n = 1; an `Aᵢ` of 0 | exit 2; exit 2; exit 3 |
| AA1 | 120.0, 121.2, 119.4, 120.6, 119.8, 121.0 | `6 runs: avg_fps 120.33 sd 0.71 (CV 0.59%) NOISE=ok`; population CV would be 0.54%; drift +0.0475%/run |
| AA2 | 118.0, 120.4, 121.9, 119.1, 122.3, 117.6 | `6 runs: avg_fps 119.88 sd 1.98 (CV 1.65%) NOISE=high` → 6 more runs |
| L1 | low1: A = 111.1, 110.8, 111.3, 111.0, 110.9, 111.2; B = 106.0, 105.2, 106.4, 105.9, 105.1, 106.2 | mean 1%-low loss 4.73% → `low1_le_5` true (SP1 clause) |

---

## 10. How this lands [proposed; the synthesis owns the PR plan]

- **`m1/bench-stats`** (Mac and CI only, no PC):
  - `tools/bench/pm-stats.ps1`, `tools/bench/ab-stats.ps1` and a test script with §9's vectors, run in CI's existing `powershell` job on `windows-latest` with pwsh (ci:216–247). Both scripts also parse under 5.1.
  - M1:85 gains the extra fields; CHANGELOG.
  - About 600 lines with tests [Estimate].
  - Instrument work, outside the exit-rule clock (M1:7).
- **The pc bench PR** carries:
  - the §3.1 invocation (validated PID, run folder made first, pipes);
  - the §4.4 cleanup;
  - the §8.4 validity checks and the per-session SHA-256 check;
  - M1:74 with `--no_track_input` if accepted, and M1:97 reworded (§4.3);
  - the file names in M1:75.
- **devrun's `etw_sessions` field** goes with devrun's safety stops (M1:103, M1:135), which unattended runs need anyway.

---

## 11. Owner questions

None from this part:
- PresentMon is already installed and pinned (M0);
- running it with the owner's limited token is what M0 set up and approved (Performance Log Users, M0:11, M0:166);
- nothing changes a Windows setting, opens a prompt or spends anything;
- test data is synthetic and built at test time.

Later, not now: the first PresentMon run on an anti-cheat game (M2) is practice or unranked, with the owner present (CL:27).

---

## 12. Unverified items and risks

| # | Item | How it gets settled | Fallback |
|---|---|---|---|
| 1 | Adrenalin's own PresentMon session name | Read-only `logman query -ets` names at the first bench session (kept in `.dogfood/`) | Irrelevant to correctness: `KairoBench` is unique |
| 2 | Loadgen's `PresentMode` on the 180 Hz monitor (iFlip, Hardware Composed iFlip, or Composed) | The first A/A run's `present_modes` | Composed is still valid data; the A/B then compares like with like |
| 3 | `MsGPUBusy` populated on the RX 9000-series card at the PC's HAGS state; HAGS adds ~0.5 ms (PMREADME:75–84) | The first run's `gpu_busy_ms > 0`; HAGS state in `env.json` | Report GPU columns as unavailable; FPS clauses don't need them |
| 4 | AMD's VCN node typed as `VIDEO_*` in DxgKrnl node metadata, so `MsVideoBusy` works on AMD | An M2 workload with video; loadgen has none | Video work stays inside `MsGPUBusy`; PDH (`encoder-usage.ps1`) is Kairo's video meter anyway (R13:864) |
| 5 | The owner's interactive token (the `\Kairo\Run` job) carries Performance Log Users; M0 proved the S4U token (M0:166) | The first bench job: exit 6 + "access denied" if not | Sign out and in (PMREADME:60); option B meanwhile |
| 6 | The AMD driver emits Intel-PresentMon frame-type events for AFMF | Only matters with AFMF on (R13 §3.2 test) | AFMF stays off in every bench (R13:826) |
| 7 | No `setlocale` in the parts of CommonUtilities not read | pm-stats parses invariantly and exits 3 if `MsBetweenPresents` doesn't parse | — |
| 8 | Orphaned-session cost on the owner's games | §4.4 cleanup; doctor reports leftovers | Reboot clears it |
| 9 | QPC frequency on the PC (10 MHz?), for lining up `CPUStartQPC` with Kairo's 100 ns traces in M2 | `wgc-probe`'s or the testbed's capability report logs `QueryPerformanceFrequency` [proposed] | Convert with the logged frequency |
| 10 | ETW losses or ring overflow at loadgen's frame rate | §8.4 rule 2 | `--set_circular_buffer_size 4096` for that workload, A and B alike |
| 11 | An elevated or other-account game reads `<unknown>` to unelevated PresentMon (M2) | The first M2 game run | `--process_id` from `(Get-Process -Name X).Id` only (§4.5) |

---

## Sources

- **PresentMon v2.6.0** (tag → `e13fce6a`): README-ConsoleApplication.md; README.md; LICENSE.txt; THIRD_PARTY.txt; Version.props; PresentMon/{CommandLine, Console, ConsumerThread, CsvOutput, LogSetup, MainThread, OutputThread, Privilege}.cpp, PresentMon.hpp, PresentMon.vcxproj; PresentData/{GpuTrace, PresentMonTraceConsumer, PresentMonTraceSession, Debug, InterPresentActivity, NvidiaTraceConsumer, TraceConsumer, TraceLogging}.cpp and the .hpp files; IntelPresentMon/CommonUtilities/mc/{MetricsCalculator, MetricsCalculatorAnimation, MetricsCalculatorCpuGpu, MetricsCalculatorDisplay, UnifiedSwapChain}.cpp/.h. https://github.com/GameTechDev/PresentMon/tree/v2.6.0
- **GitHub API**, live 2026-11-14: https://api.github.com/repos/GameTechDev/PresentMon/releases/tags/v2.6.0 · https://api.github.com/repos/GameTechDev/PresentMon/git/ref/tags/v2.6.0 · https://github.com/GameTechDev/PresentMon/releases/tag/v2.6.0
- **PresentMon v1.9.2 README:** https://github.com/GameTechDev/PresentMon/blob/v1.9.2/README.md
- **Microsoft Learn:**
  - https://learn.microsoft.com/en-us/cpp/c-runtime-library/reference/fopen-wfopen
  - https://learn.microsoft.com/en-us/windows/win32/api/evntrace/nf-evntrace-starttracew
  - https://learn.microsoft.com/en-us/windows/win32/api/tdh/nf-tdh-tdhloadmanifestfrombinary
  - https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.utility/measure-object
- **Searched without a primary answer** (Adrenalin's session name):
  - AMD DH3-038: https://amd.com/en/resources/support-articles/faqs/DH3-038.html
  - an anecdotal Framework forum post on a "presentmon" HWiNFO sensor: https://community.frame.work/t/amd-users-presentmon-in-hwinfo64/42543
- **Kairo, at ddec2c9:**
  - CLAUDE.md; docs/PLAN.md; docs/milestones/M0.md, M1.md; docs/research/13, 15, 16;
  - tools/pc/bootstrap.ps1, doctor.ps1; crates/kairo-devrun/src/run.rs, platform.rs; .github/workflows/ci.yml;
  - branches: lib.ps1 (harness, PRs #34–35), ready.rs (testbed, PR #31).
- **Mac computations:** `scratchpad/research17-scripts/statref.py` (incomplete-beta t quantiles, χ² quantiles, the pm-stats and ab-stats reference) and `vectors.py`.
