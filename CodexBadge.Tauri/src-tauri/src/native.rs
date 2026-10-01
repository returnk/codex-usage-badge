use std::ffi::c_void;
use std::path::Path;
use uiautomation::{
    types::{ControlType, Handle, PropertyConditionFlags, TreeScope, UIProperty},
    UIAutomation, UIElement,
};
use windows::core::{BOOL, PWSTR};
use windows::Win32::Foundation::{
    CloseHandle, FILETIME, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows::Win32::Graphics::Dwm::{
    DwmGetWindowAttribute, DwmSetWindowAttribute, DWMNCRP_DISABLED, DWMWA_BORDER_COLOR,
    DWMWA_EXTENDED_FRAME_BOUNDS, DWMWA_NCRENDERING_POLICY,
};
use windows::Win32::Graphics::Gdi::{
    CreateRoundRectRgn, DeleteObject, EnumDisplayMonitors, GetMonitorInfoW, MonitorFromPoint,
    SetWindowRgn, HDC, HGDIOBJ, HMONITOR, MONITORINFO, MONITORINFOEXW, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::System::Threading::{
    GetProcessTimes, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, GetDpiForWindow, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumChildWindows, EnumWindows, GetClassNameW, GetClientRect, GetCursorPos, GetForegroundWindow,
    GetGUIThreadInfo, GetWindow, GetWindowLongPtrW, GetWindowRect, GetWindowThreadProcessId,
    IsIconic, IsWindow, IsWindowVisible, SetWindowLongPtrW, SetWindowPos, ShowWindow,
    GUITHREADINFO, GWLP_HWNDPARENT, GWL_EXSTYLE, GWL_STYLE, GW_OWNER, HWND_NOTOPMOST, HWND_TOP,
    HWND_TOPMOST, MA_NOACTIVATE, PA_NOACTIVATE, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOCOPYBITS,
    SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SWP_SHOWWINDOW, SW_HIDE, SW_SHOWNOACTIVATE,
    WM_MOUSEACTIVATE, WM_NCACTIVATE, WM_NCDESTROY, WM_POINTERACTIVATE, WS_CAPTION, WS_CHILD,
    WS_EX_APPWINDOW, WS_EX_CLIENTEDGE, WS_EX_DLGMODALFRAME, WS_EX_NOACTIVATE, WS_EX_STATICEDGE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_WINDOWEDGE, WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_POPUP,
    WS_SYSMENU, WS_THICKFRAME,
};

// Win32 fixture windows share desktop z-order even when Rust runs tests on
// different threads. Keep those fixtures isolated; this is absent in builds.
#[cfg(test)]
pub(crate) static WINDOW_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn hwnd(raw: isize) -> HWND {
    HWND(raw as *mut c_void)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcessIdentity {
    pub pid: u32,
    pub created: u64,
}

fn process_identity(pid: u32) -> windows::core::Result<ProcessIdentity> {
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid)?;
        let (mut created, mut exit, mut kernel, mut user) = (
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
        );
        let result = GetProcessTimes(process, &mut created, &mut exit, &mut kernel, &mut user);
        let _ = CloseHandle(process);
        result?;
        Ok(ProcessIdentity {
            pid,
            created: ((created.dwHighDateTime as u64) << 32) | created.dwLowDateTime as u64,
        })
    }
}

fn window_pid(raw: isize) -> u32 {
    let mut pid = 0;
    unsafe {
        GetWindowThreadProcessId(hwnd(raw), Some(&mut pid));
    }
    pid
}

#[derive(Debug, PartialEq, Eq)]
pub struct WindowObservation {
    pub pid: u32,
    pub tid: u32,
    pub owner: isize,
    pub visible: bool,
    pub iconic: bool,
    pub style: isize,
    pub exstyle: isize,
    pub rect: (i32, i32, i32, i32),
    pub client: (i32, i32),
    pub dpi: u32,
}

pub fn observe_window(raw: isize) -> Option<WindowObservation> {
    if !is_window(raw) {
        return None;
    }
    let rect = window_rect(raw)?;
    Some(WindowObservation {
        pid: window_pid(raw),
        tid: unsafe { GetWindowThreadProcessId(hwnd(raw), None) },
        owner: owner(raw),
        visible: is_visible(raw),
        iconic: is_minimized(raw),
        style: unsafe { GetWindowLongPtrW(hwnd(raw), GWL_STYLE) },
        exstyle: unsafe { GetWindowLongPtrW(hwnd(raw), GWL_EXSTYLE) },
        rect: (
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
        ),
        client: client_size(raw)?,
        dpi: dpi(raw),
    })
}

pub fn log_window(label: &str, raw: isize) {
    if let Some(o) = observe_window(raw) {
        crate::diagnostics::record(format!("window_observe label={label} hwnd={raw} pid={} tid={} owner={} visible={} iconic={} style={} exstyle={} rect={},{},{},{} client={},{} dpi={}",
            o.pid,o.tid,o.owner,o.visible,o.iconic,o.style,o.exstyle,o.rect.0,o.rect.1,o.rect.2,o.rect.3,o.client.0,o.client.1,o.dpi));
    }
}

pub fn focus_observation() -> (isize, u32, u32, isize) {
    unsafe {
        let foreground = GetForegroundWindow();
        let mut pid = 0;
        let tid = GetWindowThreadProcessId(foreground, Some(&mut pid));
        let mut info = GUITHREADINFO {
            cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        let focus = if tid != 0 && GetGUIThreadInfo(tid, &mut info).is_ok() {
            info.hwndFocus.0 as isize
        } else {
            0
        };
        (foreground.0 as isize, pid, tid, focus)
    }
}

pub fn observe_codex(
    current: isize,
    identity: &mut Option<ProcessIdentity>,
) -> crate::domain::HostObservation {
    use crate::domain::HostObservation;
    if std::env::args_os().any(|arg| arg == "--diagnose-no-codex") {
        return HostObservation::default();
    }
    if let Some(trusted) = *identity {
        let matches = is_window(current) && window_pid(current) == trusted.pid;
        if let Some(observation) = trusted_observation(
            current,
            matches,
            trusted,
            process_identity(trusted.pid),
            is_visible(current),
            is_minimized(current),
        ) {
            return observation;
        }
    }
    let found = find_codex_window(0);
    if found != 0 {
        *identity = process_identity(window_pid(found)).ok();
        if identity.is_some() {
            return HostObservation {
                window: found,
                visible: is_visible(found),
                iconic: is_minimized(found),
                unknown: false,
            };
        }
        return HostObservation {
            unknown: true,
            ..HostObservation::default()
        };
    }
    HostObservation::default()
}

pub fn refresh_codex_window(
    raw: isize,
    identity: Option<ProcessIdentity>,
) -> Option<crate::domain::HostObservation> {
    let identity = identity?;
    (is_window(raw) && window_pid(raw) == identity.pid).then(|| crate::domain::HostObservation {
        window: raw,
        visible: is_visible(raw),
        iconic: is_minimized(raw),
        unknown: false,
    })
}

pub fn host_frame(raw: isize) -> Option<(i32, i32, i32, i32)> {
    let r = frame_bounds(raw)?;
    Some((r.left, r.top, r.right - r.left, r.bottom - r.top))
}

fn trusted_observation(
    current: isize,
    matches: bool,
    trusted: ProcessIdentity,
    observed: windows::core::Result<ProcessIdentity>,
    visible: bool,
    iconic: bool,
) -> Option<crate::domain::HostObservation> {
    let unknown = observed
        .as_ref()
        .is_err_and(|error| error.code().0 != 0x80070057u32 as i32);
    if unknown || (matches && observed.is_ok_and(|now| now == trusted)) {
        Some(crate::domain::HostObservation {
            window: if matches { current } else { 0 },
            visible: matches && visible,
            iconic: matches && iconic,
            unknown,
        })
    } else {
        None
    }
}

pub fn dpi(raw: isize) -> u32 {
    unsafe { GetDpiForWindow(hwnd(raw)).max(96) }
}

pub fn dpi_at(point: (i32, i32)) -> u32 {
    unsafe {
        let monitor = MonitorFromPoint(
            POINT {
                x: point.0,
                y: point.1,
            },
            MONITOR_DEFAULTTONEAREST,
        );
        let (mut x, mut y) = (0, 0);
        if GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut x, &mut y).is_ok() {
            x.max(96)
        } else {
            96
        }
    }
}

pub struct Monitor {
    pub device: String,
    pub area: (i32, i32, i32, i32),
    pub dpi: u32,
}

fn monitor_info(monitor: HMONITOR) -> Option<Monitor> {
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    unsafe {
        if !GetMonitorInfoW(monitor, &mut info.monitorInfo).as_bool() {
            return None;
        }
    }
    let rect = info.monitorInfo.rcWork;
    let end = info
        .szDevice
        .iter()
        .position(|c| *c == 0)
        .unwrap_or(info.szDevice.len());
    let (mut x, mut y) = (96, 96);
    unsafe {
        let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut x, &mut y);
    }
    Some(Monitor {
        device: String::from_utf16_lossy(&info.szDevice[..end]),
        area: (rect.left, rect.top, rect.right, rect.bottom),
        dpi: x.max(96),
    })
}

