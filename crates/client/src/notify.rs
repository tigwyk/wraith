//! Telling the user an agent needs them (spec §5.4): which ways to use (pure), and the Windows side of
//! them: toast, taskbar flash, taskbar overlay badge.
use wraith_config::AgentNotify;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ways {
    pub toast: bool,
    pub flash: bool,
    pub badge: bool,
    pub sound: bool,
}

/// The ways to announce an agent for a window (`foreground`) whose pane is or is not `visible`, at
/// `now_min` minutes after midnight. Quiet hours keep only the (silent) taskbar badge.
pub fn notify_ways(cfg: &AgentNotify, foreground: bool, visible: bool, now_min: u32) -> Ways {
    if foreground && visible && !cfg.when_always {
        return Ways::default();
    }
    let quiet = cfg.quiet.is_some_and(|(a, b)| if a <= b { (a..b).contains(&now_min) } else { now_min >= a || now_min < b });
    Ways {
        toast: cfg.toast && !quiet,
        flash: cfg.flash && !quiet && !foreground,
        badge: cfg.badge && !foreground,
        sound: cfg.sound && cfg.toast && !quiet,
    }
}

// ---- Windows side ----

use windows::Data::Xml::Dom::XmlDocument;
use windows::Foundation::TypedEventHandler;
use windows::UI::Notifications::{ToastNotification, ToastNotificationManager, ToastNotifier};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Storage::EnhancedStorage::PKEY_AppUserModel_ID;
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemAlloc, IPersistFile};
use windows::Win32::System::Variant::VT_LPWSTR;
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
use windows::Win32::UI::Shell::{ITaskbarList3, IShellLinkW, SetCurrentProcessExplicitAppUserModelID, ShellLink, TaskbarList};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    FLASHW_ALL, FLASHW_TIMERNOFG, FLASHWINFO, FlashWindowEx, GetSystemMetrics, HICON, IMAGE_ICON, LR_SHARED, LoadImageW, PostMessageW, SM_CXSMICON,
    SM_CYSMICON, WM_APP,
};
use windows::core::{HSTRING, Interface, PCWSTR, PWSTR, Result};

/// The app's identity for toasts and taskbar grouping (spec §5.4).
pub const AUMID: &str = "Wraith.Terminal";

/// Posted to the window when the user clicks a toast; `wparam` is the pane.
pub const WM_TOAST_CLICKED: u32 = WM_APP + 1;

/// Toasts kept alive so their click handlers still fire (newest last).
const KEEP: usize = 16;

/// Give this process the app id. Must run before the first window is created.
pub fn set_app_id() {
    let id = HSTRING::from(AUMID);
    if let Err(e) = unsafe { SetCurrentProcessExplicitAppUserModelID(&id) } {
        tracing::warn!("app id: {e}");
    }
}

/// The Start Menu shortcut toasts are tied to.
pub fn default_shortcut() -> Option<std::path::PathBuf> {
    Some(std::path::Path::new(&std::env::var_os("APPDATA")?).join(r"Microsoft\Windows\Start Menu\Programs\Wraith.lnk"))
}

/// An unpackaged app may only show toasts under an id some Start Menu shortcut carries. Create
/// `Wraith.lnk` once (first run, spec §5.4); an existing one is left alone.
fn ensure_shortcut() -> Result<()> {
    match default_shortcut() {
        Some(lnk) if !lnk.exists() => register_shortcut(&lnk),
        _ => Ok(()),
    }
}

/// Write (or replace) a shortcut at `lnk` to this executable, carrying the app id. The installer runs
/// this (`wraith.exe --register`) so the Start Menu entry points at the installed copy.
pub fn register_shortcut(lnk: &std::path::Path) -> Result<()> {
    let exe = std::env::current_exe().map_err(|e| windows::core::Error::new(windows::core::HRESULT(-1), e.to_string()))?;
    let wide = |s: &std::ffi::OsStr| -> Vec<u16> {
        use std::os::windows::ffi::OsStrExt;
        s.encode_wide().chain([0]).collect()
    };
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        link.SetPath(PCWSTR(wide(exe.as_os_str()).as_ptr()))?;
        let store: IPropertyStore = link.cast()?;
        // The id must be a VT_LPWSTR (CoTaskMem string, freed by PROPVARIANT's drop).
        let id: Vec<u16> = AUMID.encode_utf16().chain([0]).collect();
        let buf = CoTaskMemAlloc(id.len() * 2) as *mut u16;
        if buf.is_null() {
            return Ok(());
        }
        std::ptr::copy_nonoverlapping(id.as_ptr(), buf, id.len());
        let mut pv = PROPVARIANT::default();
        (*pv.Anonymous.Anonymous).vt = VT_LPWSTR;
        (*pv.Anonymous.Anonymous).Anonymous.pwszVal = PWSTR(buf);
        store.SetValue(&PKEY_AppUserModel_ID, &pv)?;
        store.Commit()?;
        let file: IPersistFile = link.cast()?;
        file.Save(PCWSTR(wide(lnk.as_os_str()).as_ptr()), true)?;
    }
    Ok(())
}

