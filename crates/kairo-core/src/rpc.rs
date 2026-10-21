// SPDX-License-Identifier: GPL-3.0-or-later
//! JSON-RPC 2.0 over a byte stream, framed with a 4-byte big-endian length (docs/PLAN.md §2.1).
//!
//! Each frame carries one JSON object of at most 1 MiB; batches are not supported. The framing
//! matches StreamJsonRpc's length-header handler and tokio-util's `LengthDelimitedCodec`
//! defaults, so the engine (Rust or C#) and the UI can share it later.

use std::io::{self, Read, Write};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Largest frame payload: 1 MiB.
pub const MAX_FRAME_LEN: usize = 1 << 20;

/// Invalid JSON.
pub const PARSE_ERROR: i64 = -32700;
/// Not a single JSON-RPC 2.0 request object (batches included).
pub const INVALID_REQUEST: i64 = -32600;
/// Unknown method.
pub const METHOD_NOT_FOUND: i64 = -32601;
/// Bad parameters.
pub const INVALID_PARAMS: i64 = -32602;
/// The server failed.
pub const INTERNAL_ERROR: i64 = -32603;

/// Writes one frame: a 4-byte big-endian length, then the payload, in a single write.
pub fn write_frame<W: Write + ?Sized>(writer: &mut W, payload: &[u8]) -> io::Result<()> {
    if payload.len() > MAX_FRAME_LEN {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "frame larger than 1 MiB",
        ));
    }
    let length = u32::try_from(payload.len()).map_err(io::Error::other)?;
    let mut frame = Vec::with_capacity(4 + payload.len());
    frame.extend_from_slice(&length.to_be_bytes());
    frame.extend_from_slice(payload);
    writer.write_all(&frame)?;
    writer.flush()
}

/// Reads one frame. `Ok(None)` means the peer closed the stream cleanly between frames.
pub fn read_frame<R: Read + ?Sized>(reader: &mut R) -> io::Result<Option<Vec<u8>>> {
    let mut header = [0u8; 4];
    let mut filled = 0;
    while filled < header.len() {
        match reader.read(&mut header[filled..]) {
            Ok(0) if filled == 0 => return Ok(None),
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "stream closed inside a frame header",
                ));
            }
            Ok(read) => filled += read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    let length = usize::try_from(u32::from_be_bytes(header)).map_err(io::Error::other)?;
    if length > MAX_FRAME_LEN {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame larger than 1 MiB",
        ));
    }
    let mut payload = vec![0u8; length];
    reader.read_exact(&mut payload)?;
    Ok(Some(payload))
}

/// A JSON-RPC 2.0 error object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RpcError {
    /// One of the JSON-RPC codes above, or an application code.
    pub code: i64,
    /// A short description.
    pub message: String,
    /// Optional details.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl RpcError {
    /// An error without details.
    pub fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }
}

/// A JSON-RPC 2.0 response. A server sets exactly one of `result` and `error`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    /// Always `"2.0"`.
    pub jsonrpc: String,
    /// The request's id, or null if it could not be read.
    pub id: Value,
    /// The result, on success.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// The error, on failure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

impl Response {
    /// A successful response.
    pub fn success(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_owned(),
            id,
            result: Some(result),
            error: None,
        }
    }

    /// An error response.
    pub fn failure(id: Value, error: RpcError) -> Self {
        Self {
            jsonrpc: "2.0".to_owned(),
            id,
            result: None,
            error: Some(error),
        }
    }

    /// The result (null if absent) or the error.
    pub fn into_result(self) -> Result<Value, RpcError> {
        match self.error {
            Some(error) => Err(error),
            None => Ok(self.result.unwrap_or(Value::Null)),
        }
    }

    /// The response as one JSON object, ready for [`write_frame`].
    pub fn to_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_else(|_| {
            br#"{"jsonrpc":"2.0","id":null,"error":{"code":-32603,"message":"Internal error"}}"#
                .to_vec()
        })
    }
}