pub fn monitor_at(point: (i32, i32)) -> Option<Monitor> {
    unsafe {
        monitor_info(MonitorFromPoint(
            POINT {
                x: point.0,
                y: point.1,
            },
            MONITOR_DEFAULTTONEAREST,
        ))
    }
}

pub fn saved_monitor(device: Option<&str>) -> Option<Monitor> {
    unsafe extern "system" fn collect(
        monitor: HMONITOR,
        _: HDC,
        _: *mut RECT,
        param: LPARAM,
    ) -> BOOL {
        if let Some(info) = monitor_info(monitor) {
            (*(param.0 as *mut Vec<Monitor>)).push(info);
        }
        BOOL(1)
    }
    let mut monitors = Vec::<Monitor>::new();
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(collect),
            LPARAM(&mut monitors as *mut _ as isize),
        );
    }
    monitors
        .into_iter()
        .find(|m| Some(m.device.as_str()) == device)
        .or_else(|| monitor_at(cursor().unwrap_or((0, 0))))
}

pub fn global_position(point: (i32, i32)) -> Option<crate::domain::GlobalPosition> {
    let monitor = monitor_at(point)?;
    Some(crate::domain::GlobalPosition {
        device: monitor.device,
        x_dip: crate::domain::px_to_dip(point.0 - monitor.area.0, monitor.dpi),
        y_dip: crate::domain::px_to_dip(point.1 - monitor.area.1, monitor.dpi),
    })
}

pub fn is_visible(raw: isize) -> bool {
    raw != 0 && unsafe { IsWindowVisible(hwnd(raw)).as_bool() }
}

pub fn is_minimized(raw: isize) -> bool {
    raw == 0 || unsafe { IsIconic(hwnd(raw)).as_bool() }
}

const OVERLAY_SUBCLASS: usize = 0x43424447;

unsafe extern "system" fn overlay_proc(
    window: HWND,
    msg: u32,
    w: WPARAM,
    l: LPARAM,
    id: usize,
    _: usize,
) -> LRESULT {
    match msg {
        WM_NCACTIVATE if GetWindowLongPtrW(window, GWL_STYLE) & WS_CHILD.0 as isize == 0 => {
            crate::diagnostics::record(format!(
                "overlay_message hwnd={} stage=ncactivate code={}",
                window.0 as isize, w.0
            ));
            // Preserve Tao's active/inactive bookkeeping and normal deactivation,
            // but do not let DefWindowProc paint a frame around transparent CSS.
            return DefSubclassProc(window, msg, w, LPARAM(-1));
        }
        WM_MOUSEACTIVATE | WM_POINTERACTIVATE => {
            let code = if msg == WM_MOUSEACTIVATE {
                MA_NOACTIVATE as u32
            } else {
                PA_NOACTIVATE
            };
            crate::diagnostics::record(format!(
                "overlay_message hwnd={} stage=mouseactivate code={code}",
                window.0 as isize
            ));
            return LRESULT(code as isize);
        }
        WM_NCDESTROY => {
            let _ = RemoveWindowSubclass(window, Some(overlay_proc), id);
        }
        _ => {}
    }
    DefSubclassProc(window, msg, w, l)
}

