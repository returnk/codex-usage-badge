//! User-opened update popover. Its state and lifecycle never resize quota windows.
use crate::{domain, native, updates, Shared};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow, WindowEvent};

#[derive(Default)]
pub struct Controller(pub Mutex<UiState>);
#[derive(Default)]
pub struct UiState {
    pub known_version: Option<String>,
    pub session: u64,
    pub visible: bool,
    pub shown_session: u64,
    pub checking: bool,
    pub result: Option<Value>,
    pub error: Option<String>,
    pub installing: bool,
    pub stage: &'static str,
    pub downloaded: u64,
    pub total: Option<u64>,
    pub dragging: bool,
    pub drag_id: u64,
    pub drag_origin: Option<((i32, i32), f64)>,
}

impl Controller {
    pub fn load() -> Self {
        Self(Mutex::new(UiState {
            known_version: updates::load_notice(),
            ..UiState::default()
        }))
    }
}
pub fn notice(app: &AppHandle) -> bool {
    app.try_state::<Controller>().is_some_and(|c| {
        updates::has_update_notice(
            c.0.lock().unwrap().known_version.as_deref(),
            env!("CARGO_PKG_VERSION"),
        )
    })
}
impl UiState {
    fn hide_on_blur(&self) -> bool {
        self.visible && self.shown_session == self.session && !self.dragging
    }
    fn begin(&mut self) -> bool {
        self.session = self.session.wrapping_add(1).max(1);
        self.visible = true;
        self.dragging = false;
        self.drag_origin = None;
        let query = !self.checking && !self.installing;
        if query {
            self.checking = true;
            self.result = None;
            self.error = None;
            self.stage = "idle";
        }
        query
    }
    fn begin_install(&mut self) -> Result<String, String> {
        if !self.visible || self.checking || self.installing {
            return Err("更新尚未准备好".into());
        }
        let release = self.result.as_ref().ok_or("请先检查更新")?;
        if release["available"] != true {
            return Err("当前无需更新".into());
        }
        let next = release["version"]
            .as_str()
            .filter(|v| updates::has_update_notice(Some(v), env!("CARGO_PKG_VERSION")))
            .ok_or("版本无效")?
            .to_owned();
        self.installing = true;
        self.stage = "preparing";
        self.downloaded = 0;
        self.total = None;
        self.error = None;
        Ok(next)
    }
}
pub fn frame(point: (i32, i32), area: domain::Rect, dpi: u32) -> domain::Rect {
    let p = |v| domain::dip_to_px(v, dpi);
    let (w, h) = (p(340.0).min(area.2 - area.0), p(300.0).min(area.3 - area.1));
    let (x, y) = domain::clamp_to_work_area((point.0 + p(10.0), point.1 - h / 2), (w, h), area);
    (x, y, w, h)
}
fn clip(window: &WebviewWindow) {
    use windows::Win32::Graphics::Gdi::{CreateRoundRectRgn, DeleteObject, SetWindowRgn, HGDIOBJ};
    if let (Ok(hwnd), Ok(size)) = (window.hwnd(), window.inner_size()) {
        let radius = domain::dip_to_px(32.0, native::dpi(hwnd.0 as isize));
        unsafe {
            let region =
                CreateRoundRectRgn(0, 0, size.width as i32, size.height as i32, radius, radius);
            if SetWindowRgn(windows::Win32::Foundation::HWND(hwnd.0), Some(region), true) == 0 {
                let _ = DeleteObject(HGDIOBJ(region.0));
            }
        }
    }
}
fn ensure_window(app: &AppHandle) -> Result<WebviewWindow, String> {
    if let Some(window) = app.get_webview_window("update") {
        return Ok(window);
    }
    let config = app
        .config()
        .app
        .windows
        .iter()
        .find(|w| w.label == "update")
        .ok_or("update window config missing")?;
    let window = tauri::WebviewWindowBuilder::from_config(app, config)
        .map_err(|e| e.to_string())?
        .always_on_top(true)
        .focused(false)
        .build()
        .map_err(|e| e.to_string())?;
    window
        .set_background_color(Some(tauri::window::Color(0, 0, 0, 0)))
        .map_err(|e| e.to_string())?;
    window
        .with_webview(|view| unsafe {
            let _ = view
                .controller()
                .CoreWebView2()
                .and_then(|core| core.Settings())
                .and_then(|settings| settings.SetAreDefaultContextMenusEnabled(false));
        })
        .map_err(|e| e.to_string())?;
    let handle = app.clone();
    let event_window = window.clone();
    window.on_window_event(move |event| match event {
        WindowEvent::Focused(false) => {
            let controller = handle.state::<Controller>();
            let state = controller.0.lock().unwrap();
            let should_close = state.hide_on_blur();
            drop(state);
            if should_close {
                hide(&handle);
            }
        }
        WindowEvent::CloseRequested { api, .. } => {
            api.prevent_close();
            hide(&handle);
        }
        WindowEvent::Resized(_) => clip(&event_window),
        _ => {}
    });
    Ok(window)
}
fn hide(app: &AppHandle) {
    let controller = app.state::<Controller>();
    let mut state = controller.0.lock().unwrap();
    state.visible = false;
    state.dragging = false;
    state.drag_origin = None;
    drop(state);
    if let Some(window) = app.get_webview_window("update") {
        let _ = window.hide();
    }
}
fn start_query(app: &AppHandle) {
    let handle = app.clone();
    std::thread::spawn(move || {
        let result = updates::latest();
        let completion_app = handle.clone();
        let _ = handle.run_on_main_thread(move || complete(&completion_app, result));
    });
}
pub fn complete(app: &AppHandle, result: Result<Value, String>) {
    let controller = app.state::<Controller>();
    let mut state = controller.0.lock().unwrap();
    updates::remember_release(&mut state.known_version, &result, env!("CARGO_PKG_VERSION"));
    #[cfg(test)]
    let persist = std::env::var("BADGE_PREVIEW_UPDATE").as_deref() != Ok("1");
    #[cfg(not(test))]
    let persist = true;
    if persist {
        if let Err(error) = updates::save_notice(&state.known_version) {
            crate::diagnostics::record(format!("update_notice_save_failed {error}"));
        }
    }
    state.checking = false;
    match result {
        Ok(release) => {
            state.result = Some(release);
            state.error = None;
        }
        Err(error) => {
            state.result = None;
            state.error = Some(error);
        }
    }
    drop(state);
    let shared = app.state::<Arc<Shared>>();
    let preferences = shared.inner.lock().unwrap().settings.clone();
    crate::sync_tray(app, &preferences);
    let _ = app.emit_to("update", "update-state", ());
}
fn prepare(app: &AppHandle, point: (i32, i32)) -> Result<bool, String> {
    let window = ensure_window(app)?;
    let monitor = native::monitor_at(point).ok_or("update monitor unavailable")?;
    let (x, y, w, h) = frame(point, monitor.area, monitor.dpi);
    window.hide().map_err(|e| e.to_string())?;
    window
        .set_size(tauri::PhysicalSize::new(w as u32, h as u32))
        .map_err(|e| e.to_string())?;
    window
        .set_position(tauri::PhysicalPosition::new(x, y))
        .map_err(|e| e.to_string())?;
    clip(&window);
    let controller = app.state::<Controller>();
    let mut state = controller.0.lock().unwrap();
    let query = state.begin();
    drop(state);
    let _ = app.emit_to("update", "update-state", ());
    Ok(query)
}
pub fn open(app: &AppHandle) -> Result<(), String> {
    if prepare(app, native::cursor().unwrap_or((600, 450)))? {
        start_query(app);
    }
    Ok(())
}
#[cfg(test)]
pub fn preview(app: &AppHandle) -> Result<(), String> {
    prepare(app, (1450, 650))?;
    if std::env::var("BADGE_PREVIEW_UPDATE_LIVE").as_deref() == Ok("1") {
        start_query(app);
        return Ok(());
    }
    let handle = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(3));
        let completion = handle.clone();
        let _=handle.run_on_main_thread(move ||complete(&completion,Ok(json!({"available":true,"version":"0.3.3","tag":"v0.3.3","notes":"• 优化菜单与额度浮窗体验\n• 修复显示与定位细节"}))));
    });
    Ok(())
}
#[tauri::command]
pub fn get_update_state(app: AppHandle) -> Value {
    let shared = app.state::<Arc<Shared>>();
    let theme = shared.inner.lock().unwrap().settings.theme;
    let controller = app.state::<Controller>();
    let state = controller.0.lock().unwrap();
    json!({"theme":theme,"session":state.session,"checking":state.checking,"currentVersion":env!("CARGO_PKG_VERSION"),"release":state.result,"error":state.error,"installing":state.installing,"stage":state.stage,"downloaded":state.downloaded,"total":state.total,"installed":updates::is_registered_install()})
}
#[tauri::command]
pub fn update_ready(session: u64, app: AppHandle, window: WebviewWindow) -> Result<(), String> {
    if window.label() != "update" {
        return Err("invalid update window".into());
    }
    let controller = app.state::<Controller>();
    let mut state = controller.0.lock().unwrap();
    if !state.visible || state.session != session || state.shown_session == session {
        return Ok(());
    }
    state.shown_session = session;
    drop(state);
    window.show().map_err(|e| e.to_string())?;
    window.set_focus().map_err(|e| e.to_string())
}
#[tauri::command]
pub fn close_update(app: AppHandle, window: WebviewWindow) {
    if window.label() == "update" {
        hide(&app);
    }
}
#[tauri::command]
pub fn retry_update(app: AppHandle, window: WebviewWindow) {
    if window.label() != "update" {
        return;
    }
    let controller = app.state::<Controller>();
    let mut state = controller.0.lock().unwrap();
    if state.checking || state.installing || !state.visible {
        return;
    }
    state.checking = true;
    state.result = None;
    state.error = None;
    drop(state);
    let _ = app.emit_to("update", "update-state", ());
    start_query(&app);
}
fn validate_package(expected: &str, next: &str, url: &str) -> Result<(), String> {
    let prefix =
        format!("https://github.com/returnk/codex-usage-badge/releases/download/v{expected}/");
    if next != expected
        || !url.starts_with(&prefix)
        || !url.ends_with(".exe")
        || url[prefix.len()..].contains('/')
    {
        return Err("更新包与已检查的版本不一致，请重新检查更新".into());
    }
    Ok(())
}
fn progress(app: &AppHandle, stage: &'static str, downloaded: u64, total: Option<u64>) {
    let controller = app.state::<Controller>();
    let mut state = controller.0.lock().unwrap();
    state.stage = stage;
    state.downloaded = downloaded;
    state.total = total.filter(|v| *v > 0);
    drop(state);
    let _ = app.emit_to("update", "update-state", ());
}
#[tauri::command]
pub fn install_update(app: AppHandle, window: WebviewWindow) -> Result<(), String> {
    use tauri_plugin_updater::UpdaterExt;
    if window.label() != "update" {
        return Err("invalid update window".into());
    }
    if !updates::is_registered_install() {
        return Err("绿色版或安装位置无法确认，请通过发布页下载新版；不会启动安装器".into());
    }
    let expected = app
        .state::<Controller>()
        .0
        .lock()
        .unwrap()
        .begin_install()?;
    let _ = app.emit_to("update", "update-state", ());
    tauri::async_runtime::spawn(async move {
        let result: Result<(), String> = async {
            let endpoint = format!("https://github.com/returnk/codex-usage-badge/releases/download/v{expected}/latest.json");
            let updater = app.updater_builder().endpoints(vec![endpoint.parse().map_err(|_| "更新地址无效")?]).map_err(|e| e.to_string())?.timeout(std::time::Duration::from_secs(20)).build().map_err(|e| e.to_string())?;
            let mut update = updater.check().await.map_err(|_| "更新包暂未就绪或网络不可用，请稍后重试，也可查看发布页")?.ok_or("当前没有可安装的新版")?;
            validate_package(&expected, &update.version, update.download_url.as_str())?;
            update.timeout = Some(std::time::Duration::from_secs(300));
            let mut downloaded = 0u64;
            let mut last = std::time::Instant::now() - std::time::Duration::from_secs(1);
            progress(&app, "downloading", 0, None);
            let bytes = update.download(|chunk,total| {
                downloaded = downloaded.saturating_add(chunk as u64);
                if last.elapsed().as_millis() >= 100 {
                    progress(&app,"downloading",downloaded,total);
                    last = std::time::Instant::now();
                }
            }, || {}).await.map_err(|_| "下载或签名验证失败，当前版本未更改，请重试")?;
            progress(&app,"installing",bytes.len() as u64,Some(bytes.len() as u64));
            #[cfg(test)]
            return Err("隔离测试不执行安装".into());
            #[cfg(not(test))]
            update.install(bytes).map_err(|_| "无法启动安装程序，请重试或查看发布页".to_owned())
        }.await;
        if let Err(error) = result {
            let controller = app.state::<Controller>();
            let mut state = controller.0.lock().unwrap();
            state.installing = false;
            state.stage = "failed";
            state.error = Some(error);
            drop(state);
            let _ = app.emit_to("update", "update-state", ());
        }
    });
    Ok(())
}
#[tauri::command]
pub fn drag_update(app: AppHandle, window: WebviewWindow) -> Result<u64, String> {
    if window.label() != "update" {
        return Err("invalid update window".into());
    }
    let position = window.outer_position().map_err(|e| e.to_string())?;
    let scale = window.scale_factor().map_err(|e| e.to_string())?;
    let controller = app.state::<Controller>();
    let mut state = controller.0.lock().unwrap();
    if !state.visible {
        return Err("update window is hidden".into());
    }
    state.drag_id = state.drag_id.wrapping_add(1).max(1);
    state.drag_origin = Some(((position.x, position.y), scale));
    state.dragging = true;
    Ok(state.drag_id)
}
fn drag_position(
    origin: (i32, i32),
    delta: (f64, f64),
    scale: f64,
    size: (i32, i32),
    area: domain::Rect,
) -> (i32, i32) {
    domain::clamp_to_work_area(
        (
            origin.0 + (delta.0 * scale).round() as i32,
            origin.1 + (delta.1 * scale).round() as i32,
        ),
        size,
        area,
    )
}
#[tauri::command]
pub fn move_update_drag(
    app: AppHandle,
    window: WebviewWindow,
    drag_id: u64,
    dx: f64,
    dy: f64,
) -> Result<(), String> {
    if window.label() != "update"
        || !dx.is_finite()
        || !dy.is_finite()
        || dx.abs() > 100000.0
        || dy.abs() > 100000.0
    {
        return Err("invalid drag".into());
    }
    let controller = app.state::<Controller>();
    let state = controller.0.lock().unwrap();
    if !state.visible || !state.dragging || state.drag_id != drag_id {
        return Ok(());
    }
    let Some((origin, scale)) = state.drag_origin else {
        return Ok(());
    };
    drop(state);
    let target = (
        origin.0 + (dx * scale).round() as i32,
        origin.1 + (dy * scale).round() as i32,
    );
    let size = window.outer_size().map_err(|e| e.to_string())?;
    let monitor = native::monitor_at((
        target.0 + size.width as i32 / 2,
        target.1 + size.height as i32 / 2,
    ))
    .ok_or("monitor unavailable")?;
    let (x, y) = drag_position(
        origin,
        (dx, dy),
        scale,
        (size.width as i32, size.height as i32),
        monitor.area,
    );
    window
        .set_position(tauri::PhysicalPosition::new(x, y))
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub fn end_update_drag(app: AppHandle, window: WebviewWindow, drag_id: u64) -> Result<(), String> {
    if window.label() != "update" {
        return Err("invalid update window".into());
    }
    let controller = app.state::<Controller>();
    let mut state = controller.0.lock().unwrap();
    if state.drag_id != drag_id || !state.dragging {
        return Ok(());
    }
    let visible = state.visible;
    state.dragging = false;
    state.drag_origin = None;
    drop(state);
    if visible {
        window.set_focus().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pointer_drag_uses_original_position_and_supported_scale_then_clamps() {
        for scale in [1.0, 1.25, 1.5] {
            assert_eq!(
                drag_position(
                    (100, 100),
                    (60.0, 40.0),
                    scale,
                    (340, 300),
                    (0, 0, 1920, 1080)
                ),
                (
                    100 + (60.0 * scale).round() as i32,
                    100 + (40.0 * scale).round() as i32
                )
            );
            assert_eq!(
                drag_position(
                    (100, 100),
                    (-1000.0, -1000.0),
                    scale,
                    (340, 300),
                    (0, 0, 1920, 1080)
                ),
                (0, 0)
            );
        }
    }
    #[test]
    fn drag_blur_does_not_hide_but_regular_blur_and_explicit_close_still_do() {
        let mut state = UiState {
            visible: true,
            session: 2,
            shown_session: 2,
            ..UiState::default()
        };
        assert!(state.hide_on_blur());
        state.dragging = true;
        assert!(!state.hide_on_blur());
        state.visible = false;
        state.dragging = false;
        assert!(!state.hide_on_blur());
        state.visible = true;
        assert!(state.hide_on_blur());
    }
    #[test]
    fn install_requires_visible_new_release_and_cannot_start_twice() {
        let mut state = UiState::default();
        assert!(state.begin_install().is_err());
        state.visible = true;
        state.result = Some(json!({"available":false,"version":"0.3.1"}));
        assert!(state.begin_install().is_err());
        state.result = Some(json!({"available":true,"version":"0.3.3"}));
        assert_eq!(state.begin_install().unwrap(), "0.3.3");
        assert!(state.begin_install().is_err());
        state.visible = false;
        assert!(
            !state.begin(),
            "reopening a download must not replace release state"
        );
        assert_eq!(state.result.as_ref().unwrap()["version"], "0.3.3");
    }
    #[test]
    fn manifest_must_match_checked_release_and_trusted_download_origin() {
        assert!(validate_package(
            "0.3.3",
            "0.3.3",
            "https://github.com/returnk/codex-usage-badge/releases/download/v0.3.3/Badge.exe"
        )
        .is_ok());
        for (next, url) in [
            (
                "0.3.4",
                "https://github.com/returnk/codex-usage-badge/releases/download/v0.3.3/Badge.exe",
            ),
            ("0.3.3", "https://evil.test/Badge.exe"),
            (
                "0.3.3",
                "https://github.com/returnk/codex-usage-badge/releases/download/v0.3.2/Badge.exe",
            ),
        ] {
            assert!(validate_package("0.3.3", next, url).is_err());
        }
    }
    #[test]
    fn reopening_a_pending_check_reuses_request_and_close_does_not_cancel_notice() {
        let mut state = UiState::default();
        assert!(state.begin());
        let first = state.session;
        state.visible = false;
        assert!(!state.begin());
        assert!(state.visible);
        assert!(state.session > first);
        state.visible = false;
        state.checking = false;
        state.result = Some(json!({"version":"0.3.3"}));
        assert!(!state.visible);
        assert!(state.begin());
        assert!(state.result.is_none());
    }
    #[test]
    fn update_frame_fits_edges_on_negative_monitors_at_all_supported_scales() {
        for dpi in [96, 120, 144] {
            for area in [(-1920, 0, 0, 1080), (0, 0, 1920, 1080)] {
                for point in [(area.0, area.1), (area.2 - 1, area.3 - 1)] {
                    let (x, y, w, h) = frame(point, area, dpi);
                    assert!(x >= area.0 && y >= area.1 && x + w <= area.2 && y + h <= area.3);
                    assert_eq!(w, domain::dip_to_px(340.0, dpi));
                }
            }
        }
    }
}
