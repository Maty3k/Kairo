//! Control's UI pipe: tokio 1.54 named pipe, user-SID DACL, remote clients rejected.
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions};
use windows::core::{HSTRING, PWSTR};
use windows::Win32::Foundation::{CloseHandle, LocalFree, ERROR_PIPE_BUSY, HANDLE, HLOCAL};
use windows::Win32::Security::Authorization::{ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1};
use windows::Win32::Security::{GetTokenInformation, TokenUser, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

pub fn current_user_sid() -> windows::core::Result<String> {
    unsafe {
        let mut tok = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut tok)?;
        let mut len = 0u32;
        let _ = GetTokenInformation(tok, TokenUser, None, 0, &mut len); // sizes the buffer
        let mut buf = vec![0u64; (len as usize).div_ceil(8)];           // u64: pointer-aligned
        let r = GetTokenInformation(tok, TokenUser, Some(buf.as_mut_ptr().cast()), len, &mut len);
        let _ = CloseHandle(tok);
        r?;
        let user = &*(buf.as_ptr() as *const TOKEN_USER);
        let mut s = PWSTR::null();
        ConvertSidToStringSidW(user.User.Sid, &mut s)?;
        let out = s.to_string();
        let _ = LocalFree(Some(HLOCAL(s.0.cast())));
        Ok(out?)
    }
}
pub fn pipe_name(sid: &str) -> String { format!(r"\\.\pipe\kairo-{sid}") }

pub struct PipeSecurity(PSECURITY_DESCRIPTOR);
unsafe impl Send for PipeSecurity {}
unsafe impl Sync for PipeSecurity {}
impl PipeSecurity {
    pub fn user_only(sid: &str) -> windows::core::Result<Self> {
        let sddl = HSTRING::from(format!("D:P(A;;GA;;;SY)(A;;GA;;;{sid})")); // default DACL lets Everyone read
        let mut psd = PSECURITY_DESCRIPTOR::default();
        unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(&sddl, SDDL_REVISION_1, &mut psd, None)? };
        Ok(Self(psd))
    }
}
impl Drop for PipeSecurity {
    fn drop(&mut self) { unsafe { let _ = LocalFree(Some(HLOCAL(self.0 .0))); } }
}

fn instance(name: &str, sec: &PipeSecurity, first: bool) -> std::io::Result<NamedPipeServer> {
    let mut sa = SECURITY_ATTRIBUTES { nLength: size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: sec.0 .0, bInheritHandle: false.into() };
    let mut o = ServerOptions::new();
    o.first_pipe_instance(first)   // default false: set it, so a squatter makes us fail (ERROR_ACCESS_DENIED)
        .reject_remote_clients(true) // already the default; keep explicit
        .max_instances(8);          // identical on every instance
    unsafe { o.create_with_security_attributes_raw(name, &mut sa as *mut _ as *mut core::ffi::c_void) }
}

pub async fn serve(name: String, sec: PipeSecurity, on_conn: impl Fn(NamedPipeServer) + Send + 'static) -> std::io::Result<()> {
    let mut server = instance(&name, &sec, true)?;
    loop {
        server.connect().await?;
        let connected = std::mem::replace(&mut server, instance(&name, &sec, false)?); // next instance BEFORE hand-off
        on_conn(connected);
    }
}

/// Client (Tauri shell, kairo-cli): tokio's ClientOptions default to SECURITY_IDENTIFICATION | SQOS_PRESENT.
pub async fn connect(name: &str) -> std::io::Result<NamedPipeClient> {
    loop {
        match ClientOptions::new().open(name) {
            Ok(c) => return Ok(c),
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY.0 as i32) => {}
            Err(e) => return Err(e), // NotFound => engine not running
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

pub fn codec() -> tokio_util::codec::LengthDelimitedCodec {
    tokio_util::codec::LengthDelimitedCodec::builder()
        .length_field_type::<u32>() // big-endian is the default: StreamJsonRpc LengthHeaderMessageHandler
        .max_frame_length(1 << 20)
        .new_codec()
}

pub fn sid_io() -> std::io::Result<String> { current_user_sid().map_err(std::io::Error::other) }