fn guard_activation(raw: isize) -> windows::core::Result<()> {
    if !unsafe { SetWindowSubclass(hwnd(raw), Some(overlay_proc), OVERLAY_SUBCLASS, 0).as_bool() } {
        return Err(windows::core::Error::from_win32());
    }
    Ok(())
}

// Install after ready as well: WebView's child HWNDs may not exist at construction.
pub fn guard_webview_children(raw: isize) -> windows::core::Result<()> {
    unsafe extern "system" fn collect(window: HWND, data: LPARAM) -> BOOL {
        (*(data.0 as *mut Vec<isize>)).push(window.0 as isize);
        BOOL(1)
    }
    let mut children = Vec::<isize>::new();
    unsafe {
        let _ = EnumChildWindows(
            Some(hwnd(raw)),
            Some(collect),
            LPARAM(&mut children as *mut _ as isize),
        );
    }
    for child in children {
        // SetWindowSubclass is deliberately limited to our own UI thread/process.
        if window_pid(child) == std::process::id()
            && unsafe { GetWindowThreadProcessId(hwnd(child), None) }
                == unsafe { windows::Win32::System::Threading::GetCurrentThreadId() }
        {
            guard_activation(child)?;
        }
    }
    Ok(())
}

pub fn make_nonactivating(raw: isize) -> windows::core::Result<()> {
    guard_activation(raw)?;
    unsafe {
        // Tao hides the nonclient layout but retains WS_CAPTION (including
        // WS_BORDER). DefWindowProc can still paint it on mouse activation.
        let frames =
            WS_CAPTION.0 | WS_THICKFRAME.0 | WS_SYSMENU.0 | WS_MINIMIZEBOX.0 | WS_MAXIMIZEBOX.0;
        let style = GetWindowLongPtrW(hwnd(raw), GWL_STYLE);
        SetWindowLongPtrW(
            hwnd(raw),
            GWL_STYLE,
            (style & !(frames as isize)) | WS_POPUP.0 as isize,
        );
        let edges = WS_EX_WINDOWEDGE.0
            | WS_EX_CLIENTEDGE.0
            | WS_EX_DLGMODALFRAME.0
            | WS_EX_STATICEDGE.0
            | WS_EX_APPWINDOW.0;
        let exstyle = GetWindowLongPtrW(hwnd(raw), GWL_EXSTYLE);
        SetWindowLongPtrW(
            hwnd(raw),
            GWL_EXSTYLE,
            (exstyle & !(edges as isize))
                | WS_EX_TOOLWINDOW.0 as isize
                | WS_EX_NOACTIVATE.0 as isize,
        );
        SetWindowPos(
            hwnd(raw),
            None,
            0,
            0,
            0,
            0,
            SWP_FRAMECHANGED | SWP_NOACTIVATE | SWP_NOZORDER | SWP_NOMOVE | SWP_NOSIZE,
        )?;
        if GetWindowLongPtrW(hwnd(raw), GWL_STYLE) & frames as isize != 0
            || GetWindowLongPtrW(hwnd(raw), GWL_EXSTYLE) & edges as isize != 0
        {
            return Err(windows::core::Error::from_hresult(windows::core::HRESULT(
                0x80004005u32 as i32,
            )));
        }
        // The browser menu can activate the HWND; an overlay has no native focus frame.
        let border: u32 = 0xfffffffe;
        let policy = DWMNCRP_DISABLED;
        let _ = DwmSetWindowAttribute(
            hwnd(raw),
            DWMWA_BORDER_COLOR,
            &border as *const _ as _,
            std::mem::size_of_val(&border) as u32,
        );
        let _ = DwmSetWindowAttribute(
            hwnd(raw),
            DWMWA_NCRENDERING_POLICY,
            &policy as *const _ as _,
            std::mem::size_of_val(&policy) as u32,
        );
    }
    Ok(())
}

pub fn is_window(raw: isize) -> bool {
    raw != 0 && unsafe { IsWindow(Some(hwnd(raw))).as_bool() }
}

pub fn bind_owner(raw: isize, owner: isize) {
    unsafe {
        if GetWindow(hwnd(raw), GW_OWNER)
            .ok()
            .map_or(0, |current| current.0 as isize)
            != owner
        {
            SetWindowLongPtrW(hwnd(raw), GWLP_HWNDPARENT, owner);
        }
    }
}

pub fn owner(raw: isize) -> isize {
    unsafe {
        GetWindow(hwnd(raw), GW_OWNER)
            .ok()
            .map_or(0, |current| current.0 as isize)
    }
}

pub fn is_topmost(raw: isize) -> bool {
    unsafe { GetWindowLongPtrW(hwnd(raw), GWL_EXSTYLE) & WS_EX_TOPMOST.0 as isize != 0 }
}

pub fn round_capsule(raw: isize, width: i32, height: i32) {
    // GDI region coordinates are physical pixels. Windows takes ownership on success.
    unsafe {
        if crate::diagnostics::no_native_region() {
            let cleared = SetWindowRgn(hwnd(raw), None, true) != 0;
            crate::diagnostics::record(format!(
                "capsule_region cleared={cleared} size={width}x{height}"
            ));
            return;
        }
        // Leave the CSS antialiased edge inside the native region. A matching
        // GDI curve clips fractional WebView pixels into a jagged edge.
        let curve = crate::domain::native_capsule_curve(height);
        let region = CreateRoundRectRgn(0, 0, width, height, curve, curve);
        let applied = SetWindowRgn(hwnd(raw), Some(region), true) != 0;
        if !applied {
            let _ = DeleteObject(HGDIOBJ(region.0));
        }
        crate::diagnostics::record(format!(
            "capsule_region applied={applied} size={width}x{height}"
        ));
    }
}

pub fn move_only(raw: isize, x: i32, y: i32) {
    unsafe {
        let _ = SetWindowPos(
            hwnd(raw),
            Some(HWND_TOP),
            x,
            y,
            0,
            0,
            SWP_NOACTIVATE | SWP_NOZORDER | SWP_NOSIZE | SWP_NOCOPYBITS,
        );
    }
}

