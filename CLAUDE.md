# CLAUDE.md: Kairo house rules
Windows instant-replay clipper, GPL-3.0, PUBLIC repo. The owner play-tests and decides; you build everything.
Read first: docs/PLAN.md, docs/adr/, docs/milestones/<current>.md; before engine work, docs/research/15-implementation-cookbook.md. Gates, never dates.
Current milestone: M1, see docs/milestones/M1.md (M0 closed 2026-10-23)

## Never (anti-cheat; CI enforces)
1. Code in other processes: injection, graphics hooks, CreateRemoteThread, in-context or low-level hooks.
2. Capture other than Windows.Graphics.Capture or DXGI Desktop Duplication.
3. Game handles beyond PROCESS_QUERY_LIMITED_INFORMATION; memory reads; module lists.
4. SendInput, virtual devices, drivers, services, packers, writes to PlayStation pads; elevation, except the
   dev-only elevated test task and the v1.1 elevated input helper once its ADR is approved.
Any anti-cheat warning = P0: stop, remove the mechanism, report.

## Privacy and licences
- Sockets only in crates/kairo-net; new host = ADR + PRIVACY.md + owner approval; no analytics.
- Never log clips, audio, typed keys, non-game window titles or raw paths; crash dumps stack-only, after consent.
- No GPL deps or copied GPL code (x264, x265, libobs, OBS shaders, boxblur, blackframe); FFmpeg = LGPL DLLs.
- Never commit logs, clips, PC details, raw CSVs or keys; docs/ gets aggregates only.
- Never send the owner's name or email to outside services (e.g. as an API User-Agent contact);
  use the repo URL instead.

## The PC (ssh kairo-pc)
- `pc status` first; busy (game, save in flight, owner input <10 min) → work on the Mac.
- GUI, capture and audio start only via the \Kairo tasks; stay in C:\kairo; deploy between sessions; keep 3 builds.
- Jobs have a max duration and abort on owner input or a game start; overnight soaks only after the owner says so.
- Patterns on the 180 Hz monitor; OLED: moving content ≤10 min, black cover when unattended; tones at −30 dBFS.
- First run in any anti-cheat game: practice or unranked, with the owner present.

## Engineering
- Exact pins (windows =0.62.2, wasapi =0.26.0, ffmpeg-next =9.0.0, ffmpeg-sys-next =9.0.1).
- AMF: NV12 only; every option set and read back; H.264 peak ≤50 Mbps until SP7 proves more is safe.
- unsafe only in capture/gpu/encode/audio/platform/input, with // SAFETY:; no unwrap on runtime paths;
  nothing blocks the hotkey thread; capture never stalls on a save.
- Contract first: edit kairo-rpc, regenerate, update fixtures and mock. Media PRs need a green harness smoke.

## Exit rule (M3)
Every working day on Rust capture/encode/audio code counts from the first wgc-probe commit; exclude a day only if
the PC was unavailable or the blocker reproduces with the FFmpeg CLI or a C++ sample, logged that day with evidence.
7 counted days or 14 calendar days after M3 starts → switch to C# unless the owner replies "keep Rust".

## Owner and git
Plain language; questions come with a recommendation. Ask before spending, publishing, UAC prompts, logins,
physical actions, legal or identity choices, security or privacy settings, or PC changes outside Kairo's folders.
Branches m<N>/<slug>; Conventional Commits + DCO + the session's co-author trailer; squash after CI.
