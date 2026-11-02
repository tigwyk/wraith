//! Win32 top-level window: per-monitor DPI, dark title bar, optional Mica, message loop.
use std::cell::RefCell;

use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, LRESULT, RECT, WAIT_OBJECT_0, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DWMSBT_MAINWINDOW, DWMWA_SYSTEMBACKDROP_TYPE, DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{BLACK_BRUSH, GetStockObject, HBRUSH};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForWindow, GetSystemMetricsForDpi, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DispatchMessageW,
    GetClientRect, IDC_ARROW, IsZoomed, LoadCursorW, MSG, NCCALCSIZE_PARAMS, PM_REMOVE, PeekMessageW,
    SM_CXPADDEDBORDER, SM_CYFRAME, SWP_FRAMECHANGED, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SetWindowPos,
    WM_NCCALCSIZE,
    PostQuitMessage, RegisterClassExW, SW_SHOW, ShowWindow, TranslateMessage, WM_CLOSE, WM_KEYDOWN,
    WM_DESTROY, WM_DPICHANGED, WM_QUIT, WM_SIZE, WNDCLASSEXW, WS_EX_NOREDIRECTIONBITMAP,
    WS_OVERLAPPEDWINDOW, MsgWaitForMultipleObjectsEx, MWMO_INPUTAVAILABLE, QS_ALLINPUT,
};
use windows::Win32::UI::Input::KeyboardAndMouse::VK_PROCESSKEY;
use windows::core::{PCWSTR, Result, w};

/// Messages the app cares about; everything else arrives as `Raw` (keys, mouse, IME, focus).
pub enum WinMsg {
    Resize { w: u32, h: u32 },
    DpiChanged { dpi: u32, suggested: RECT },
    Close,
    Raw { msg: u32, wparam: WPARAM, lparam: LPARAM },
}

type Handler = Box<dyn FnMut(HWND, WinMsg) -> Option<LRESULT>>;

thread_local! {
    static HANDLER: RefCell<Option<Handler>> = const { RefCell::new(None) };
}

pub struct Window {
    hwnd: HWND,
}

impl Window {
    pub fn create(title: &str, w: i32, h: i32, mica: bool) -> Result<Window> {
        unsafe {
            let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            let hinst = GetModuleHandleW(None)?;
            let class = w!("WraithWindow");
            let wc = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(wndproc),
                hInstance: hinst.into(),
                hCursor: LoadCursorW(None, IDC_ARROW)?,
                hbrBackground: HBRUSH(GetStockObject(BLACK_BRUSH).0),
                lpszClassName: class,
                ..Default::default()
            };
            RegisterClassExW(&wc);
            let title: Vec<u16> = title.encode_utf16().chain([0]).collect();
            // No redirection bitmap: the flip-model swap chain is the only surface.
            let hwnd = CreateWindowExW(
                WS_EX_NOREDIRECTIONBITMAP,
                class,
                PCWSTR(title.as_ptr()),
                WS_OVERLAPPEDWINDOW,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                w,
                h,
                None,
                None,
                Some(hinst.into()),
                None,
            )?;
            let dark: i32 = 1;
            let _ = DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, (&raw const dark).cast(), 4);
            if mica {
                let backdrop = DWMSBT_MAINWINDOW;
                let _ = DwmSetWindowAttribute(hwnd, DWMWA_SYSTEMBACKDROP_TYPE, (&raw const backdrop).cast(), 4);
            }
            let _ = SetWindowPos(hwnd, None, 0, 0, 0, 0, SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER);
            Ok(Window { hwnd })
        }
    }

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    pub fn show(&self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_SHOW);
        }
    }

    pub fn is_maximized(&self) -> bool {
        unsafe { IsZoomed(self.hwnd) }.as_bool()
    }

    pub fn dpi(&self) -> u32 {
        unsafe { GetDpiForWindow(self.hwnd) }.max(96)
    }

    pub fn client_size(&self) -> (u32, u32) {
        let mut r = RECT::default();
        unsafe {
            let _ = GetClientRect(self.hwnd, &mut r);
        }
        ((r.right - r.left).max(0) as u32, (r.bottom - r.top).max(0) as u32)
    }
}