pub fn show(raw: isize) {
    unsafe {
        let _ = ShowWindow(hwnd(raw), SW_SHOWNOACTIVATE);
    }
    crate::diagnostics::record(format!(
        "native_show hwnd={raw} visible={}",
        is_visible(raw)
    ));
}

// All visibility and z-order changes use Win32 on the UI thread. Tauri's
// cached visible flag stays false for these nonactivating windows, so its
// flag-changing APIs would otherwise reapply SW_HIDE during a mode switch.
pub fn set_topmost(raw: isize, enabled: bool) -> bool {
    let result = unsafe {
        SetWindowPos(
            hwnd(raw),
            Some(if enabled {
                HWND_TOPMOST
            } else {
                HWND_NOTOPMOST
            }),
            0,
            0,
            0,
            0,
            SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE | SWP_NOCOPYBITS,
        )
    };
    let observed = is_topmost(raw);
    if result.is_err() || observed != enabled {
        crate::diagnostics::record(format!(
            "native_topmost_failed hwnd={raw} desired={enabled} observed={observed} visible={} owner={} style={} window_style={} thread={}/{} code={}",
            is_visible(raw), owner(raw), unsafe { GetWindowLongPtrW(hwnd(raw), GWL_EXSTYLE) },
            unsafe { GetWindowLongPtrW(hwnd(raw), windows::Win32::UI::WindowsAndMessaging::GWL_STYLE) },
            unsafe { windows::Win32::System::Threading::GetCurrentThreadId() },
            unsafe { GetWindowThreadProcessId(hwnd(raw), None) },
            result.as_ref().err().map(|error| error.code().0).unwrap_or(0)
        ));
    }
    result.is_ok() && observed == enabled
}

pub fn raise(raw: isize) -> bool {
    unsafe {
        SetWindowPos(
            hwnd(raw),
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW,
        )
        .is_ok()
    }
}
pub fn hide(raw: isize) {
    unsafe {
        let _ = ShowWindow(hwnd(raw), SW_HIDE);
    }
}

pub fn client_size(raw: isize) -> Option<(i32, i32)> {
    let mut rect = RECT::default();
    unsafe { GetClientRect(hwnd(raw), &mut rect).ok()? };
    Some((rect.right - rect.left, rect.bottom - rect.top))
}

pub fn cursor_in_window(raw: isize) -> bool {
    let (Some((x, y)), Some(rect)) = (cursor(), window_rect(raw)) else {
        return false;
    };
    x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom
}

pub fn cursor_in_rect(x: i32, y: i32, width: i32, height: i32) -> bool {
    cursor().is_some_and(|(cx, cy)| cx >= x && cx < x + width && cy >= y && cy < y + height)
}

pub fn window_rect(raw: isize) -> Option<RECT> {
    let mut rect = RECT::default();
    unsafe { GetWindowRect(hwnd(raw), &mut rect).ok()? };
    Some(rect)
}

pub fn position(raw: isize) -> Option<(i32, i32)> {
    let rect = window_rect(raw)?;
    Some((rect.left, rect.top))
}

pub fn cursor() -> Option<(i32, i32)> {
    let mut point = POINT::default();
    unsafe {
        GetCursorPos(&mut point).ok()?;
    }
    Some((point.x, point.y))
}

pub fn clamp_to_work_area(
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    cursor: (i32, i32),
) -> (i32, i32) {
    unsafe {
        let monitor = MonitorFromPoint(
            POINT {
                x: cursor.0,
                y: cursor.1,
            },
            MONITOR_DEFAULTTONEAREST,
        );
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(monitor, &mut info).as_bool() {
            return crate::domain::clamp_to_work_area(
                (x, y),
                (width, height),
                (
                    info.rcWork.left,
                    info.rcWork.top,
                    info.rcWork.right,
                    info.rcWork.bottom,
                ),
            );
        }
    }
    (x, y)
}

pub fn left_mouse_down() -> bool {
    unsafe { (GetAsyncKeyState(VK_LBUTTON.0 as i32) as u16 & 0x8000) != 0 }
}

fn frame_bounds(raw: isize) -> Option<RECT> {
    let mut rect = RECT::default();
    unsafe {
        if DwmGetWindowAttribute(
            hwnd(raw),
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut rect as *mut _ as *mut c_void,
            std::mem::size_of::<RECT>() as u32,
        )
        .is_ok()
        {
            return Some(rect);
        }
        GetWindowRect(hwnd(raw), &mut rect).ok()?;
    }
    Some(rect)
}

fn is_codex_process(pid: u32) -> bool {
    let Ok(process) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) })
    else {
        return false;
    };
    let mut buffer = [0u16; 1024];
    let mut length = buffer.len() as u32;
    let result = unsafe {
        QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut length,
        )
    };
    unsafe {
        let _ = CloseHandle(process);
    }
    if result.is_err() {
        return false;
    }
    let path = String::from_utf16_lossy(&buffer[..length as usize]);
    let name = Path::new(&path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    name.eq_ignore_ascii_case("Codex")
        || (name.eq_ignore_ascii_case("ChatGPT")
            && (path.contains("OpenAI.Codex_") || path.contains("\\OpenAI\\Codex\\")))
}

unsafe extern "system" fn enumerate(hwnd: HWND, param: LPARAM) -> BOOL {
    let found = &mut *(param.0 as *mut Vec<isize>);
    if !IsWindowVisible(hwnd).as_bool() {
        return BOOL(1);
    }
    if GetWindowLongPtrW(hwnd, GWL_EXSTYLE) & WS_EX_TOOLWINDOW.0 as isize != 0 {
        return BOOL(1);
    }
    if GetWindow(hwnd, GW_OWNER).is_ok_and(|owner| owner.0 as isize != 0) {
        return BOOL(1);
    }
    // CLI/app-server codex.exe is not the Electron desktop main window.
    let mut class = [0u16; 128];
    let length = GetClassNameW(hwnd, &mut class) as usize;
    if String::from_utf16_lossy(&class[..length]) != "Chrome_WidgetWin_1" {
        return BOOL(1);
    }
    let mut pid = 0;
    GetWindowThreadProcessId(hwnd, Some(&mut pid));
    if pid == 0 || pid == std::process::id() || !is_codex_process(pid) {
        return BOOL(1);
    }
    let Some(rect) = frame_bounds(hwnd.0 as isize) else {
        return BOOL(1);
    };
    if IsIconic(hwnd).as_bool() || (rect.right - rect.left >= 280 && rect.bottom - rect.top >= 200)
    {
        found.push(hwnd.0 as isize);
    }
    BOOL(1)
}

