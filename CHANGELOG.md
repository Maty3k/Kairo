# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Implementation plan (`docs/PLAN.md`) and the research behind it (`docs/research/`).
- `.gitignore` for IDE settings, local dogfood data and private notes.
- Licence: GPL-3.0-or-later (`LICENSE`).
- README, house rules for the coding agent (`CLAUDE.md`), security policy (`SECURITY.md`), this changelog and `.editorconfig`.
- ADR-0001: architecture and stack (`docs/adr/`).
- Milestone M0, setup and bridge: goal, gate, test card and progress (`docs/milestones/M0.md`).
- M0 development tooling: the Windows PC bootstrap script and `SETUP.md`; `kairo-devrun` (job runner and watch agent); the `kairo-hello` test window; `kairo-cli` (`ping`, `keys`); the Mac-side `tools/mac/pc` tool; CI.
- A CI job that runs the bootstrap end to end on a fresh Windows runner, three times (#2).
- The bootstrap switches off Visual Studio's optional usage data before installing the Build Tools, and its DONE screen repeats the yellow warnings (#4).
- Milestone M0 closed on 2026-10-23: all six gates passed (gate report in `docs/milestones/M0.md`).
- Milestone M1, test instruments: goal, gate and progress (`docs/milestones/M1.md`).
- `kairo-core` media maths: the clock that ties output ticks and audio samples to QPC time, the frame pacer, the `wgc-probe` trace format with its offline replay, and cadence statistics (#10).
- `kairo-core` replay ring and clip cut: keyframe-indexed packets evicted a whole GOP at a time, gapless audio tracks, pins for saves in flight, and cutting a clip that starts on a keyframe, never spans two encoder epochs and starts its audio one AAC packet early (#11).
- `kairo-core` audio aligner: places each capture source on the 48 kHz timeline, follows its clock drift with a PI loop steering the rubato 5.0.1 resampler, and splices over gaps, overlaps, pauses and reported losses with 2 ms fades, never behind the mixer (#12).

### Changed

- The plan pins Tauri 2.12.5 with `tauri-build` 2.7.4 and drops the updater and process plugins (#3).

### Fixed

- The bootstrap makes Administrators the owner of `C:\kairo` and everything in it except `logs` and `state` (#2).
- `SETUP.md` now matches what the bootstrap and the undo script do (#3).

[Unreleased]: https://github.com/Maty3k/Kairo/commits/main
