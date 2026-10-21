# 06 — After the clip: library, editing, export and sharing (local-first, cloud later)

Research for **Kairo** (Windows-only instant-replay clipper). Prepared 2026-10-21.
Method: primary sources (vendor docs, pricing pages and price APIs, Microsoft Learn, GitHub source and docs, legal texts), fetched on 2026-10-21 unless a date is noted. I also ran FFmpeg 8.0.1 experiments on the developer's M2 Mac to check the editing and export commands. Those numbers show how the commands behave. They are not Windows/GPU performance numbers.
Confidence tags: **[H]** verified in a primary source or by experiment · **[M]** secondary source, inference from source code, or partial verification · **[L]** unverified or from memory, flagged for a check.
Tooling note: the shared web-search budget ran out late in this research. A few items, all marked [M] or [L], could not be re-checked and are listed under "Open verification items" in §9.

---

## 0. TL;DR: decisions this research supports

1. **Library = files on disk + a SQLite index in WAL mode.** Clips are user-visible MP4s under a library root. The DB, thumbnails and sprites live under `%LOCALAPPDATA%\Kairo` and can be rebuilt. Trims are stored as non-destructive in/out points. **[H]** for the SQLite facts; the layout itself is a design recommendation.
2. **Make Kairo's capture encoder emit a keyframe every 1 s (closed GOP).** This one capture-side setting makes lossless trims snap to within 1 s and makes merges instant stream-copies. It also gives one hover-scrub thumbnail per second almost for free (keyframe-only decode: about 0.5 s per 30 s 1080p60 clip on the M2). **[H]** experiment.
3. **Codec choice drives your UI-stack choice.** In **WebView2** (Tauri, Wails, .NET hybrids) and in **Media Foundation** (WinUI MediaPlayerElement), HEVC plays only if the user has a valid **HEVC Video Extensions** licence ($0.99 in the Microsoft Store; a free OEM variant ships on some PCs). **Electron/Chrome** decode HEVC through D3D11VA with no extension, but only on GPUs with hardware HEVC decode. AV1 plays in Chromium/WebView2 without an extension. **[H]**
4. **Every share/export copy is H.264 + AAC in MP4 with `+faststart`.** That plays everywhere, embeds in Discord, and fits Instagram's "moov atom at the front, no edit lists" rule. **[H]**
5. **Discord's free upload limit is 20 MB again (since Aug 13–14, 2026)**, not 10 MB or 25 MB. Nitro Basic is 50 MB. Nitro is "up to 1 GB" per Discord's Help Center (edited 2026-09-29), while Aug 2026 press said 500 MB. Boost Level 2 is 50 MB and Level 3 is 100 MB. Target ≤ 19,000,000 bytes for the free preset. **[H]**, except the Nitro figure **[M]** because Discord says it is "experimenting with limits".
6. **No-backend sharing is enough for the private beta:** Copy-file-to-clipboard (CF_HDROP; Chromium-based apps such as Discord read it), drag-out, "Show in folder", target-size export presets, and optionally "post to my Discord server" via a webhook the user pastes. Infra cost $0. **[H]** API facts, **[M]** paste behaviour in specific apps.
7. **When share links arrive, use progressive MP4 on Cloudflare R2** (custom domain; R2-hosted video is allowed on Cloudflare's CDN; egress is free). Worked example (1,000 users × 50 clips × 30 MB, 30-day retention, 20 views per clip = 30 TB/month egress): **R2 ≈ $22/mo; S3 + CloudFront flat-rate Pro ≈ $49/mo; B2 + Bunny CDN ≈ $160/mo; Bunny Stream ≈ $165–324/mo; Mux ≈ $380–475/mo; Cloudflare Stream ≈ $625/mo; S3 + CloudFront pay-as-you-go ≈ $2,400/mo.** **[H]** prices, **[M]** traffic assumptions.
8. **YouTube changed upload quota in 2025–26.** `videos.insert` now has its own bucket with a **default of 100 uploads/day per Cloud project** (all Kairo users combined), 1 unit each, since 2026-06-01. Before that it was ~1,600 units, then ~100 units (2025-12-04). That is fine for an "Upload to my YouTube" button, but YouTube cannot host every clip. **[H]**
9. **Social features (profiles, feeds, comments) carry real legal duties.** Under the EU DSA, every hosting service must provide notice-and-action, statements of reasons and contact points. "Online platform" duties largely exempt micro/small enterprises. A US DMCA agent costs $6 per 3 years. US CSAM reporting applies on actual knowledge, with no duty to scan. UK Ofcom codes expect CSAM hash-matching for higher-risk file-sharing services. Keep v1 links **unlisted** and defer public feeds. **[H]**

---

## 1. Library

### 1.1 Storage layout (recommended)

```
<Library root>  (default: Known Folder FOLDERID_Videos\Kairo; user-changeable, any local NTFS drive)
  Valorant\
    2026-10-21_21-14-03_Valorant.mp4        <- the clip (MP4 with moov at front)
    2026-10-21_21-14-03_Valorant.png        <- screenshots live beside clips (or Screenshots\ subfolder)
  Desktop\ ...                              <- "anything else on your screen" captures
  _Exports\
    2026-10-21_21-14-03_Valorant.discord20.mp4
    2026-10-21_21-14-03_Valorant.vertical.mp4
%LOCALAPPDATA%\Kairo\
  library.db  (+ library.db-wal, library.db-shm)   <- SQLite, WAL mode, never on a network/cloud folder
  cache\posters\<clipId>.webp                      <- regenerable
  cache\sprites\<clipId>.jpg (+ .json map)         <- regenerable hover-scrub sheets
  buffer\                                          <- rolling replay buffer (capture topic)
```

Why this shape:
- **Clips stay ordinary files.** Users can open them in Explorer, back them up and drag them anywhere. If the DB is lost, Kairo rebuilds the index by scanning the root with ffprobe.
- **Mirror the important metadata into the file itself**: the MP4 `title`/`comment` tags (`-metadata title=… -metadata comment='{"kairo":…}'`), or a hidden sidecar `.kairo.json`. Then a rescan restores game, time and favourites.
- **Keep the DB local.** SQLite WAL requires all processes on the same host and "does not work over a network filesystem" [H, sqlite.org/wal]. If the user puts the library on a NAS, the DB still stays in `%LOCALAPPDATA%`.
- **OneDrive trap [M]:** Windows "Known Folder Move" often redirects `Videos` into OneDrive. Multi-GB clip folders then sync, eat OneDrive quota, and can become online-only placeholders. LosslessCut's troubleshooting guide warns that cloud-synced folders "may serve incomplete (on-demand) files or interfere with files while they are being written" [H for the LosslessCut statement]. On first run, detect a redirected Videos folder and suggest a local path such as `D:\Kairo`.
- **Watch the folder** (`ReadDirectoryChangesW` / .NET `FileSystemWatcher` / Rust `notify`) so renames and deletes made in Explorer update the index.

### 1.2 SQLite index (sketch)

```sql
PRAGMA journal_mode=WAL;          -- readers don't block the single writer
PRAGMA synchronous=NORMAL;        -- usual pairing with WAL for desktop apps
CREATE TABLE games    (id INTEGER PRIMARY KEY, name TEXT, exe TEXT, steam_app_id INTEGER, cover_path TEXT);
CREATE TABLE sessions (id INTEGER PRIMARY KEY, game_id INTEGER REFERENCES games, started_at INTEGER, ended_at INTEGER);
CREATE TABLE clips (
  id TEXT PRIMARY KEY,                    -- ULID: time-sortable, safe to expose in share URLs later
  kind TEXT NOT NULL,                     -- 'replay' | 'recording' | 'screenshot'
  rel_path TEXT NOT NULL UNIQUE,          -- relative to library root (survives drive-letter changes)
  size_bytes INTEGER, duration_ms INTEGER, width INTEGER, height INTEGER, fps REAL,
  vcodec TEXT, acodec TEXT, audio_tracks INTEGER,
  game_id INTEGER REFERENCES games, session_id INTEGER REFERENCES sessions,
  captured_at INTEGER NOT NULL,           -- UTC ms of the hotkey press
  tz_offset_min INTEGER,
  buffer_seconds INTEGER,                 -- the N in "last N seconds"
  trim_in_ms INTEGER, trim_out_ms INTEGER,-- non-destructive trim
  title TEXT, favorite INTEGER DEFAULT 0, locked INTEGER DEFAULT 0,  -- locked = never auto-delete
  poster_path TEXT, sprite_path TEXT, sprite_interval_ms INTEGER,
  content_hash TEXT,                      -- BLAKE3/xxHash: dedupe + idempotent uploads later
  deleted_at INTEGER, created_at INTEGER, updated_at INTEGER);
CREATE INDEX clips_by_time ON clips(captured_at DESC);
CREATE INDEX clips_by_game ON clips(game_id, captured_at DESC);
CREATE TABLE exports (id INTEGER PRIMARY KEY, clip_id TEXT, preset TEXT, rel_path TEXT, size_bytes INTEGER, created_at INTEGER);
CREATE TABLE shares  (id INTEGER PRIMARY KEY, clip_id TEXT, target TEXT, remote_id TEXT, url TEXT, created_at INTEGER);
CREATE VIRTUAL TABLE clip_search USING fts5(title, game, tags);   -- search box
```

Metadata worth capturing at save time: game (exe name and window title, plus Steam AppID or other store IDs where detectable), session (contiguous run of the game process; start a new session after e.g. 15 min idle), UTC capture time plus local offset, buffer length, encoder/codec/bitrate/resolution/fps, audio track layout (game / mic / voice chat), and the monitor/window captured. Cover art can come from a game DB later.

### 1.3 Thumbnails and hover-scrub previews (measured)

FFmpeg 8.0.1 on the M2, 30 s 1080p60 H.264 clip at ~20 Mbit/s with a 2 s GOP [H, experiment]:

| Job | Command core | Wall time | Output |
|---|---|---|---|
| Poster frame | `-ss 15 -i clip.mp4 -frames:v 1 -vf scale=480:-2` | 0.17 s | 11 KB JPG |
| Scrub sprite, keyframes only | `-skip_frame nokey -i clip.mp4 -vf scale=160:90,tile=8x2 -fps_mode vfr -frames:v 1` | 0.48 s | 31 KB, 1 thumb per keyframe |
| Scrub sprite, full decode 1 fps | `-i clip.mp4 -vf fps=1,scale=160:90,tile=10x3 -frames:v 1` | 1.94 s (12 s CPU) | 64 KB |
| Animated WebP preview, keyframes | `-skip_frame nokey … -c:v libwebp` | 0.48 s | 42 KB |

- With a **1 s GOP**, the cheap keyframe-only path yields one thumbnail per second. That is enough for hover-scrub over a 30–120 s clip.
- Render hover-scrub in the UI as a CSS `background-position` over the sprite. Seeking a hidden `<video>` on hover is much more expensive.
- Thumbnail and sprite jobs must run at **low priority** (`IDLE_PRIORITY_CLASS` or `PROCESS_MODE_BACKGROUND_BEGIN` on the ffmpeg child). Pause or throttle them while a game is in the foreground so FPS doesn't suffer.
- **Where thumbnails live:** SQLite's measurements show blobs **< 100 KB read faster from inside the DB than from separate files**, and 10 KB blobs about 35% faster than direct disk I/O (SQLite 3.19). Posters and sprites (10–70 KB) could therefore be BLOBs [H, sqlite.org]. In a web-tech UI it is usually simpler to serve them as files through the same local protocol as the videos. Either works at thousands of clips.

### 1.4 Disk quotas and auto-cleanup (policy recommendation)

- Three independent limits, each user-configurable: **max library size** (e.g. 100 GB), **min free space on the drive** (e.g. 15 GB or 5%, via `GetDiskFreeSpaceExW`), and **max age for unstarred clips** (off by default).
- Delete **oldest first among clips that are not favourite, locked, edited, exported or shared**. Never touch the user's manual exports folder.
- Show a dry run ("will remove 37 clips, 12.4 GB") and a toast after the cleanup.
- **Auto-cleanup should delete permanently.** Moving files to the Recycle Bin does not free space until the bin is emptied. **Manual deletes** go to the Recycle Bin: Electron `shell.trashItem(path)` [H, Electron docs]; .NET `Microsoft.VisualBasic.FileIO.FileSystem.DeleteFile(path, …, RecycleOption.SendToRecycleBin)`; Win32 `IFileOperation` with `FOFX_RECYCLEONDELETE` [M].
- Keep the replay-buffer budget separate. That is the capture topic.

### 1.5 Performance with thousands of clips

- **SQLite:** 10k–100k rows is trivial with the two indexes above. Keep one writer thread (WAL lets readers proceed concurrently [H]). Use prepared statements and page results by `captured_at` cursor (`WHERE captured_at < ? ORDER BY captured_at DESC LIMIT 60`), not OFFSET.
- **UI:** virtualise the grid. Web: TanStack Virtual / react-window. WinUI: `ItemsRepeater`/`GridView` with virtualisation. Load posters lazily with `IntersectionObserver`.
- **Index-time work** (ffprobe, poster, sprite, hash) goes into a background queue that persists across restarts (a `jobs` table).
- **Serving local media to a web UI (gotchas by stack) [H]:**
  - *WebView2* (Tauri/Wails/.NET hybrid): `SetVirtualHostNameToFolderMapping` is the documented approach, but Microsoft's own docs say "*Due to a current limitation, media files that are accessed using a virtual host name can be slow to load*" (doc updated 2026-09-02). `WebResourceRequested` is slower still, because it runs on the host UI thread. Benchmark early. The fallback is a loopback HTTP server with Range support.
  - *Electron:* register the scheme as privileged with `stream: true`. The `<video>` element otherwise expects buffered responses. Serve with `protocol.handle` + `net.fetch(file://…)`. Recent Electron also has an experimental `protocol.registerSource` that streams files like `file:` URLs without touching the main thread.
  - *Tauri:* the asset protocol (`convertFileSrc`) needs scope configuration. A long-open issue (#4133) reports whole-file reads unless a `Range` header is sent, so test seeking on 500 MB+ recordings [M].

---

## 2. Editing

### 2.1 Trim: lossless keyframe cut vs frame-accurate re-encode (experiment)

Source: 30 s, 1080p60, keyframes every 2.0 s. Request: start 7.3 s, length 10 s, `-c copy` [H, experiment; MP4 boxes parsed with a small script]:

| Variant | What ffmpeg wrote | Plays as | Notes |
|---|---|---|---|
| `-ss 7.3 -i in.mp4 -t 10 -c copy` (default) | 680 frames from the **6.0 s keyframe** + an **MP4 edit list** (`media_time` = 1.333 s) | **10.05 s, starting at 7.3 s** in players that honour edit lists (FFmpeg decodes 602 frames) | Lossless *and* visually frame-accurate, but the file still carries 1.3 s of hidden pre-roll |
| `… -avoid_negative_ts make_zero` | Pre-roll shifted to t = 0 | 11.33 s, starting at the 6.0 s keyframe | Start snaps to the previous keyframe |
| `… -use_editlist 0` | No edit list | 11.35 s from the keyframe | Same as above, maximally compatible |

Implications:
- **Edit-list trims are fine inside Kairo.** FFmpeg honours them. Chromium's MP4 parser applies a single edit with non-negative `media_time` (source: `media/formats/mp4/track_run_iterator.cc`) [M: inferred from source code; verify in WebView2/Electron].
- **Don't publish them.** Instagram's Reels spec explicitly requires "**no edit lists, moov atom at the front**" [H]. Other platforms and players may ignore edit lists and show the hidden pre-roll.
- **Precise trim = re-encode** with the GPU encoder that is already present (NVENC/AMF/QSV). For 10–60 s clips this takes seconds. It is the right default for exports.
- **"Smart cut"** (re-encode only from the cut to the next keyframe, then stream-copy) is still marked *experimental* in LosslessCut, which "will not work for many files" [H]. Don't build it for v1. A **1 s GOP** from the capture side caps the lossless-mode error at under 1 s. LosslessCut's own docs recommend reducing the keyframe interval when you control the encoder [H].
- UX: offer **Quick (lossless, snaps to keyframe)** and **Precise (re-encode)**. Keep the trim non-destructive in the DB until "Save as new clip" or "Export".

### 2.2 Vertical 9:16 for TikTok / Shorts / Reels (tested filter graphs)

All three produced 1080×1920 output [H, experiment]:

```bash
# A) Facecam on top (cam region assumed at top-left 480x270 of a 1920x1080 frame), gameplay below
-filter_complex "[0:v]split=2[cam][game];
  [cam]crop=480:270:0:0,scale=1080:608:flags=lanczos,setsar=1[top];
  [game]crop=ih*1080/1312:ih:(iw-ih*1080/1312)/2:0,scale=1080:1312:flags=lanczos,setsar=1[bottom];
  [top][bottom]vstack=inputs=2[v]"
# B) Centre crop
-vf "crop=ih*9/16:ih:(iw-ih*9/16)/2:0,scale=1080:1920:flags=lanczos,setsar=1"
# C) Full frame over blurred background ("fit")
-filter_complex "[0:v]split=2[bg][fg];[bg]scale=1080:1920:force_original_aspect_ratio=increase,crop=1080:1920,boxblur=20:2[b];
  [fg]scale=1080:-2[f];[b][f]overlay=(W-w)/2:(H-h)/2[v]"
```

- The facecam rectangle must come from the user: a draggable box in the editor, remembered per game. A webcam captured as a *separate* track/file (capture topic) makes this far better than cropping it out of the game frame.
- The crop window can be keyframed (pan to the action) with ffmpeg `sendcmd`/expressions. That is a v2 feature.
- Instagram Reels spec [H]: MP4/MOV, H.264 or HEVC, closed GOP, 4:2:0, AAC ≤ 48 kHz, 23–60 fps, max 1920 px wide, 3 s–15 min, ≤ 300 MB, 9:16 recommended, VBR ≤ 25 Mbit/s. TikTok [H]: MP4/WebM/MOV, H.264 recommended (H.265/VP8/VP9 accepted), 23–60 fps, 360–4096 px per side, ≤ 4 GB.

### 2.3 Merging clips

- Clips from the same capture settings concat **losslessly and almost instantly** with the concat demuxer (two 6 s parts in 0.14 s; output faststart verified) [H]:
  `ffmpeg -f concat -safe 0 -i list.txt -c copy -movflags +faststart merged.mp4`
- Different resolution, fps, codec or encoder settings, or any transition (`xfade`), means a re-encode with the concat *filter*. Kairo controls capture settings, so most same-game merges can stay lossless.

### 2.4 Text and captions

- **Burned-in titles:** `drawtext` (bundle a font and pass `fontfile=` explicitly). **Subtitles:** `subtitles=`/`ass=` (libass). Both tested OK [H].
- **Auto-captions:** FFmpeg 8.0's changelog lists a new **Whisper filter** (`af_whisper`) [H]. It needs a build with whisper.cpp; the Homebrew build tested here lacks it. Alternatively run whisper.cpp as a sidecar, produce SRT/ASS, and let the user edit before burning in.

### 2.5 Multi-track audio caveat (affects editor design) [H]

`HTMLMediaElement.audioTracks` works in Chrome/Edge **only behind `#enable-experimental-web-platform-features`** (MDN browser-compat data). A web-tech editor therefore can't toggle the "game / mic / Discord" tracks of one MP4 natively. Options:
- Store the tracks also as separate `.m4a` sidecars and mix with Web Audio for preview.
- Enable the Blink feature via WebView2/Electron browser arguments [M].
- Always mix down for previews and exports.

Native players (libVLC, FFmpeg-based, Media Foundation) can select tracks.

### 2.6 Player and timeline components

**Web tech** (Tauri / Electron / Wails / WebView2 hybrid). Repo status checked via GitHub on 2026-10-21 [H]:

| Component | Use | Status / licence |
|---|---|---|
| Video.js **10.1.0** (stable 2026-10-12; v10 rewrite with storyboard/thumbnail support) | player chrome | active |
| Vidstack Player | player | MIT, active |
| Media Chrome (Mux) | web-component controls | MIT, active |
| wavesurfer.js (+ Regions plugin) | audio waveform with trim handles | BSD-3, active |
| @xzdarcy/react-timeline-editor | multi-track timeline UI | MIT, last push 2026-01 |
| **Mediabunny 1.62** | WebCodecs-based read/write/convert (MP4/WebM/MKV…), trimming/cropping/transcoding in-browser, microsecond-accurate | MPL-2.0, very active |
| Remotion | programmatic (React→video) rendering | source-available; free for individuals and companies with ≤ 3 employees, company licence above that |

A practical v1 editor is a single-clip trimmer (player, filmstrip from the sprite, two handles, waveform). Ship the multi-clip timeline later. Do the actual export in ffmpeg (sidecar) with hardware encoders. Mediabunny/WebCodecs is a credible alternative for simple exports without bundling ffmpeg, but ffmpeg's filter graphs (vertical layouts, captions) are more mature.

**.NET native**:
- WinUI 3 `MediaPlayerElement` with **`Windows.Media.Editing.MediaComposition`** (docs updated 2026-08 for Windows App SDK). It supports MP4 clips, `TrimTimeFromStart/End`, overlays, background audio, effects, and `RenderToFileAsync` with `MediaTrimmingPreference.Precise|Fast` [H]. It has no text-overlay primitive; render text to an image overlay. It is Media Foundation-based, so the HEVC extension caveat applies.
- **LibVLCSharp** (LGPL-2.1, active) and **Flyleaf** (FFmpeg-based, LGPL-3.0, active) bring their own decoders: HEVC/AV1 without Store extensions [H for repo status, M for codec behaviour].
- **FFME** (unosquare) was last pushed 2024-06 [H]; avoid it.
- There is no good free WinUI timeline control. Expect to draw one (Win2D/SkiaSharp) or host the web timeline in WebView2.

### 2.7 HEVC / AV1 playback on Windows (the paid-extension question)

| Playback path | Who uses it | H.264 | HEVC | AV1 |
|---|---|---|---|---|
| **WebView2 / Edge** (Chromium + Edge media stack) | Tauri, Wails, .NET+WebView2 | yes | **Only with a valid HEVC Video Extension** + working decoder. Edge "relies on the Windows platform MediaFoundationVideoDecoder path" for HEVC; an invalid licence gives `0xC00DB3B3` [H, Microsoft Learn 2026-03-31]. WebView2 is assumed to behave like Edge [M; confirm with `MediaSource.isTypeSupported('video/mp4; codecs="hvc1.1.6.L123.B0"')`] | yes (Edge: "Supported", HW decode device-dependent) [H] |
| **Electron / Chrome** | Electron apps | yes | yes via `D3D11VideoDecoder`/D3D11VA, "no need to install anything" (Electron ≥ 22), but **hardware only**: no software fallback in official builds [H, StaZhu guide] | yes |
| **Media Foundation** (MediaPlayerElement, MediaComposition, Movies & TV) | WinUI/WPF native | yes | needs HEVC Video Extension | needs **AV1 Video Extension (free)** |
| **libVLC / FFmpeg-based players** | LibVLCSharp, Flyleaf | yes | yes (bundled decoders) | yes |
| **Firefox** (share-page viewers) | web | yes | **Windows only** (≥ 133), via MFT + HEVC extension [H, StaZhu guide] | yes |

Microsoft Store prices on 2026-10-21 (US Store API) [H]:
- **HEVC Video Extensions (9NMZLZ57R3T7): $0.99**
- **HEVC Video Extensions from Device Manufacturer (9N4WGH0Z6VHQ): $0.00.** It is hidden from Store search and only preinstalled or linked on some OEM PCs.
- **AV1 Video Extension (9MVZQVXJBQ9V): free.**

Consequence: if Kairo is a WebView2 app **and** records HEVC by default, some users can't play their own clips in Kairo until they pay $0.99. Ways out:
- Default to H.264, or to AV1 where the GPU can encode it.
- Make HEVC opt-in, with an in-app check that probes `isTypeSupported` and links to the Store.
- Use a native libVLC/FFmpeg player surface for playback.
- Generate H.264 preview proxies.

---

## 3. Sharing with no backend

### 3.1 Copy the video file to the clipboard (paste into Discord / WhatsApp)

**Format [H, Microsoft Learn]:** `CF_HDROP` is the predefined format for "the locations of a group of existing files". It holds a `DROPFILES` header followed by a double-NUL-terminated list of full paths. Add **`Preferred DropEffect` = `DROPEFFECT_COPY`** (`CFSTR_PREFERREDDROPEFFECT`). Otherwise Explorer may treat a paste as a move.

**Will Discord/WhatsApp accept it?** Chromium's Windows clipboard code (`ClipboardWin::ReadFilenamesInternal`) reads **`CF_HDROP` first**, then `CFSTR_FILENAMEW`, then `CFSTR_FILENAMEA` [H, Chromium source]. Discord desktop is Chromium/Electron-based, so a CF_HDROP paste arrives as a file attachment, the same as Ctrl+C on a file in Explorer [M: inferred from source plus common use; test Discord and WhatsApp desktop in the beta]. Discord then applies its upload limit (§3.4). "Copy for Discord" should therefore first produce a ≤ 19 MB export and put *that* path on the clipboard.

How to write it, by stack:

| Stack | API |
|---|---|
| .NET WPF / WinForms | `Clipboard.SetFileDropList(StringCollection)` stores FileDrop (CF_HDROP) with auto-conversion [H]. Must run on an STA thread. |
| WinRT / WinUI 3 | `DataPackage.SetStorageItems(…)` + `Clipboard.SetContent(…)` (+ `Clipboard.Flush()` so the data survives app exit) [M] |
| Electron | No first-class "copy files" API. The clipboard module was rebuilt around the W3C Clipboard API in July 2026 and exposes raw *named* OS formats (`electron application/osclipboard;format="…"`) [H]. CF_HDROP is a predefined, unnamed format, so use a tiny native addon or `powershell.exe -NoProfile -Command Set-Clipboard -LiteralPath '<file>'`. Windows PowerShell 5.1 (in-box) supports `-Path/-LiteralPath` [H]. |
| Tauri (Rust) | `tauri-plugin-clipboard` (CrossCopy) has `writeFilesURIs` [H] but was last pushed 2024-10 (maintenance risk). Otherwise ~40 lines of Rust with the `windows` crate (`OpenClipboard`/`SetClipboardData(CF_HDROP)`). |
| Wails (Go) | Call Win32 via `golang.org/x/sys/windows` or shell out to PowerShell as above [L]. |

### 3.2 Drag-and-drop out of Kairo

- Electron: `webContents.startDrag({ file | files, icon })` [H].
- Tauri v2: `tauri-plugin-drag` (CrabNebula `drag-rs`, active 2026-10) [H].
- WPF: `DragDrop.DoDragDrop(src, new DataObject(DataFormats.FileDrop, paths), DragDropEffects.Copy)`. WinUI 3: `DragStarting` → `args.Data.SetStorageItems(...)` [M].
- **UIPI gotcha [H/M]:** Windows blocks messages from lower- to higher-integrity processes, and drag-and-drop runs on them. Dragging from Explorer or Discord (medium IL) into an **elevated** app fails (Microsoft blog, archived). Dragging out of an elevated app is also commonly reported broken [M]. **Keep Kairo's UI process non-elevated.** If capture ever needs admin rights for some games, put that in a separate helper process.

### 3.3 "Show in folder"

Electron `shell.showItemInFolder(fullPath)` [H] · Tauri `revealItemInDir(path)` from `@tauri-apps/plugin-opener` [H] · Win32 `SHOpenFolderAndSelectItems` or `explorer.exe /select,"C:\path\clip.mp4"`.

### 3.4 Discord upload limits (as of 2026-10-21)

| Who | Per-file limit | Source |
|---|---|---|
| Free users | **20 MB** (developer docs: "20 MiB") since **2026-08-13/14**. It was 10 MB from late 2024 / Jan 2025 and 25 MB before that | Discord Help Center File Attachments FAQ (edited 2026-09-30); API changelog 2026-09-03 ("increased from 10 MiB to 20 MiB for users, bots, webhooks, and interaction responses") [H] |
| Nitro Basic | **50 MB** | Help Center Nitro page (edited 2026-09-29) [H] |
| Nitro | **"up to 1 GB"** per Help Center (edited 2026-09-29). Aug 2026 press said 500 MB, and the FAQ says Discord is "experimenting with various file upload size limits" | [M], treat 500 MB as the safe preset |
| Server Boost Level 2 / Level 3 | **50 MB / 100 MB** for all members (that server only). Experiment: "Larger File Uploads" perk raises it to **250 MB** for 5 Boosts | Server Boosting FAQ (edited 2026-09-14) [H] |
| Bots / webhooks / interaction replies | 20 MiB default, "may be higher … by the server's Boost Tier". Separate note: "maximum request size when sending a message is 25 MiB" | API docs `reference#uploading-files`, `resources/message` [H] |
| Mobile | size now checked *before* compression (was after) | Help Center FAQ [H] |

Discord lists "MP4 (Video encoded with H264, HEVC/H265, or AV1)" among common uploads [H]. Whether a recipient can *play* HEVC/AV1 still depends on their client and device. The "Discord-safe" preset should be H.264.

### 3.5 Target-size export ("fit under 20 MB")

Formula:

`video_kbps = (target_bytes × 8 / duration_s) / 1000 − audio_kbps`, with target_bytes = 0.95 × limit.

The 0.95 margin covers container overhead and rate-control overshoot. Using decimal MB keeps you safe whichever unit Discord checks. Results with 160 kbit/s audio:

| Limit | 15 s clip | 30 s | 60 s | 120 s |
|---|---|---|---|---|
| 20 MB (free) | 9,973 kb/s | 4,907 kb/s | 2,373 kb/s | 1,107 kb/s |
| 50 MB (Nitro Basic / Boost L2) | 25,173 | 12,507 | 6,173 | 3,007 |
| 100 MB (Boost L3) | 50,507 | 25,173 | 12,507 | 6,173 |

Suggested resolution ladder for H.264 when bits are tight: ≥ 8 Mb/s → 1080p60; 4–8 → 1080p30 or 720p60; 2–4 → 720p30/60; < 2 → 540p/480p. HEVC/AV1 buy roughly one rung but carry playback risk.

**How accurate is size targeting? (experiment, illustrative only)** [H]. Target 19 MiB, 30 s, 720p60 output:

| Source | x264 2-pass | x264 1-pass ABR | Apple VideoToolbox (HW) avg bitrate |
|---|---|---|---|
| Uniform-complexity clip | 0.991 × target | 0.995 × | 1.000 × |
| Half static, half extreme noise | **1.041 ×** (over) | **2.262 ×** (way over) | 1.001 × |

Takeaways:
- **One-pass ABR can blow the budget badly** on variable content.
- Two-pass software is close but can still exceed the target by about 4%.
- Hardware rate control *may* track the average well. VideoToolbox did here; this is not evidence for NVENC/AMF/QSV, so test on Windows.
- Always **measure the output and retry** at about 0.9× bitrate if it is over, and keep a 5% margin.

Encoder knobs:
- NVENC: `-rc vbr|cbr`, `-multipass qres|fullres` (two passes *within one encode*), `-rc-lookahead`.
- AMF: `-rc cbr|vbr_peak|qvbr|hqvbr|hqcbr`, `-preanalysis`.
- QSV: bitrate modes with `look_ahead`.

These are all verified in FFmpeg source/docs [H]. True file-level two-pass (`-pass 1/2`) exists only for software encoders (x264/x265/SVT-AV1). Use it when quality per byte matters more than speed.

### 3.6 How Discord embeds links: official docs, published 2026-10-06 [H]

Discord now documents unfurling at `discord.com/developers/docs/link-previews`:
- **One `GET`, no JavaScript executed.** Tags must be in server-rendered HTML.
- Response must be 2xx, `text/html`, at most 5 redirects, at most 50 MB. The **whole fetch "including measuring your images and video" must finish within 10 s.**
- Crawler user agent: `Mozilla/5.0 (compatible; Discordbot/2.0; +https://discordapp.com)`.
- **Video embeds:** point `og:video` (or `og:video:url`, `og:video:secure_url`, `twitter:player:stream`) **directly at the file** (.mp4/.webm/.mov). Set `og:type` to `video`/`video.*` *or* `twitter:card=player`. **`og:image` is required** and becomes the preview frame. **HTTPS required.**
- `og:video:type` accepts only `video/mp4`, `video/webm`, `video/quicktime`. Width/height are optional because Discord measures the file. If it can't, the video is dropped.
- Previews are cached about 30 min (append `?v=2` to bust). There is an **Embed Debugger** at `discord.com/developers/embeds`, and **Component Embeds** (custom layouts via a JSON payload ≤ 3,000 bytes).
- Bots cannot set an embed's `video` field (API docs) [H]. To show video, attach the file or post a link that unfurls.
- Raw `.mp4` URLs pasted in chat usually render as an inline player. This is common observation, not something the 2026 docs state [L].

Minimal share-page head (Blade):

```html
<meta property="og:site_name" content="Kairo">
<meta property="og:type" content="video.other">
<meta property="og:title" content="Ace clutch — Valorant">
<meta property="og:video" content="https://clips.example.com/c/01JABC.../share.mp4">
<meta property="og:video:type" content="video/mp4">
<meta property="og:video:width" content="1280"><meta property="og:video:height" content="720">
<meta property="og:image" content="https://clips.example.com/c/01JABC.../poster.jpg">
<meta property="og:image:width" content="1280"><meta property="og:image:height" content="720">
<meta name="twitter:card" content="player">
<meta name="theme-color" content="#7C3AED">
```

Because of the 10 s budget, the MP4 must be **faststart** and served from a fast CDN. Cloudflare caches `.mp4` by default up to 512 MB per file on Free/Pro/Business [H].

### 3.7 Optional, still no backend: "Post to my Discord server"

- Execute a webhook with `multipart/form-data` (`files[0]`, `payload_json`); the 20 MiB default applies [H].
- The user can paste a webhook URL. A smoother route is the OAuth2 **`webhook.incoming`** scope: the user picks a channel and the token response returns the webhook URL [H]. That needs a client secret for the code exchange, so it really belongs in the backend stage.
- Webhook URLs are secrets. Store them with DPAPI or Windows Credential Manager, never in plain settings.

---

## 4. Sharing with a backend (later stage)

### 4.1 2026 pricing snapshot (verified 2026-10-21)

| Service | Storage | Delivery | Encoding | Free tier / notes |
|---|---|---|---|---|
| **Cloudflare R2** (page updated 2026-10-01) | $0.015/GB-mo (IA $0.01) | **egress $0** | n/a | 10 GB-mo, 1M Class A ($4.50/M after), 10M Class B ($0.36/M after). `r2.dev` URLs are rate-limited and "for development purposes" only; production needs a custom domain [H]. Cloudflare terms allow video on the CDN when it's hosted on a Cloudflare service such as R2; video hosted elsewhere stays restricted [H] |
| **Cloudflare Stream** (updated 2026-10-15) | $5 per 1,000 min stored (prepaid) | $1 per 1,000 min delivered | free | Max file 30 GB. MP4 downloads (for `og:video`) bill as minutes delivered, with no extra storage. tus required for files > 200 MB [H] |
| **Bunny Stream** | $0.01/GB (default region; +$0.01 2nd region, +$0.005 each further). **Billed on everything encoding generates** (all renditions, MP4 fallback, previews) | CDN Standard $0.01/GB EU/NA, $0.03 Asia/Oceania, $0.045 S. America, $0.06 ME/Africa; **Volume $0.005/GB** (first 500 TB) | standard free; Premium $0.025–$0.15 per output minute | $1/month minimum. AI captions $0.10 per language-minute. DRM $99/mo + per-licence [H] |
| **Mux** (pay-as-you-go) | basic/plus: $0.0024/min-mo (≤ 720p), $0.003 (1080p). Auto cold storage −40% after 30 days unplayed, −60% after 90 | **first 100k min/mo free**, then $0.0008/min (≤ 720p), $0.001 (1080p) | basic quality free; plus $0.025–0.1/min | $20 monthly credit. Launch plan $20/mo ($100 credit), Scale $500/mo ($1,000 credit). Basic assets have a 1-month storage minimum [H] |
| **Backblaze B2** | **$6.95/TB-mo** (first 10 GB free) | free up to 3× average stored, then $0.01/GB; free egress to partners (Cloudflare, Bunny, Fastly…) | n/a | Cloudflare's terms restrict serving non-Cloudflare-hosted video through its CDN, so pair B2 with **Bunny or Fastly** for video [H] |
| **AWS S3 + CloudFront** | S3 Standard us-east-1 $0.023/GB-mo; PUT $0.005/1k; GET $0.0004/1k (price list 2026-10-15). S3 → CloudFront transfer free | CloudFront pay-as-you-go US/EU: first 1 TB free, next 9 TB $0.085/GB, next 40 TB $0.080, next 100 TB $0.060…; HTTPS $0.010/10k (US). **Flat-rate plans: Free $0 (100 GB, 1M req), Pro $15 (50 TB, 10M req, 50 GB S3 credit), Business $200 (50 TB, 125M req, 1 TB S3 credit), Premium $1,000 (50 TB, 500M; configurable up to 600 TB for $10,000)** | n/a | Flat-rate: "no overage charges", but allowances "are not hard limits". The first spike up to 3× is absorbed; sustained excess can lead to "traffic delivery … adjusted". One apex domain per plan; a WAF web ACL is mandatory [H] |
| **YouTube (unlisted)** | $0 | $0 | $0 | **100 `videos.insert`/day per Cloud project by default** (own bucket since 2026-06-01). More requires a compliance audit. Uploads land in **each user's own channel** via OAuth `youtube.upload`; max 256 GB [H] |

### 4.2 Worked monthly example

Assumptions:
- 1,000 users × 50 clips/month × 30 MB = **50,000 clips, 1.5 TB uploaded/month**.
- 30-day retention means about **1.5 TB stored** at steady state.
- 20 views per clip = **1M views**. Worst case, every view downloads the full file: **30 TB egress**.
- For minute-priced services: 30 MB ≈ **30 s** at about 8 Mbit/s, giving **25,000 min stored** and **500,000 min delivered**.
- Decimal GB/TB. Binary billing lowers some totals by about 7%. Calculated with `scripts06/costs.py`.

| Option | Monthly cost | Why |
|---|---|---|
| **Cloudflare R2** + custom domain (progressive MP4) | **≈ $22** | 1,490 GB × $0.015. ~100k Class A and ~2–4M Class B fit the free tier. Egress $0 |
| **S3 + CloudFront flat-rate Pro** | **≈ $49** | $15 plan + $34.50 S3 − 50 GB credit. 30 TB is inside the 50 TB allowance |
| B2 + Bunny CDN (Volume) | ≈ $160 | $10 storage + 30,000 GB × $0.005 |
| Bunny Stream | ≈ $165 (MP4 only, Volume tier) to ≈ $324 (renditions +60%, Standard EU/NA) | delivery dominates |
| B2 served directly | ≈ $265 | 25.5 TB over the 3× free egress × $0.01 |
| Mux (basic) | ≈ $380 (720p) / ≈ $475 (1080p), before the $20 credit | 400k billable delivery minutes |
| Cloudflare Stream | ≈ $625 | $125 storage + $500 delivery |
| S3 + CloudFront pay-as-you-go | ≈ $2,400 | 29 TB of paid egress |
| YouTube unlisted | $0, **but infeasible as the main host** | 50,000 uploads/month ≈ 1,667/day vs a default of 100/day per project; every user needs a channel and Google consent |

Sensitivity at **5× virality** (100 views per clip = 150 TB): R2 stays ≈ $22 (Class B still within about 10M). Bunny Volume ≈ $765. Mux 720p ≈ $1,828. Stream ≈ $2,625. CloudFront pay-as-you-go ≈ $10,000; flat-rate Premium at 200 TB is $3,500.
**Bottom line:** R2's zero egress makes it the clear default for progressive MP4 links. Stream, Mux and Bunny earn their price only when you need HLS adaptive bitrate, analytics, server-side transcoding, or a pre-built player.

**Not included above:** app hosting (§4.5), a domain, transactional email, error monitoring, moderation tooling.

### 4.3 Resumable uploads

- **A 30 MB clip doesn't need resumability.** One presigned `PUT` (R2/S3 single PUT up to 5 GiB [H]) from the desktop app is simplest.
- **For long recordings or flaky links, use S3 multipart:**
  - R2 rules [H]: parts of 5 MiB–5 GiB, **all parts except the last must be the same size**, ≤ 10,000 parts, objects ≤ 5 TiB. Incomplete uploads auto-abort after **7 days** by default.
  - Persist `UploadId` plus part ETags in SQLite to resume after a crash.
  - Whether R2 accepts presigned **UploadPart** URLs isn't covered on R2's multipart page. Verify it, or have the backend sign each part [L].
- **tus** (open protocol; `tusd` server, Uppy/tus-js-client) is natively accepted by:
  - **Cloudflare Stream** [H]: required above 200 MB; chunk ≥ 5,242,880 B, ≤ 200 MiB, multiple of 256 KiB; `Upload-Creator` header for per-user attribution.
  - **Bunny Stream** [H]: `https://video.bunnycdn.com/tusupload` with headers `AuthorizationSignature` = SHA256(library_id + api_key + expiry + video_id), `AuthorizationExpire`, `VideoId`, `LibraryId`, signed server-side. Resumable until expiry or about 48 h of inactivity.
- **Mux direct uploads** [H]: the server creates `POST /video/v1/uploads` and returns a URL. The client `PUT`s resumable chunks in multiples of 256 KB (UpChunk). Webhooks fire `video.asset.ready`.

### 4.4 Transcoding needs

- **Transcode on the client, which already has NVENC/AMF/QSV.** Upload a "share copy": H.264 High, ≤ 1080p60, about 6–8 Mbit/s, AAC 128–160 kbit/s, `+faststart`, closed GOP. Also upload the poster JPG/WebP and the sprite. Then **R2/S3 needs zero server-side transcoding.** H.264 matters for viewers: Firefox plays HEVC only on Windows with the extension, and Chromium plays HEVC only with hardware support [H].
- Server-side work is only **validation**: a queued job runs `ffprobe` to enforce codec, duration (≤ 3 min) and size limits. Optionally it computes perceptual hashes for moderation (§6) and deletes expired objects (an R2 lifecycle rule or a scheduled job).
- If you later want HLS/ABR, analytics or AI captions, buy them (Stream / Mux / Bunny) rather than running an ffmpeg farm.

### 4.5 Fit with a Laravel backend

- **Framework:** Laravel **13.x** is current (v13.38.0 released 2026-10-20) [H].
- **Uploads:** `Storage::temporaryUploadUrl($path, now()->plus(minutes: 5))` returns `['url' => …, 'headers' => …]` for direct-to-bucket uploads. It is supported on the `s3` and `local` drivers, and R2 works through the S3 driver with an endpoint [H, Laravel 12/13 docs].
- **Queues:** jobs for validation, hashing, expiry and webhook fan-out. **Horizon** needs Redis [M on Laravel Cloud's Valkey compatibility; Cloud also offers "managed queues"].
- **Laravel Cloud** (pricing page fetched 2026-10-21) [H]:
  - Plans: Starter **$5/mo** (includes $5 usage credit, scale-to-zero), Growth $20/mo, Business $200/mo, all plus usage. Flex compute from $6/mo for 512 MiB.
  - **Bandwidth overage $0.10/GB.** Never proxy video through the app; serve it from R2 or a CDN.
  - **Laravel Object Storage is "Powered by Cloudflare R2"**: bucket-level public/private, `Storage::temporaryUrl` for private files. It rejects `.html` uploads.
- **Forge** [H]: Hobby **$12/mo**, Growth $19, Business $39, plus a VPS (Laravel VPS or your own provider). Cheapest steady-state for a solo dev who doesn't mind servers.
- **Login with Socialite** [H]:
  - First-party providers: Google, GitHub, X, Facebook, LinkedIn, GitLab, Bitbucket, Slack.
  - **Discord and Steam** come from the community **SocialiteProviders** packages.
  - **Steam login is OpenID 2.0** (`https://steamcommunity.com/openid/`; the claimed ID contains the SteamID), not OAuth [H].
  - Desktop sign-in should follow **RFC 8252** (system browser + loopback or custom-scheme redirect) with **PKCE (RFC 7636)**. The desktop app gets a Sanctum/Passport token. Never embed client secrets in the app.

---

## 5. Direct posting APIs

| Target | How | Gate / limits (2026) | Cost |
|---|---|---|---|
| **YouTube** (Data API v3) | OAuth `youtube.upload`; resumable `videos.insert` | **Default 100 uploads/day per project** (own quota bucket since 2026-06-01; 1 unit/call). Other methods share 10,000 units/day. Quota increases require a compliance audit. History: ~1,600 units → ~100 units (2025-12-04) → separate bucket (2026-06-01) [H]. **Conflict:** the current `videos.insert` reference says "*Videos uploaded from unverified API projects are not restricted to private viewing mode*", while the 2020 revision-history entry restricted them to private until audit. Test with a fresh project [M] | free. Google OAuth app verification for this scope is likely needed for a public app [L] |
| **TikTok** (Content Posting API) | **Upload** (`video.upload`): video goes to the creator's inbox and they finish in the app. **Direct Post** (`video.publish`) | Direct Post by **unaudited** clients: "all content … restricted to private viewing", ≤ **5 users / 24 h**, accounts must be private, `SELF_ONLY`. An audit lifts it. All clients: ~**15 posts/day per creator** (shared across apps). Strict UX rules: show the creator nickname, no default privacy value, music-usage consent, **no watermarks/branding**, editable title. Media: ≤ 4 GB, 23–60 fps, H.264 recommended; FILE_UPLOAD chunks 5–64 MB (last ≤ 128 MB), 1–1000 chunks. Docs updated 2026-08-04 [H] | free |
| **X** | v2 chunked media upload: `POST /2/media/upload/initialize` → `/{id}/append` → `/{id}/finalize`, then `POST /2/tweets` with `media_id` | `tweet_video` typical max **20 min / 8 GB** (Premium 125 min / 16 GB); segments ≤ 5 MB recommended (8 MB server max). The duration cap is enforced when attaching to a post [H] | **Pay-per-use:** Post create **$0.015**, **Post with URL $0.20**, Media Metadata $0.005. No separate media-upload line item is listed [H, docs.x.com]. Legacy Basic/Pro tiers are for existing subscribers [M] |
| **Instagram Reels** | Instagram API with Instagram Login (`instagram_business_content_publish`) or with Facebook Login (`instagram_content_publish` + Page). Create a container `media_type=REELS` from a **public `video_url`** (or `upload_type=resumable` via rupload, Facebook Login only), poll `status_code`, then `media_publish` | **Professional accounts only.** Advanced Access needs **App Review + Business Verification** to serve accounts you don't own. **100 API-published posts / 24 h** per account (a carousel note says 50; check `content_publishing_limit`). Reels spec in §2.2 [H] | free |
| **Discord** | Webhook (user-pasted, or OAuth `webhook.incoming`) or a bot in the server | 20 MiB default per file (Boost tiers raise it); bots can't set embed `video`. For bigger clips post a share link that unfurls (§3.6) [H] | free |

---

## 6. Social features: moderation and legal obligations

### 6.1 EU Digital Services Act (Reg. 2022/2065) [H, article text]

Any Kairo feature that **stores clips for users** makes Kairo a *hosting service*. **Publishing them to the public** (public profiles or feeds) makes it an *online platform*. Duties:

| Applies to | Articles | Duty |
|---|---|---|
| All intermediary services | 11, 12 | Single electronic points of contact for authorities and for users |
| | 13 | **Legal representative in an EU member state** if Kairo has no EU establishment but offers services in the EU |
| | 14 | T&Cs must describe content-moderation policies and tools |
| | 15 | Annual transparency report. **Micro/small enterprises exempt** (15(2)) |
| Hosting services (incl. micro/small) | 16 | **Notice-and-action** form (electronic; URL, reason, notifier contact, good-faith statement) |
| | 17 | **Statement of reasons** to the affected user for removals, demotions, suspensions |
| | 18 | Notify police of suspected threats to life or safety |
| Online platforms | 19 | **Section 3 (Arts. 20–28) does not apply to micro or small enterprises**, except 24(3), and for 12 months after they outgrow that status. Section 3 covers: internal complaint handling (20), out-of-court disputes (21), trusted flaggers (22), misuse measures (23), Transparency Database submissions (24(5)), dark patterns (25), ads (26), recommender transparency (27), **protection of minors (28)** |

Micro or small enterprise means fewer than 50 staff and ≤ €10M turnover or balance sheet (Rec. 2003/361). The Commission's Art. 28 minors guidelines (final, 14 July 2025) recommend age verification for high-risk features. Commentary says they exclude micro/small enterprises [M]. They become relevant once Kairo grows past "small".

### 6.2 United States

- **DMCA §512 safe harbor:** designate an agent with the US Copyright Office online system. **$6 per designation, amendment or resubmission. Expires after 3 years** unless renewed, and lapsing risks losing safe harbor [H]. Also needed: a repeat-infringer policy and an expeditious takedown process. Game footage and music are frequent DMCA targets.
- **CSAM, 18 U.S.C. §2258A** [H]: providers must report apparent CSAM to **NCMEC's CyberTipline "as soon as reasonably possible" after obtaining actual knowledge**. The statute **does not require** monitoring or scanning (§2258A(f)). Reported content must be **preserved for 1 year** (2024 REPORT Act amendment, previously 90 days). Knowing and wilful failure to report: up to **$600k** first offence / **$850k** subsequent for providers with < 100M MAU.
- **Minors, COPPA:** the amended FTC rule took effect 2025-06-23 with **compliance required by 2026-04-22** [M, law-firm summaries of the Federal Register notice]. Simplest posture: **13+ only**, a neutral age screen, and no knowing collection from under-13s.

### 6.3 CSAM detection expectations outside the US

- **EU:** no general monitoring obligation. The temporary ePrivacy derogation (Reg. 2021/1232) *permits* voluntary scanning; it mostly matters for messaging services. Its status in 2026 is messy: due to expire 2026-04-03/04; one report says it lapsed and was restored by Parliament on 2026-07-09 until 2028 or the permanent regulation; the permanent CSA Regulation ("chat control") is still in negotiation [M].
- **UK Online Safety Act:** illegal-harms duties are enforceable since **17 March 2025**. Ofcom's codes expect **perceptual hash matching for CSAM** at higher-risk services and explicitly extended it to smaller **file-sharing/file-storage** services. Ofcom has investigated file-sharing hosts [M, law-firm and press summaries]. A clip host with public links sits close to that category.
- **Tools:**
  - Cloudflare's free **CSAM Scanning Tool** compares *images* served through Cloudflare's cache against NCMEC-supplied lists and blocks matches. Its docs don't mention video [H].
  - For video, sample frames and hash them (PhotoDNA/Thorn Safer-class tools). Pricing was not researched [L].
  - Cheap structural mitigations: **unlisted-by-default links**, 30-day expiry, login required to upload, upload caps (e.g. ≤ 3 min, ≤ 200 MB), rate limits, a report button on every share page, and hash-blocking re-uploads of removed content.

### 6.4 Minors and age

- 13+ globally is the practical floor (COPPA).
- In the EU, GDPR Art. 8 lets member states set the age of digital consent between 13 and 16 for consent-based processing. Avoid relying on consent for core features, or gate public social features at 16+ in the EU.
- If public feeds or DMs ever ship, DSA Art. 28 and the UK OSA children's-safety duties start to bite. Defer feeds and comments until there is traction and someone to do moderation.

---

## 7. Staged sharing roadmap with costs

| Stage | What ships | Infra cost / month | Legal and ops load |
|---|---|---|---|
| **A. Private beta (local-only)** | Library (SQLite + sprites), Quick/Precise trim, merge, vertical presets, **export presets: Discord 20 MB / 50 MB / 500 MB, Shorts/TikTok/Reels 9:16**, Copy-file-to-clipboard, drag-out, Show-in-folder, optional user-pasted Discord webhook | **$0** | none (no hosting) |
| **B. Free public release, still no backend** | + "Upload to YouTube" (user's own channel, OAuth, resumable) | **$0** (100 uploads/day/project default; audit if exceeded) | Google OAuth consent screen and verification; privacy policy |
| **C. Share links** | Laravel API + Socialite (Discord/Google/Steam) + **R2** (or S3 + CloudFront flat-rate) + server-rendered share page with OG video tags; unlisted links; 30-day retention | Storage and delivery **≈ $22 (R2) – $49 (S3+CF Pro)** at 1,000 users × 50 clips. App hosting **≈ $15–60** (Forge $12 + small VPS, or Laravel Cloud $5–20 + usage). **Total ≈ $40–110/mo** | **Hosting-service duties:** DSA Arts. 11–18 (contact points, notice-and-action, statements of reasons), **DMCA agent ($6/3 yrs)**, NCMEC reporting runbook, ToS + privacy policy, an EU legal representative if not EU-established |
| **D. Social (profiles, feeds, comments)** | Public profiles, follow and feed, comments, likes | delivery grows with views (R2 egress still $0); DB/compute up | **Online-platform duties** (most of DSA Section 3 waived while micro/small), moderation queue, **CSAM hash matching**, age gating (13+, EU 16+ for public features), UK OSA risk assessments. Cost is mostly people and tools |
| **E. Direct posting** | TikTok Upload→inbox (later Direct Post after audit), Instagram Reels (App Review + Business Verification), X | X: **$0.015 per post** ($0.20 if the post contains a URL); others free | platform audits and app reviews; UX compliance (TikTok rules) |

---

## 8. What changed in 2025–2026 (older guides get these wrong)

- Discord free uploads **20 MB** (Aug 2026), after 10 MB (late 2024 / Jan 2025) and 25 MB before that. Nitro now advertised "up to 1 GB". Official **link-preview docs** published 2026-10-06.
- YouTube upload quota: **100 uploads/day bucket** (2026-06-01), no longer "1,600 of 10,000 units".
- X API moved to **pay-per-use**: $0.015 per post, $0.20 for a post with a URL.
- CloudFront launched **flat-rate plans** ($15 Pro with a 50 TB allowance). Backblaze B2 is **$6.95/TB**.
- **Video.js 10** is stable (10.1.0, 2026-10-12). Electron's clipboard API was **rebuilt** (2026-07). FFmpeg 8.0 added a **Whisper** filter.

## 9. Open verification items (not fully verified)

1. WebView2-specific HEVC behaviour without the extension. Inferred from Edge docs; test on the Windows PC.
2. Discord desktop and WhatsApp desktop accepting CF_HDROP pastes. Inferred from Chromium source; test.
3. Raw `.mp4` URL unfurling in Discord (observed behaviour, undocumented).
4. Nitro limit, 500 MB vs "up to 1 GB" (Discord is experimenting).
5. YouTube unverified-project private lock (current doc vs 2020 policy). Google OAuth verification requirements for `youtube.upload`.
6. R2 presigned UploadPart support.
7. Status of the EU interim CSAM regulation after July 2026.
8. NVENC/AMF/QSV size accuracy for target-size exports. The experiment only covered x264 and VideoToolbox.

---

## Sources

Discord
- https://github.com/discord/discord-api-docs/blob/main/developers/reference.mdx (Uploading Files: "default limit is 20 MiB", commit 2026-09-04)
- https://github.com/discord/discord-api-docs/blob/main/developers/change-log.mdx (2026-09-03 "Default File Upload Limit Increase"; 2024-12-16 change to 10 MiB)
- https://discord.com/developers/docs/link-previews/overview and https://discord.com/developers/docs/link-previews/component-embeds (published 2026-10-06; source: github.com/discord/discord-api-docs/tree/main/developers/link-previews)
- https://github.com/discord/discord-api-docs/blob/main/developers/topics/oauth2.mdx (`webhook.incoming`)
- https://github.com/discord/discord-api-docs/blob/main/developers/resources/channel.mdx (25 MiB max request; embed `video` not settable)
- https://support.discord.com/hc/en-us/articles/25444343291031-File-Attachments-FAQ (edited 2026-09-30; fetched via Zendesk API)
- https://support.discord.com/hc/en-us/articles/115000435108-What-are-Nitro-Nitro-Basic (edited 2026-09-29)
- https://support.discord.com/hc/en-us/articles/360028038352-Server-Boosting-FAQ (edited 2026-09-14)
- https://www.dexerto.com/entertainment/discord-raises-free-upload-limit-to-20mb-after-lowering-it-two-years-ago-3398251/ (2026-08-13)
- https://x.com/discord/status/2087987317298360489 (announcement referenced by Dexerto)

Codecs / playback / Windows
- https://learn.microsoft.com/en-us/troubleshoot/microsoft-edge/development/video-playback-issues (ms.date 2026-03-31)
- https://github.com/StaZhu/enable-chromium-hevc-hardware-decoding (last commit 2026-08-17)
- https://apps.microsoft.com/detail/9nmzlz57r3t7 · https://apps.microsoft.com/detail/9n4wgh0z6vhq · https://apps.microsoft.com/detail/9mvzqvxjbq9v (prices via storeedgefd.dsx.mp.microsoft.com API, 2026-10-21)
- https://github.com/MicrosoftEdge/WebView2Feedback/issues/4285
- https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/working-with-local-content (updated 2026-09-02)
- https://learn.microsoft.com/en-us/windows/apps/develop/media-authoring-processing/media-compositions-and-editing (2026-08)
- https://github.com/mdn/browser-compat-data/blob/main/api/HTMLMediaElement.json (audioTracks flags)
- https://learn.microsoft.com/en-us/windows/win32/shell/clipboard (Shell clipboard formats, CF_HDROP)
- https://github.com/chromium/chromium/blob/main/ui/base/clipboard/clipboard_win.cc (ReadFilenamesInternal: CF_HDROP → FileNameW → FileNameA)
- https://github.com/chromium/chromium/blob/main/media/formats/mp4/track_run_iterator.cc (edit list handling)
- https://learn.microsoft.com/en-us/dotnet/api/system.windows.clipboard.setfiledroplist
- https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.management/set-clipboard?view=powershell-5.1
- https://learn.microsoft.com/pl-pl/archive/blogs/patricka/q-why-doesnt-drag-and-drop-work-when-my-application-is-running-elevated-a-mandatory-integrity-control-and-uipi
- https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nn-shobjidl_core-idatatransfermanagerinterop

Electron / Tauri / libraries
- https://github.com/electron/electron/blob/main/docs/api/clipboard.md (W3C-aligned rewrite, PR #51707, 2026-07-29)
- https://github.com/electron/electron/blob/main/docs/api/web-contents.md (`startDrag`) · https://github.com/electron/electron/blob/main/docs/api/shell.md (`showItemInFolder`, `trashItem`) · https://github.com/electron/electron/blob/main/docs/api/protocol.md (`stream: true`, `registerSource`)
- https://github.com/electron/electron/issues/9035
- https://github.com/CrossCopy/tauri-plugin-clipboard · https://github.com/crabnebula-dev/drag-rs · https://github.com/tauri-apps/plugins-workspace/blob/v2/plugins/opener/guest-js/index.ts · https://github.com/tauri-apps/tauri/issues/4133 · https://v2.tauri.app/security/asset-protocol/
- https://github.com/videojs/video.js/releases (10.1.0, 2026-10-12) · https://github.com/vidstack/player · https://github.com/muxinc/media-chrome · https://github.com/katspaugh/wavesurfer.js · https://github.com/Vanilagy/mediabunny · https://github.com/xzdarcy/react-timeline-editor · https://github.com/remotion-dev/remotion/blob/main/LICENSE.md · https://github.com/videolan/libvlcsharp · https://github.com/SuRGeoNix/Flyleaf · https://github.com/unosquare/ffmediaelement
- https://github.com/mifi/lossless-cut (README, docs/troubleshooting.md: smart cut experimental; keyframe interval advice)

FFmpeg / SQLite
- https://ffmpeg.org/ffmpeg-formats.html (mov muxer `use_editlist`, `+faststart`, fragmentation)
- https://ffmpeg.org/ffmpeg-codecs.html (QSV options) · https://github.com/FFmpeg/FFmpeg/blob/master/libavcodec/nvenc_h264.c (`multipass`, `rc`) · https://github.com/FFmpeg/FFmpeg/blob/master/libavcodec/amfenc_h264.c (`rc`, `preanalysis`)
- https://github.com/FFmpeg/FFmpeg/blob/release/8.0/Changelog (Whisper filter in 8.0)
- https://www.sqlite.org/intern-v-extern-blob.html · https://www.sqlite.org/wal.html

Storage / CDN / video platforms
- https://developers.cloudflare.com/r2/pricing/ (2026-10-01) · https://developers.cloudflare.com/r2/buckets/public-buckets/ (2026-09-25) · https://developers.cloudflare.com/r2/objects/multipart-objects/ (2026-07-29)
- https://www.cloudflare.com/service-specific-terms-application-services/ (2026-09-28) · https://blog.cloudflare.com/updated-tos/ (2023-05-16, modified 2026-07-15)
- https://developers.cloudflare.com/cache/concepts/default-cache-behavior/ (2026-09-14) · https://developers.cloudflare.com/cache/reference/csam-scanning/ (2026-05-01)
- https://developers.cloudflare.com/stream/pricing/ (2026-10-15) · https://developers.cloudflare.com/stream/viewing-videos/download-videos/ (2026-10-12) · https://developers.cloudflare.com/stream/uploading-videos/resumable-uploads/ (2026-04-21)
- https://bunny.net/pricing/stream/ · https://bunny.net/pricing/cdn/ · https://bunny.net/pricing/storage/ · https://bunny.net/docs/stream/pricing · https://bunny.net/docs/reference/tus-resumable-uploads
- https://www.mux.com/pricing · https://www.mux.com/docs/pricing/video · https://www.mux.com/docs/guides/upload-files-directly
- https://www.backblaze.com/cloud-storage/pricing
- https://aws.amazon.com/cloudfront/pricing/ · https://aws.amazon.com/cloudfront/pricing/pay-as-you-go/ · https://docs.aws.amazon.com/AmazonCloudFront/latest/DeveloperGuide/flat-rate-pricing-plan.html · https://aws.amazon.com/blogs/networking-and-content-delivery/amazon-cloudfront-flat-rate-pricing-plans-new-features-and-expanded-capabilities (2026-03-24)
- https://aws.amazon.com/s3/pricing/ · https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonS3/current/us-east-1/index.json (publicationDate 2026-10-15)

Direct posting
- https://developers.google.com/youtube/v3/docs/videos/insert (2026-10-15) · https://developers.google.com/youtube/v3/determine_quota_cost (2026-10-15) · https://developers.google.com/youtube/v3/revision_history · https://developers.google.com/youtube/v3/guides/quota_and_compliance_audits · https://developers.google.com/youtube/v3/guides/uploading_a_video
- https://developers.tiktok.com/doc/content-posting-api-get-started · https://developers.tiktok.com/doc/content-sharing-guidelines · https://developers.tiktok.com/doc/content-posting-api-get-started-upload-content · https://developers.tiktok.com/doc/content-posting-api-media-transfer-guide (all 2026-08-04)
- https://docs.x.com/x-api/getting-started/pricing · https://docs.x.com/x-api/media/quickstart/media-upload-chunked
- https://developers.facebook.com/docs/instagram-platform/content-publishing/ · https://developers.facebook.com/docs/instagram-platform/instagram-graph-api/reference/ig-user/media · https://developers.facebook.com/docs/development/release/access-verification/

Laravel
- https://github.com/laravel/docs/blob/13.x/filesystem.md (`temporaryUploadUrl`) · https://github.com/laravel/docs/blob/13.x/socialite.md · https://github.com/laravel/framework/releases (v13.38.0)
- https://laravel.com/cloud/pricing · https://laravel.com/cloud/docs/resources/object-storage · https://laravel.com/forge/pricing
- https://socialiteproviders.com/ · https://partner.steamgames.com/doc/features/auth (Steam OpenID 2.0)
- https://www.rfc-editor.org/rfc/rfc8252 · https://www.rfc-editor.org/rfc/rfc7636

Legal
- https://www.eu-digital-services-act.com/ (DSA article text mirror: Arts. 11–19, 20, 24, 28, 29, 33; official: https://eur-lex.europa.eu/eli/reg/2022/2065/oj)
- https://dsa-observatory.eu/2025/07/30/do-the-dsa-guidelines-on-protecting-minors-online-strike-the-right-balance/ · https://www.taylorwessing.com/en/insights-and-events/insights/2025/07/rd-european-commission-guidelines-on-protection-of-minors-under-the-digital-services-act
- https://www.copyright.gov/dmca-directory/ · https://www.copyright.gov/dmca-directory/faq.html
- https://www.law.cornell.edu/uscode/text/18/2258A
- https://www.edps.europa.eu/press-publications/press-news/press-releases/2026/extension-interim-rules-combat-child-sexual-abuse-online-must-address-shortcomings-and-prevent-indiscriminate-scanning · https://cyberinsider.com/eu-parliament-voted-to-restore-private-communications-scanning/ (2026-07-09)
- https://www.lewissilkin.com/en/insights/2024/12/16/ofcom-publishes-final-illegal-harms-codes-102jrep · https://iwf.org.uk/news-media/news/online-safety-act-uk-tech-companies-must-now-tackle-illegal-harms-including-child-sexual-abuse-imagery/
- https://jonesday.com/en/insights/2025/05/ftc-finalizes-amendments-to-coppa--rule · https://www.mondaq.com/unitedstates/privacy-protection/1626104/childrens-online-privacy-protection-act-amendments-effective-june-23-2025

Experiments (local, FFmpeg 8.0.1 on Apple M2): scripts in `prototypes/scripts/` (`mp4boxes.py` edit-list and faststart inspector, `costs.py` worked example).