/// A message received by a server.
#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    /// A request that expects a response.
    Call {
        /// The request id, echoed in the response.
        id: Value,
        /// The method name.
        method: String,
        /// An object, an array or null.
        params: Value,
    },
    /// A notification: no response is sent.
    Notification {
        /// The method name.
        method: String,
        /// An object, an array or null.
        params: Value,
    },
}

/// A frame that is not a valid request; the server answers with [`Rejection::into_response`].
#[derive(Debug, Clone, PartialEq)]
pub struct Rejection {
    /// The request's id, or null if it could not be read.
    pub id: Value,
    /// [`PARSE_ERROR`] or [`INVALID_REQUEST`].
    pub code: i64,
    /// The standard message for the code.
    pub message: &'static str,
}

impl Rejection {
    fn parse_error() -> Self {
        Self {
            id: Value::Null,
            code: PARSE_ERROR,
            message: "Parse error",
        }
    }

    fn invalid_request(id: Value) -> Self {
        Self {
            id,
            code: INVALID_REQUEST,
            message: "Invalid Request",
        }
    }

    /// The error response to send.
    pub fn into_response(self) -> Response {
        Response::failure(self.id, RpcError::new(self.code, self.message))
    }
}

/// Parses one frame.
pub fn parse_incoming(payload: &[u8]) -> Result<Incoming, Rejection> {
    let value: Value = serde_json::from_slice(payload).map_err(|_| Rejection::parse_error())?;
    let Value::Object(mut object) = value else {
        return Err(Rejection::invalid_request(Value::Null));
    };
    let id = object.remove("id");
    if id
        .as_ref()
        .is_some_and(|id| !(id.is_string() || id.is_number() || id.is_null()))
    {
        return Err(Rejection::invalid_request(Value::Null));
    }
    let reply_id = id.clone().unwrap_or(Value::Null);
    if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Err(Rejection::invalid_request(reply_id));
    }
    let Some(Value::String(method)) = object.remove("method") else {
        return Err(Rejection::invalid_request(reply_id));
    };
    let params = match object.remove("params") {
        None => Value::Null,
        Some(params @ (Value::Object(_) | Value::Array(_))) => params,
        Some(_) => return Err(Rejection::invalid_request(reply_id)),
    };
    Ok(match id {
        Some(id) => Incoming::Call { id, method, params },
        None => Incoming::Notification { method, params },
    })
}

