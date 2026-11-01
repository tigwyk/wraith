//! ConPTY child process inside a kill-on-close Job Object.
use std::ffi::c_void;
use std::fs::File;
use std::io::{self, Write};
use std::os::windows::io::{FromRawHandle, OwnedHandle};
use std::path::Path;

use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows::Win32::System::Console::{
    COORD, ClosePseudoConsole, CreatePseudoConsole, HPCON, ResizePseudoConsole,
};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectBasicProcessIdList,
    JobObjectExtendedLimitInformation, QueryInformationJobObject, SetInformationJobObject,
};
use windows::Win32::System::Pipes::CreatePipe;
use windows::Win32::System::Threading::{
    CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, CreateProcessW, DeleteProcThreadAttributeList,
    EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess, InitializeProcThreadAttributeList,
    LPPROC_THREAD_ATTRIBUTE_LIST, OpenProcess, PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE,
    PROCESS_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, ResumeThread, STARTF_USESTDHANDLES,
    STARTUPINFOEXW, UpdateProcThreadAttribute, WaitForSingleObject,
};
use windows::core::{PCWSTR, PWSTR};

/// A pane never goes below 2x1 cells (zero-size resizes crash TUIs).
pub fn clamp_size(cols: u16, rows: u16) -> (u16, u16) {
    (cols.max(2), rows.max(1))
}

fn coord(cols: u16, rows: u16) -> COORD {
    let (c, r) = clamp_size(cols, rows);
    COORD { X: c as i16, Y: r as i16 }
}

pub struct Pty {
    hpcon: HPCON,
    job: OwnedHandle,
    process: OwnedHandle,
    pid: u32,
    input: File,
    output: Option<File>,
}

impl Pty {
    pub fn spawn(
        cmdline: &str,
        cwd: Option<&Path>,
        env: &[(String, String)],
        cols: u16,
        rows: u16,
    ) -> windows::core::Result<Pty> {
        unsafe {
            let (mut in_read, mut in_write) = (HANDLE::default(), HANDLE::default());
            let (mut out_read, mut out_write) = (HANDLE::default(), HANDLE::default());
            CreatePipe(&mut in_read, &mut in_write, None, 0)?;
            CreatePipe(&mut out_read, &mut out_write, None, 0)?;
            let input = File::from_raw_handle(in_write.0);
            let output = File::from_raw_handle(out_read.0);

            let hpcon = CreatePseudoConsole(coord(cols, rows), in_read, out_write, 0);
            // ConPTY duplicated its ends; ours must close or reads never see EOF.
            let _ = CloseHandle(in_read);
            let _ = CloseHandle(out_write);
            let hpcon = hpcon?;

            let mut attr_size = 0usize;
            let _ = InitializeProcThreadAttributeList(None, 1, None, &mut attr_size);
            let mut attr_buf = vec![0u8; attr_size];
            let attrs = LPPROC_THREAD_ATTRIBUTE_LIST(attr_buf.as_mut_ptr().cast());
            let result = (|| {
                InitializeProcThreadAttributeList(Some(attrs), 1, None, &mut attr_size)?;
                UpdateProcThreadAttribute(
                    attrs,
                    0,
                    PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE as usize,
                    Some(hpcon.0 as *const c_void),
                    size_of::<HPCON>(),
                    None,
                    None,
                )?;

                let mut si = STARTUPINFOEXW::default();
                si.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
                // Null std handles stop the child inheriting ours when we are redirected.
                si.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
                si.lpAttributeList = attrs;

                let mut cmd: Vec<u16> = cmdline.encode_utf16().chain([0]).collect();
                let env_block = env_block(env);
                let cwd_w: Option<Vec<u16>> =
                    cwd.map(|p| p.as_os_str().to_string_lossy().encode_utf16().chain([0]).collect());
                let mut pi = PROCESS_INFORMATION::default();
                CreateProcessW(
                    PCWSTR::null(),
                    Some(PWSTR(cmd.as_mut_ptr())),
                    None,
                    None,
                    false,
                    EXTENDED_STARTUPINFO_PRESENT | CREATE_SUSPENDED | CREATE_UNICODE_ENVIRONMENT,
                    Some(env_block.as_ptr().cast()),
                    cwd_w.as_ref().map_or(PCWSTR::null(), |w| PCWSTR(w.as_ptr())),
                    &si.StartupInfo,
                    &mut pi,
                )?;
                Ok(pi)
            })();
            DeleteProcThreadAttributeList(attrs);
            let pi = match result {
                Ok(pi) => pi,
                Err(e) => {
                    ClosePseudoConsole(hpcon);
                    return Err(e);
                }
            };
            let process = OwnedHandle::from_raw_handle(pi.hProcess.0);
            let thread = OwnedHandle::from_raw_handle(pi.hThread.0);

            let job = OwnedHandle::from_raw_handle(CreateJobObjectW(None, PCWSTR::null())?.0);
            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            SetInformationJobObject(
                raw(&job),
                JobObjectExtendedLimitInformation,
                (&raw const limits).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )?;
            AssignProcessToJobObject(raw(&job), raw(&process))?;
            ResumeThread(raw(&thread));

            Ok(Pty { hpcon, job, process, pid: pi.dwProcessId, input, output: Some(output) })
        }
    }

