//! ConPTY child process inside a kill-on-close Job Object.
use std::ffi::c_void;
use std::fs::File;
use std::io::{self, Write};
use std::os::windows::io::{FromRawHandle, OwnedHandle};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

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
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::core::{HRESULT, PCWSTR, PWSTR, s};

/// A pane never goes below 2x1 cells (zero-size resizes crash TUIs).
pub fn clamp_size(cols: u16, rows: u16) -> (u16, u16) {
    (cols.max(2), rows.max(1))
}

fn coord(cols: u16, rows: u16) -> COORD {
    let (c, r) = clamp_size(cols, rows);
    COORD { X: c as i16, Y: r as i16 }
}

/// Which ConPTY implementation creates pseudoconsoles.
#[derive(Clone, Debug)]
pub enum Backend {
    /// Microsoft's redistributable conpty.dll (+ OpenConsole.exe beside it), run in passthrough mode.
    Bundled(Bundled),
    /// kernel32's ConPTY. Strips the Kitty graphics APC, so no inline images.
    Inbox,
}

type CreateFn = unsafe extern "system" fn(COORD, HANDLE, HANDLE, u32, *mut HPCON) -> HRESULT;
type ResizeFn = unsafe extern "system" fn(HPCON, COORD) -> HRESULT;
type CloseFn = unsafe extern "system" fn(HPCON);
type Proc = unsafe extern "system" fn() -> isize;

#[derive(Clone)]
pub struct Bundled {
    pub path: PathBuf,
    create: CreateFn,
    resize: ResizeFn,
    close: CloseFn,
}

impl std::fmt::Debug for Bundled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Bundled").field(&self.path).finish()
    }
}

/// PSEUDOCONSOLE_PASSTHROUGH_MODE: forward VT the child writes instead of re-rendering it.
const PASSTHROUGH: u32 = 8;

/// `Bundled` when `dll` loads and exports the three Conpty* functions, else `Inbox`.
pub fn load_backend(dll: Option<&Path>) -> Backend {
    let Some(dll) = dll else { return Backend::Inbox };
    unsafe {
        let w: Vec<u16> = dll.as_os_str().to_string_lossy().encode_utf16().chain([0]).collect();
        // The module stays loaded for the life of the process.
        let Ok(m) = LoadLibraryW(PCWSTR(w.as_ptr())) else { return Backend::Inbox };
        let (Some(c), Some(r), Some(x)) = (
            GetProcAddress(m, s!("ConptyCreatePseudoConsole")),
            GetProcAddress(m, s!("ConptyResizePseudoConsole")),
            GetProcAddress(m, s!("ConptyClosePseudoConsole")),
        ) else {
            return Backend::Inbox;
        };
        Backend::Bundled(Bundled {
            path: dll.to_path_buf(),
            create: std::mem::transmute::<Proc, CreateFn>(c),
            resize: std::mem::transmute::<Proc, ResizeFn>(r),
            close: std::mem::transmute::<Proc, CloseFn>(x),
        })
    }
}

/// `WRAITH_CONPTY_DLL`, else conpty.dll beside the running exe, else inbox (logged once).
pub fn default_backend() -> &'static Backend {
    static B: OnceLock<Backend> = OnceLock::new();
    B.get_or_init(|| {
        let path = std::env::var_os("WRAITH_CONPTY_DLL").map(PathBuf::from).or_else(|| {
            std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("conpty.dll")))
        });
        let b = load_backend(path.as_deref().filter(|p| p.exists()));
        if matches!(b, Backend::Inbox) {
            tracing::warn!("bundled conpty.dll not found; using inbox ConPTY (no inline images)");
        }
        b
    })
}

impl Backend {
    unsafe fn create(&self, size: COORD, input: HANDLE, output: HANDLE) -> windows::core::Result<HPCON> {
        match self {
            Backend::Bundled(b) => {
                let mut h = HPCON::default();
                unsafe { (b.create)(size, input, output, PASSTHROUGH, &mut h) }.ok()?;
                Ok(h)
            }
            Backend::Inbox => unsafe { CreatePseudoConsole(size, input, output, 0) },
        }
    }

    unsafe fn resize(&self, h: HPCON, size: COORD) -> windows::core::Result<()> {
        match self {
            Backend::Bundled(b) => unsafe { (b.resize)(h, size) }.ok(),
            Backend::Inbox => unsafe { ResizePseudoConsole(h, size) },
        }
    }

    unsafe fn close(&self, h: HPCON) {
        match self {
            Backend::Bundled(b) => unsafe { (b.close)(h) },
            Backend::Inbox => unsafe { ClosePseudoConsole(h) },
        }
    }
}

pub struct Pty {
    backend: Backend,
    hpcon: HPCON,
    job: OwnedHandle,
    process: OwnedHandle,
    pid: u32,
    input: File,
    output: Option<File>,
}

impl Pty {
    /// Spawn on the process-wide default backend.
    pub fn spawn(
        cmdline: &str,
        cwd: Option<&Path>,
        env: &[(String, String)],
        cols: u16,
        rows: u16,
    ) -> windows::core::Result<Pty> {
        Self::spawn_with(default_backend(), cmdline, cwd, env, cols, rows)
    }

    pub fn spawn_with(
        backend: &Backend,
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

            let hpcon = backend.create(coord(cols, rows), in_read, out_write);
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
                    backend.close(hpcon);
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

            Ok(Pty { backend: backend.clone(), hpcon, job, process, pid: pi.dwProcessId, input, output: Some(output) })
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
        unsafe { self.backend.resize(self.hpcon, coord(cols, rows)) }
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

    pub fn backend(&self) -> &Backend {
        &self.backend
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
            self.backend.close(self.hpcon);
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
