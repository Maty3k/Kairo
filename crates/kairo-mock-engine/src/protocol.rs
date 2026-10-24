// SPDX-License-Identifier: GPL-3.0-or-later
//! The engine protocol as the mock speaks it (docs/research/11-engine-shell-rust.md §4.6):
//! `engine.hello` comes first and checks the client's major version, and `engine.shutdown`
//! stops the mock. `kairo-rpc` v0.1 (M3 task 1) replaces these hand-written shapes with the
//! contract's types and methods; the scenarios (PLAN §3.1, research 12 §7.1) stay in this crate,
//! not in the contract.

use kairo_core::rpc::{self, Incoming, Response, RpcError};
use serde_json::{Value, json};

/// The protocol's major version, after the plan's `kairo-rpc` v0.1 label; `kairo-rpc` sets the
/// real value.
pub const PROTOCOL_MAJOR: u64 = 0;
/// The protocol's minor version.
pub const PROTOCOL_MINOR: u64 = 1;
/// The handshake, which every connection starts with.
pub const METHOD_HELLO: &str = "engine.hello";
/// Stops the mock. It works before the handshake too: a client that speaks another major
/// version calls it, so that a matching engine can start.
pub const METHOD_SHUTDOWN: &str = "engine.shutdown";
/// A call came before a successful `engine.hello` (research 11's `NotReady`).
pub const NOT_READY: i64 = -32001;
/// The client speaks another major version (research 11's `ProtocolMismatch`).
pub const PROTOCOL_MISMATCH: i64 = -32002;

/// Who the mock is: the same for every connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineInfo {
    /// The process id.
    pub pid: u32,
    /// When the mock started, RFC 3339 UTC.
    pub started_at: String,
}

/// What the server does after a frame.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// Sends this response.
    Reply(Response),
    /// Sends nothing: the frame was a notification.
    Nothing,
    /// Sends this response, then shuts the mock down.
    Shutdown(Response),
}

/// One connection's state.
#[derive(Debug, Default)]
pub struct Session {
    /// Whether an `engine.hello` succeeded.
    greeted: bool,
}

impl Session {
    /// Answers one frame.
    pub fn handle(&mut self, engine: &EngineInfo, payload: &[u8]) -> Outcome {
        let (id, method, params) = match rpc::parse_incoming(payload) {
            Err(rejection) => return Outcome::Reply(rejection.into_response()),
            Ok(Incoming::Notification { .. }) => return Outcome::Nothing,
            Ok(Incoming::Call { id, method, params }) => (id, method, params),
        };
        let result = match method.as_str() {
            METHOD_HELLO => self.hello(engine, &params),
            METHOD_SHUTDOWN => {
                return match shutdown_params(&params) {
                    Ok(()) => Outcome::Shutdown(Response::success(id, json!({}))),
                    Err(error) => Outcome::Reply(Response::failure(id, error)),
                };
            }
            _ if !self.greeted => Err(RpcError::new(NOT_READY, "call engine.hello first")),
            _ => Err(RpcError::new(rpc::METHOD_NOT_FOUND, "Method not found")),
        };
        Outcome::Reply(match result {
            Ok(result) => Response::success(id, result),
            Err(error) => Response::failure(id, error),
        })
    }

    /// `engine.hello`: `{"protocol": {"major", "minor"}, ...}`, other fields ignored. Only the
    /// major version must match; minor versions only add.
    fn hello(&mut self, engine: &EngineInfo, params: &Value) -> Result<Value, RpcError> {
        let protocol = params.get("protocol");
        let version = |part: &str| protocol.and_then(|protocol| protocol.get(part)?.as_u64());
        let (Some(major), Some(_minor)) = (version("major"), version("minor")) else {
            return Err(RpcError::new(
                rpc::INVALID_PARAMS,
                "engine.hello takes {\"protocol\": {\"major\": n, \"minor\": n}}",
            ));
        };
        if major != PROTOCOL_MAJOR {
            return Err(RpcError {
                code: PROTOCOL_MISMATCH,
                message: "Protocol mismatch".to_owned(),
                data: Some(json!({
                    "engine": {"major": PROTOCOL_MAJOR, "minor": PROTOCOL_MINOR},
                })),
            });
        }
        self.greeted = true;
        Ok(json!({
            "protocol": {"major": PROTOCOL_MAJOR, "minor": PROTOCOL_MINOR},
            "engine": {
                "name": "kairo-mock-engine",
                "version": env!("CARGO_PKG_VERSION"),
                "pid": engine.pid,
                "startedAt": engine.started_at,
            },
            "features": [],
            "safeMode": null,
        }))
    }
}

