# Research 17, PC tooling: how `pc bench` runs on the PC

*2026-11-14 · Read-only. Written against `main` at ddec2c9, the testbed branch `m1/testbed` at f88ed79 (PR #31) and the harness branch `m1/harness-run` at 1cbfa9f (PRs #34–#35). Nothing in the repo or on the PC was changed, built or run. The PresentMon files the earlier run downloaded (`research17/pm-src/`) were checked before use: all 36 match the git blob hashes in `tree.json` for tag `v2.6.0` (commit e13fce6, `tagref.json`), and `release.json` lists `PresentMon-2.6.0-x64.exe`, 980,320 bytes, digest `sha256:b2a706bc…88f1af`, which is the bootstrap's pin. The .NET and PowerShell sources in `research17/dl/` carry no tag, so what they show is labelled [M].*

**Citation keys.**

| Key | File |
|---|---|
| `CL:n` | CLAUDE.md |
| `M0:n`, `M1:n`, `PLAN:n` | docs/milestones/M0.md, M1.md, docs/PLAN.md (main) |
| `R13:n`, `R16:n`, `CB:n` | docs/research/13-amd-performance.md, 16-verify-testbed/README.md, 15-implementation-cookbook.md (main) |
| `pc:n` | tools/mac/pc (main) |
| `lib:n`, `run.ps1:n`, `stub:n`, `rdoc:n` | tools/mac/remote/lib.ps1, run.ps1, stub.ps1, run-doctor.ps1 (main) |
| `pcH:n`, `libH:n`, `runH.ps1:n`, `hfetch:n`, `hevents:n`, `hpre:n` | the harness branch's tools/mac/pc, lib.ps1, run.ps1, harness-fetch.ps1, harness-events.ps1, harness-pre.ps1 |
| `devrun/x.rs:n`, `devrunH/run.rs:n` | crates/kairo-devrun/src (main; harness branch) |
| `core/x.rs:n`, `display/lib.rs:n` | crates/kairo-core/src, crates/kairo-display/src (main) |
| `harness/x.rs:n` | crates/kairo-harness/src (harness branch) |
| `testbed/x.rs:n` | crates/kairo-testbed/src (testbed branch) |
| `boot:n`, `doctor:n` | tools/pc/bootstrap.ps1, tools/pc/doctor.ps1 |
| `PM:file:n` | PresentMon v2.6.0 source, `research17/pm-src/` |
| `NET:file:n`, `PS:file:n` | dotnet/runtime `System.Diagnostics.Process` and PowerShell `TypeTable_Types_Ps1Xml.cs`, `research17/dl/` (tag not recorded) |

**Evidence labels.** **[read]** checked in the cited code or document today. **[M]** a source whose version isn't pinned, or well-known behaviour not re-read today. **[Inference]** follows from read sources but isn't stated in them. **[Unverified]** not checked; §17 says how it gets settled. **[Estimate]** a planning number. **[proposed]** Claude's choice, to confirm with PC data.

---

## 1. Summary

1. **PresentMon runs inside the bench job, as the owner, not in the elevated SSH session** [proposed]. An elevated PresentMon has nowhere allowed to write: job output belongs in `C:\kairo\logs\` (M1:75), elevated sessions never create files there (M1:81, lib:92–99), and PresentMon opens its CSV by path in mode `"w,ccs=UTF-8"`, which truncates whatever a link there points to (PM:CsvOutput.cpp:1382). The owner's limited token already ran PresentMon in M0 (M0:166); without `SeDebugPrivilege` PresentMon only warns and carries on (PM:MainThread.cpp:266–277); `--process_id` needs no name lookup. "PresentMon may run from SSH" (M1:97) stays as the fallback (§3).
2. **One `\Kairo\Run` job per bench session.** A new verb, `kairo-harness bench`, runs the warm-up, runs and gaps of every workload. For each run it starts loadgen, waits for its ready file, starts PresentMon with `--process_id <pid> --delay 5 --timed 60 --terminate_after_timed`, and rewrites `bench.json` after each run. Everything lands in `C:\kairo\logs\bench-<stamp>\`. The job survives a dropped SSH connection or a sleeping Mac; `pc bench fetch` collects later.
3. **The elevated side only reads**: `env.json` and `encoder-usage.ps1` before and after the job, the TDR check (`harness-events.ps1`, reused), `pm-stats.ps1` over each CSV through lib.ps1's owner-checked handle, and CSVs fetched gzip+Base64 in 48 KiB blocks. Apart from handing over the job, its one action on the PC is stopping an orphaned `KairoBench` ETW session.
4. **A full A/A takes about 27 minutes end to end** (a 25-minute job), and up to about 53 minutes with one extension per workload [Estimate]. That isn't a "short job" (M1:103), so it waits for the devrun safety stops, including the OLED cover (M1:135), and for agreed test days (M1:35). Until then, only attended rehearsals of about 2.5 minutes run.
5. **`encoder-usage.ps1`** samples PDH `GPU Engine` and names processes from one `[Diagnostics.Process]::GetProcesses()` snapshot. It never reads `.Path`, which enumerates a process's modules with `PROCESS_VM_READ` (CL:9). It prints file names and LUIDs in kairo-display's `0x` + 16-hex form.
6. **`env.json`** is read from the registry, CIM and `powercfg`, all read-only, plus kairo-display's `displays.json`. It is compared before and after each session and stays in `.dogfood/` (M1:87).
7. **The AMD diff script** runs as two calls. It reads `HKCU\Software\AMD`, `HKLM\SOFTWARE\AMD` and the 32-bit view recursively, plus the driver's class key with `--driver`. It redacts path-like strings, prints `path\name: before -> after`, and never refuses.
8. **PresentMon is already installed and pinned** by the bootstrap. Nothing needs downloading. Re-installing the same pinned file is Claude's job between sessions; a version change is a plan change.

---

## 2. What exists, and the rules it sets

### 2.1 How a verb reaches the PC [read]

- **Transport.** `run_ps <verb>` pipes `$PRELUDE` + `remote/lib.ps1` + `remote/<verb>.ps1` into `ssh` (pc:210–223).
  - The remote command line is `"C:\Program Files\PowerShell\7\pwsh.exe" -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -OutputFormat Text -EncodedCommand <stub.ps1, Base64 UTF-16LE>` (pc:144–152). The PC's SSH shell is `cmd.exe /d /c`.
  - Comment lines are dropped so the line fits cmd's 8,191 characters (pc:52, :136–138).
- **The stub.** `stub.ps1` resets user variables to the machine's, limits `PSModulePath` to admin-only folders, then reads **stdin** to its end as UTF-8 and runs it as one script block (stub:14–45). The script itself is the stdin, so a remote script can never wait for a key press (M1:88).
- **Arguments.** The `$Pc` hashtable is built from PowerShell literals the Mac validated first: `pre_str` (printable ASCII, no quote), `pre_int` (at most 6 digits), `pre_bool`, `pre_list` (pc:178–207). Blobs travel as Base64 strings, for example `JobB64` (pc:684–687) and `LocalB64` (pc:367–373, rdoc:10–20).
- **Output.** On the Mac, awk turns control characters into spaces. The **last** `@@KAIRO-EXIT <n>` line is the exit code; `@@KV key=value` lines go to a file that `kv_get` reads (pc:224–233, :247–249). Untrusted text is printed indented, so it can't forge a marker (lib:202–206).
- **Fetching files.** `harness-fetch.ps1` reads through the owner check and sends `@@KV out=<Base64>`, at most 1 MiB, on main. On the harness branch it can also send 48 KiB `@@KV clip=` blocks with `clip_sha256` (hfetch:61–96). The Mac decodes beside the target and moves the file into place (pc:814–831).
- **Exit codes.** 0 ok, 1 failed, 2 usage, 3 PC busy, 4 busy state unknown, 5 still running, 6 stopped by devrun, 255 ssh failed (pc:330–332). The harness branch adds 7 for a display-driver reset or an unexpected reboot (hevents:37–40).
- **The busy rule.** `Assert-PcIdle` reads `C:\kairo\state\watch.json`. The PC is busy on input in the last 600,000 ms, an unknown input time or a fullscreen foreground window; a file older than 5 s means UNKNOWN. It refuses with 3 or 4 unless `$Pc['Force']` (lib:283–370; core/watch.rs:11–15, :55–57). `--force` needs the owner's OK in this session (pc:6–8). On the harness branch, `FullscreenRefuses` refuses a fullscreen foreground window even with `--force` (libH:355–366).
- **Files in the owner's folders.** The elevated session never creates, renames or deletes anything in `logs\`, `state\` or `build\`. It reads a file there only through `Open-OwnedFile`: a disk file whose owner is the owner's SID, checked on the open handle (lib:92–137, :163–170; M1:81).

### 2.2 Jobs [read]

- **Schema** (core/job.rs:47–68; mirrored in pc:603–619 and lib:508–559). Unknown fields are refused.

  | Field | Rule |
  |---|---|
  | `id` | `^[A-Za-z0-9_-]{1,64}$` |
  | `cmd` | non-empty list of strings, no control characters |
  | `cwd` | null (meaning `C:\kairo`) or a folder |
  | `max_seconds` | 1 to 86,400 |
  | `abort_on_input` | default true |
  | `window` | `hidden` (default, `CREATE_NO_WINDOW`) or `visible` |
  | `priority` | `normal` (default) or `below_normal`. Timing work needs `normal` (core/job.rs:31–40; P13 at CB:54) |

- **Start.** `Start-KairoJob` re-checks the job, refuses while `\Kairo\Run` is running, writes `C:\kairo\jobs\next.json` atomically into that admin-only folder, and runs `schtasks /run /tn \Kairo\Run` (lib:561–598). The task is Interactive, Limited, priority 4, with no time limit and `IgnoreNew` (boot:1887–1889).
- **What `kairo-devrun run` does.**
  - It refuses session 0 (`not_interactive`). It takes the mutex `Global\Kairo.Devrun.Run`; a second job exits 5 and writes no result (devrun/run.rs:27, :54–97).
  - It deletes the old result, sends the job's stdout and stderr to `<id>.out.log` and `<id>.err.log`, and gives the job a NUL stdin (run.rs:152, :180–211).
  - It starts the program suspended inside a kill-on-close job object and sets the priority class explicitly (devrun/spawn.rs:51–72; platform.rs:122–187; run.rs:198–204).
  - It polls every 100 ms. Past `max_seconds` the reason is `timeout`. If the job asks, a change in the `GetLastInputInfo` tick since the start is `input` (core/job.rs:187–193; run.rs:222–247). Either way `TerminateJobObject(…, 1)` stops the whole tree (run.rs:249).
  - A job ends normally when its first program has exited and no process is left in the job (run.rs:236–238).
- **Result.** `C:\kairo\logs\<id>.result.json` is written through a `.tmp` file and a rename: `{id, exit_code (u32 or null), aborted, reason: exited|timeout|input|invalid_job|not_interactive|spawn_failed|internal_error, started_utc, ended_utc}` (core/job.rs:196–259; run.rs:300–308).
- **Waiting.** `Wait-KairoJob` polls the result every second and accepts only one whose `started_utc` is no earlier than the hand-over minus 2 ms. It gives up 5 s after the task stops running (20 s if it was never seen running) and prints progress every 60 s. An optional `StopWhen` block runs every 2 s and ends the task with `schtasks /end` (lib:606–656). `run.ps1` maps an aborted result to exit 6 and a job still running to 5 (run.ps1:27–64).
- **Pruning and the devrun log.**
  - devrun prunes old job logs by id prefix: the 10 newest `build-`, 3 `cli-` (devrun/run.rs:116), and on the harness branch 5 `harness-`, counting only ids with a timestamp (devrunH/run.rs:130–136).
  - On main, devrun logs the whole `cmd` (run.rs:98–102). The harness branch logs only the program's file name and its argument count (devrunH/run.rs:98–122). The bench job's arguments name a monitor's device path, so the bench needs that change (PR #34) first.
- **Not built yet.** devrun doesn't stop on a game start or another fullscreen window, restore displays and audio, keep the PC awake or cover the OLED. Its input stop also ignores game controllers (M1:103, :135).

### 2.3 The harness pattern, the newest model [read]

`pc harness capture` (pcH:937 onwards) is what the bench copies:
- **Job file in `.dogfood/`.** The job file goes to `.dogfood/jobs/<id>.json` with umask 077, since it names a monitor (pcH:1031–1046; M1:76).
- **Monitor by alias.** The monitor comes from `.dogfood/pc/monitors.json` by alias. The OLED is refused, whether by its own alias or by any alias with the same device path (`JQ_CAPTURE_MONITOR`, pcH:880–888).
- **Before, during and after.**
  - Read-only notes come first (`harness-pre.ps1`).
  - The job goes through `run.ps1` with `FullscreenRefuses`, and `Send-KV started` reports when it began (runH.ps1:8, :14).
  - Afterwards the System log is read for events 4101 (`Display`) and 41 (`Microsoft-Windows-Kernel-Power`) (hevents:18–40). A `tdr-pending` marker on the Mac makes a check that never ran block the next capture (pcH:904–928, :1076–1088).
  - The reports come back into `.dogfood/harness/<stamp>/`.
- **The orchestrator.** `kairo-harness run`:
  - makes `C:\kairo\logs\<run-id>\` first and refuses one that exists;
  - holds `ES_DISPLAY_REQUIRED` and sets `SetErrorMode`;
  - wants 2 GiB free;
  - refuses a second `ffmpeg.exe` by counting image names in a Toolhelp snapshot, which opens no process;
  - starts its children with `CREATE_NO_WINDOW`, then kills and reaps every child before it exits;
  - prunes to the newest 5 run folders without following links;
  - writes its report on every way out (harness/run.rs:37, :48–96, :163–215; harness/platform.rs:26–103; harness/prune.rs:1–60).

### 2.4 What this means for the bench

- **Where things run.** loadgen opens a window, so it starts only through `\Kairo\Run` (CL:24; M1:97). Everything the job writes goes to `C:\kairo\logs\` (M1:75, :98). One job runs at a time (M1:98; the devrun mutex).
- **Limits.** The job has `max_seconds`, `abort_on_input: true` and `priority: normal`.
- **Screens.** Patterns go on the 180 Hz monitor, the place answered for M1's loadgen (M1:40, :99). The OLED is never the bench monitor, and it is black while unattended (CL:26).
- **Banned mechanisms.** No synthetic input (CL:10): PresentMon's `--scroll_indicator` calls `SendInput` (PM:MainThread.cpp:42–77), so it is never passed.
- **Privacy.** No raw paths or non-game window titles in any log (CL:16). Device paths appear only in `.dogfood/` job files.

---

## 3. Where PresentMon runs

| | **S**: in the elevated SSH session (M1:97, R13:542–549, CB:1072–1074, :1174) | **J**: inside the bench job, a child of `kairo-harness bench`, as the owner |
|---|---|---|
| CSV location | Not `logs\`: elevated sessions never create files there (M1:81), and PresentMon's `_wfopen_s(…, "w,ccs=UTF-8")` truncates through a link (PM:CsvOutput.cpp:1376–1390). M1:48 dropped `C:\kairo\bench\`. The only clean option is `--output_stdout`, which keeps the CSV in the elevated process's memory (PM:CommandLine.cpp:528–561) | `C:\kairo\logs\bench-<stamp>\<w>-<nn>.csv`, created by the owner, as M1:75 already says. The elevated side reads it through `Open-OwnedFile` |
| Lining up with loadgen | Across sessions: poll the owner-written ready file through the owner check, then start PresentMon with the PID from it. `Wait-KairoJob` polls once a second, so either one job per run (14 jobs) or two sequencers that must agree | Local: start right after `ready.json`; the PID is known from the spawn. No SSH in the loop |
| SSH drop or Mac sleep | Kills the sequencer. PresentMon either dies, leaving an orphaned session, or records into a broken pipe [Unverified: whether Win32-OpenSSH ends the session's children]. That run and the rest of the session are lost | No effect: the job runs to its end, and the results wait for `pc bench fetch` |
| Owner input | devrun kills the loadgen job; PresentMon, outside it, has to be stopped by the script | devrun kills loadgen and PresentMon together; the ETW session is left orphaned (§6) |
| Privileges | Elevated PresentMon enables `SeDebugPrivilege` (PM:MainThread.cpp:266–267; PM:Privilege.cpp:27–68), though it still opens processes only with `PROCESS_QUERY_LIMITED_INFORMATION` (PM:OutputThread.cpp:303) | Limited token: `SeDebugPrivilege` fails, and PresentMon prints a warning and continues (PM:MainThread.cpp:266–277). Processes "started on another account" show as `<unknown>`, but loadgen is the owner's, and `--process_id` filters by PID (PM:MainThread.cpp:322–325; PM:OutputThread.cpp:110–145). An access-denied Kernel-Process provider is tolerated (PM:PresentMonTraceSession.cpp:813–819) |
| Contract | Allowed by M1:97, but a CSV location must be added | Within M1:75 and :97; M1:74's PresentMon row gains "inside the bench job" |
| Evidence it works | Sketches only, never run | M0 exit 4 passed: "PresentMon recorded 2 seconds with your limited token" (M0:166). That was an S4U task (doctor:438–500); an Interactive task's filtered token is [Unverified] (§17 risk 1) |

**Decision: J** [proposed]. **Fallback**, if the first rehearsal shows J can't record (PresentMon exit 6 "access denied", or empty GPU, `FrameType` or `MsVideoBusy` columns): S with `--output_stdout`. The CSV then streams to the Mac, the statistics are computed in memory, and nothing is written on the PC.

---

## 4. The bench job: `kairo-harness bench`

### 4.1 Job file (`.dogfood/jobs/bench-<stamp>.json`) [proposed]

```json
{"id":"bench-<yyyyMMdd-HHmmss>","cmd":["C:\\kairo\\bin\\current\\kairo-harness.exe","bench",
 "--run-id","bench-<yyyyMMdd-HHmmss>","--monitor","<180 Hz device path>",
 "--workloads","alu,bw","--runs","6","--warmup","120","--delay","5","--seconds","60","--gap","30"],
 "cwd":"C:\\kairo\\logs","max_seconds":2400,"abort_on_input":true,"window":"hidden","priority":"normal"}
```

- **Id.** `--run-id` repeats the job id and names the folder, as harness `run` does (R16:780–789). The `bench-` prefix plus a UTC stamp lets devrun prune by time: add `("bench-", 5)` to `PRUNED` (devrunH/run.rs:130).
- **Time limit.** `max_seconds` is the sum of the phase limits (§4.2) plus slack. The Mac computes it, as `phases::max_seconds` does for the harness (harness/phases.rs:87–91).
- **Rehearsal variant.** `--workloads alu --runs 2 --warmup 20 --seconds 20 --gap 10`, with `max_seconds` 240.

### 4.2 Phases and limits [proposed]

| Phase | What | Limit | Typical [Estimate] |
|---|---|---|---|
| 0 setup | Make `bench-<stamp>\` (exists → exit 2). Prune to the newest 5 real `bench-<stamp>` folders, never following a link. `SetErrorMode`, hold `ES_CONTINUOUS + ES_DISPLAY_REQUIRED`, at least 2 GiB free. Refuse (exit 3) if a process named `PresentMon*.exe` or `kairo-loadgen.exe` runs: image names from Toolhelp, as `count_processes_named` (harness/platform.rs:77–103). Check that PresentMon and loadgen exist. Resolve the monitor with kairo-display (refresh, HDR, adapter LUID); the Mac has already refused the OLED. Stop a leftover session with `PresentMon-2.6.0-x64.exe --session_name KairoBench --terminate_existing_session`, which exits 0 when it stopped one and 7 when there was none (PM:MainThread.cpp:244–253). Write `bench.json` with the plan | 15 s | 2 s |
| 1 warm-up, per workload | loadgen `--seconds W+30 --stop-on-stdin-eof --ready ready-<w>-warmup.json`. Wait for the ready file, wait W s, close stdin, wait for the exit, then kill | ready 20 s, end 10 s | W + 3 s |
| gap | Nothing runs | G | 30 s |
| 2 run n (n = 1…N) | §4.3 | ready 20 s; PresentMon D + T + 25 s; loadgen end 10 s | ≈ 70 s |
| gap after each run and between blocks | Nothing runs | G | 30 s |
| 3 report | Final `bench.json`, one summary on stdout | — | <1 s |

For 2 workloads × 6 runs: 15 + 2 × (20 + 120 + 10 + 30 + 6 × 120 + 5 × 30) = 2,115 s of limits, so `max_seconds` is 2,400.

The 2-minute warm-up is per workload [proposed]. M1:29 says "a 2-minute warm-up" and R13:830 says "run the workload once for 2 minutes", but ALU and bandwidth loads reach different clocks and temperatures.

### 4.3 One run, step by step

1. **Start loadgen**: `C:\kairo\bin\current\kairo-loadgen.exe --monitor <path> --mode alu|bw --seconds <D+T+20> --stop-on-stdin-eof --ready ready-<w>-<nn>.json`, with stdin piped, `CREATE_NO_WINDOW`, stdout to null and stderr to `loadgen-<w>-<nn>.err`. `--seconds` is only a safety cap; the orchestrator ends loadgen through its stdin once PresentMon has finished, so loadgen always covers the whole recording window.
2. **Wait for its ready file** (at most 20 s, polling every 50 ms). Check the pid against the child's, the adapter LUID against the monitor's adapter (M1:66), the mode (borderless), flip-discard, allow-tearing and interval 0.
3. **Start PresentMon**, with stdin null, stdout to `pm-<w>-<nn>.out`, stderr to `pm-<w>-<nn>.err` and `CREATE_NO_WINDOW`:
   ```
   C:\kairo\tools\presentmon\PresentMon-2.6.0-x64.exe
     --process_id <loadgen pid> --output_file C:\kairo\logs\bench-<stamp>\<w>-<nn>.csv
     --session_name KairoBench --stop_existing_session
     --delay 5 --timed 60 --terminate_after_timed
     --no_console_stats --qpc_time --track_frame_type --track_gpu_video --no_track_input
   ```
   - **Flags.** M1:74's four flags, plus:
     - `--no_console_stats`; without it PresentMon adds the flag itself and prints a warning (PM:CommandLine.cpp:613–615).
     - `--no_track_input` [proposed]. Without it, PresentMon traces Win32k keyboard and mouse reads (device type and time only, PM:PresentMonTraceConsumer.cpp:1782–1796) and writes `MsAllInputToPhotonLatency` and `MsClickToPhotonLatency`. The bench needs neither, and the owner's activity doesn't belong in its data.
   - **The recording window.** The ETW session starts at once (PM:MainThread.cpp:331). Recording starts when the delay timer fires (:176–181, :401–405), and the `--timed` timer starts with the recording (:121–123). So the recorded window is [start + 5 s, start + 65 s]. Then PresentMon stops recording, stops the session and closes the CSV (:183–188, :431–447).
   - **Safety of `--stop_existing_session`.** Setup has checked that no PresentMon process runs, and devrun's mutex rules out a second bench job.
4. **Wait for PresentMon to exit**, at most D + T + 25 s. If it hasn't, run `… --session_name KairoBench --terminate_existing_session`: stopping the session makes `ProcessTrace` return, which posts the exit (PM:ConsumerThread.cpp:30–38), so the CSV is still closed properly. After 5 more seconds, kill it. Mark the run `stopped_early`.
5. **Close loadgen's stdin**, wait at most 10 s, then kill.
6. **Record the run in `bench.json`** (temporary file and rename):
   - PresentMon's exit code;
   - whether the CSV exists and its size. PresentMon creates the CSV at the first recorded frame (PM:CsvOutput.cpp:1376–1390), so "no CSV" means no frame was recorded;
   - stderr warnings, classified: "ETW buffers were lost", "ETW events were lost" and "overflowed present events" (PM:MainThread.cpp:448–462) set `etw_loss`. The expected "requires elevated privilege" warning (:272–276) is ignored;
   - whether loadgen was still running when PresentMon ended.
7. **Gap.**

The job computes no statistics: `pm-stats.ps1` is the one source of them (§8).

### 4.4 Never passed

| Flag | Why |
|---|---|
| `--restart_as_admin` | `ShellExecuteEx "runas"` puts a UAC prompt on the owner's desktop (PM:Privilege.cpp:96–155; CL:42) |
| `--scroll_indicator` | Sends Scroll Lock with `SendInput` (PM:MainThread.cpp:42–77; CL:10) |
| `--hotkey` | `RegisterHotKey`, and a new CSV per toggle (PM:CsvOutput.cpp:133–136) |
| No target at all | Every program that presents frames would be listed by its exe name (PM:OutputThread.cpp:127–130) |
| `--date_time` | Writes local wall-clock time into every row |
| `--multi_csv`, `--etl_file` | Not needed |
| The default session name `PresentMon` | Adrenalin ships its own PresentMon (R13:540) |

### 4.5 Exit codes and stdout [proposed]

The exit codes follow harness `run`'s (harness/phases.rs:6–20):
- 0: a report exists and every planned run recorded a CSV;
- 1: a step failed or timed out, and the report says which;
- 2: bad command line, or the folder exists;
- 3: refused (monitor not found, disk, another PresentMon or loadgen running, a missing program);
- 5: internal.

Stdout becomes the job's `.out.log`, which `pc` prints. It holds phase names, run counts, exit codes and file names only: no device path, LUID or model name.

### 4.6 `bench.json` [proposed]

Written as the owner, a local file:
- **Top level:** `kairo_bench: 1`, `run_id`, `started_utc`, `ended_utc`, `outcome` (`complete`, `partial` or `failed`), `exit_code`.
- **`plan`:** workloads, runs, `warmup_s`, `delay_s`, `timed_s`, `gap_s`.
- **`monitor`:** `refresh_mhz`, `hdr`, `adapter_luid`, `size`.
- **`presentmon`:** file name, session name, `leftover_session_stopped`.
- **`blocks[]`:** `{workload, warmup: {ok, seconds}, runs: [...]}`. Each run holds `{n, csv, csv_bytes, pm_exit, etw_loss, stopped_early, loadgen_exit, loadgen_alive_at_pm_end, elapsed_s}`.

No device paths or names.

### 4.7 What the bench needs from loadgen (for the loadgen part)

- **Command line.** `--monitor <device path>`, `--mode alu|bw`, `--seconds S` as a cap, `--stop-on-stdin-eof` and `--ready FILE`.
- **Ready file.** Written after the first present, like the testbed's (testbed/ready.rs:13–37): `pid`, `adapter_luid` (`0x` + 16 hex digits), `refresh_mhz`, `mode`, the workload's parameters, the swap-chain flags and the first present's QPC time.
- **Window.** Shown without activation (`SW_SHOWNOACTIVATE`, as testbed/platform.rs:298), unless the rehearsal shows that independent flip needs focus (§17 risk 2).
- **Power.** The EcoQoS opt-out (testbed/platform.rs:68–86).
- **Exit codes** like the testbed's (R16:405).

---

## 5. `pc bench` on the Mac and its remote scripts

### 5.1 Verbs [proposed]

```
pc bench aa [--monitor 180hz] [--workloads alu,bw] [--runs 6] [--rehearsal] [--no-extend] [--fetch-csv] [--force]
pc bench fetch STAMP [--csv]     # after a dropped connection; refuses while the job runs
pc bench cleanup                 # stop an orphaned KairoBench session; never refuses
pc bench env                     # env.json alone, read-only
pc amd-diff before | after [--label NAME] [--driver]
```

### 5.2 Steps of `pc bench aa`

1. **Mac.** Take `.dogfood/bench/.lock` with `mkdir`; a lock whose PID has gone is stale. Run any pending checks first: `tdr-pending` and `cleanup-pending` markers under `.dogfood/bench/*/`, as `harness_pending_checks` does (pcH:904–928). An exit 7 stops everything.
2. **Mac.** Map the alias to a device path from `.dogfood/pc/monitors.json`, refusing the OLED as `JQ_CAPTURE_MONITOR` does (pcH:880–888). Write the job into `.dogfood/jobs/bench-<stamp>.json` and check it with `JQ_JOB_CHECK` (pc:603–653).
3. **PC, `bench-pre.ps1`** (read-only; never refuses):
   - `env.json` (§10) and a 3-second `encoder-usage` sample (§9);
   - PresentMon's SHA-256 against the pin, as doctor:678 checks it; a mismatch stops the session;
   - any process named `PresentMon*`;
   - `logman query KairoBench -ets` [M]: a leftover session is reported, and the job's setup stops it;
   - the state of `\Kairo\Run`.

   The data comes back as `@@KV env=…` and `@@KV encoder=…` in Base64. The Mac keeps `env-before.json` and `encoder-before.json`.
4. **PC, `bench.ps1`** (modelled on build.ps1): `Assert-PcIdle 'bench' $true` (fullscreen refuses even with `--force`), `Start-KairoJob`, `Send-KV started`, then the wait in §5.3.
5. **TDR check.** The Mac writes `tdr-pending`, then the PC runs `harness-events.ps1`, with its id rule widened from `^harness-…` (hevents:9) to accept `bench-<stamp>`. Exit 7 is a P0: stop all PC work and report (M1:214).
6. **PC, `bench-fetch.ps1`.** Fetch `bench.json`, run `pm-stats` over each recorded CSV (§8), and with `--fetch-csv` send the CSVs (§7.3).
7. **PC, `bench-pre.ps1` again**, giving `env-after.json`. The Mac compares the fields listed in §10. A mismatch means "the environment changed during the session, so this session doesn't count" (M1:217).
8. **Mac, the decision.** Per workload: the run count, mean `avg_fps`, CV (sample SD ÷ mean), present-mode shares and ETW-loss flags. With CV above 1%, run one extension job for that workload (`--workloads <w> --runs 6`, with its own warm-up) unless `--no-extend`. Above 1% still at 12 runs: stop and report "fix the setup", with `env.json`'s suspects (thermals, background CPU, power plan; R13:831) [proposed].

### 5.3 Waiting

- **How long.** Up to `max_seconds + 30`. The result file is polled every second; every ~10 s `bench.json` is read (`Read-JsonFile` → owner check, lib:264–275) to print "alu run 3/6 recorded (2.4 MB)" [proposed].
- **The fullscreen stop.** Every 2 s, `StopWhen` calls `Stop-KairoRun` when a fullscreen window is in front whose exe isn't `kairo-loadgen.exe`. `watch.json` names only fullscreen programs (core/watch.rs:27–30). The stop applies even with `--force`: that flag is the owner's OK to start despite recent input, never to run over a game.
- **Input** is devrun's (`abort_on_input`). The fullscreen stop holds only while the SSH session lasts, as for builds (M1:103).
- **Cleanup.** After `Stop-KairoRun` or an aborted result, the waiter runs the cleanup in §6 straight away.

### 5.4 Exit codes

- `pc bench aa`: 0 complete; 1 failed; 2 usage; 3 busy or a fullscreen window at the start; 4 busy state unknown; 5 still running when the wait ended; 6 stopped (input, time limit or the fullscreen stop); 7 P0.
- `pc bench fetch`: 0 fetched; 1 failed; 5 the job is still running.

### 5.5 Never two sessions

| Level | Guard |
|---|---|
| Bench jobs | `\Kairo\Run`'s `IgnoreNew`, `Start-KairoJob`'s refusal (lib:572–574) and devrun's global mutex (devrun/run.rs:27, :80–97) |
| PresentMon processes | Setup refuses while any `PresentMon*.exe` runs, AMD's overlay copy included (R13:540) [proposed] |
| ETW sessions | Only the name `KairoBench` is used. A leftover is stopped only when no PresentMon process runs |
| The doctor's test | `pc doctor --test-presentmon` (session `KairoDoctor`, doctor:453–460) skips only while the owner is busy (doctor:450–452). Add: skip while `\Kairo\Run` runs, since two consumers of the DxgKrnl events would perturb a run [proposed] |
| The Mac | `.dogfood/bench/.lock` |

---

## 6. Failures and recovery

| Event | What happens | Recovery |
|---|---|---|
| **Owner input** | devrun's `TerminateJobObject` kills the orchestrator, loadgen and PresentMon; result `input`, exit 6. The CSVs of finished runs and `bench.json`, which is rewritten after each run, stay. The `KairoBench` session outlives its killed controller: PresentMon stops it only on a clean exit (PM:MainThread.cpp:150–168, :441). It keeps the DxgKrnl, DWM and Win32k providers enabled and holds 256–1,024 buffers of 64 KB, 16–64 MiB (PM:PresentMonTraceSession.cpp:496–500; nonpaged pool by default [M]), while the owner plays | `pc bench` stops it at once while still connected: `logman stop KairoBench -ets` [M], elevated, this one name only, and only when no PresentMon process runs. Otherwise the next bench setup does. A completed workload block (warm-up plus N runs) is kept and counts [proposed]; the A/A needs both blocks |
| **Fullscreen window in front** (a game) | `StopWhen` → `Stop-KairoRun` (lib:600–604): no result file is written, and the session is orphaned as above | As above; recorded as `stopped: fullscreen` |
| **SSH drop or Mac sleep** | The job continues. The Mac sees ssh 255, or a script with no exit marker | `pc bench fetch <stamp>` later: it refuses while the job runs and runs the pending TDR check first. `env-after` is taken late but still compared. The fullscreen stop is gone in the meantime. On the Mac, check `pmset` before a long verb and wrap it in `caffeinate -i` [proposed] |
| **`max_seconds` reached** | devrun `timeout`, exit 6 | Report it; clean up the orphaned session |
| **PresentMon exit ≠ 0** | 6: the session couldn't start, e.g. access denied (PM:MainThread.cpp:361–383); 1: a manifest or command-line failure (:229–240) | Stop the block. After the rehearsal, fall back from J to S |
| **No CSV** | No frame was recorded: loadgen died, or the PID was wrong | The run is marked failed |
| **An ETW-loss warning** | The run is flagged | Leave flagged runs out. If it repeats, add `--set_circular_buffer_size 4096` (PM:CommandLine.cpp:304, :470) |
| **loadgen fails** | The ready file times out, or loadgen exits early | Stop the block and report |
| **Display off** | Few frames are displayed (`MsUntilDisplayed` `NA`), so pm-stats flags the run [proposed] | Invalid. Start on "go" with the displays on (R16:773) |
| **TDR or unexpected reboot** | Events 4101 or 41 | P0 (M1:214) |
| **An update during the session** | `env-after` differs | The session doesn't count (M1:217) |

---

## 7. Outputs, privacy, and what comes back

### 7.1 On the PC

- **The run folder.** `C:\kairo\logs\bench-<stamp>\`, created by the owner, holds:
  - `bench.json`, `ready-<w>-warmup.json` and `ready-<w>-<nn>.json`;
  - `<w>-<nn>.csv`;
  - `pm-<w>-<nn>.out`, `pm-<w>-<nn>.err` and `loadgen-<w>-<nn>.err`.

  `<w>` is `alu` or `bw`; `<nn>` runs from 01.
- **devrun's files** sit beside it: `bench-<stamp>.result.json`, `.out.log` and `.err.log`.
- **Pruning.** The newest 5 folders and job logs are kept. The names go into M1's Files table (M1:75).
- **Size.** About 2.5 MB of CSV per run at 125 fps (R13:707 tunes ALU to ~8 ms a frame), up to ~20 MB at ~1,000 fps [Estimate], so 30–240 MB per session.

### 7.2 What the CSV columns reveal [read]

With these flags the default 2.6 header (PM:CsvOutput.cpp:531–668) is:
`Application, ProcessID, SwapChainAddress, PresentRuntime, SyncInterval, PresentFlags, AllowsTearing, PresentMode, FrameType, TimeInQPC, MsBetweenSimulationStart, MsBetweenPresents, MsBetweenDisplayChange, MsInPresentAPI, MsRenderPresentLatency, MsUntilDisplayed, CPUStartQPC, MsBetweenAppStart, MsCPUBusy, MsCPUWait, MsGPULatency, MsGPUTime, MsGPUBusy, MsGPUWait, MsVideoBusy, MsAnimationError, AnimationTime, MsFlipDelay`.

With `--qpc_time` the first time column is `TimeInQPC`, not the `TimeInMs` research 13 lists (R13:557; PM:CommandLine.cpp:633). The file starts with a UTF-8 BOM (`ccs=UTF-8`, PM:CsvOutput.cpp:1382) [M].

| Column(s) | What it reveals |
|---|---|
| `Application` | `kairo-loadgen.exe`, the file name only: PresentMon cuts paths off (PM:OutputThread.cpp:306–312; PM:PresentMonTraceConsumer.cpp:2854–2858) |
| `ProcessID`, `SwapChainAddress` | A PID, and a pointer in loadgen's address space |
| `PresentRuntime`, `SyncInterval`, `PresentFlags`, `AllowsTearing`, `PresentMode`, `FrameType` | How loadgen presents |
| `TimeInQPC`, `CPUStartQPC` | Absolute QPC ticks since boot, so the PC's uptime and, with the file's time, its boot time: a minor PC detail |
| The `Ms…` columns | Frame timings |
| Never | User, account, monitor, adapter or GPU names, or a path |

- With input tracking left on, the CSV would also show whether a key or mouse button was used (not which key); `--no_track_input` drops that.
- `--process_id` keeps every other program out. Without a target, the CSV would list the exe names of everything the owner has running.

The raw CSVs are never committed (CL:18).

### 7.3 Fetch [proposed]

- **`bench-fetch.ps1`**, new, modelled on `harness-fetch.ps1`:
  - refuses a file older than the job's start (hfetch:26–33);
  - takes file names from `bench.json` and requires `^(alu|bw)-[0-9]{2}\.csv\z` or a fixed name;
  - reads through `Read-SharedText`/`Open-ReadFile` with caps: 1 MiB for `bench.json`, 64 MiB per CSV;
  - gzips each CSV in memory (`IO.Compression.GZipStream` into a `MemoryStream`, so nothing is written) and sends 48 KiB `@@KV csv=` blocks with the SHA-256 of the raw CSV and of the gzip. That is one call per CSV, like the clip (hfetch:61–96), so the blocks of two files never interleave in `KV_FILE`.
- **On the Mac.** Check free disk first: the Mac is nearly full, so require 2.5× the size plus 1 GiB, as the clip fetch does (pcH). Decode beside the target, verify, and move into `.dogfood/bench/<stamp>/csv/<w>-<nn>.csv.gz`.
- **Defaults.** The statistics are always fetched. CSVs come with `--fetch-csv`, which is on for the first full A/A as the gate's evidence.

### 7.4 `.dogfood/` vs `docs/`

| Where | What |
|---|---|
| `.dogfood/jobs/` | `bench-<stamp>.json`: it names the 180 Hz monitor's device path (M1:76) |
| `.dogfood/bench/<stamp>/` | `env-before.json`, `env-after.json`, `encoder-before.json`, `encoder-after.json`, `bench.json`, `stats.jsonl` (pm-stats lines), `aa.json` (n, mean, CV per workload), `csv/*.csv.gz`, and the `tdr-pending`/`cleanup-pending` markers |
| `.dogfood/amd-diff/<stamp>/` | The snapshots and diffs (§11) |
| `docs/` | The gate report line (M1:219–221), aggregates and hardware class only: "A/A on kairo-loadgen, RX 9000-series, 180 Hz SDR: ALU 6 runs, CV x%; bandwidth 6 runs, CV y%; Hardware: Independent Flip z%; PresentMon 2.6.0". Also M1's contract rows (file names and flags) and this research. Never a CSV, `env.json`, a model name or a driver version (M1:87, :221; CL:18) |

---

## 8. `pm-stats.ps1` and `ab-stats.ps1` on the PC

- **Where it runs.** `pm-stats` runs in the elevated `bench-fetch.ps1`. The Mac sends it as Base64 in the prelude (`PmStatsB64`), and the script turns it into a script block in memory. So the PC always runs the Mac's current copy, as M1:88 and :92 have scripts arrive on stdin, and nothing lands on the PC's disk. The precedent is run-doctor's `LocalB64` (rdoc:10–20), without its temporary file.
- **How it reads a CSV.** `-Csv PATH` is read through lib.ps1's `Read-SharedText` when that function is in scope, which gives the owner check and a size cap; standalone, it uses `[IO.File]::ReadAllText`.
  - In the elevated case it never calls `Import-Csv` on the path: that reopens the path and would follow a link. It uses `ConvertFrom-Csv` on the checked text.
  - `Read-SharedText`'s `StreamReader` detects the BOM (lib:156).
- **Output.** One JSON line per CSV (M1:85), printed and sent as `@@KV stat=`.
- **`ab-stats.ps1`** takes only numbers (`-A -B`), so it runs as a script block on pm-stats' `avg_fps` values. M1 uses it once as a self-test on A/A data [proposed]. Pairs (1,2), (3,4) and (5,6) of one workload should show a loss near 0 with a CI around 0. The CI's half-width then tells whether 6 pairs can decide M2's gate of ≤3% with an upper bound ≤4% (PLAN:353).
- Their definitions and checks belong to the statistics part of research 17.

---

## 9. `encoder-usage.ps1`

- **Where.** `tools/pc/bench/encoder-usage.ps1 -Seconds N [-Json]` [proposed]. It is sent as a script block to the elevated session by `bench-pre.ps1`, before and after each bench job, and by `harness-pre.ps1` before each capture (R16:865; research 16's PR 9c). It only reads and never refuses.
- **Sampling.** `Get-Counter -Counter '\GPU Engine(*)\Utilization Percentage' -SampleInterval 1 -MaxSamples N` (R13:611, :625; CB:1077).
  - It's a rate counter, so it needs two collections (R13:620); Get-Counter computes each sample over its interval [M].
  - Instances come and go with processes, so the script uses the wildcard and filters in code.
- **Parsing.** Each instance name, lower-cased by Get-Counter (R13:613), is matched against `^pid_(\d+)_luid_0x([0-9a-f]{8})_0x([0-9a-f]{8})_phys_(\d+)_eng_(\d+)_engtype_(.+)$` (R13:612).
  - The engine type is lower-cased with spaces removed.
  - The per-process table keeps types starting with `video`. AMD reports `videocodec`, which includes decoding (R13:614–618; M1:195).
  - Every type seen is counted once in a census (CB:1077; M1:194: "logs every type").
- **Process names, safely.** One `[System.Diagnostics.Process]::GetProcesses()` snapshot is taken before sampling and one after; only `Id` and `ProcessName` are used [M].
  - `GetProcesses` builds every `Process` from one `NtQuerySystemInformation` snapshot (NET:ProcessManager.Windows.cs:47–55; NET:Process.cs:1092–1098). `ProcessName` then returns the snapshot's name without opening the process (NET:Process.cs:315–326 → NET:ProcessManager.Windows.cs:148–155; `ThrowIfExited` is a partial with no Windows body, NET:Process.cs:914).
  - `Get-Process -Id` would open each PID with `PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE` (NET:ProcessManager.Windows.cs:60–83): allowed, but needless.
- **Never `.Path`.** PowerShell defines it as `$this.Mainmodule.FileName` (PS:TypeTable_Types_Ps1Xml.cs:1141–1142). `MainModule` opens the process with `PROCESS_QUERY_INFORMATION | PROCESS_VM_READ` and enumerates its modules (NET:ProcessManager.Windows.cs:794–808; NET:Process.Windows.cs:166–175). That is a module list and a memory read, which CL:9 forbids against a game. `.FileVersion`, `.Company`, `.Description` and `.Product` go through it too (PS:TypeTable_Types_Ps1Xml.cs:214–220, :343–397). Research 13's sketch prints `$p.Path` (R13:631); that line is not copied.
- **Output.** One JSON line, plus a text table for the terminal:
  ```json
  {"kairo_encoder_usage":1,"seconds":3,"samples":3,"elevated":true,
   "types":{"3d":24,"copy":12,"videocodec":4},
   "video":[{"name":"AMDRSServ","pid":5120,"luid":"0x00000000000123ab","engtype":"videocodec","eng":4,"avg_pct":3.1,"max_pct":7.5,"seen":3}]}
  ```
  - **Names:** file names only, since `ProcessName` has no folder (M1:86). A PID gone by the second snapshot prints `(exited)`.
  - **LUIDs** use kairo-display's form, `0x` + 16 hex digits, high part first (display/lib.rs:51–55). So the Mac can map each to "the 180 Hz monitor's adapter" or "the other adapter" through `displays.json`. Both adapters are AMD (M1:66), so vendor ids can't tell them apart.
  - **Where it goes.** The terminal shows only rows with `max_pct > 0`; the JSON goes to `.dogfood/bench/<stamp>/`.
- **Exit codes** [proposed]: 0 sampled; 2 usage; 3 the counter set can't be read (missing on a CI runner, or a localized name, §17 risk 6).
- **Visibility.**
  - Elevated: PDH counters are machine-wide and the session is an administrator's, so other sessions' processes should appear [Inference].
  - The owner's limited token is [Unverified] (CB:1212 item 6; M1:194), and that matters because the engine runs unelevated. It is settled once: run the same script as a short `\Kairo\Run` job (`"C:\Program Files\PowerShell\7\pwsh.exe" -NoLogo -NoProfile -NonInteractive -File C:\kairo\src\tools\pc\bench\encoder-usage.ps1 -Seconds 5 -Json`, with its output in the job's `.out.log`) while the elevated side samples the same seconds, then compare the two PID sets. `C:\kairo\src` is admin-owned (M0:61).
- **Cost.** `Get-Counter` at 1 Hz costs CPU, so in M1 it never runs during a recorded run, only before and after the job [proposed]. M2 samples one B run (R13:835).

---

## 10. `env.json`

Everything is read-only, from the elevated SSH session, in `bench-pre.ps1`. It covers research 13's §9 fields (R13:819–822), whether AMD Software is installed (M1:87) and RAM (PLAN:663).

| Field | Read from | Why |
|---|---|---|
| `windows.display_version`, `build` (`CurrentBuild.UBR`), `edition` | `HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion`, as boot:657–672 and doctor:528–529 read it | R13:820 |
| `reboot_pending` | `Test-Path 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\WindowsUpdate\Auto Update\RebootRequired'` [M] | An update landing during a milestone (M1:217) |
| `uptime_s` | `Win32_OperatingSystem.LastBootUpTime` | Fresh boot vs. long uptime [proposed] |
| `gpus[]`: `driver_version`, `driver_date`, `amd_software`, `pci` | `Win32_VideoController` `DriverVersion`, `DriverDate`, `PNPDeviceID`; the display class key `HKLM:\SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}\00NN` value `RadeonSoftwareVersion` (boot:797–830; the value names are TODO(verify) there). Mapped to LUIDs by matching `VEN_xxxx&DEV_xxxx` with `displays.json`'s ids | R13:820's driver store version. That WMI's version equals DXGI's user-mode version (R13:417–424) is [M] |
| `amd_software`: `installed`, `version`, `basis` | Uninstall entries under `HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\*` whose `DisplayName` starts "AMD Software" [M]; `Test-Path 'C:\Program Files\AMD\CNext\CNext\RadeonSoftware.exe'` [path Unverified, R13:471]; the class key's `RadeonSoftwareVersion` | M1:87 |
| `amd_hints` | `HKLM:\SOFTWARE\AMD\DVR` `InstantReplayEnabled`, `InGameReplayEnabled`; `HKCU:\Software\AMD\DVR` `DvrEnabled`, `DesktopRecordingEnabled`; `HKCU:\Software\AMD\CN\Performance` `EnableMetricsOverlay`. Each is an integer or `absent`, as hpre:9–28 reads them. The names are [Unverified] (R13:472, :374) | PLAN:353's list of features that must be off; the diff script confirms the names |
| `amd_processes` | Names only, from one `GetProcesses()` snapshot: `RadeonSoftware`, `AMDRSServ`, `AMDRSSrcExt`, `amdow`, any `PresentMon*` (R13:473, :540) | Coexistence; a second PresentMon |
| `hags.hw_sch_mode` | `HKLM:\SYSTEM\CurrentControlSet\Control\GraphicsDrivers` `HwSchMode`: 2 on, 1 off, absent = the driver's default [M]. That's the setting, applied at the next boot; the effective state (`D3DKMT_WDDM_2_7_CAPS`, R13:395–415) comes from `wgc-probe`'s capability report (M1:28) once it exists | R13:820, :375 |
| `dx_user` | `HKCU:\Software\Microsoft\DirectX\UserGpuPreferences` `DirectXUserGlobalSettings`: `VRROptimizeEnable`, `SwapEffectUpgradeEnable`, `AutoHDREnable` [M] | Windows' VRR and windowed-game settings |
| `vrr_per_display` | `unknown`: no read-only API was found, and FreeSync is an AMD setting [Unverified] until the diff script finds its value | R13:820 |
| `displays[]`: `alias`, `refresh_mhz`, `hdr`, `size` | kairo-display, through `pc harness displays` at the session's start (`.dogfood/pc/displays.json` with its aliases, M1:77). The bench job also records its own monitor in `bench.json` | R13:820 |
| `power.scheme`, `overlay_ac`, `display_off_ac_s` | `powercfg /getactivescheme`; `HKLM:\SYSTEM\CurrentControlSet\Control\Power\User\PowerSchemes` `ActiveOverlayAcPowerScheme` [M]; `powercfg /q SCHEME_CURRENT SUB_VIDEO VIDEOIDLE`, the AC index in seconds [M] | The power plan is a noise source (R13:831); the screen-off timer research 16 left open (R16:773) |
| `game_mode`, `game_bar_capture` | `HKCU:\Software\Microsoft\GameBar` `AutoGameModeEnabled`; `HKCU:\System\GameConfigStore` `GameDVR_Enabled`; `HKCU:\Software\Microsoft\Windows\CurrentVersion\GameDVR` `AppCaptureEnabled` [M] | R13:828 |
| `mpo` | `HKLM:\SOFTWARE\Microsoft\Windows\Dwm` `OverlayTestMode`; 5 turns MPO off [M] | Present modes |
| `cpu`, `ram` | `Win32_Processor`; `Win32_ComputerSystem.TotalPhysicalMemory` and `Win32_PhysicalMemory` capacity and speed (boot:772–795) | PLAN:663 |
| `background_cpu_pct` | `\Processor(_Total)\% Processor Time` over the encoder-usage seconds | R13:831 |
| `kairo` | `C:\kairo\bin\current` → build id and commit (lib:447–468); PresentMon's SHA-256 against the pin (doctor:77); the FFmpeg `KAIRO-PIN.json` asset (hpre:30–36) | R13:822 |

- **Compared before and after.** `windows.build`, `reboot_pending`, every `driver_version`, `amd_software.version`, `hags`, the power scheme and overlay, the bench monitor's refresh and HDR, and `amd_hints`. Game settings (R13:822) don't apply to loadgen; its parameters go in `bench.json`.
- **HKCU** is the SSH account's hive, which today is the owner's (R13:476; hpre:27). If SSH moves to a separate admin account (M1:172), these reads must use `Registry::HKEY_USERS\<owner SID>`, with the SID of `\Kairo\Run`'s principal; that task was registered by an admin, so it can be trusted [proposed].
- **Privacy.** `env.json` holds PC details (model names, PCI ids, RAM), so it lives only in `.dogfood/bench/<stamp>/` (M1:78, :87; CL:18). The terminal shows a three-line digest: Windows build, driver version, HAGS and power plan [proposed].

---

## 11. The AMD settings diff script

- **Files.** The script is `tools/mac/remote/amd-diff.ps1`, run as `pc amd-diff before|after [--label NAME] [--driver]` [proposed].
- **Two calls** (M1:88).
  - `before` takes a snapshot and returns it as `@@KV snapshot=<Base64 JSON>`. The Mac keeps it as `.dogfood/amd-diff/<stamp>/before.json`.
  - The owner then changes one setting in AMD Software.
  - `after` gets the before snapshot back in the prelude (`BeforeB64`; it travels on stdin, so the 8,191-character limit doesn't apply), takes a new snapshot, prints the differences and returns its snapshot too (`after-<label>.json`).
- **Never refuses, never writes.**
  - There is no `Assert-PcIdle`: the script only reads, and the owner is at the keyboard by design, as for `status` and `harness-pre`.
  - Keys are opened with `[Microsoft.Win32.RegistryKey]::OpenBaseKey(…).OpenSubKey(path, $false)`, which is read-only. There are no `Set-*`, `New-*` or `Remove-*` calls. AMD's settings are never written (R13:465; CB:1066).
- **Scope.**
  - Default: `HKCU\Software\AMD` and `HKLM\SOFTWARE\AMD`, both whole subtrees. They contain research 13's four keys (R13:479) and the hotkeys under `HKLM\SOFTWARE\AMD\DVR` (R13:509). Also `HKLM\SOFTWARE\WOW6432Node\AMD`, the 32-bit view [proposed].
  - `--driver` adds the AMD adapter's class key `HKLM\SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}\00NN` (the one whose `ProviderName` is AMD) and its subkeys. Driver-level 3D settings are said to live there [Unverified], which SP1's checklist needs: Chill, Boost, AFMF, FRTC, Enhanced Sync, RSR (PLAN:353).
  - Limits: depth 8 and 20,000 values. Past them the script stops and says so.
- **Rendering values.**
  - DWORD and QWORD values print as numbers.
  - `REG_SZ`, `REG_EXPAND_SZ` and `REG_MULTI_SZ` print only when they are short and plain: they match `^[A-Za-z0-9 _.,+=#:;()\[\]-]{0,64}$`, contain no `\`, `/` or `@`, and don't contain the account or PC name. Expandable strings are read with `DoNotExpandEnvironmentNames`, so `%USERPROFILE%` is never expanded into a path with the account name in it. Anything else prints as `<str len=N sha256=8hex>`.
  - `REG_BINARY` prints as `<bin len=N sha256=8hex>`.
  - A change still shows, but a save folder such as `C:\Users\<name>\Videos\…` never leaves the PC (CL:16).
- **Printed lines** (M1:88): `HKCU\Software\AMD\DVR\DvrEnabled: 1 -> 0`, `… (absent) -> 1`, `… 0 -> (absent)`, and type changes such as `REG_DWORD 1 -> REG_SZ "1"`, then `N values changed`. The hives print as HKCU and HKLM, never as a SID.
- **The session.** For each of Instant Replay, Record Desktop and the Metrics Overlay (M1:42): `before`, the owner toggles that one setting, `after --label <name>`. Optionally, one more pair checks that toggling back restores the value. About 10 minutes with the owner [Estimate].
  - A toggle that changes nothing in scope gets its pair repeated with `--driver`.
  - If that still shows nothing, the setting isn't in the registry ([Unverified] R13:472), and SP9 notes it.
- **HKCU** is today's SSH account's hive, the owner's (R13:476; hpre:27). The same `HKEY_USERS\<SID>` change as in §10 applies if the SSH account changes.
- **Outputs.** `.dogfood/amd-diff/<stamp>/{before,after-<label>}.json` and `diff-<label>.txt`. Confirmed value names are AMD's, not PC details, so they may go into `docs/`: research 13's correction and the probes in the bootstrap and the harness (M1:184) [proposed].

---

## 12. PresentMon's install and pin [read]

- **Installed in M0.** The bootstrap pins:
  - `$PresentMonUrl = https://github.com/GameTechDev/PresentMon/releases/download/v2.6.0/PresentMon-2.6.0-x64.exe`
  - `$PresentMonSha256 = b2a706bc…88f1af`
  - `$PresentMonDir = C:\kairo\tools\presentmon` (boot:169–174)

  `Install-PresentMon` downloads to `.download`, checks the SHA-256, deletes the file on a mismatch and moves it into place (boot:1447–1460). It ran on 2026-10-23 (M0:114). The doctor checks the hash and the Performance Log Users group (doctor:674–695). `release.json` confirms the asset's digest and size.
- **Who may change it.** `tools\` is admin-only, so nothing running as the owner can swap the file; the owner can still read and run it (M0:61). That's what J needs.
- **Before each session.** `pc bench` checks the SHA-256 in `bench-pre` and refuses on a mismatch.
- **Missing or wrong file.** Claude re-installs the same pinned file over SSH between the owner's sessions, with the bootstrap's own steps, as for the FFmpeg re-pin (M1:163). It stays inside Kairo's folder and involves no spending or login, so it is no owner question.
- **Another version** changes PLAN:308 and the pins in the bootstrap and the doctor: Claude tells the owner and records it in M1's contract. Adrenalin's own PresentMon is never used (R13:540).
- **No `KAIRO-PIN.json`** is needed: the file name carries the version, and two scripts pin its hash.

---

## 13. Time budget, the owner-away rules and test days

| Session | Job | Mac before and after | Total [Estimate] |
|---|---|---|---|
| Rehearsal (1 workload, 20 s warm-up, 2 × 20 s runs, 10 s gaps) | ≈ 1 min 45 s | ≈ 45 s | ≈ 2.5 min; two of them, one per workload |
| Full A/A (2 workloads × 6 runs, 120 s warm-up, 60 s runs, 30 s gaps) | 2 × (120 + 3 + 30 + 6 × 70 + 5 × 30) + 30 ≈ 1,480 s ≈ 25 min | ≈ 2 min (`env.json`, encoder 2 × 6 s, TDR, pm-stats on 12 CSVs, fetch) | ≈ 27 min |
| One extension (one workload, +6 runs, its own warm-up) | ≈ 12 min | ≈ 1 min | ≈ 13 min |
| Worst case (both workloads extended once) | | | ≈ 53 min |
| M2's ABBA ×3, per workload (§14) | 120 + 30 + 12 × 70 + 11 × 30 ≈ 22 min | | ≈ 23 min, plus starting and stopping the engine |

How it fits the rules:
- **Not a short job.** About 27 minutes is far beyond the 1–2-minute jobs M1 runs while the owner is home (M1:103).
  - Rehearsals of about 2.5 minutes run attended, on "go": `--force` with the owner's OK this session, displays on, hands off (R16:773–777).
  - The full A/A waits for two things. First, devrun's safety stops (M1:135): a stop on a game start, display and audio restore, keep-awake, the OLED cover. Second, agreed test days (M1:35).
- **The OLED.** The bench holds `ES_DISPLAY_REQUIRED`, because a dark display means no scan-out and invalid runs, and that keeps the OLED on too. For 25–50 minutes it would show the unchanging desktop, against "black cover when unattended" (CL:26). So the cover must exist first. The 180 Hz monitor shows loadgen's moving load (M1:40).
- **Test-day windows.** A window of at least an hour fits one A/A with an extension (≈ 53 min worst case), but not also the `audio-soak` dry run (≤ 30 min, M1:200). Use separate windows; research 16 recommends two weekday windows of about an hour (R16:947).
- **Daytime only** (M1:37).
- **The start.** The owner says "go, I'm leaving", and Claude starts at once with `--force` while the displays are still on; the input stop stays on. A start after 10 idle minutes would need the displays still to be on, which is open (M1:206).

---

## 14. Hooks for M2's A/B (design now, build in M2)

- **Plans.** M1 builds `--plan aa`. M2 adds `--plan abba3`: 12 runs per workload as A B B A ×3 (R13:833; PLAN:353), with a `condition` per run in `bench.json`. A means the engine isn't running; B means it runs with the buffer armed (R13:834). How the job starts and stops the engine (as a child, stopped through `kairo-cli`, or through `\Kairo\Engine`) is M2's design.
- **Games.** The owner starts a game's built-in benchmark when the beep says so (PLAN:440).
  - The job finds the game's PID by image name in a Toolhelp snapshot, which opens no process, and passes `--process_id`.
  - PresentMon runs as the owner without `SeDebugPrivilege` and opens only `PROCESS_QUERY_LIMITED_INFORMATION` (PM:OutputThread.cpp:303), so J suits anti-cheat games better than S.
  - The first run in any anti-cheat game happens with the owner present (CL:27).
- **Statistics and encoder use.** `ab-stats` runs on pm-stats' numbers (§8). `encoder-usage` samples one B run, timed from `bench.json`'s progress (R13:835).
- **Job length.** One workload per job keeps a game session's job at about 22 minutes.

---

## 15. PR plan (this part)

Branch names follow `m<N>/<slug>`, with Conventional Commits and DCO, squashed after CI (CL:44). Sizes are [Estimate].

| # | Branch | Scope | Needs | PC and owner |
|---|---|---|---|---|
| B1 | `m1/bench-scripts` | `tools/pc/bench/{pm-stats,ab-stats,encoder-usage}.ps1`: standalone, PowerShell 7, ASCII. Windows CI parses all three, runs pm-stats and ab-stats on CSVs generated inside the test (never PC data), and checks that encoder-usage exits 3 cleanly where `GPU Engine` is missing. S ~400 | — | none |
| B2 | `m1/amd-diff` | `remote/amd-diff.ps1`, `pc amd-diff before\|after`, the redactor with tests on the Mac and Linux CI. S ~350 | — | The owner at the PC for ~10 min, toggling three settings while Claude runs the pairs |
| B3 | `m1/bench` | `kairo-harness bench`; devrun's `PRUNED` gains `("bench-", 5)`; `pc bench aa\|fetch\|cleanup\|env`; remote `bench-pre.ps1`, `bench.ps1`, `bench-fetch.ps1`; `harness-events.ps1`'s id rule; the doctor skips its PresentMon test while `\Kairo\Run` runs; M1's contract rows (Files: `bench-<stamp>\` and its names; PresentMon "inside the bench job"; Formats: `bench.json`); the one-off PDH visibility comparison. M ~1,200 | The loadgen PR; PRs #34–#35 (the devrun log change and the harness patterns); B1 | Two attended rehearsals of ~2.5 min. The full A/A after the devrun safety PR and agreed test days |

---

## 16. Owner questions (real decisions only)

1. **Test days** (M1:35, still open). A full A/A takes about 30 minutes, up to about 50.
   - Recommendation: a fixed weekday window of about an hour. The first full A/A goes in it once the runner can stop on a game start and black out the OLED.
2. **Two short bench rehearsals while you're home**, each about 2.5 minutes. You say "go" and keep your hands off the keyboard and mouse. The 180 Hz screen shows the moving GPU load, and the OLED shows your desktop for those minutes.
   - Recommendation: yes, together with another attended session such as gate 2's.
3. **AMD settings check**, about 10 minutes at the PC. You switch Instant Replay, then Record Desktop, then the Metrics Overlay in AMD Software, one at a time when Claude asks. Claude only reads the registry and changes nothing.
   - Recommendation: any evening; AMD Software is back (M1:42).
4. **Instant Replay during M1's A/A.**
   - Recommendation: leave it as you have it. A/A compares identical runs, and Claude records whether it is using the encoder. It needs to be off only for M2's gate pairs (PLAN:353). Nothing to do.

Claude decides these without asking: where PresentMon runs (J), `--no_track_input`, run counts, and whether to fetch the CSVs.

---

## 17. Risks and [Unverified] items

| # | Item | How it gets settled | Fallback |
|---|---|---|---|
| 1 | PresentMon in an Interactive `\Kairo\Run` job records with the owner's filtered token: Performance Log Users kept, and the GPU, `FrameType` and `MsVideoBusy` columns filled [Unverified; M0:166's test was an S4U task; that UAC doesn't filter Performance Log Users is M] | The first rehearsal | S with `--output_stdout` (§3) |
| 2 | A borderless loadgen window that isn't activated gets independent flip [Unverified] | The rehearsal's `present_modes` | Activate once per block (≥ 205 s foreground-lock wait, R16:419) and keep one loadgen per block, each run a separate PresentMon window |
| 3 | An abort leaves `KairoBench` orphaned, with providers enabled and 16–64 MiB of buffers [read] | Cleanup in `pc bench` and in setup; `pc status` could show a hint [proposed] | — |
| 4 | ETW loss at a high loadgen frame rate: a 2,048-present circular buffer [Unverified] | The stderr warnings | `--set_circular_buffer_size 4096` |
| 5 | PDH visibility: elevated [Inference], the limited token [Unverified] (CB:1212 item 6; M1:194) | The one-off comparison (§9) | If the limited token sees only its own processes, the unelevated engine detects Instant Replay from the registry hints (R13:472) until there's a decision |
| 6 | The English counter path works on a localized Windows [Unverified] | The first run (exit 3 if not) | Map the name through the Perflib counter index |
| 7 | `ES_DISPLAY_REQUIRED` wakes a display that is already dark [Unverified] (R16:996; M1:206) | The devrun safety PR | Start on "go" with the displays on |
| 8 | AMD's toggles live in the registry [Unverified] (R13:472) | B2's session | `--driver`; then list AMD's settings folder, file names and sizes only [proposed] |
| 9 | Windows throttles the windowless PresentMon with EcoQoS [Unverified] | ETW-loss flags | Opt the child out with `SetProcessInformation` on Kairo's own child [proposed] |
| 10 | `ProcessName` from `GetProcesses()` opens no process [M; source tag not recorded] | — (no game runs in M1) | — |
| 11 | HKCU is the owner's hive only while SSH logs in as the owner (M1:172) | The ADR for a separate SSH account | `HKEY_USERS\<owner SID>` |
| 12 | CSV size at the bandwidth mode's frame rate [Estimate] | The rehearsal | Fetch statistics only, by default |
| 13 | Win32-OpenSSH ends a session's children on disconnect [Unverified] | — (matters only for S) | — |
| 14 | A driver or Windows update during a session | The `env.json` comparison | The session doesn't count |

---

## Departures from the research sketches

| Sketch | Here | Why |
|---|---|---|
| `C:\kairo\bench\$Tag.csv` (R13:545; CB:1072) | `C:\kairo\logs\bench-<stamp>\<w>-<nn>.csv`, written as the owner | M1:48, :75, :81 |
| PresentMon from SSH, `--process_name` (R13:542–549; CB:1174) | Inside the job, `--process_id` | §3 |
| `encoder-usage` prints `$p.Path` (R13:631) | Names from `GetProcesses()` only | CL:9; §9 |
| Diff script with `Read-Host` (R13:485) | Two calls, `before` and `after` | M1:88 |
| `env.json` published with the summary (R13:843) | `.dogfood/` only | M1:87 |
| — | `--no_track_input` added | CL:16, §4.3 |
| One 2-minute warm-up (R13:830) | One per workload | §4.2 |
