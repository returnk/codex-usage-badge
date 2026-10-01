use crate::{
    domain, native,
    tray::{Checks, TrayAction},
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant};
use windows::core::w;
use windows::Win32::Foundation::{
    COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM,
};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;

use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, ReleaseCapture, SetCapture, VK_ESCAPE, VK_LBUTTON,
};
use windows::Win32::UI::WindowsAndMessaging::*;

pub struct Open {
    pub point: (i32, i32),
    pub avoid: Option<domain::Rect>,
    pub settings: bool,
    pub tray_anchor: Option<domain::Rect>,
}
struct State {
    checks: Arc<Checks>,
    callback: Arc<dyn Fn(TrayAction) + Send + Sync>,
    active: Arc<AtomicBool>,
    settings: bool,
    anchor: (i32, i32),
    avoid: Option<domain::Rect>,
    tray_anchor: Option<domain::Rect>,
    dpi: u32,
    left_submenu: bool,
    hover: i32,
    pressed: i32,
    hover_since: Instant,
    opened: Instant,
    left: Option<Instant>,
    entered: bool,
    button_down: bool,
    tooltip: isize,

    tooltip_shown: bool,
}
pub fn create(
    owner: HWND,
    checks: Arc<Checks>,
    callback: Arc<dyn Fn(TrayAction) + Send + Sync>,
    active: Arc<AtomicBool>,
) -> windows::core::Result<isize> {
    unsafe {
        let instance = HINSTANCE(GetModuleHandleW(None)?.0);
        let class = w!("CodexBadgePopup");
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: class,
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            ..Default::default()
        });
        let window = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST | WS_EX_LAYERED,
            class,
            w!("Codex Badge Menu"),
            WS_POPUP,
            0,
            0,
            0,
            0,
            Some(owner),
            None,
            Some(instance),
            None,
        )?;
        let tooltip = match create_hint(window, checks.clone(), instance) {
            Ok(value) => value,
            Err(error) => {
                let _ = DestroyWindow(window);
                return Err(error);
            }
        };
        let state = Box::new(State {
            checks,
            callback,
            active,
            settings: false,
            anchor: (0, 0),
            avoid: None,
            tray_anchor: None,
            dpi: 96,
            left_submenu: false,
            hover: -1,
            pressed: -1,
            hover_since: Instant::now(),
            opened: Instant::now(),
            left: None,
            entered: false,
            button_down: false,
            tooltip,
            tooltip_shown: false,
        });
        SetWindowLongPtrW(window, GWLP_USERDATA, Box::into_raw(state) as isize);
        Ok(window.0 as isize)
    }
}
struct HintState {
    checks: Arc<Checks>,
    dpi: u32,
}
unsafe fn create_hint(
    owner: HWND,
    checks: Arc<Checks>,
    instance: HINSTANCE,
) -> windows::core::Result<isize> {
    let class = w!("CodexBadgeHint");
    RegisterClassW(&WNDCLASSW {
        lpfnWndProc: Some(hint_proc),
        hInstance: instance,
        lpszClassName: class,
        ..Default::default()
    });
    let window = CreateWindowExW(
        WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST | WS_EX_LAYERED | WS_EX_TRANSPARENT,
        class,
        w!("Codex Badge Hint"),
        WS_POPUP,
        0,
        0,
        0,
        0,
        Some(owner),
        None,
        Some(instance),
        None,
    )?;
    SetWindowLongPtrW(
        window,
        GWLP_USERDATA,
        Box::into_raw(Box::new(HintState { checks, dpi: 96 })) as isize,
    );
    Ok(window.0 as isize)
}
unsafe extern "system" fn hint_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let pointer = GetWindowLongPtrW(window, GWLP_USERDATA) as *mut HintState;
    match message {
        WM_NCDESTROY => {
            if !pointer.is_null() {
                SetWindowLongPtrW(window, GWLP_USERDATA, 0);
                drop(Box::from_raw(pointer));
            }
        }
        WM_MOUSEACTIVATE => return LRESULT(MA_NOACTIVATE as isize),
        WM_POINTERACTIVATE => return LRESULT(PA_NOACTIVATE as isize),
        WM_NCHITTEST => return LRESULT(HTTRANSPARENT as isize),
        WM_ERASEBKGND => return LRESULT(1),
        WM_PAINT if !pointer.is_null() => {
            let state = &*pointer;
            let mut ps = PAINTSTRUCT::default();
            let _ = BeginPaint(window, &mut ps);
            if let Some(position) = native::position(window.0 as isize) {
                paint_hint(window, state, position);
            }
            let _ = EndPaint(window, &ps);
            return LRESULT(0);
        }
        _ => {}
    }
    DefWindowProcW(window, message, wparam, lparam)
}
unsafe fn paint_hint(window: HWND, state: &HintState, position: (i32, i32)) -> bool {
    let p = |v: f64| domain::dip_to_px(v, state.dpi);
    let size = (p(270.0), p(52.0));
    render_layer(
        window,
        position,
        size,
        &[(0.0, 0.0, size.0 as f64, size.1 as f64)],
        p(15.0) as f64,
        |dc| {
            let client = RECT {
                left: 0,
                top: 0,
                right: size.0,
                bottom: size.1,
            };
            background(dc, client, state.checks.dark.load(Ordering::Relaxed));
            let dark = state.checks.dark.load(Ordering::Relaxed);
            let fg = COLORREF(if dark { 0x00b8afa8 } else { 0x00887766 });
            let p = |v: f64| domain::dip_to_px(v, state.dpi);
            text(
                dc,
                "额度偏低或恢复时显示 Windows 通知。",
                RECT {
                    left: p(12.0),
                    top: p(10.0),
                    right: client.right - p(12.0),
                    bottom: p(26.0),
                },
                11,
                fg,
                DT_LEFT,
                state.dpi,
            );
            text(
                dc,
                "关闭后，胶囊提示点仍然显示。",
                RECT {
                    left: p(12.0),
                    top: p(26.0),
                    right: client.right - p(12.0),
                    bottom: p(42.0),
                },
                11,
                fg,
                DT_LEFT,
                state.dpi,
            );
        },
    )
}
unsafe fn tip(window: HWND, state: &mut State, show: bool) {
    if show == state.tooltip_shown {
        return;
    }
    let hint = HWND(state.tooltip as _);
    if !show {
        let _ = ShowWindow(hint, SW_HIDE);
        state.tooltip_shown = false;
        return;
    }
    let mut rect = RECT::default();
    let _ = GetWindowRect(window, &mut rect);
    let Some(monitor) = native::monitor_at(state.anchor) else {
        return;
    };
    let p = |v: f64| domain::dip_to_px(v, state.dpi);
    let (width, height, gap) = (p(270.0), p(52.0), p(6.0));
    let x = if state.left_submenu {
        rect.left
    } else {
        rect.right - width
    };
    let y = if rect.bottom + gap + height <= monitor.area.3 {
        rect.bottom + gap
    } else {
        rect.top - gap - height
    };
    let point = domain::clamp_to_work_area((x, y), (width, height), monitor.area);
    if let Some(hint_state) = (GetWindowLongPtrW(hint, GWLP_USERDATA) as *mut HintState).as_mut() {
        hint_state.dpi = state.dpi;
    }
    if let Some(hint_state) = (GetWindowLongPtrW(hint, GWLP_USERDATA) as *const HintState).as_ref()
    {
        if paint_hint(hint, hint_state, point) {
            let _ = ShowWindow(hint, SW_SHOWNOACTIVATE);
            state.tooltip_shown = true;
        }
    }
}
pub fn show(raw: isize, open: Open) {
    unsafe {
        let window = HWND(raw as _);
        if let Some(state) = (GetWindowLongPtrW(window, GWLP_USERDATA) as *mut State).as_mut() {
            tip(window, state, false);
            state.settings = open.settings;
            state.anchor = open.point;
            state.avoid = open.avoid;
            state.tray_anchor = open.tray_anchor;
            state.dpi = native::dpi_at(open.point);
            state.opened = Instant::now();
            state.hover_since = Instant::now();
            state.hover = -1;
            state.pressed = -1;
            state.entered = false;
            state.left = None;
            state.button_down = GetAsyncKeyState(VK_LBUTTON.0 as i32) < 0;
            arrange(window, state);
            state.active.store(true, Ordering::Release);
            let _ = SetTimer(Some(window), 1, 40, None);
            let _ = InvalidateRect(Some(window), None, false);
        }
    }
}
pub fn refresh(raw: isize) {
    unsafe {
        let _ = InvalidateRect(Some(HWND(raw as _)), None, false);
    }
}
fn tray_position(
    anchor: domain::Rect,
    size: (i32, i32),
    area: domain::Rect,
    gap: i32,
) -> (i32, i32) {
    let left = (anchor.0 + anchor.2 - size.0, anchor.1 - size.1 - gap);
    let right = (anchor.0, anchor.1 - size.1 - gap);
    let fits = |p: (i32, i32)| {
        p.0 >= area.0 && p.1 >= area.1 && p.0 + size.0 <= area.2 && p.1 + size.1 <= area.3
    };
    if fits(left) {
        left
    } else if fits(right) {
        right
    } else {
        domain::clamp_to_work_area(left, size, area)
    }
}
fn submenu_left(root: (i32, i32), width: i32, gap: i32, area: domain::Rect) -> bool {
    root.0 + width * 2 + gap > area.2
}
fn submenu_clearance(
    root: (i32, i32),
    size: (i32, i32),
    area: domain::Rect,
    cap: domain::Rect,
    gap: i32,
    child_offset: i32,
) -> (i32, i32) {
    let child_x = if submenu_left(root, size.0, child_offset - size.0, area) {
        root.0 - child_offset
    } else {
        root.0 + child_offset
    };
    if child_x < cap.0 + cap.2
        && child_x + size.0 > cap.0
        && root.1 < cap.1 + cap.3
        && root.1 + size.1 > cap.1
    {
        let above = cap.1 - size.1 - gap;
        if above >= area.1 {
            return (root.0, above);
        }
        let below = cap.1 + cap.3 + gap;
        if below + size.1 <= area.3 {
            return (root.0, below);
        }
    }
    root
}
unsafe fn arrange(window: HWND, state: &mut State) {
    let Some(monitor) = native::monitor_at(state.anchor) else {
        return;
    };
    let p = |v: f64| domain::dip_to_px(v, state.dpi);
    let size = (p(180.0), p(118.0));
    let root = state
        .tray_anchor
        .map(|a| tray_position(a, size, monitor.area, p(6.0)))
        .or_else(|| {
            state
                .avoid
                .and_then(|cap| domain::place_popup(cap, size, monitor.area, &[cap], p(6.0)))
                .map(|r| (r.0, r.1))
        })
        .unwrap_or_else(|| {
            tray_position(
                (state.anchor.0, state.anchor.1, 1, 1),
                size,
                monitor.area,
                p(6.0),
            )
        });
    // Reserve child clearance while collapsed so opening never moves the root.
    let root = state.avoid.map_or(root, |cap| {
        submenu_clearance(root, size, monitor.area, cap, p(6.0), p(184.0))
    });
    state.left_submenu = state.settings && submenu_left(root, size.0, p(4.0), monitor.area);
    let root_x = if state.left_submenu { p(184.0) } else { 0 };
    let x = root.0 - root_x;
    let width = if state.settings { p(364.0) } else { size.0 };
    if paint_menu(window, state, (x, root.1), (width, size.1)) {
        native::raise(window.0 as isize);
    }
}
// Right-facing coordinates; a left-facing menu is mapped to this layout before hit testing.
fn hit(expanded: bool, x: f64, y: f64) -> i32 {
    if expanded && (192.0..356.0).contains(&x) {
        if (42.0..74.0).contains(&y) {
            3
        } else if (76.0..108.0).contains(&y) {
            4
        } else {
            -1
        }
    } else if (8.0..172.0).contains(&x) {
        if (8.0..40.0).contains(&y) {
            0
        } else if (42.0..74.0).contains(&y) {
            1
        } else if (80.0..112.0).contains(&y) {
            2
        } else {
            -1
        }
    } else {
        -1
    }
}
fn local_hit(state: &State, x: f64, y: f64) -> i32 {
    let x = if state.left_submenu {
        if x >= 184.0 {
            x - 184.0
        } else if x < 180.0 {
            x + 184.0
        } else {
            -1.0
        }
    } else {
        x
    };
    hit(state.settings, x, y)
}