pub fn find_codex_window(current: isize) -> isize {
    if std::env::args_os().any(|argument| argument == "--diagnose-no-codex") {
        return 0;
    }
    let mut found = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(enumerate), LPARAM(&mut found as *mut _ as isize));
    }
    let foreground = unsafe { GetForegroundWindow().0 as isize };
    if found.contains(&foreground) {
        foreground
    } else if found.contains(&current) {
        current
    } else {
        found.first().copied().unwrap_or(0)
    }
}

fn is_sidebar_container(frame: RECT, scale: f64, rect: uiautomation::types::Rect) -> bool {
    let left = rect.get_left() as f64;
    let right = rect.get_right() as f64;
    let bottom = rect.get_bottom() as f64;
    let width = right - left;
    (left - frame.left as f64).abs() <= 8.0 * scale
        && width >= 200.0 * scale
        && right <= frame.right as f64 - 260.0 * scale
        && bottom >= frame.bottom as f64 - 8.0 * scale
        && bottom <= frame.bottom as f64 + 4.0 * scale
}

pub struct AnchorProbe {
    cached: Option<UIElement>,
    owner: isize,
    avatar: Option<UIElement>,
    column: Option<UIElement>,
    pub navigation: Option<crate::domain::Rect>,
    pub sidebar: Option<crate::domain::Rect>,
}

impl AnchorProbe {
    pub fn has_avatar(&self) -> bool {
        self.avatar.is_some()
    }
    pub fn new() -> Self {
        Self {
            cached: None,
            owner: 0,
            avatar: None,
            column: None,
            navigation: None,
            sidebar: None,
        }
    }

    pub fn read_cached(&mut self, owner: isize) -> Option<(i32, i32)> {
        if self.owner != owner {
            self.owner = owner;
            self.cached = None;
            self.avatar = None;
            self.column = None;
            self.navigation = None;
            self.sidebar = None;
        }
        let frame = frame_bounds(owner)?;
        let scale = dpi(owner) as f64 / 96.0;
        if let (Some(avatar), Some(column)) = (&self.avatar, &self.column) {
            self.sidebar = self
                .cached
                .as_ref()
                .and_then(|element| element.get_bounding_rectangle().ok())
                .filter(|rect| is_sidebar_container(frame, scale, *rect))
                .map(|rect| {
                    (
                        rect.get_left(),
                        rect.get_top(),
                        rect.get_right() - rect.get_left(),
                        rect.get_bottom() - rect.get_top(),
                    )
                });
            let avatar_rect = avatar.get_bounding_rectangle().ok()?;
            let column_rect = column.get_bounding_rectangle().ok()?;
            if avatar.is_offscreen().unwrap_or(true)
                || !is_avatar_column(frame, scale, avatar_rect, column_rect)
            {
                // The wide sidebar cache is geometry-only in avatar mode; do
                // not turn it into the legacy avatar anchor after invalidation.
                self.cached = None;
                self.avatar = None;
                self.column = None;
                self.navigation = None;
                self.sidebar = None;
                return None;
            }
            self.navigation = Some((
                column_rect.get_left(),
                column_rect.get_top(),
                column_rect.get_right() - column_rect.get_left(),
                column_rect.get_bottom() - column_rect.get_top(),
            ));
            return Some(profile_anchor_point(avatar_rect, dpi(owner)));
        }
        let rect = self.cached.as_ref()?.get_bounding_rectangle().ok()?;
        let valid = is_sidebar_container(frame, scale, rect);
        if !valid {
            self.cached = None;
            return None;
        }
        Some(legacy_anchor_point(rect, scale))
    }

    pub fn discover(&mut self, automation: &UIAutomation, owner: isize) -> Option<(i32, i32)> {
        self.owner = owner;
        self.cached = None;
        self.avatar = None;
        self.column = None;
        self.navigation = None;
        self.sidebar = None;
        if let Some((avatar, column, sidebar)) = avatar_anchor(automation, owner) {
            self.cached = sidebar;
            self.avatar = Some(avatar);
            self.column = Some(column);
            return self.read_cached(owner);
        }
        let (element, point) = voice_anchor(automation, owner)?;
        self.cached = Some(element);
        Some(point)
    }
}

fn profile_anchor_point(rect: uiautomation::types::Rect, dpi: u32) -> (i32, i32) {
    // Round the final HWND origin once, using the CSS frame's fractional physical width.
    // capsule_frame subtracts half its integer width from this point.
    let center = (rect.get_left() as f64 + rect.get_right() as f64) / 2.0;
    let half_css = 15.0 * dpi.max(96) as f64 / 96.0;
    (
        (center - half_css).round() as i32 + crate::domain::dip_to_px(30.0, dpi) / 2,
        rect.get_top(),
    )
}

fn is_avatar_column(
    frame: RECT,
    scale: f64,
    avatar: uiautomation::types::Rect,
    column: uiautomation::types::Rect,
) -> bool {
    let width = column.get_right() - column.get_left();
    (column.get_left() - frame.left).abs() as f64 <= 8.0 * scale
        && width as f64 >= 48.0 * scale
        && width as f64 <= 100.0 * scale
        && column.get_bottom() as f64 >= frame.bottom as f64 - 12.0 * scale
        && avatar.get_left() >= column.get_left()
        && avatar.get_right() <= column.get_right()
        && avatar.get_top() > column.get_top()
        && avatar.get_bottom() <= column.get_bottom()
        && (avatar.get_bottom() - avatar.get_top()) as f64 >= 24.0 * scale
        && (avatar.get_bottom() - avatar.get_top()) as f64 <= 64.0 * scale
}

