// SPDX-License-Identifier: GPL-3.0-or-later
//! The probe's named-pipe server: blocking Win32 pipes, one thread per client.
//!
//! Security (docs/PLAN.md §2.1): the DACL lets only SYSTEM and the current user open the pipe
//! (the default DACL would let Everyone read), `PIPE_REJECT_REMOTE_CLIENTS` refuses network
//! clients, and the first instance uses `FILE_FLAG_FIRST_PIPE_INSTANCE`, so the server fails
//! rather than join a pipe another process created first. The user SID (not the logon SID) is
//! deliberate: the same user's SSH session runs `kairo-cli`.
// TODO(verify) on the PC (M0): `kairo-cli ping` from the owner's elevated SSH session (session 0)
// opens this pipe in the interactive session (cookbook §12, item 17).

use std::fs::File;
use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use kairo_core::{hello, rpc};
use windows::Win32::Foundation::{ERROR_PIPE_CONNECTED, HANDLE, HLOCAL};
use windows::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
use windows::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};
use windows::core::{HSTRING, Owned};

/// Answers one request frame; `None` sends nothing back.
pub type Handler = dyn Fn(&[u8]) -> Option<Vec<u8>> + Send + Sync;

/// Initial pipe buffer sizes; frames up to 1 MiB still pass, written in pieces.
const BUFFER_SIZE: u32 = 64 * 1024;

/// A pipe instance waiting for a client, and what it takes to create the next one.
pub struct Listener {
    path: HSTRING,
    security: PipeSecurity,
    waiting: File,
}

impl Listener {
    /// Creates the first instance of `\\.\pipe\<name>`. Fails if any process already has a pipe
    /// by that name, for example a second `kairo-hello`.
    pub fn bind(name: &str) -> io::Result<Self> {
        let path = HSTRING::from(hello::pipe_path(name));
        let security = PipeSecurity::current_user_only()?;
        let waiting = create_instance(&path, &security, true)?;
        Ok(Self {
            path,
            security,
            waiting,
        })
    }

    /// Accepts clients forever, each on its own thread. Returns only when no further pipe
    /// instance can be created.
    pub fn serve(mut self, handler: Arc<Handler>) -> io::Result<()> {
        loop {
            if connect(&self.waiting).is_err() {
                // The client left before it was accepted: reset the instance and wait again.
                disconnect(&self.waiting);
                thread::sleep(Duration::from_millis(10));
                continue;
            }
            // Create the next instance before handing this one over, so the name is never free
            // for another process to take.
            let next = create_instance(&self.path, &self.security, false)?;
            let client = std::mem::replace(&mut self.waiting, next);
            let handler = Arc::clone(&handler);
            let _ = thread::spawn(move || serve_client(client, handler.as_ref()));
        }
    }
}

/// Answers frames until the client disconnects or sends something unreadable.
fn serve_client(mut pipe: File, handler: &Handler) {
    while let Ok(Some(frame)) = rpc::read_frame(&mut pipe) {
        let Some(reply) = handler(frame.as_slice()) else {
            continue;
        };
        if rpc::write_frame(&mut pipe, &reply).is_err() {
            return;
        }
    }
}

/// A security descriptor that allows only SYSTEM and the current user.
struct PipeSecurity {
    descriptor: Owned<HLOCAL>,
}

// SAFETY: the descriptor is a LocalAlloc block that this value alone owns and nothing mutates
// after creation; Windows may read it and LocalFree may free it from any thread. The listener
// moves it to the server thread once and never shares it.
unsafe impl Send for PipeSecurity {}

impl PipeSecurity {
    fn current_user_only() -> io::Result<Self> {
        let sddl = HSTRING::from(hello::pipe_sddl(&super::current_user_sid()?));
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        // SAFETY: `sddl` outlives the call; Windows allocates the descriptor with LocalAlloc.
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                &sddl,
                SDDL_REVISION_1,
                &mut descriptor,
                None,
            )
        }?;
        // SAFETY: the descriptor came from LocalAlloc and is freed only when this drops.
        let descriptor = unsafe { Owned::new(HLOCAL(descriptor.0)) };
        Ok(Self { descriptor })
    }

    fn attributes(&self) -> SECURITY_ATTRIBUTES {
        let descriptor: HLOCAL = *self.descriptor;
        SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor.0,
            bInheritHandle: false.into(),
        }
    }
}

fn create_instance(path: &HSTRING, security: &PipeSecurity, first: bool) -> io::Result<File> {
    let attributes = security.attributes();
    let open_mode = if first {
        PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE
    } else {
        PIPE_ACCESS_DUPLEX
    };
    // SAFETY: `path`, `attributes` and the descriptor it points to outlive the call.
    let handle = unsafe {
        CreateNamedPipeW(
            path,
            open_mode,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_UNLIMITED_INSTANCES,
            BUFFER_SIZE,
            BUFFER_SIZE,
            0,
            Some(&attributes),
        )
    };
    if handle.is_invalid() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: CreateNamedPipeW returned a new handle that only we own; the File closes it.
    Ok(unsafe { File::from_raw_handle(handle.0) })
}

/// Waits for a client on this instance.
fn connect(pipe: &File) -> io::Result<()> {
    // SAFETY: a synchronous pipe handle that `pipe` keeps open; no OVERLAPPED.
    let connected = unsafe { ConnectNamedPipe(HANDLE(pipe.as_raw_handle()), None) };
    match connected {
        Ok(()) => Ok(()),
        // The client connected between CreateNamedPipeW and ConnectNamedPipe: also a success.
        Err(error) if error.code() == ERROR_PIPE_CONNECTED.to_hresult() => Ok(()),
        Err(error) => Err(error.into()),
    }
}

