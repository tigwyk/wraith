//! Who may talk on the pipe: only the current Windows user (spec §3.4).
use std::io;

use windows::Win32::Foundation::{CloseHandle, HANDLE, HLOCAL, LocalFree};
use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows::Win32::Security::{
    GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TOKEN_USER, TokenElevation, TokenUser,
};
use windows::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::core::PWSTR;

/// String SID (`S-1-5-21-...`) of the user running this process.
pub fn current_user_sid() -> String {
    token_user_sid(unsafe { GetCurrentProcess() }).expect("current process token is always readable")
}

/// Owner-only DACL, protected from inheritance: full access for the current user, nobody else.
pub fn pipe_sddl() -> String {
    sddl_for(is_elevated())
}

/// String SID of the user that owns process `pid`.
pub fn process_owner_sid(pid: u32) -> io::Result<String> {
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid)?;
        let sid = token_user_sid(process);
        let _ = CloseHandle(process);
        sid
    }
}

fn token_user_sid(process: HANDLE) -> io::Result<String> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(process, TOKEN_QUERY, &mut token)?;
        let mut len = 0u32;
        let _ = GetTokenInformation(token, TokenUser, None, 0, &mut len);
        let mut buf = vec![0u8; len as usize];
        let r = GetTokenInformation(token, TokenUser, Some(buf.as_mut_ptr().cast()), len, &mut len);
        let _ = CloseHandle(token);
        r?;
        let user = &*(buf.as_ptr() as *const TOKEN_USER);
        let mut s = PWSTR::null();
        ConvertSidToStringSidW(user.User.Sid, &mut s)?;
        let out = s.to_string().map_err(io::Error::other);
        let _ = LocalFree(Some(HLOCAL(s.0.cast())));
        out
    }
}

/// Elevated ("Run as administrator") and normal Wraith keep separate pipes and servers.
pub fn pipe_name_for(sid: &str, elevated: bool) -> String {
    format!(r"\\.\pipe\wraith-{sid}{}", if elevated { "-admin" } else { "" })
}

/// Owner-only DACL; an elevated server's pipe also gets a High mandatory label so
/// medium-integrity processes of the same user cannot open it.
pub fn sddl_for(elevated: bool) -> String {
    let dacl = format!("D:P(A;;GA;;;{})", current_user_sid());
    if elevated { format!("{dacl}S:(ML;;NWNR;;;HI)") } else { dacl }
}

pub fn is_elevated() -> bool {
    token_elevated(unsafe { GetCurrentProcess() }).unwrap_or(false)
}

pub fn process_is_elevated(pid: u32) -> io::Result<bool> {
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid)?;
        let r = token_elevated(process);
        let _ = CloseHandle(process);
        r
    }
}

fn token_elevated(process: HANDLE) -> io::Result<bool> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(process, TOKEN_QUERY, &mut token)?;
        let mut e = TOKEN_ELEVATION::default();
        let mut len = 0u32;
        let r = GetTokenInformation(
            token,
            TokenElevation,
            Some((&raw mut e).cast()),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut len,
        );
        let _ = CloseHandle(token);
        r?;
        Ok(e.TokenIsElevated != 0)
    }
}
