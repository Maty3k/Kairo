# Kairo implementation plan

*2026-10-21 · For Kairo's owner (product calls, play-testing) and Claude Code (builder). Inputs: the owner's 36 decisions (§8) and research 00–15. `docs/research/` holds 00–13, the implementation cookbook 15 and its type-checked prototypes; the name check (14) stays private in the git-ignored `.private/` folder. Research 13 (the owner's AMD rig) overrides earlier reports, except the WGC interval (SP2 decides). **[Uncertain]**/**[Estimate]** come from the research; **[proposed]** marks a default you can change. Not legal advice.*

---

## 0. At a glance

**What Kairo is.** A free Windows app that quietly keeps the last moments of your game in memory. Press **Alt+F8** and the last 30 seconds become a video that plays everywhere (Discord, WhatsApp, phones, editors). Outside games, a tray switch ("screen mode") records the monitor you pick. Kairo also takes screenshots, sorts clips by game, trims them, and makes vertical (TikTok, Shorts, Reels) and Discord-sized copies you paste into a chat. Links that play inside Discord come with the public launch.

**Public promises.**
- No ads and no watermark.
- Your clips never train AI and are never sold.
- No tracking. On its own, Kairo only checks for updates (you can turn this off); it sends crash reports only if you say yes.
- No account needed. An optional account later unlocks share links.

**The stack in two sentences.** A small always-on program (the *engine*, written in Rust) sits next to the clock, notices games and records them with your graphics card's video chip through Windows' official capture features, so it never touches the game, which is what anti-cheat systems watch for (no known bans for this method; Claude still checks each of your games in practice mode first). The window where you browse and edit clips is a separate app (Tauri: a web-style interface in React and TypeScript) that talks to the engine over a private local channel and closes completely while you play.

| ID | Milestone (size) | After it you can… | Gate, in plain words |
|---|---|---|---|
| M0 | Setup (S) | Nothing visible: Claude can work on your PC | A test window opens on your screen from the Mac and reads your key press |
| M1 | Test instruments (M) | Nothing visible: a checker tests recordings | It catches every deliberately broken test video |
| M2 | Reality check (M) | See in numbers how your PC and games behave | Speed cost, 360 Hz smoothness, HDR, sound, hotkeys measured |
| M3 | Engine core (L) | Nothing visible. If Rust stalls ~7 working days, Claude switches the engine to C#; nothing you see changes | 20 test clips in a row are perfect; 19 of 20 saved in under 1 s |
| M4 | Trial build "DD0" (S) | Trial sessions: Alt+F8 → beep → clip | 3 sessions: every press gives a clip that plays on Discord and your phone |
| M5 | Switchover (L) | Turn AMD Instant Replay off; screen mode; Copy for Discord | At most 3% of your frame rate (≤1 point more than AMD's tool), then 20 h of play with no missed clip |
| M6 | Library app (L) | Browse, favourite, search; guided setup | Fast with 2,000 clips; 6 everyday tasks without help |
| M7 | Edit and share (L) | Trim, go vertical, Discord / WhatsApp / TikTok copies | Every Discord copy is under 20 MB and plays in the chat |
| M8 | Hardening (L) | Install and update like any app; Report a problem | Clean install, update, uninstall; 8-hour stress test |
| M9 | Solo sign-off (S) | Decide to invite friends | 40 h of play, no missed clip in the last 20 h |
| M10 | Friends beta (L) | Friends use Kairo | Friends install alone; no unresolved missed clips |
| M11 | Launch prep (L) | Ready for strangers | Windows 10, Store, name check, privacy policy, website |
| M12 | Share links (XL) | Paste a link that plays in Discord | Links play in Discord; optional sign-in works |
| M13 | v1.1 (L) | Controller button; clips over 2 min; manual recording | All pass tests in your games |
| M14 | v2 (XL) | Say "clip that"; captions, music, montages | Rare misfires; you accept the captions |

Stages: M4–M9 = just you (S0); M10 = friends (S1); M11–M12 = free public launch (S2); money (S3) only if people ask for it. Sizes are relative (S < M < L < XL); no dates: a milestone ends when its test passes. AMD Instant Replay stays on as your safety net until M5.

**Who does what.** Claude writes all the code, tests it on your PC over your home network and ships builds between sessions, never while you play: a test stops the moment you touch the mouse or start a game. You play as usual (20+ h a week), press a "flag" key when something feels off, tell Claude in a sentence, and make the product calls. On days you agree, leave the PC on, signed in and unlocked while you're away. Claude asks before spending, publishing or changing your PC outside its folders.

**What happens next.** M0 takes about 1–1.5 hours on your PC (longer if Windows needs a feature update): update Windows, paste one setup command, answer a few questions (§4). Then Claude builds the test instruments alone, and you play a few guided test sessions.

---

## 1. Product

### 1.1 Vision, positioning and public promises

**Vision.** The clipper you forget is running until you need it: it never costs you the game, never misses the play, and its files play everywhere. **Positioning** (research 04): vendor-neutral instant replay, first-class on AMD and Intel; AMD's tool stops at 60 fps without per-app audio, Intel ships nothing, Medal has ads, a watermark and an AI-training sister company, and the NVIDIA app needs RTX 20+.

| Promise or edge | Enforced or proved by | Gate |
|---|---|---|
| No ads, no watermark | No ad/analytics SDK (CI allowlist); exports never draw on video | CI |
| Never trains AI, never sold | Clips leave the PC only as links you create; cloud terms forbid both | M12 |
| No tracking | Sockets only in `kairo-net`; Network activity page; silence test | M8 |
| No account needed | Everything local works signed out | M12 |
| Light | Engine ≤60 MB + buffer [Estimate]; UI closed while playing; §3.2 FPS gate | M5 |
| Sharing, creator tools | Copy for Discord ≤19,000,000 bytes; paste, drag-out, presets; 9:16 layouts, stems, SDR from HDR | M7 |

"Lightweight" goes public only after measurement.

### 1.2 Stages

| Stage | Entry | Exit |
|---|---|---|
| **S0 Solo dogfood** | M4 passed | M9: ≥40 h play since switchover, ≥4 games, ≥3 anti-cheat families, ≥1 HDR game; 0 missed clips in the last 20 h (press detector = hotkey presses = valid files, §2.5); release installer used ≥10 h; 0 open P0/P1; you'd "give it to a friend today" |
| **S1 Friends beta** (3–8) | M9; M8 installer, updater, reporting, privacy policy v1 | M10: ≥10 sessions per tester; ≤1 crash per started 100 sessions, each diagnosed; 0 unresolved missed clips; 0 anti-cheat warnings; self-test passes everywhere; ≥1 NVIDIA PC; ≥half active after 2 weeks. Counted from weekly beta summaries (§2.9); none for 2 weeks = inactive |
| **S2 Free public launch** | M10; M11 and M12 done; launch spend approved | Store, download and share links live; two stable releases without P0 regressions |
| **S3 Monetize, if wanted** | Tracking-free demand signals (downloads, links, requests, hosting bill) + your decision; CRA, PLD and codec-patent review (§5) | Company + company Store account; CRA readiness. Cloud extras only; never the replay, paywalled viewing, ads, watermarks or AI training |

### 1.3 Scope by stage

| Area | S0 (M3–M9) | S1 | S2 | Later |
|---|---|---|---|---|
| Capture | Win11 game window, screen mode, HDR → SDR, 60 fps (120 if M2 allows), H.264 (HEVC/AV1 opt-in), replay ≤120 s, screenshots, DDA fallback | NVENC, QSV ("experimental" until validated) | Win10 22H2 via DDA | Disk buffer, manual recording (v1.1) |
| Audio, triggers | Game, mic, voice chat (off + consent), everything else; stems + mix; keyboard, mouse; cue, pop-up | | Win10 audio "best effort" | Controller + Xbox rumble (v1.1); "clip that" (v2) |
| Library, editor | Grid, favourites, search, filters, storage meter, file info, optional cleanup (off); trim + 9:16 | | | Captions, music, montages (v2) |
| Sharing, shipping | Paste, drag-out, presets; installer + updater | Beta channel; Store private audience | Share links + accounts; Store; website | |
| **Out** | Other GPUs, Win10, Store, links, accounts | Win10, public Store, links, accounts | Controller, disk buffer, recording, voice trigger, v2 editor | Future ideas |

The v1 RAM buffer is capped at **120 s** (Recommended: ≈381 MB typical, ≤612 MB in heavy scenes); longer lengths are greyed out: "needs disk buffer (v1.1)". **Never:** injection or hooks in games, memory reads, synthetic input, drivers, ads, watermarks, analytics, AI training, paywalled viewing.

**Future ideas backlog:** smart auto-highlights (official local APIs only); console and cloud gaming (cloud-window presets, capture cards, console clip import); phone and Stream Deck triggers; YouTube upload; direct posting to TikTok, Reels and X; Discord webhook; public profiles; webcam track (then a facecam layout); HDR10 export and share copies; keyframed vertical pan; stem preview; screenshot annotation; game art; per-game profiles; more languages; macOS; CPU encoding (Q11); noise suppression.

### 1.4 Key user journeys

**J1 First run** (S0: Claude asks in chat; from M6: a wizard). Each screen has a recommended button; "Use recommended for the rest" still shows screen 7.
1. Welcome: the promises.
2. Quality cards (MB per 30 s; RAM typical / up to): **Recommended** 1080p60 H.264; High 1440p60; Smooth 120 fps if M2 passed; Light 720p60; HEVC/AV1 under Advanced.
3. Audio with live meters: Game on; Mic on (off on Bluetooth headsets, which drop game audio to call quality); **Voice chat off** (consent when turned on); Everything else off in games, on in screen mode (Win10 and plan B: off until voice consent). The screen notes that voice a game plays itself (e.g. CS2) is part of Game sound.
4. Hotkeys, press to test: Alt+F8 30 s; Alt+Shift+F8 2 min (RAM shown); Alt+F6 screenshot.
5. Feedback: cue and pop-up on (pop-up off in games if G8 fails).
6. Other recorders → switch-off guide (AMD: "running both roughly doubles encoder load and FPS cost"; Kairo never changes other apps); library folder `Videos\Kairo`, OneDrive warning.
7. Start with Windows: **two equal buttons, nothing preselected** (Store 10.2.8); crash reports Never / Ask each time / Always, none preselected; update checks explained, with a switch.
8. Test clip: Alt+F8 → 5 s of desktop + tone plays back.

- **J2 Play and clip.** Game detected ≤5 s [proposed] → tray red dot → Alt+F8 → cue + "Saved · 30 s" pop-up → verified MP4 in `Videos\Kairo\<Game>\` in <1 s; a failure gives an error cue, never a silent miss.
- **J3 Screen mode.** Tray → Screen mode → Main/Second (remembered); a detected game takes precedence.
- **J4 Library.** Grid by date then game, hover-scrub, sidebar (All · Favorites · Games · Screenshots · Settings); filters: date range, game, duration, type (clip / screenshot / export), size, resolution, favourite; sorting by date, game, size, duration; storage meter, file info, Show in Explorer.
- **J5 Trim and vertical.** Quick (lossless) or Precise; centre crop, split (a box you draw, e.g. the killfeed, over the action) or fit over blur.
- **J6 Share.** **Discord:** Copy for Discord (library, clip view, tray, hotkey) → Ctrl+V. **WhatsApp:** "WhatsApp / phone" preset → Ctrl+V into WhatsApp Desktop, or send it to yourself to get it on your phone. **TikTok/Shorts/Reels:** vertical editor → preset → drag into the TikTok, YouTube Studio or Instagram web uploader, or post from your phone. Copies carry the mix only, voice chat out unless ticked; S2 adds share links.
- **J7 Report a problem.** Describe → preview → **Open GitHub issue**: Kairo saves `kairo-report-<date>.zip` (redacted logs + capability report, never clips), selects it in Explorer and opens a pre-filled issue form (title, description, Windows build, GPU, driver, Kairo version); you drag the zip in. The preview says in bold: "Issues on Kairo's GitHub are public: anyone can read this report." No GitHub account: **Save report file** and send it to the owner (S1) or the support address (S2). **Copy for Claude** stays for the owner.

### 1.5 UX and design system

**Tokens [proposed]:** near-black `#0B0C10`, surfaces `#131318`/`#1A1A21`, borders `#2A2A33`, text `#EDEDF2`/`#A1A1AD`; one accent chosen in M6 (volt `#C6F432`, mint `#3DF5C6` or violet `#7C5CFF`); REC `#FF4D4F` with icon + text; Inter; 4 px grid; short motion, off under reduced motion.
- **Tray:** right-click = native menu, instant and safe mid-game (save, screenshot, screen mode + monitor, pause, Copy last clip for Discord, library, report, quit); left-click = popup window (REC state, last 3 clips, monitor picker).
- **Windows:** main (sidebar, REC pill, search, filters, storage meter, virtualised grid); clip view (native `<video>`); editor (filmstrip, waveform, trim handles, live 9:16 preview, stem toggles); settings (§2.8 groups, "Reset to recommended", a Privacy page with consents and Network activity); onboarding (J1).
- **Accessibility, i18n:** WCAG 2.2 AA, keyboard-complete, forced-colors, 200% zoom, never colour-only, axe-core in CI; i18next with typed keys and a no-literal-string lint, shared with the engine's error keys; English until S2; the brand is one key.

---

## 2. Architecture and technical design

**Implementation detail** lives in `docs/research/15-implementation-cookbook.md`: verified crate APIs, code sketches, dev-loop scripts and the M3 spike checklist. Its §0 lists 22 refinements (P1–P22) that this plan adopts; the ones that change the design are written into the sections below.

### 2.1 Process model

```
login ─ HKCU Run "Kairo" (after consent) │ windows.startupTask (Store)
 └► kairo-engine.exe  CONTROL: per-user · unelevated · interactive session · no web view
     ├ T0: hidden window · tray · RegisterHotKey · WinEvent hooks · cue · pop-up
     ├ tokio: core · pipe \\.\pipe\kairo-<user-SID> · export jobs · db (sole SQLite writer)
     ├ detect · crash server (minidumper) · kairo-net (updates, Sentry)
     ├ kairo-engine.exe --probe   (sequential encoder probe, cached)
     ├ Job Object (kill-on-close): kairo-engine.exe --media
     │    WGC/DDA → QPC pacer → NV12 shader → AMF/NVENC/QSV/MF ─┐ governor, stall watchdog
     │    WASAPI → aligners → mix + limiter → AAC ──────────────┴► ring → mux → verify
     │  + ffmpeg.exe exports (below-normal priority)
     └ Kairo.exe (Tauri UI: main window, tray popup), on demand, outside the job
kairo-cli.exe (dev, over SSH) ── same pipe ──► control
```

- **Per-user, not a service:** session 0 can't capture or receive hotkeys.
- **Control vs media** (research 11): one binary, two roles, so driver and FFmpeg crashes never take down tray or hotkeys. Media sends a 1 s heartbeat and is dumped and restarted after 10 s of silence (the encoder watchdog acts first). M3 is in-process behind a `Pipeline` trait; M5 splits.
- **Safe mode:** backoff 0.5→30 s; ≥3 crashes in 10 min → L1 `h264_mf`, L2 DDA + `h264_mf`, L3 no capture; cleared after 24 h healthy. A dead control is restarted by the UI or `\Kairo\Ensure` (logon + every 1 min; exits at once if control lives); the Store flavour uses `windows.startupTask` only.
- **Single instance, autostart:** per-user mutex + first-pipe-instance flag; a quoted HKCU `Run` value only after your explicit choice, respecting StartupApproved (`auto-launch` 0.6.0 rejected).
- **UI:** the Start-menu shortcut runs `kairo-engine.exe --open`; control spawns `Kairo.exe --from-engine` (main window; frameless tray popup that closes on blur) as its child, so the exclude-Kairo audio rule covers UI sounds; it exits fully when its windows close. A `Kairo.exe` started any other way hands off with `ui.open{waitPid}` and exits (P22).
- **Updates live in control:** `kairo-net` fetches the channel's `latest.json` from GitHub Releases (start + daily), checks its minisign signature (`tauri signer` format), downloads the full NSIS installer while idle, and runs it with `/S` outside the Job Object on "Restart to update" or when idle with no game or save. Not in the Store flavour.
- **Version watch:** a new Windows build, GPU driver store version or AMF runtime version clears the probe cache, re-probes and self-tests (5 s) before arming and re-reads AMD's recorder and hotkey settings; `data/known-bad-drivers.json` maps versions to workarounds.
- **Lifecycle:** lock → pause buffer; display off → pause capture; sleep → stop media.

**IPC (`kairo-rpc`).** A named pipe with DACL `D:P(A;;GA;;;SY)(A;;GA;;;<user SID>)` (the default lets Everyone read), remote clients rejected, identification-level SQOS (a Unix socket for the Mac mock); JSON-RPC 2.0 with a 4-byte big-endian length prefix (StreamJsonRpc's format, so a C# engine needs no custom handler). `engine.hello` negotiates version and features; a major mismatch → "Finish updating". Main calls: `clip.save{bindingId or lengthS, qpc}`, `library.*`, `export.*{requestId}`, `settings.patch{baseRevision}`, `diag.*`, `update.*`; reserved `share.*`, `account.*`; notifications include `clip.saved/failed`, `encoderPressure`, `update.available`. One `rpc_methods!` registry generates the dispatcher, Tauri allow-list, TypeScript (ts-rs 12.0.1), JSON Schema (schemars 1.2.3) and docs; JSON fixtures bind the mock, the Rust engine and any C# engine. Saves carry the hotkey's QPC time; the webview passes clip IDs, never paths.

### 2.2 Capture pipeline

| Situation (decision 13) | Target | Windows 11 | Windows 10 (S2) |
|---|---|---|---|
| Detected game | The game window | WGC window, borderless, no cursor | DDA on its monitor (WGC would show a yellow border) |
| No game, screen mode on | Remembered monitor | WGC monitor, cursor on | DDA |
| No game, screen mode off | Nothing; no RAM used | — | — |
| Window frozen 3 s, black or elevated | The game's monitor + borderless advice | WGC monitor, then DDA | DDA |

Trade-off: window capture keeps notifications and DMs out of clips but needs a freeze fallback.

| Stage | Design |
|---|---|
| WGC | Own ≈250-line module on `windows` 0.62.2 (research 08), not `windows-capture`; free-threaded 2-buffer pool (FP16 in HDR); frames copied and released at once |
| Interval | Per target monitor, a whole multiple of the output fps (research 08: 6.94 ms at 360 Hz = 120/s; 2.78 ms at 180 Hz = 180/s); research 13's cheaper 13.9/11.1 ms (72, 90/s) judder at 60 fps. SP2's cadence gate decides; recomputed on monitor change |
| DDA (M5) | Own ≈200-line module + pointer shader; per-monitor-v2 DPI; polled per tick; re-created on access loss |
| Device, pacer | One D3D11 device on the monitor's adapter (by LUID), shared with FFmpeg, rebuilt on loss. `SetMultithreadProtected(TRUE)` plus one `GpuLock` (a `CRITICAL_SECTION` also handed to FFmpeg through `AVD3D11VADeviceContext.lock/unlock`), never held across `avcodec_send_frame`/`receive_packet`: `windows` 0.62.2 marks `ID3D11DeviceContext` Send + Sync, so the compiler won't catch unsynchronised use (P1). QPC pacer at 60 (or 120) fps on a high-resolution waitable timer: newest frame ≤ tick, else repeat; pts = tick (CFR); a 2–4 slot channel to the encoder thread. The media process opts out of EcoQoS power throttling (P16) |
| Epochs | Fixed output size per session; a new epoch on target change, encoder restart or device loss; saves never span epochs (the pop-up says when one was shortened) |
| Convert | Two-pass HLSL from the published BT.709, BT.2100 PQ and BT.2390 formulas (no OBS code), rendering NV12 into the encoder's frame, `colr` set. **AMF gets NV12 only** (RGB → DCC corruption on RX 7000, AMF #574): a failed self-test → `ID3D11VideoProcessor` NV12 → a last-resort R8/R8G8 render with system-memory NV12 readback (P2) → L1 `h264_mf` |
| HDR → SDR | Curve from SP3: **Natural** (BT.2390 EETF, SDR white ≈80% linear), **Match SDR apps** (clip), **Reinhard** (SDR white ≈75% code value) |
| Probe | Lazy and sequential in the `--probe` process: H.264 only unless HEVC/AV1 is opted in; ≥1 s between sessions, each released first; cached per driver store + AMF runtime version (Sunshine #5385: back-to-back sessions hung an RX 9070 XT). Chain: AMF/NVENC/QSV → `h264_mf` → "no encoder" (Q11) |
| Encode | `bf=0`, 1 s closed IDR GOP with `gop_size` = fps set explicitly (AMF's own IDR default is 240, P3), VBR. **AMD** (research 13 §8; each via `av_opt_set`, rejection = error, read-back logged): `usage=transcoding quality=balanced rc=vbr_peak vbaq=1 preanalysis=0 high_motion_quality_boost_enable=0 preencode=0 bf=0 max_b_frames=0 frame_skipping=0 filler_data=0 coder=cabac profile=high header_spacing=60 async_depth=4`; AV1 adds `latency=power_saving_real_time aq_mode=caq`. H.264 peak ≤50 Mbps (AMF #614 stall) until SP7 proves more is safe. NVENC: driver ≥610 (BtbN) or ≥531.61 (own build); QSV via `av_hwframe_map`, "experimental" |
| Governor | Every 1 s: PDH `\GPU Engine(*)\Utilization Percentage` (`VideoCodec`/`VideoEncode`, all processes) + Kairo's backlog. >85% for 5 s or backlog > `async_depth` → preset down, then fps (≥60 by default), plus `encoderPressure` naming the competitor. No packet for >2 s while frames go in → recreate the session (new epoch, IDR, ring kept). Mid-game exports wait for headroom |
| Ring | Encoded video + all audio, keyframe-indexed; cap = longest binding + post-roll + 1 GOP, in seconds and in bytes at peak bitrate (oldest GOPs go at the byte cap; the UI shows the shorter length); pinned GOPs for saves |
| Screenshots, health | Latest native frame → PNG (HDR: JXR + tone-mapped PNG); freeze, access-loss and black-frame detection; Kairo's windows excluded from capture |

| Preset (no knobs by default) | Video | VBR target/peak | RAM 30 s / 2 min: typical (cap), audio included |
|---|---|---|---|
| **Recommended** | ≤1080p60 H.264 | 25/40 Mbps | 95 / 381 MB (157 / 612) |
| High | 1440p60 | 36/50 (AMF #614) | 137 / 547 MB (195 / 763) |
| Smooth (if SP2 passes) | 1080p120 | 37.5/50 [Estimate] | 142 / 570 MB (195 / 763) |
| Light | 720p60 | ≈12/20 [Estimate] | ≈47 / 187 MB (79 / 309) [Estimate] |
| Custom | Up to native; HEVC 16/25 (1080p), 24/36 (1440p); AV1 12/20, 18/28: on RDNA3 only at 1440p (1080p is coded as 1082 lines), on RDNA4 after SP7's `coded_height`/`clap` check | — | Shown live |

**Save flow.** Hotkey QPC time T → `SaveJob{keyframe ≤ T − length … T + post-roll}`, GOP pinned, cue → packets snapshotted → temp MP4 (`+faststart`, `use_editlist=0`, IDR first, audio from the AAC packet covering the IDR plus one earlier packet, with negative timestamps and no edit list (exact sync: FFmpeg 9.0.2's MP4 muxer then holds video frame 0 for ≤42.7 ms instead of shifting the audio; confirmed by the testbed flash/beep test in M3, P8), only the mix track flagged default and the temp file on the library volume (P9), a JSON tag of library fields without paths or names) → fsync → `<root>\<Game>\YYYY-MM-DD_HH-MM-SS_<Game>.mp4` → **verify** (duration, IDR at 0, tracks, moov first; one retry, then error cue + `clip.failed`) → poster, DB, pop-up. Saves queue; capture never stalls.

### 2.3 Audio

| Source | Capture | Default | Track |
|---|---|---|---|
| **Game** | Process loopback, include-tree on the game's root process (walk up within the install folder; stop at launchers) | On | T2 |
| **Mic** | WASAPI shared, console-role default, mono; `IAudioClientDuckingControl` (research 10); never auto-on for Bluetooth headsets | On (asked) | T3 |
| **Voice chat** | Include-tree on `Discord.exe` + an editable list seeded with `RiotClientServices.exe` [Uncertain] | **Off** + consent | T4 |
| **Everything else** | Plan A: per-process clients minus game, voice apps and Kairo [Uncertain: SP4]. Plan B: exclude-tree on control, labelled "includes voice chat". **Win10:** endpoint loopback hears everything, voice chat and Kairo's cue included, so plan B's rules apply, Kairo plays no cue while it records (the pop-up confirms), and the mix uses it instead of Game | Off in games; on in screen mode (plan B, Win10: off until voice consent) | T5 |
| **Mix** | Enabled stems × gain → limiter at −1 dBFS | — | **T1, default** |

- **Capture, sync, encode:** `wasapi` 0.26.0 behind a `LoopbackMode` enum (its `include_tree=false` means *exclude*); float32 48 kHz; QPC timestamps on the video clock; per-source aligners (rubato 5.0.1 + a PI loop) in `kairo-core`; gaps become silence; `TIMESTAMP_ERROR` is counted, never a restart; AAC-LC 48 kHz (160 kbps; mic 128) on one 1,024-sample grid.
- **Watchdog:** reconnect with backoff; restart on silence while Windows shows the target playing; a seamless A/B swap every 10 min on Win10 and pre-24H2 Win11, 60 min on 24H2+.
- **Consent** (voice chat, plan B, Win10 "Everything else"), shown once: "Recording other people may require their consent where you live. Their voices are saved in the clip, on their own track and in the main mix; copies Kairo makes for sharing leave them out unless you tick 'Include voice chat'." Voice a game plays itself lands in Game (research 10 [Uncertain per game]): onboarding says so, `games.json` flags such games (`game_voice`), and the first save and first share in one show a notice.
- **Players** play only the default track, so share copies are mix-only.

### 2.4 Game detection and metadata

- **Signals:** foreground-window events → `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)` → image path and elevation (access denied = "unknown") → match; a 3 s process poll; heuristic fallback (window covers its monitor + fullscreen + GPU load) → "possible game" [Estimate thresholds].
- **Index:** Steam (`steamlocate` 2.1.1), Epic, GOG, Ubisoft, Battle.net, Xbox, a curated `data/games.json` (exclusive-fullscreen and `game_voice` flags) and "This is a game / Not a game"; launchers are never games. **Never:** module enumeration, memory reads, Discord's app list (its Terms forbid scraping), the restricted `GameList` API. **Metadata v1:** name, launcher, icon, sessions (split after 15 min idle).

### 2.5 Triggers and feedback (staged)

| When | Trigger | Rules |
|---|---|---|
| DD0 (M4) | Alt+F8 → 30 s | `RegisterHotKey`, `MOD_NOREPEAT`; conflicts → "key in use" + alternatives; AMD's keys read from its registry (`HKLM\SOFTWARE\AMD\DVR\*Hotkey` values [Uncertain], P18), since its in-game `amdihk64.dll` means `RegisterHotKey` success proves nothing |
| Beta (M5) | Alt+Shift+F8 → 2 min [proposed]; ≤4 bindings of 5–120 s; Alt+F6 screenshot; mouse side buttons (opt-in) | Mouse polled every 8 ms only while bound and no Kairo window is in front; no low-level hooks; elevated games explained [Uncertain: SP5] |
| Dev/beta | **Flag key Alt+F11** [proposed]; **press detector** | `GetAsyncKeyState` polled every 8 ms for the bound chords, logging QPC time and foreground elevation. Gates need detector presses = hotkey presses = verified files; a mismatch is P0; elevated-foreground time is "unmeasured", never a pass. Not in stable builds (Q17) |
| v1.1 | Raw Input; controller; optional elevated input helper (M13) | GameInput 4.0.302 background Share, 500–800 ms holds; Xbox rumble if background haptics work [Uncertain]; **PlayStation pads read-only** |
| v2 | "Clip that" | sherpa-onnx (Vosk fallback); local, opt-in, games only |

Avoided defaults: NVIDIA Alt+F1/F9/F10/Z, AMD Ctrl+Shift+…/Alt+R/Alt+Z, Win+Alt+G, Medal F8/Alt+F7, Steam F12, PrintScreen, Ctrl+Alt+letter. Alt+Shift chords are tested with your layouts in SP5 (fallback 2-minute key Ctrl+Alt+F8 [proposed]).

**Feedback** (each switchable): the <300 ms cue (never recorded) is primary. The pop-up is created once and hidden when idle, never left up as a transparent topmost window (that can force Composed Flip; Adrenalin 26.10.1 lists overlay-toggle timeouts); it shows ≤1.5 s via `SW_SHOWNOACTIVATE` (`WS_EX_NOACTIVATE|TRANSPARENT|TOOLWINDOW|TOPMOST`), is excluded from capture and is skipped under `QUNS_RUNNING_D3D_FULL_SCREEN`. Rumble in v1.1; no toasts during play.

### 2.6 Library and storage

- **Layout:** `Videos\Kairo\<Game>\`, `Screen\`, `_Exports\`; OneDrive check; app data in `%LOCALAPPDATA%\io.github.maty3k.kairo\` (research 11); never network or cloud folders.
- **Schema sketch** (`rusqlite` 0.40.2, bundled SQLite; control is the only writer):

```sql
games(id, name, exe_path UNIQUE, source, icon_path, is_game, flags JSON)
sessions(id, game_id, mode /*game|screen*/, started_at, ended_at)
clips(id /*ULID*/, kind /*replay|screenshot|export|recording|import*/, parent_id, rel_path UNIQUE, size_bytes,
      duration_ms, width, height, fps, vcodec, encoder, hdr_source, curve, has_voice, game_id, session_id,
      captured_at, trim_in_ms, trim_out_ms, reframe JSON, markers JSON, title, favorite, locked, poster_path,
      sprite_path, deleted_at)
clip_tracks(clip_id, idx, role) · exports(id, clip_id, out_clip_id, preset) · jobs(id, kind, state, request_id UNIQUE)
shares(id, clip_id, url, expires_at) -- Stage C · clip_fts USING fts5(title, game_name)
```

- **Rebuildable** from each MP4's JSON tag; thumbnails and waveforms are idle jobs, paused while gaming; a watcher reconciles Explorer changes; the meter shows space by game plus buffer RAM.
- **Optional cleanup (off):** max size, min free space or max age of unstarred clips; oldest first; never favourite, locked, edited, exported or shared clips; dry run first; deletes go to the Recycle Bin.
- **Video to the UI:** a `kairo-media` scheme by clip ID with `206` responses of ≤2 MiB (research 12), not the asset protocol (tauri#4133); fallback: WebView2 virtual-host mapping.

### 2.7 Editor and export

- **Trim:** Quick = lossless keyframe snap (edit lists only inside Kairo); Precise = GPU re-encode, the export default (Reels rejects edit lists).
- **Vertical 1080×1920:** centre crop; split (a drawn box over the action, remembered per game); fit over a **`gblur`** background (`boxblur` and `blackframe` are GPL-only, so the black-frame check is in-house).
- **Export engine:** bundled LGPL `ffmpeg.exe`, one process per job in the Job Object, below-normal priority, hardware encode, cancellable, idempotent (`requestId`); background jobs pause while gaming.

| Preset | Rule |
|---|---|
| **Copy for Discord** | Target 0.90 × 20,000,000 bytes: total kbps = 0.90 × 20,000,000 × 8 / s / 1000 (30 s → 4,800 = 4,640 video + 160 audio), minus measured mux overhead (≈0.3% [Estimate]); hard cap ≤19,000,000 bytes. Ladder 1080p60 (≥8 Mbps) → 1080p30/720p60 → 720p → 540p; overshoot → retry at 0.9×, then a rung lower [Uncertain: SP7]. Nitro sizes under "More…" |
| WhatsApp / phone | H.264 ≤1080p30 [proposed], AAC 128 kbps; limits from SP8 |
| TikTok / Shorts / Reels | 1080×1920, ≤60 fps, ≤25 Mbps, closed GOP; Reels ≤15 min, ≤300 MB |
| Share copy (S2) | H.264 ≤1080p60, ~6–8 Mbps |
| Original (all tracks), under More… | Stream copy; "includes voice chat; may not play everywhere" when true |

Share copies are H.264 + AAC MP4 with `+faststart`, no edit list, the mix only (voice chat out unless ticked) and no metadata but the title (`-map_metadata -1`). **Plain Copy and drag-out** hand over the original only when it is H.264 + AAC with faststart and has no voice-chat audio; otherwise Kairo first makes a quick share copy (video stream-copied, audio re-mixed without voice chat). Copy uses `CF_HDROP` (~40 lines of Rust, shared with the tray); drag-out uses Kairo's own `SHDoDragDrop` command on Windows (`tauri-plugin-drag` only for the Mac mock), which keeps the plugin's older `windows` crates and open crash issues out of the shipped app (P11).

### 2.8 Settings model

One schema-versioned `settings.json` owned by control (validated, migrated with a backup, written atomically), changed only via `settings.patch` with a revision check; layers defaults → user → per-game; "Reset to recommended" everywhere.

| Setting | Default | Asked at onboarding |
|---|---|---|
| Quality | Recommended | Yes |
| Clip keys / screenshot | Alt+F8 30 s, Alt+Shift+F8 2 min [proposed] / Alt+F6 PNG | Yes |
| Audio game / mic / voice / everything else | On / on (off on Bluetooth) / **off** / off in games, on in screen mode (Win10 and plan B: off until voice consent) | Yes, with voice consent |
| Feedback | Cue 60% + pop-up 1.5 s | Yes |
| Start with Windows | None: explicit choice | Yes |
| Crash reports / update checks | Never until chosen / on, disclosed, switchable | Yes / shown |
| Library root | `Videos\Kairo` | If OneDrive |
| HDR curve; post-roll; mouse buttons | From SP3; 0 s; unbound | No |
| Screen mode; cleanup; jobs while gaming; channel | Off (Main); off; paused; stable | Tray / no |

### 2.9 Diagnostics

- **Logs:** `tracing` JSON lines per process, rotated 10 × 10 MB (`file-rotate` 0.8.0); paths, user and host names redacted; never non-game window titles, typed keys or clip content; per-session metrics (presses, detector presses, verified saves, latency, restarts, encoder recreates).
- **Capability report:** Windows build; GPUs, driver store and AMF runtime versions, VCN count, HAGS; encoder probe; monitors (Hz, HDR, BitsPerColor, SDR white); audio endpoints; conflicts (AMD Instant Replay, Record Desktop, Metrics Overlay via PDH + registry; taken keys); TDRs in the last 7 days (event 4101, `LiveKernelReports\WATCHDOG`); Defender detections on Kairo's paths; Smart App Control state; safe level.
- **"Test my setup":** a 5 s self-test checked by `kairo-verify`'s analysis; this is how capture is verified on friends' PCs.
- **Beta summary** (beta builds, Settings → Diagnostics): counts only, from a local session ledger (sessions, hours, presses, verified saves, crashes, restarts), previewed and sent by the tester via Save report file or Copy.
- **Crash reports (opt-in;** EDPB Guidelines 2/2023 ¶33): Sentry starts only after opt-in; media's native crashes are dumped out of process (`crash-handler` 0.8.1 → `minidumper` 0.11.0 in control). **Dumps are stack-only** (`MiniDumpNormal | MiniDumpWithThreadInfo`; no full memory, heap, data segments, indirect memory or handles), so frame and audio buffers can't be in them; before upload Kairo masks the Windows user name in module paths (same length, `C:\Users\xxxxxx\…`), because Sentry doesn't scrub attachments. "Ask each time" lists files and sizes before Send. `sentry` 0.49.3 with default features off (no sessions or metrics), `server_name` set explicitly (host names are personal data, P15), no minidump storage, IP storage off; PDBs uploaded from CI. Panics call `simulate_exception` and then exit with `EXIT_PANIC`, because abort and nounwind panics are `__fastfail` and skip in-process handlers; the dev PC also keeps WER LocalDumps (P14).

### 2.10 Safety, privacy and security rules

- **Anti-cheat house rules** (decision 14; Appendix A): nothing inside games; WGC/DDA only; query-only game handles; no `SendInput`, virtual devices, drivers or low-level hooks; unelevated (exceptions: a dev-only elevated test task; the v1.1 elevated input helper once its ADR is approved); PlayStation pads read-only. A **CI forbidden-API check** fails on `CreateRemoteThread`, `WriteProcessMemory`, `ReadProcessMemory`, `VirtualAllocEx`, `PROCESS_VM_*`, `SendInput`, hook-installing `SetWindowsHookEx` and in-context WinEvent hooks. Any anti-cheat warning is P0: stop, remove the mechanism, report. Safety rests on the absence of ban reports for out-of-process capture [Uncertain].
- **Pipe, UI, egress:** §2.1's pipe rules; strict CSP; app commands under Tauri's ACL; no shell, filesystem or asset-protocol access; the WebDriver plugin only in test builds. Only `kairo-net` opens sockets (CI-checked), with a host allowlist and a Network activity page; a new host needs an ADR, a `PRIVACY.md` change and your approval.
- **Consent ledger:** `consents.json` records topic, choice, policy version and time; nothing pre-ticked; policy changes ask again.

| What leaves the PC | When | Data → destination | Default |
|---|---|---|---|
| Update check | Start + daily [proposed] | GET of the signed manifest (IP, version, OS build) → GitHub | On, disclosed, switchable |
| Update download | While idle, or on "Restart to update" | Installer ← GitHub Releases | Follows the check |
| Crash report | Only with consent | Stack-only minidump (thread stacks, module names, user name masked), OS/GPU/driver, version → Sentry EU (IP not stored) | Off until chosen |
| Report a problem | Your click | Description + redacted logs + capability report → a public GitHub issue you submit in your browser | Manual |
| Share link (S2) | Your click | Share copy, poster, title, game name, duration, size → Kairo cloud | Manual |
| Sign-in (S2) | Your click | Provider ID, display name, email if the provider gives it → Kairo cloud; OAuth → Discord/Google/Steam | Manual |

Nothing else: no analytics, remote fonts or lookups. WebView2 keeps wry's default `--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection` (research 12); the M8 silence test runs 30 min with the UI open, and any remaining Microsoft host is listed on the Network activity page and in `PRIVACY.md`. **Public repo:** secrets only in a protected release environment; secret scanning and gitleaks; uncommitted snapshots, logs and PC details never reach GitHub; reviewed PR branches do.

### 2.11 Repository layout, languages, key dependencies, coding standards

```
Kairo/ (public, GPL-3.0)  CLAUDE.md LICENSE SECURITY.md PRIVACY.md CHANGELOG.md THIRD_PARTY_NOTICES.md SETUP.md
├─ crates/  kairo-core (pure logic) · kairo-rpc · kairo-engine · kairo-net · kairo-cli · kairo-mock-engine
│           kairo-verify · kairo-testbed · kairo-loadgen · kairo-probes · kairo-devrun
├─ apps/ui/  Tauri 2 (src-tauri: engine client, kairo-media, clipboard, drag) + React/TS
├─ locales/ · data/{games,known-bad-drivers}.json · tools/{mac,pc}/ · .claude/commands/
└─ docs/  PLAN.md (this plan) · research/ (00–13, 15, prototypes/) · adr/ · spikes/ · milestones/
kairo-cloud/ (separate PRIVATE repo, Laravel 13, Stage C)
```

Languages: Rust (engine, UI shell, tools), TypeScript/React (UI), HLSL (runtime-compiled), PowerShell (PC), PHP/Laravel (Stage C).

| Area | Pins (research 08–13; others: pin at implementation time) |
|---|---|
| Toolchain | Rust 1.99.1, cargo-xwin 0.23.1, Node 24.22.1 LTS, PowerShell 7.6.7, PresentMon 2.6.0 |
| Engine | `windows` =0.62.2; `ffmpeg-next` =9.0.0 + `ffmpeg-sys-next` =9.0.1 over FFmpeg 9.0.2 LGPL (dev PC: BtbN `n9.0.2-29-g6dfaba896f`; distributed: Kairo's own build, §3.3); `wasapi` =0.26.0; `rubato` =5.0.1; `tray-icon` 0.26.1; `tokio` 1.54.0; `ts-rs` 12.0.1; `schemars` 1.2.3; `rusqlite` 0.40.2; `steamlocate` 2.1.1; `sentry` 0.49.3; `crash-handler` 0.8.1; `minidumper` 0.11.0; `minisign-verify` (pin at implementation time) |
| UI | `tauri` 2.12.4 (review 2.12.5 in M0, P20) + single-instance 2.5.4, drag 2.1.1 (Mac mock only); React 19.3.1; Vite 8.3.5; TypeScript 6.0.3; Tailwind 4.3.3; TanStack Query/Virtual; wavesurfer.js 8.0.3; i18next 26.5.4; Vitest 5.0.5; WebdriverIO 9.32.0 |
| Later / C# exit | GameInput 4.0.302; Velopack 1.2.161; Laravel 13.x · .NET 10 NativeAOT, CsWinRT 2.3.1, CsWin32 0.3.366, Vortice 3.8.3, NAudio 3.3.0, FFmpeg.AutoGen 9.0.1.1, StreamJsonRpc 2.26.150 |
| Not used | `windows-capture`, `global-hotkey`, `auto-launch`, the `interprocess` client, `sentry-rust-minidump`, `tauri-plugin-updater` (updates live in control), Video.js, ADLX (licence unchecked) |

**Licences** (decision 32): `cargo-deny` and an npm check allow permissive licences plus LGPL for the FFmpeg DLLs and deny GPL/AGPL and non-free code (x264, x265, libobs, GPL-only FFmpeg filters); no code is copied or adapted from OBS or any other third-party GPL project. Trade-off: libobs is licence-compatible but excluded, so Kairo builds its own capture stack.

**Standards:** lockfiles and `--locked`; no host-side CMake build dependencies (cargo-xwin exports `CL_FLAGS`, `LIB` and `CMAKE_*` variables that aren't target-specific, P21); `unsafe` only in capture, gpu, encode, audio, platform and input modules, with `// SAFETY:`; no `unwrap` on runtime paths; nothing blocks the hotkey thread; no per-frame allocation; AMF gets NV12 and an explicit option set; clippy `-D warnings`; strict TypeScript; generated code never hand-edited; one ADR per architectural choice.

---

## 3. Development workflow

### 3.1 Machines and toolchains, the SSH bridge, interactive launching, the mock engine

| | Mac (M2, 8 GB) | Windows gaming PC |
|---|---|---|
| Role | Claude Code, editing, `kairo-core` tests, `cargo xwin check/clippy`, UI against the mock, docs, Stage C | Every capture, encode, audio and installer build and test; harness, soaks, bench; your play-testing |
| Install | rustup 1.99.1, cargo-xwin 0.23.1 + `brew install llvm` (you accept Microsoft's SDK licence in M0); Node 24.22.1 for Kairo only, via fnm or nvm and a `.node-version` file (your global Node 22.18 stays); pnpm | Via the bootstrap: Git, VS Build Tools, rustup msvc, LLVM, Node 24, PowerShell 7 (runs every SSH command; the SSH shell itself is `cmd.exe /d /c`, see [M0](milestones/M0.md)), the pinned FFmpeg zip, PresentMon 2.6.0 |
| Limits | No Windows VM. For 8 GB: `CARGO_BUILD_JOBS=4`, `debug = "line-tables-only"`, minimal `windows` features, one rust-analyzer target, browser-only UI mode (`VITE_MOCK=1`) by default. Disk: about 12 GB free on 2026-10-21, while toolchains, the Windows SDK files and build caches need roughly 15–20 GB [Estimate], so free space first (M0) and prune `target/` regularly | Signed in and unlocked for capture tests; never RDP; no self-hosted CI runner |

- **SSH:** key-only, Private-profile firewall rule, alias `kairo-pc`; the key in `C:\ProgramData\ssh\administrators_authorized_keys`, locked with `icacls … /inheritance:r /grant *S-1-5-32-544:F /grant *S-1-5-18:F`. Admin SSH sessions are **elevated and non-interactive**, so nothing user-facing starts from SSH. The key is a standing elevated shell, so its passphrase lives in the macOS Keychain (ssh-agent); a command allow-list is Q21.
- **Sync** (research 11): working-tree snapshots pushed to a non-bare clone `C:\kairo\src` (`receive.denyCurrentBranch=updateInstead`); fallback: bundle + scp.
- **Interactive launch:** tasks registered once (interactive, unelevated) with `Register-ScheduledTask … -Priority 4 -ExecutionTimeLimit 0`, because tasks made with `schtasks /create /it` run at below-normal priority and would skew benchmarks and timing tests (P13): `\Kairo\Engine` runs the current build; `\Kairo\Run` runs the windowless `kairo-devrun.exe` on a job file, so no console flash knocks a game out of fullscreen; an elevated twin opens the elevated test window.
- **`tools/mac/pc`** (also `.claude/commands/`): **`pc status`** asks a watch agent (`kairo-devrun --watch`, started at logon in your session, since `GetLastInputInfo` can't see your input over SSH); busy = a game, a save in flight or input in the last 10 min [proposed]. Every job has a maximum duration and is killed within 2 s of your input, a game start or another fullscreen window, restoring displays and audio; overnight soaks only after you say so. **`pc doctor`** checks the toolchain and Smart App Control each session (`HKLM\SYSTEM\CurrentControlSet\Control\CI\Policy\VerifiedAndReputablePolicyState`: 0 Off, 1 On, 2 Evaluation [verify in M0]). `pc deploy` keeps 3 builds; `pc rollback` undoes one; after a Windows, GPU-driver or Adrenalin update, a harness smoke runs at the next idle window.
- **Unattended runs, OLED care:** the runner keeps the PC awake and the displays on (`SetThreadExecutionState`) and covers the OLED with a black, capture-excluded window until your first input. Patterns, harness and soaks use the 180 Hz monitor; the OLED shows only moving content, ≤10 min per run. Soak tones play at −30 dBFS; HDR-toggle and device-switch scripts run as `\Kairo\Run` jobs; sleep tests need wake timers or you present; soaks log System events 41, 1074 and 6008.
- **Dev privacy, mock:** PC facts, logs and flagged clips stay in gitignored `.dogfood/`. The mock engine speaks the real protocol over a Unix socket with scenarios (recording, errors, first run, 2,000 clips, crash, encoder pressure, update); final UI checks run in WebView2 on the PC.

### 3.2 Testing strategy

| Layer | What | When |
|---|---|---|
| Unit + property | `kairo-core`: ring, pacer on real SP2 traces, save jobs, aligners, size-fit maths, migrations | Mac + Linux CI |
| GPU on WARP | BT.709 matrix (75% bars ±1 code), tone-map values | Windows CI |
| IPC contract, UI | JSON fixtures vs mock and engine; generated-code drift; pipe DACL; Vitest, axe-core, pseudo-locale, WebdriverIO e2e on the mock | Every PR |
| Capture harness | testbed + engine + `kairo-verify` | Smoke per media merge; full matrix per milestone |
| Soak | 8 h: a verified save every 10 min; memory, handles, VRAM; tone SNR, drift; encoder stalls, event 4101; device switch, HDR toggle, sleep | M3 (1 h), M5, M8, releases |
| Performance | `pc bench` | M2, M5, M8, releases; >1 pp regression blocks |
| Privacy | Silence test, 2 h incl. 30 min with the UI open: no connections from Kairo's processes with updates and crash reports off, only GitHub with updates on; per-process attribution via WFP auditing (events 5156/5157, on for the test with your OK) | M8, releases |

**Instruments (M1).**
- **`testbed.exe`** (windowed, borderless or exclusive fullscreen; 60–360 fps): a block barcode that survives scaling and H.264; moving bars; a per-second flash with a 1 kHz "game" beep; a 2 kHz "voice" beep from another process; HDR patches; a QPC log as ground truth.
- **`kairo-verify`** (ffprobe plus its own analysis): drops and duplicates (≤0.5% [proposed]), content cadence, CFR, A/V offset per track ≤40 ms, IDR at 0, closed 1 s GOP, pts = dts, moov first, no edit list, `colr`, decoded bars ≥40 dB PSNR (catches silent encoder corruption), routing (voice tone only on T4; Kairo's cue absent), tone SNR, curve levels, export profiles; nothing GPL linked.
- **`kairo-loadgen`:** deterministic GPU load (ALU and bandwidth modes, research 13 §5.5), borderless on the OLED, scriptable.
- **`pc bench`** (research 13 §9): environment in `bench/env.json`; AMD Chill, Boost, AFMF, FRTC, Enhanced Sync, RSR, Metrics Overlay and Instant Replay off, VSync off; warm-up; A/A noise floor (6 runs; CV >1% → more); ABBA×3 (12 × 60 s, 6 pairs) of Kairo off/on; PresentMon 2.6.0 with its own `--session_name KairoBench` (Adrenalin ships a PresentMon too), `--qpc_time --track_frame_type --track_gpu_video` (P17); `pm-stats.ps1`, `ab-stats.ps1`, PDH `encoder-usage.ps1`. **Gate (Instant Replay off):** mean FPS loss ≤3% with the upper 95% CI ≤4%; 1%-low loss ≤5%; PresentMode unchanged; no worse than Instant Replay alone + 1 pp (the three-way numbers feed the onboarding warning). **Workloads:** loadgen, built-in benchmarks of 2 of your games, one HDR run, video on the 180 Hz screen, alt-tab between monitors, a Copy for Discord export during play; live play only for soaks.

| Games (yours first: Steam and Epic story games with HDR, M0) | Anti-cheat | Focus |
|---|---|---|
| Clair Obscur: Expedition 33 | — [Uncertain] | Your daily games: Unreal Engine 5, DX12 + HDR |
| Horizon Zero Dawn Remastered, Horizon Forbidden West | — [Uncertain] | Your daily games: DX12 + HDR |
| Ghost of Tsushima | — [Uncertain] | Your daily game: DX12 + HDR (SP3's HDR test game) |
| VALORANT, League of Legends | Vanguard | Hotkeys, no warnings, voice process [Uncertain] |
| CS2, Dota 2 | VAC (FACEIT) | 300+ fps on 360 Hz, Independent Flip, in-game voice in the Game track |
| Fortnite, Apex, Call of Duty | EAC (Fortnite also BattlEye), Ricochet | DX12 capture cost, hotkeys |
| Roblox | Hyperion (Byfron) [not in research; verify] | First run with you present; hotkeys, window capture |
| Minecraft Java | — | Exclusive-fullscreen freeze |

First runs in anti-cheat games are in practice or unranked mode, with you present; the matrix reruns every Kairo release, since anti-cheat updates are invisible. **Hardware:** S0 your AMD rig; S1 adds ≥1 NVIDIA, ≥1 Intel, Bluetooth headsets; S2 adds Win10 22H2, a hybrid laptop, an N edition.

### 3.3 CI/CD

- **`ci.yml`** (every PR): fmt, clippy, tests, contract drift, licence checks (`cargo-deny`, npm), forbidden-API and sockets-only checks, gitleaks, UI checks (Ubuntu); full build, WARP tests, e2e (`windows-latest`). PRs upload test reports only, never installers or DLLs, so nothing containing FFmpeg is distributed from a PR. Hosted runners are free for public repos (verify at implementation time).
- **FFmpeg job** (from M3–M5): Kairo's minimal LGPL build, cross-compiled in CI (research 09 §6.4 configure line plus export filters; nv-codec-headers 12.1.14.0; MIT AMF and libvpl headers), used by every build anyone but you receives, from M8 at the latest.
- **`release.yml`** (tags; you approve via the protected `release` environment): minisign-signed installer and `latest.json`; Authenticode from S1/S2 (SignPath, Store); PDBs to Sentry; checksums; FFmpeg source, diff and configure line; a no-test-plugin check; Claude publishes the draft after a PC smoke test and a Defender submission (microsoft.com/wdsi/filesubmission, as a software developer).
- **Hygiene:** `pull_request` only; no secrets in PR jobs; first-time contributor approval; SHA-pinned actions; **no self-hosted runner** (fork PRs could run on your PC).

### 3.4 Working agreement

- **Milestones:** Claude opens `docs/milestones/M<N>.md` (goal, gate, ≤10-step test card), works in small PRs and closes with a gate report for your "pass": aggregates and hardware class only (e.g. "RX 7000-series, 1440p 360 Hz OLED"); PC name, IP, play times, account names and raw CSVs stay in `.dogfood/`.
- **"Tonight's test":** ≤5 plain bullets: what changed, what to try, what to watch.
- **Usage limits:** Claude Code's usage limits can pause work mid-task (it happened twice during planning). Work is checkpointed in small commits, milestone docs and the engine day log, so it resumes cleanly after a reset; multi-agent research and review runs are kept for milestone boundaries.
- **During and after play:** glance at the tray once; press Alt+F8 ~2 minutes in (the **canary clip**); press the **Flag key** (Alt+F11) when something feels off. Afterwards, one line, e.g. `VALORANT ~21–23h. Clips fine. Stutter once. Flags: 1.`; Claude replies with cause, fix and build id. Every ~20 h: a 10-minute priorities review.
- **Severity:** **P0** (missed or broken clip, buffer-losing crash, stutter, driver timeout, anti-cheat warning, privacy leak) → roll back now, fix before your next session; **P1** (wrong track, bad HDR, A/V offset) → next build; **P2/P3** → batched.
- **Claude asks first** for UAC prompts, physical actions, logins, purchases, first runs in anti-cheat games, publishing, legal or identity choices, security or privacy settings, or PC changes outside its folders.
- **Done** = tests and harness green, contract regenerated, strings catalogued, docs and CHANGELOG, no new egress or licence issue, your acceptance of visible changes; milestones add gate evidence and ADRs.
- **Conventions:** trunk `main`; branches `m<N>/<slug>`; Conventional Commits + DCO + the session's co-author trailer; squash after CI; tags `v0.y.z` (1.0.0 at S2); dev builds `0.y.z+dd.N`; `.idea/`, `.dogfood/` ignored; `CLAUDE.md` = Appendix A.

---

## 4. Milestones

Sizes are relative (S < M < L < XL). No dates: a milestone closes when its gate passes and you say "pass".

### M0 · Setup and bridge (S)

**Goal.** Claude edits on the Mac, builds on the PC, launches into your desktop and reads results without you at the keyboard.
**Deliverables.** A reviewed `bootstrap.ps1` at a pinned raw GitHub URL linked from `SETUP.md` (installs OpenSSH and the toolchain; adds you to Performance Log Users for unelevated PresentMon); SSH key and alias; `C:\kairo\` with the synced clone; the `\Kairo\Engine`, `\Kairo\Run` and `\Kairo\Ensure` tasks; `kairo-devrun` (runner + watch agent); a "hello" probe; `kairo-cli` and `tools/mac/pc`; the repo scaffold (LICENSE per Q18, CLAUDE.md, `docs/PLAN.md`, `.gitignore` with `.idea/`, `.dogfood/` and `.private/` (added 2026-10-21), the research already in `docs/research/`, ADR-0001, CI, `SECURITY.md`); the permanent Tauri identifier `io.github.maty3k.kairo`; an optional Wake-on-LAN check.
**Exit.** (1) Password SSH refused; key login works. (2) `pc run hello` opens a window on your main monitor, in your session, unelevated; your key press reaches Claude's log in ≤2 min; `pc status` sees your input. (3) On the PC, `cargo build` of a test crate with a build script and a proc-macro succeeds and the hello exe runs. (4) `pc doctor` green: FFmpeg lists `gfxcapture`, `ddagrab`, the AMF encoders, `h264_mf`, `aac_mf`; unelevated PresentMon works; `\Kairo\Ensure` registered from your limited token and firing at logon. (5) `cargo xwin check` against BtbN's FFmpeg passes on the Mac (unverified in research 09; fallback: bindings generated on the PC). (6) CI green.

**You do (≈1–1.5 h, once).**
1. **Settings → Windows Update:** install everything; on 24H2 or older (**System → About**), take **25H2** (24H2 Home/Pro updates ended 2026-10-13); on 26H2, stay. Later feature updates are taken deliberately. **Advanced options → Active hours:** your evenings and nights; Claude asks you to pause updates in soak weeks.
2. **Settings → Network & internet →** your connection → **Private**.
3. **Settings → System → Power:** sleep "Never" while Claude has PC work (through M9); turn off Dynamic Lock or a screen-saver lock on agreed test days.
4. On the PC, open `SETUP.md` on GitHub and copy its one-line command; right-click **Start → Terminal (Admin)**, paste, read what it installs, press **Y**, approve UAC prompts (20–40 min). It prints DONE, the PC name and IP; then **sign out and back in**.
5. Send Claude the PC name and IP.
6. **Start → "Smart App Control":** report On, Evaluation or Off. On blocks Kairo's builds (it checks every program, build tools included), and Evaluation can switch itself to On later. If it isn't Off, choose (Q3): **(a) turn it off**, Claude's recommendation (reportedly reversible since KB5083769, though one report says KB5127648 greys the toggle out [Uncertain]); or **(b) keep it on and pay** for signing (Certum Open Source, €25, non-commercial only), with builds moved off the PC and each signed using your phone's one-time code: slower fixes, no longer $0. M0 closes only when one route works.
7. **AMD Adrenalin → Record & Stream:** report Instant Replay's state, length and hotkeys. **Leave it on.**
8. Tell Claude your games, launchers, HDR games, any run as admin, keyboard layouts, play times and the days the PC may test while you're away.
9. When asked, press a key in the "Hello from Kairo" window.
10. On the Mac, free up disk space to at least 20 GB (about 12 GB is free today), then accept Microsoft's licence for the SDK files cargo-xwin downloads (Claude shows the text).
11. Optional: a Dev Drive for `C:\kairo`, or a Defender exclusion for `C:\kairo\src\target` only.

**Claude.** Writes the bootstrap, tools and scaffold; sets up the Mac; records PC facts in `.dogfood/`. **Risks → fallbacks.** Tasks fail over SSH → the bootstrap creates them; Git over SSH misbehaves → bundle + scp; SSH flaky → Claude Code on the PC.

### M1 · Test instruments (M)

**Goal.** Machine-verified recordings before any spike or engine code. Testbed, `kairo-verify`, loadgen and bench are outside the exit-rule clock; days on `wgc-probe` and `audio-soak` count toward it.
**Deliverables.** `testbed.exe` + voice-tone process; `kairo-verify`; `kairo-loadgen`; `wgc-probe` (Kairo's own WGC module per research 08, later the engine's); `audio-soak` (research 10's tone-marker soak + noise detector); `hotkey-probe`; `pc bench` with `pm-stats.ps1`, `ab-stats.ps1`, `encoder-usage.ps1`; `pc harness`; the AMD settings diff script. On the Mac: `kairo-core` and mock-engine skeletons.
**Exit.** (1) `kairo-verify` flags 100% of synthetic bad clips (dropped or duplicated frame, judder, 50 ms A/V offset, no faststart, edit list, B-frames, wrong `colr`, swapped tracks, corrupted bars) and passes good ones. (2) Unattended testbed + FFmpeg CLI capture → a verify report in <5 min. (3) `wgc-probe` traces from both monitors. (4) Bench A/A and soak dry runs produce data.
**Claude.** Builds and validates the tools. **You do.** Nothing; keep the PC on and unlocked on agreed days. **Risks → fallbacks.** Barcode lost in compression → bigger blocks; early Rust/COM friction is reported before M3 (those days count).

### M2 · Reality check: spikes on the PC (M)

**Goal.** Measure what could stop Kairo being a daily driver here. SP1, SP2 and SP5 block M3; the rest run alongside M3–M4.
**Deliverables.** SP1–SP11 raw CSVs and clips in `.dogfood/`; `docs/spikes/M2-report.md`; one ADR per decided setting, incl. "AMD defaults (research 13)".

| Spike | Method | Pass gate [proposed] | If it fails |
|---|---|---|---|
| **SP1 AMD overhead** (blocks) | `pc bench`: FFmpeg `gfxcapture` → `h264_amf` with research 13's options; Instant Replay as its own pair; 1440p, HEVC, AV1 variants (3 pairs, 6 near a threshold) | §3.2 gate | Window vs monitor, interval, DDA; then an ADR and your call |
| **SP2 WGC pacing** (blocks) | `wgc-probe` + testbed at 360 fps on both monitors; research 08 and 13 intervals; window vs monitor; monitor moves; border check | 60 fps: ≥99.5% fresh ticks; cadence p99 \|content Δt − 16.67 ms\| ≤ one refresh period and ≤0.5% repeated or skipped frames on both monitors; callback CPU <1%; no border; 120 fps only if it meets the same rules | Other interval; no 120 fps; DDA |
| **SP5 Hotkeys, anti-cheat** (blocks) | `hotkey-probe`: all default keys and mouse buttons, 10 presses each in VALORANT, LoL, Fortnite, Apex, CS2 (practice first), an elevated window and a no-hotkeys app; AMD on; your layouts | 100% received in non-elevated games; 0 anti-cheat warnings; 0 language switches; no AMD clash | Other defaults; Raw Input earlier. **Any warning: stop, remove, report** |
| **SP3 HDR** | Ghost of Tsushima + testbed patches; three curves compared on your phone with Game Bar's capture | A curve "matches what I saw" in ≥4/5 scenes; values hold ±2 codes (1,000-nit peak, 240-nit SDR white: Natural ≈Y 215 [Estimate], Match SDR apps Y 235) | White-level override; HDR "experimental" |
| **SP4 Audio + encoder soak** | `audio-soak` 8 h with disruptions + a 3 h real session (ducking, Discord and Riot voice, Bluetooth); an 8 h 1440p AMF run alongside | 0 unrecovered failures or noise alarms; recovery <3 s; drift ±2 ms; 0 encoder stalls >2 s; 0 event 4101 | Plan B with its label; 10 min refresh |
| **SP6 Exclusive fullscreen** | Each game: present mode, fullscreen state, capture result | Every game classified; freeze detector catches 100% | Per-game flags |
| **SP7 Encoders, sizes** | AMF read-back; `bf=0`; AAC encoder ADR (`aac` or `aac_mf`); AV1 `coded_height`/`clap`; 1 h of 1440p at a 50 Mbps peak; Discord size-fit on 30 clips | Options read back as set; ≥90% ≤19,000,000 bytes first pass, 100% after one 0.9× retry | Lower peak; wider margin |
| **SP8 Playback, paste** | 4-track MP4 in common players, Discord, WhatsApp, phone; CF_HDROP paste; edit-list variant; WhatsApp 30 s / 2 min / 5 min clips from Desktop and phone, standard and HD | Mix plays everywhere; paste attaches the file; WhatsApp's size, length and re-compression limits in an ADR | Drag-out only |
| **SP9 AMD detection** | PDH per-process video-engine use + AMD registry values (diff script) [Uncertain] | 10/10 toggles, 0 false positives | Onboarding asks |
| **SP10 Discord embed ($0)** | GitHub Pages page with `og:video` → faststart footage | Plays inline on desktop and mobile | Adjust share copy before M12 |
| **SP11 WebView2, media** | Minimal Tauri window + `kairo-media`; 600 MB seek test | WebView2 gone ≤5 s after close; seek p95 <150 ms | Virtual-host mapping |

**Exit.** The report with a go/no-go per spike; ADRs (capture target, interval, 120 fps, AMF settings, AAC encoder, HDR curve, audio plan, hotkeys, thresholds); blocking spikes green or covered by an ADR you approved; you say "go". **Claude.** Runs and analyses everything remotely.
**You do (≈4–6 h over 3–4 sessions).** Start each built-in benchmark when the beep says so (≈30 min per game); key presses on request; 10 min of HDR Ghost of Tsushima plus a few Game Bar captures; one overnight soak (speakers down); paste test files into Discord and WhatsApp and send WhatsApp clips from your phone; open the embed page on your phone.

### M3 · Engine skeleton, with the Rust → C# exit rule (L)

**Goal.** Prove the hard path in Rust: capture → convert → encode → ring → save, with game and mic audio.
**Deliverables (timed scope).** `kairo-rpc` v0.1, frozen by day 2 so UI work starts on the Mac; pipe server; `kairo-cli`; WGC; SDR shader; pacer; `h264_amf` through a ≈300-line FFmpeg FFI module on its own thread, with research 13's options and the stall watchdog; game + mic → aligner → AAC; ring; Alt+F8 → save → verify; all in-process behind a `Pipeline` trait. Untimed alongside: the CI FFmpeg build job.
**Timed gate.** 20/20 harness saves via `kairo-cli` pass `kairo-verify` (30 s ± 1 GOP, CFR 60, IDR at 0, pts = dts, moov first, no edit list, `colr`, mix + game + mic, A/V ≤40 ms); save → file p95 <1 s; no capture stall during saves.
**Then, untimed.** Engine ≤60 MB + ring, <3% CPU [Estimate]; 1 h run with <5 MB/h growth; an injected 20 s encoder stall recovers in <3 s with the ring intact. A miss goes to an ADR, not the C# switch, unless it is Rust-specific.
**Exit rule (decision 26).**
- Every working day on Rust capture, encode or audio code counts from the first `wgc-probe` commit (≥4 h of engine work with the PC available ≥2 h). A day is excluded only if the PC was unavailable or the blocker reproduces with the FFmpeg CLI or a C++ sample, logged that day in `docs/spikes/engine-log.md` with evidence.
- Loop signals: the same compile, borrow or COM error class unresolved after 3 attempts, or an `unsafe` crash unresolved after 1 day.
- From day 3, a one-line daily summary to you. Day 4 without an end-to-end hard path: Claude drafts the C# plan and installs the .NET 10 SDK on the Mac (compile checks with `EnableWindowsTargeting=true`) and the PC (NativeAOT publishes only there or in CI). Day 5: a burn-down note listing every exclusion.
- At 7 counted days, or 14 calendar days after M3 starts, without the timed gate: Claude switches the engine to C#/.NET 10 NativeAOT unless you reply "keep Rust"; UI, contract, fixtures, mock and harness stay.

**Claude.** Compile-checks on the Mac; runs the harness when the PC is idle; keeps the day log. **You do.** 15 minutes: AMD off, Alt+F8 in one game, play the clip on your phone and in Discord, AMD back on.
**Risks → fallbacks.** `ffmpeg-next` gaps → raw `ffmpeg-sys-next`; NV12 render broken → `ID3D11VideoProcessor`, then `h264_mf` (never BGRA into AMF); AMF quirks → `h264_mf`.

### M4 · DD0: the first daily-driver build (S)

**Goal.** The thinnest robust slice in real games.
**Deliverables.** Native tray menu (open clips folder, pause, revert to previous build, quit); Alt+F8; cues; press detector; detection v0 (your list + foreground fullscreen window); window capture with a monitor fallback; game, mic and mix tracks; per-game folders; verify + retry; Flag key; deploy and rollback; `\Kairo\Ensure`; settings from Claude's chat onboarding.
**Exit.** 3 trial sessions (≥45 min, ≥2 games, ≥10 deliberate presses each): detector presses = hotkey presses = valid files; 5 random clips per session play with sound in Discord and on your phone; no anti-cheat warning; no stutter you notice; a killed engine is back within 60 s.
**Claude.** Deploys between sessions with "Tonight's test"; fixes any P0 before the next one. **You do.** Turn AMD Instant Replay **off only for trial sessions** (so they don't fight over the encoder and keys); play with Windows HDR off in trial sessions (HDR arrives in M5); canary clip, ≥10 presses, one-line note.
**Risks → fallbacks.** Window capture misbehaves → per-game monitor capture; game missed → "This is a game" in the tray.

### M5 · Switchover: Kairo replaces AMD Instant Replay (L)

**Goal.** Good enough that you turn AMD Instant Replay off for good, proven in real play.
**Deliverables (DD builds between sessions).** Control/media split and safe levels; the **DDA backend** (Win11 fallback, L2); autostart after your yes; detection v1; screen mode; HDR → SDR; presets; audio v1 (four sources, mix, A/B refresh, voice consent, in-game-voice notice); 2-minute key, custom lengths, screenshots, mouse buttons, pop-up; probe process, governor, version watch; the Instant Replay warning; **"Copy last clip for Discord" in the tray** (waits for encoder headroom mid-game); crash outbox.

| Switchover gate | All must hold |
|---|---|
| G1 Coverage | Every game on your list records (or has a documented override) |
| G2 Reliability | Harness 500/500, incl. overlapping, 2-minute and HDR-toggle saves; ≥100 real presses over ≥5 sessions, detector = hotkey presses = valid files |
| G3 Performance | `pc bench` with Kairo's real path meets the §3.2 gate, incl. the HDR run, video on the second screen and a Copy for Discord export during play |
| G4 Picture | HDR and SDR clips judged "what I saw" on 5 scenes each |
| G5 Sound | Correct game/mic/mix through a ≥4 h session with voice chat running; no noise or drift; Discord voices absent while voice chat is off |
| G6 Resilience | Media killed 20×: back in <3 s; encoder recreated 20×: 0 lost saves; forced DDA passes the harness; autostart on 10/10 logins |
| G7 Screenshots | 20/20 valid, incl. HDR pairs |
| G8 Pop-up | 100 show/hide cycles with AMD's Metrics Overlay on: 0 event 4101, PresentMode back to Independent Flip within 1 s, no FPS dip; else the pop-up defaults off in games |

**Then the run.** You turn AMD off; M5 closes after ≥20 h of real play (≥8 sessions, ≥4 games) with zero missed or broken clips; any P0 resets the counter.
**Claude.** Ships DD builds, runs harness, soaks and bench, tracks the gate from logs. **You do.** One 30-minute bench session; ≥100 Alt+F8 presses over ≥5 normal sessions; compare 5 HDR and 5 SDR clips with what you saw (match / no match); one ≥4 h session with voice chat running; onboarding answers in chat; switch AMD off at the gate, then ≥20 h over ≥8 sessions.
**Risks → fallbacks.** G3 fails → ADR and your call; plan A audio unstable → plan B with its consent label; G8 fails → cue only in games.

### M6 · Library app (L)

**Goal.** The window you open after playing, plus the onboarding wizard and settings, built on the Mac against the mock since M3.
**Deliverables.** Main window, tray popup, clip view, settings, onboarding with test clip, "Test my setup", Report a problem screen (bundle preview, Copy for Claude); engine library (SQLite, watcher, posters, sprites, search); `kairo-media`; e2e and accessibility suites.
**Exit.** With 2,000 clips: cold open ≤1.5 s [proposed], search ≤100 ms, smooth scrolling; seek p95 <150 ms on a 600 MB clip; ≤300 MB RAM; no Kairo WebView2 process 5 s after closing; contract, axe and keyboard-only tests pass. You do 6 tasks unaided (find yesterday's clip, favourite it, show it in Explorer, set 60 s, start screen mode on the second monitor, show only clips over 100 MB from last week) and use the app in ≥10 sessions; your top three annoyances are fixed.
**Claude.** Builds on the Mac, validates on the PC. **You do.** Pick the accent; use it; send annoyances. **Risks → fallbacks.** Slow seeking → virtual-host mapping; slow popup → the native tray menu stays the quick path.

### M7 · Edit and share (L)

**Goal.** From clip to Discord, WhatsApp or TikTok in seconds.
**Deliverables.** Quick/Precise trim; vertical editor (3 layouts); export queue and presets; Copy and Copy for Discord from library, clip view, tray popup and a hotkey; drag-out; per-stem toggles; first-share notice.
**Exit.** 50 automated Discord exports (15–120 s, calm and chaotic, SDR and HDR): 100% ≤19,000,000 bytes, ≥90% first pass, mean ≥85% of 18,000,000 bytes; with the voice tone recorded, Copy, drag-out and every preset contain no voice tone unless ticked (`kairo-verify`); Ctrl+V into Discord plays inline on desktop and phone 10/10; 10 WhatsApp sends play on your phone; vertical outputs pass `kairo-verify --profile reels`; Precise trim ±1 frame; you share ≥10 real clips and make 5 vertical ones.
**Claude.** Builds and verifies every output. **You do.** Share in real use; check vertical clips on your phone. **Risks → fallbacks.** Size overshoot → 0.9× retry, then a lower rung; paste rejected → drag-out.

### M8 · Hardening: installer, updater, diagnostics, soak (L)

**Goal.** Installs, updates, recovers and reports like a finished product.
**Deliverables.** Per-user installer; control's updater (stable/beta, idle-only, minisign-verified); release workflow; Report a problem → GitHub issue form, Save report file, redaction tests; opt-in Sentry with outbox; `kairo-net` allowlist + Network activity page; Kairo's own FFmpeg build shipped; robustness matrix (display and HDR changes, unplugged monitor, sleep, audio hot-plug, mic privacy, full disk, OneDrive and non-ASCII paths); soak and silence tests; privacy policy v1; licence notices; an **MSIX spike** (winapp CLI).
**Exit.** Fresh standard account: install → first clip ≤3 min [proposed]; uninstall leaves no process or autostart and keeps clips; updates keep settings and library and never run during a game or save; 8 h soak: memory growth <10% [proposed], 48/48 saves valid, 0 event 4101, 0 encoder stalls >2 s; injected crashes recover <3 s; a media crash forced during testbed's barcode and tones yields a dump with no barcode bytes, tone samples, user or PC name (byte search), uploaded only after consent; silence test passes; `\Kairo\Ensure` works on a standard account (else Run key + UI restart); the MSIX build records, autostarts and captures without a border; S1 distribution route decided.
**Claude.** Builds and runs it all; drafts privacy policy v1. **You do.** Create a second, standard Windows account (**Settings → Accounts → Other users**) and install there like a friend; keep **two offline backups of the updater key** (losing it strands every install); send one test report; give the name and contact address the policy shows (Q9).
**Risks → fallbacks.** Updates too heavy → Velopack deltas; winapp CLI changes → NSIS stays primary.

### M9 · S0 solo-dogfood gate (S)

**Goal / deliverables.** Decide on evidence: a dogfood summary from local logs and open issues. **Exit.** The S0 exit (§1.2). **Claude.** Compiles the summary. **You do.** A 15-minute review; "Would you give this to a friend today?"; pick 3–8 friends with mixed hardware (Q13, Q16). **Risks → fallbacks.** Gate fails → fixes, then rerun.

### M10 · S1 friends beta (L)

**Goal.** Kairo works on PCs Claude never touched.
**Deliverables.** Friend guide ("More info → Run anyway", Smart App Control, other recorders off, Flag key, reporting, Defender quarantines); beta channel; Beta summary screen; unsigned NSIS for testers with Smart App Control off and a **Store private-audience MSIX** for those with it on (free, re-signed by Microsoft; testers need a Microsoft account [M]). The MSIX needs your Partner Center account (Individual vs Company is irreversible, Q10), name reservation, IARC, a privacy-policy URL and capabilities (`runFullTrust`, `graphicsCaptureProgrammatic`, `graphicsCaptureWithoutBorder`, microphone, `windows.startupTask`). Also a SignPath Foundation application, every tester's self-test, triage.
**Exit.** The S1 exit (§1.2); NVENC validated on ≥1 tester; QSV validated or labelled "experimental".
**Claude.** Ships betas, triages against capability reports and beta summaries, fixes P0/P1 first. **You do.** Invite friends; open Partner Center if needed (ID + selfie); forward problems; remind friends to send their weekly summary.
**Risks → fallbacks.** Store delays → those testers wait for SignPath; old NVIDIA driver → `h264_mf`.

### M11 · S2 public-launch prep (L)

**Goal.** Ready for strangers.
**Deliverables.**
- **Windows 10 22H2** (decision 8): DDA; endpoint loopback with the voice-consent gate and cue suppression, checked by `kairo-verify` (no voice tone and no cue in T1/T5 before consent); 2 × 8 h soaks; audio "best effort" if noisy.
- **Store MSIX public listing** and the **signing route** (§5).
- **Name:** the private knockout search (`.private/14-name-check.md`) rates "Kairo" HIGH risk for video software in the EU, US and UK, so a paid clearance opinion (your approval) or a rename comes before any branding spend (Q22); then a modifier domain (Q20).
- **Website** on GitHub Pages: download, FAQ, privacy policy v2, terms, `security.txt`.
- **`docs/milestones/M11-launch-checklist.md`:** Win10 gate; Store certification; SignPath-signed download; notices + FFmpeg source and configure line; policy v2, terms and `security.txt` live; trademark opinion; domain + site; M12 gate; S1 exit still holding on the latest beta; rollback rehearsed.

**Exit.** Harness 100/100 and an 8 h soak on Windows 10 (dual boot or field-test bundle, Q12); Store certified, and the Store build records and autostarts with Smart App Control **on**; trademark opinion with no blocking conflict (or a rename decided); site and policy live; launch checklist 100%.
**Claude.** Builds Win10 and MSIX; prepares the brief, site and policy; runs launch QA. **You do.** Set up the Win10 test system from Claude's steps (≈1 h; disconnect the main drive while installing) or hand a friend the field-test bundle; approve domain and clearance spend; confirm the identity for policy v2.
**Risks → fallbacks.** Certification fails → launch the download, resubmit; counsel advises against the name → rename before branding spend; no Win10 machine → the launch waits.

### M12 · Stage C: share links + optional accounts (XL; private repo; parallel with M11)

**Goal.** A Share link that plays inside Discord; accounts for nothing else. S2 needs M11 and M12 (decision 23).
**Deliverables.** `kairo-cloud` (Laravel 13, §6); desktop sign-in (system browser + PKCE), Create link, Links view, delete link and account.
**Exit.** 50/50 links embed and play in Discord desktop and mobile within the 10 s budget; Discord, Google and Steam sign-in work; signed-out use loses nothing; deletion removes file and page; 30-day expiry passes a mocked-clock test; account deletion purges everything ≤24 h; legal pages and DMCA agent live; cost within budget.
**Claude.** Builds backend and desktop flow; sets up R2 and hosting after approval; writes moderation runbooks. **You do.** Approve hosting (the launch spend decision 6 anticipates); create developer and hosting accounts from Claude's steps; set retention, limits, minimum age and cost ceiling (Q19).
**Risks → fallbacks.** Embed too slow → smaller share copy, CDN caching; costs climb → shorter retention or tighter caps.

### M13 · v1.1 (L)

**Goal / deliverables.** Controller trigger via GameInput 4.0.302 after a background-input spike; Xbox rumble if it works; PlayStation read-only; Raw Input fallback. Optional elevated mode only if SP5 shows elevated games among yours: an ADR you approve amends the house rule; off by default; a separate elevated helper that only registers hotkeys / Raw Input and forwards presses over the pipe (no capture, no game handles), tested in the anti-cheat matrix first. Disk buffer for clips >120 s (SSD-wear notice); manual recording.
**Exit.** 100/100 controller presses in 3 games; 0 writes to PlayStation pads in a HID trace; rumble felt on 10/10 saves with an Xbox pad (else an ADR drops it); 10 manual recordings of 5–60 min pass `kairo-verify`, stop → file <3 s; a 5-minute disk-buffer clip saves <3 s [Estimate]; clean 8 h disk-mode soak.
**Claude** spikes, builds, tests. **You** use an Xbox controller (plus a PlayStation pad for the read-only check) and make long clips. **Risks → fallbacks.** GameInput deployment [Uncertain] → bundled MSI with consent; no background haptics → no rumble.

### M14 · v2 (XL)

**Goal / deliverables.** "Clip that" (local sherpa-onnx, opt-in, games only); captions (whisper.cpp, editable); music (your files, ducked); montages. **Exit.** ≤1 false trigger per 5 h with voice chat, ≥90% detection [proposed], <1 CPU core [Uncertain]; captions on 10 clips need ≤1 fix each; a 5-clip montage exports <60 s [Estimate]. **Claude** measures keyword spotting during play first; **you** use it and review captions. **Risks → fallbacks.** False triggers → push-to-arm; Whisper too heavy → captions on demand, never while gaming.

---

## 5. Shipping and operations

**Installer and updates.** Tauri NSIS, per user, no admin, bundling UI, engine, FFmpeg LGPL DLLs, `ffmpeg.exe` and notices; control installs full updates from a minisign-signed `latest.json` per channel (§2.1; Velopack deltas only if updates get heavy); uninstall keeps clips; the updater key has two offline backups. Store flavour: MSIX with Store updates and a startup task.

**Versions and channels.** dev `0.y.z+dd.N` (your PC), beta `0.y.z-beta.N`, stable SemVer with 1.0.0 at S2; rollback = ship N+1. After any Windows, GPU-driver or Adrenalin change: version watch + harness smoke; feature updates are taken deliberately; the anti-cheat matrix reruns every release.

**$0 signing path.** **S0:** unsigned; dev builds copied over SSH carry no download mark, but the M8 release installer from GitHub does (SmartScreen: More info → Run anyway), and Smart App Control blocks unsigned builds either way (the M0 decision). **S1:** unsigned NSIS + Store private-audience MSIX; apply to **SignPath Foundation** (free for OSI-licensed projects; needs a released project, CI builds and some reputation [Uncertain]; "SignPath Foundation" shows as publisher). **S2:** **Store MSIX** (Microsoft re-signs) + a SignPath-signed download, every PE file signed. **Pay only if** SignPath declines and download friction shows: Certum Open Source (€25, non-commercial only), Certum OV (from €209) only when monetizing; Artifact Signing ($9.99/month) needs an organisation in one of 12 regions or a US/Canada individual.

**Crash reporting.** Sentry EU region, free Developer plan (5k errors/month); IP storage off, server-side scrubbing on; the DSN is a CI secret, so forks never report.

**Privacy policy** (Store policy 10.5.1, GDPR Art. 13). v1 (S1, written in M8): controller identity (Q9; Impressum if required); clips stay local; the §2.10 flows and their legal basis; processors (GitHub, Sentry); retention; rights. v2 (S2) adds accounts, uploads (which may contain others' voices), 30-day retention, Cloudflare, the host and DSA/DMCA contacts.

**GPL-3.0 + LGPL FFmpeg.** Kairo: `LICENSE`, SPDX headers, source = the public tag (Q18); no code copied from GPL projects such as OBS. FFmpeg: LGPL only, Kairo's own build for everything distributed, credited in About under LGPLv3 with the LGPLv3 + GPLv3 texts; users may replace the DLLs; every release attaches the exact FFmpeg source, diff and configure line. Whether the Store package's locked files satisfy LGPLv3 is [Uncertain] → counsel brief. Codec patents (AMF grants no licence; Via LA's H.264 pool is royalty-free to 100,000 units a year [Uncertain]): review before S3 or at 50,000 installs a year. GPL vs Store terms [Uncertain]: while you are sole author, add a GPL §7 Store permission if needed and accept no outside code until settled.

**Security contact.** `SECURITY.md` + GitHub private vulnerability reporting (M0); `security.txt` (S2); acknowledge ≤72 h [proposed]. Non-monetized FOSS is outside the CRA; once commercial, CRA vulnerability reporting (24 h / 72 h / 14 days, in force since 2026-09-11) applies at once and full duties from 2027-12-11, and the Product Liability Directive covers software placed on the market after 2026-12-09.

---

## 6. Share links and optional accounts (Stage C design sketch)

**Shape.** Private repo `kairo-cloud`: Laravel 13.x (13.38.0 at research time) with an API and Blade share pages, a queue, a database (pin at implementation time) and Cloudflare R2 behind a custom domain. Hosting: Laravel Cloud (from $5/month) or Forge ($12 + VPS). Video never passes through the app.

**Flow.**
1. Sign-in only when sharing: system browser + PKCE; Socialite (Google) and SocialiteProviders (Discord, Steam); a Sanctum token stored with DPAPI.
2. The desktop encodes a share copy (H.264 ≤1080p60, ~6–8 Mbps, mix only, faststart, metadata stripped) + poster.
3. `POST /api/v1/shares` → quotas (≤3 min, ≤200 MB [proposed], rate limits) → presigned upload URL (`Storage::temporaryUploadUrl`) → R2 → queued ffprobe check → link copied.
4. **Discord embed:** one GET, no JavaScript, 10 s budget; server-rendered `og:video` (direct HTTPS MP4 on the CDN), `og:image`, `twitter:card=player`.
5. **Retention:** unlisted, unguessable, 30 days (R2 lifecycle + cleanup job); deleting a link or account removes everything.

**Data.** `users`, `identities` (provider ID, display name, email if given), `shares` (object keys, size, duration, title, game, status, expiry), `notices`, `audit_log`; a view counter is the only metric.

**Costs** (research 06). 1,000 users × 50 clips × 30 MB a month, kept 30 days, 20 views each: R2 ≈ $22/month (free egress) vs Cloudflare Stream ≈ $625; with hosting ≈ $40–110/month; at launch, R2's free tier + $5–20 hosting [Estimate]. Every spend needs your approval.

**Legal duties.** DSA Arts. 11–18 (contact points, notice-and-action, statements of reasons); an EU representative if not EU-established; a DMCA agent + repeat-infringer policy; an NCMEC runbook; a UK Online Safety Act risk assessment; Store policy 11.12; users 13+; terms forbidding AI training, sale and paywalled viewing; abuse controls (sign-in to upload, caps, rate limits, a report button, hash-blocking of removed content).

---

## 7. Risks and mitigations

| # | Risk | L / I | Mitigation (signal) |
|---|---|---|---|
| R1 | Capture costs too much FPS (LTT: OBS's hook-based Game Capture −13% at 1080p / −17% at 1440p on an RX 9060 XT; AMD ReLive −4% / −7%) | M/H | Out-of-process capture, one device, NV12, no pre-analysis; §3.2 statistics (SP1) |
| R2 | Capture misbehaves in some games: Independent Flip lost, exclusive-fullscreen freezes | M/M | PresentMode in every bench; hidden pop-up; watchdog; monitor or DDA fallback (SP2, SP6, G8) |
| R3 | Long-session audio noise or drift | M/H | Aligners, A/B refresh, soak noise detector (SP4) |
| R4 | A missed clip breaks trust | M/H | Verified saves + retry, press detector, canary clip, rollback, AMD safety net until switchover |
| R5 | Rust/COM friction stalls the engine | M/H | Thin modules; counted-day exit rule with a calendar backstop; C# kit on day 4 |
| R6 | Hotkeys fail (elevated games, clashes, layouts) or an anti-cheat flags Kairo [Uncertain] | M/H | Modifier defaults, AMD registry parse, Raw Input v1.1; house rules + CI check; practice modes; matrix every release (SP5) |
| R7 | HDR clips look wrong | M/M | Your curve choice vs Game Bar (SP3) |
| R8 | Trust friction: unsigned builds, Smart App Control, antivirus false positives on a tray + hotkey + capture + mic app | H/M | M0 decision; Store private audience; SignPath; Defender submission per release; detections in the capability report |
| R9 | Dev work disturbs your gaming or wears the OLED | L/H | Watch agent, abort on input, black OLED cover, patterns on the 180 Hz monitor |
| R10 | Narrow hardware coverage (one AMD PC; NVENC driver floor; QSV) | H/M | Friends' hardware, capability reports, self-test; own FFmpeg build; `h264_mf` (M10) |
| R11 | Dependency, OS or driver churn (windows-rs 0.100, wasapi 0.27, Tauri 3; Adrenalin updates; Windows Update swapping drivers) | M/M | Exact pins; version watch + re-probe; `known-bad-drivers.json`; harness after updates |
| R12 | Public repo leaks data, or the updater key is lost | L/H | `.dogfood/` ignored, aggregates only, gitleaks; two offline key backups |
| R13 | Name collision: the private knockout search rates "Kairo" HIGH risk for video software (EU, US, UK) | H/H | Counsel or a rename before launch; no branding spend before; the brand is one i18n key and the internal id stays `kairo` (M11, Q22) |
| R14 | Stage C cost, abuse or legal load | M/M | R2, unlisted links, caps, retention, runbooks (M12) |
| R15 | No Windows 10 hardware | M/M | Dual boot on a spare SSD; field-test bundle; the launch waits (M11) |
| R16 | AMD driver timeout or AMF stall in long sessions (research 13 §1.6: an RX 7900 GRE 1440p OLED replay buffer ended in a TDR) | M/H | Lazy probe, governor, stall watchdog, peak ≤50 Mbps, NV12 only, 8 h soaks, TDR count in reports (SP4, G6, M8) |
| R17 | Mixed-refresh monitors: judder, HDR mismatch, a shared video engine (RX 9000) | M/M | Per-monitor interval and pool format; cadence gate; bench with video on the second screen (SP2) |
| R18 | Upkeep depends on Claude Code; the 8 GB Mac (about 12 GB free disk) slows builds | M/H | ADRs, `docs/PLAN.md`, CLAUDE.md, tests and harness let any agent or contributor continue; 30-day links; §3.1 build settings |

---

## 8. Decision log and open questions

**Your decisions, all dated 2026-10-21 (binding).** The note column records where research disagreed or the plan adds a rule.

| # | Decision | Note |
|---|---|---|
| 1 | Staged S0 → S3; product quality from day one | |
| 2 | Edges: light/private, sharing, creator tools; auto-highlights deferred | |
| 3 | No ads/watermark; never AI training or sale; no tracking (opt-in crash reports only); no account needed, optional account for cloud features | Update checks on by default (anonymous, switchable) go beyond "only crash reports": confirmed by the owner (decision 33) |
| 4 | Keep "Kairo"; trademark check + modifier domain before launch | The private knockout search (run after this decision) rates the name HIGH risk; Q20, Q22 |
| 5 | Desktop GPL-3.0; closed cloud in a private repo | Research preferred MIT/Apache; GPL still qualifies for SignPath |
| 6 | $0 now; spend only on demand | Research advised a €209 certificate; the plan uses unsigned + Store + SignPath |
| 7 | Opt-in crash reports; Report a problem → GitHub; reports to Claude | Issue links can't carry files: you attach the saved zip; issues are public |
| 8 | Windows only; Win10 22H2 via DDA before launch | Win10 per-app audio "best effort" |
| 9 | AMD RX 7000/9000; 360 Hz HDR OLED + 180 Hz; HDR automatic, SDR clips | |
| 10 | Console/cloud gaming → Future | Screen mode still records cloud windows |
| 11 | Replace AMD Instant Replay (knobs, playback) | |
| 12 | v1 = replay + screenshots; manual recording later | |
| 13 | Games auto-recorded (window + audio); screen mode via tray, monitor remembered | Research preferred monitor capture; window capture needs a freeze fallback |
| 14 | All games, so never inject | |
| 15 | 30 s default, custom lengths, several hotkeys; RAM ≤120 s | Longer clips need the v1.1 disk buffer |
| 16 | Onboarding asks quality; no knobs; H.264; H.264/AAC faststart exports | Plain Copy hands over an original only when it already meets this |
| 17 | Per-source tracks + mix; voice chat off with consent | Win10 "Everything else" and plan B sit behind the same consent |
| 18 | Keyboard + mouse → controller + Xbox rumble (v1.1) → voice (v2) | |
| 19 | Cue, pop-up, rumble, each configurable | Pop-up off in games if G8 fails |
| 20 | Library features; cleanup optional, off | |
| 21 | Editor v1 trim + 9:16; v2 captions, music, montages | |
| 22 | v1 sharing with no server; YouTube maybe later | |
| 23 | Share links + optional accounts at the public launch | |
| 24 | Library-grid window + tray popup | |
| 25 | Dark, sleek, one accent | |
| 26 | Tauri 2 + Rust engine over named-pipe JSON-RPC; C# after ~7 days | Proposed counting: Rust days count from M1's `wgc-probe`; PC-unavailable days and blockers reproducible outside Rust are excluded; a 14-calendar-day backstop; you can reply "keep Rust" |
| 27 | Claude builds; you play-test, no code | |
| 28 | No deadline; quality gates | |
| 29 | Mac for code, PC for tests; SSH first | |
| 30 | You first, friends later | |
| 31 | Great defaults, everything adjustable | |
| 32 | Never inject; WGC + DDA; H.264 default; Alt+F8/Alt+F6; voice off; opt-in Sentry EU; no GPL deps | libobs excluded although licence-compatible; OBS shader code also excluded, keeping the GPL §7 Store option open |
| 33 | Update checks on by default: anonymous (version + Windows build to GitHub), disclosed in onboarding, one switch to turn off | Decided after the plan was drafted |
| 34 | "Kairo" is the working name; the public name is settled before any branding spend | Decided after the private knockout search rated the name HIGH risk; Q22 |
| 35 | Licence: GPL-3.0-or-later | Answers Q18 |
| 36 | Smart App Control off on the dev PC (route a) | Answers Q3; decided 2026-10-22, after the bootstrap found it On |

**Plan proposals** (veto any at a gate): instruments before spikes; switchover before the library app; control/media split; engine-owned updates; 2-minute key on by default; Alt+F11 Flag key; press detector in dev/beta; "Natural" HDR curve pending SP3; `bf=0`, no edit lists; Discord target 18,000,000 bytes; pop-up rules; permanent identifier `io.github.maty3k.kairo`; voice-free share copies; the exit-rule counting rules (row 26); no outside code until GPL vs Store is settled.

| Q | Open question | Recommendation | Decide by |
|---|---|---|---|
| Q1 | GPU model, Windows build, CPU, RAM | Claude reads them | M0 |
| Q2 | Games, anti-cheats, admin games, keyboard layouts, unattended test days | Answered 2026-10-22: single-player story games with HDR on Steam and Epic, US layout; admin games unknown; details in `.dogfood/` | Done |
| Q3 | Smart App Control: off (maybe irreversible), or on with paid signing | Decided: off (decision 36) | Done |
| Q4 | Voice apps; push-to-talk; headset or Bluetooth | — | M5 |
| Q5 | Default quality; 120 fps | From M2 numbers | M2 |
| Q6 | 2-minute key on by default | Yes | M5 |
| Q7 | Library drive | Local SSD, not OneDrive | M5 |
| Q8 | Accent colour | Volt | M6 |
| Q9 | Legal residence; name and contact address in the privacy policy; company plans | — | M8 |
| Q10 | Store account Individual or Company (irreversible) | Individual unless a company exists | First Store submission (M10) |
| Q11 | PCs without a hardware encoder | Unsupported, clear message | Before M10 |
| Q12 | Windows 10 test system | Win10 22H2 on a spare SSD in your PC (dual boot, if AMD still ships Win10 drivers for your GPU [Uncertain]), driven over SSH; else a friend runs a no-admin field-test bundle | Before M11 |
| Q13 | NVIDIA/Intel coverage | Friends first | M9 |
| Q14 | S2 spend: domain, counsel, hosting, Certum if needed | Approve the first three | M11/M12 |
| Q15 | DCO or CLA | DCO, after the GPL/Store question | First outside PR |
| Q16 | Which friends | 3–8, ≥1 NVIDIA; Windows 10 friends join at M11 | M9 |
| Q17 | Flag key and press detector in stable builds | No | M8 |
| Q18 | GPL-3.0-or-later or -only | Decided: -or-later (decision 35) | Done |
| Q19 | Link retention, caps, minimum age, cost ceiling | 30 days, ≤3 min, ≤200 MB, 13+ | Before M12 |
| Q20 | Register kairoclip.com now, before someone else does? | Wait until Q22 is settled: no money on a name that may change | After Q22 |
| Q21 | Narrow Claude's SSH access to a command allow-list | Your call: safer, but other tasks then need you at the PC | M8 |
| Q22 | Final public name, given the HIGH trademark risk | Working name confirmed (decision 34); Claude shortlists and checks alternatives before M11 | Before any branding spend |

---

## Appendix A. Proposed CLAUDE.md

```markdown
# CLAUDE.md: Kairo house rules
Windows instant-replay clipper, GPL-3.0, PUBLIC repo. The owner play-tests and decides; you build everything.
Read first: docs/PLAN.md, docs/adr/, docs/milestones/<current>.md; before engine work, docs/research/15-implementation-cookbook.md. Gates, never dates.

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
```

---

## Appendix B. Research references and glossary

**Research** (web-verified 2026-10-21; in `docs/research/`): `00-brief.md` (synthesis, fact-check) · `01-video-capture.md` · `02-audio-console-cloud.md` · `03-triggers-safety-detection.md` · `04-competitors.md` · `05-stack-workflow.md` · `06-post-clip-sharing.md` · `07-shipping-legal.md` · `08-capture-rust.md` · `09-encode-mux-rust.md` · `10-audio-rust.md` · `11-engine-shell-rust.md` · `12-tauri-ui.md` · `13-amd-performance.md` (AMF settings, AMD driver issues, measuring) · `15-implementation-cookbook.md` (verified APIs, code sketches, the M3 checklist; P1–P22) · `prototypes/` (type-checked sketches, not product code) · `14-name-check.md` (private, in the git-ignored `.private/`).

| Term | Meaning |
|---|---|
| WGC / DDA | Windows.Graphics.Capture (window or monitor) / DXGI Desktop Duplication (monitor; the Win10 path) |
| AMF / QSV / NVENC; VCN | AMD / Intel / NVIDIA hardware encoders; AMD's video engine |
| GOP / IDR | Frames from one keyframe to the next / a keyframe that starts a clean segment |
| Ring buffer, epoch | Memory holding the newest encoded media / a stretch from one encoder session |
| QPC, CFR | Windows' precise clock; constant frame rate |
| WASAPI, process loopback | Windows' audio API; recording one app's sound |
| Faststart, edit list | MP4 index at the front; trim metadata many apps ignore |
| MSIX, NSIS, CF_HDROP | Store package; classic installer; clipboard format for files (paste into Discord) |
| PresentMon, Independent Flip | Frame-timing tool; the fast fullscreen path |
| TDR, NativeAOT, ADR | GPU driver timeout and reset; natively compiled C#; Architecture Decision Record |