/// Resets an instance so it can wait for a new client.
fn disconnect(pipe: &File) {
    // SAFETY: `pipe` keeps the handle open for the call.
    let _ = unsafe { DisconnectNamedPipe(HANDLE(pipe.as_raw_handle())) };
}

#[cfg(test)]
mod tests {
    use std::fs::OpenOptions;
    use std::os::windows::fs::OpenOptionsExt;

    use windows::Win32::Security::Authorization::{
        ConvertSecurityDescriptorToStringSecurityDescriptorW, GetSecurityInfo, SE_KERNEL_OBJECT,
    };
    use windows::Win32::Security::DACL_SECURITY_INFORMATION;
    use windows::Win32::Storage::FileSystem::SECURITY_IDENTIFICATION;
    use windows::core::PWSTR;

    use super::*;

    fn unique(tag: &str) -> String {
        format!("kairo-hello-test-{tag}-{}", std::process::id())
    }

    /// Opens the pipe the way `kairo-cli` does.
    fn open_client(name: &str) -> File {
        OpenOptions::new()
            .read(true)
            .write(true)
            .security_qos_flags(SECURITY_IDENTIFICATION.0)
            .open(hello::pipe_path(name))
            .unwrap()
    }

    #[test]
    fn answers_framed_requests_from_several_clients() {
        let name = unique("echo");
        let listener = Listener::bind(&name).unwrap();
        let handler: Arc<Handler> = Arc::new(|frame: &[u8]| Some(frame.to_vec()));
        let _server = thread::spawn(move || listener.serve(handler));
        let mut first = open_client(&name);
        let payloads: [&[u8]; 2] = [b"first", b"second"];
        for payload in payloads {
            rpc::write_frame(&mut first, payload).unwrap();
            assert_eq!(rpc::read_frame(&mut first).unwrap().unwrap(), payload);
        }
        let mut second = open_client(&name);
        rpc::write_frame(&mut second, b"third").unwrap();
        assert_eq!(rpc::read_frame(&mut second).unwrap().unwrap(), b"third");
    }

    #[test]
    fn a_second_server_cannot_take_the_name() {
        let name = unique("first");
        let _first = Listener::bind(&name).unwrap();
        assert!(Listener::bind(&name).is_err());
    }

    #[test]
    fn only_system_and_the_current_user_may_open_the_pipe() {
        let name = unique("dacl");
        let listener = Listener::bind(&name).unwrap();
        let actual = dacl_sddl(&listener.waiting).unwrap();
        // Protected, exactly SYSTEM and the user, with the pipe's full rights (FA: Windows maps
        // the GA of hello::pipe_sddl). The expected text goes through the same conversion,
        // because Windows writes some SIDs as aliases: a CI runner's user, the built-in
        // Administrator account, comes back as LA.
        let user = crate::platform::current_user_sid().unwrap();
        let expected = canonical_dacl(&format!("D:P(A;;FA;;;SY)(A;;FA;;;{user})")).unwrap();
        assert_eq!(actual, expected);
    }

    /// `sddl` the way Windows writes it back.
    fn canonical_dacl(sddl: &str) -> io::Result<String> {
        let sddl = HSTRING::from(sddl);
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        // SAFETY: `sddl` outlives the call; Windows allocates the descriptor with LocalAlloc.
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                &sddl,
                SDDL_REVISION_1,
                &mut descriptor,
                None,
            )
        }?;
        // SAFETY: the descriptor came from LocalAlloc and is freed only here.
        let _descriptor = unsafe { Owned::new(HLOCAL(descriptor.0)) };
        dacl_text(descriptor)
    }

    fn dacl_sddl(pipe: &File) -> io::Result<String> {
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        // SAFETY: `pipe` keeps the handle open; Windows allocates the descriptor with LocalAlloc.
        unsafe {
            GetSecurityInfo(
                HANDLE(pipe.as_raw_handle()),
                SE_KERNEL_OBJECT,
                DACL_SECURITY_INFORMATION,
                None,
                None,
                None,
                None,
                Some(&mut descriptor),
            )
        }
        .ok()?;
        // SAFETY: GetSecurityInfo allocated the descriptor with LocalAlloc; freed only here.
        let _descriptor = unsafe { Owned::new(HLOCAL(descriptor.0)) };
        dacl_text(descriptor)
    }

    /// The DACL of a valid descriptor in SDDL form.
    fn dacl_text(descriptor: PSECURITY_DESCRIPTOR) -> io::Result<String> {
        let mut text = PWSTR::null();
        // SAFETY: a valid descriptor; Windows allocates `text` with LocalAlloc.
        unsafe {
            ConvertSecurityDescriptorToStringSecurityDescriptorW(
                descriptor,
                SDDL_REVISION_1,
                DACL_SECURITY_INFORMATION,
                &mut text,
                None,
            )
        }?;
        // SAFETY: `text` came from LocalAlloc and is freed only here.
        let _text = unsafe { Owned::new(HLOCAL(text.0.cast())) };
        // SAFETY: a NUL-terminated UTF-16 string from Windows.
        unsafe { text.to_string() }.map_err(io::Error::other)
    }
}
