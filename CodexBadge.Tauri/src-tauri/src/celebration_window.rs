use crate::{diagnostics, domain, epoch_now, native, settings, Shared};
use serde_json::{json, Value};
use std::{sync::Arc, thread, time::Duration};
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

#[derive(Default)]
pub struct Overlay {
    hwnd: isize,
    generation: u64,
    pending_session: Option<u64>,
}

fn initialize_native(window: &WebviewWindow, children: bool) -> Result<(), String> {
    let hwnd = window.hwnd().map_err(|e| e.to_string())?.0 as isize;
    let (sender, receiver) = std::sync::mpsc::channel();
    // SetWindowSubclass must run on the HWND's thread. Async commands run on a worker.
    window
        .app_handle()
        .run_on_main_thread(move || {
            if !children {
                native::hide(hwnd);
            }
            let result = native::make_nonactivating(hwnd)
                .and_then(|_| {
                    if children {
                        native::guard_webview_children(hwnd)
                    } else {
                        Ok(())
                    }
                })
                .map_err(|e| e.to_string());
            let _ = sender.send(result);
        })
        .map_err(|e| e.to_string())?;
    receiver
        .recv_timeout(Duration::from_secs(3))
        .map_err(|_| "animation window initialization timed out".to_string())?
}

pub fn hide(overlay: &mut Overlay) {
    native::hide(overlay.hwnd);
    overlay.generation = overlay.generation.wrapping_add(1);
    overlay.pending_session = None;
}

fn fresh(m: &crate::Model) -> bool {
    !m.quota_failed
        && m.account_key.is_some()
        && m.snapshot
            .as_ref()
            .is_some_and(|s| (0..=180).contains(&epoch_now().saturating_sub(s.fetched_at)))
}

fn frame(
    rect: (i32, i32, i32, i32),
    area: (i32, i32, i32, i32),
    dpi: u32,
) -> (i32, i32, i32, i32, f64, f64) {
    let w = domain::dip_to_px(600.0, dpi).min(area.2 - area.0).max(1);
    let h = domain::dip_to_px(450.0, dpi).min(area.3 - area.1).max(1);
    let center = ((rect.0 + rect.2) / 2, (rect.1 + rect.3) / 2);
    let (x, y) = domain::clamp_to_work_area((center.0 - w / 2, center.1 - h / 2), (w, h), area);
    (
        x,
        y,
        w,
        h,
        (center.0 - x) as f64 / w as f64,
        (center.1 - y) as f64 / h as f64,
    )
}

#[tauri::command]
pub async fn request_celebration(
    detail_session: u64,
    state: State<'_, Arc<Shared>>,
    window: WebviewWindow,
    app: AppHandle,
) -> Result<Option<&'static str>, String> {
    if window.label() != "detail" {
        return Ok(None);
    }
    {
        let mut m = state.inner.lock().unwrap();
        if !m.detail_visible || m.detail_session != detail_session || !m.ready[1] {
            return Ok(None);
        }
        if m.settings.celebration_state.welcome_done
            && (!fresh(&m) || m.settings.celebration_state.pending_reset.is_none())
        {
            return Ok(None);
        }
        if m.celebration_overlay.pending_session.is_some() {
            return Ok(None);
        }
        m.celebration_overlay.pending_session = Some(detail_session);
    }
    let built = (|| -> Result<WebviewWindow, String> {
        let overlay = if let Some(existing) = app.get_webview_window("celebration") {
            existing
        } else {
            WebviewWindowBuilder::new(
                &app,
                "celebration",
                WebviewUrl::App("celebration.html".into()),
            )
            .title("Codex Badge Celebration")
            .inner_size(600.0, 450.0)
            .transparent(true)
            .decorations(false)
            .shadow(false)
            .resizable(false)
            .skip_taskbar(true)
            .focusable(false)
            .focused(false)
            .visible(false)
            .build()
            .map_err(|e| e.to_string())?
        };
        initialize_native(&overlay, false)?;
        overlay
            .set_background_color(Some(tauri::window::Color(0, 0, 0, 0)))
            .map_err(|e| e.to_string())?;
        overlay
            .set_ignore_cursor_events(true)
            .map_err(|e| e.to_string())?;
        Ok(overlay)
    })();
    match built {
        Ok(overlay) => {
            state.inner.lock().unwrap().celebration_overlay.hwnd =
                overlay.hwnd().map_err(|e| e.to_string())?.0 as isize;
            let _ = overlay.emit("celebration-request", ());
            Ok(None)
        }
        Err(error) => {
            state
                .inner
                .lock()
                .unwrap()
                .celebration_overlay
                .pending_session = None;
            Err(error)
        }
    }
}

