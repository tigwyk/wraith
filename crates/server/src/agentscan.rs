//! Passive agent detection by process (spec §5.2): which known harness, if any, runs in a pane's job.
use regex::Regex;
use windows::Wdk::System::Threading::{NtQueryInformationProcess, ProcessCommandLineInformation};
use windows::Win32::Foundation::{CloseHandle, HANDLE, UNICODE_STRING};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW};
use windows::core::PWSTR;

/// Compile `agent-detect` entries, skipping (and logging) bad regexes.
pub fn compile(detect: &[(String, String)]) -> Vec<(String, Regex)> {
    detect
        .iter()
        .filter_map(|(name, re)| match Regex::new(re) {
            Ok(r) => Some((name.clone(), r)),
            Err(e) => {
                tracing::warn!("agent-detect {name}: {e}");
                None
            }
        })
        .collect()
}

/// The first agent among `pids`.
pub fn scan(pids: &[u32], extra: &[(String, Regex)]) -> Option<String> {
    pids.iter().find_map(|&pid| {
        let (exe, cmdline) = process_info(pid)?;
        wraith_agent::match_process(&exe, &cmdline, extra)
    })
}

/// Image path and command line of `pid` (the command line is empty when it cannot be read).
fn process_info(pid: u32) -> Option<(String, String)> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let exe = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len)
            .ok()
            .map(|()| String::from_utf16_lossy(&buf[..len as usize]));
        let cmdline = command_line(h).unwrap_or_default();
        let _ = CloseHandle(h);
        Some((exe?, cmdline))
    }
}

unsafe fn command_line(h: HANDLE) -> Option<String> {
    // ProcessCommandLineInformation returns a UNICODE_STRING followed by its characters.
    let mut buf = vec![0u8; 8192];
    let mut needed = 0u32;
    unsafe {
        let query = |buf: &mut Vec<u8>, needed: &mut u32| NtQueryInformationProcess(h, ProcessCommandLineInformation, buf.as_mut_ptr().cast(), buf.len() as u32, needed).ok();
        // Command lines run to 32K characters: one that does not fit says how much room it needs.
        if query(&mut buf, &mut needed).is_err() {
            if needed as usize <= buf.len() {
                return None;
            }
            buf.resize(needed as usize, 0);
            query(&mut buf, &mut needed).ok()?;
        }
        let us = &*(buf.as_ptr() as *const UNICODE_STRING);
        let chars = std::slice::from_raw_parts(us.Buffer.0, usize::from(us.Length) / 2);
        Some(String::from_utf16_lossy(chars))
    }
}
