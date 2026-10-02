use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc, Arc, Mutex,
};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIM_ADD, NIM_DELETE,
    NIM_MODIFY, NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::*;

const ICON_MESSAGE: u32 = WM_APP + 41;
const ICON_ID: u32 = 1;
const OPEN_POPUP: u32 = WM_APP + 42;
const REFRESH_POPUP: u32 = WM_APP + 43;
static CLASS_ID: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Copy, Debug)]
pub enum TrayAction {
    Startup,
    Topmost,
    Notifications,
    CheckUpdate,
    Exit,
}

#[derive(Default)]
pub(crate) struct Checks {
    pub startup: AtomicBool,
    pub topmost: AtomicBool,
    pub notifications: AtomicBool,
    pub palette: AtomicUsize,
    pub update_available: AtomicBool,
}

struct Client {
    hwnd: isize,
    checks: Arc<Checks>,
    pending: Arc<Mutex<Option<crate::popup::Open>>>,
    active: Arc<AtomicBool>,
}
impl Drop for Client {
    fn drop(&mut self) {
        unsafe {
            let _ = PostMessageW(Some(HWND(self.hwnd as _)), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
    }
}

#[derive(Clone)]
pub struct TrayHandle(Arc<Client>);
impl TrayHandle {
    pub fn sync(
        &self,
        startup: bool,
        topmost: bool,
        notifications: bool,
        palette: usize,
        update_available: bool,
    ) {
        self.0.checks.startup.store(startup, Ordering::Relaxed);
        self.0.checks.topmost.store(topmost, Ordering::Relaxed);
        self.0
            .checks
            .notifications
            .store(notifications, Ordering::Relaxed);
        self.0.checks.palette.store(palette, Ordering::Relaxed);
        self.0
            .checks
            .update_available
            .store(update_available, Ordering::Relaxed);
        unsafe {
            let _ = PostMessageW(
                Some(HWND(self.0.hwnd as _)),
                REFRESH_POPUP,
                WPARAM(0),
                LPARAM(0),
            );
        }
    }

    pub fn open(&self, point: (i32, i32), avoid: Option<crate::domain::Rect>, settings: bool) {
        *self.0.pending.lock().unwrap() = Some(crate::popup::Open {
            point,
            avoid,
            settings,
            tray_anchor: None,
        });
        self.0.active.store(true, Ordering::Release);
        if unsafe {
            PostMessageW(
                Some(HWND(self.0.hwnd as _)),
                OPEN_POPUP,
                WPARAM(0),
                LPARAM(0),
            )
        }
        .is_err()
        {
            self.0.active.store(false, Ordering::Release);
        }
    }

    pub fn popup_visible(&self) -> bool {
        self.0.active.load(Ordering::Acquire)
    }

    /// Windows decides whether to display an accepted balloon (for example, Focus Assist).
    pub fn notify(&self, message: &str) -> Result<(), String> {
        let mut data = icon_data(HWND(self.0.hwnd as _));
        data.uFlags = NIF_INFO;
        data.dwInfoFlags = NIIF_INFO;
        wide_copy(&mut data.szInfoTitle, "CodexBadge 额度提醒");
        wide_copy(&mut data.szInfo, message);
        if unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) }.as_bool() {
            Ok(())
        } else {
            Err("Windows 未接受托盘通知".into())
        }
    }
}

struct State {
    checks: Arc<Checks>,
    pending: Arc<Mutex<Option<crate::popup::Open>>>,
    popup: isize,
    icon: HICON,
    taskbar_created: u32,
}