/// `engine.shutdown` takes `{}` or `{"reason": "..."}`.
fn shutdown_params(params: &Value) -> Result<(), RpcError> {
    let fine = match params {
        Value::Null => true,
        Value::Object(object) => object.get("reason").is_none_or(Value::is_string),
        _ => false,
    };
    if fine {
        Ok(())
    } else {
        Err(RpcError::new(
            rpc::INVALID_PARAMS,
            "engine.shutdown takes {} or {\"reason\": \"...\"}",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine() -> EngineInfo {
        EngineInfo {
            pid: 42,
            started_at: "2026-10-24T12:00:00.000Z".to_owned(),
        }
    }

    /// A call; null params are left out, as JSON-RPC wants.
    fn frame(id: u64, method: &str, params: Value) -> Vec<u8> {
        let mut call = json!({"jsonrpc": "2.0", "id": id, "method": method});
        if !params.is_null() {
            call["params"] = params;
        }
        call.to_string().into_bytes()
    }

    fn hello(major: u64, minor: u64) -> Vec<u8> {
        frame(
            1,
            METHOD_HELLO,
            json!({
                "client": {"name": "kairo-ui", "version": "0.1.0"},
                "protocol": {"major": major, "minor": minor},
                "topics": ["engine.state"],
            }),
        )
    }

    /// The reply's result, or its error code.
    fn answer(outcome: Outcome) -> Result<Value, i64> {
        match outcome {
            Outcome::Reply(response) => response.into_result().map_err(|error| error.code),
            other => panic!("expected a reply, got {other:?}"),
        }
    }

    #[test]
    fn greets_a_client_that_speaks_the_same_major_version() {
        let mut session = Session::default();
        // A newer minor version only adds, so it is welcome.
        let result = answer(session.handle(&engine(), &hello(0, 7))).unwrap();
        assert_eq!(
            result,
            json!({
                "protocol": {"major": 0, "minor": 1},
                "engine": {
                    "name": "kairo-mock-engine",
                    "version": env!("CARGO_PKG_VERSION"),
                    "pid": 42,
                    "startedAt": "2026-10-24T12:00:00.000Z",
                },
                "features": [],
                "safeMode": null,
            })
        );
        // Asking again is fine.
        assert!(answer(session.handle(&engine(), &hello(0, 1))).is_ok());
    }

    #[test]
    fn another_major_version_is_a_protocol_mismatch() {
        let mut session = Session::default();
        let Outcome::Reply(response) = session.handle(&engine(), &hello(1, 0)) else {
            panic!("expected a reply");
        };
        let error = response.into_result().unwrap_err();
        assert_eq!(error.code, PROTOCOL_MISMATCH);
        assert_eq!(
            error.data,
            Some(json!({"engine": {"major": 0, "minor": 1}}))
        );
        // The session still waits for a handshake.
        let status = frame(2, "engine.status", Value::Null);
        assert_eq!(answer(session.handle(&engine(), &status)), Err(NOT_READY));
    }

    #[test]
    fn other_calls_wait_for_the_handshake() {
        let mut session = Session::default();
        let status = frame(2, "engine.status", json!({}));
        assert_eq!(answer(session.handle(&engine(), &status)), Err(NOT_READY));
        answer(session.handle(&engine(), &hello(0, 1))).unwrap();
        assert_eq!(
            answer(session.handle(&engine(), &status)),
            Err(rpc::METHOD_NOT_FOUND)
        );
    }

    #[test]
    fn a_hello_without_a_version_is_refused() {
        let bad = [
            Value::Null,
            json!({}),
            json!({"protocol": {"major": 0}}),
            json!({"protocol": {"major": -1, "minor": 1}}),
            json!({"protocol": {"major": "0", "minor": 1}}),
            json!({"protocol": {"major": 0.5, "minor": 1}}),
            json!([0, 1]),
        ];
        for params in bad {
            let mut session = Session::default();
            let reply = session.handle(&engine(), &frame(1, METHOD_HELLO, params.clone()));
            assert_eq!(answer(reply), Err(rpc::INVALID_PARAMS), "{params}");
        }
    }

    #[test]
    fn shutdown_works_with_or_without_a_handshake() {
        for greeted in [false, true] {
            let mut session = Session::default();
            if greeted {
                answer(session.handle(&engine(), &hello(0, 1))).unwrap();
            }
            for params in [Value::Null, json!({}), json!({"reason": "update"})] {
                let outcome = session.handle(&engine(), &frame(3, METHOD_SHUTDOWN, params));
                assert_eq!(
                    outcome,
                    Outcome::Shutdown(Response::success(json!(3), json!({})))
                );
            }
        }
        let mut session = Session::default();
        for params in [json!({"reason": 5}), json!(["update"])] {
            let reply = session.handle(&engine(), &frame(3, METHOD_SHUTDOWN, params));
            assert_eq!(answer(reply), Err(rpc::INVALID_PARAMS));
        }
    }

    #[test]
    fn notifications_get_nothing_even_a_shutdown() {
        let mut session = Session::default();
        let notification = |method: &str| {
            json!({"jsonrpc": "2.0", "method": method, "params": {}})
                .to_string()
                .into_bytes()
        };
        for method in [METHOD_SHUTDOWN, METHOD_HELLO, "engine.status"] {
            assert_eq!(
                session.handle(&engine(), &notification(method)),
                Outcome::Nothing
            );
        }
    }

    #[test]
    fn broken_frames_get_the_standard_errors() {
        let mut session = Session::default();
        let cases: [(&[u8], i64); 3] = [
            (b"not json", rpc::PARSE_ERROR),
            (
                br#"[{"jsonrpc":"2.0","id":1,"method":"engine.hello"}]"#,
                rpc::INVALID_REQUEST,
            ),
            (br#"{"id":1,"method":"engine.hello"}"#, rpc::INVALID_REQUEST),
        ];
        for (payload, code) in cases {
            assert_eq!(answer(session.handle(&engine(), payload)), Err(code));
        }
    }
}