#[tauri::command]
pub async fn celebration_ready(
    state: State<'_, Arc<Shared>>,
    window: WebviewWindow,
) -> Result<Option<Value>, String> {
    if window.label() != "celebration" {
        return Ok(None);
    }
    initialize_native(&window, true)?;
    let shared = Arc::clone(state.inner());
    let app = window.app_handle().clone();
    let (sender, receiver) = std::sync::mpsc::channel();
    app.run_on_main_thread(move || {
        let _ = sender.send(show_ready(&shared, &window));
    })
    .map_err(|e| e.to_string())?;
    receiver
        .recv()
        .map_err(|_| "animation window closed".to_string())?
}

// Visibility and z-order operations execute on the UI thread; never wait for
// that thread while holding the model lock on an async worker.
fn show_ready(shared: &Arc<Shared>, window: &WebviewWindow) -> Result<Option<Value>, String> {
    let hwnd = window.hwnd().map_err(|e| e.to_string())?.0 as isize;
    let mut m = shared.inner.lock().unwrap();
    if m.celebration_overlay.hwnd != hwnd {
        return Ok(None);
    }
    let Some(session) = m.celebration_overlay.pending_session else {
        return Ok(None);
    };
    if !m.detail_visible
        || m.detail_session != session
        || (m.settings.celebration_state.welcome_done && !fresh(&m))
    {
        hide(&mut m.celebration_overlay);
        return Ok(None);
    }
    let rect = native::window_rect(m.windows[1]).ok_or("detail rectangle unavailable")?;
    let monitor = native::monitor_at(((rect.left + rect.right) / 2, (rect.top + rect.bottom) / 2))
        .ok_or("monitor unavailable")?;
    let (x, y, w, h, ox, oy) = frame(
        (rect.left, rect.top, rect.right, rect.bottom),
        monitor.area,
        monitor.dpi,
    );
    window
        .set_size(tauri::PhysicalSize::new(w as u32, h as u32))
        .map_err(|e| e.to_string())?;
    window
        .set_position(tauri::PhysicalPosition::new(x, y))
        .map_err(|e| e.to_string())?;
    native::bind_owner(hwnd, m.windows[1]);
    native::set_topmost(hwnd, m.settings.always_on_top);
    let before = m.settings.celebration_state.clone();
    let account = m.account_key;
    let Some(kind) = m.settings.celebration_state.take(account, epoch_now()) else {
        settings::save(&m.settings)?;
        hide(&mut m.celebration_overlay);
        return Ok(None);
    };
    if let Err(error) = settings::save(&m.settings) {
        m.settings.celebration_state = before;
        hide(&mut m.celebration_overlay);
        return Err(error);
    }
    m.celebration_overlay.pending_session = None;
    m.celebration_overlay.generation = m.celebration_overlay.generation.wrapping_add(1);
    let generation = m.celebration_overlay.generation;
    native::show(hwnd);
    let shared = Arc::clone(shared);
    let app = window.app_handle().clone();
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(2800));
        let _ = app.run_on_main_thread(move || {
            let mut m = shared.inner.lock().unwrap();
            if m.celebration_overlay.generation == generation {
                hide(&mut m.celebration_overlay);
            }
        });
    });
    diagnostics::record("celebration_play");
    Ok(Some(json!({"kind":kind,"origin":{"x":ox,"y":oy}})))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn outside_canvas_is_clamped_at_monitor_edges_and_retains_its_origin() {
        for dpi in [96, 120, 144, 168, 192] {
            for rect in [
                (0, 0, 300, 180),
                (1600, 900, 1920, 1080),
                (-1600, 0, -1300, 200),
            ] {
                let area = if rect.0 < 0 {
                    (-1600, 0, 0, 900)
                } else {
                    (0, 0, 1920, 1080)
                };
                let (x, y, w, h, ox, oy) = frame(rect, area, dpi);
                assert!(x >= area.0 && y >= area.1 && x + w <= area.2 && y + h <= area.3);
                assert!((0.0..=1.0).contains(&ox) && (0.0..=1.0).contains(&oy));
                assert_eq!(x + (ox * w as f64).round() as i32, (rect.0 + rect.2) / 2);
            }
        }
    }
}
