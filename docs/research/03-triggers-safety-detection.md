# 03 — Clip triggers, anti-cheat safety, in-game feedback and game detection (Windows)

**Project:** Kairo (Windows-only instant-replay clipper) · **Research date:** 2026-10-21 · **Scope:** hotkeys, controller and voice triggers, anti-cheat compatibility, "clip saved" feedback, game detection and metadata.

**How to read this.** Each claim is tagged:
- **[V]** verified against a primary source (vendor docs, source code, official post) during this research.
- **[S]** secondary source (forum, news, third-party docs).
- **[U]** unverified, or my own inference. Treat it as a hypothesis to test on the Windows PC.

**Method note.** Partway through, the shared web-search budget for this session ran out. After that I could only fetch known primary URLs directly: Microsoft Learn, GitHub source, vendor pages and live API endpoints. Some items I could not check: Activision/Ricochet and FACEIT policy pages (blocked or timed out), Discord's developer-terms pages (HTTP 403), SteamGridDB's terms (JavaScript-rendered) and the Picovoice pricing page (JavaScript-rendered). Each one is flagged where it matters.

---

## 0. TL;DR

1. **Hotkeys.** Use `RegisterHotKey`. Electron's `globalShortcut` (through Chromium) and Tauri's `global-hotkey` both use it on Windows [V]. It needs no hook and no injection, and it reports conflicts by failing.
   - Single-key and Shift+key hotkeys do not fire when the foreground game runs elevated (UIPI) [S], and Medal tells users to run as admin to fix hotkey problems [V].
   - Some games disable other apps' hotkeys with `RIDEV_NOHOTKEYS` [V].
   - **So the default clip hotkey must use Ctrl or Alt.** A Raw Input fallback is worth adding later.
2. **Elevated games break most passive input methods.** Without UIAccess, a normal-integrity process cannot read input aimed at a higher-integrity window through low-level hooks, Raw Input or `GetAsyncKeyState` [V, Microsoft]. UIAccess requires a trusted code signature and an install under Program Files [V].
3. **Controllers.**
   - Windows.Gaming.Input delivers nothing in the background [V, SDL source and Unity].
   - XInput is documented as focus-gated on Windows 10+ [V], but practitioners use it in the background [S]. **This conflict must be tested.**
   - **Microsoft GameInput** offers documented, opt-in background input, plus Guide/Share button callbacks. It runs on Windows 10 19H1+, and the current NuGet release is 4.0.302 (2026-10-19) [V].
   - PlayStation controllers must be read-only. Any output or feature report can switch a Bluetooth DualSense into "enhanced" mode and break DirectInput for other apps until the controller is power-cycled [V, SDL].
4. **Voice.**
   - Picovoice's Free Tier ended on **2026-06-30**. The SDK will not start without a valid AccessKey, and no non-commercial tier is planned [S, Picovoice reply quoted on the Home Assistant forum].
   - openWakeWord's pre-trained models are licensed **CC BY-NC-SA 4.0** (non-commercial only) [V].
   - The viable local options are **sherpa-onnx keyword spotting** (Apache-2.0; any phrase without retraining) and **Vosk** (Apache-2.0; 40 MB English model) [V].
   - Voice belongs after v1.
5. **Anti-cheat.** The safe zone is "never touch the game process". Use out-of-process capture (WGC/DXGI) and no injection, no memory reads, no synthetic input and no drivers.
   - Riot blocks game-memory access by unknown third-party apps by default from **2026-10-06**. Approved tools have a grace period until April 2027, after which only official APIs are allowed [V].
   - Injection-based tools keep breaking. OBS Game Capture was blacked out by a Vanguard update on 2026-07-21 [S]. OBS 31's new certificate was not accepted by Riot and FACEIT anti-cheats [S]. AMD Anti-Lag+ caused VAC bans in CS2 in Oct 2023 [S].
6. **Feedback.**
   - Windows toasts are unreliable in games. Windows 11 can automatically turn on Do Not Disturb "when playing a game" or "when using an app in full-screen mode" [V].
   - Use a short sound cue, kept out of the clip with process-loopback exclusion [V], plus an optional tiny non-activating topmost window excluded from capture with `WDA_EXCLUDEFROMCAPTURE` (Windows 10 2004+) [V].
   - Skip the visual toast in true exclusive fullscreen.
7. **Game detection.**
   - Discord's `/api/v9/applications/detectable` endpoint works without authentication: 24,780 apps, 13 MB, checked 2026-10-21 [V].
   - It is **not documented** in Discord's API docs [V], and Discord's Terms prohibit scraping without written consent [V]. **Don't ship a dependency on it.**
   - Build Kairo's own database from launcher manifests (Steam, Epic, Ubisoft, GOG, Xbox), plus foreground-window heuristics and a "mark as game" button.
   - IGDB is free for non-commercial use at 4 requests/s. Commercial use needs a (free) partnership with attribution, and the client secret must stay off the client [V].

---

## 1. Global hotkeys while a fullscreen game has focus

### 1.1 Mechanisms compared

| Mechanism | How it works | Injects into game? | Swallows the key? | Mouse buttons? | Elevated (admin) foreground game | Main failure modes |
|---|---|---|---|---|---|---|
| **`RegisterHotKey`** | The system matches the combination and posts `WM_HOTKEY` to your thread or window [V] | No | Yes, for the hotkey keystroke [U on whether raw-input games still see it] | **No** (virtual-key codes only) | Single-key and Shift+key hotkeys fail; Ctrl/Alt combinations reportedly work [S: LibreAutomate docs] | Fails if another app already registered the combination [V]. F12 is reserved for debuggers [V]. Win-key combinations are reserved for the OS [V]. Disabled while a game registered Raw Input with `RIDEV_NOHOTKEYS` has focus [V] |
| **`WH_KEYBOARD_LL` / `WH_MOUSE_LL`** | The system calls your callback in your own thread for every input event, before it is delivered [V] | No ("not injected into another process") [V] | Optional (return non-zero) | Yes (`WH_MOUSE_LL`) | Blocked by UIPI unless UIAccess [V] | Silently removed if the callback exceeds `LowLevelHooksTimeout`, capped at 1,000 ms since Windows 10 1709 [V]. Every keystroke and mouse move in the game waits on your thread, so a garbage-collection pause or stall adds input lag. Microsoft says to prefer Raw Input [V] |
| **Raw Input + `RIDEV_INPUTSINK`** | Asynchronous `WM_INPUT` delivery even when not in the foreground; needs `hwndTarget` [V] | No | No | Yes | Blocked by UIPI unless UIAccess [V] | `RIDEV_EXINPUTSINK` is useless here because it only delivers input the foreground app does not process, and games use Raw Input [V]. Registering for the mouse means a `WM_INPUT` for every movement (thousands per second on high-polling mice) [U] |
| **`GetAsyncKeyState` polling** | Read the physical key state on a timer. OBS polls every 25 ms on a hotkey thread [V, OBS source] | No | No | Yes (`VK_XBUTTON1/2`) [V] | Returns 0 when "UIPI prevents the calling thread from accessing the foreground thread" [V] | Can miss presses shorter than the poll interval [U]. Looks like a keylogger to antivirus heuristics [U] |
| **In-context hooks** (`WH_KEYBOARD`, `WH_GETMESSAGE`, `SetWinEventHook(WINEVENT_INCONTEXT)`) | Hook DLL mapped into other processes [V] | **Yes, effectively DLL injection** | — | — | — | **Never use.** BattlEye logs "Blocked loading of file" when a game process tries to load foreign DLLs [V] |

