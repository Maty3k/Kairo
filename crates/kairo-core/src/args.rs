// SPDX-License-Identifier: GPL-3.0-or-later
//! Command lines of `kairo-devrun` and `kairo-cli` (docs/PLAN.md §3.1, M0 contract).

use crate::hello;

/// `kairo-devrun` usage.
pub const DEVRUN_USAGE: &str =
    "usage: kairo-devrun run [--job PATH] | watch | ensure | engine | --version | --help";
/// `kairo-cli` usage.
pub const CLI_USAGE: &str =
    "usage: kairo-cli ping [--pipe NAME] | keys [--pipe NAME] | --version | --help";

/// What `kairo-devrun` was asked to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DevrunCommand {
    /// `run [--job PATH]`: run one job file (`None`: `C:\kairo\jobs\next.json`).
    Run {
        /// The job file.
        job: Option<String>,
    },
    /// `watch`: rewrite `watch.json` every second (started by `ensure`).
    Watch,
    /// `ensure`: start `watch` unless it is already running, then exit.
    Ensure,
    /// `engine`: start the engine; in M0 that is `kairo-hello.exe`.
    Engine,
    /// `--version`.
    Version,
    /// `--help`.
    Help,
}

/// What `kairo-cli` was asked to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliCommand {
    /// `ping [--pipe NAME]`: call `engine.hello`.
    Ping {
        /// Bare pipe name, e.g. `kairo-hello`.
        pipe: String,
    },
    /// `keys [--pipe NAME]`: call `hello.keys`.
    Keys {
        /// Bare pipe name, e.g. `kairo-hello`.
        pipe: String,
    },
    /// `--version`.
    Version,
    /// `--help`.
    Help,
}

/// Parses `kairo-devrun`'s arguments (without the program name).
pub fn parse_devrun(args: &[String]) -> Result<DevrunCommand, String> {
    let Some((command, rest)) = args.split_first() else {
        return Err(DEVRUN_USAGE.to_owned());
    };
    let simple = |parsed: DevrunCommand| {
        if rest.is_empty() {
            Ok(parsed)
        } else {
            Err(format!("`{command}` takes no arguments; {DEVRUN_USAGE}"))
        }
    };
    match command.as_str() {
        "run" => parse_run(rest),
        "watch" => simple(DevrunCommand::Watch),
        "ensure" => simple(DevrunCommand::Ensure),
        "engine" => simple(DevrunCommand::Engine),
        "--version" | "-V" | "version" => simple(DevrunCommand::Version),
        "--help" | "-h" | "help" => simple(DevrunCommand::Help),
        other => Err(format!("unknown command `{other}`; {DEVRUN_USAGE}")),
    }
}

fn parse_run(rest: &[String]) -> Result<DevrunCommand, String> {
    let job = match rest {
        [] => None,
        [flag, path] if flag == "--job" && !path.is_empty() => Some(path.clone()),
        [flag] => match flag.strip_prefix("--job=") {
            Some(path) if !path.is_empty() => Some(path.to_owned()),
            _ => return Err(format!("unexpected `{flag}`; {DEVRUN_USAGE}")),
        },
        _ => return Err(format!("bad arguments to `run`; {DEVRUN_USAGE}")),
    };
    Ok(DevrunCommand::Run { job })
}

/// Parses `kairo-cli`'s arguments (without the program name).
pub fn parse_cli(args: &[String]) -> Result<CliCommand, String> {
    let Some((command, rest)) = args.split_first() else {
        return Err(CLI_USAGE.to_owned());
    };
    match command.as_str() {
        "ping" => Ok(CliCommand::Ping {
            pipe: parse_pipe_option(rest)?,
        }),
        "keys" => Ok(CliCommand::Keys {
            pipe: parse_pipe_option(rest)?,
        }),
        "--version" | "-V" | "version" if rest.is_empty() => Ok(CliCommand::Version),
        "--help" | "-h" | "help" if rest.is_empty() => Ok(CliCommand::Help),
        other => Err(format!("unknown command `{other}`; {CLI_USAGE}")),
    }
}

fn parse_pipe_option(rest: &[String]) -> Result<String, String> {
    let value = match rest {
        [] => return Ok(hello::PIPE_NAME.to_owned()),
        [flag, value] if flag == "--pipe" => value.as_str(),
        [flag] => match flag.strip_prefix("--pipe=") {
            Some(value) => value,
            None => return Err(format!("unexpected `{flag}`; {CLI_USAGE}")),
        },
        _ => return Err(format!("bad arguments; {CLI_USAGE}")),
    };
    hello::parse_pipe_arg(value).ok_or_else(|| {
        format!(
            "bad pipe name `{value}`: use NAME or \\\\.\\pipe\\NAME with letters, digits, - _ ."
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(line: &str) -> Vec<String> {
        line.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn devrun_commands() {
        let run = |job: Option<&str>| DevrunCommand::Run {
            job: job.map(str::to_owned),
        };
        assert_eq!(parse_devrun(&args("run")), Ok(run(None)));
        assert_eq!(
            parse_devrun(&args(r"run --job C:\kairo\jobs\next.json")),
            Ok(run(Some(r"C:\kairo\jobs\next.json")))
        );
        assert_eq!(
            parse_devrun(&args("run --job=x.json")),
            Ok(run(Some("x.json")))
        );
        assert_eq!(parse_devrun(&args("watch")), Ok(DevrunCommand::Watch));
        assert_eq!(parse_devrun(&args("ensure")), Ok(DevrunCommand::Ensure));
        assert_eq!(parse_devrun(&args("engine")), Ok(DevrunCommand::Engine));
        assert_eq!(parse_devrun(&args("--version")), Ok(DevrunCommand::Version));
        assert_eq!(parse_devrun(&args("--help")), Ok(DevrunCommand::Help));
        for bad in [
            "",
            "run --job",
            "run --job=",
            "run x",
            "watch now",
            "start",
            "run --job a b",
        ] {
            assert!(parse_devrun(&args(bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn cli_commands() {
        let ping = |pipe: &str| CliCommand::Ping {
            pipe: pipe.to_owned(),
        };
        assert_eq!(parse_cli(&args("ping")), Ok(ping("kairo-hello")));
        assert_eq!(parse_cli(&args("ping --pipe kairo-x")), Ok(ping("kairo-x")));
        assert_eq!(
            parse_cli(&args(r"ping --pipe=\\.\pipe\kairo-y")),
            Ok(ping("kairo-y"))
        );
        assert_eq!(
            parse_cli(&args("keys")),
            Ok(CliCommand::Keys {
                pipe: "kairo-hello".to_owned()
            })
        );
        assert_eq!(parse_cli(&args("--version")), Ok(CliCommand::Version));
        for bad in [
            "",
            "ping --pipe",
            r"ping --pipe \\host\pipe\x",
            "ping x",
            "pong",
            "help me",
        ] {
            assert!(parse_cli(&args(bad)).is_err(), "{bad}");
        }
    }
}