fn avatar_anchor(
    automation: &UIAutomation,
    owner: isize,
) -> Option<(UIElement, UIElement, Option<UIElement>)> {
    let frame = frame_bounds(owner)?;
    let scale = dpi(owner) as f64 / 96.0;
    let root = automation.element_from_handle(Handle::from(owner)).ok()?;
    let walker = automation.get_raw_view_walker().ok()?;
    for name in [
        "打开个人资料菜单",
        "Open profile menu",
        "Open personal profile menu",
    ] {
        let condition = automation
            .create_property_condition(
                UIProperty::Name,
                name.into(),
                Some(PropertyConditionFlags::IgnoreCase),
            )
            .ok()?;
        for avatar in root.find_all(TreeScope::Descendants, &condition).ok()? {
            if avatar.is_offscreen().unwrap_or(true)
                || avatar.get_control_type().ok() != Some(ControlType::Button)
            {
                continue;
            }
            let avatar_rect = avatar.get_bounding_rectangle().ok()?;
            let mut parent = avatar.clone();
            let mut column = None;
            for _ in 0..8 {
                parent = match walker.get_parent(&parent) {
                    Ok(parent) => parent,
                    Err(_) => break,
                };
                if parent
                    .get_bounding_rectangle()
                    .is_ok_and(|rect| is_avatar_column(frame, scale, avatar_rect, rect))
                {
                    column.get_or_insert_with(|| parent.clone());
                }
                if column.is_some()
                    && parent
                        .get_bounding_rectangle()
                        .is_ok_and(|rect| is_sidebar_container(frame, scale, rect))
                {
                    return Some((avatar, column.unwrap(), Some(parent)));
                }
            }
            if let Some(column) = column {
                return Some((avatar, column, None));
            }
        }
    }
    None
}

fn legacy_anchor_point(rect: uiautomation::types::Rect, scale: f64) -> (i32, i32) {
    (
        rect.get_right() - (23.0 * scale).round() as i32,
        rect.get_bottom() + (3.0 * scale).round() as i32,
    )
}