Notes:
- **UIPI (official).** Microsoft's UIAccess policy page lists what a UIAccess process gains: it can "read input for all integrity levels by using low-level hooks, raw input, GetKeyState, GetAsyncKeyState, and GetKeyboardInput" [V]. So a normal, non-elevated Kairo **cannot** see input going to an elevated game through hooks, Raw Input or polling. UIAccess needs a signature that chains to a trusted root, plus installation under `%ProgramFiles%` or `%WinDir%` [V].
- **`RegisterHotKey` under UIPI.** LibreAutomate's documentation of its `RegisterHotKey` wrapper says: "Most single-key and Shift+key hotkeys don't work when the active window has higher UAC integrity level … Media keys may work" [S]. Medal's default is single-key **F8**, and its support article says "Some games require Medal to run with administrator privileges … This can resolve hotkey recognition issues" (modified 2025-09-25) [V]. That fits the UIPI explanation [U].
- **Games that disable other apps' hotkeys.** `RIDEV_NOHOTKEYS` means "application-defined keyboard device hotkeys are not handled" while that app has focus [V]. SDL 3.4.0 added `SDL_HINT_WINDOWS_RAW_KEYBOARD_EXCLUDE_HOTKEYS` (default off), which "blocks any hotkeys that have been registered by applications" [V]. The Witness used this flag to block the Windows key [S, Casey Muratori]. In such games `RegisterHotKey` never fires. A Raw Input sink would still see the keys, unless the game is elevated.
- **Which frameworks use what.** Tauri's `global-hotkey` crate calls `RegisterHotKey` with `MOD_NOREPEAT` [V, source]. Electron's `globalShortcut` goes through Chromium's `GlobalAcceleratorListener` → `SingletonHwndHotKeyObserver` → `RegisterHotKey` [V, source]. Picking either framework gives you `RegisterHotKey` semantics.
- **Anti-cheat stance on hooks and macros.**
  - BattlEye: "Generally we only ever ban for the use of actual cheats/hacks … We might decide to kick (not ban) you … for using a specific program (such as macro tools)" [V].
  - EA Javelin's help page approves PowerToys Keyboard Manager, Steam Input, KeyMapper, Kanata and KMonad, and blocks reWASD and Daemon Tools from running alongside protected games [V].
  - I found **no primary source** saying any of the six anti-cheats bans for a passive low-level hook. The risk lies in *injecting* input or loading code, which Kairo never does [U].
  - AutoHotkey's FAQ notes that games protected by GameGuard or HackShield often block it entirely [V].

### 1.2 Mouse side buttons

- `RegisterHotKey` cannot bind mouse buttons. Options are `GetAsyncKeyState(VK_XBUTTON1/2)` polling (what OBS does: Mouse 4 and 5 are mapped, polled every 25 ms) [V], a Raw Input mouse sink, or `WH_MOUSE_LL`.
- Recommendation: poll every 8–10 ms **only while a mouse button is bound** [U on the interval choice]. Keep `WH_MOUSE_LL` off the default path because it puts Kairo's thread inside every mouse movement of the game.
- Passive methods do **not** swallow the click, so the game (and Discord push-to-talk) still sees it. Tell users to pick an unbound button.
- All mouse methods fail for elevated games (UIPI) [V].

### 1.3 Conflicts: what competitors use and their defaults

