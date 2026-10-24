// SPDX-License-Identifier: GPL-3.0-or-later
//! The mock's Unix socket (M1 contract): `kairo-engine.sock` in the per-user temp folder, which
//! only its owner can use, framed like the engine's pipe. Each connection gets a thread, at most
//! [`MAX_CLIENTS`] at once, until a client calls `engine.shutdown`.

use std::fmt;
use std::fs::{self, File};
use std::io::{self, ErrorKind};
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Sender};
use std::thread;

use kairo_core::rpc::{self, Response, RpcError};
use serde_json::Value;

use crate::protocol::{EngineInfo, Outcome, Session};

/// The socket's file name.
pub const SOCKET_NAME: &str = "kairo-engine.sock";
/// The most connections served at once, as many as the engine pipe's instances.
pub const MAX_CLIENTS: usize = 8;

/// Where the mock listens unless told otherwise: the per-user temp folder (`$TMPDIR` on the
/// Mac), as the UI's engine client expects.
pub fn default_socket_path() -> PathBuf {
    std::env::temp_dir().join(SOCKET_NAME)
}

/// Why the mock can't serve.
#[derive(Debug)]
pub enum ServeError {
    /// Another engine already answers on the socket.
    AlreadyRunning,
    /// Something other than a socket is at the socket's path; the mock leaves it alone.
    NotASocket,
    /// The system refused.
    Io(io::Error),
}