fn inside_layout(expanded: bool, left: bool, x: f64, y: f64) -> bool {
    let root = if expanded && left { 184.0 } else { 0.0 };
    let child = if left { 0.0 } else { 184.0 };
    ((root..root + 180.0).contains(&x) && (0.0..118.0).contains(&y))
        || (expanded && ((child..child + 180.0).contains(&x) && (34.0..116.0).contains(&y)))
        || (expanded && (180.0..184.0).contains(&x) && (42.0..108.0).contains(&y))
}
unsafe fn hide(window: HWND, state: &mut State) {
    tip(window, state, false);
    let _ = ShowWindow(window, SW_HIDE);
    let _ = KillTimer(Some(window), 1);
    state.active.store(false, Ordering::Release);
    state.pressed = -1;
}
unsafe fn set_hover(window: HWND, state: &mut State, row: i32) {
    if row != state.hover {
        tip(window, state, false);
        state.hover = row;
        state.hover_since = Instant::now();
        if state.settings && (row == 0 || row == 2) {
            state.settings = false;
            arrange(window, state);
        }
        let _ = InvalidateRect(Some(window), None, false);
    }
}
unsafe extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let pointer = GetWindowLongPtrW(window, GWLP_USERDATA) as *mut State;
    if message == WM_NCDESTROY {
        if !pointer.is_null() {
            SetWindowLongPtrW(window, GWLP_USERDATA, 0);
            let state = Box::from_raw(pointer);
            state.active.store(false, Ordering::Release);
        }
    } else if message == WM_MOUSEACTIVATE {
        return LRESULT(MA_NOACTIVATE as isize);
    } else if message == WM_POINTERACTIVATE {
        return LRESULT(PA_NOACTIVATE as isize);
    } else if message == WM_ERASEBKGND {
        return LRESULT(1);
    } else if !matches!(
        message,
        WM_PAINT
            | WM_SETCURSOR
            | WM_MOUSEMOVE
            | WM_LBUTTONDOWN
            | WM_LBUTTONUP
            | WM_TIMER
            | WM_CLOSE
    ) {
        return DefWindowProcW(window, message, wparam, lparam);
    } else if let Some(state) = pointer.as_mut() {
        match message {
            WM_PAINT => {
                paint(window, state);
                return LRESULT(0);
            }
            WM_SETCURSOR => {
                if let Ok(cursor) = LoadCursorW(
                    None,
                    if state.hover >= 0 {
                        IDC_HAND
                    } else {
                        IDC_ARROW
                    },
                ) {
                    SetCursor(Some(cursor));
                }
                return LRESULT(1);
            }
            WM_MOUSEMOVE | WM_LBUTTONDOWN | WM_LBUTTONUP => {
                let scale = state.dpi as f64 / 96.0;
                // Queued client coordinates can describe the pre-expansion origin.
                let row = native::cursor()
                    .zip(native::position(window.0 as isize))
                    .map_or(-1, |(cursor, origin)| {
                        local_hit(
                            state,
                            (cursor.0 - origin.0) as f64 / scale,
                            (cursor.1 - origin.1) as f64 / scale,
                        )
                    });
                set_hover(window, state, row);
                if message == WM_LBUTTONDOWN {
                    state.pressed = row;
                    SetCapture(window);
                }
                if message == WM_LBUTTONUP {
                    let pressed = state.pressed;
                    state.pressed = -1;
                    let _ = ReleaseCapture();
                    if row >= 0 && row == pressed {
                        if row == 1 {
                            state.settings = true;
                            arrange(window, state);
                        } else {
                            let action = match row {
                                0 => TrayAction::Topmost,
                                2 => TrayAction::Exit,
                                3 => TrayAction::Startup,
                                _ => TrayAction::Notifications,
                            };
                            if row < 3 {
                                hide(window, state);
                            }
                            let callback = state.callback.clone();
                            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                callback(action)
                            }));
                        }
                    }
                }
                return LRESULT(0);
            }
            WM_TIMER => {
                let now = Instant::now();
                let cursor = native::cursor()
                    .zip(native::position(window.0 as isize))
                    .map(|(cursor, origin)| {
                        let scale = state.dpi as f64 / 96.0;
                        (
                            (cursor.0 - origin.0) as f64 / scale,
                            (cursor.1 - origin.1) as f64 / scale,
                        )
                    });
                let inside = cursor
                    .is_some_and(|(x, y)| inside_layout(state.settings, state.left_submenu, x, y));
                let row = if inside {
                    cursor.map_or(-1, |(x, y)| local_hit(state, x, y))
                } else {
                    -1
                };
                set_hover(window, state, row);
                let in_anchor = state
                    .avoid
                    .is_some_and(|r| native::cursor_in_rect(r.0, r.1, r.2, r.3));
                let down = GetAsyncKeyState(VK_LBUTTON.0 as i32) < 0;
                if (down && !state.button_down && !inside)
                    || GetAsyncKeyState(VK_ESCAPE.0 as i32) < 0
                {
                    hide(window, state);
                    return LRESULT(0);
                }
                if inside || in_anchor {
                    state.entered |= inside;
                    state.left = None;
                } else if state.entered
                    || now.duration_since(state.opened) >= Duration::from_millis(1200)
                {
                    let left = state.left.get_or_insert(now);
                    if now.duration_since(*left) >= Duration::from_millis(300) {
                        hide(window, state);
                        return LRESULT(0);
                    }
                }
                if !inside {
                    set_hover(window, state, -1);
                }
                if state.hover == 1
                    && !state.settings
                    && now.duration_since(state.hover_since) >= Duration::from_millis(200)
                {
                    state.settings = true;
                    arrange(window, state);
                }
                if state.hover == 4
                    && inside
                    && now.duration_since(state.hover_since) >= Duration::from_millis(500)
                {
                    tip(window, state, true);
                }
                state.button_down = down;
                return LRESULT(0);
            }
            WM_CLOSE => {
                hide(window, state);
                return LRESULT(0);
            }
            _ => {}
        }
    }
    DefWindowProcW(window, message, wparam, lparam)
}
unsafe fn text(
    dc: HDC,
    value: &str,
    rect: RECT,
    size: i32,
    color: COLORREF,
    align: DRAW_TEXT_FORMAT,
    dpi: u32,
) {
    let font = CreateFontW(
        -domain::dip_to_px(size as f64, dpi),
        0,
        0,
        0,
        400,
        0,
        0,
        0,
        DEFAULT_CHARSET,
        OUT_DEFAULT_PRECIS,
        CLIP_DEFAULT_PRECIS,
        CLEARTYPE_QUALITY,
        0,
        w!("Segoe UI"),
    );
    let previous = SelectObject(dc, HGDIOBJ(font.0));
    SetTextColor(dc, color);
    SetBkMode(dc, TRANSPARENT);
    let mut rect = rect;
    let mut value: Vec<u16> = value.encode_utf16().collect();
    DrawTextW(
        dc,
        &mut value,
        &mut rect,
        align | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
    );
    SelectObject(dc, previous);
    let _ = DeleteObject(HGDIOBJ(font.0));
}
unsafe fn background(dc: HDC, rect: RECT, dark: bool) {
    let colors = if dark {
        [(46u16, 50u16, 58u16), (34, 36, 41)]
    } else {
        [(255u16, 255u16, 255u16), (226, 238, 252)]
    };
    let vertices = [
        TRIVERTEX {
            x: rect.left,
            y: rect.top,
            Red: colors[0].0 << 8,
            Green: colors[0].1 << 8,
            Blue: colors[0].2 << 8,
            Alpha: 0,
        },
        TRIVERTEX {
            x: rect.right,
            y: rect.bottom,
            Red: colors[1].0 << 8,
            Green: colors[1].1 << 8,
            Blue: colors[1].2 << 8,
            Alpha: 0,
        },
    ];
    let mesh = GRADIENT_RECT {
        UpperLeft: 0,
        LowerRight: 1,
    };
    let _ = GradientFill(
        dc,
        &vertices,
        (&mesh as *const GRADIENT_RECT).cast(),
        1,
        GRADIENT_FILL_RECT_V,
    );
}
unsafe fn paint(window: HWND, state: &State) {
    let mut ps = PAINTSTRUCT::default();
    let _ = BeginPaint(window, &mut ps);
    if let (Some(position), Some(size)) = (
        native::position(window.0 as isize),
        native::client_size(window.0 as isize),
    ) {
        if size.0 > 0 && size.1 > 0 {
            paint_menu(window, state, position, size);
        }
    }
    let _ = EndPaint(window, &ps);
}
unsafe fn paint_menu(window: HWND, state: &State, position: (i32, i32), size: (i32, i32)) -> bool {
    let panes = menu_panes(state.dpi, state.settings, state.left_submenu);
    render_layer(
        window,
        position,
        size,
        &panes,
        15.0 * state.dpi as f64 / 96.0,
        |dc| draw_menu(dc, state),
    )
}
unsafe fn draw_menu(dc: HDC, state: &State) {
    let dark = state.checks.dark.load(Ordering::Relaxed);
    let fg = COLORREF(if dark { 0x00f4f1ed } else { 0x00463224 });
    let accent = COLORREF(if dark { 0x00ffbb77 } else { 0x00e77928 });
    let p = |v: f64| domain::dip_to_px(v, state.dpi);
    let rect = |x: f64, y: f64, w: f64, h: f64| RECT {
        left: p(x),
        top: p(y),
        right: p(x + w),
        bottom: p(y + h),
    };
    let root = if state.left_submenu { 184.0 } else { 0.0 };
    let child = if state.left_submenu { 0.0 } else { 184.0 };
    background(dc, rect(root, 0.0, 180.0, 118.0), dark);
    if state.settings {
        background(dc, rect(child, 34.0, 180.0, 82.0), dark);
    }
    let mut rows = vec![
        (
            0,
            rect(root + 8.0, 8.0, 164.0, 32.0),
            "置顶模式",
            Some(state.checks.topmost.load(Ordering::Relaxed)),
        ),
        (1, rect(root + 8.0, 42.0, 164.0, 32.0), "设置", None),
        (2, rect(root + 8.0, 80.0, 164.0, 32.0), "退出", None),
    ];
    if state.settings {
        rows.extend([
            (
                3,
                rect(child + 8.0, 42.0, 164.0, 32.0),
                "开机启动",
                Some(state.checks.startup.load(Ordering::Relaxed)),
            ),
            (
                4,
                rect(child + 8.0, 76.0, 164.0, 32.0),
                "系统通知",
                Some(state.checks.notifications.load(Ordering::Relaxed)),
            ),
        ]);
    }
    for (index, row, label, checked) in rows {
        if state.hover == index || (index == 1 && state.settings) {
            let brush = CreateSolidBrush(COLORREF(if dark { 0x00443c35 } else { 0x00f9e9d9 }));
            let old_brush = SelectObject(dc, HGDIOBJ(brush.0));
            let old_pen = SelectObject(dc, GetStockObject(NULL_PEN));
            let _ = RoundRect(
                dc,
                row.left,
                row.top,
                row.right,
                row.bottom,
                p(12.0),
                p(12.0),
            );
            SelectObject(dc, old_brush);
            SelectObject(dc, old_pen);
            let _ = DeleteObject(HGDIOBJ(brush.0));
        }
        let mut label_rect = row;
        label_rect.left += p(10.0);
        label_rect.right -= p(10.0);
        text(dc, label, label_rect, 13, fg, DT_LEFT, state.dpi);
        if checked == Some(true) {
            text(dc, "✓", label_rect, 16, accent, DT_RIGHT, state.dpi);
        }
        if index == 1 {
            text(dc, "›", label_rect, 16, fg, DT_RIGHT, state.dpi);
        }
    }
    let separator = CreateSolidBrush(COLORREF(if dark { 0x00504840 } else { 0x00eddfd2 }));
    FillRect(dc, &rect(root + 16.0, 76.0, 148.0, 1.0), separator);
    let _ = DeleteObject(HGDIOBJ(separator.0));
}
fn menu_panes(dpi: u32, expanded: bool, left: bool) -> Vec<(f64, f64, f64, f64)> {
    let p = |v: f64| domain::dip_to_px(v, dpi) as f64;
    let root = if left { p(184.0) } else { 0.0 };
    let child = if left { 0.0 } else { p(184.0) };
    let mut panes = vec![(root, 0.0, p(180.0), p(118.0))];
    if expanded {
        panes.push((child, p(34.0), p(180.0), p(116.0) - p(34.0)));
    }
    panes
}
fn corner_alpha(x: i32, y: i32, pane: (f64, f64, f64, f64), radius: f64) -> u8 {
    if x as f64 >= pane.0 + pane.2
        || y as f64 >= pane.1 + pane.3
        || (x + 1) as f64 <= pane.0
        || (y + 1) as f64 <= pane.1
    {
        return 0;
    }
    let radius = radius.min(pane.2 / 2.0).min(pane.3 / 2.0);
    let cx = (x as f64 + 0.5).clamp(pane.0 + radius, pane.0 + pane.2 - radius);
    let cy = (y as f64 + 0.5).clamp(pane.1 + radius, pane.1 + pane.3 - radius);
    // One physical pixel of coverage blends into the desktop at every DPI.
    ((radius + 0.5 - (x as f64 + 0.5 - cx).hypot(y as f64 + 0.5 - cy)).clamp(0.0, 1.0) * 255.0)
        .round() as u8
}
unsafe fn render_layer(
    window: HWND,
    position: (i32, i32),
    size: (i32, i32),
    panes: &[(f64, f64, f64, f64)],
    radius: f64,
    draw: impl FnOnce(HDC),
) -> bool {
    if size.0 <= 0 || size.1 <= 0 {
        return false;
    }
    let dc = CreateCompatibleDC(None);
    if dc.is_invalid() {
        return false;
    }
    let mut bits = std::ptr::null_mut();
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: size.0,
            biHeight: -size.1,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let bitmap = match CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0) {
        Ok(bitmap) => bitmap,
        Err(_) => {
            let _ = DeleteDC(dc);
            return false;
        }
    };
    let old = SelectObject(dc, HGDIOBJ(bitmap.0));
    std::ptr::write_bytes(bits, 0, (size.0 as usize) * (size.1 as usize) * 4);
    draw(dc);
    // Flush GDI's batched writes before reading the DIB directly.
    let _ = GdiFlush();
    let pixels =
        std::slice::from_raw_parts_mut(bits as *mut u8, (size.0 as usize) * (size.1 as usize) * 4);
    for y in 0..size.1 {
        for x in 0..size.0 {
            let alpha = panes
                .iter()
                .map(|pane| corner_alpha(x, y, *pane, radius))
                .max()
                .unwrap_or(0);
            let offset = ((y * size.0 + x) * 4) as usize;
            for channel in 0..3 {
                pixels[offset + channel] =
                    ((pixels[offset + channel] as u16 * alpha as u16 + 127) / 255) as u8;
            }
            pixels[offset + 3] = alpha;
        }
    }
    let blend = BLENDFUNCTION {
        BlendOp: AC_SRC_OVER as u8,
        BlendFlags: 0,
        SourceConstantAlpha: 255,
        AlphaFormat: AC_SRC_ALPHA as u8,
    };
    let result = UpdateLayeredWindow(
        window,
        None,
        Some(&POINT {
            x: position.0,
            y: position.1,
        }),
        Some(&SIZE {
            cx: size.0,
            cy: size.1,
        }),
        Some(dc),
        Some(&POINT { x: 0, y: 0 }),
        COLORREF(0),
        Some(&blend),
        ULW_ALPHA,
    );
    SelectObject(dc, old);
    let _ = DeleteObject(HGDIOBJ(bitmap.0));
    let _ = DeleteDC(dc);
    if let Err(error) = &result {
        crate::diagnostics::record(format!("popup_render_failed code={}", error.code().0));
    }
    result.is_ok()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn layered_frames_commit_position_and_size_without_activation() {
        unsafe {
            let foreground = GetForegroundWindow();
            let window = HWND(
                create(
                    HWND::default(),
                    Arc::new(Checks::default()),
                    Arc::new(|_| {}),
                    Arc::new(AtomicBool::new(false)),
                )
                .unwrap() as _,
            );
            for index in 0..20 {
                let size = if index % 2 == 0 {
                    (225, 148)
                } else {
                    (455, 148)
                };
                let point = (100 + index, 100);
                assert!(render_layer(
                    window,
                    point,
                    size,
                    &[(0.0, 0.0, 225.0, 148.0)],
                    18.75,
                    |dc| {
                        background(
                            dc,
                            RECT {
                                left: 0,
                                top: 0,
                                right: 225,
                                bottom: 148,
                            },
                            false,
                        );
                    }
                ));
                assert_eq!(native::position(window.0 as isize), Some(point));
                assert_eq!(native::client_size(window.0 as isize), Some(size));
                assert_eq!(GetForegroundWindow(), foreground);
            }
            DestroyWindow(window).unwrap();
        }
    }
    #[test]
    fn rounded_edge_has_partial_coverage_and_transparent_outside() {
        for dpi in [96, 120, 144, 192] {
            let scale = dpi as f64 / 96.0;
            let pane = (0.0, 0.0, 180.0 * scale, 118.0 * scale);
            assert_eq!(corner_alpha(0, 0, pane, 15.0 * scale), 0);
            assert_eq!(corner_alpha(90, 60, pane, 15.0 * scale), 255);
            assert!(
                (0..(15.0 * scale) as i32).any(|y| (0..(15.0 * scale) as i32).any(|x| {
                    let a = corner_alpha(x, y, pane, 15.0 * scale);
                    a > 0 && a < 255
                }))
            );
        }
    }
    #[test]
    fn opening_root_reserves_clearance_for_left_submenu() {
        assert_eq!(
            submenu_clearance(
                (2254, 832),
                (225, 148),
                (0, 0, 2560, 1380),
                (2156, 937, 90, 43),
                8,
                230
            ),
            (2254, 781)
        );
        assert_eq!(
            submenu_clearance(
                (100, 10),
                (180, 118),
                (0, 0, 1000, 800),
                (300, 40, 72, 34),
                6,
                184
            ),
            (100, 80)
        );
        assert_eq!(
            submenu_clearance(
                (100, 100),
                (180, 118),
                (0, 0, 1000, 800),
                (10, 400, 30, 30),
                6,
                184
            ),
            (100, 100)
        );
    }
    #[test]
    fn transparent_space_is_outside_but_the_menu_bridge_is_inside() {
        assert!(!inside_layout(true, false, 204.0, 20.0));
        assert!(!inside_layout(true, true, 20.0, 20.0));
        assert!(inside_layout(true, false, 182.0, 58.0));
        assert!(inside_layout(true, true, 182.0, 58.0));
    }
    #[test]
    fn submenu_mask_stops_at_the_painted_bottom_at_fractional_dpi() {
        for dpi in [96, 120, 144, 168, 192] {
            let child = menu_panes(dpi, true, false)[1];
            let bottom = domain::dip_to_px(116.0, dpi);
            assert_eq!(child.1 + child.3, bottom as f64);
            assert_eq!(
                corner_alpha(child.0 as i32 + 40, bottom, child, 15.0 * dpi as f64 / 96.0),
                0
            );
        }
    }
    #[test]
    fn settings_expands_beside_the_main_menu_instead_of_replacing_it() {
        assert_eq!(hit(true, 20.0, 20.0), 0);
        assert_eq!(hit(true, 204.0, 58.0), 3);
        assert_eq!(hit(true, 204.0, 95.0), 4);
        assert_eq!(hit(true, 182.0, 58.0), -1);
    }
    #[test]
    fn submenu_geometry_and_menu_gaps_are_consistent() {
        assert_eq!(hit(false, 20.0, 55.0), 1);
        assert_eq!(hit(false, 20.0, 77.0), -1);
        assert_eq!(hit(false, 204.0, 55.0), -1);
        assert_eq!(hit(true, 204.0, 75.0), -1);
        assert!(!submenu_left((100, 100), 180, 4, (0, 0, 1000, 800)));
        assert!(submenu_left((800, 100), 180, 4, (0, 0, 1000, 800)));
    }
    #[test]
    fn tray_menu_uses_icon_corner_and_flips_at_left_edge() {
        assert_eq!(
            tray_position((2082, 1258, 50, 50), (225, 148), (0, 0, 2560, 1380), 8),
            (1907, 1102)
        );
        assert_eq!(
            tray_position((10, 1258, 50, 50), (225, 148), (0, 0, 2560, 1380), 8),
            (10, 1102)
        );
    }
}
