# 04 - Competitive landscape: game clipping on Windows (as of 2026-10-21)

Researcher brief for the Kairo planning team. Kairo is a Windows-only, local-first instant-replay and screenshot app. It must treat AMD AMF and Intel Quick Sync as first-class, and also support NVIDIA NVENC.

**Confidence tags:** [H] = primary source, current. [M] = secondary source, an older primary doc, or a vendor's claim about itself. [L] = a single weak source or an inference. "n/v" = not verified in this session.

**Method notes**
- Primary sources were preferred: vendor docs, release notes, GitHub, Steam's news API, the Discord Zendesk API, RDAP and WHOIS. Each source's date is given where known.
- The session's shared web-search budget ran out late in the research. Later checks used only direct fetches of known primary URLs.
- Reddit threads could not be retrieved. The complaint evidence comes from Trustpilot, vendor forums (AMD, Intel, Steam, Overwolf) and the vendors' own bug-fix lists.
- Some competitor-written comparison pages (Insights, Eklipse, Medal) are used only where nothing better exists, and they are flagged when used.

---

## 0. TL;DR - what matters for Kairo

1. **Intel GPU owners have no first-party instant replay.**
   - Intel retired the "Studio" page (recording and streaming) when Intel Graphics Software replaced Arc Control in late 2024. Its support article, last reviewed 2026-07-07, points users to OBS, Steam and Game Bar [H].
   - Discord Clips' GPU list leaves out Intel entirely [H].
   - Steam's recording FAQ (2024) mentions hardware encoding only for NVIDIA and AMD, with a CPU fallback otherwise [H, possibly stale].
   - Users kept asking Intel for instant replay in May 2025 and June 2026 [H].
2. **AMD's built-in Instant Replay is functional but dated.**
   - Limits: 30 or 60 FPS only, buffer up to 20 min, and the microphone can only be split out as a separate file [H; AMD doc last updated 2024-11-11 and based on Adrenalin 23.9.1].
   - Users have long reported stutter with Instant Replay on [M].
   - AMD driver notes still list driver timeouts involving Instant Replay in Sept 2026 [H].
3. **On NVIDIA hardware the built-in tool is strong.** The NVIDIA app is free, login is optional, it supports AV1 and up to 240 FPS capture on RTX 40/50 [H], and it now requires an RTX 20-series or newer GPU [H]. Kairo's wedge is therefore cross-vendor parity, especially on AMD and Intel, not beating ShadowPlay on NVIDIA.
4. **The market leader, Medal, is now tied to an AI lab.**
   - Scale: 17M+ MAU and about 3bn uploaded clips a year (Sept 2026) [M].
   - Its spin-out General Intuition trains agents on Medal clips: $133.7M seed in Oct 2025, then $220M at a $6.2bn valuation in Sept 2026 [H/M].
   - Medal offers an opt-out toggle [H].
   - Insights Capture now pays users for recordings that "may" train AI models [H].
   - A clear "local-first, never trained on, no ads" position is open and credible.
5. **Free-tier sharing is restricted everywhere.**
   - Steam share links expire after 2 days [H].
   - Medal's free links expire after 14 days unless re-copied or posted [H]. Free uploads are capped at 20 min and 1080p and carry a watermark [M; docs conflict].
   - Outplayed's free plan hosts clips for 3 months with a 400 MB cap [H].
   - Allstar's free plan allows 5 clips a month at 720p30 [M].
6. **Per-app audio tracks are a real gap and are feasible on all of Kairo's target OS builds.**
   - OBS registers its process-loopback "Application Audio Capture" source on Windows 10 build 19041 and later. Its code comment reads: "MS says 20348, but process filtering seems to work earlier" [H].
   - Steam users have asked for separate tracks since 2024 [H]. AMD only splits the mic into its own file [H]. No separate-track option was found for Game Bar.
7. **Overwolf apps (Outplayed) carry ads and an extra client.** The Outplayed developer says there is "no plan" for a standalone version [M]. Insights moved off Overwolf with V3 because of performance [M].
8. **A new entrant already targets the same positioning.** "Clippy" (May 2026) is a Rust + Tauri app with no account needed to record, but only NVENC is mentioned and its traction is tiny [L]. The lightweight, no-account niche is not yet owned by anyone.
9. **The name "Kairo" collides in adjacent spaces.**
   - A "KAIRO" sports highlight-reel SaaS launched at kairoclips.com (domain registered 2026-05-24).
   - Kairosoft uses the "Kairo" brand across its games (Kairo Land, Kairo Club).
   - A 2013 Steam game is called *Kairo*.
   - kairo.gg and kairo.app are taken. A trademark search is needed before launch.

---

## 1. Market map

| Category | Products | How they win | Weakness Kairo can exploit |
|---|---|---|---|
| GPU vendor tools | NVIDIA app (ShadowPlay), AMD Adrenalin (Record & Stream), Intel (none) | Free, preinstalled, low overhead | Locked to one vendor. AMD is capped at 60 FPS. Intel has nothing. |
| Platform built-ins | Xbox Game Bar, Steam Game Recording, Discord Clips | Zero install, "good enough" | Game Bar: 60 FPS, one window, no editing. Steam: only games launched through Steam, 2-day links. Discord: only while streaming, Nitro early access, no Intel. |
| Third-party clipping apps | Medal, Outplayed (Overwolf), Insights Capture, SteelSeries Moments, DOR, Clippy | Auto-clips, social and cloud features | Ads, Electron or Overwolf weight, accounts, AI-training data programs, reliability complaints |
| Cloud / server-side highlights | Allstar (demo-based), Eklipse (stream VODs), Killcam.gg (CS2), GeForce NOW Highlights | No FPS cost, polished edits | Few games, subscription-heavy, not real-time capture |
| Power-user and open source | OBS Studio (replay buffer), ShareX (screenshots, recording) | Free, OSS, maximum control | OBS is complex, with a RAM buffer and no library or editor. ShareX has no replay buffer. |

---

## 2. Feature matrix

### 2a. Capture capabilities