impl fmt::Display for ServeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyRunning => write!(f, "another engine already answers on the socket"),
            Self::NotASocket => write!(f, "something other than a socket is at the socket's path"),
            Self::Io(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for ServeError {}

/// A bound socket, ready to serve.
#[derive(Debug)]
pub struct Server {
    listener: UnixListener,
    socket: Socket,
}

/// The socket file this mock made, known by device and inode as well as by path, so the mock
/// never removes a file that has since become another's.
#[derive(Debug, Clone)]
struct Socket {
    path: PathBuf,
    id: (u64, u64),
}

impl Socket {
    /// Removes the socket file, if it is still this mock's.
    fn release(&self) {
        let ours = fs::symlink_metadata(&self.path)
            .is_ok_and(|metadata| (metadata.dev(), metadata.ino()) == self.id);
        if ours {
            let _ = fs::remove_file(&self.path);
        }
    }
}

/// What every connection shares.
#[derive(Debug)]
struct Shared {
    engine: EngineInfo,
    socket: Socket,
    /// Tells [`Server::run`] to return: `Ok` after `engine.shutdown`.
    finish: Sender<Result<(), ServeError>>,
}

/// One of the [`MAX_CLIENTS`] places, given back when its connection ends.
struct Place(Arc<AtomicUsize>);

impl Drop for Place {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Server {
    /// Binds the socket at `path`. A socket left behind by a mock that didn't shut down is
    /// replaced; a live engine, or anything that isn't a socket, is left alone.
    pub fn bind(path: &Path) -> Result<Self, ServeError> {
        let _turn = take_turn(path);
        // Look before binding: on the Mac, bind(2) follows a symlink.
        if let Ok(metadata) = fs::symlink_metadata(path) {
            if !metadata.file_type().is_socket() {
                return Err(ServeError::NotASocket);
            }
            if UnixStream::connect(path).is_ok() {
                return Err(ServeError::AlreadyRunning);
            }
            fs::remove_file(path).map_err(ServeError::Io)?;
        }
        let listener = UnixListener::bind(path).map_err(ServeError::Io)?;
        // The per-user temp folder is private already; the socket is made so too.
        let made = fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .and_then(|()| fs::symlink_metadata(path));
        match made {
            Ok(metadata) => Ok(Self {
                listener,
                socket: Socket {
                    path: path.to_owned(),
                    id: (metadata.dev(), metadata.ino()),
                },
            }),
            Err(error) => {
                let _ = fs::remove_file(path);
                Err(ServeError::Io(error))
            }
        }
    }

    /// Serves until a client calls `engine.shutdown`. The socket file is gone before that call
    /// is answered, so a new mock can start at once; connections still open are left to end
    /// with the process.
    pub fn run(self, engine: &EngineInfo) -> Result<(), ServeError> {
        let Self { listener, socket } = self;
        let (finish, finished) = mpsc::channel();
        let shared = Arc::new(Shared {
            engine: engine.clone(),
            socket,
            finish,
        });
        // Accepting has a thread of its own, so a shutdown needn't wake it.
        let accepting = Arc::clone(&shared);
        let started = thread::Builder::new()
            .name("mock-accept".to_owned())
            .spawn(move || {
                let result = accept(&listener, &accepting);
                let _ = accepting.finish.send(result);
            });
        let result = match started {
            Ok(_) => finished.recv().unwrap_or(Ok(())),
            Err(error) => Err(ServeError::Io(error)),
        };
        shared.socket.release();
        result
    }
}

/// Holds a lock on the socket's folder, where the system allows one, until it is dropped.
/// Mocks starting together then take turns, so none mistakes another's new socket for a stale
/// one.
fn take_turn(path: &Path) -> Option<File> {
    let folder = path
        .parent()
        .filter(|folder| !folder.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let folder = File::open(folder).ok()?;
    folder.lock().ok()?;
    Some(folder)
}

/// Accepts connections until the listener fails.
fn accept(listener: &UnixListener, shared: &Arc<Shared>) -> Result<(), ServeError> {
    let clients = Arc::new(AtomicUsize::new(0));
    for stream in listener.incoming() {
        let stream = match stream {
            Ok(stream) => stream,
            Err(error)
                if matches!(
                    error.kind(),
                    ErrorKind::Interrupted | ErrorKind::ConnectionAborted
                ) =>
            {
                continue;
            }
            Err(error) => return Err(ServeError::Io(error)),
        };
        // Over the limit, the connection is closed at once.
        if clients.load(Ordering::SeqCst) >= MAX_CLIENTS {
            continue;
        }
        clients.fetch_add(1, Ordering::SeqCst);
        let place = Place(Arc::clone(&clients));
        let shared = Arc::clone(shared);
        // If no thread can start, the connection and its place are dropped.
        let _ = thread::Builder::new()
            .name("mock-session".to_owned())
            .spawn(move || {
                let _place = place;
                serve(stream, &shared);
            });
    }
    Ok(())
}

/// Answers one connection's frames until it closes or shuts the mock down.
fn serve(mut stream: UnixStream, shared: &Shared) {
    let mut session = Session::default();
    loop {
        let payload = match rpc::read_frame(&mut stream) {
            Ok(Some(payload)) => payload,
            Ok(None) => return,
            Err(error) => {
                // A frame over 1 MiB: say so, then hang up, since the stream can't be read on.
                if error.kind() == ErrorKind::InvalidData {
                    let error = RpcError::new(rpc::INVALID_REQUEST, "frame larger than 1 MiB");
                    let reply = Response::failure(Value::Null, error);
                    let _ = rpc::write_frame(&mut stream, &reply.to_bytes());
                }
                return;
            }
        };
        match session.handle(&shared.engine, &payload) {
            Outcome::Nothing => {}
            Outcome::Reply(reply) => {
                if rpc::write_frame(&mut stream, &reply.to_bytes()).is_err() {
                    return;
                }
            }
            Outcome::Shutdown(reply) => {
                shared.socket.release();
                let _ = rpc::write_frame(&mut stream, &reply.to_bytes());
                let _ = shared.finish.send(Ok(()));
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::sync::Barrier;
    use std::thread::JoinHandle;
    use std::time::{Duration, Instant};

    use serde_json::json;

    use super::*;
    use crate::protocol::{METHOD_HELLO, METHOD_SHUTDOWN, NOT_READY};

    /// A socket path of its own for each test, short enough for `sun_path`.
    fn socket_path() -> PathBuf {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, Ordering::SeqCst);
        std::env::temp_dir().join(format!("kairo-mock-{}-{n}.sock", std::process::id()))
    }

    fn serve_at(path: &Path) -> JoinHandle<Result<(), ServeError>> {
        let server = Server::bind(path).unwrap();
        let engine = EngineInfo {
            pid: 42,
            started_at: "2026-10-24T12:00:00.000Z".to_owned(),
        };
        thread::spawn(move || server.run(&engine))
    }

    fn start() -> (PathBuf, JoinHandle<Result<(), ServeError>>) {
        let path = socket_path();
        let server = serve_at(&path);
        (path, server)
    }

    fn connect(path: &Path) -> UnixStream {
        let stream = UnixStream::connect(path).unwrap();
        // macOS refuses socket options once the mock has closed the connection, as it does
        // over the limit; reads then end at once, so no timeout is needed there.
        if let Err(error) = stream.set_read_timeout(Some(Duration::from_secs(5))) {
            assert_eq!(error.kind(), ErrorKind::InvalidInput, "{error}");
        }
        stream
    }

    fn call(stream: &mut UnixStream, method: &str, params: Value) -> Result<Value, RpcError> {
        rpc::write_frame(stream, &rpc::request(1, method, params)).unwrap();
        let frame = rpc::read_frame(stream).unwrap().unwrap();
        serde_json::from_slice::<Response>(&frame)
            .unwrap()
            .into_result()
    }

    fn hello(stream: &mut UnixStream) -> Result<Value, RpcError> {
        call(
            stream,
            METHOD_HELLO,
            json!({"protocol": {"major": 0, "minor": 1}}),
        )
    }

    fn shut_down_through(stream: &mut UnixStream, server: JoinHandle<Result<(), ServeError>>) {
        let reply = call(stream, METHOD_SHUTDOWN, json!({"reason": "test"}));
        assert_eq!(reply, Ok(json!({})));
        server.join().unwrap().unwrap();
    }

    fn shut_down(path: &Path, server: JoinHandle<Result<(), ServeError>>) {
        shut_down_through(&mut connect(path), server);
        assert!(!path.exists());
    }

    #[test]
    fn serves_the_handshake_then_shuts_down() {
        let (path, server) = start();
        let mut stream = connect(&path);
        let result = hello(&mut stream).unwrap();
        assert_eq!(result["protocol"], json!({"major": 0, "minor": 1}));
        assert_eq!(result["engine"]["pid"], json!(42));
        shut_down(&path, server);
        // New connections find no socket; the first one is left to end on its own.
        assert!(UnixStream::connect(&path).is_err());
    }

    #[test]
    fn only_its_owner_can_use_the_socket() {
        let (path, server) = start();
        let mode = fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        shut_down(&path, server);
    }

    #[test]
    fn connections_have_their_own_handshake() {
        let (path, server) = start();
        let (mut first, mut second) = (connect(&path), connect(&path));
        hello(&mut first).unwrap();
        let early = call(&mut second, "engine.status", json!({})).unwrap_err();
        assert_eq!(early.code, NOT_READY);
        let late = call(&mut first, "engine.status", json!({})).unwrap_err();
        assert_eq!(late.code, rpc::METHOD_NOT_FOUND);
        shut_down(&path, server);
    }

    #[test]
    fn replaces_a_stale_socket_but_not_a_live_engine_or_another_file() {
        // A socket left behind by a mock that was killed.
        let path = socket_path();
        drop(UnixListener::bind(&path).unwrap());
        assert!(path.exists());
        let server = Server::bind(&path).unwrap();
        // A live engine is left alone.
        assert!(matches!(
            Server::bind(&path),
            Err(ServeError::AlreadyRunning)
        ));
        drop(server);
        fs::remove_file(&path).unwrap();
        // So is a file that isn't a socket.
        fs::write(&path, "keep me").unwrap();
        assert!(matches!(Server::bind(&path), Err(ServeError::NotASocket)));
        assert_eq!(fs::read_to_string(&path).unwrap(), "keep me");
        fs::remove_file(&path).unwrap();
    }

    #[test]
    fn leaves_a_symlink_alone_even_a_dangling_one() {
        // On the Mac, bind(2) would follow the link and make the socket at its target.
        let (path, target) = (socket_path(), socket_path());
        std::os::unix::fs::symlink(&target, &path).unwrap();
        let bound = Server::bind(&path);
        let target_made = fs::symlink_metadata(&target).is_ok();
        let _ = fs::remove_file(&target);
        assert!(matches!(bound, Err(ServeError::NotASocket)), "{bound:?}");
        assert!(!target_made);
        assert!(
            fs::symlink_metadata(&path)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        fs::remove_file(&path).unwrap();
    }

    #[test]
    fn mocks_starting_together_take_turns() {
        // Over a stale socket, exactly one binds, and the others find it answering.
        for _ in 0..20 {
            let path = socket_path();
            drop(UnixListener::bind(&path).unwrap());
            let barrier = Arc::new(Barrier::new(4));
            let starts: Vec<_> = (0..4)
                .map(|_| {
                    let (path, barrier) = (path.clone(), Arc::clone(&barrier));
                    thread::spawn(move || {
                        barrier.wait();
                        Server::bind(&path)
                    })
                })
                .collect();
            let results: Vec<_> = starts.into_iter().map(|t| t.join().unwrap()).collect();
            assert_eq!(results.iter().filter(|bound| bound.is_ok()).count(), 1);
            let mut refused = results.iter().filter_map(|bound| bound.as_ref().err());
            assert!(refused.all(|error| matches!(error, ServeError::AlreadyRunning)));
            drop(results);
            fs::remove_file(&path).unwrap();
        }
    }

    #[test]
    fn a_new_mock_can_start_as_soon_as_shutdown_is_answered() {
        let (path, server) = start();
        let mut stream = connect(&path);
        let reply = call(&mut stream, METHOD_SHUTDOWN, json!({}));
        assert_eq!(reply, Ok(json!({})));
        let next = Server::bind(&path);
        assert!(next.is_ok(), "{next:?}");
        server.join().unwrap().unwrap();
        drop(next);
        fs::remove_file(&path).unwrap();
    }

    #[test]
    fn the_connection_frees_the_socket_before_it_answers_a_shutdown() {
        // One connection's handler on its own, with no `run` that could remove the file later.
        let path = socket_path();
        let server = Server::bind(&path).unwrap();
        let (finish, finished) = mpsc::channel();
        let shared = Shared {
            engine: EngineInfo {
                pid: 42,
                started_at: "2026-10-24T12:00:00.000Z".to_owned(),
            },
            socket: server.socket.clone(),
            finish,
        };
        let (mut client, mock_end) = UnixStream::pair().unwrap();
        let shutdown = rpc::request(1, METHOD_SHUTDOWN, json!({}));
        rpc::write_frame(&mut client, &shutdown).unwrap();
        thread::spawn(move || serve(mock_end, &shared));
        let frame = rpc::read_frame(&mut client).unwrap().unwrap();
        assert!(!path.exists());
        let reply: Response = serde_json::from_slice(&frame).unwrap();
        assert_eq!(reply.into_result(), Ok(json!({})));
        assert!(matches!(finished.recv(), Ok(Ok(()))));
    }

    #[test]
    fn shuts_down_even_when_its_socket_file_is_gone() {
        let (path, server) = start();
        let mut stream = connect(&path);
        fs::remove_file(&path).unwrap();
        shut_down_through(&mut stream, server);
    }

    #[test]
    fn never_removes_a_socket_that_is_no_longer_its_own() {
        let (path, first) = start();
        let mut stream = connect(&path);
        // Another mock takes over the path while the first still serves a client.
        fs::remove_file(&path).unwrap();
        let second = serve_at(&path);
        shut_down_through(&mut stream, first);
        assert!(path.exists());
        let mut fresh = connect(&path);
        hello(&mut fresh).unwrap();
        shut_down(&path, second);
    }

    #[test]
    fn a_frame_over_one_mebibyte_gets_an_error_and_a_hang_up() {
        let (path, server) = start();
        let mut stream = connect(&path);
        let length = u32::try_from(rpc::MAX_FRAME_LEN + 1).unwrap();
        stream.write_all(&length.to_be_bytes()).unwrap();
        let frame = rpc::read_frame(&mut stream).unwrap().unwrap();
        let reply: Response = serde_json::from_slice(&frame).unwrap();
        assert_eq!(reply.id, Value::Null);
        assert_eq!(reply.into_result().unwrap_err().code, rpc::INVALID_REQUEST);
        assert_eq!(rpc::read_frame(&mut stream).unwrap(), None);
        shut_down(&path, server);
    }

    #[test]
    fn serves_at_most_eight_connections_at_once() {
        let (path, server) = start();
        let mut open: Vec<UnixStream> = (0..MAX_CLIENTS).map(|_| connect(&path)).collect();
        for stream in &mut open {
            hello(stream).unwrap();
        }
        // The ninth is closed without an answer.
        let mut ninth = connect(&path);
        let _ = rpc::write_frame(&mut ninth, &rpc::request(1, METHOD_HELLO, json!({})));
        assert!(!matches!(rpc::read_frame(&mut ninth), Ok(Some(_))));
        // Once one closes, its place is free again.
        drop(open.pop());
        let began = Instant::now();
        let mut next = loop {
            let mut next = connect(&path);
            let hello = rpc::request(
                1,
                METHOD_HELLO,
                json!({"protocol": {"major": 0, "minor": 1}}),
            );
            if rpc::write_frame(&mut next, &hello).is_ok()
                && matches!(rpc::read_frame(&mut next), Ok(Some(_)))
            {
                break next;
            }
            assert!(
                began.elapsed() < Duration::from_secs(5),
                "no place came free"
            );
            thread::sleep(Duration::from_millis(20));
        };
        // Every place is taken again, so shut down through one of them.
        shut_down_through(&mut next, server);
        assert!(!path.exists());
    }
}