pub fn start(callback: impl Fn(TrayAction) + Send + Sync + 'static) -> Result<TrayHandle, String> {
    let checks = Arc::new(Checks::default());
    let thread_checks = checks.clone();
    let pending = Arc::new(Mutex::new(None));
    let thread_pending = pending.clone();
    let active = Arc::new(AtomicBool::new(false));
    let thread_active = active.clone();
    let callback: Arc<dyn Fn(TrayAction) + Send + Sync> = Arc::new(callback);
    let (sender, receiver) = mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("codex-badge-tray".into())
        .spawn(move || unsafe {
            let result = (|| -> Result<(), String> {
                let instance = HINSTANCE(GetModuleHandleW(None).map_err(|e| e.to_string())?.0);
                let class_name: Vec<u16> = format!(
                    "CodexBadgeTray{}-{}",
                    std::process::id(),
                    CLASS_ID.fetch_add(1, Ordering::Relaxed)
                )
                .encode_utf16()
                .chain(Some(0))
                .collect();
                let class = PCWSTR(class_name.as_ptr());
                let definition = WNDCLASSW {
                    lpfnWndProc: Some(window_proc),
                    hInstance: instance,
                    lpszClassName: class,
                    ..Default::default()
                };
                if RegisterClassW(&definition) == 0 {
                    return Err("无法注册托盘窗口".into());
                }
                let icon = official_icon();
                let icon = match icon {
                    Ok(v) => v,
                    Err(e) => {
                        let _ = UnregisterClassW(class, Some(instance));
                        return Err(e.to_string());
                    }
                };
                let mut state = Box::new(State {
                    checks: thread_checks.clone(),
                    pending: thread_pending,
                    popup: 0,
                    icon,
                    taskbar_created: RegisterWindowMessageW(w!("TaskbarCreated")),
                });
                // A hidden top-level window receives Explorer's TaskbarCreated broadcast.
                let hwnd = CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    class,
                    w!("CodexBadge tray"),
                    WS_OVERLAPPED,
                    0,
                    0,
                    0,
                    0,
                    None,
                    None,
                    Some(instance),
                    Some((&mut *state as *mut State).cast()),
                );
                let hwnd = match hwnd {
                    Ok(v) => v,
                    Err(e) => {
                        let _ = UnregisterClassW(class, Some(instance));
                        return Err(e.to_string());
                    }
                };
                state.popup =
                    match crate::popup::create(hwnd, thread_checks, callback, thread_active) {
                        Ok(popup) => popup,
                        Err(error) => {
                            let _ = DestroyWindow(hwnd);
                            let _ = DestroyIcon(icon);
                            let _ = UnregisterClassW(class, Some(instance));
                            return Err(error.to_string());
                        }
                    };
                if !add_icon(hwnd, state.icon) {
                    let _ = DestroyWindow(hwnd);
                    let _ = UnregisterClassW(class, Some(instance));
                    return Err("无法创建 Windows 托盘图标".into());
                }
                if sender.send(Ok(hwnd.0 as isize)).is_err() {
                    let _ = DestroyWindow(hwnd);
                } else {
                    let mut message = MSG::default();
                    while GetMessageW(&mut message, None, 0, 0).0 > 0 {
                        let _ = TranslateMessage(&message);
                        DispatchMessageW(&message);
                    }
                    if IsWindow(Some(hwnd)).as_bool() {
                        let _ = DestroyWindow(hwnd);
                    }
                }
                let _ = UnregisterClassW(class, Some(instance));
                let _ = DestroyIcon(icon);
                Ok(())
            })();
            if let Err(error) = result {
                let _ = sender.send(Err(error));
            }
        })
        .map_err(|e| e.to_string())?;
    let hwnd = receiver.recv().map_err(|e| e.to_string())??;
    Ok(TrayHandle(Arc::new(Client {
        hwnd,
        checks,
        pending,
        active,
    })))
}

fn icon_data(hwnd: HWND) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: ICON_ID,
        ..Default::default()
    }
}

fn icon_image(bytes: &[u8], desired: i32) -> Option<&[u8]> {
    if bytes.get(..4)? != [0, 0, 1, 0] {
        return None;
    }
    let count = u16::from_le_bytes(bytes.get(4..6)?.try_into().ok()?) as usize;
    (0..count)
        .filter_map(|index| {
            let entry = bytes.get(6 + index * 16..22 + index * 16)?;
            let width = if entry[0] == 0 { 256 } else { entry[0] as i32 };
            let length = u32::from_le_bytes(entry[8..12].try_into().ok()?) as usize;
            let offset = u32::from_le_bytes(entry[12..16].try_into().ok()?) as usize;
            let image = bytes.get(offset..offset.checked_add(length)?)?;
            Some((
                (
                    if width >= desired { 0 } else { 1 },
                    (width - desired).abs(),
                ),
                image,
            ))
        })
        .min_by_key(|(key, _)| *key)
        .map(|(_, image)| image)
}