/// A request as one JSON object, ready for [`write_frame`].
pub fn request(id: u64, method: &str, params: Value) -> Vec<u8> {
    let mut object = Map::new();
    object.insert("jsonrpc".to_owned(), Value::from("2.0"));
    object.insert("id".to_owned(), Value::from(id));
    object.insert("method".to_owned(), Value::from(method));
    object.insert("params".to_owned(), params);
    Value::Object(object).to_string().into_bytes()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn frames_round_trip_with_a_big_endian_length() {
        let mut stream = Vec::new();
        write_frame(&mut stream, b"{}").unwrap();
        write_frame(&mut stream, b"").unwrap();
        assert_eq!(&stream[..6], [0, 0, 0, 2, b'{', b'}']);
        let mut reader = stream.as_slice();
        assert_eq!(read_frame(&mut reader).unwrap(), Some(b"{}".to_vec()));
        assert_eq!(read_frame(&mut reader).unwrap(), Some(Vec::new()));
        assert_eq!(read_frame(&mut reader).unwrap(), None);
    }

    #[test]
    fn frames_over_one_mebibyte_are_refused() {
        let big = vec![b' '; MAX_FRAME_LEN + 1];
        assert!(write_frame(&mut Vec::new(), &big).is_err());
        assert!(write_frame(&mut Vec::new(), &big[..MAX_FRAME_LEN]).is_ok());
        let header = u32::try_from(MAX_FRAME_LEN + 1).unwrap().to_be_bytes();
        let error = read_frame(&mut header.as_slice()).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn truncated_frames_are_errors() {
        let error = read_frame(&mut [0u8, 0].as_slice()).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
        let error = read_frame(&mut [0u8, 0, 0, 5, b'{'].as_slice()).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn parses_calls_and_notifications() {
        let call = parse_incoming(br#"{"jsonrpc":"2.0","id":7,"method":"engine.hello"}"#).unwrap();
        assert_eq!(
            call,
            Incoming::Call {
                id: json!(7),
                method: "engine.hello".to_owned(),
                params: Value::Null,
            }
        );
        let note = parse_incoming(br#"{"jsonrpc":"2.0","method":"x","params":{"a":1}}"#).unwrap();
        assert_eq!(
            note,
            Incoming::Notification {
                method: "x".to_owned(),
                params: json!({"a": 1}),
            }
        );
        let string_id = parse_incoming(br#"{"jsonrpc":"2.0","id":"a","method":"x"}"#).unwrap();
        assert!(matches!(string_id, Incoming::Call { id, .. } if id == "a"));
    }

    #[test]
    fn rejects_invalid_requests_with_standard_codes() {
        let cases: [(&[u8], i64, Value); 7] = [
            (b"{", PARSE_ERROR, Value::Null),
            (
                br#"[{"jsonrpc":"2.0","id":1,"method":"x"}]"#,
                INVALID_REQUEST,
                Value::Null,
            ),
            (
                br#"{"jsonrpc":"1.0","id":1,"method":"x"}"#,
                INVALID_REQUEST,
                json!(1),
            ),
            (br#"{"jsonrpc":"2.0","id":2}"#, INVALID_REQUEST, json!(2)),
            (
                br#"{"jsonrpc":"2.0","id":3,"method":5}"#,
                INVALID_REQUEST,
                json!(3),
            ),
            (
                br#"{"jsonrpc":"2.0","id":4,"method":"x","params":1}"#,
                INVALID_REQUEST,
                json!(4),
            ),
            (
                br#"{"jsonrpc":"2.0","id":{},"method":"x"}"#,
                INVALID_REQUEST,
                Value::Null,
            ),
        ];
        for (payload, code, id) in cases {
            let rejection = parse_incoming(payload).unwrap_err();
            assert_eq!(rejection.id, id);
            assert_eq!(rejection.code, code);
            let response = rejection.into_response();
            assert_eq!(response.error.map(|error| error.code), Some(code));
        }
    }

    #[test]
    fn responses_serialize_per_json_rpc() {
        let ok = Response::success(json!(1), json!({"name": "kairo-hello"}));
        assert_eq!(
            String::from_utf8(ok.to_bytes()).unwrap(),
            r#"{"jsonrpc":"2.0","id":1,"result":{"name":"kairo-hello"}}"#
        );
        let failed = Response::failure(
            json!(2),
            RpcError::new(METHOD_NOT_FOUND, "Method not found"),
        );
        assert_eq!(
            String::from_utf8(failed.to_bytes()).unwrap(),
            r#"{"jsonrpc":"2.0","id":2,"error":{"code":-32601,"message":"Method not found"}}"#
        );
        let parsed: Response = serde_json::from_slice(&ok.to_bytes()).unwrap();
        assert_eq!(
            parsed.into_result().unwrap(),
            json!({"name": "kairo-hello"})
        );
        let parsed: Response = serde_json::from_slice(&failed.to_bytes()).unwrap();
        assert_eq!(parsed.into_result().unwrap_err().code, METHOD_NOT_FOUND);
    }

    #[test]
    fn requests_are_single_json_objects() {
        let bytes = request(9, "hello.keys", json!({"limit": 5}));
        let parsed = parse_incoming(&bytes).unwrap();
        assert_eq!(
            parsed,
            Incoming::Call {
                id: json!(9),
                method: "hello.keys".to_owned(),
                params: json!({"limit": 5}),
            }
        );
    }
}