fn voice_anchor(automation: &UIAutomation, owner: isize) -> Option<(UIElement, (i32, i32))> {
    let frame = frame_bounds(owner)?;
    let scale = dpi(owner) as f64 / 96.0;
    let root = automation.element_from_handle(Handle::from(owner)).ok()?;
    let chinese = automation
        .create_property_condition(
            UIProperty::Name,
            "语音".into(),
            Some(PropertyConditionFlags::All),
        )
        .ok()?;
    let english = automation
        .create_property_condition(
            UIProperty::Name,
            "voice".into(),
            Some(PropertyConditionFlags::All),
        )
        .ok()?;
    let condition = automation.create_or_condition(chinese, english).ok()?;
    let walker = automation.get_raw_view_walker().ok()?;
    let controls = root.find_all(TreeScope::Descendants, &condition).ok()?;
    for control in controls {
        let Ok(process_id) = control.get_process_id() else {
            continue;
        };
        if process_id == std::process::id() {
            continue;
        }
        let Ok(name) = control.get_name() else {
            continue;
        };
        if !name.contains("语音") && !name.to_ascii_lowercase().contains("voice") {
            continue;
        }
        let mut parent = control;
        let mut anchor = None;
        for _ in 0..8 {
            if let Ok(rect) = parent.get_bounding_rectangle() {
                if is_sidebar_container(frame, scale, rect) {
                    anchor = Some((parent.clone(), legacy_anchor_point(rect, scale)));
                }
            }
            let Ok(next) = walker.get_parent(&parent) else {
                break;
            };
            parent = next;
        }
        if anchor.is_some() {
            return anchor;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn odd_avatar_width_does_not_bias_the_badge_to_the_left_at_125_percent() {
        let point = profile_anchor_point(uiautomation::types::Rect::new(86, 1126, 131, 1171), 120);
        let frame = crate::domain::capsule_frame(
            &crate::domain::Settings::default(),
            Some(point),
            120,
            None,
        )
        .unwrap();
        // The WebView's 30 DIP frame is 37.5 physical pixels. UIA avatar center is 108.5.
        let rendered_center = frame.0 as f64 + 18.75;
        assert!((rendered_center - 108.5).abs() <= 0.5);
    }

    #[test]
    fn avatar_and_css_centers_agree_at_fractional_dpi() {
        let point = profile_anchor_point(uiautomation::types::Rect::new(86, 1126, 149, 1189), 168);
        let frame = crate::domain::capsule_frame(
            &crate::domain::Settings::default(),
            Some(point),
            168,
            None,
        )
        .unwrap();
        assert!((frame.0 as f64 + 26.25 - 117.5).abs() <= 0.5);
    }
    #[test]
    fn avatar_navigation_uses_real_column_and_rejects_chat_list_and_root() {
        let frame = RECT {
            left: 393,
            top: 144,
            right: 2055,
            bottom: 1233,
        };
        let avatar = uiautomation::types::Rect::new(403, 1179, 448, 1224);
        assert!(is_avatar_column(
            frame,
            1.25,
            avatar,
            uiautomation::types::Rect::new(393, 199, 458, 1229)
        ));
        for rect in [
            uiautomation::types::Rect::new(393, 199, 818, 1229),
            uiautomation::types::Rect::new(393, 144, 2055, 1233),
        ] {
            assert!(!is_avatar_column(frame, 1.25, avatar, rect));
        }
    }

    #[test]
    #[ignore = "requires the user's interactive desktop and running unified client"]
    fn live_unified_anchor_discovery_and_cache_agree() {
        use windows::Win32::UI::HiDpi::{
            SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT,
            DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        };
        struct Awareness(DPI_AWARENESS_CONTEXT);
        impl Drop for Awareness {
            fn drop(&mut self) {
                unsafe {
                    SetThreadDpiAwarenessContext(self.0);
                }
            }
        }
        // Rust's test runner has no Tauri DPI setup. Match the production
        // per-monitor awareness before comparing UIA and DWM physical pixels.
        let _awareness = Awareness(unsafe {
            SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)
        });
        let owner = find_codex_window(0);
        assert_ne!(owner, 0, "desktop client must be running");
        let automation = UIAutomation::new().unwrap();
        let mut probe = AnchorProbe::new();
        let point = probe.discover(&automation, owner).expect("avatar anchor");
        assert!(probe.avatar.is_some(), "must select the profile avatar");
        assert!(probe.navigation.is_some());
        assert_eq!(probe.read_cached(owner), Some(point));
        let rect = crate::domain::capsule_frame(
            &crate::domain::Settings::default(),
            Some(point),
            dpi(owner),
            None,
        )
        .unwrap();
        println!(
            "owner={owner} anchor={point:?} capsule={rect:?} dpi={}",
            dpi(owner)
        );
    }
    #[test]
    fn webview_child_guard_catches_a_child_that_does_not_forward_activation() {
        let _window_guard = WINDOW_TEST_LOCK.lock().unwrap();
        use windows::core::w;
        use windows::Win32::Foundation::HINSTANCE;
        use windows::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DestroyWindow, RegisterClassW, SendMessageW,
            WNDCLASSW, WS_CHILD,
        };
        unsafe extern "system" fn child_proc(
            window: HWND,
            msg: u32,
            w: WPARAM,
            l: LPARAM,
        ) -> LRESULT {
            if msg == WM_MOUSEACTIVATE {
                LRESULT(1)
            } else {
                DefWindowProcW(window, msg, w, l)
            }
        }
        unsafe {
            RegisterClassW(&WNDCLASSW {
                lpfnWndProc: Some(child_proc),
                hInstance: HINSTANCE::default(),
                lpszClassName: w!("BadgeChildActivationRegression"),
                ..Default::default()
            });
            let parent = CreateWindowExW(
                WS_EX_TOOLWINDOW,
                w!("STATIC"),
                w!(""),
                WS_POPUP,
                -100,
                -100,
                1,
                1,
                None,
                None,
                None,
                None,
            )
            .unwrap();
            make_nonactivating(parent.0 as isize).unwrap();
            let child = CreateWindowExW(
                Default::default(),
                w!("BadgeChildActivationRegression"),
                w!(""),
                WS_CHILD,
                0,
                0,
                1,
                1,
                Some(parent),
                None,
                None,
                None,
            )
            .unwrap();
            let before = SendMessageW(child, WM_MOUSEACTIVATE, Some(WPARAM(0)), Some(LPARAM(0))).0;
            guard_webview_children(parent.0 as isize).unwrap();
            let after = SendMessageW(child, WM_MOUSEACTIVATE, Some(WPARAM(0)), Some(LPARAM(0))).0;
            DestroyWindow(parent).unwrap();
            assert_eq!(
                before, 1,
                "parent-only NOACTIVATE must not masquerade as a child guard"
            );
            assert_eq!(after, 3);
        }
    }
    #[test]
    fn overlay_mouse_activation_is_blocked_before_the_underlying_proc() {
        let _window_guard = WINDOW_TEST_LOCK.lock().unwrap();
        use windows::core::w;
        use windows::Win32::Foundation::{HINSTANCE, LRESULT, WPARAM};
        use windows::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DestroyWindow, RegisterClassW, SendMessageW,
            WM_MOUSEACTIVATE, WNDCLASSW, WS_POPUP,
        };
        unsafe extern "system" fn activating_proc(
            window: HWND,
            msg: u32,
            w: WPARAM,
            l: LPARAM,
        ) -> LRESULT {
            if msg == WM_MOUSEACTIVATE {
                LRESULT(1)
            } else {
                DefWindowProcW(window, msg, w, l)
            }
        }
        unsafe {
            RegisterClassW(&WNDCLASSW {
                lpfnWndProc: Some(activating_proc),
                hInstance: HINSTANCE::default(),
                lpszClassName: w!("BadgeActivationRegression"),
                ..Default::default()
            });
            let window = CreateWindowExW(
                WS_EX_TOOLWINDOW,
                w!("BadgeActivationRegression"),
                w!(""),
                WS_POPUP,
                -100,
                -100,
                1,
                1,
                None,
                None,
                None,
                None,
            )
            .unwrap();
            make_nonactivating(window.0 as isize).unwrap();
            let result = SendMessageW(window, WM_MOUSEACTIVATE, Some(WPARAM(0)), Some(LPARAM(0))).0;
            DestroyWindow(window).unwrap();
            assert_eq!(
                result, 3,
                "must prevent activation while preserving the click"
            );
        }
    }

    #[test]
    fn overlay_style_has_no_native_caption_or_edge_after_initialization() {
        let _window_guard = WINDOW_TEST_LOCK.lock().unwrap();
        use windows::core::w;
        use windows::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, WS_CAPTION, WS_EX_WINDOWEDGE, WS_POPUP,
        };
        unsafe {
            let window = CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_WINDOWEDGE,
                w!("STATIC"),
                w!(""),
                WS_POPUP | WS_CAPTION,
                -100,
                -100,
                1,
                1,
                None,
                None,
                None,
                None,
            )
            .unwrap();
            make_nonactivating(window.0 as isize).unwrap();
            let style = GetWindowLongPtrW(window, GWL_STYLE);
            let exstyle = GetWindowLongPtrW(window, GWL_EXSTYLE);
            DestroyWindow(window).unwrap();
            assert_eq!(
                style & WS_CAPTION.0 as isize,
                0,
                "hidden caption still allows native borders to repaint"
            );
            assert_eq!(exstyle & WS_EX_WINDOWEDGE.0 as isize, 0);
            assert_ne!(exstyle & WS_EX_NOACTIVATE.0 as isize, 0);
        }
    }
    #[test]
    fn denied_identity_keeps_trusted_hwnd_but_missing_pid_is_absent() {
        let identity = ProcessIdentity { pid: 7, created: 9 };
        let denied =
            windows::core::Error::from_hresult(windows::core::HRESULT(0x80070005u32 as i32));
        let missing =
            windows::core::Error::from_hresult(windows::core::HRESULT(0x80070057u32 as i32));
        assert_eq!(
            trusted_observation(123, true, identity, Err(denied), true, false)
                .unwrap()
                .window,
            123
        );
        assert!(
            trusted_observation(
                0,
                false,
                identity,
                Err(windows::core::Error::from_hresult(windows::core::HRESULT(
                    0x80070005u32 as i32
                ))),
                false,
                false
            )
            .unwrap()
            .unknown
        );
        assert!(trusted_observation(0, false, identity, Err(missing), false, false).is_none());
    }

    #[test]
    fn native_topmost_toggle_preserves_visibility_without_activation() {
        let _window_guard = WINDOW_TEST_LOCK.lock().unwrap();
        use windows::core::w;
        use windows::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, WS_POPUP};
        let window = unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                w!("STATIC"),
                w!("Badge visibility regression"),
                WS_POPUP,
                -100,
                -100,
                1,
                1,
                None,
                None,
                None,
                None,
            )
        }
        .unwrap();
        let raw = window.0 as isize;
        let identity = process_identity(std::process::id()).unwrap();
        assert_eq!(
            trusted_observation(
                raw,
                window_pid(raw) == identity.pid,
                identity,
                process_identity(identity.pid),
                is_visible(raw),
                is_minimized(raw)
            )
            .unwrap()
            .window,
            raw
        );
        let observation = observe_window(raw).unwrap();
        assert_eq!(observation.pid, std::process::id());
        assert!(!observation.visible);
        assert_eq!(observation.client, (1, 1));
        assert!(set_topmost(raw, true));
        assert!(!is_visible(raw));
        unsafe {
            SetWindowLongPtrW(
                window,
                GWL_EXSTYLE,
                GetWindowLongPtrW(window, GWL_EXSTYLE) & !(WS_EX_TOPMOST.0 as isize),
            );
        }
        assert!(set_topmost(raw, true));
        assert!(set_topmost(raw, false));
        show(raw);
        let foreground = unsafe { GetForegroundWindow() };
        assert!(set_topmost(raw, true));
        assert!(is_visible(raw));
        assert!(is_topmost(raw));
        assert!(set_topmost(raw, false));
        assert!(is_visible(raw));
        assert!(!is_topmost(raw));
        assert_eq!(unsafe { GetForegroundWindow() }, foreground);
        unsafe {
            DestroyWindow(window).unwrap();
        }
    }

    #[test]
    fn sidebar_candidate_rejects_conversation_area() {
        let frame = RECT {
            left: 0,
            top: 0,
            right: 1200,
            bottom: 900,
        };
        assert!(is_sidebar_container(
            frame,
            1.0,
            uiautomation::types::Rect::new(0, 0, 344, 900)
        ));
        assert!(!is_sidebar_container(
            frame,
            1.0,
            uiautomation::types::Rect::new(397, 0, 900, 900)
        ));
    }

    #[test]
    fn expanded_sidebar_remains_recognizable_without_accepting_the_whole_host() {
        let frame = RECT {
            left: 0,
            top: 0,
            right: 2000,
            bottom: 1000,
        };
        assert!(is_sidebar_container(
            frame,
            1.25,
            uiautomation::types::Rect::new(0, 0, 1000, 1000)
        ));
        assert!(!is_sidebar_container(
            frame,
            1.25,
            uiautomation::types::Rect::new(0, 0, 2000, 1000)
        ));
        assert!(!is_sidebar_container(
            frame,
            1.25,
            uiautomation::types::Rect::new(1000, 0, 2000, 1000)
        ));
    }

    #[test]
    fn activation_state_is_forwarded_without_requesting_native_frame_repaint() {
        let _window_guard = WINDOW_TEST_LOCK.lock().unwrap();
        use std::sync::atomic::{AtomicIsize, Ordering};
        use windows::core::w;
        use windows::Win32::Foundation::HINSTANCE;
        use windows::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DestroyWindow, RegisterClassW, SendMessageW,
            WM_NCACTIVATE, WNDCLASSW,
        };
        static PARAM: AtomicIsize = AtomicIsize::new(99);
        static ACTIVE: AtomicIsize = AtomicIsize::new(99);
        static RETURN: AtomicIsize = AtomicIsize::new(1);
        unsafe extern "system" fn observing_proc(
            window: HWND,
            msg: u32,
            w: WPARAM,
            l: LPARAM,
        ) -> LRESULT {
            if msg == WM_NCACTIVATE {
                PARAM.store(l.0, Ordering::SeqCst);
                ACTIVE.store(w.0 as isize, Ordering::SeqCst);
                return LRESULT(RETURN.load(Ordering::SeqCst));
            }
            DefWindowProcW(window, msg, w, l)
        }
        unsafe {
            RegisterClassW(&WNDCLASSW {
                lpfnWndProc: Some(observing_proc),
                hInstance: HINSTANCE::default(),
                lpszClassName: w!("BadgeFramePaintRegression"),
                ..Default::default()
            });
            let window = CreateWindowExW(
                WS_EX_TOOLWINDOW,
                w!("BadgeFramePaintRegression"),
                w!(""),
                WS_POPUP,
                -100,
                -100,
                1,
                1,
                None,
                None,
                None,
                None,
            )
            .unwrap();
            make_nonactivating(window.0 as isize).unwrap();
            let mut results = Vec::new();
            for (active, returned) in [(0, 1), (1, 0)] {
                RETURN.store(returned, Ordering::SeqCst);
                let result =
                    SendMessageW(window, WM_NCACTIVATE, Some(WPARAM(active)), Some(LPARAM(0))).0;
                results.push((
                    result,
                    ACTIVE.load(Ordering::SeqCst),
                    PARAM.load(Ordering::SeqCst),
                ));
            }
            let child = CreateWindowExW(
                Default::default(),
                w!("BadgeFramePaintRegression"),
                w!(""),
                WS_CHILD,
                0,
                0,
                1,
                1,
                Some(window),
                None,
                None,
                None,
            )
            .unwrap();
            guard_activation(child.0 as isize).unwrap();
            let result = SendMessageW(child, WM_NCACTIVATE, Some(WPARAM(1)), Some(LPARAM(321))).0;
            let child_result = (
                result,
                ACTIVE.load(Ordering::SeqCst),
                PARAM.load(Ordering::SeqCst),
            );
            DestroyWindow(child).unwrap();
            DestroyWindow(window).unwrap();
            assert_eq!(
                results,
                [(1, 0, -1), (0, 1, -1)],
                "preserve activation state and downstream result while suppressing frame paint"
            );
            assert_eq!(
                child_result,
                (0, 1, 321),
                "child messages must pass through unchanged"
            );
        }
    }
}
