// SPDX-License-Identifier: GPL-3.0-or-later
//! `kairo-mock-engine`: a stand-in for Kairo's engine on the Mac (docs/PLAN.md §3.1; M1
//! contract), so the UI can be built and tested without the PC.
//!
//! It serves `kairo-engine.sock` in the per-user temp folder, or `--socket PATH`, with the
//! engine's framing: one JSON-RPC 2.0 object per frame behind a 4-byte big-endian length, at
//! most 1 MiB. Each connection starts with `engine.hello`; `engine.shutdown` stops the mock.
//! Exit codes: 0 after `engine.shutdown`, 1 when it can't serve, 2 for a bad command line.
//! On Windows it builds to a stub, since the PC runs the real engine.

#![forbid(unsafe_code)]

#[cfg(unix)]
mod protocol;
#[cfg(unix)]
mod server;

use std::process::ExitCode;

#[cfg(unix)]
fn main() -> ExitCode {
    use std::io::Write;

    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    let path = match parse_args(&args) {
        Ok(Command::Serve(path)) => path,
        Ok(Command::Version) => {
            let _ = writeln!(
                std::io::stdout(),
                concat!("kairo-mock-engine ", env!("CARGO_PKG_VERSION"))
            );
            return ExitCode::SUCCESS;
        }
        Ok(Command::Help) => {
            let _ = writeln!(std::io::stdout(), "{USAGE}");
            return ExitCode::SUCCESS;
        }
        Err(message) => {
            let _ = writeln!(std::io::stderr(), "{message}");
            return ExitCode::from(2);
        }
    };
    let server = match server::Server::bind(&path) {
        Ok(server) => server,
        Err(error) => {
            let _ = writeln!(std::io::stderr(), "kairo-mock-engine: {error}");
            return ExitCode::FAILURE;
        }
    };
    // The file name only: the house rules keep raw paths out of output.
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let _ = writeln!(std::io::stderr(), "kairo-mock-engine: listening on {name}");
    let engine = protocol::EngineInfo {
        pid: std::process::id(),
        started_at: kairo_core::time::now_utc(),
    };
    match server.run(&engine) {
        Ok(()) => {
            let _ = writeln!(std::io::stderr(), "kairo-mock-engine: shut down");
            ExitCode::SUCCESS
        }
        Err(error) => {
            let _ = writeln!(std::io::stderr(), "kairo-mock-engine: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(not(unix))]
fn main() -> ExitCode {
    eprintln!("kairo-mock-engine runs on the Mac; the PC runs the real engine");
    ExitCode::FAILURE
}

/// The command line's usage line.
#[cfg(unix)]
const USAGE: &str = "usage: kairo-mock-engine [--socket PATH] | --version | --help";

/// What the command line asks for.
#[cfg(unix)]
#[derive(Debug, PartialEq, Eq)]
enum Command {
    /// Serve on this socket.
    Serve(std::path::PathBuf),
    /// `--version`.
    Version,
    /// `--help`.
    Help,
}

/// Parses the arguments, without the program name.
#[cfg(unix)]
fn parse_args(args: &[std::ffi::OsString]) -> Result<Command, String> {
    use std::path::PathBuf;

    match args {
        [] => Ok(Command::Serve(server::default_socket_path())),
        [flag, path] if flag == "--socket" && !path.is_empty() => {
            Ok(Command::Serve(PathBuf::from(path)))
        }
        [flag] if flag == "--version" || flag == "-V" => Ok(Command::Version),
        [flag] if flag == "--help" || flag == "-h" => Ok(Command::Help),
        [flag] => match flag
            .to_str()
            .and_then(|flag| flag.strip_prefix("--socket="))
        {
            Some(path) if !path.is_empty() => Ok(Command::Serve(PathBuf::from(path))),
            _ => Err(format!("unexpected `{}`; {USAGE}", flag.to_string_lossy())),
        },
        _ => Err(format!("bad arguments; {USAGE}")),
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::ffi::OsString;
    use std::path::PathBuf;

    use super::*;

    fn args(line: &str) -> Vec<OsString> {
        line.split_whitespace().map(OsString::from).collect()
    }

    #[test]
    fn parses_the_command_line() {
        assert_eq!(
            parse_args(&args("")),
            Ok(Command::Serve(server::default_socket_path()))
        );
        assert!(server::default_socket_path().ends_with("kairo-engine.sock"));
        let at = |path: &str| Ok(Command::Serve(PathBuf::from(path)));
        assert_eq!(parse_args(&args("--socket /tmp/a.sock")), at("/tmp/a.sock"));
        assert_eq!(parse_args(&args("--socket=/tmp/b.sock")), at("/tmp/b.sock"));
        assert_eq!(parse_args(&args("--version")), Ok(Command::Version));
        assert_eq!(parse_args(&args("-h")), Ok(Command::Help));
        for bad in [
            "--socket",
            "--socket=",
            "--sockets /tmp/a",
            "serve",
            "--help --version",
        ] {
            assert!(parse_args(&args(bad)).is_err(), "{bad}");
        }
        // An empty value, as from an unset variable in `--socket "$SOCK"`.
        assert!(parse_args(&[OsString::from("--socket"), OsString::new()]).is_err());
    }
}