| Product | Default clip / replay key | Other defaults | Mechanism (where known) |
|---|---|---|---|
| **Medal** | **F8** (single key) [V] | Alt+F7 long recording; F8 becomes the bookmark key during long recording [V] | Not documented. Advises running as admin [V] |
| **NVIDIA app** | **Alt+Shift+F10** per NVIDIA's launch article (2024-11-12) [V]. Many guides say Alt+F10 saves and Alt+Shift+F10 toggles [S]; **this conflicts**, check the installed app | Alt+Z overlay, Alt+F9 record, Alt+F1 screenshot, Alt+F2 Photo Mode [V] | Not documented |
| **AMD Adrenalin** | Ctrl+Shift+S (save replay) [S] | Ctrl+Shift+E / Ctrl+Shift+R / Ctrl+Shift+I [S; varies by version]. AMD: "hotkeys require the use of CTRL and/or ALT keys" [V] | Not documented. The modifier requirement fits UIPI-robust `RegisterHotKey` use [U] |
| **Xbox Game Bar** | **Win+Alt+G** ("record that"; last 30 s by default, adjustable) [S: search snippet of Microsoft's support page; the direct fetch redirected] | Win+Alt+R record, Win+Alt+PrtScn screenshot, Win+G open; needs Game Bar ≥ 5.721.7292.0 [S, same source] | OS level; Win+key combinations are reserved for the OS [V] |
| **OBS Studio** | none (user-defined) | — | `GetAsyncKeyState` polling every 25 ms; supports Mouse 1–5 [V] |
| **Steam Game Recording** | A clip-save shortcut was added in a Sept 2024 beta; default unknown [S] | Ctrl+F11 for on-demand recording [S]; F12 screenshot (long-standing default) [U] | — |

Keys also taken by the OS:
- PrintScreen launches the Snipping Tool and can only be overridden while one of *your* windows is in the foreground [V].
- F12 is reserved for debuggers [V].
- Alt+F4, Alt+Tab, Alt+Enter and Shift+Tab (Steam overlay) are effectively off limits [U].

**Suggested Kairo defaults (to validate in beta):**
- **Clip: Alt+F8.** It uses a modifier (survives UIPI), Alt does nothing in most shooters (Ctrl is often crouch), and it avoids the NVIDIA, AMD, Game Bar, Medal and Steam keys listed above [U].
- **Screenshot: Alt+F6** [U].
- Avoid Ctrl+Alt+letter defaults. On layouts that use AltGr for letters (e.g. Romanian ș/ț/ă), AltGr = Ctrl+Alt, so these would fire while typing in game chat [U, standard Windows behaviour].
- On first run, show a "press your clip key" capture. If `RegisterHotKey` fails, say the key is already in use [V on the failure semantics].
- Show a warning when NVIDIA or AMD replay hotkeys are detected (installed software) [U].

---

## 2. Controller triggers in the background

### 2.1 Which APIs deliver input while Kairo is unfocused?

| API | Background input? | Xbox Share / Guide | Notes |
|---|---|---|---|
| **Windows.Gaming.Input (WGI)** | **No.** SDL's driver: "The axes are all zero when the application loses focus" [V]. Unity: WGI "blocks polling when the window isn't focused" [S]. SDL's original WGI commit listed "can't get events in the background" [S] | No Guide [S] | Avoid for triggers |
| **XInput 1.4** | **Conflicting.** Microsoft: `XInputEnable` is "Deprecated [on Windows 10+], as game controller input is automatically enabled/disabled by the system based on the application window focus" [V]. Unity tells developers to switch to XInput as the workaround for background input [S]. **Test on the PC** | No Share. Guide only through the undocumented ordinal-100 `XInputGetStateEx`, which SDL uses ("we think") [V] | At most 4 controllers; Xbox-compatible devices only |
| **Microsoft GameInput** (NuGet `Microsoft.GameInput`) | **Yes, opt-in.** Default is "GameInput will not provide background input". `SetFocusPolicy(GameInputEnableBackgroundInput = 0x40)` enables it [V] | `GameInputEnableBackgroundGuideButton` (0x80) and `GameInputEnableBackgroundShareButton` (0x100), with `IGameInput::RegisterSystemButtonCallback` for press/release [V]. The focused game can claim exclusivity (`ExclusiveForegroundShareButton`), and then Kairo gets nothing [V] | Windows 10 19H1+ and Windows 11 [V]. The v1+ runtime requires the redistributable MSI or `winget install Microsoft.GameInput` (elevated) [V] |
| **Raw Input (HID gamepad usage) + `RIDEV_INPUTSINK`** | Yes, unless the foreground is elevated (UIPI) [V] | Depends on the device | You parse HID reports yourself |
| **Direct HID read** (hidapi / `CreateFile` + `ReadFile`) | Yes. Not a window-message path, so probably not subject to UIPI [U] | PlayStation Create and PS buttons are in the input report [V, SDL mapping] | Needs per-device parsers. HidHide (DS4Windows users) can hide devices [S] |

**GameInput status, 2025–2026 [V]:**
- **2025-03-17:** v1.0 moves to NuGet with a redistributable "detached from the GDK and Windows Update"; adds HID registry mappings and remote-session support.
- **2025-09-16:** v2.2 adds advanced haptics, motion sensors and "background input support for all XInput devices"; the redistributable MSI works under Wine. Its notes also say: "Fixed background input not working on gamepads plugged in after the input policy is specified."
- **2025-12-02:** v3.1 brings console/PC parity from the October 2025 GDK.
- **2026:**
  - 3.3 adds paddles and native ARM.
  - 3.4 adds raw HID reports.
  - 3.5 adds "Agility SDK style side-by-side deployment", "XInput and background GIP raw device reports", a DualSense companion header, and an MIT licence for the header and lib.
  - **4.0** (4.0.300 on 2026-10-15, 4.0.302 on 2026-10-19) adds Switch Pro support, side-by-side binaries, touch devices and API version 4.
- The GameInput FAQ (2026-08-11) adds that Unreal Engine 5.4+ ships GameInput plug-ins.
- Whether side-by-side deployment removes the need for the system-wide MSI and service on PC is **not confirmed** [U].

### 2.2 DualSense / DualShock and Steam Input

- **Hazard: never write to PlayStation controllers.** SDL documents that enhanced reports "break Windows DirectInput for other applications that don't use SDL" and "can't be disabled on PlayStation controllers without power cycling" [V]. SDL's PS5 driver says even reading feature reports (serial number, firmware) "will also enable enhanced reports over Bluetooth", and "We can't even send an invalid effects packet, or it will put the controller in enhanced mode" [V].
  - **So Kairo reads input reports passively and never sends output or feature reports.** That means no rumble and no LED changes.
- **Report formats.** Bluetooth simple report 0x01 vs enhanced 0x31; USB is 64 bytes [V, SDL]. Kairo must parse both, because Steam may already have switched the controller to enhanced mode [S, Steam forum reports and a Wine developer].
- **Steam Input** presents remapped or virtual devices to games. Reading the physical HID device in parallel works because each HID handle gets its own input copy [U]. Users of DS4Windows with **HidHide** may need to allow Kairo explicitly [S].
- GameInput on PC also exposes DualSense (haptics since v2.0, gyro since v3.3) [V]. Whether GameInput flips a Bluetooth DualSense into enhanced mode when merely read is **unknown**. Test this before relying on it.

### 2.3 Detecting a held Share/View button or a combo

- **Xbox Series controllers:** Share through `RegisterSystemButtonCallback` with the background Share flag [V]. Measure the hold time, e.g. 500–800 ms, to avoid accidental triggers [U].
  - Xbox Game Bar also uses the controller buttons (Xbox button opens it; Share captures by default) [U]. Pressing Share could produce **two** captures. Let users choose Share vs a chord, and explain how to turn off Game Bar capture.
- **Xbox One controllers** have no Share. Use a chord with a hold, e.g. View + RB held for 600 ms [U]. Avoid "View alone" because View is the scoreboard or map in many games.
- **DualSense:** Create maps to bit 0x10 of buttons byte 1 and PS to bit 0x01 of byte 2 in SDL's mapping [V]. Steam often uses the PS button for Big Picture [U].
- Design all chords as **"held ≥ N ms" + debounce + per-game enable**. Make controller triggers opt-in during beta.

---

## 3. Voice trigger ("clip that")

| Engine | License / cost | Footprint | Custom phrase | Notes / verdict |
|---|---|---|---|---|
| **Picovoice Porcupine** | SDK repo Apache-2.0 [V], but every SDK needs an AccessKey. The Free Tier ended **2026-06-30**; Picovoice: "The SDK does not run without a valid key", "There is no non-commercial tier planned" [S, quoted reply] | Small | Trained in Picovoice Console | **Reject.** Enterprise-only now; validates the key at startup (network) |
| **openWakeWord** | Code Apache-2.0; **pre-trained models CC BY-NC-SA 4.0** [V] | 15–20 models run in real time on **one Raspberry Pi 3 core** [V] | Train with synthetic speech in under an hour (Colab) [V] | Fine for a private beta with your own trained model. Pre-trained models cannot ship commercially. Last release v0.6.0 (2024-02-11) [V] |
| **sherpa-onnx keyword spotting** | Apache-2.0 [V] | English model `sherpa-onnx-kws-zipformer-gigaspeech-3.3M-2024-01-01` [V] | **Any keywords without retraining**: `keywords.txt` with a boosting score and trigger threshold [V] | **Best fit.** C/C++/C# APIs, Windows supported [V] |
| **Vosk** | Apache-2.0 (most models) [V] | `vosk-model-small-en-us-0.15` = **40 MB**; runs on Raspberry Pi and Android [V] | Small models allow runtime vocabulary changes [V] | Good fallback; restrict the grammar to "clip that" and "[unk]" [U on the exact API form] |
| **whisper.cpp** | MIT [V] | tiny 75 MiB disk / ~273 MB RAM; base 142 MiB / ~388 MB [V]. The stream example re-samples about every 0.5 s [V] | Free text | Too heavy as an always-on listener while gaming, and GPU paths compete with the game [U]. Possible second-stage check after keyword spotting |
| **Windows.Media.SpeechRecognition** | Built into Windows | — | List or SRGS constraints run on the device. Predefined dictation and web-search grammars use "a remote web service" and need online speech recognition enabled [V] | Usable from desktop apps; continuous recognition with a list constraint [U on desktop-app details] |
| **System.Speech / SAPI** | Built in (.NET NuGet) | — | Grammars | Legacy. **Windows Speech Recognition is deprecated (Dec 2023), replaced by voice access** [V]. Whether the desktop recognisers ship in future builds is unclear [U] |

**CPU while gaming.** The keyword-spotting engines above are designed for single-board computers, so expect well under 1 core on a modern desktop CPU [U, extrapolated from the Raspberry Pi 3 figures]. Measure with a game running. Gate listening to "only while a game is detected" plus a voice-activity detector.

**Privacy.** Process audio 100% locally and keep no recordings. Make it opt-in, show a visible "listening" state, and pause outside games. WASAPI shared-mode capture coexists with Discord and in-game voice chat [U]. Speaker bleed can cause false triggers, so require a confidence threshold and add a short cooldown.

**Verdict:** post-v1 (v1.1 or v2). Prototype with sherpa-onnx.

---

## 4. Anti-cheat compatibility

### 4.1 Principle: stay out of the game process

- Microsoft's DirectX team (2019-12-17): with true fullscreen exclusive, an overlay app must "step into and intercept the rendering process", which can cause "performance regressions, instability and issues with anti-cheat". With fullscreen optimizations, "the DWM reassumes control" and composes overlays safely [V].
- **Windows Graphics Capture and DXGI Desktop Duplication read from the compositor or desktop, not from the game.** I found **no primary-source report** of a ban for WGC/DXGI-based capture [U: absence of evidence, high-traffic topic]. Recent breakages all involve **injection**:

| Date | Incident | Lesson |
|---|---|---|
| Oct 2023 | Valve: AMD Anti-Lag+ "detours" CS2 engine DLL functions, which led to VAC bans. Valve reversed them after AMD pulled the feature (driver 23.10.2) [S: GGRecon, PCGamesN] | Even a GPU vendor's signed code got bans for hooking game functions |
| 2024–2025 | OBS 31 changed the code-signing certificate of its game-capture hook. Riot's anti-cheat "does not yet allow an exception for it", and FACEIT needed an update [S: OBS forum moderator and users] | Injected DLLs live or die by anti-cheat allow-lists tied to certificates |
| 2026-07-21 | A Vanguard update turned OBS **Game Capture** black in VALORANT and LoL. OBS advised using Window Capture meanwhile; Riot pushed a fix [S: win.gg 2026-07-22] | Out-of-process window or display capture kept working |
| 2026-10-06 → Apr 2027 | **Riot blocks game-memory access by default for all unknown third-party apps** (League, TFT, VALORANT). Approved tools have until April 2027; afterwards "the official API [is] the only allowed mechanism for retrieving game data". "This change does not result in bans by itself" [V, Riot 2026-09-29] | Auto-highlights must use official APIs, never memory |

### 4.2 Vendor by vendor (what could be verified)

- **Riot Vanguard (VALORANT, LoL).**
  - Kernel driver; blocks files from loading and shows "Vanguard has blocked the following file from loading" notifications [V, 2026-06-02].
  - Third-party policy (2025-02-10): no apps that expose obfuscated information, bot or script, or "draw conclusions for you during gameplay"; apps on the official Riot API "should rarely cause issues" [V].
  - Plus the 2026 memory-access block above [V].
  - League's official local **Live Client Data API** at `https://127.0.0.1:2999/liveclientdata/…` (`eventdata`, `allgamedata`; self-signed certificate) is the sanctioned event source [V].
- **Easy Anti-Cheat (Fortnite, Apex, …).** I could not find official EAC documentation on overlays or allow-lists. Only forum evidence: OBS hook failures in EAC games, and an RTSS allow-list that lagged new builds (Guru3D, 2016) [S]. **Gap.**
- **BattlEye (R6 Siege, PUBG, …).**
  - FAQ: bans only "for the use of actual cheats/hacks or components of such hacks". Macro tools may get kicks, not bans. "Blocked loading of file" messages carry no ban risk and mean "the game process tried to load them". Reshade-type overlays are "generally supported". It blocks known-vulnerable kernel drivers [V].
  - A Ubisoft-hosted BattlEye FAQ says Discord, TeamSpeak, Fraps, OBS and XSplit are not blocked [S].
- **FACEIT (CS2).** Official pages blocked (HTTP 403). Forum evidence only: FACEIT needed updates for the new OBS certificates, and OBS Game Capture historically failed with FACEIT running [S]. **Gap.**
- **Ricochet (Call of Duty).** Activision's enforcement-policy page timed out. Secondary coverage: a kernel driver active only while CoD runs; detects recoil-assist hardware (warnings, then bans) [S]. **Gap.** No evidence about recorders either way.
- **EA Javelin (Battlefield 6, EA FC).**
  - Software or driver denylist "for security reasons". Examples: vulnerable RGB and monitoring drivers (ASUS Aura, CPU-Z, Core Temp…), reWASD and Daemon Tools (virtual devices). Approved alternatives: Kanata, KMonad, KeyMapper, PowerToys Keyboard Manager and Steam Input. Secure Boot guidance for unsigned-driver errors [V].
  - BF6 anticheat update (2026-08-28): prohibits "Overlays that provide artificial zoom enhancements, highlighting targets". Warns about products claiming a "Compatibility Mode". **TPM enforcement** is coming; VBS/HVCI are required but not yet enforced [V].
- **Discord.** Its detectable-apps list carries per-game overlay fields. `overlay: true` appears for only 141 of 24,780 apps; it is true for VALORANT and LoL and false for CS2, Apex, Battlefield 6 and Call of Duty [V on the data]. **The field semantics are undocumented** [U]. Claims that Discord's 2025 overlay "no longer injects" come from a third-party blog only [S]. **Medal and Overwolf vendor statements: not found or verified.**

### 4.3 Technique risk matrix for Kairo

| Technique | Risk | Kairo rule |
|---|---|---|
| DLL injection or graphics-API hooks (Present/swapchain) | **High:** blocks, black screens, possible flags; needs per-anti-cheat allow-listing by certificate | **Never** |
| In-context Windows hooks (`WH_KEYBOARD`, `WH_GETMESSAGE`, `WINEVENT_INCONTEXT`) | High: Windows maps your DLL into the game | **Never** |
| Reading or writing game memory, `PROCESS_VM_*` handles | **High:** Riot now blocks it; classic cheat signature | **Never.** Open games with `PROCESS_QUERY_LIMITED_INFORMATION` only (allowed even for protected processes; enough for `QueryFullProcessImageName`) [V] |
| Synthetic input (`SendInput`), virtual HID or controller drivers, kernel drivers | High: Javelin blocks reWASD and virtual buses; Vanguard blocks vulnerable drivers [V] | **Never** |
| Low-level hooks / Raw Input sink / key polling | Low ban risk [U]; antivirus "keylogger" heuristics [U]; UIPI limits [V] | Allowed, minimal, documented in the privacy policy |
| WGC / DXGI capture, separate topmost window | Lowest [U, no counter-evidence found] | Default |
| Official local game APIs (League Live Client Data, CS2 Game State Integration) | Low; sanctioned | For future auto-highlights |

### 4.4 Does code signing matter?

- **For injected DLLs, critically.** Anti-cheats allow-list by certificate; OBS 31's certificate change broke capture until whitelists were updated [S].
- **For UIAccess, it is mandatory** (trusted root + Program Files) [V].
- **For drivers**, required: Javelin's unsigned-driver error and Secure Boot guidance; BattlEye needs SHA-2 signature support [V].
- **For Kairo's plain user-mode exe**, signing is not documented as an anti-cheat requirement [U], but it matters for SmartScreen and antivirus reputation. Kairo combines screen capture + keyboard polling + microphone, which looks spyware-like to heuristics [U].
- Microsoft's managed service is now named **Artifact Signing** (formerly Trusted Signing): Basic includes 5,000 signatures/month and Premium 100,000; the page did not render prices [V].

---

## 5. "Clip saved" feedback in game

### 5.1 Sound cue (primary feedback)

- Short (under 300 ms), distinct, with adjustable volume, played in WASAPI shared mode on the default output [U on the parameters].
- **Keep cues out of clips.** Capture system audio as "everything except Kairo" with `AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK` + `PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE` [V].
  - Microsoft documents the minimum as **Windows 10 build 20348** [V]. OBS gates this at **19041** with the comment "MS says 20348, but process filtering seems to work earlier" [V, OBS source].
  - Windows 10 22H2 (build 19045) should therefore work. **Test it.**

### 5.2 Visual toast (optional, small)

- Use a separate top-level window: `WS_EX_TOPMOST | WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW` [V on the style semantics]. Show it with `SW_SHOWNOACTIVATE` and `SWP_NOACTIVATE` [U]. **Never** call `SetForegroundWindow`.
- Call `SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE)` (Windows 10 2004+) so the toast never appears in Kairo's own monitor captures [V].
- **Borderless or fullscreen-optimized flip-model games:** the overlay shows. With desktop content on top, the DWM "can either seamlessly transition back to composed mode … 'reverse compose' … or leverage MPO to maintain the independent flip mode" [V]. Keep the toast short (≤ 1.5–2 s) to minimise any latency impact [U].
- **True exclusive fullscreen:** overlays need render interception [V]. Detect it with `SHQueryUserNotificationState() == QUNS_RUNNING_D3D_FULL_SCREEN` ("A full-screen (exclusive mode) Direct3D application is running") [V] and **fall back to sound only**.
- Show the toast on the monitor that contains the game's window [U].

### 5.3 Windows toast notifications

- Windows 11 "Turn on do not disturb automatically" rules include **"When playing a game"** and **"When using an app in full-screen mode"**. While Do Not Disturb is on, only "alarms, reminders, and apps of your choice" show banners [V].
- Whether these rules are on by default is not stated on Microsoft's page [U].
- Use toasts only for after-session summaries (e.g. "5 clips saved"), never as the in-game confirmation.

### 5.4 Controller rumble

- Not for v1:
  - XInput output is documented as focus-gated on Windows 10+ [V].
  - Rumble collides with the game's own rumble [U].
  - On PlayStation controllers, any output report risks the enhanced-mode problem in section 2.2 [V].
- Revisit with GameInput haptics later.

### 5.5 What competitors do

- NVIDIA and AMD show their own overlay notifications; Game Bar uses system UI [U, general knowledge, not re-verified].
- Medal documents bookmarks and clip lengths, but its support pages don't describe notification behaviour [V on that absence].

---

## 6. Game detection

### 6.1 Runtime signal (no injection)

- `SetWinEventHook(EVENT_SYSTEM_FOREGROUND, …, WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS)`. The out-of-context callback "is not mapped into the address space of the process that generates the event", and events are queued and delivered on your thread, which needs a message loop [V].
- On each foreground change:
  1. `GetWindowThreadProcessId`
  2. `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)`
  3. `QueryFullProcessImageNameW` [V]
  4. Match the path against the game database.
- Supplement with a process-list poll every 2–5 s (Toolhelp snapshot gives exe names without opening handles) to catch game start and exit [U].
- Fullscreen heuristics:
  - `QUNS_BUSY` / `QUNS_RUNNING_D3D_FULL_SCREEN` [V].
  - Window rectangle equals the monitor rectangle [U].
  - Per-process GPU engine utilisation. Task Manager's GPU data comes from VidSch/VidMm and needs WDDM 2.0+ drivers [V]; the PDH "GPU Engine" counter naming was not verified here [U].
  - **Do not** enumerate loaded modules (looking for d3d11/vulkan) in game processes: that needs `PROCESS_VM_READ`, which anti-cheats watch [V on the access rights].
- Windows' own `GameList` API (`Windows.Gaming.Preview.GamesEnumeration`) requires the restricted `gameList` capability. "Unless your developer account is specially provisioned by Microsoft, calls … will fail" [V]. Not usable.

### 6.2 Launcher libraries (installed-games index → path prefix → game)

| Launcher | Where to read (verified in Playnite source unless noted) |
|---|---|
| Steam | `HKCU\Software\Valve\Steam\SteamPath` → `<Steam>\steamapps\libraryfolders.vdf` (KeyValues; each library's `path`) → `steamapps\appmanifest_<appid>.acf` (`installdir`) [V]. `HKCU\…\Steam\RunningAppID` as a "currently running" signal is widely cited but **not verified** [U] |
| Epic | `%ProgramData%\Epic\EpicGamesLauncher\Data\Manifests\*.item` (JSON: `DisplayName`, `InstallLocation`, `LaunchExecutable`, `AppName`); `%ProgramData%\Epic\UnrealEngineLauncher\LauncherInstalled.dat` [V] |
| Ubisoft Connect | `HKLM\SOFTWARE\ubisoft\Launcher\Installs\<id>` (32-bit registry view) [V]; value name `InstallDir` [U] |
| GOG Galaxy | Client: `HKLM\SOFTWARE\WOW6432Node\GOG.com\GalaxyClient\paths` [V]. Games under `…\GOG.com\Games\<id>` [U] |
| Battle.net | Playnite matches products to Windows **Uninstall** registry entries [V] |
| EA app | **No verified method.** Playnite's library repo (2026-09-21) has no EA/Origin library [V]. Fall back to Uninstall entries plus the exe database [U] |
| Xbox app / Game Pass | Installed games are packaged apps. Playnite enumerates installed packages and matches package family names against the user's Xbox title history (needs sign-in) [V]. GDK titles must ship a **`MicrosoftGame.config`** [V], so its presence in the package's install folder is a strong local "this is a game" signal [U] |

### 6.3 Discord's detectable list: facts and terms

- `GET https://discord.com/api/v9/applications/detectable` returned **HTTP 200 without authentication**: 13.0 MB JSON, **24,780 entries**, 10,459 with a Windows executable (11,167 win32 executable rules; 15 with `arguments`) [V, fetched 2026-10-21].
- Fields: `executables[{os, name, is_launcher, arguments}]`, `third_party_skus` (steam 19,785, xbox 7,830, epic 2,569, …), `aliases`, `overlay*` [V].
- **Not in Discord's API docs:** no mention in the `discord/discord-api-docs` repo at commit 2026-10-19 [V].
- Discord's Terms of Service (effective 2025-09-29) prohibit "scraping our services without our written consent, including by using any robot, spider, crawler, scraper, or other automatic device, process, or software" [V].
- The Developer Terms and Policy pages returned 403 and could not be read [—].
- **Verdict:** don't fetch it from the shipped app, and don't redistribute it. At most, ask Discord for permission, or use it for one-off manual research [U on legal interpretation; not legal advice].

### 6.4 Cloud and console

- **Cloud** (Xbox Cloud Gaming in a browser or the Xbox app, GeForce NOW) runs as a normal window. Window capture works, and **no local anti-cheat is involved** [U].
  - Detection: cloud client exe names, plus browser window titles via `GetWindowTextW` [U].
  - Controller triggers through GameInput still work, because the browser also reads the pad [U].
- **Console** clips need a capture card. That is out of scope for triggers.

### 6.5 Metadata and art

- **IGDB** [V, api-docs.igdb.com fetched 2026-10-21]:
  - Twitch app → client ID + secret → `client_credentials` app token (example `expires_in` ≈ 5.59 M s ≈ 65 days).
  - **4 requests/s**, max **8 open requests**, HTTP 429 on excess. No browser CORS.
  - "free for non-commercial usage under the terms of the Twitch Developer Service Agreement". Commercial use is allowed through a (free) partnership with "user facing attribution". Caching is encouraged.
  - Field renames ran Feb 18 → Aug 31; the page gives no year, probably 2025 (e.g. `external_game.category` → `external_game_source`).
  - **Consequence:** the secret can't ship in the desktop app. Use a tiny proxy, or ship a pre-built local database.
- **SteamGridDB** [V, OpenAPI 2.10.0]:
  - Bearer API key per account; look up by `steam / origin / egs / bnet / uplay / flashpoint / eshop` IDs. No rate limit is stated.
  - Its own spec warns: "Do not rely on this data to be always up to date or accurate".
  - Terms page not readable (JavaScript) [—]. Assets are community uploads, so licensing for commercial redistribution is unclear [U].
- **Steam** [V]:
  - Steam Web API Terms (last updated 2010): **100,000 calls/day**; keep the key confidential; no implied Valve endorsement.
  - The storefront `store.steampowered.com/api/appdetails` endpoint is not named in those terms [V]. Its rate limits are undocumented [U].

---

## 7. Recommended v1 approach

### Triggers

1. **Keyboard (v1):** `RegisterHotKey` with `MOD_NOREPEAT` through the chosen framework.
   - Defaults must use Alt or Ctrl (candidates Alt+F8 clip, Alt+F6 screenshot).
   - Treat a registration failure as "key in use". Support per-game overrides later.
2. **Mouse side buttons (v1, opt-in):** `GetAsyncKeyState` polling at ~8–10 ms on a dedicated thread, only when a mouse button is bound. No `WH_MOUSE_LL`.
3. **Elevated-game handling (v1):**
   - Detect it: opening the game's token fails, or the game is known to need admin.
   - Show "This game runs as admin: Alt/Ctrl hotkeys work; mouse/controller triggers need Kairo elevated."
   - **v1.1:** an optional "start Kairo elevated" setting via a scheduled task.
   - **Later:** UIAccess, only after code signing and a Program Files install.
4. **v1.1 fallback:** a Raw Input keyboard sink that detects the same chord when a game uses `RIDEV_NOHOTKEYS`. De-duplicate events within ~100 ms.
5. **Controller (v1.1, beta opt-in):**
   - GameInput 4.x with `SetFocusPolicy(EnableBackgroundInput | EnableBackgroundShareButton | EnableBackgroundGuideButton)`.
   - `RegisterSystemButtonCallback` for Share; hold-threshold chords for everything else.
   - Passive read-only HID for DualSense/DS4 if GameInput proves unsuitable. Never write to PlayStation controllers.
6. **Voice (v2):** sherpa-onnx keyword spotting (or Vosk grammar mode). Local only, opt-in, active only while a game is detected.

### Feedback

- Sound cue on by default, excluded from captured audio by process-loopback exclusion.
- Small, non-activating, click-through, capture-excluded toast for ≤ 2 s; skip it in exclusive fullscreen.
- No Windows toasts during play. No rumble.

### Safety rules (hard rules; put them in CONTRIBUTING / CLAUDE.md for the coding agent)

1. No code ever runs inside a game process: no DLL injection, no graphics hooks, no in-context Windows hooks, no `WINEVENT_INCONTEXT`.
2. Capture only via WGC / DXGI Desktop Duplication.
3. Game process handles: `PROCESS_QUERY_LIMITED_INFORMATION` only. No `ReadProcessMemory`, no module enumeration, no handle duplication.
4. No synthetic input, no virtual HID or controller devices, no kernel drivers.
5. Game events only from official local APIs, e.g. League Live Client Data API and CS2 Game State Integration (CS2 GSI was not re-verified in this pass).
6. Overlays only as separate top-level windows.
7. Kairo runs non-elevated by default. Sign releases before the public launch. No packers or obfuscators.
8. Keep a compatibility matrix and re-test after anti-cheat updates: VALORANT/LoL (Vanguard), Fortnite/Apex (EAC), R6/PUBG (BattlEye), CS2 + FACEIT, CoD (Ricochet), BF6 (Javelin).

### Game detection

- Foreground WinEvent hook + process poll → path match against Kairo's own database. Build the database from:
  - Steam, Epic, Ubisoft and GOG manifests and registry.
  - Xbox packaged apps with `MicrosoftGame.config`.
  - A curated executable list.
  - "Mark this app as a game" in the UI.
- Heuristic fallback: fullscreen or borderless + GPU 3D load.
- Metadata in v1: exe icon + launcher names.
- Later: IGDB through a proxy, or a bundled snapshot. SteamGridDB only with the user's own key.
- **Do not depend on Discord's endpoint.**

### Test plan on the Windows PC (first spike)

- (a) `RegisterHotKey` Alt+F8 and plain F8 with (i) a normal game, (ii) an elevated window such as Task Manager or a game run as admin, (iii) an SDL 3.4 test app with the exclude-hotkeys hint.
- (b) XInput vs GameInput background reads; Share callback; Game Bar double-capture.
- (c) DualSense over Bluetooth with Steam running: passive read of both report formats; confirm Kairo never switches the mode.
- (d) Process-loopback exclusion on Windows 10 22H2 (19045) and Windows 11.
- (e) Toast visibility and frame-time impact (PresentMon) in borderless vs exclusive fullscreen.
- (f) Kairo running next to each anti-cheat title, watching for warnings or blocks.

---

## 8. Decisions needed from the user

1. Default clip and screenshot keys (Alt+F8 / Alt+F6?), keyboard layout, and whether Kairo may "eat" that key from the game.
2. Do you play any game that runs as administrator? If so, is an optional "run Kairo elevated" mode acceptable, or should we plan for code signing + UIAccess?
3. Which controllers (Xbox Series / Xbox One / DualSense / DS4 / Switch Pro / 8BitDo)? Through Steam Input or DS4Windows? Is a controller trigger needed in v1 or v1.1?
4. Voice trigger: wanted at all? English only, or also another language? OK with always-local, opt-in listening?
5. Which anti-cheat games do you and your beta friends play? This defines the test matrix.
6. Feedback defaults: sound on? Visual toast on? Should the toast ever appear in clips (some creators want a watermark)?
7. Buffer only while a game is detected, or always (desktop too)?
8. Is a tiny Kairo backend acceptable in beta (needed for IGDB secret handling), or must v1 stay 100% offline?
9. Which cloud services (Xbox Cloud Gaming, GeForce NOW…)? Is console capture through a capture card in scope at all?

---

## 9. Caveats and unverified items

- How `RegisterHotKey` behaves under UIPI comes from LibreAutomate's documentation, not Microsoft's. Verify on the PC.
- XInput background behaviour on Windows 10/11 desktop apps: the official documentation and practitioner reports conflict.
- Whether GameInput needs the system MSI vs side-by-side deployment on PC, and whether it changes DualSense report modes: unverified.
- NVIDIA's replay key: NVIDIA's own article (Alt+Shift+F10 saves) conflicts with most guides (Alt+F10 saves). AMD defaults come from third parties.
- Picovoice's Free Tier shutdown is sourced from a community post quoting Picovoice support. The pricing page could not be rendered.
- No official EAC, FACEIT or Ricochet statements on recorders or overlays were retrievable. Medal and Overwolf vendor statements were not found.
- Discord's detectable-list field meanings and permission status: undocumented. Developer Terms not readable (HTTP 403).
- SteamGridDB terms not readable; Steam `RunningAppID`, GOG game keys and the EA app's paths not verified.

---

## Sources

Microsoft — input, windows, processes:
- https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-registerhotkey (updated 2025-07-01)
- https://learn.microsoft.com/en-us/windows/win32/winmsg/lowlevelkeyboardproc (2025-07-14)
- https://learn.microsoft.com/en-us/windows/win32/api/winuser/ns-winuser-rawinputdevice (updated 2024-06-26)
- https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getasynckeystate (updated 2026-01-29)
- https://learn.microsoft.com/en-us/windows/security/threat-protection/security-policy-settings/user-account-control-only-elevate-uiaccess-applications-that-are-installed-in-secure-locations (archived; updated 2026-04-27)
- https://devblogs.microsoft.com/oldnewthing/20140213-00/?p=1773
- https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwineventhook
- https://learn.microsoft.com/en-us/windows/win32/procthread/process-security-and-access-rights (2025-07-14)
- https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-openprocess
- https://learn.microsoft.com/en-us/windows/win32/api/shellapi/ne-shellapi-query_user_notification_state
- https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowdisplayaffinity
- https://learn.microsoft.com/en-us/windows/win32/winmsg/extended-window-styles
- https://learn.microsoft.com/en-us/windows/win32/direct3ddxgi/for-best-performance--use-dxgi-flip-model (updated 2026-04-15)
- https://devblogs.microsoft.com/directx/demystifying-full-screen-optimizations/ (2019-12-17)
- https://devblogs.microsoft.com/directx/gpus-in-the-task-manager/ (2017-07-21)
- https://learn.microsoft.com/en-us/windows/win32/api/audioclientactivationparams/ns-audioclientactivationparams-audioclient_process_loopback_params
- https://learn.microsoft.com/en-us/samples/microsoft/windows-classic-samples/applicationloopbackaudio-sample/ (2026-02-19)
- https://support.microsoft.com/en-us/windows/notifications-and-do-not-disturb-in-windows-feeca47f-0baf-5680-16f0-8801db1a8466
- https://support.microsoft.com/en-us/help/4026387 (Game Bar shortcuts)
- https://learn.microsoft.com/en-us/windows/whats-new/deprecated-features (updated 2026-09-25)
- https://learn.microsoft.com/en-us/windows/apps/design/input/speech-recognition (updated 2026-03-04)
- https://learn.microsoft.com/en-us/windows/apps/design/input/define-custom-recognition-constraints
- https://learn.microsoft.com/en-us/uwp/api/windows.gaming.preview.gamesenumeration.gamelist
- https://learn.microsoft.com/en-us/gaming/gdk/docs/features/common/game-config/microsoftgameconfig-overview (updated 2026-05-12)
- https://azure.microsoft.com/en-us/pricing/details/trusted-signing/ (now "Artifact Signing")

Microsoft — controllers and GameInput:
- https://learn.microsoft.com/en-us/windows/win32/api/xinput/nf-xinput-xinputenable
- https://devdocs.xbox.com/reference/input/gameinput/enums/gameinputfocuspolicy
- https://devdocs.xbox.com/reference/input/gameinput/enums/gameinputsystembuttons.md
- https://devdocs.xbox.com/reference/input/gameinput/interfaces/igameinput/methods/igameinput_registersystembuttoncallback.md
- https://devdocs.xbox.com/reference/input/gameinput/interfaces/igameinput/methods/igameinput_setfocuspolicy.md
- https://learn.microsoft.com/en-us/gaming/gdk/docs/features/common/input/overviews/input-faq (2026-08-11)
- https://devdocs.xbox.com/build/core-features/common/input/overviews/input-overview.md
- https://devdocs.xbox.com/build/core-features/common/input/overviews/input-nuget
- https://www.nuget.org/packages/Microsoft.GameInput (4.0.302, 2026-10-19)
- https://developer.microsoft.com/en-us/games/articles/2025/03/gdc-2025-gameinput-pc-expands-to-meet-your-needs/ (2025-03-17)
- https://developer.microsoft.com/en-us/games/articles/2025/09/gameinput-pc-update-adds-haptics-motion-and-more/ (2025-09-16)
- https://developer.microsoft.com/en-us/games/articles/2025/12/gameinput-unifies-your-console-and-pc-workflows/ (2025-12-02)
- https://github.com/microsoftconnect/GameInput

Source code (fetched 2026-10-21):
- https://github.com/obsproject/obs-studio/blob/master/libobs/obs-hotkey.c
- https://github.com/obsproject/obs-studio/blob/master/libobs/obs-windows.c
- https://github.com/obsproject/obs-studio/blob/master/plugins/win-wasapi/plugin-main.cpp
- https://github.com/obsproject/obs-studio/blob/master/plugins/win-wasapi/win-wasapi.cpp
- https://github.com/tauri-apps/global-hotkey/blob/dev/src/platform_impl/windows/mod.rs
- https://github.com/electron/electron/blob/main/shell/browser/api/electron_api_global_shortcut.cc
- https://chromium.googlesource.com/chromium/src/+/main/ui/base/accelerators/global_accelerator_listener/global_accelerator_listener_win.cc
- https://chromium.googlesource.com/chromium/src/+/main/ui/gfx/win/singleton_hwnd_hot_key_observer.cc
- https://github.com/libsdl-org/SDL/blob/main/src/joystick/hidapi/SDL_hidapi_ps5.c
- https://github.com/libsdl-org/SDL/blob/main/src/core/windows/SDL_xinput.c
- https://github.com/libsdl-org/SDL/blob/main/src/joystick/windows/SDL_windows_gaming_input.c
- https://github.com/JosefNemec/PlayniteExtensions (source/Libraries; commit 2026-09-21)
- https://github.com/discord/discord-api-docs (commit 2026-10-19; no "detectable" endpoint)

SDL docs, practitioners, forums:
- https://wiki.libsdl.org/SDL3/SDL_HINT_WINDOWS_RAW_KEYBOARD_EXCLUDE_HOTKEYS
- https://wiki.libsdl.org/SDL3/SDL_HINT_JOYSTICK_ENHANCED_REPORTS
- https://git.axiodl.com/encounter/SDL/commit/aba279289685be3fec7cfd820ae66e2d09479efd (SDL WGI commit; cited via search snippet)
- https://issuetracker.unity.com/issues/9537/controller-input-is-ignored-when-editor-is-not-focused
- https://libreautomate.com/api/Au.More.RegisteredHotkey.Register.html
- https://www.autohotkey.com/docs/v2/FAQ.htm
- https://www.autohotkey.com/docs/v2/Program.htm
- https://caseymuratori.com/blog_0006
- https://steamcommunity.com/discussions/forum/1/3719440044279265784/ (DualSense enhanced-mode reports)

Competitors:
- https://support.medal.tv/support/solutions/articles/48000922120-no-game-detected-f8-isn-t-working- (2025-09-25)
- https://support.medal.tv/support/solutions/articles/48001157618-how-to-record-and-make-clips (2025-12-12)
- https://support.medal.tv/support/solutions/articles/48001271487-getting-to-know-medal-s-settings
- https://www.nvidia.com/en-us/geforce/news/nvidia-app-download-and-features/ (2024-11-12)
- https://www.amd.com/en/resources/support-articles/faqs/DH3-023.html
- https://digitaltrends.com/?p=3631680 (Steam Game Recording Ctrl+F11 claim; secondary)
- https://gamingonlinux.com/2024/09/steam-beta-adds-new-shortcut-key-to-save-a-clip-of-recent-gameplay

Anti-cheat and publishers:
- https://support.riotgames.com/en-us/riot/performance/game-memory-access-removed-for-third-party-apps (2026-09-29)
- https://support.riotgames.com/en-us/valorant/account/third-party-applications (2025-02-10)
- https://support.riotgames.com/en-us/riot/performance/what-is-vanguard (2026-06-02)
- https://developer.riotgames.com/docs/lol (Live Client Data API)
- https://win.gg/obs-game-capture-not-working-valorant-league-of-legends-fix/ (2026-07-22)
- https://obsproject.com/forum/threads/help-valorant-freezes-when-i-open-obs.182913/
- https://obsproject.com/forum/threads/capture-cs2-with-anti-cheat.182477/
- https://www.battleye.com/support/faq/
- https://www.ubisoft.com/de-de/game/rainbow-six/siege/news-updates/2999gdyIXLbeiZP78R7545/battleye-faq
- https://help.ea.com/en/articles/platforms/pc-ea-anticheat/
- https://www.ea.com/games/battlefield/redsec/news/battlefield-6-anticheat-update-season-4 (2026-08-28)
- https://ggrecon.com/articles/counter-strike-2-bans-for-amd-anti-lag-users-set-to-be-reversed
- https://www.pcgamesn.com/counter-strike-2/vac-bans-reverse
- https://www.guru3d.com/story/update-watch-dogs-2-anti-cheat-system-blocks-rtss-overlay-software/
- https://stevivor.com/news/call-duty-ricochet-anti-cheat-system-detailed-activision-blizzard

Voice:
- https://community.home-assistant.io/t/fyi-picovoice-confirmed-free-tier-accesskeys-will-stop-working-after-june-30-2026/1012744 (2026-06-04)
- https://github.com/Picovoice/porcupine
- https://picovoice.ai/blog/introducing-picovoices-free-tier (Jan 2025; superseded)
- https://github.com/dscripka/openWakeWord
- https://k2-fsa.github.io/sherpa/onnx/kws/index.html
- https://github.com/k2-fsa/sherpa-onnx
- https://alphacephei.com/vosk/models
- https://github.com/ggml-org/whisper.cpp

Game detection and metadata:
- https://discord.com/api/v9/applications/detectable (fetched 2026-10-21)
- https://discord.com/terms (effective 2025-09-29)
- https://api-docs.igdb.com/
- https://www.steamgriddb.com/api/v2 and https://www.steamgriddb.com/static/openapi.yml (v2.10.0)
- https://steamcommunity.com/dev/apiterms
