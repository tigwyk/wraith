//! Per-user named pipe between the Wraith client, CLI and server (spec §3.4).
//! Handles are overlapped so one connection can read and write concurrently from two threads.
pub mod security;

use std::io::{self, Read, Write};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::sync::Arc;

use windows::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_BROKEN_PIPE, ERROR_FILE_NOT_FOUND, ERROR_IO_PENDING, ERROR_NO_DATA,
    ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED, ERROR_PIPE_NOT_CONNECTED, GENERIC_READ, GENERIC_WRITE, HANDLE, HLOCAL,
    LocalFree,
};
use windows::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, FILE_SHARE_NONE, OPEN_EXISTING,
    PIPE_ACCESS_DUPLEX, ReadFile, SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT, WriteFile,
};
use windows::Win32::System::IO::{GetOverlappedResult, OVERLAPPED};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, GetNamedPipeServerProcessId, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
    PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT, WaitNamedPipeW,
};
use windows::Win32::System::Threading::CreateEventW;
use windows::core::{HSTRING, PCWSTR};

/// `WRAITH_PIPE_NAME`, else `\\.\pipe\wraith-<user SID>`.
pub fn pipe_name() -> String {
    std::env::var("WRAITH_PIPE_NAME")
        .unwrap_or_else(|_| security::pipe_name_for(&security::current_user_sid(), security::is_elevated()))
}

const BUF: u32 = 64 * 1024;

fn win(e: windows::core::Error) -> io::Error {
    io::Error::from_raw_os_error(e.code().0 & 0xFFFF)
}

#[derive(Debug)]
pub struct PipeListener {
    name: HSTRING,
    next: OwnedHandle,
}

impl PipeListener {
    /// Claim `name`. Fails with `AddrInUse` if another process already serves it.
    pub fn bind(name: &str) -> io::Result<PipeListener> {
        let name = HSTRING::from(name);
        let next = create_instance(&name, true).map_err(|e| {
            if e.code() == ERROR_ACCESS_DENIED.to_hresult() {
                io::Error::new(io::ErrorKind::AddrInUse, "another Wraith server owns this pipe")
            } else {
                win(e)
            }
        })?;
        Ok(PipeListener { name, next })
    }

    /// Wait for the next client. A fresh instance is ready before this returns, so no connect is missed.
    pub fn accept(&mut self) -> io::Result<PipeConn> {
        let h = HANDLE(self.next.as_raw_handle());
        loop {
            let r = unsafe {
                let event = Event::new()?;
                let mut ov = OVERLAPPED { hEvent: event.0, ..Default::default() };
                match ConnectNamedPipe(h, Some(&mut ov)) {
                    Ok(()) => Ok(()),
                    Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => {
                        let mut n = 0;
                        GetOverlappedResult(h, &ov, &mut n, true)
                    }
                    Err(e) if e.code() == ERROR_PIPE_CONNECTED.to_hresult() => Ok(()),
                    Err(e) => Err(e),
                }
            };
            match r {
                Ok(()) => break,
                // A client connected and left before we accepted it: reset this instance, wait again.
                Err(e) if disconnected(&e) => unsafe {
                    let _ = DisconnectNamedPipe(h);
                },
                Err(e) => return Err(win(e)),
            }
        }
        let fresh = create_instance(&self.name, false).map_err(win)?;
        let conn = std::mem::replace(&mut self.next, fresh);
        Ok(PipeConn { h: Arc::new(conn) })
    }
}

fn create_instance(name: &HSTRING, first: bool) -> windows::core::Result<OwnedHandle> {
    unsafe {
        let sddl = HSTRING::from(security::pipe_sddl());
        let mut sd = PSECURITY_DESCRIPTOR::default();
        ConvertStringSecurityDescriptorToSecurityDescriptorW(&sddl, SDDL_REVISION_1, &mut sd, None)?;
        let sa = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: sd.0,
            bInheritHandle: false.into(),
        };
        let mut flags = PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED;
        if first {
            flags |= FILE_FLAG_FIRST_PIPE_INSTANCE;
        }
        let h = CreateNamedPipeW(
            name,
            flags,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_UNLIMITED_INSTANCES,
            BUF,
            BUF,
            0,
            Some(&sa),
        );
        let _ = LocalFree(Some(HLOCAL(sd.0)));
        if h.is_invalid() {
            return Err(windows::core::Error::from_thread());
        }
        Ok(OwnedHandle::from_raw_handle(h.0))
    }
}

