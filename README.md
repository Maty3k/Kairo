# Kairo

Catch the perfect moment. Kairo clips your best plays from games and anything else on your screen.

**Status:** planning is done, milestone M0 (setup and bridge) closed on 2026-10-23, and milestone M1 (test instruments) is in progress. There is nothing to download or install yet. "Kairo" is a working name; the public name is settled before launch.

## What it is

Kairo is a free instant-replay app for Windows. It quietly keeps the last moments of your game in memory: press **Alt+F8** and the last 30 seconds become a video that plays everywhere (Discord, WhatsApp, phones, video editors). Outside games, a tray switch ("screen mode") records the monitor you pick. Kairo also takes screenshots, sorts clips by game, trims them, and makes vertical (TikTok, Shorts, Reels) and Discord-sized copies you can paste straight into a chat. Share links that play inside Discord come with the public launch.

## Why

A clipper should be something you forget is running until you need it: it never costs you the game, never misses the play, and its files play everywhere. Built-in recorders are tied to one graphics-card brand, and many other clippers come with ads, watermarks or a required account. Kairo aims to be vendor-neutral (first-class on AMD and Intel graphics, not only NVIDIA) and to leave your clips yours.

## How it works

- A small always-on **engine** (Rust) sits in the tray, notices games and records them with your graphics card's hardware video encoder, through Windows' own capture features (Windows.Graphics.Capture, with DXGI Desktop Duplication as the fallback). It never puts code into a game, reads a game's memory or sends a game input.
- The **library app** (Tauri, React and TypeScript), where you browse, trim and share clips, opens only when you want it and closes completely while you play. It talks to the engine over a private local channel.
- Clips are ordinary MP4 files in `Videos\Kairo`, H.264 video with AAC audio by default.

The reasoning behind this design is in [ADR-0001](docs/adr/0001-architecture-and-stack.md).

## Promises

- No ads and no watermark.
- Your clips never train AI and are never sold.
- No tracking. On its own, Kairo only checks for updates (you can turn this off); it sends crash reports only if you say yes.
- No account needed. An optional account later unlocks share links.

## Platforms

Windows 11 and AMD Radeon RX 7000/9000 graphics come first. NVIDIA and Intel graphics join in the friends beta, and Windows 10 22H2 before the public launch. Versions for other operating systems are not planned for v1.

## Documentation

| Document | What's in it |
|---|---|
| [docs/PLAN.md](docs/PLAN.md) | The implementation plan: product, architecture, milestones and decisions |
| [SETUP.md](SETUP.md) | Setting up the Windows test PC (milestone M0) |
| [docs/milestones/M1.md](docs/milestones/M1.md) | The current milestone: goal, gate, your part and progress |
| [docs/milestones/M0.md](docs/milestones/M0.md) | Milestone M0, setup and bridge (closed 2026-10-23), with its gate report |
| [docs/adr/](docs/adr/) | Architecture decision records |
| [docs/research/](docs/research/) | The research behind the plan |
| [CLAUDE.md](CLAUDE.md) | House rules for the coding agent |
| [SECURITY.md](SECURITY.md) | How to report a security problem privately |
| [CHANGELOG.md](CHANGELOG.md) | Notable changes |

## How it's built

Kairo's code is written by Claude Code, Anthropic's AI coding agent, following the plan and the house rules in [CLAUDE.md](CLAUDE.md). The project owner play-tests every build and makes the product calls. Outside code contributions aren't accepted yet, until a licensing question about Microsoft Store distribution is settled ([PLAN §5](docs/PLAN.md#5-shipping-and-operations)). Please report security problems privately, as described in [SECURITY.md](SECURITY.md).

## Licence

Kairo is free software: you can redistribute it and/or modify it under the terms of the GNU General Public License as published by the Free Software Foundation, either version 3 of the License, or (at your option) any later version (SPDX: `GPL-3.0-or-later`). It is distributed in the hope that it will be useful, but WITHOUT ANY WARRANTY; see [LICENSE](LICENSE) for the full text.

FFmpeg will be used under the LGPL: Kairo will ship its own LGPL-only FFmpeg build as separate libraries you can replace, and each release will publish the matching FFmpeg source, changes and build configuration.