    /// The ConPTY output pipe. Panics if called twice.
    pub fn take_reader(&mut self) -> File {
        self.output.take().expect("take_reader called twice")
    }

    pub fn write(&self, bytes: &[u8]) -> io::Result<()> {
        (&self.input).write_all(bytes)
    }

    pub fn resize(&self, cols: u16, rows: u16) -> windows::core::Result<()> {
        unsafe { ResizePseudoConsole(self.hpcon, coord(cols, rows)) }
    }

    pub fn try_wait(&self) -> Option<u32> {
        unsafe {
            if WaitForSingleObject(raw(&self.process), 0) != WAIT_OBJECT_0 {
                return None;
            }
            let mut code = 0u32;
            GetExitCodeProcess(raw(&self.process), &mut code).ok()?;
            Some(code)
        }
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Every live process in this pane's job (shell and all descendants).
    pub fn job_pids(&self) -> Vec<u32> {
        #[repr(C)]
        struct IdList {
            assigned: u32,
            in_list: u32,
            ids: [usize; 512],
        }
        let mut list = IdList { assigned: 0, in_list: 0, ids: [0; 512] };
        let ok = unsafe {
            QueryInformationJobObject(
                Some(raw(&self.job)),
                JobObjectBasicProcessIdList,
                (&raw mut list).cast(),
                size_of::<IdList>() as u32,
                None,
            )
        };
        if ok.is_err() {
            return Vec::new();
        }
        list.ids[..list.in_list as usize].iter().map(|&p| p as u32).collect()
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        // Closing our read end first stops ClosePseudoConsole blocking on a full pipe.
        self.output.take();
        unsafe {
            // Kill descendants before conhost goes away. OwnedHandle closes the job again below; this is the kill.
            let _ = windows::Win32::System::JobObjects::TerminateJobObject(raw(&self.job), 1);
            ClosePseudoConsole(self.hpcon);
        }
    }
}

/// True while `pid` names a running process.
pub fn is_alive(pid: u32) -> bool {
    unsafe {
        match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE, false, pid) {
            Ok(h) => {
                let alive = WaitForSingleObject(h, 0) == WAIT_TIMEOUT;
                let _ = CloseHandle(h);
                alive
            }
            Err(_) => false,
        }
    }
}

fn raw(h: &OwnedHandle) -> HANDLE {
    use std::os::windows::io::AsRawHandle;
    HANDLE(h.as_raw_handle())
}

/// Current environment with `overrides` applied, as a sorted UTF-16 block.
fn env_block(overrides: &[(String, String)]) -> Vec<u16> {
    let mut vars: Vec<(String, String)> = std::env::vars()
        .filter(|(k, _)| !overrides.iter().any(|(o, _)| o.eq_ignore_ascii_case(k)))
        .chain(overrides.iter().cloned())
        .collect();
    vars.sort_by_key(|(k, _)| k.to_uppercase());
    let mut block: Vec<u16> = Vec::new();
    for (k, v) in vars {
        block.extend(format!("{k}={v}").encode_utf16());
        block.push(0);
    }
    block.push(0);
    block
}
