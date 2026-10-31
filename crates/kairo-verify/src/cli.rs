// SPDX-License-Identifier: GPL-3.0-or-later
//! The command line (docs/research/16-verify-testbed/README.md §6.1, §6.6), behind `main`
//! so tests can run it whole.
//!
//! Exit codes for `check`: 0 pass (warnings allowed), 1 fail, 2 usage, 3 cannot evaluate (not
//! an MP4, no H.264 track, unreadable, a non-UTF-8 file name, or a required check this build
//! can't run), 4 internal (the report couldn't be written). Messages and reports name the file
//! only.

use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::expect::Expect;
use crate::profile::{Plan, Profile};
use crate::{Clip, boxes};

/// The usage text.
pub const USAGE: &str =
    "usage: kairo-verify check <clip.mp4> --profile PROFILE [--gate m3] [--expect FILE]
                    [--decoder none] [--json FILE|-] [--summary FILE|-]
       kairo-verify boxes <clip.mp4>
       kairo-verify --version | --help
profiles: test capture smoke engine share discord whatsapp reels original
This build checks structure only; the content checks and the linked and cli decoders come later.";

/// Where a report goes.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Output {
    Stdout,
    File(PathBuf),
}

/// What the command line asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Command {
    Check {
        clip: PathBuf,
        profile: Profile,
        gate_m3: bool,
        expect: Option<PathBuf>,
        json: Option<Output>,
        summary: Option<Output>,
    },
    Boxes(PathBuf),
    Version,
    Help,
}

fn output(value: &OsString) -> Output {
    if value == "-" {
        Output::Stdout
    } else {
        Output::File(PathBuf::from(value))
    }
}

/// A command-line word that is safe to echo in a message: a short token, never a path or a
/// piece of one, since output must not name folders (CLAUDE.md).
fn shown(word: &OsStr) -> String {
    match word.to_str() {
        Some(w)
            if w.len() <= 32
                && !w.contains(['/', '\\', ':'])
                && !w.chars().any(char::is_control) =>
        {
            format!("{w:?}")
        }
        _ => "(a word that may be a path, not shown)".to_owned(),
    }
}

/// Options README §6.1 documents for the content checks, which come with research 16's PRs 4
/// to 6.
const LATER_OPTIONS: [&str; 6] = [
    "--testbed-log",
    "--voice-log",
    "--cue",
    "--av-tight",
    "--voice-included",
    "--ffmpeg",
];

/// Parses the arguments, without the program name.
fn parse_args(args: &[OsString]) -> Result<Command, String> {
    let Some((verb, rest)) = args.split_first() else {
        return Err(USAGE.to_owned());
    };
    match verb.to_str() {
        Some("--version" | "-V") if rest.is_empty() => return Ok(Command::Version),
        Some("--help" | "-h") if rest.is_empty() => return Ok(Command::Help),
        Some("boxes") => {
            return match rest {
                [clip] if !clip.is_empty() => Ok(Command::Boxes(PathBuf::from(clip))),
                _ => Err(format!("boxes takes one clip\n{USAGE}")),
            };
        }
        Some("check") => {}
        _ => return Err(format!("unknown command {}\n{USAGE}", shown(verb))),
    }
    let mut clip = None;
    let mut profile = None;
    let mut gate_m3 = false;
    let mut expect = None;
    let mut json = None;
    let mut summary = None;
    let mut words = rest.iter();
    while let Some(word) = words.next() {
        let flag = word.to_str().unwrap_or_default();
        let mut value = |name: &str| {
            words
                .next()
                .filter(|v| !v.is_empty())
                .ok_or_else(|| format!("{name} needs a value\n{USAGE}"))
        };
        match flag {
            "--profile" => {
                let name = value("--profile")?;
                profile = Some(
                    name.to_str()
                        .and_then(Profile::parse)
                        .ok_or_else(|| format!("unknown profile {}\n{USAGE}", shown(name)))?,
                );
            }
            "--gate" => match value("--gate")?.to_str() {
                Some("m3") => gate_m3 = true,
                _ => return Err(format!("--gate takes m3\n{USAGE}")),
            },
            "--expect" => expect = Some(PathBuf::from(value("--expect")?)),
            "--decoder" => match value("--decoder")?.to_str() {
                Some("none") => {}
                Some("linked" | "cli") => {
                    return Err("this build has no decoder: only --decoder none".to_owned());
                }
                _ => return Err(format!("--decoder takes linked, cli or none\n{USAGE}")),
            },
            "--json" => json = Some(output(value("--json")?)),
            "--summary" => summary = Some(output(value("--summary")?)),
            _ if LATER_OPTIONS.contains(&flag) => {
                return Err(format!(
                    "{flag} comes with the content checks, which this build doesn't have yet"
                ));
            }
            _ if flag.starts_with('-') && flag.len() > 1 => {
                // Only the part before any `=`: what follows may be a path.
                let name = OsStr::new(flag.split('=').next().unwrap_or_default());
                return Err(format!("unknown option {}\n{USAGE}", shown(name)));
            }
            _ if clip.is_none() && !word.is_empty() => clip = Some(PathBuf::from(word)),
            _ => {
                return Err(format!(
                    "check takes one clip; an extra argument was given\n{USAGE}"
                ));
            }
        }
    }
    let clip = clip.ok_or_else(|| format!("check needs a clip\n{USAGE}"))?;
    let profile = profile.ok_or_else(|| format!("check needs --profile\n{USAGE}"))?;
    if json == Some(Output::Stdout) && summary == Some(Output::Stdout) {
        return Err("--json - and --summary - can't both use stdout".to_owned());
    }
    Ok(Command::Check {
        clip,
        profile,
        gate_m3,
        expect,
        json,
        summary,
    })
}

