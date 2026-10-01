use serde::Serialize;
use windows::core::w;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::ValidateRect;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT,
};
use windows::Win32::UI::Shell::{
    DefSubclassProc, GetWindowSubclass, RemoveWindowSubclass, SetWindowSubclass,
};
use windows::Win32::UI::WindowsAndMessaging::*;

const SIZE_SUBCLASS: usize = 0x4342494e;
// WM_MOUSELEAVE is grouped under optional UI::Controls in windows-rs.
const WM_MOUSELEAVE: u32 = 0x02a3;

#[derive(Clone, Serialize)]
pub struct Input {
    kind: &'static str,
    x: f64,
    y: f64,
    delta: i16,
}

struct State {
    callback: Box<dyn Fn(Input)>,
    buttons: Vec<[f64; 4]>,
    draggable: bool,
}

pub fn set_regions(parent: isize, buttons: Vec<[f64; 4]>, draggable: bool) {
    unsafe {
        let mut child = 0;
        if GetWindowSubclass(
            HWND(parent as _),
            Some(parent_proc),
            SIZE_SUBCLASS,
            Some(&mut child),
        )
        .as_bool()
        {
            let pointer = GetWindowLongPtrW(HWND(child as _), GWLP_USERDATA) as *mut State;
            if let Some(state) = pointer.as_mut() {
                state.buttons = buttons
                    .into_iter()
                    .filter(|r| r.iter().all(|v| v.is_finite()) && r[2] > 0.0 && r[3] > 0.0)
                    .collect();
                state.draggable = draggable;
            }
        }
    }
}

/// A paint-transparent native child receives clicks before Chromium can focus
/// its renderer. The webview continues drawing; actions go through the existing
/// frontend handlers. Must be created and destroyed on the parent's UI thread.
pub fn attach(parent: isize, callback: impl Fn(Input) + 'static) -> windows::core::Result<()> {
    unsafe {
        let parent = HWND(parent as _);
        let mut existing = 0usize;
        if GetWindowSubclass(
            parent,
            Some(parent_proc),
            SIZE_SUBCLASS,
            Some(&mut existing),
        )
        .as_bool()
            && IsWindow(Some(HWND(existing as _))).as_bool()
        {
            return Ok(());
        }
        let instance = HINSTANCE(GetModuleHandleW(None)?.0);
        let class = w!("CodexBadgeInput");
        let definition = WNDCLASSW {
            style: CS_DBLCLKS,
            lpfnWndProc: Some(input_proc),
            hInstance: instance,
            lpszClassName: class,
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            ..Default::default()
        };
        // Reusing an already registered class is expected for the other flyouts.
        RegisterClassW(&definition);
        let mut rect = RECT::default();
        GetClientRect(parent, &mut rect)?;
        let child = CreateWindowExW(
            WS_EX_TRANSPARENT | WS_EX_NOACTIVATE,
            class,
            w!("CodexBadge input"),
            WS_CHILD | WS_VISIBLE,
            0,
            0,
            rect.right,
            rect.bottom,
            Some(parent),
            None,
            Some(instance),
            None,
        )?;
        let state = Box::new(State {
            callback: Box::new(callback),
            buttons: Vec::new(),
            draggable: false,
        });
        SetWindowLongPtrW(child, GWLP_USERDATA, Box::into_raw(state) as isize);
        if !SetWindowSubclass(parent, Some(parent_proc), SIZE_SUBCLASS, child.0 as usize).as_bool()
        {
            let error = windows::core::Error::from_win32();
            let _ = DestroyWindow(child);
            return Err(error);
        }
        Ok(())
    }
}

unsafe extern "system" fn parent_proc(
    parent: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    id: usize,
    child: usize,
) -> LRESULT {
    if message == WM_SIZE {
        let mut rect = RECT::default();
        if GetClientRect(parent, &mut rect).is_ok() {
            let _ = SetWindowPos(
                HWND(child as _),
                Some(HWND_TOP),
                0,
                0,
                rect.right,
                rect.bottom,
                SWP_NOACTIVATE,
            );
        }
    } else if message == WM_NCDESTROY {
        let _ = RemoveWindowSubclass(parent, Some(parent_proc), id);
    }
    DefSubclassProc(parent, message, wparam, lparam)
}

