// SPDX-License-Identifier: GPL-3.0-or-later
//! `kairo-harness`'s program (docs/research/16-verify-testbed/README.md §8; the library holds
//! the work).
//!
//! `displays --out FILE` writes `displays.json` (every active display with its device path, GDI
//! name, model name, rectangle, refresh rate, HDR state and whether it mirrors another, and every
//! adapter with its LUID, DXGI index, PCI ids and model name) to FILE, through a temporary file
//! and a rename, for `pc harness displays` to fetch into `.dogfood/pc/`. The file names the
//! owner's monitors and GPU, so it stays local (CLAUDE.md), and standard output, which
//! `kairo-devrun` keeps as the job's `.out.log` and `pc` prints, gets only a count. `run` is gate
//! 2's capture, started by `pc harness capture` (`kairo_harness::run`); `corpus` and `probe`
//! come with research 16's PR 9b and 9c.
//!
//! Exit codes: `displays` 0 ok, 1 failed, 2 usage; `run` as `kairo_harness::phases::exit`. On
//! other systems both fail.

#![deny(unsafe_code)]

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use kairo_display::Displays;
use kairo_harness::cli::{self, Command, USAGE};

#[cfg(windows)]
fn displays() -> Result<Displays, String> {
    // Physical pixels, before anything asks for a rectangle.
    kairo_display::make_dpi_aware();
    let targets = kairo_display::targets().map_err(|e| e.to_string())?;
    let adapters = kairo_display::adapters().map_err(|e| e.to_string())?;
    Ok(Displays {
        kairo_displays: kairo_display::DISPLAYS_VERSION,
        targets,
        adapters,
    })
}

#[cfg(not(windows))]
fn displays() -> Result<Displays, String> {
    Err("kairo-harness runs on the Windows PC".to_owned())
}

/// Writes `doc` to `out` through a temporary file beside it and a rename, so a reader never
/// sees half a file, and returns the line for standard output: counts, no names.
fn write_displays(doc: &Displays, out: &Path) -> Result<String, String> {
    let json = serde_json::to_string_pretty(doc).map_err(|e| e.to_string())?;
    let mut temporary = out.as_os_str().to_owned();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);
    std::fs::write(&temporary, format!("{json}\n"))
        .and_then(|()| std::fs::rename(&temporary, out))
        .map_err(|e| {
            let _ = std::fs::remove_file(&temporary);
            format!("can't write displays.json: {e}")
        })?;
    let cloned = doc.targets.iter().filter(|t| t.cloned).count();
    Ok(format!(
        "displays.json: {} displays ({cloned} mirrored), {} adapters",
        doc.targets.len(),
        doc.adapters.len()
    ))
}

#[cfg(windows)]
fn run(request: &cli::Request) -> u8 {
    kairo_harness::run::run(request)
}

#[cfg(not(windows))]
fn run(_: &cli::Request) -> u8 {
    use std::io::Write;
    let _ = writeln!(
        std::io::stderr(),
        "kairo-harness: run runs on the Windows PC"
    );
    kairo_harness::phases::exit::FAILED
}

fn main() -> ExitCode {
    use std::io::Write;

    let args: Vec<String> = std::env::args().skip(1).collect();
    match cli::parse(&args) {
        Ok(Command::Run(request)) => ExitCode::from(run(&request)),
        Ok(Command::Displays { out }) => {
            match displays().and_then(|doc| write_displays(&doc, &out)) {
                Ok(line) => {
                    if writeln!(std::io::stdout(), "{line}").is_err() {
                        return ExitCode::FAILURE;
                    }
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    let _ = writeln!(std::io::stderr(), "kairo-harness: {error}");
                    ExitCode::FAILURE
                }
            }
        }
        Ok(Command::Version) => {
            let _ = writeln!(
                std::io::stdout(),
                concat!("kairo-harness ", env!("CARGO_PKG_VERSION"))
            );
            ExitCode::SUCCESS
        }
        Ok(Command::Help) => {
            let _ = writeln!(std::io::stdout(), "{USAGE}");
            ExitCode::SUCCESS
        }
        Err(message) => {
            let _ = writeln!(std::io::stderr(), "{message}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(line: &str) -> Vec<String> {
        line.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn parses_the_command_line() {
        assert_eq!(
            cli::parse(&args("displays --out d.json")),
            Ok(Command::Displays {
                out: PathBuf::from("d.json")
            })
        );
        assert_eq!(cli::parse(&args("--version")), Ok(Command::Version));
        assert_eq!(cli::parse(&args("-h")), Ok(Command::Help));
        for bad in [
            "",
            "displays",
            "display --out d.json",
            "displays --out",
            "displays d.json",
            "displays --out d.json extra",
            "capture",
            "--displays",
        ] {
            assert!(cli::parse(&args(bad)).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn displays_json_is_written_whole_and_only_counted_on_stdout() {
        let dir = std::env::temp_dir().join(format!("kairo-harness-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("harness-displays.json");
        std::fs::write(&out, "old").unwrap();
        let doc = Displays {
            kairo_displays: kairo_display::DISPLAYS_VERSION,
            targets: Vec::new(),
            adapters: Vec::new(),
        };
        let line = write_displays(&doc, &out).unwrap();
        assert_eq!(line, "displays.json: 0 displays (0 mirrored), 0 adapters");
        let written: Displays =
            serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
        assert_eq!(written, doc);
        assert!(!dir.join("harness-displays.json.tmp").exists());
        // A folder that doesn't exist fails cleanly.
        assert!(write_displays(&doc, &dir.join("missing").join("d.json")).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// On a Windows machine (CI or the PC) the real lists come back, and every display's
    /// adapter is among the adapters.
    #[cfg(windows)]
    #[test]
    fn displays_lists_targets_and_their_adapters() {
        let doc = displays().unwrap();
        assert_eq!(doc.kairo_displays, kairo_display::DISPLAYS_VERSION);
        // DXGI always has at least the Microsoft Basic Render Driver.
        assert!(!doc.adapters.is_empty());
        for target in &doc.targets {
            assert!(
                doc.adapters.iter().any(|a| a.luid == target.adapter_luid),
                "{target:?}"
            );
        }
    }
}