pub struct Notifier {
    hwnd: HWND,
    toasts: Option<ToastNotifier>,
    live: Vec<ToastNotification>,
    taskbar: Option<ITaskbarList3>,
    badged: bool,
}

impl Notifier {
    pub fn new(hwnd: HWND) -> Notifier {
        if let Err(e) = ensure_shortcut() {
            tracing::warn!("start menu shortcut: {e}");
        }
        let toasts = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(AUMID))
            .inspect_err(|e| tracing::warn!("toasts unavailable: {e}"))
            .ok();
        let taskbar = unsafe { CoCreateInstance::<_, ITaskbarList3>(&TaskbarList, None, CLSCTX_INPROC_SERVER) }
            .ok()
            .filter(|t| unsafe { t.HrInit() }.is_ok());
        Notifier { hwnd, toasts, live: Vec::new(), taskbar, badged: false }
    }

    /// Announce that `pane` needs the user, in the chosen ways.
    pub fn show(&mut self, ways: Ways, pane: u64, title: &str, body: &str) {
        if ways.toast
            && let Err(e) = self.toast(pane, title, body, ways.sound)
        {
            tracing::warn!("toast: {e}");
        }
        if ways.flash {
            let info = FLASHWINFO { cbSize: size_of::<FLASHWINFO>() as u32, hwnd: self.hwnd, dwFlags: FLASHW_ALL | FLASHW_TIMERNOFG, uCount: 0, dwTimeout: 0 };
            unsafe {
                let _ = FlashWindowEx(&info);
            }
        }
        if ways.badge
            && let Some(t) = &self.taskbar
        {
            // Resource 2 from build.rs: the orange ghost, at the small-icon size the overlay is drawn at.
            let icon = unsafe {
                let size = (GetSystemMetrics(SM_CXSMICON), GetSystemMetrics(SM_CYSMICON));
                GetModuleHandleW(None).and_then(|m| LoadImageW(Some(m.into()), PCWSTR(2 as _), IMAGE_ICON, size.0, size.1, LR_SHARED))
            }
            .map(|h| HICON(h.0))
            .unwrap_or_default();
            self.badged = unsafe { t.SetOverlayIcon(self.hwnd, icon, &HSTRING::from(title)) }.is_ok();
        }
    }

    /// The window came to the foreground: the taskbar badge has done its job.
    pub fn clear_badge(&mut self) {
        if self.badged
            && let Some(t) = &self.taskbar
        {
            unsafe {
                let _ = t.SetOverlayIcon(self.hwnd, Default::default(), PCWSTR::null());
            }
            self.badged = false;
        }
    }

    fn toast(&mut self, pane: u64, title: &str, body: &str, sound: bool) -> Result<()> {
        let Some(notifier) = &self.toasts else { return Ok(()) };
        // Text goes in as text nodes: agent messages are untrusted and never markup (spec §5.5).
        let audio = if sound { "" } else { r#"<audio silent="true"/>"# };
        let doc = XmlDocument::new()?;
        doc.LoadXml(&HSTRING::from(format!(
            r#"<toast activationType="foreground"><visual><binding template="ToastGeneric"><text/><text/></binding></visual><actions><action content="Open" arguments="open" activationType="foreground"/></actions>{audio}</toast>"#
        )))?;
        let texts = doc.GetElementsByTagName(&HSTRING::from("text"))?;
        texts.Item(0)?.AppendChild(&doc.CreateTextNode(&HSTRING::from(title))?)?;
        texts.Item(1)?.AppendChild(&doc.CreateTextNode(&HSTRING::from(body))?)?;
        let toast = ToastNotification::CreateToastNotification(&doc)?;
        let hwnd = self.hwnd.0 as isize;
        toast.Activated(&TypedEventHandler::new(move |_, _| {
            unsafe {
                let _ = PostMessageW(Some(HWND(hwnd as *mut _)), WM_TOAST_CLICKED, WPARAM(pane as usize), LPARAM(0));
            }
            Ok(())
        }))?;
        notifier.Show(&toast)?;
        tracing::info!(pane, "toast shown");
        self.live.push(toast);
        if self.live.len() > KEEP {
            self.live.remove(0);
        }
        Ok(())
    }
}
