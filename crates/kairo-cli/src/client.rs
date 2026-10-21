// SPDX-License-Identifier: GPL-3.0-or-later
//! The pipe client. It opens the pipe at identification level only (the server may check who
//! is calling but never act as the caller), sends one framed JSON-RPC request and prints the
//! answer.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::os::windows::fs::OpenOptionsExt;
use std::process::ExitCode;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use kairo_core::args::{self, CliCommand};
use kairo_core::hello;
use kairo_core::rpc::{self, Response, RpcError};
use serde_json::{Map, Value};
use windows::Win32::Foundation::{ERROR_ACCESS_DENIED, ERROR_FILE_NOT_FOUND, ERROR_PIPE_BUSY};
use windows::Win32::Storage::FileSystem::SECURITY_IDENTIFICATION;

/// How long to retry while every pipe instance is busy.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// How long to wait for the whole call.
const CALL_TIMEOUT: Duration = Duration::from_secs(10);

/// Runs the command line.
pub fn main() -> ExitCode {
    let args: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    let (pipe, method) = match args::parse_cli(&args) {
        Ok(CliCommand::Ping { pipe }) => (pipe, hello::METHOD_ENGINE_HELLO),
        Ok(CliCommand::Keys { pipe }) => (pipe, hello::METHOD_HELLO_KEYS),
        Ok(CliCommand::Version) => {
            print_line(concat!("kairo-cli ", env!("CARGO_PKG_VERSION")));
            return ExitCode::SUCCESS;
        }
        Ok(CliCommand::Help) => {
            print_line(args::CLI_USAGE);
            return ExitCode::SUCCESS;
        }
        Err(message) => {
            let _ = writeln!(io::stderr(), "{message}");
            return ExitCode::from(2);
        }
    };
    let path = hello::pipe_path(&pipe);
    let began = Instant::now();
    let outcome = call_with_timeout(path.clone(), method);
    let mut report = Map::new();
    report.insert("ok".to_owned(), Value::from(matches!(outcome, Ok(Ok(_)))));
    report.insert("pipe".to_owned(), Value::from(path));
    report.insert("method".to_owned(), Value::from(method));
    let succeeded = match outcome {
        Ok(Ok(result)) => {
            let round_trip_ms = began.elapsed().as_secs_f64() * 1000.0;
            report.insert("round_trip_ms".to_owned(), Value::from(round_trip_ms));
            report.insert("result".to_owned(), result);
            true
        }
        Ok(Err(error)) => {
            report.insert("error".to_owned(), rpc_error_value(error));
            false
        }
        Err(message) => {
            report.insert(
                "error".to_owned(),
                rpc_error_value(RpcError::new(0, message)),
            );
            false
        }
    };
    let text = serde_json::to_string_pretty(&Value::Object(report))
        .unwrap_or_else(|error| format!("{{\"ok\": false, \"error\": \"{error}\"}}"));
    print_line(&text);
    if succeeded {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// Prints a line; a closed stdout (e.g. `| Select-Object -First 1`) is not an error.
fn print_line(text: &str) {
    let _ = writeln!(io::stdout(), "{text}");
}

fn rpc_error_value(error: RpcError) -> Value {
    serde_json::to_value(&error).unwrap_or(Value::Null)
}

type CallOutcome = Result<Result<Value, RpcError>, String>;

/// Makes the call on a worker thread, so a server that never answers cannot hang SSH.
fn call_with_timeout(path: String, method: &'static str) -> CallOutcome {
    let (sender, receiver) = mpsc::channel();
    let _ = thread::spawn(move || {
        let _ = sender.send(call(&path, method));
    });
    receiver
        .recv_timeout(CALL_TIMEOUT)
        .unwrap_or_else(|_| Err(format!("no answer within {} s", CALL_TIMEOUT.as_secs())))
}

fn call(path: &str, method: &str) -> CallOutcome {
    let mut pipe = open(path)?;
    let request = rpc::request(1, method, Value::Object(Map::new()));
    rpc::write_frame(&mut pipe, &request).map_err(|error| format!("cannot send: {error}"))?;
    let frame = rpc::read_frame(&mut pipe)
        .map_err(|error| format!("cannot read the answer: {error}"))?
        .ok_or_else(|| "the server closed the pipe without answering".to_owned())?;
    let response: Response = serde_json::from_slice(&frame)
        .map_err(|error| format!("the answer is not JSON-RPC: {error}"))?;
    Ok(response.into_result())
}

/// Opens the pipe, retrying for a while if all its instances are busy.
fn open(path: &str) -> Result<File, String> {
    let deadline = Instant::now() + CONNECT_TIMEOUT;
    loop {
        let opened = OpenOptions::new()
            .read(true)
            .write(true)
            .security_qos_flags(SECURITY_IDENTIFICATION.0)
            .open(path);
        match opened {
            Ok(file) => return Ok(file),
            Err(error) if is_os_error(&error, ERROR_PIPE_BUSY.0) && Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(50));
            }
            Err(error) => return Err(describe_open_error(&error)),
        }
    }
}

fn describe_open_error(error: &io::Error) -> String {
    if is_os_error(error, ERROR_FILE_NOT_FOUND.0) {
        "not running: there is no pipe with this name".to_owned()
    } else if is_os_error(error, ERROR_ACCESS_DENIED.0) {
        "access denied: the pipe belongs to another user".to_owned()
    } else if is_os_error(error, ERROR_PIPE_BUSY.0) {
        "busy: no free pipe instance".to_owned()
    } else {
        format!("cannot open the pipe: {error}")
    }
}

fn is_os_error(error: &io::Error, code: u32) -> bool {
    error
        .raw_os_error()
        .is_some_and(|raw| raw.cast_unsigned() == code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_pipe_reads_as_not_running() {
        let path = hello::pipe_path(&format!("kairo-cli-test-none-{}", std::process::id()));
        let error = open(&path).unwrap_err();
        assert!(error.starts_with("not running"), "{error}");
        let outcome = call_with_timeout(path, hello::METHOD_ENGINE_HELLO);
        assert!(matches!(outcome, Err(message) if message.starts_with("not running")));
    }
}