| Tool | Rolling replay (length) | Auto-highlights | HW encoders / GPU support | Max capture FPS | Audio separation | Capture scope |
|---|---|---|---|---|---|---|
| **Medal** | Yes, hotkey clip from buffer, plus full-session recording. Max length n/v. | Yes, auto-clip in supported games via game events and plug-ins [H] | Claims NVIDIA, AMD and Intel [M] | n/v for capture. Free uploads ≤60 fps. Premium keeps original quality since 2026-06 [H]. | Game, mic and Discord separate, plus per-app tracks (Medal's claim, 2026-10-01) [M] | Games and screen recording. Imports Xbox console clips [H]. |
| **Outplayed (Overwolf)** | Yes (manual and auto modes). Buffer length n/v. | Yes, Overwolf game events, "5,000+ games" [H] | n/v | n/v | n/v | Games, through the Overwolf client |
| **Insights Capture** | Auto-records whole matches, then you clip. Buffer n/v. | Event timeline. Valorant and LoL at V3 launch [M]. | "GPU-agnostic" (vendor) [L] | n/v | n/v | Supported games: "500+" vs "10,000+" (the vendor contradicts itself) |
| **SteelSeries Moments (GG)** | Hotkey clip. Length n/v. | Yes, "Moments AI" in about 27 titles [M] | System requirements list NVIDIA, AMD and Intel GPUs [M] | "High frame rate" (unspecified) | Multi-track audio editing, per-track boost to 150% (GG 110, 2026-05-05) [H] | Auto-detected games, including WeGame (2026-06) [H] |
| **NVIDIA app** | Yes. Default "last 30 seconds". GFE-era max was 20 min; the app's max is n/v. [H/M] | NVIDIA Highlights in supported games [H] | NVENC only, RTX 20+ per requirements. H.264, HEVC, and AV1 on RTX 40/50 [H]. | **240 fps** on RTX 40/50 (11.0.8). 4K120 HDR and 8K30 HDR [H]. | Game and mic as separate layers (user reports, GFE era) [L]. Per-app n/v. | Games, desktop n/v |
| **AMD Adrenalin (Record & Stream)** | Yes, **up to 20 min** in 15 s steps [H] | None found | AMD only. AVC, HEVC, AV1 (GPU-dependent) [H]. | **30 or 60 fps only** [H, doc dated 2024-11] | "Separate Microphone Track" saves the mic **to a separate file** [H]. No per-app audio. | Games, desktop, borderless region. Instant GIF of 1-30 s [H]. |
| **Intel Graphics Software** | **None.** Studio retired [H]. | None | n/a | n/a | n/a | n/a |
| **Xbox Game Bar** | "Record that": 30 s default, options up to 10 min (vary by build) [M] | None | Vendor-neutral (Windows' own encoder) | **30/60 fps** [M] | None found | One app/game window. No desktop or File Explorer [M]. |
| **Steam Game Recording** | Background recording to disk with user-set duration and storage, per game, including "infinite". Instant Clip defaults to 30 s (since 2024-11) [H]. | Steam Timeline markers if the game integrates `ISteamTimeline`. Achievements are marked. [H] | NVIDIA and AMD hardware encode, CPU otherwise (FAQ 2024) [H]. H.264 and HEVC, up to 4K with HEVC [H]. AV1 not found. | n/v | One mixed track (game plus selected programs plus optional mic). Users have asked for separate tracks since 2024 [H]. | Only games launched through Steam, including non-Steam shortcuts. No desktop [H]. |
| **Discord Clips** | 30 s, 60 s or 2 min, **only while streaming an app** [H] | None | NVIDIA GTX 980/16xx/RTX and AMD RX 500+. **Intel not listed** [H]. | n/v | Toggle game audio and voice-channel audio when editing [H] | The streamed app only |
| **OBS Studio** | RAM replay buffer. Defaults 20 s / 512 MB, configurable [H]. | None built in | NVENC, AMF, QSV. H.264, HEVC, AV1. | Configurable | Up to 6 tracks. Application Audio Capture on Win10 build 19041+ [H]. | Anything: game, window, display, capture card |
| **ShareX** | **No replay buffer** (none in the changelog) [H] | None | Native recorder since 22.0. The FFmpeg path supports QSV (changelog) [H]. | 120 fps limit (22.0) [H] | System audio and mic (22.0) [H] | Screen, region, window. Screenshots: scrolling, OCR, HDR corrector [H]. |
| **Allstar.gg** | n/a. Clips are rendered server-side from match data and demos. CS2 needs no capture app [M]. | Yes, server-side | n/a (cloud render) | 30/60/120 fps by tier [M] | n/a | CS2, Dota 2, LoL, Fortnite. Web code also references Valorant and OW2 (n/v) [M]. |
| **Eklipse** | n/a. AI highlights from stream VODs (Twitch, YouTube, Kick, Facebook) [M]. | Yes. "Gameplay Intelligence" engine since 2026-06-01 [M]. | n/a (cloud) | Export up to 1080p60 (Premium) [M] | n/a | Streams only. No local capture. |
| **Clippy (new, May 2026)** | Yes, last 30 s, rolling RAM buffer [L] | n/v | NVENC mentioned only [L] | n/v | Game, desktop and mic on separate tracks [L] | Active game window [L] |

### 2b. Business model, sharing and footprint

| Tool | Paid tier price | Account needed? | Ads / watermark (free) | Hosted sharing (free limits) | Editor / vertical | Footprint | Notable 2024-2026 change |
|---|---|---|---|---|---|---|---|
| **Medal** | Premium **$9.99/mo or $7.99/mo billed yearly**, 14-day trial [H] | Yes to upload, share or post [H]. Local-only use n/v; setup says "create an account". | Ads [H]. Watermark/outro on free downloads (Premium FAQ) [H]. The landing page says "no watermarks", which conflicts. | Free: links expire 14 days after creation, timer resets on Copy Link, posting to profile keeps them [H]. Uploads ≤20 min and 1080p (Premium page and FAQ; a Jan 2025 blog said 10 min) [M]. Premium: unlimited length, original quality (2026-06-17), never-expiring links, cloud backup [H]. | Multi-track editor (rolled out 2025-11-20), subtitles (Premium), strong mobile and TikTok pipeline | Electron/Chromium app (Chromium 136 update, May 2025) [H]. "100 to 400MB of RAM" for the buffer (vendor claim) [M]. | General Intuition spin-out trains AI on Medal clips. Opt-out toggle exists [H]. |
| **Outplayed** | Premium via Tebex, monthly. **Price n/v.** "Outplayed50" code for 50% off month one. Yearly plan (-16%) "coming" [M]. | Not required to use Overwolf apps [H]. Login required to share [H]. | Contains ads (app creators get 70% of ad revenue) [H]. Premium removes ads and the editor watermark [M]. | Outplayed.tv link/QR. **400 MB cap.** **Core hosting 3 months**. Premium unlimited for uploads after 2025-03-01. X/Twitter sharing disabled [H]. | Trim, collage editor. Shares to TikTok, YouTube, Reddit, Discord [H]. | Overwolf client required (10.3 MB stub installer) [H] | Hosting limits introduced (Mar 2025). No standalone version planned [M]. |
| **Insights Capture** | Free. Team plans **$39-$229/mo** ($27-$160/mo billed yearly) [H] | Sign-in ("Download Insights Capture and sign in") [H] | Vendor says no ads and no watermark on free [M]. The pricing page says ad-free is an "Elite" perk, which conflicts. | Basic plan 5 GB active storage [H]. "Links don't expire" (vendor) [M]. | Clipping and VOD review (comments, drawing) [M] | n/v. V3 is standalone, with no Overwolf [H]. | **Shard Program** pays $1 per 2,000 Shards (min $5). Recordings "may" train AI models [H]. |
| **SteelSeries Moments** | Free [M] | n/v | None found | Direct upload to YouTube, Discord, Reddit, X. No hosted links found [M]. | Trim, montage [M] | GG requires 500 MB disk and 8 GB system RAM [M] | Active 2026 updates. Memory-use bug fixed 2026-10-20 (GG 122) [H]. |
| **NVIDIA app** | Free | **Optional** [H] | None | None (local) | Minimal | 600 MB disk, 2 GB RAM, driver 551.52+, **RTX 20/30/40/50** [H] | Replaced GFE 2024-11-12 [M]. 240 fps capture (11.0.8) [H]. **Local MCP server for AI agents** (11.0.9.565 beta, 2026-10-13), reported on by default [M]. |
| **AMD Adrenalin** | Free | No | None | None | None (Instant GIF) | Part of the driver package (size n/v) | 26.10.1 (2026-10-20) has no capture changes. Instant Replay timeouts listed in 26.9.2 [H]. |
| **Intel Graphics Software** | n/a | n/a | n/a | n/a | n/a | n/a | Studio (recording, streaming) retired (Dec 2024). Reaffirmed 2026-07-07 [H]. |
| **Xbox Game Bar** | Free (built in) | Not for local capture (n/v) | None | None hosted | None. Trim in Photos [M]. | Built in | UI revamp, Mar 2025 [M] |
| **Steam** | Free | Steam account | None | Steam-hosted **share link expires after 2 days**. "Send to other device" within 2 days [H]. | Timeline trim, MP4 export [H] | Built in. About 10 GB per 2 h at the 12 Mbps default (press, citing Valve) [M]. | Beta 2024-06-26, GA 2024-11-05 [H]. 2026-10-13: lower D3D12 overhead, H.264 fallback [H]. |
| **Discord Clips** | Nitro ("Early Access for Nitro members") [H] | Discord account and Nitro | None | Share inside Discord | Trim | Part of the Discord client | Overlay rebuilt (2025-03-25) [M] |
| **OBS Studio** | Free, **GPL-2.0** [H] | No | None | None | None | 159.8 MB installer (32.2.2 x64) [H] | 32.0 (2025-09-22) made Hybrid MP4 the default. 32.1 (2026-03-11) overhauled the mixer. **33.0 RC3** (2026-10-20) [H]. |
| **ShareX** | Free, **GPL-3.0** [H] | No | None | Many upload targets: custom uploaders, S3, R2, GCS guides [H] | Image editor, video trimmer (22.0) [H] | 131.7 MB setup (22.0.0) [H] | **22.0.0 (2026-10-20)**: full UI rewrite, native recorder, .NET 10 [H] |
| **Allstar.gg** | Essentials **$0.99**, Pro **$4.99** ($3.99 yearly), Unlimited **$11.99** ($7.99 yearly). An A/B variant shows $14.99. Observed in the web-app code on 2026-10-21 [M]. | Required | Watermark on free (removal is a paid feature) [M] | Free: **5 clips/mo, 10 stored, 720p30** [M] | Effects, cameras, montages, "mobile clips" [M] | Cloud | $12M Series A (Dec 2023) [M] |
| **Eklipse** | Premium **$19.99/mo**. Yearly is **$119.99** (inferred from Eklipse's stated "$119.89/yr" saving, 2026-08-13) vs $149.99 on Capterra. The sources conflict [M]. | Required (connect channel) | Free is a preview only. Premium has no watermark [M]. | n/a | AI edits, captions, templates [M] | Cloud | Gameplay Intelligence engine (2026-06-01) [M] |
| **Clippy** | Free [L] | Not needed to record. Needed to post to clippy.rip [L]. | n/v | clippy.rip feed and links [L] | Built-in editor [L] | Native Rust + Tauri (not Electron) [L] | Product Hunt, May 2026, 3 upvotes [L] |

---

## 3. Competitor profiles (details, limits, 2024-2026 changes)

### 3.1 Medal.tv (market leader, third-party)
- **Scale.**
  - "20M+" users and "3B+" clips per Medal's own landing page [M].
  - 17M+ MAU and about 3bn uploaded clips a year per MobileGamer.biz (2026-09-29) [M].
  - 10M MAU and about 2bn videos a year per TechCrunch (2025-10-16) [H].
- **Premium (2026-06-17 blog).**
  - "Same Price, More Stuff": $9.99/mo or $7.99/mo billed yearly.
  - Premium uploads went from a 30-min limit to unlimited and are no longer downscaled to 1440p/120. 4K stays 4K.
  - Premium also adds unlimited auto subtitles, full library cloud backup, never-expiring links, removable watermark, and ad-free use with exceptions: "programmatic IAB ads and platform featuring" are excluded [H].
- **Free tier.**
  - Uploads up to 20 min, 1080p and 60 fps [H: Premium FAQ, modified "Sat, 5 Sep", year not shown; the Premium page also says "Ditch the 1080p limit" and gives free a 20-min cap].
  - Older posts conflict: 2024-11-18 said free was 720p and 10 min; 2025-01-14 said free was 10 min [H, superseded].
  - Watermark/outro on downloads is not removable on free (FAQ) [H]. Ads [H].
- **Links.** "Expiring links" last 14 days. Clicking Copy Link resets the timer, and posting to your profile keeps the clip. Cloud sync is Premium-only [H].
- **Features.** Changelogs and pages show:
  - Auto-clipping with game-event "plug-ins".
  - Full-session recording (MP4 default since 2025-06-26).
  - Screen recording.
  - Optional keyboard/mouse input overlay (2025-08-28).
  - Multi-track editor (2025-11-20).
  - Medalbot for Discord.
  - Xbox clip import through a linked Xbox account [H].
  - Per-app separate audio tracks, from Medal's own marketing page (2026-10-01) [M]. That page's comparison table contradicts its own FAQ on competitors, so its competitor claims are unreliable.
- **Performance.** Medal claims "little to no FPS difference" and "100 to 400MB of RAM" [M]. It is an Electron/Chromium app (the changelog mentions a Chromium v136 update) [H].
- **AI / privacy.**
  - General Intuition was spun out of Medal B.V. in Oct 2025. Seed of $133.7M, led by Khosla and General Catalyst. Pim de Witte is CEO of both companies [H].
  - OpenAI reportedly tried to buy Medal for about $500M in late 2024 (The Information via TechCrunch; unconfirmed) [M].
  - $320M at $2.3bn in Jan 2026, then $220M at $6.2bn led by Valor (MobileGamer.biz, 2026-09-29) [M].
  - Medal support has an "Opt-Out of AI Training" toggle (Settings > Privacy & Security > Privacy & Visibility > AI Research and Development). The default state is not stated [H].

### 3.2 Outplayed (Overwolf)
- Free with ads. "Subscription available." 16M downloads, 4.6/5 from 2,496 ratings. Overwolf client required, 10.3 MB stub. "You're not required to sign in to use apps" [H].
- **Sharing (support article modified 2026-09-16).**
  - Outplayed.tv link and QR code. 400 MB limit for Outplayed.tv, Discord, YouTube and Reddit.
  - Core (free) hosts clips for 3 months, then deletes them permanently. Premium hosts for unlimited time for content uploaded after 2025-03-01.
  - Auto-share length defaults to 30 s. X/Twitter sharing is disabled because of API changes.
  - Discord embeds don't play AV1. Login is required to share [H].
- **Premium.** Tebex billing, monthly. Removes ads and adds layout options, an enhanced full-screen mode and editor watermark removal [M, from a search snippet of an Overwolf FAQ]. **Price n/v.**
- **Platform risk.** The developer says there are no plans for a standalone version because of its Overwolf game-events dependency [M, Trustpilot reply].
- **Performance.** Overwolf's support says capture apps cost some FPS, "usually very minor", and warns of RivaTuner (RTSS) conflicts [M]. A user feature request reports overlay-injection FPS drops [L].
- **Corporate context.** Overwolf acquired Nitro (ad-tech) in May 2024. Overwolf in-game ad sales reportedly doubled to $100M in 2024 [M/L].

### 3.3 NVIDIA app (ShadowPlay / Instant Replay / Highlights)
- **Replaced GeForce Experience.** Left beta on 2024-11-12. GFE gets no new features after driver 566.36 [M].
- **Requirements (accessed 2026-10-21).** Windows 10/11, 2 GB RAM, 600 MB disk, GeForce driver 551.52+, **GeForce RTX 20/30/40/50 GPUs** (GTX is not listed) [H].
- **Capture** [H]:
  - Instant Replay: "save the last 30 seconds" is the stated default.
  - Record: up to 8K30 HDR or 4K120 HDR.
  - Highlights (auto).
  - AV1 for Record, Instant Replay and Highlights (10.0.1). Bitrates default by resolution, fps and codec (11.0.2).
  - **240 fps ShadowPlay on RTX 40/50 (11.0.8)**.
  - Fixes for real user bugs: Instant Replay temp files ignoring the chosen folder (11.0.2); HDR colors wrong in HEVC/AV1 (11.0.4.159); bitrate resetting to 150 Mbps (11.0.6); recordings failing on special characters in the game title (11.0.8).
- **Login.** Optional, used only to redeem bundles and rewards [H].
- **Game Filters.** Default off since 11.0.1. This followed performance-overhead reports [H for the change; the cause was not verified in this session].
- **2026 controversy.** 11.0.9.565 (beta, 2026-10-13) adds a local MCP server so AI agents can "enable ShadowPlay to record gameplay" and change settings. Outlets report it was **on by default**, and users raised privacy and bloat complaints [M; WindowsMode 2026-10-17].
- **Instant Replay max length in the NVIDIA app is n/v.** Many older guides say GFE allowed up to 20 min [M].

### 3.4 AMD Software: Adrenalin Edition (Record & Stream)
- **AMD support article DH3-023** (last updated 2024-11-11, based on Adrenalin 23.9.1 on an RX 7900 XTX) [H]:
  - **Instant Replay**: "last 20 minutes", adjustable in 15-second increments.
  - **Instant GIF**: 1-30 s at up to 1280x720.
  - **In-Game Replay**.
  - **Record Desktop**: OpenGL only in windowed mode.
  - **Borderless Region Capture**.
  - **Resolution**: 360p to 2160p, or In-Game.
  - **FPS: 30 or 60 only**.
  - **Codecs: AV1, HEVC or AVC**.
  - **"Separate Microphone Track"**: the mic is "saved to a separate file".
  - Push-to-talk, audio boost. Default folder `Videos\Radeon ReLive`.
- **Current drivers.** Adrenalin **26.10.1** (2026-10-20) has no Record changes. 26.9.2 lists a fix for "Intermittent driver timeout ... War Thunder in DirectX 12 mode with Instant Replay and Borderless Region Capture enabled" on RX 6000. 26.10.1 also mentions installing "AI Bundle components" [H].
- **No 120 fps capture option found** in any AMD source [M; absence of evidence].

### 3.5 Intel graphics software
- **Intel support article 000100252** (last reviewed 2026-07-07): Intel Graphics Software "does not include a Studio page or features like Intel Arc Control does". Intel "made the tough decision to retire the Studio page" because of "very low user usage". It recommends OBS, Steam and Windows Game Bar and "may consider bringing back these features in the future if demand is high" [H].
- **Demand persists.** A community feature request (2026-06-05) asks for "Built-In Recording and Instant Replay" to return. The staff reply is generic [H]. An Arc B580 streamer asked for native instant replay (2025-05-28); staff asked which tool they use and closed the thread [H].
- **Prior quality.** One user described the old Arc Control capture as having audio drift, 30 fps recording and no hotkey mapping [L].

### 3.6 Xbox Game Bar ("Record that")
- Win+Alt+G saves the last N seconds when background recording is on. GeekRewind (2026-10-07) lists lengths "such as 30 seconds, 1 minute, 3 minutes, 5 minutes or 10 minutes" and says they vary by build.
- Manual recording maximum: 30 min to 4 h.
- Frame rate 30 or 60 fps. Quality "Standard" (≤1080p) or "High" [M].
- **Limits.** "Apps such as File Explorer and Notepad are not supported", the Game Bar "does not open on the desktop", and saves go to `Videos\Captures` as MP4 (Dell KB, 2026-04-18) [M]. Game Bar can't trim; you use Photos [M].
- **User pain.** Intel forum users cite Game Bar's "60 FPS cap and no bitrate control" [M].
- **Changes.** The official Xbox support article (support.xbox.com) is a JavaScript-only page and could not be read. The UI was revamped in March 2025 [M].

### 3.7 Steam Game Recording
- **Dates.** Beta 2024-06-26, available to all 2024-11-05. GA added game-specific settings, quick-capture hotkeys, advanced export and Session View [H].
- **FAQ (timestamp 2024-07-10)** [H]:
  - Background or on-demand recording to disk, with user-set disk space. Recordings are stored in a raw format and must be exported to MP4.
  - "It will not capture video of your desktop or other programs", but it can include audio from voice chat.
  - Share link "expires after 2 days". Send to Other Device works within 2 days.
  - Uses "NVIDIA and AMD graphics cards ... systems without those graphics cards, the system's CPU is used".
- **Changes since** [H]:
  - Beta 2024-10-03: HEVC on Windows, 4K with HEVC, 32:9 ultrawide, auto bitrate, mic recording, unlimited per-game recording.
  - 2024-11-12: Instant Clip default raised from 10 s to 30 s.
  - 2025-05-20: fixed green frames capturing Vulkan games on Intel GPUs.
  - 2026-04-28: instant clip fix for non-Steam games.
  - **2026-10-13**: "Decreased performance impact of game recording for some D3D12 games on Windows" and "Automatically fall back to H.264 if HEVC isn't available".
- **Developer API.** `ISteamTimeline` (`AddTimelineEvent`, `SetTimelineGameMode`) lets games add markers. Only the game's own developer can integrate it [H].
- **Gaps.** No separate audio tracks: users asked in July-Sept 2024, with no Valve reply [H]. No AV1 recording found [M].

### 3.8 OBS Studio (replay buffer)
- **Replay buffer.** Defaults `RecRBTime=20` seconds and `RecRBSize=512` MB (source, 32.2.2). The buffer lives in RAM [H/M].
- **Application Audio Capture** (WASAPI process loopback) is registered on **Windows 10 build 19041+**. Code comment: "MS says 20348, but process filtering seems to work earlier" [H].
- **Releases** [H]:
  - 32.0 (2025-09-22): plugin manager, Hybrid MP4 default, updated AMD encoder defaults.
  - 32.1 (2026-03-11): audio mixer overhaul.
  - 32.2 (2026-07-21): NVIDIA SDK 13, minimum driver 570.
  - 32.2.2 stable (2026-08-14).
  - **33.0 RC3 (2026-10-20)**: CEF 150, plugin loading overhaul, NVENC on Arm64.
- License **GPL-2.0**. 77.9k GitHub stars. If Kairo links libobs, Kairo must be GPL [H].

### 3.9 Insights Capture (Insights.gg)
- **V3** is "a complete rebuild ... A standalone app (no more Overwolf)" with "roughly 40 percent less FPS drop compared to previous versions" (vendor). Timeline at launch covers Valorant and LoL. The page still talks about game support rolling out "throughout 2025" and shows a waitlist [M].
- **Inconsistency.** Insights' own comparison blog (2026-09-02) links the download button to an **Overwolf** installer URL. Distribution status is unclear [M].
- **Pricing page.** Basic free with 5 GB active storage. Silver, Gold, Platinum and Diamond at $39, $79, $109 and $229/mo. "Elite" seats get "no ads, permanent upload" [H]. The company is positioned toward team review and coaching.
- **Shard Program.** Pays gift cards (2,000 Shards = $1, $5 minimum, 18+, "$100,000+ paid"). "We may use your recordings to help train AI models." "Every video is anonymized before it leaves your PC" [H].

### 3.10 SteelSeries Moments (inside SteelSeries GG)
- **Features page** [M]:
  - Free.
  - "Moments AI" auto-clips about 27 titles, including Fortnite, Valorant, CS2, Apex, SF6 and BO7.
  - Hotkey clips, event markers, non-destructive trim, multi-track audio editing, montage.
  - Uploads to YouTube, Discord, Reddit, X.
  - Requirements: Win10+, 8 GB RAM, 500 MB disk, GTX 960+ / Radeon HD 7870+ / Intel HD 4000+.
- **2026 GG release notes** [H]:
  - Delta Force auto-clip (GG 111, 2026-05-20).
  - Per-track volume boost to 150% (GG 110, 2026-05-05).
  - WeGame detection (GG 113, 2026-06-16).
  - "Moments capture could consume an abnormal amount of memory" fixed (GG 122, 2026-10-20).
  - macOS GG gets no new features (GG 109, 2026-04-21).

### 3.11 Allstar.gg (server-side clips)
- Renders clips in the cloud from match data and demo files. "Zero FPS drop". Officially lists CS2, Dota 2, LoL and Fortnite (Eklipse comparison, checked 2026-07-15) [M]. Allstar's web code maps VALORANT, DOTA2, LOL, CS2, CS_GO, FORTNITE and OW2, and includes a "Valorant Notification" prompt. Valorant and OW2 may be coming soon; n/v [M].
- **Tiers** (strings in the allstar.gg web-app bundle, 2026-10-21; subject to A/B tests) [M]:
  - FREE: 5 clips/mo, 10 clips of cloud storage, 720p30.
  - ESSENTIALS: $0.99/mo, 10 clips/mo, 50 stored, 1080p60, downloads, montages, watermark removal.
  - PRO: $4.99/mo or $3.99/mo yearly, 50 clips/mo, 600 stored, 1080p120 (max 4K), bulk download, timeline editor.
  - Unlimited "Creator Platinum": $11.99/mo or $7.99/mo yearly, with a $14.99 test variant. Clip-storage add-ons from $0.49/mo.
- **Funding.** $12M Series A (Dec 2023, Drive Capital) [M].

### 3.12 Discord Clips
- **Support article 16861982215703** (created 2023-08-18, **updated 2026-10-16**) [H]:
  - "Clips is currently available in Early Access for Nitro members."
  - Windows only.
  - Minimum GPU: "Nvidia GTX 980s/1600s, all Nvidia RTX models, or AMD Radeon RX 500". Intel isn't mentioned.
  - Works only while streaming an application. Alt+C saves 30 s, 60 s or 2 min.
  - Clips are local. You can toggle game and voice-channel audio when editing.
  - "may impact your game performance if you have lower end hardware".
- **Overlay.** Rebuilt from the ground up (2025-03-25) [M].

### 3.13 AI highlight tools
- **Eklipse.**
  - Cloud highlights from Twitch, YouTube, Kick and Facebook streams. A free preview of a recent stream. Premium is "Up to 1080p 60fps, no watermark" and "Unlimited AI clipping" (2026-08-13) [M].
  - New "Gameplay Intelligence" engine replaced the previous AI layer on 2026-06-01 [M].
  - Price $19.99/mo. The yearly price conflicts between sources ($119.99 inferred vs $149.99) [M].
- **Built-in auto-clippers.** Medal auto-clip, Outplayed (Overwolf events), NVIDIA Highlights (games integrate the SDK; also on GeForce NOW), SteelSeries Moments AI, Insights timeline, Steam Timeline (game-integrated markers) [H/M].
- **Powder.gg** (AI clips, with Xbox console support) shut down in late 2024. Sources are Eklipse blog posts, a competitor [L/M].
- **DOR (clip.dor.gg).** Free; auto-saves highlights in Roblox, Valorant, CS, FC Online and Fortnite; NVENC; browser editor. Vendor claims [L].
- **Killcam.gg (CS2).** Free tier 1080p30 or 720p60 with watermark. $4.99/mo or $39.99/yr for 4K60 without watermark. Secondary source [L].
- **StreamYard AI Clips** (2026-01-15) and Spikes Studio: stream-oriented [L].
- **NVIDIA's MCP server** (Oct 2026) shows "AI agent" control of capture arriving in vendor tools [M].

### 3.14 ShareX
- **22.0.0 (2026-10-20).** "the biggest update we've ever released". Native screen recording "without relying on FFmpeg", system audio and mic, camera overlay, FPS limit raised to 120, HDR screenshot color corrector, code-signed installer, .NET 10, FFmpeg 9.0. Marked as a GitHub pre-release [H].
- **21.0.0 (2026-07-03).** Stable [H].
- The .NET 9 move dropped Windows 7, so Windows 10 64-bit+ is required. Setup is now about 100+ MB [H].
- **GPL-3.0**, 40.3k stars [H].
- **No instant-replay buffer.** It is a screenshot and recording utility. It is the de facto standard for screenshots plus uploads (custom uploaders, S3, R2, GCS) [H].

### 3.15 Console and cloud coverage among competitors
- **Medal** imports **Xbox** captures after you link an Xbox account (desktop: one clip at a time) [H]. No PlayStation import doc was found [M].
- **Powder** (Xbox support) is gone [L/M]. **Eklipse** works from console streams on Twitch and similar [M].
- **GeForce NOW** has its own overlay with Highlights, manual recording and screenshots (2021 post) [M]. Instant Replay on GFN was mentioned in older sources, n/v.
- **Nintendo Switch 2** keeps a 30-second capture limit (Nintendo support, via search) [M].
- **Capture cards.** No consumer clipping app reviewed offers a replay buffer on a capture-card feed except OBS, which can do it through a video capture device source [M]. This is an opening for Kairo if the user's console setup includes a capture card.

---

## 4. Real user complaints (evidence-backed)

| Product | Complaint | Evidence (date) |
|---|---|---|
| Medal | Unreliable capture: "basically never was active when I wanted to clip something"; "my r6 clips were just a black screen" | Trustpilot (2026-10-18, 2026-10-13). TrustScore 3.6/5 from 301 reviews; 36% one-star [M]. |
| Medal | Uninstall problems: "I uninstall it and it just comes back"; "removed it's uninstaller, then continued opening itself on reboot" | Trustpilot (2026-09-07, 2026-04-25) [M] |
| Medal | Fills storage: "It just fills up my storage" | Trustpilot (2026-09-06) [M] |
| Medal | Free-tier friction: watermark, expiring links (14 days), upload caps, ads | Medal support and Premium FAQ [H]. Competitors market against it (Insights, 2026-09-02) [M]. |
| Medal | AI training of user clips (General Intuition) | TechCrunch 2025-10-16, MobileGamer 2026-09-29, Medal opt-out doc [H/M]. No organized Reddit backlash was found (Reddit not retrievable this session). |
| Outplayed | "records EVERYTHING by default", fills disk; "will not record games back to back"; window pops up after every game | Trustpilot (2026-09-29, 2026-08-29, 2026-09-20). Score 4.7 from 328 reviews [M]. |
| Outplayed / Overwolf | Ads, extra client, overlay FPS drops, RTSS conflicts | Overwolf app page (contains ads) [H]. Overwolf support on FPS/RTSS [M]. Overwolf forum feature request [L]. |
| NVIDIA app | AI-agent MCP server enabled without a prompt; privacy and bloat concerns | WindowsMode 2026-10-17 [M] |
| NVIDIA app | Recording bugs (temp replay files ignore folder; HDR colors wrong in HEVC/AV1; bitrate resets; failures on special characters) | NVIDIA release highlights 11.0.2-11.0.8 [H] |
| AMD Adrenalin | Stutter every few seconds with Instant Replay on (RX 7900 XTX owner tried many settings); driver timeouts | AMD community threads (older; the pages now redirect) [M]. AMD release notes 20.7.1, 23.7.2, 26.9.2 [H]. |
| AMD Adrenalin | Mic saved as a separate file, not as a track in the MP4. Users asked for one file with separate tracks. | AMD doc DH3-023 [H]. AMD community post (2020-era) [M]. |
| AMD Adrenalin | 60 fps ceiling | AMD doc DH3-023 [H] |
| Intel | No instant replay at all since Arc Control's Studio was removed | Intel support 000100252 (2026-07-07). Intel community (2025-05-28, 2026-06-05) [H]. |
| Xbox Game Bar | 60 FPS cap, no bitrate control; no desktop or File Explorer; no trimming | Intel community users [M]. Dell KB 2026-04-18 [M]. GeekRewind 2026-10-07 [M]. |
| Steam | No separate audio tracks (mic or Discord can't be muted per clip) | Steam Client Beta forum thread (2024-07-21 to 2024-09-10), no Valve reply [H] |
| Steam | Share links expire in 2 days; only Steam-launched games | Steam FAQ [H] |
| Discord Clips | Only while streaming; Nitro early access; Intel GPUs excluded | Discord support (2026-10-16) [H] |
| OBS | RAM-hungry buffer; complex setup; no library or editor | OBS forum (RAM / page file) [M]. Competitor comparison (Insights) [M]. |

**Commonly reported but not re-verified this session** (from prior knowledge; verify before quoting):
- Medal requiring an account at first launch.
- Overwolf-based apps tripping anti-cheat.
- ShadowPlay Instant Replay silently turning itself off.
- Game Bar background recording costing FPS.
- HDR clips looking washed out on many tools.

---

## 5. What AMD and Intel GPU owners use today, and what frustrates them

**AMD (Radeon) owners**
- **Adrenalin Instant Replay** is the default. It is free, has AV1/HEVC/AVC and a 20-min buffer. Frustrations:
  - 30/60 fps only [H].
  - Mic only as a separate file [H].
  - No per-app audio [H].
  - Long-running stutter and timeout reports tied to Instant Replay [M/H].
  - The features are tied to the driver, so a driver bug breaks capture.
- **OBS replay buffer** (AMF H.264/HEVC/AV1). Powerful and free, but complex, with RAM sizing and no library [M].
- **Medal, Outplayed, Insights, SteelSeries Moments** are vendor-neutral, but bring ads, Overwolf or Electron weight, and account or AI-data trade-offs [H/M].
- **Steam Game Recording** uses AMD hardware encode, but is Steam-only, has 2-day links and one mixed audio track [H].
- **Game Bar**: 60 fps, one window [M].
- **Discord Clips** (RX 500+) needs Nitro and an active stream [H].

**Intel (Arc dGPU or iGPU) owners**
- **No vendor tool.** Intel tells them to use OBS, Steam or Game Bar [H].
- Discord Clips doesn't list Intel [H].
- Steam's FAQ only names NVIDIA/AMD hardware encode (2024 text). A 2025 fix mentions capturing on Intel GPUs, so Quick Sync encode in Steam is unconfirmed [M/L].
- Medal and SteelSeries list Intel as supported [M].
- The forum pattern is repeated requests for a native, low-overhead instant replay, plus complaints about third-party resource use [H].
- **Implication for Kairo:** Intel Quick Sync (H.264/HEVC/AV1 on Arc) plus a polished, light replay is the least-contested segment.

---

## 6. Market dynamics 2024-2026 (timeline)

- 2024-05: Overwolf buys Nitro (ad-tech) [M].
- 2024-06-26 / 2024-11-05: Steam Game Recording beta, then GA [H].
- 2024-11-12: NVIDIA app replaces GeForce Experience [M].
- Late 2024: Powder.gg shuts down [L/M]. Intel Graphics Software ships without Studio [H].
- 2025-01-14: Medal raises upload limits [H].
- 2025-03: Outplayed hosting limits for content after 2025-03-01 [H]. Discord overlay rebuild [M]. Game Bar UI revamp [M].
- 2025-09-22: OBS 32.0 [H].
- 2025-10-16: General Intuition spins out of Medal with a $133.7M seed [H].
- 2025-11-20: Medal's new editor [H].
- 2026-01: General Intuition raises $320M at $2.3bn [M].
- 2026-05: Clippy appears on Product Hunt [L]. kairoclips.com registered (2026-05-24) [H].
- 2026-06-01: Eklipse Gameplay Intelligence [M].
- 2026-06-17: Medal Premium update [H].
- 2026-07-07: Intel reaffirms Studio retirement [H].
- 2026-09-29: General Intuition raises $220M at $6.2bn [M].
- 2026-10-13: NVIDIA app MCP server (beta) [M/H]. Steam lowers D3D12 capture overhead [H].
- 2026-10-20: AMD 26.10.1, OBS 33.0 RC3, ShareX 22.0.0, SteelSeries GG 122 [H].
- **Trend:** gameplay footage has become **AI training data**: Medal through General Intuition, Insights through its Shard Program. Vendors are adding **AI agents** (NVIDIA MCP). Users who care about privacy have fewer clean options.
- **Note (not re-verified this session):** Windows 10 mainstream support ended 2025-10-14 and consumer ESU ran to 2026-10-13. Win10 22H2 users are now unsupported by Microsoft, which may push vendors to drop Win10. Kairo's Win10 22H2 target is still reasonable for a beta.

---

## 7. Gaps where an indie app can win (ranked)

1. **A vendor-neutral replay that is equally good on AMD AMF and Intel Quick Sync (plus NVENC).**
   - **Evidence:** Intel has nothing first-party. AMD is capped at 60 fps with no per-app audio. Discord excludes Intel. NVIDIA's tool only covers NVIDIA, now RTX 20+.
   - **Bar to clear:** reliability ("never miss the clip" is Medal's #1 complaint), saves in about 1-2 s, 60-144 fps on AMD/Intel, HEVC/AV1 where the hardware allows.
2. **Real multi-track audio in one file:** game, voice chat (Discord etc.), mic and "other apps", each mutable at export.
   - **Evidence:** AMD writes a separate mic file. Steam has a single mix. No separate-track option was found for Game Bar. ShadowPlay offers game + mic only (user reports).
   - **Feasibility:** OBS's process-loopback capture works from build 19041, which covers Win10 22H2 (19045) and Win11. Test on real Win10 22H2, since Microsoft documents 20348 as the minimum.
3. **Privacy-first by design:** local-only by default, no account to record, no ads, no telemetry or opt-in only, a public "your clips are never used for AI training" promise, and a clean uninstall.
   - **Evidence:** Medal and General Intuition, the Insights Shard Program, the NVIDIA MCP default-on backlash, and Medal uninstall complaints.
4. **Lightweight native footprint**, with no Electron, no Overwolf and no game injection.
   - **Evidence:** Medal is Electron at 100-400 MB RAM by its own claim. Outplayed needs the Overwolf client. Insights rebuilt V3 to drop Overwolf for performance. The new Clippy pitches the same idea but only mentions NVENC.
5. **Sharing without accounts and with fair retention** (later stage). Options:
   - Discord-friendly export presets (size-targeted encodes).
   - User-owned storage (S3, R2, SFTP, as ShareX does).
   - A Kairo link service with clearly better retention than Steam's 2 days or Medal's free 14 days.
   - **Watch cost:** Medal and Outplayed fund hosting with ads and subscriptions.
6. **A fast editor plus vertical (9:16) export** with crop/reframe, captions and quick trims.
   - **Evidence:** Game Bar, AMD and NVIDIA have little or no editing. Medal and Allstar win creators on mobile and vertical.
7. **Any-window capture, plus capture-card and cloud-gaming sources, with the same replay buffer.**
   - **Evidence:** Steam only captures Steam games. Game Bar is limited to one window and no desktop. The GFN overlay works only inside GFN. No consumer clipper found offers a capture-card replay except OBS.
   - **Risk:** DRM or protected surfaces in some cloud clients (n/v; test).
8. **Smart highlights**, later. Candidates: game-integration APIs (where third parties are allowed), audio-spike or kill-feed vision detection run locally (privacy angle), and Steam Timeline only for games that integrate it. This is hard and game-specific; incumbents (Overwolf events, NVIDIA Highlights, SteelSeries) cover only a few dozen to a few thousand titles.
9. **Open source (optional).** OBS (GPL-2.0) and ShareX (GPL-3.0) show the trust value of OSS. No OSS Windows instant-replay app with a library, editor and sharing was found. ReplaySorcery and GPU Screen Recorder are Linux-only [M]. Open-sourcing affects monetization and licensing choices.
10. **HDR-correct screenshots and clips** with SDR tone-mapped exports. Several vendors shipped HDR color fixes (NVIDIA 10.0.2 and 11.0.4.159; ShareX 22.0 HDR corrector) [H], which shows real pain.

---

## 8. Pricing anchors (for a later monetization stage)

- Medal Premium: $9.99/mo or $7.99/mo yearly [H].
- Allstar: $0.99, $4.99 and $11.99/mo tiers [M].
- Outplayed Premium: price n/v.
- Insights teams: $39-$229/mo [H].
- Eklipse: $19.99/mo [M].
- Killcam.gg: $4.99/mo [L].
- Discord Nitro is required for Clips [H].
- The built-ins (NVIDIA, AMD, Game Bar, Steam, OBS, ShareX) are free. The free baseline is high, so a paid tier must sell cloud, sharing or editing extras, not basic replay.

---

## 9. Name check: "Kairo"

**Existing uses in gaming and software**
- **KAIRO (kairoclips.com).** A sports/football "highlight reels" SaaS: "Sign in to upload match videos". It has a KAIRO logo and pricing in GBP: Free with 3 reels/mo and a "Kairo watermark"; Pro £5 (or £3/mo yearly) with 15 reels/mo; Ultra Pro £10 (or £8/mo yearly) with 100/mo. Handle "@kairo_plays". The domain was **registered 2026-05-24** (Namecheap) [H].
  - **This is the most direct collision:** same name and the same "clips/highlights" vocabulary, though in sports rather than PC gaming.
- **Kairosoft** (Tokyo game developer) uses "Kairo" as a brand prefix: Kairo Land (hub app), Kairo Club, Kairobotica. kairo.tv redirects to the Kairosoft store [H/M].
- ***Kairo*, a game on Steam** (app 233230), released 2013-04-24, developer Locked Door Puzzle, Windows/macOS/Linux [H].
- **Other "Kairo" products:**
  - getkairo.com: time, habit and goals tracker [H].
  - kairoapp.com: iOS time tracking [H].
  - kairo.so: content engine for local businesses [H].
  - kairo.com: KAIRO Consultores, Spain [H].
- **Trademarks** (secondary aggregators; official registries not queried):
  - US application "KAIRO NOVUS", serial 99027038, Class 9 AI software, filed 2025-02-03 [L/M].
  - UK application UK00004237072 "KAIRO" (Kairo Consulting Ltd, UK TM Journal 2025-10-03) [L/M].
  - Indian "KairoID" (Class 42) [L].
  - **No plain "KAIRO" US Class 9 registration surfaced**, but this is not a clearance search.

**Domains (DNS, RDAP and WHOIS snapshot, 2026-10-21; confirm at a registrar)**

| Domain | Status |
|---|---|
| kairo.gg | **Taken**. Registered 2025-10-08 (GoDaddy). Content not visible (flagged as a "newly observed domain"). |
| kairo.app | **Taken** since 2018-08-27 (Namecheap). Namecheap parking page. Possibly buyable via aftermarket. |
| getkairo.com | **Taken** (2021-02-14). An active habit-tracker product. |
| kairo.io | Taken. Redirects to an Afternic "lander", so probably **for sale** (price unknown). |
| kairoclips.com | **Taken** (2026-05-24). The KAIRO sports-highlights product. |
| kairoapp.com / usekairo.com / trykairo.com / kairo.so / kairo.com / kairo.tv | Taken |
| getkairo.app | Registered 2025-08-25 (Cloudflare). Now in **"redemption period / pending delete"**, so it may drop soon. |
| kairoclip.com | **Unregistered** (Verisign "No match") |
| kairo.video | No DNS. RDAP not available, so the status is **unverified**. |

**Assessment.** "Kairo" is usable for a private beta. Before going public, plan a proper clearance search in USPTO, EUIPO, UK IPO and WIPO for Classes 9, 41 and 42. Also budget for a different primary domain, or a modifier such as "Kairo Replay", "getkairo.gg" or "kairo.video".

---

## 10. Unverified items and open research debt

- Outplayed Premium price. Medal's desktop sign-in requirement. Max Instant Replay length in the NVIDIA app.
- Per-app audio in the NVIDIA app and Game Bar. Steam Quick Sync encode. Insights V3's real distribution (standalone vs the Overwolf link).
- Allstar's current public prices: observed in client code and could be A/B tests.
- Eklipse yearly price ($119.99 inferred vs $149.99).
- Install sizes for Medal, Adrenalin and Insights.
- Reddit sentiment, which was not retrievable this session.
- Trademark registries, which were not queried directly.
- Whether cloud-gaming clients block window capture (DRM).
- AMD 120 fps capture: no evidence found, but AMD's doc is from Nov 2024.

---

## 11. Questions the user should answer (product decisions)

1. Exact GPU model (AMD RDNA2/3/4? Intel Arc A/B or iGPU?) and monitor refresh/HDR. These decide AV1/HEVC availability and the default FPS (60 vs 120/144).
2. Is "no account, ever, for local use" a hard promise? Will sharing in a later stage also work without an account?
3. Will Kairo publicly commit to "no ads, no data sale, never used for AI training"? This shapes the business model.
4. Default and maximum replay length (30 s? 2 min? 20 min?), and RAM vs disk ring buffer (RAM is faster; disk allows long buffers).
5. Which voice and chat apps do you and your friends use (Discord, in-game, Steam chat)? Should every app get its own track?
6. Consoles and cloud: which ones (Xbox, PS5, Switch 2, GeForce NOW, Xbox Cloud)? Do you own or plan to buy a capture card?
7. Where do clips go: Discord DMs or servers, TikTok/Shorts, YouTube? Is a Discord-size preset more urgent than hosted links?
8. Is vertical (9:16) export with auto-crop needed in v1?
9. Auto-highlights: which games matter to you? Are they a v1 requirement or a later stage?
10. Open source or closed source? If you consider reusing OBS (libobs), GPL-2.0 obligations apply.
11. Keep the name "Kairo" given the KAIRO sports-highlights collision, Kairosoft and the 2013 game? What domain budget is there (kairo.app and kairo.io aftermarket vs a modifier domain)?
12. Screenshots: just capture, or also annotate, upload or HDR→SDR? (ShareX is the bar for power users.)

---

## Sources

Dates in parentheses are publication/update dates where known. "Accessed" means read on 2026-10-21.

**Medal / General Intuition**
- https://medal.tv/premium (accessed; ©2026)
- https://medal.tv/blog/posts/same-price-more-stuff-check-out-the-updates-to-medal-premium (2026-06-17)
- https://support.medal.tv/support/solutions/articles/48001218475-medal-premium-faq (modified "Sat, 5 Sep", year not shown)
- https://medal.tv/blog/posts/introducing-longer-uploads-for-everyone (2025-01-14)
- https://medal.tv/blog/posts/medal-vs-medal-premium-benefits-and-features (2024-11-18)
- https://medal.tv/blog/posts/medal-changelog-updates-2025 (2025)
- https://medal.tv/blog/categories/features (accessed)
- https://support.medal.tv/support/solutions/articles/48001259493-how-to-upload-clips-expiring-links (accessed via search)
- https://support.medal.tv/support/solutions/articles/48001258674-auto-upload-faq (via search)
- https://support.medal.tv/support/solutions/articles/48001147502-what-is-medal-and-how-to-create-an-account (accessed)
- https://medal.tv/features/separate-audio-tracks (2026-10-01; vendor marketing)
- https://medal.tv/developer/reddit-landing-trust (accessed; vendor claims)
- https://support.medal.tv/support/solutions/articles/48001281414-opt-out-of-ai-training (modified "Mon, 16 Mar")
- https://support.medal.tv/support/solutions/articles/48001197741 (Xbox import; via search)
- https://www.trustpilot.com/review/medal.tv (accessed)
- https://techcrunch.com/2025/10/16/general-intuition-lands-134m-seed-to-teach-agents-spatial-reasoning-using-video-game-clips/ (2025-10-16)
- https://mobilegamer.biz/general-intuition-raises-220m-at-6-2bn-valuation-trains-ai-on-gameplay-clips/ (2026-09-29)

**Outplayed / Overwolf**
- https://outplayed.tv/ (accessed)
- https://www.overwolf.com/app/overwolf-outplayed (accessed)
- https://support.overwolf.com/support/solutions/articles/9000284628-outplayed-sharing-clips-faq (modified 2026-09-16)
- https://support.overwolf.com/support/solutions/articles/9000234362-outplayed-premium-switching-to-tebex-payment-system (via search snippet; now 404)
- https://support.overwolf.com/support/solutions/articles/9000184425-performance-issues-fps-cpu-memory (via search snippet)
- https://discuss.overwolf.com/t/feature-request-disable-game-injection-but-without-disabling-auto-startup-game-integration/2919 (via search snippet)
- https://www.trustpilot.com/review/outplayed.tv (accessed)
- https://insights.gg/blog/insights-vs-outplayed (2026-10-08; competitor)

**NVIDIA**
- https://www.nvidia.com/en-us/software/nvidia-app/release-highlights/ (accessed; undated entries)
- https://www.nvidia.com/en-us/software/nvidia-app/system-requirements/ (accessed)
- https://www.nvidia.com/en-gb/geforce/geforce-experience/shadowplay/ (accessed; installer 11.0.9.565)
- https://www.windowsmode.com/?p=47973 (2026-10-17)
- https://letsdatascience.com/news/nvidia-enables-local-mcp-control-in-app-50d4b946 (via search)
- https://tomshardware.com/pc-components/gpu-drivers/nvidia-bids-goodbye-to-geforce-experience-nvidia-app-officially-replaces-it-in-the-latest-driver-update (Nov 2024; via search)
- https://alternativeto.net/news/2024/11/nvidia-officially-launches-its-new-all-in-one-nvidia-app-replacing-geforce-experience (Nov 2024; via search)
- https://blogs.nvidia.com/blog/geforce-now-highlights-freestyle-montage (2021-09-13)

**AMD**
- https://www.amd.com/en/resources/support-articles/faqs/DH3-023.html (last updated 2024-11-11)
- https://www.amd.com/en/resources/support-articles/release-notes/RN-RAD-WIN-26-10-1.html (2026-10-20)
- https://www.amd.com/en/resources/support-articles/release-notes/RN-RAD-WIN-26-9-2.html (Sept 2026)
- https://www.amd.com/en/resources/support-articles/release-notes/RN-RAD-WIN-26-9-1.html (2026-09-03)
- https://community.amd.com/t5/pc-drivers-software/relive-instant-replay-causing-game-stutter/m-p/654813 (via search; now redirects)
- https://community.amd.com/t5/general-discussions/amd-instant-replay-saving-separate-tracked-audio-in-separate-m4a/td-p/631649 (via search; now redirects)

**Intel**
- https://www.intel.com/content/www/us/en/support/articles/000100252.html (last reviewed 2026-07-07)
- https://community.intel.com/t5/Graphics/Feature-Requests-for-Intel-Graphics-Software-and-Arc-GPUs/td-p/1750326 (2026-06-05)
- https://community.intel.com/t5/Intel-ARC-Graphics/Instant-Replay-needed/td-p/1693001 (2025-05-28)
- https://www.kitguru.net/gaming/joao-silva/intels-new-graphics-software-removes-studio-recording-and-streaming-features/ (Dec 2024; via search)

**Xbox Game Bar / Windows**
- https://support.xbox.com/en-US/help/friends-social-activity/share-socialize/record-game-clips-game-bar-windows-10 (official; JavaScript-only, not readable)
- https://www.dell.com/support/kbdoc/en-us/000103410/how-to-use-the-windows-10-game-bar-to-capture-video-from-applications (2026-04-18)
- https://geekrewind.com/how-to-manage-background-recording-length-settings-in-windows-11-game-bar/ (2026-09-23, updated 2026-10-07)
- https://www.ionos.com/digitalguide/server/configuration/screen-capture-windows-11/ (via search)
- https://elevenforum.com/t/change-video-frame-rate-for-game-recording-in-windows-11.18224/ (via search)
- https://alternativeto.net/news/2025/3/microsoft-revamps-its-xbox-game-bar-ui-with-an-improved-design-for-handheld-devices/ (2025-03)

**Steam**
- https://help.steampowered.com/en/faqs/view/23B7-49AD-4A28-9590 (FAQ timestamp 2024-07-10)
- https://store.steampowered.com/gamerecording (accessed)
- https://partner.steamgames.com/doc/features/timeline (accessed)
- https://steamcommunity.com/ogg/593110/announcements/detail/6087258371786989398 (beta, 2024-06-26)
- https://steamstore-a.akamaihd.net/news/externalpost/steam_community_announcements/6146943657174074702 (GA, 2024-11-05)
- https://api.steampowered.com/ISteamNews/GetNewsForApp/v2/?appid=593110 (client update notes: 2024-11-12, 2025-05-20, 2026-04-28, 2026-10-13 and others)
- https://www.gamingonlinux.com/2024/10/steam-beta-gets-big-game-recording-improvements-plus-steam-deck-steamos-3616-beta-out-now/ (2024-10-03)
- https://steamcommunity.com/groups/SteamClientBeta/discussions/5/4514379558399445210 (2024-07-21 to 2024-09-10)
- https://www.techspot.com/community/topics/steam-client-gets-game-recording-editing-and-sharing-capabilities.286622/ (via search)

**OBS Studio**
- https://github.com/obsproject/obs-studio/releases (33.0.0-rc3 2026-10-20; 32.2.2 2026-08-14; 32.2.0 2026-07-21; 32.1.0 2026-03-11; 32.0.0 2025-09-22)
- https://github.com/obsproject/obs-studio/blob/32.2.2/plugins/win-wasapi/plugin-main.cpp (build-19041 gate)
- https://github.com/obsproject/obs-studio/blob/32.2.2/plugins/win-wasapi/win-wasapi.cpp (process loopback)
- https://github.com/obsproject/obs-studio/blob/32.2.2/frontend/widgets/OBSBasic.cpp (replay-buffer defaults)
- https://obsproject.com/forum/threads/replay-buffer-cache-to-disk-instead-of-ram.53738 (via search)

**Insights**
- https://insights.gg/pricing (accessed)
- https://insights.gg/v3-updates (accessed)
- https://insights.gg/shard-program (accessed)
- https://insights.gg/insights-capture (accessed)
- https://insights.gg/blog/insights-vs-medal (2026-09-02; competitor)
- https://insights.gg/blog/best-medal-alternatives (2026-09-02; competitor)

**SteelSeries**
- https://steelseries.com/gg/moments (accessed)
- https://techblog.steelseries.com/ (GG 117-122, Aug-Oct 2026)
- https://techblog.steelseries.com/page2/ (GG 109-113, Apr-Jun 2026)

**Allstar**
- https://allstar.gg/ and https://allstar.gg/static/js/main.8371dc8e.js (tier strings, accessed 2026-10-21)
- https://allstar.gg/sitemap.xml (accessed)
- https://eklipse.gg/compare/allstar-alternatives/ (checked 2026-07-15; competitor)
- https://gamesbeat.com/allstar-raises-12m-content-creation-studio/ (Series A; via search)
- https://www.cbinsights.com/company/allstar (via search)

**Discord**
- https://support.discord.com/hc/en-us/articles/16861982215703 (updated 2026-10-16; read via the Zendesk API)
- https://techcrunch.com/2025/03/25/discord-made-its-streaming-overlay-a-lot-more-user-friendly/ (2025-03-25)

**AI highlight tools and new entrants**
- https://eklipse.gg/help/what-are-the-main-differences-between-the-free-and-premium-plans/ (2026-08-13)
- https://eklipse.gg/help/plus-plan-moves-to-premium/ (via search)
- https://www.capterra.com/p/10015243/Eklipse/pricing/ (via search)
- https://blog.eklipse.gg/tools/top-alternatives-powder-gg-tsai.html (via search; competitor)
- https://clip.dor.gg/en/blog/medal-tv-alternative (vendor)
- https://www.thailand-business-news.com/?p=331751 (Killcam.gg; via search)
- https://streamyard.com/blog/gaming-highlights-ai-streamyard (2026-01-15; via search)
- https://www.hunted.space/product/clippy-9 (Clippy; May 2026)
- https://alternativeto.net/software/medal (via search)

**ShareX**
- https://getsharex.com/changelog (22.0.0 2026-10-20; 21.0.0 2026-07-03)
- https://github.com/ShareX/ShareX/releases

**Console / cloud**
- https://support.medal.tv/support/solutions/articles/48001197457-how-to-connect-your-xbox-account-to-medal (via search)
- https://en-americas-support.nintendo.com/app/answers/detail/a_id/68293/ (Switch 2 capture; via search)
- https://www.notebookcheck.net/Despite-improved-Nintendo-Switch-2-specs-video-captures-are-still-limited-to-30-seconds.1019105.0.html (via search)

**Name check**
- https://store.steampowered.com/app/233230 (Kairo, 2013-04-24; via the Steam appdetails API)
- https://en.wikipedia.org/wiki/Kairosoft
- https://toucharcade.com/games/kairo-land
- https://kairosoft.wiki.gg/wiki/Kairo_Club
- https://kairoclips.com/ and https://kairoclips.com/pricing (accessed)
- https://getkairo.com
- https://kairoapp.com
- https://kairo.so
- https://kairo.com
- https://trademarks.justia.com/990/27/kairo-99027038.html (via search; 403 on fetch)
- https://www.ipo.gov.uk/t-tmj/tm-journals/2025-040/UK00004237072.html (via search; 403 on fetch)
- https://rdap.org/domain/kairo.app
- https://rdap.org/domain/getkairo.app
- https://rdap.org/domain/usekairo.com
- https://rdap.org/domain/kairoapp.com
- https://rdap.org/domain/trykairo.com
- WHOIS: whois.gg (kairo.gg) and whois.verisign-grs.com (kairoclips.com, kairoclip.com, getkairo.com), queried 2026-10-21