unsafe extern "system" fn input_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let pointer = GetWindowLongPtrW(window, GWLP_USERDATA) as *mut State;
    if message == WM_NCDESTROY {
        if !pointer.is_null() {
            SetWindowLongPtrW(window, GWLP_USERDATA, 0);
            drop(Box::from_raw(pointer));
        }
    } else if message == WM_MOUSEACTIVATE {
        return LRESULT(MA_NOACTIVATE as isize);
    } else if message == WM_POINTERACTIVATE {
        return LRESULT(PA_NOACTIVATE as isize);
    } else if message == WM_ERASEBKGND {
        return LRESULT(1);
    } else if message == WM_PAINT {
        let _ = ValidateRect(Some(window), None);
        return LRESULT(0);
    } else if !pointer.is_null() {
        if message == WM_SETCURSOR {
            let mut point = windows::Win32::Foundation::POINT::default();
            let mut rect = RECT::default();
            let mut cursor = IDC_ARROW;
            if GetCursorPos(&mut point).is_ok() && GetWindowRect(window, &mut rect).is_ok() {
                let scale = GetDpiForWindow(window).max(96) as f64 / 96.0;
                let x = (point.x - rect.left) as f64 / scale;
                let y = (point.y - rect.top) as f64 / scale;
                if (*pointer).draggable {
                    cursor = IDC_SIZEALL;
                } else if (*pointer)
                    .buttons
                    .iter()
                    .any(|r| x >= r[0] && x < r[0] + r[2] && y >= r[1] && y < r[1] + r[3])
                {
                    cursor = IDC_HAND;
                }
            }
            if let Ok(cursor) = LoadCursorW(None, cursor) {
                SetCursor(Some(cursor));
            }
            return LRESULT(1);
        }
        let kind = match message {
            WM_MOUSEMOVE => {
                let mut tracking = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: window,
                    ..Default::default()
                };
                let _ = TrackMouseEvent(&mut tracking);
                Some("move")
            }
            WM_MOUSELEAVE => Some("leave"),
            WM_LBUTTONDOWN => {
                SetCapture(window);
                Some("down")
            }
            WM_LBUTTONUP => Some("up"),
            WM_LBUTTONDBLCLK => Some("double"),
            WM_RBUTTONUP => Some("right"),
            WM_MOUSEWHEEL => Some("wheel"),
            WM_CAPTURECHANGED => Some("cancel"),
            _ => None,
        };
        if let Some(kind) = kind {
            let scale = GetDpiForWindow(window).max(96) as f64 / 96.0;
            let mut x = lparam.0 as i16 as i32;
            let mut y = (lparam.0 >> 16) as i16 as i32;
            if message == WM_MOUSEWHEEL {
                let mut rect = RECT::default();
                if GetWindowRect(window, &mut rect).is_ok() {
                    x -= rect.left;
                    y -= rect.top;
                }
            }
            let input = Input {
                kind,
                x: x as f64 / scale,
                y: y as f64 / scale,
                delta: (wparam.0 >> 16) as i16,
            };
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                ((*pointer).callback)(input)
            }));
            if message == WM_LBUTTONUP {
                let _ = ReleaseCapture();
            }
            return LRESULT(0);
        }
    }
    DefWindowProcW(window, message, wparam, lparam)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, rc::Rc};

    #[test]
    fn input_is_nonactivating_reusable_resized_and_released_with_parent() {
        let _guard = crate::native::WINDOW_TEST_LOCK.lock().unwrap();
        unsafe {
            let parent = CreateWindowExW(
                WS_EX_NOACTIVATE,
                w!("STATIC"),
                w!("input fixture"),
                WS_POPUP,
                100,
                100,
                100,
                100,
                None,
                None,
                None,
                None,
            )
            .unwrap();
            let events = Rc::new(RefCell::new(Vec::new()));
            let captured = events.clone();
            attach(parent.0 as isize, move |input| {
                captured.borrow_mut().push(input)
            })
            .unwrap();
            let mut raw = 0;
            assert!(
                GetWindowSubclass(parent, Some(parent_proc), SIZE_SUBCLASS, Some(&mut raw))
                    .as_bool()
            );
            let child = HWND(raw as _);
            attach(parent.0 as isize, |_| {
                panic!("must reuse the first receiver")
            })
            .unwrap();
            assert_eq!(
                SendMessageW(child, WM_MOUSEACTIVATE, None, None).0,
                MA_NOACTIVATE as isize
            );
            let point = LPARAM((20 << 16) | 10);
            SendMessageW(child, WM_LBUTTONDOWN, None, Some(point));
            SendMessageW(child, WM_LBUTTONUP, None, Some(point));
            assert_eq!(
                events
                    .borrow()
                    .iter()
                    .filter(|e| e.kind != "cancel")
                    .map(|e| e.kind)
                    .collect::<Vec<_>>(),
                ["down", "up"]
            );
            let scale = GetDpiForWindow(child).max(96) as f64 / 96.0;
            assert_eq!(events.borrow()[0].x, 10.0 / scale);
            assert_eq!(events.borrow()[0].y, 20.0 / scale);
            SetWindowPos(
                parent,
                None,
                100,
                100,
                120,
                80,
                SWP_NOACTIVATE | SWP_NOZORDER,
            )
            .unwrap();
            let mut rect = RECT::default();
            GetClientRect(child, &mut rect).unwrap();
            assert_eq!((rect.right, rect.bottom), (120, 80));
            DestroyWindow(parent).unwrap();
            assert!(!IsWindow(Some(child)).as_bool());
            assert_eq!(
                Rc::strong_count(&events),
                1,
                "receiver is released with the parent"
            );
        }
    }
}
