//! Explicit account consent. The quota worker retains account revalidation and
//! persistence; this window only returns the user's decision for one session.
use crate::{domain, native, tray, Shared};
use serde_json::{json, Value};
use std::sync::{mpsc, Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow, WindowEvent};

#[derive(Default)]
pub struct Controller(Mutex<Consent>);
#[derive(Default)]
struct Consent {
    session: u64,
    label: String,
    active: bool,
    busy: bool,
    error: Option<String>,
    reply: Option<mpsc::SyncSender<Result<bool, String>>>,
}
impl Consent {
    fn answer(&mut self, session: u64, accepted: bool) -> Result<(), String> {
        if !self.active
            || session != self.session
            || self.busy
            || (accepted && self.error.is_some())
        {
            return Err("此授权操作已结束或正在保存".into());
        }
        if let Some(reply) = self.reply.take() {
            reply.send(Ok(accepted)).map_err(|_| "授权操作已结束")?;
        } else if accepted || self.error.is_none() {
            return Err("请重新打开授权窗口".into());
        }
        self.busy = accepted;
        if !accepted {
            self.active = false;
        }
        Ok(())
    }
}

fn ensure_window(app: &AppHandle) -> Result<WebviewWindow, String> {
    if let Some(window) = app.get_webview_window("authorization") {
        return Ok(window);
    }
    let config = app
        .config()
        .app
        .windows
        .iter()
        .find(|w| w.label == "authorization")
        .ok_or("授权窗口配置缺失")?;
    let window = tauri::WebviewWindowBuilder::from_config(app, config)
        .map_err(|e| e.to_string())?
        .always_on_top(true)
        .focused(false)
        .build()
        .map_err(|e| e.to_string())?;
    window
        .set_background_color(Some(tauri::window::Color(0, 0, 0, 0)))
        .map_err(|e| e.to_string())?;
    let handle = app.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let session = handle.state::<Controller>().0.lock().unwrap().session;
            let _ = answer(&handle, session, false);
        }
    });
    Ok(window)
}

fn open(app: &AppHandle) -> Result<(), String> {
    let window = ensure_window(app)?;
    let point = native::cursor().unwrap_or((600, 450));
    let monitor = native::monitor_at(point).ok_or("无法取得显示区域")?;
    let p = |v| domain::dip_to_px(v, monitor.dpi);
    let (w, h) = (
        p(440.0).min(monitor.area.2 - monitor.area.0),
        p(500.0).min(monitor.area.3 - monitor.area.1),
    );
    let (x, y) =
        domain::clamp_to_work_area((point.0 - w / 2, point.1 - h / 2), (w, h), monitor.area);
    let raw = window.hwnd().map_err(|e| e.to_string())?.0 as isize;
    native::move_only(raw, x, y);
    window
        .set_size(tauri::PhysicalSize::new(w as u32, h as u32))
        .map_err(|e| e.to_string())?;
    // CSS draws the visible rounded edge; the native guard stays well outside it.
    native::round_surface(raw, w, h, p(14.0));
    app.emit_to("authorization", "authorization-state", ())
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Runs on the quota worker, just like the former blocking confirmation dialog.
pub fn confirm(app: &AppHandle, label: &str) -> Result<bool, String> {
    let (tx, rx) = mpsc::sync_channel(1);
    let handle = app.clone();
    let label = label.to_owned();
    app.run_on_main_thread(move || {
        let controller = handle.state::<Controller>();
        {
            let mut state = controller.0.lock().unwrap();
            if state.active {
                let _ = tx.send(Err("已有授权窗口，请先完成当前操作".into()));
                return;
            }
            state.session = state.session.wrapping_add(1).max(1);
            state.label = label;
            state.active = true;
            state.busy = false;
            state.error = None;
            state.reply = Some(tx);
        }
        if let Err(error) = open(&handle) {
            let mut state = controller.0.lock().unwrap();
            state.active = false;
            if let Some(reply) = state.reply.take() {
                let _ = reply.send(Err(error));
            }
        }
    })
    .map_err(|e| e.to_string())?;
    rx.recv().map_err(|_| "授权窗口已关闭".to_string())?
}

pub fn finish(app: &AppHandle, error: Option<String>) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let controller = handle.state::<Controller>();
        let failed = error.is_some();
        {
            let mut state = controller.0.lock().unwrap();
            state.busy = false;
            state.reply = None;
            if failed && !state.active {
                state.session = state.session.wrapping_add(1).max(1);
                state.label.clear();
            }
            state.active = failed;
            state.error = error;
        }
        if failed {
            if let Err(error) = open(&handle) {
                handle.state::<Controller>().0.lock().unwrap().active = false;
                crate::expiring_reset::show_error(&error);
                if let Some(tray) = handle.try_state::<tray::TrayHandle>() {
                    tray.authorization(false);
                }
            }
        } else {
            if let Some(window) = handle.get_webview_window("authorization") {
                let _ = window.hide();
            }
            if let Some(tray) = handle.try_state::<tray::TrayHandle>() {
                tray.authorization(false);
            }
        }
    });
}

