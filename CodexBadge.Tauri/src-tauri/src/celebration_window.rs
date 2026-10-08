use crate::{diagnostics, domain, epoch_now, native, settings, Shared};
use serde_json::{json, Value};
use std::{sync::Arc, thread, time::Duration};
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

#[derive(Default)]
pub struct Overlay {
    hwnd: isize,
    generation: u64,
    pending_session: Option<u64>,
    playing: Option<Playing>,
}

struct Playing {
    id: u64,
    event: crate::celebration::Playback,
    account: Option<u64>,
    session: u64,
    acknowledged: bool,
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
    overlay.playing = None;
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
        let Some(event) = m
            .settings
            .celebration_state
            .peek(m.account_key, epoch_now())
        else {
            return Ok(None);
        };
        if event.kind == "reset" && !fresh(&m) {
            return Ok(None);
        }
        if m.celebration_overlay.pending_session.is_some()
            || m.celebration_overlay.playing.is_some()
        {
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
    if let Err(error) = initialize_native(&window, true) {
        hide(&mut state.inner.lock().unwrap().celebration_overlay);
        diagnostics::record("celebration_play_failed reason=initialize");
        return Err(error);
    }
    let shared = Arc::clone(state.inner());
    let app = window.app_handle().clone();
    let (sender, receiver) = std::sync::mpsc::channel();
    app.run_on_main_thread(move || {
        let result = show_ready(&shared, &window);
        if result.is_err() {
            hide(&mut shared.inner.lock().unwrap().celebration_overlay);
            diagnostics::record("celebration_play_failed reason=initialize");
        }
        let _ = sender.send(result);
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
    native::move_only(hwnd, x, y);
    window
        .set_size(tauri::PhysicalSize::new(w as u32, h as u32))
        .map_err(|e| e.to_string())?;
    native::bind_owner(hwnd, m.windows[1]);
    native::set_topmost(hwnd, m.settings.always_on_top);
    let account = m.account_key;
    let Some(event) = m.settings.celebration_state.peek(account, epoch_now()) else {
        hide(&mut m.celebration_overlay);
        return Ok(None);
    };
    m.celebration_overlay.pending_session = None;
    m.celebration_overlay.generation = m.celebration_overlay.generation.wrapping_add(1);
    let generation = m.celebration_overlay.generation;
    let kind = event.kind;
    m.celebration_overlay.playing = Some(Playing {
        id: generation,
        event,
        account,
        session,
        acknowledged: false,
    });
    native::show(hwnd);
    let shared = Arc::clone(shared);
    let app = window.app_handle().clone();
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(2800));
        let emitter = app.clone();
        let _ = app.run_on_main_thread(move || {
            let mut m = shared.inner.lock().unwrap();
            if m.celebration_overlay.generation == generation {
                if m.celebration_overlay
                    .playing
                    .as_ref()
                    .is_some_and(|p| !p.acknowledged)
                {
                    diagnostics::record("celebration_play_failed reason=timeout");
                }
                let acknowledged = m
                    .celebration_overlay
                    .playing
                    .as_ref()
                    .is_some_and(|p| p.acknowledged);
                hide(&mut m.celebration_overlay);
                drop(m);
                if acknowledged {
                    let _ = emitter.emit("state-updated", ());
                }
            }
        });
    });
    Ok(Some(
        json!({"kind":kind,"playId":generation,"origin":{"x":ox,"y":oy}}),
    ))
}

#[tauri::command]
pub fn celebration_started(
    play_id: u64,
    state: State<'_, Arc<Shared>>,
    window: WebviewWindow,
) -> Result<bool, String> {
    if window.label() != "celebration" {
        return Ok(false);
    }
    let mut m = state.inner.lock().unwrap();
    let Some(playing) = m
        .celebration_overlay
        .playing
        .as_ref()
        .filter(|p| p.id == play_id && !p.acknowledged)
    else {
        return Ok(false);
    };
    if !m.detail_visible
        || m.detail_session != playing.session
        || m.account_key != playing.account
        || !native::is_visible(m.celebration_overlay.hwnd)
    {
        return Ok(false);
    }
    let id = playing.event.id.clone();
    let account = playing.account;
    let mut saved = m.settings.clone();
    let acknowledged =
        m.settings
            .celebration_state
            .acknowledge(&id, account, epoch_now(), |next| {
                saved.celebration_state = next.clone();
                settings::save(&saved)
            });
    match acknowledged {
        Ok(true) => {
            if let Some(playing) = m.celebration_overlay.playing.as_mut() {
                playing.acknowledged = true;
            }
            diagnostics::record("celebration_play");
            Ok(true)
        }
        result => {
            hide(&mut m.celebration_overlay);
            diagnostics::record("celebration_play_failed reason=save");
            result
        }
    }
}

#[tauri::command]
pub fn celebration_failed(play_id: u64, state: State<'_, Arc<Shared>>, window: WebviewWindow) {
    if window.label() != "celebration" {
        return;
    }
    let mut m = state.inner.lock().unwrap();
    if m.celebration_overlay
        .playing
        .as_ref()
        .is_some_and(|p| p.id == play_id && !p.acknowledged)
    {
        hide(&mut m.celebration_overlay);
        diagnostics::record("celebration_play_failed reason=renderer");
    }
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