unsafe fn official_icon() -> windows::core::Result<HICON> {
    let size = GetSystemMetrics(SM_CXSMICON).max(16);
    let bytes = include_bytes!("../icons/icon.ico");
    let image = icon_image(bytes, size).ok_or_else(windows::core::Error::from_win32)?;
    CreateIconFromResourceEx(image, true, 0x00030000, size, size, LR_DEFAULTCOLOR)
}

fn wide_copy<const N: usize>(destination: &mut [u16; N], source: &str) {
    let mut length = 0;
    for character in source.chars() {
        let mut buffer = [0; 2];
        let units = character.encode_utf16(&mut buffer);
        if length + units.len() >= N {
            break;
        }
        destination[length..length + units.len()].copy_from_slice(units);
        length += units.len();
    }
    destination[length] = 0;
}

unsafe fn add_icon(hwnd: HWND, icon: HICON) -> bool {
    let mut data = icon_data(hwnd);
    data.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
    data.uCallbackMessage = ICON_MESSAGE;
    data.hIcon = icon;
    wide_copy(&mut data.szTip, "CodexBadge");
    Shell_NotifyIconW(NIM_ADD, &data).as_bool()
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_NCCREATE {
        let create = &*(lparam.0 as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
    }
    let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const State;
    if !pointer.is_null() {
        let state = &*pointer;
        if message == state.taskbar_created && message != 0 {
            add_icon(hwnd, state.icon);
            return LRESULT(0);
        }
        match message {
            ICON_MESSAGE
                if lparam.0 as u32 == WM_RBUTTONUP || lparam.0 as u32 == WM_CONTEXTMENU =>
            {
                let mut point = Default::default();
                if GetCursorPos(&mut point).is_ok() {
                    crate::popup::show(
                        state.popup,
                        crate::popup::Open {
                            point: (point.x, point.y),
                            avoid: None,
                            settings: false,
                            tray_anchor: windows::Win32::UI::Shell::Shell_NotifyIconGetRect(
                                &windows::Win32::UI::Shell::NOTIFYICONIDENTIFIER {
                                    cbSize: std::mem::size_of::<
                                        windows::Win32::UI::Shell::NOTIFYICONIDENTIFIER,
                                    >() as u32,
                                    hWnd: hwnd,
                                    uID: ICON_ID,
                                    ..Default::default()
                                },
                            )
                            .ok()
                            .map(|r| (r.left, r.top, r.right - r.left, r.bottom - r.top)),
                        },
                    );
                }
                return LRESULT(0);
            }
            OPEN_POPUP => {
                let pending = state.pending.lock().unwrap().take();
                if let Some(open) = pending {
                    crate::popup::show(state.popup, open);
                }
                return LRESULT(0);
            }
            WM_SETTINGCHANGE | WM_THEMECHANGED => {
                if state.checks.palette.load(Ordering::Relaxed) != 0 {
                    state
                        .checks
                        .palette
                        .store(crate::popup::system_palette(), Ordering::Relaxed);
                    crate::popup::refresh(state.popup);
                }
                return LRESULT(0);
            }
            REFRESH_POPUP => {
                crate::popup::refresh(state.popup);
                return LRESULT(0);
            }
            WM_CLOSE => {
                let _ = DestroyWindow(hwnd);
                return LRESULT(0);
            }
            WM_DESTROY => {
                if state.popup != 0 {
                    let _ = DestroyWindow(HWND(state.popup as _));
                }
                let data = icon_data(hwnd);
                let _ = Shell_NotifyIconW(NIM_DELETE, &data);
                PostQuitMessage(0);
                return LRESULT(0);
            }
            WM_NCDESTROY => {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            }
            _ => {}
        }
    }
    DefWindowProcW(hwnd, message, wparam, lparam)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn packaged_tray_icon_has_valid_images_at_supported_sizes() {
        let bytes = include_bytes!("../icons/icon.ico");
        for size in [16, 20, 24, 32, 48] {
            assert!(icon_image(bytes, size).is_some_and(|image| !image.is_empty()));
        }
        assert!(icon_image(&[0; 20], 16).is_none());
    }
    #[test]
    fn notification_text_is_terminated_without_splitting_surrogates() {
        let mut target = [0u16; 4];
        wide_copy(&mut target, "a😀b");
        assert_eq!(String::from_utf16(&target[..3]).unwrap(), "a😀");
        assert_eq!(target[3], 0);
        let mut short = [0u16; 2];
        wide_copy(&mut short, "😀");
        assert_eq!(short, [0, 0]);
    }
}