#[tauri::command]
pub fn get_authorization_state(app: AppHandle, window: WebviewWindow) -> Result<Value, String> {
    if window.label() != "authorization" {
        return Err("窗口无效".into());
    }
    let controller = app.state::<Controller>();
    let state = controller.0.lock().unwrap();
    let shared = app.state::<Arc<Shared>>();
    let theme = shared.inner.lock().unwrap().settings.theme;
    Ok(
        json!({"session":state.session,"label":state.label,"busy":state.busy,"error":state.error,"theme":theme}),
    )
}

#[tauri::command]
pub fn authorization_ready(
    app: AppHandle,
    window: WebviewWindow,
    session: u64,
) -> Result<(), String> {
    if window.label() != "authorization" {
        return Err("窗口无效".into());
    }
    let controller = app.state::<Controller>();
    let state = controller.0.lock().unwrap();
    if state.active && state.session == session {
        if let Some(tray) = app.try_state::<tray::TrayHandle>() {
            tray.authorization(true);
        }
        if !window.is_visible().unwrap_or(false) {
            window.show().map_err(|e| e.to_string())?;
            window.set_focus().map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn answer(app: &AppHandle, session: u64, accepted: bool) -> Result<(), String> {
    app.state::<Controller>()
        .0
        .lock()
        .unwrap()
        .answer(session, accepted)?;
    if !accepted {
        if let Some(window) = app.get_webview_window("authorization") {
            let _ = window.hide();
        }
        // A failed save has no waiting worker; close its error view here.
        let shared = app.state::<Arc<Shared>>();
        if !shared
            .reset_authorize
            .load(std::sync::atomic::Ordering::Acquire)
        {
            if let Some(tray) = app.try_state::<tray::TrayHandle>() {
                tray.authorization(false);
            }
        }
    }
    let _ = app.emit_to("authorization", "authorization-state", ());
    Ok(())
}

#[tauri::command]
pub fn answer_authorization(
    app: AppHandle,
    window: WebviewWindow,
    session: u64,
    accepted: bool,
) -> Result<(), String> {
    if window.label() != "authorization" {
        return Err("窗口无效".into());
    }
    answer(&app, session, accepted)
}

#[test]
fn consent_requires_explicit_current_session_and_never_accepts_twice() {
    let (tx, rx) = mpsc::sync_channel(1);
    let mut state = Consent {
        session: 3,
        active: true,
        reply: Some(tx),
        ..Default::default()
    };
    assert!(state.answer(2, true).is_err());
    assert!(rx.try_recv().is_err());
    state.answer(3, true).unwrap();
    assert_eq!(rx.recv().unwrap().unwrap(), true);
    assert!(state.answer(3, true).is_err());
    assert!(state.answer(3, false).is_err());
    let (tx, rx) = mpsc::sync_channel(1);
    let mut state = Consent {
        session: 4,
        active: true,
        reply: Some(tx),
        ..Default::default()
    };
    state.answer(4, false).unwrap();
    assert_eq!(rx.recv().unwrap().unwrap(), false);
    assert!(!state.active);
    assert!(!state.busy);
    let mut error = Consent {
        session: 5,
        active: true,
        error: Some("未能保存授权".into()),
        ..Default::default()
    };
    assert!(error.answer(5, true).is_err());
    error.answer(5, false).unwrap();
    assert!(!error.active);
}