pub enum LoopControl {
    Continue,
    Exit,
}

/// What `idle` asks of the loop: whether to keep going, how long it may sleep, and an optional
/// handle to wait on as well (the swap chain's frame-latency object while a frame is pending).
pub struct Idle {
    pub ctl: LoopControl,
    pub sleep_ms: u32,
    pub wait_on: Option<HANDLE>,
}

/// Run until `WM_QUIT`. `handler` receives window messages; `idle(signaled)` runs whenever the queue
/// is empty. `signaled` is true when the loop woke because `wait_on` fired: that wait *acquired* it.
pub fn run_message_loop(
    handler: impl FnMut(HWND, WinMsg) -> Option<LRESULT> + 'static,
    mut idle: impl FnMut(bool) -> Idle,
) {
    let mut signaled = false;
    HANDLER.with(|h| *h.borrow_mut() = Some(Box::new(handler)));
    let mut msg = MSG::default();
    loop {
        // Window messages (input) are always drained before the app does frame work.
        while unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE) }.as_bool() {
            if msg.message == WM_QUIT {
                HANDLER.with(|h| h.borrow_mut().take());
                return;
            }
            // TranslateMessage only for IME keys (it drives ImmTranslateMessage -> WM_IME_COMPOSITION);
            // everything else goes to input::KeyTranslator, which owns key->text and dead-key state.
            unsafe {
                if msg.message == WM_KEYDOWN && msg.wParam.0 == usize::from(VK_PROCESSKEY.0) {
                    let _ = TranslateMessage(&msg);
                }
                DispatchMessageW(&msg);
            }
        }
        let next = idle(std::mem::take(&mut signaled));
        if let LoopControl::Exit = next.ctl {
            HANDLER.with(|h| h.borrow_mut().take());
            return;
        }
        let handles: Vec<HANDLE> = next.wait_on.into_iter().collect();
        let r = unsafe { MsgWaitForMultipleObjectsEx(Some(&handles), next.sleep_ms, QS_ALLINPUT, MWMO_INPUTAVAILABLE) };
        signaled = !handles.is_empty() && r == WAIT_OBJECT_0;
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        let ev = match msg {
            WM_SIZE => Some(WinMsg::Resize { w: (lparam.0 & 0xffff) as u32, h: ((lparam.0 >> 16) & 0xffff) as u32 }),
            WM_DPICHANGED => Some(WinMsg::DpiChanged {
                dpi: (wparam.0 & 0xffff) as u32,
                suggested: *(lparam.0 as *const RECT),
            }),
            // Custom title bar: keep Windows' left/right/bottom frame (resize edges, shadow), drop the
            // caption so the tab bar is drawn there. Handled here because it arrives during CreateWindowExW.
            WM_NCCALCSIZE if wparam.0 != 0 => {
                let p = &mut *(lparam.0 as *mut NCCALCSIZE_PARAMS);
                let top = p.rgrc[0].top;
                let r = DefWindowProcW(hwnd, msg, wparam, lparam);
                p.rgrc[0].top = top;
                if IsZoomed(hwnd).as_bool() {
                    // A maximized window hangs its frame off the monitor; keep the bar on screen.
                    let dpi = GetDpiForWindow(hwnd);
                    p.rgrc[0].top += GetSystemMetricsForDpi(SM_CYFRAME, dpi) + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi);
                }
                return r;
            }
            WM_CLOSE => Some(WinMsg::Close),
            WM_DESTROY => {
                PostQuitMessage(0);
                None
            }
            _ => Some(WinMsg::Raw { msg, wparam, lparam }),
        };
        if let Some(ev) = ev {
            // try_borrow: a handler that re-enters (e.g. SetWindowPos inside DpiChanged) falls back to default.
            let r = HANDLER.with(|h| h.try_borrow_mut().ok().and_then(|mut h| h.as_mut().and_then(|f| f(hwnd, ev))));
            if let Some(r) = r {
                return r;
            }
        }
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }
}