/// The file's name for messages and reports, or `None` if it isn't UTF-8.
fn file_name(path: &Path) -> Option<&str> {
    path.file_name()?.to_str()
}

fn say(err: &mut dyn Write, text: &str) {
    let _ = writeln!(err, "kairo-verify: {text}");
}

fn write_to(output: &Output, text: &str, out: &mut dyn Write) -> std::io::Result<()> {
    match output {
        Output::Stdout => out.write_all(text.as_bytes()),
        Output::File(path) => std::fs::write(path, text),
    }
}

fn check(command: Command, out: &mut dyn Write, err: &mut dyn Write) -> u8 {
    let Command::Check {
        clip,
        profile,
        gate_m3,
        expect,
        json,
        summary,
    } = command
    else {
        return 4;
    };
    let Some(name) = file_name(&clip).map(str::to_owned) else {
        say(err, "the clip's file name must be UTF-8");
        return 3;
    };
    let expect = match expect {
        None => None,
        Some(path) => match std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|t| Expect::parse(&t).map_err(|e| e.to_string()))
        {
            Ok(expect) => Some(expect),
            Err(error) => {
                say(
                    err,
                    &format!("{}: {error}", file_name(&path).unwrap_or("expect file")),
                );
                return 2;
            }
        },
    };
    let plan = match Plan::new(profile, gate_m3, expect) {
        Ok(plan) => plan,
        Err(error) => {
            say(err, &error.to_string());
            return 2;
        }
    };
    let report = match File::open(&clip) {
        Ok(mut file) => crate::check(&mut file, &name, &plan),
        Err(error) => {
            say(err, &format!("{name}: {error}"));
            return 3;
        }
    };
    // Without options the summary goes to stdout, unless the JSON does.
    let summary = summary.or(match json {
        Some(Output::Stdout) => None,
        _ => Some(Output::Stdout),
    });
    for (target, text) in [(json, report.to_json() + "\n"), (summary, report.to_text())] {
        if let Some(target) = target
            && let Err(error) = write_to(&target, &text, out)
        {
            say(err, &format!("can't write the report: {error}"));
            return 4;
        }
    }
    report.verdict.exit_code()
}

fn show_boxes(path: &Path, out: &mut dyn Write, err: &mut dyn Write) -> u8 {
    let Some(name) = file_name(path).map(str::to_owned) else {
        say(err, "the clip's file name must be UTF-8");
        return 3;
    };
    let clip = match File::open(path)
        .map_err(|e| e.to_string())
        .and_then(|mut f| Clip::read(&mut f).map_err(|e| e.to_string()))
    {
        Ok(clip) => clip,
        Err(error) => {
            say(err, &format!("{name}: {error}"));
            return 3;
        }
    };
    let text = format!(
        "{name}: {} bytes\n{}",
        clip.mp4.file_len,
        boxes::tree(&clip)
    );
    match out.write_all(text.as_bytes()) {
        Ok(()) => 0,
        Err(_) => 4,
    }
}

/// Runs the command line `args` (without the program name), writing to `out` and `err`, and
/// returns the exit code.
pub fn run(args: &[OsString], out: &mut dyn Write, err: &mut dyn Write) -> u8 {
    match parse_args(args) {
        Ok(Command::Version) => {
            let _ = writeln!(out, concat!("kairo-verify ", env!("CARGO_PKG_VERSION")));
            0
        }
        Ok(Command::Help) => {
            let _ = writeln!(out, "{USAGE}");
            0
        }
        Ok(Command::Boxes(path)) => show_boxes(&path, out, err),
        Ok(command) => check(command, out, err),
        Err(message) => {
            let _ = writeln!(err, "{message}");
            2
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(line: &str) -> Vec<OsString> {
        line.split_whitespace().map(OsString::from).collect()
    }

    #[test]
    fn parses_the_command_line() {
        assert_eq!(parse_args(&args("--version")), Ok(Command::Version));
        assert_eq!(parse_args(&args("-h")), Ok(Command::Help));
        assert_eq!(
            parse_args(&args("boxes a.mp4")),
            Ok(Command::Boxes(PathBuf::from("a.mp4")))
        );
        assert_eq!(
            parse_args(&args(
                "check a.mp4 --profile engine --gate m3 --json - --summary s.txt"
            )),
            Ok(Command::Check {
                clip: PathBuf::from("a.mp4"),
                profile: Profile::Engine,
                gate_m3: true,
                expect: None,
                json: Some(Output::Stdout),
                summary: Some(Output::File(PathBuf::from("s.txt"))),
            })
        );
        assert!(matches!(
            parse_args(&args(
                "check --profile test --expect e.json --decoder none c.mp4"
            )),
            Ok(Command::Check {
                expect: Some(_),
                ..
            })
        ));
        for bad in [
            "",
            "check",
            "check a.mp4",
            "check a.mp4 --profile clip",
            "check a.mp4 --profile engine --gate m4",
            "check a.mp4 --profile engine --decoder cli",
            "check a.mp4 --profile engine --json - --summary -",
            "check a.mp4 b.mp4 --profile engine",
            "check a.mp4 --profile engine --frobnicate",
            "check a.mp4 --profile",
            "boxes",
            "boxes a b",
            "synth --preset 1080p60",
            "--version extra",
        ] {
            assert!(parse_args(&args(bad)).is_err(), "{bad:?}");
        }
    }
}