/// Connect to a server. `NotFound` when none is listening; `PermissionDenied` if the process
/// serving the name is not owned by the current user (someone squatting the name).
pub fn connect(name: &str) -> io::Result<PipeConn> {
    let wname = HSTRING::from(name);
    let h = loop {
        let r = unsafe {
            CreateFileW(
                &wname,
                (GENERIC_READ | GENERIC_WRITE).0,
                FILE_SHARE_NONE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED | SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
                None,
            )
        };
        match r {
            Ok(h) => break unsafe { OwnedHandle::from_raw_handle(h.0) },
            Err(e) if e.code() == ERROR_FILE_NOT_FOUND.to_hresult() => {
                return Err(io::Error::new(io::ErrorKind::NotFound, "no Wraith server is listening"));
            }
            Err(e) if e.code() == ERROR_PIPE_BUSY.to_hresult() => {
                // All instances busy for a moment (server between accepts); wait for one.
                if !unsafe { WaitNamedPipeW(PCWSTR(wname.as_ptr()), 2000) }.as_bool() {
                    return Err(io::Error::new(io::ErrorKind::TimedOut, "Wraith server pipe stayed busy"));
                }
            }
            Err(e) => return Err(win(e)),
        }
    };
    let mut pid = 0u32;
    unsafe { GetNamedPipeServerProcessId(HANDLE(h.as_raw_handle()), &mut pid) }.map_err(win)?;
    if security::process_owner_sid(pid)? != security::current_user_sid() {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "pipe is served by another user's process"));
    }
    // Same user but different elevation would hand out admin shells to normal windows (or vice versa).
    if security::process_is_elevated(pid)? != security::is_elevated() {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "pipe is served at a different elevation"));
    }
    Ok(PipeConn { h: Arc::new(h) })
}

/// One connected pipe end. `split` gives a reader and a writer usable from different threads.
#[derive(Debug)]
pub struct PipeConn {
    h: Arc<OwnedHandle>,
}

impl PipeConn {
    /// Process id of the server end (used to stop a server that speaks another protocol version).
    pub fn server_pid(&self) -> io::Result<u32> {
        let mut pid = 0u32;
        unsafe { GetNamedPipeServerProcessId(HANDLE(self.h.as_raw_handle()), &mut pid) }.map_err(win)?;
        Ok(pid)
    }

    pub fn split(self) -> (PipeReader, PipeWriter) {
        (PipeReader { h: self.h.clone() }, PipeWriter { h: self.h })
    }
}

pub struct PipeReader {
    h: Arc<OwnedHandle>,
}

pub struct PipeWriter {
    h: Arc<OwnedHandle>,
}

struct Event(HANDLE);

impl Event {
    fn new() -> io::Result<Event> {
        unsafe { CreateEventW(None, true, false, PCWSTR::null()) }.map(Event).map_err(win)
    }
}

impl Drop for Event {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

fn disconnected(e: &windows::core::Error) -> bool {
    [ERROR_BROKEN_PIPE, ERROR_PIPE_NOT_CONNECTED, ERROR_NO_DATA].iter().any(|c| e.code() == c.to_hresult())
}

/// Issue one overlapped read or write and wait for it.
fn overlapped(h: &OwnedHandle, op: impl FnOnce(HANDLE, &mut OVERLAPPED) -> windows::core::Result<()>) -> windows::core::Result<u32> {
    let h = HANDLE(h.as_raw_handle());
    let event = Event::new().map_err(|e| windows::core::Error::from_hresult(windows::core::HRESULT(e.raw_os_error().unwrap_or(1))))?;
    let mut ov = OVERLAPPED { hEvent: event.0, ..Default::default() };
    if let Err(e) = op(h, &mut ov)
        && e.code() != ERROR_IO_PENDING.to_hresult()
    {
        return Err(e);
    }
    let mut n = 0u32;
    unsafe { GetOverlappedResult(h, &ov, &mut n, true)? };
    Ok(n)
}

impl Read for PipeReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let len = buf.len().min(u32::MAX as usize);
        match overlapped(&self.h, |h, ov| unsafe { ReadFile(h, Some(&mut buf[..len]), None, Some(ov)) }) {
            Ok(n) => Ok(n as usize),
            Err(e) if disconnected(&e) => Ok(0),
            Err(e) => Err(win(e)),
        }
    }
}

impl Write for PipeWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let len = buf.len().min(u32::MAX as usize);
        match overlapped(&self.h, |h, ov| unsafe { WriteFile(h, Some(&buf[..len]), None, Some(ov)) }) {
            Ok(n) => Ok(n as usize),
            Err(e) if disconnected(&e) => Err(io::Error::new(io::ErrorKind::BrokenPipe, "peer closed the pipe")),
            Err(e) => Err(win(e)),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl PipeWriter {
    /// Server side: end the connection now, even while our reader is blocked in a read. The peer
    /// sees EOF instead of a pipe that silently stopped talking.
    pub fn disconnect(&self) {
        unsafe {
            let _ = DisconnectNamedPipe(HANDLE(self.h.as_raw_handle()));
        }
    }
}
