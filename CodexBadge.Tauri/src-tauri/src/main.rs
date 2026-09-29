#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod diagnostics;
mod domain;
mod native;
mod quota;
mod settings;

use domain::{Settings, Snapshot};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State};

#[derive(Default)]
struct Shared {
    inner: Mutex<Model>,
    quota_active: AtomicBool,
    quota_generation: AtomicU64,
}

struct Model {
    windows: [isize; 4],
    ready: [bool; 4],
    created_at: [Option<Instant>; 4],
    ready_timeout: [bool; 4],
    owner: isize,
    host: domain::HostObservation,
    host_identity: Option<native::ProcessIdentity>,
    anchor_generation: u64,
    last_host_frame: Option<(i32, i32, i32, i32)>,
    last_focus: Option<(isize, u32, u32, isize)>,
    last_data_signature: Option<(String, Option<u64>, usize)>,
    anchor: Option<(i32, i32)>,
    anchor_frame: Option<(i32, i32, i32, i32)>,
    pending_anchor: Option<(i32, i32)>,
    anchor_samples: u8,
    missing_anchor_samples: u8,
    capsule_visible: bool,
    detail_visible: bool,
    detail_hover_blocked: bool,
    hover_detail_enabled: bool,
    last_capsule_hover: Option<bool>,
    credit_visible: bool,
    left_at: Option<Instant>,
    credit_open: bool,
    drag: Option<Drag>,
    settings: Settings,
    snapshot: Option<Snapshot>,
    quota_failed: bool,
    last_capsule: Option<(i32, i32, i32, i32)>,
    menu_visible: bool,
    menu_request_id: u64,
    menu_ready_request: u64,
    menu_target: Option<(i32, i32, i32, i32)>,
    menu_requested_at: Option<Instant>,
    menu_left_at: Option<Instant>,
    menu_shown_at: Option<Instant>,
    menu_entered: bool,
    menu_observe_at: Option<Instant>,
    menu_resize_failed_logged: bool,
    menu_origin: Option<(i32, i32)>,
    menu_from_capsule: bool,
    last_theme_change: Option<Instant>,
    last_discovery: Instant,
    last_window_creation: Option<Instant>,
    absent_since: Option<Instant>,
    quota_status: String,
}

impl Default for Model {
    fn default() -> Self {
        Self {
            windows: [0; 4],
            ready: [false; 4],
            created_at: [None; 4],
            ready_timeout: [false; 4],
            owner: 0,
            host: domain::HostObservation::default(),
            host_identity: None,
            anchor_generation: 0,
            last_host_frame: None,
            last_focus: None,
            last_data_signature: None,
            anchor: None,
            anchor_frame: None,
            pending_anchor: None,
            anchor_samples: 0,
            missing_anchor_samples: 0,
            capsule_visible: false,
            detail_visible: false,
            detail_hover_blocked: false,
            hover_detail_enabled: !diagnostics::no_hover_detail(),
            last_capsule_hover: None,
            credit_visible: false,
            left_at: None,
            credit_open: false,
            drag: None,
            settings: Settings::default(),
            snapshot: None,
            quota_failed: false,
            last_capsule: None,
            menu_visible: false,
            menu_request_id: 0,
            menu_ready_request: 0,
            menu_target: None,
            menu_requested_at: None,
            menu_left_at: None,
            menu_shown_at: None,
            menu_entered: false,
            menu_observe_at: None,
            menu_resize_failed_logged: false,
            menu_origin: None,
            menu_from_capsule: false,
            last_theme_change: None,
            last_window_creation: None,
            last_discovery: Instant::now() - Duration::from_secs(2),
            absent_since: None,
            quota_status: "正在连接额度服务".into(),
        }
    }
}

struct Drag {
    cursor: (i32, i32),
    window: (i32, i32),
    last: (i32, i32),
    size: (i32, i32),
    start_dpi: u32,
}

fn hide_flyouts(m: &mut Model) {
    if m.detail_visible {
        diagnostics::record("popup_hide label=detail");
    }
    native::hide(m.windows[1]);
    native::hide(m.windows[2]);
    m.detail_visible = false;
    m.credit_visible = false;
    m.credit_open = false;
    m.left_at = None;
}

fn hide_all(m: &mut Model) {
    hide_flyouts(m);
    native::hide(m.windows[0]);
    m.capsule_visible = false;
}

fn release_badge_model(m: &mut Model) -> [isize; 4] {
    if m.menu_from_capsule {
        close_menu(m, m.menu_request_id);
    }
    let windows = m.windows;
    forget_windows(m, 0..3);
    m.snapshot = None;
    m.last_capsule = None;
    m.host_identity = None;
    m.quota_status = "等待 Codex 打开".into();
    windows
}

fn detail_hover_inside(m: &mut Model, capsule: bool, detail: bool, credit: bool) -> bool {
    if !m.hover_detail_enabled || m.menu_visible || m.menu_target.is_some() {
        return false;
    }
    if m.detail_hover_blocked {
        if !capsule {
            m.detail_hover_blocked = false;
        }
        return false;
    }
    capsule || detail || credit
}

fn epoch_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn active_credits(m: &Model) -> Vec<domain::ResetCredit> {
    m.snapshot
        .as_ref()
        .filter(|snapshot| {
            domain::quota_freshness(Some(snapshot.fetched_at), m.quota_failed, epoch_now())
                != "unavailable"
        })
        .map(|s| {
            s.credits
                .iter()
                .filter(|c| c.expires_at > epoch_now())
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

fn resize_move(app: &AppHandle, label: &str, hwnd: isize, x: i32, y: i32, w: i32, h: i32) -> bool {
    let Some(window) = app.get_webview_window(label) else {
        return false;
    };
    // Move first: WM_DPICHANGED must settle before setting the physical client size.
    if native::position(hwnd) != Some((x, y)) {
        native::move_only(hwnd, x, y);
    }
    if native::client_size(hwnd) != Some((w, h))
        && window
            .set_size(tauri::PhysicalSize::new(w as u32, h as u32))
            .is_err()
    {
        return false;
    }
    // Only record settled size transitions, never one event per tracking tick.
    native::client_size(hwnd) == Some((w, h))
}

fn apply_badge_mode(windows: [isize; 4], codex_owner: isize, enabled: bool) -> Result<(), String> {
    let target_owner = domain::badge_owner(codex_owner, enabled);
    // Check the live owner, not the previous tick's cached host state. Hide before
    // changing ownership/topmost so no intermediate normal window can flash.
    let may_show = enabled
        || (native::is_window(codex_owner)
            && native::is_visible(codex_owner)
            && !native::is_minimized(codex_owner));
    if !may_show {
        for hwnd in windows.iter().take(3) {
            native::hide(*hwnd);
        }
    }
    for (index, label) in ["capsule", "detail", "credit"].iter().enumerate() {
        if !native::is_window(windows[index]) {
            continue;
        }
        let was_visible = may_show && native::is_visible(windows[index]);
        if enabled {
            native::bind_owner(windows[index], 0);
        }
        if !native::set_topmost(windows[index], enabled) {
            return Err(format!("{label} topmost state did not change"));
        }
        if !enabled {
            native::bind_owner(windows[index], target_owner);
        }
        if native::owner(windows[index]) != target_owner {
            return Err(format!("{label} owner did not change"));
        }
        if native::is_topmost(windows[index]) != enabled {
            return Err(format!("{label} topmost state did not change"));
        }
        if was_visible && !native::is_visible(windows[index]) {
            native::show(windows[index]);
        }
    }
    Ok(())
}

fn layout(m: &mut Model, app: &AppHandle) {
    if m.windows[0] == 0 {
        return;
    }
    if m.drag.is_some() {
        return;
    }
    let anchor = if native::is_visible(m.owner) && !native::is_minimized(m.owner) {
        m.anchor
    } else {
        None
    };
    if m.capsule_visible && !native::is_visible(m.windows[0]) {
        m.capsule_visible = false;
    }
    if m.detail_visible && !native::is_visible(m.windows[1]) {
        m.detail_visible = false;
    }
    if m.credit_visible && !native::is_visible(m.windows[2]) {
        m.credit_visible = false;
    }

    let global_monitor = m
        .settings
        .always_on_top
        .then(|| {
            native::saved_monitor(
                m.settings
                    .global_position
                    .as_ref()
                    .map(|p| p.device.as_str()),
            )
        })
        .flatten();
    let reference_dpi = if let Some(monitor) = &global_monitor {
        monitor.dpi
    } else {
        anchor
            .or_else(|| m.last_capsule.map(|(x, y, w, h)| (x + w / 2, y + h / 2)))
            .map(native::dpi_at)
            .unwrap_or(96)
    };
    let frame = if let Some(monitor) = &global_monitor {
        Some(domain::global_frame(
            m.settings.global_position.as_ref(),
            monitor.area,
            monitor.dpi,
        ))
    } else {
        domain::capsule_frame(&m.settings, anchor, reference_dpi, m.last_capsule)
    };
    let Some((reference_x, reference_y, _, _)) = frame else {
        hide_all(m);
        return;
    };
    let inset = domain::dip_to_px(2.0, reference_dpi);
    let (origin_x, origin_y) = (reference_x + inset, reference_y + inset);
    let dpi = native::dpi_at((origin_x + 33, origin_y + 13));
    let size = |value: f64| domain::dip_to_px(value, dpi);
    let capsule_w = size(67.0);
    let capsule_h = size(26.0);
    let (outer_x, outer_y, _, _) = domain::capsule_window_rect(origin_x, origin_y, dpi);
    let outer_w = size(71.0);
    let outer_h = size(30.0);
    let (outer_x, outer_y) = native::clamp_to_work_area(
        outer_x,
        outer_y,
        outer_w,
        outer_h,
        (outer_x + outer_w / 2, outer_y + outer_h / 2),
    );
    let x = outer_x + size(2.0);
    let y = outer_y + size(2.0);
    let placement = (outer_x, outer_y, outer_w, outer_h);
    if m.settings.always_on_top {
        let saved = native::global_position((outer_x, outer_y));
        if saved != m.settings.global_position {
            m.settings.global_position = saved;
            let _ = settings::save(&m.settings);
        }
    }
    if m.last_capsule != Some(placement) {
        let sized = resize_move(
            app,
            "capsule",
            m.windows[0],
            outer_x,
            outer_y,
            outer_w,
            outer_h,
        );
        let mut clip_queued = true;
        if sized
            && m.last_capsule
                .is_none_or(|(_, _, w, h)| w != outer_w || h != outer_h)
        {
            let hwnd = m.windows[0];
            let show_after_clip = !m.capsule_visible;
            native::round_capsule(hwnd, outer_w, outer_h);
            if show_after_clip && m.ready[0] {
                native::show(hwnd);
            }
            clip_queued = native::is_window(hwnd);
            if clip_queued && show_after_clip && m.ready[0] {
                m.capsule_visible = true;
            }
        }
        if sized && clip_queued {
            m.last_capsule = Some(placement);
        }
    }
    if !m.ready[0] {
        return;
    }
    if !m.capsule_visible && m.last_capsule == Some(placement) {
        native::show(m.windows[0]);
        m.capsule_visible = true;
    }

    let Some(monitor) = native::monitor_at((x + capsule_w / 2, y + capsule_h / 2)) else {
        hide_flyouts(m);
        return;
    };
    let Some((detail_x, detail_y, detail_w, detail_h)) = domain::place_detail_popup(
        placement,
        (size(270.0), size(148.0)),
        monitor.area,
        size(6.0),
        dpi,
    ) else {
        hide_flyouts(m);
        return;
    };
    let capsule_inside = native::cursor_in_rect(x, y, capsule_w, capsule_h);
    if m.last_capsule_hover != Some(capsule_inside) {
        diagnostics::record(format!("capsule_hover entered={capsule_inside}"));
        m.last_capsule_hover = Some(capsule_inside);
    }
    let detail_inside = m.detail_visible && native::cursor_in_window(m.windows[1]);
    let credit_inside = m.credit_visible && native::cursor_in_window(m.windows[2]);
    let cursor_inside = detail_hover_inside(m, capsule_inside, detail_inside, credit_inside);
    if cursor_inside {
        m.left_at = None;
        if !m.detail_visible {
            if resize_move(
                app,
                "detail",
                m.windows[1],
                detail_x,
                detail_y,
                detail_w,
                detail_h,
            ) && m.ready[1]
            {
                native::show(m.windows[1]);
                m.detail_visible = true;
            }
        }
    } else if m.detail_visible {
        let (left_at, close) = domain::popup_departure(m.left_at, false, Instant::now());
        m.left_at = left_at;
        if close {
            hide_flyouts(m);
        }
    }
    if !m.detail_visible {
        return;
    }
    resize_move(
        app,
        "detail",
        m.windows[1],
        detail_x,
        detail_y,
        detail_w,
        detail_h,
    );
    if m.credit_open {
        let count = active_credits(m).len();
        if count == 0 {
            native::hide(m.windows[2]);
            m.credit_open = false;
            m.credit_visible = false;
            return;
        }
        let Some((credit_x, credit_y, credit_w, credit_h)) = domain::place_popup(
            (detail_x, detail_y, detail_w, detail_h),
            (size(205.0), size(domain::credit_popup_height(count))),
            monitor.area,
            &[placement, (detail_x, detail_y, detail_w, detail_h)],
            size(6.0),
        ) else {
            native::hide(m.windows[2]);
            m.credit_visible = false;
            return;
        };
        let sized = resize_move(
            app,
            "credit",
            m.windows[2],
            credit_x,
            credit_y,
            credit_w,
            credit_h,
        );
        if !m.credit_visible && sized && m.ready[2] {
            native::show(m.windows[2]);
            m.credit_visible = true;
        }
    }
}

fn track(shared: Arc<Shared>, app: AppHandle) {
    let queued = Arc::new(AtomicBool::new(false));
    loop {
        if !queued.swap(true, Ordering::AcqRel) {
            let (shared, handle, queued_callback) = (shared.clone(), app.clone(), queued.clone());
            if app
                .run_on_main_thread(move || {
                    track_frame(&shared, &handle);
                    queued_callback.store(false, Ordering::Release);
                })
                .is_err()
            {
                break;
            }
        }
        let dragging = shared.inner.lock().unwrap().drag.is_some();
        thread::sleep(Duration::from_millis(if dragging { 8 } else { 50 }));
    }
}

// All HWND/Tauri mutations run here on the UI thread. The worker only queues one frame.
fn track_frame(shared: &Arc<Shared>, app: &AppHandle) {
    run_tracking_frame(
        || track_badge_frame(shared, app),
        || track_menu_frame(shared, app),
    );
}

fn run_tracking_frame(badge: impl FnOnce(), menu: impl FnOnce()) {
    badge();
    menu();
}

fn forget_windows(m: &mut Model, indices: std::ops::Range<usize>) {
    for index in indices {
        m.windows[index] = 0;
        m.ready[index] = false;
        m.created_at[index] = None;
        m.ready_timeout[index] = false;
        if index == 0 {
            m.last_capsule = None;
        }
        if index == 3 {
            m.menu_visible = false;
            m.menu_ready_request = 0;
        }
    }
}

fn finish_initialization<T, E>(
    result: Result<T, E>,
    new: bool,
    cleanup: impl FnOnce(),
) -> Result<T, E> {
    if result.is_err() && new {
        cleanup();
    }
    result
}

fn track_badge_frame(shared: &Arc<Shared>, app: &AppHandle) {
    let (periodic_scan, current, mut identity, unknown) = {
        let m = shared.inner.lock().unwrap();
        (
            m.drag.is_none() && m.last_discovery.elapsed() >= Duration::from_secs(1),
            m.owner,
            m.host_identity,
            m.host.unknown,
        )
    };
    let refreshed = native::refresh_codex_window(current, identity);
    let scan = periodic_scan || (current != 0 && refreshed.is_none());
    let next_host = if scan {
        Some(native::observe_codex(current, &mut identity))
    } else {
        refreshed.map(|mut host| {
            host.unknown = unknown;
            host
        })
    };
    let mut m = shared.inner.lock().unwrap();
    for (index, label) in ["capsule", "detail", "credit", "menu"].iter().enumerate() {
        if !m.ready[index]
            && !m.ready_timeout[index]
            && m.created_at[index].is_some_and(|at| at.elapsed() >= Duration::from_secs(10))
        {
            diagnostics::record(format!(
                "window_ready_timeout label={label} hwnd={}",
                m.windows[index]
            ));
            m.ready_timeout[index] = true;
            let _ = app.emit_to(*label, "retry-ready", ());
        }
    }
    if m.windows
        .iter()
        .take(3)
        .enumerate()
        .any(|(index, h)| *h != 0 && (!native::is_window(*h) || m.ready_timeout[index]))
    {
        hide_all(&mut m);
        forget_windows(&mut m, 0..3);
        drop(m);
        for label in ["credit", "detail", "capsule"] {
            if let Some(window) = app.get_webview_window(label) {
                let _ = window.destroy();
            }
        }
        return;
    }

    let capsule_hwnd = m.windows[0];
    if let Some(drag) = &mut m.drag {
        if native::left_mouse_down() {
            if let Some(cursor) = native::cursor() {
                let pos = (
                    drag.window.0 + cursor.0 - drag.cursor.0,
                    drag.window.1 + cursor.1 - drag.cursor.1,
                );
                let pos = native::clamp_to_work_area(
                    pos.0,
                    pos.1,
                    drag.size.0,
                    drag.size.1,
                    (pos.0 + drag.size.0 / 2, pos.1 + drag.size.1 / 2),
                );
                if pos != drag.last {
                    let dpi = native::dpi_at((pos.0 + drag.size.0 / 2, pos.1 + drag.size.1 / 2));
                    let size = (domain::dip_to_px(71.0, dpi), domain::dip_to_px(30.0, dpi));
                    resize_move(app, "capsule", capsule_hwnd, pos.0, pos.1, size.0, size.1);
                    if size != drag.size {
                        native::round_capsule(capsule_hwnd, size.0, size.1);
                        drag.size = size;
                    }
                    drag.last = native::position(capsule_hwnd).unwrap_or(pos);
                }
            }
        } else {
            let settings = finish_drag(&mut m);
            drop(m);
            if let Some(settings) = settings {
                let _ = settings::save(&settings);
            }
            return;
        }
        drop(m);
        return;
    }

    if let Some(host) = next_host {
        let next = host.window;
        if host != m.host {
            diagnostics::record(format!(
                "host_observe hwnd={next} visible={} iconic={} reason={}",
                host.visible,
                host.iconic,
                if host.unknown {
                    "unknown"
                } else if next == 0 {
                    "missing"
                } else if host.iconic {
                    "iconic"
                } else if host.visible {
                    "visible"
                } else {
                    "hidden"
                }
            ));
        }
        let owner_changed = next != m.owner;
        if owner_changed || (m.host.iconic && !host.iconic) || (!m.host.visible && host.visible) {
            if !m.settings.always_on_top {
                hide_all(&mut m);
            }
            m.owner = next;
            m.anchor_generation += 1;
            if owner_changed {
                m.anchor = None;
                m.anchor_frame = None;
                m.last_host_frame = None;
            }
            m.pending_anchor = None;
            m.anchor_samples = 0;
            m.missing_anchor_samples = 0;
            for hwnd in m.windows.iter().take(3) {
                native::bind_owner(*hwnd, domain::badge_owner(next, m.settings.always_on_top));
            }
        }
        if m.host_identity.is_some() && identity.is_some() && m.host_identity != identity {
            shared.quota_generation.fetch_add(1, Ordering::AcqRel);
            m.snapshot = None;
        }
        m.host = host;
        m.host_identity = identity;
        if scan {
            m.last_discovery = Instant::now();
        }
    }
    // Reading under this lock cannot race an already committed UIA sample.
    let frame = (m.host.window != 0 && m.host.visible && !m.host.iconic)
        .then(|| native::host_frame(m.owner))
        .flatten();
    update_host_frame(&mut m, frame);
    let now = Instant::now();
    if m.host.window != 0 || m.host.unknown {
        m.absent_since = None;
    } else if m.host_identity.is_some() {
        m.absent_since.get_or_insert(now);
    }
    let (retain, display) =
        domain::host_policy(m.host, m.settings.always_on_top, m.absent_since, now);
    if shared.quota_active.swap(retain, Ordering::AcqRel) != retain {
        shared.quota_generation.fetch_add(1, Ordering::AcqRel);
    }
    let focus = native::focus_observation();
    if m.last_focus != Some(focus) {
        diagnostics::record(format!(
            "focus_observe foreground={} pid={} tid={} focus={} ime=unknown",
            focus.0, focus.1, focus.2, focus.3
        ));
        m.last_focus = Some(focus);
    }
    if !display {
        m.drag = None;
        hide_all(&mut m);
    }
    if !retain {
        if m.windows[0] != 0 || m.snapshot.is_some() {
            let windows = release_badge_model(&mut m);
            drop(m);
            for label in ["credit", "detail", "capsule"] {
                if let Some(window) = app.get_webview_window(label) {
                    let _ = window.destroy();
                }
            }
            diagnostics::record(format!("lifecycle_release reason=exit windows={windows:?}"));
            return;
        }
    } else {
        if m.windows.iter().take(3).any(|window| *window == 0) && display {
            if m.last_window_creation
                .is_some_and(|last| last.elapsed() < Duration::from_secs(5))
            {
                return;
            }
            m.last_window_creation = Some(Instant::now());
            drop(m);
            for label in ["capsule", "detail", "credit"] {
                if let Err(error) = create_window(app, shared, label) {
                    diagnostics::record(format!(
                        "window_create_failed label={label} error={error}"
                    ));
                    let mut m = shared.inner.lock().unwrap();
                    hide_all(&mut m);
                    forget_windows(&mut m, 0..3);
                    drop(m);
                    for label in ["credit", "detail", "capsule"] {
                        if let Some(window) = app.get_webview_window(label) {
                            let _ = window.destroy();
                        }
                    }
                    return;
                }
            }
            diagnostics::record("lifecycle_start");
            return;
        }
    }
    let was_detail = m.detail_visible;
    let was_credit = m.credit_visible;
    let signature = (
        domain::quota_freshness(
            m.snapshot.as_ref().map(|s| s.fetched_at),
            m.quota_failed,
            epoch_now(),
        )
        .to_string(),
        domain::credit_count(m.snapshot.as_ref(), epoch_now()),
        active_credits(&m).len(),
    );
    let data_changed = m.last_data_signature.as_ref() != Some(&signature);
    m.last_data_signature = Some(signature);
    if display {
        layout(&mut m, &app);
    }
    let flyout_changed = was_detail != m.detail_visible || was_credit != m.credit_visible;
    drop(m);
    if flyout_changed || data_changed {
        let _ = app.emit("state-updated", ());
    }
}

fn update_host_frame(m: &mut Model, frame: Option<(i32, i32, i32, i32)>) {
    let Some(frame) = frame else { return };
    if m.last_host_frame.is_some_and(|old| old != frame) {
        // Preserve the known sidebar width while the accessibility tree catches
        // up. UIA replaces this provisional point with the verified new bounds.
        if let (Some((x, y)), Some(source)) = (&mut m.anchor, m.anchor_frame) {
            *x += frame.0 - source.0;
            *y += (frame.1 + frame.3) - (source.1 + source.3);
            m.anchor_frame = Some(frame);
        }
        m.anchor_generation += 1;
        m.pending_anchor = None;
        m.anchor_samples = 0;
        m.missing_anchor_samples = 0;
        diagnostics::record(format!(
            "host_geometry rect={},{},{},{} generation={}",
            frame.0, frame.1, frame.2, frame.3, m.anchor_generation
        ));
    }
    m.last_host_frame = Some(frame);
}

fn update_menu_departure(m: &mut Model, inside: bool, now: Instant) {
    let (entered, left_at, close) = domain::menu_departure(
        m.menu_entered,
        m.menu_shown_at.unwrap_or(now),
        m.menu_left_at,
        inside,
        now,
    );
    m.menu_entered = entered;
    m.menu_left_at = left_at;
    if close {
        diagnostics::record(format!("menu_hide pointer_exit entered={entered}"));
        close_menu(m, m.menu_request_id);
    }
}

fn expire_pending_menu(m: &mut Model, now: Instant) -> bool {
    if m.menu_target.is_some()
        && m.menu_requested_at
            .is_some_and(|at| now.saturating_duration_since(at) >= Duration::from_secs(10))
    {
        diagnostics::record(format!(
            "menu_hide reason=timeout request_id={}",
            m.menu_request_id
        ));
        close_menu(m, m.menu_request_id);
        return true;
    }
    false
}

fn track_menu_frame(shared: &Arc<Shared>, app: &AppHandle) {
    let mut m = shared.inner.lock().unwrap();
    expire_pending_menu(&mut m, Instant::now());
    if m.windows[3] != 0
        && (!native::is_window(m.windows[3]) || (m.ready_timeout[3] && m.menu_target.is_some()))
    {
        forget_windows(&mut m, 3..4);
        drop(m);
        if let Some(window) = app.get_webview_window("menu") {
            let _ = window.destroy();
        }
        m = shared.inner.lock().unwrap();
    }
    if m.menu_target.is_some() && m.windows[3] == 0 {
        drop(m);
        if let Err(error) = create_window(app, shared, "menu") {
            diagnostics::record(format!("menu_create_failed error={error}"));
        }
        return;
    }
    if m.menu_target.is_some() && !m.ready[3] && m.menu_ready_request != m.menu_request_id {
        m.menu_ready_request = m.menu_request_id;
        let _ = app.emit_to("menu", "retry-ready", ());
    }
    let mut menu_opened = false;
    if let Some((x, y, width, height)) = m.menu_target {
        if resize_move(&app, "menu", m.windows[3], x, y, width, height) && m.ready[3] {
            let raised = native::raise(m.windows[3]);
            diagnostics::record(format!(
                "menu_raise accepted={raised} rect={x},{y},{width},{height}"
            ));
            if raised {
                let now = Instant::now();
                m.menu_visible = true;
                m.menu_target = None;
                m.menu_requested_at = None;
                m.menu_left_at = None;
                m.menu_shown_at = Some(now);
                m.menu_entered = false;
                m.menu_observe_at = Some(now);
                menu_opened = true;
            }
        } else if !m.menu_resize_failed_logged {
            diagnostics::record(format!(
                "menu_resize_pending rect={x},{y},{width},{height} client={:?}",
                native::client_size(m.windows[3])
            ));
            m.menu_resize_failed_logged = true;
        }
    }
    if let Some(shown_at) = m.menu_observe_at {
        if shown_at.elapsed() >= Duration::from_millis(200) {
            diagnostics::record(format!(
                "menu_observe visible={} position={:?} client={:?} owner={} topmost={}",
                native::is_visible(m.windows[3]),
                native::position(m.windows[3]),
                native::client_size(m.windows[3]),
                native::owner(m.windows[3]),
                native::is_topmost(m.windows[3])
            ));
            m.menu_observe_at = None;
        }
    }
    if m.menu_visible && m.menu_target.is_none() {
        let inside = native::cursor()
            .zip(native::window_rect(m.windows[3]))
            .is_some_and(|(point, rect)| {
                domain::menu_pointer_inside(
                    point,
                    (
                        rect.left,
                        rect.top,
                        rect.right - rect.left,
                        rect.bottom - rect.top,
                    ),
                    (m.menu_from_capsule && m.capsule_visible)
                        .then_some(m.last_capsule)
                        .flatten(),
                )
            });
        update_menu_departure(&mut m, inside, Instant::now());
    }
    let release_menu =
        m.owner == 0 && m.windows[3] != 0 && !m.menu_visible && m.menu_target.is_none();
    if release_menu {
        forget_windows(&mut m, 3..4);
    }
    drop(m);
    if release_menu {
        if let Some(window) = app.get_webview_window("menu") {
            let _ = window.destroy();
        }
    }
    if menu_opened {
        let _ = app.emit("state-updated", ());
    }
}

fn create_window(app: &AppHandle, shared: &Arc<Shared>, label: &str) -> Result<(), String> {
    let config = app
        .config()
        .app
        .windows
        .iter()
        .find(|config| config.label == label)
        .ok_or("window config missing")?;
    let existing = app.get_webview_window(label);
    let new = existing.is_none();
    let window = if let Some(window) = existing {
        window
    } else {
        let topmost = label == "menu" || shared.inner.lock().unwrap().settings.always_on_top;
        tauri::WebviewWindowBuilder::from_config(app, config)
            .map_err(|error| error.to_string())?
            .always_on_top(topmost)
            .focused(false)
            .build()
            .map_err(|error| error.to_string())?
    };
    let result = (|| -> Result<(), String> {
        let hwnd = window.hwnd().map_err(|error| error.to_string())?.0 as isize;
        native::make_nonactivating(hwnd).map_err(|error| error.to_string())?;
        // Consume STARTF_USESHOWWINDOW while still hidden. Later ShowWindow calls
        // must honor SW_SHOWNOACTIVATE even when the process launched with SW_HIDE.
        native::hide(hwnd);
        window
            .set_background_color(Some(tauri::window::Color(0, 0, 0, 0)))
            .map_err(|error| error.to_string())?;
        window
            .with_webview(|webview| unsafe {
                let result = webview
                    .controller()
                    .CoreWebView2()
                    .and_then(|core| core.Settings())
                    .and_then(|settings| settings.SetAreDefaultContextMenusEnabled(false));
                diagnostics::record(format!(
                    "webview_browser_menu_disabled success={}",
                    result.is_ok()
                ));
            })
            .map_err(|error| error.to_string())?;
        let mut m = shared.inner.lock().unwrap();
        let index = ["capsule", "detail", "credit", "menu"]
            .iter()
            .position(|value| *value == label)
            .ok_or("window label invalid")?;
        let owner = if label == "menu" {
            0
        } else {
            domain::badge_owner(m.owner, m.settings.always_on_top)
        };
        native::bind_owner(hwnd, owner);
        if !native::set_topmost(hwnd, label == "menu" || m.settings.always_on_top) {
            return Err(format!("{label} topmost state did not change"));
        }
        m.windows[index] = hwnd;
        m.ready[index] = false;
        m.created_at[index] = Some(Instant::now());
        m.ready_timeout[index] = false;
        if index == 0 {
            m.last_capsule = None;
        }
        native::log_window(label, hwnd);
        diagnostics::record(format!("window_created label={label} hwnd={hwnd}"));
        drop(m);
        if !new {
            let _ = app.emit_to(label, "retry-ready", ());
        }
        Ok(())
    })();
    finish_initialization(result, new, || {
        let _ = window.destroy();
    })
}

fn probe_anchor(shared: Arc<Shared>) {
    let Ok(automation) = uiautomation::UIAutomation::new() else {
        return;
    };
    let mut probe = native::AnchorProbe::new();
    let mut last_scan = Instant::now() - Duration::from_secs(5);
    let mut scan_owner = 0;
    let mut scan_generation = 0;
    let mut missing_scans = 0u32;
    let mut recovery_until = Instant::now();
    let mut had_anchor = false;
    let mut was_recovering = false;
    loop {
        let (owner, dragging, generation) = {
            let m = shared.inner.lock().unwrap();
            (m.owner, m.drag.is_some(), m.anchor_generation)
        };
        if dragging {
            thread::sleep(Duration::from_millis(250));
            continue;
        }
        if owner != 0 && !native::is_minimized(owner) {
            if owner != scan_owner || generation != scan_generation {
                scan_owner = owner;
                scan_generation = generation;
                probe = native::AnchorProbe::new();
                missing_scans = 0;
                had_anchor = false;
                recovery_until = Instant::now() + Duration::from_secs(2);
                was_recovering = true;
                last_scan = Instant::now() - Duration::from_secs(60);
            }
            let sampled_frame = native::host_frame(owner);
            let mut anchor = probe.read_cached(owner);
            if anchor.is_none() && had_anchor {
                recovery_until = Instant::now() + Duration::from_secs(2);
                was_recovering = true;
                missing_scans = 0;
                last_scan = Instant::now() - Duration::from_secs(60);
            }
            let recovering = Instant::now() < recovery_until;
            if anchor.is_none()
                && domain::anchor_scan_due(
                    Instant::now(),
                    last_scan,
                    recovery_until,
                    was_recovering,
                    missing_scans,
                )
            {
                let started = Instant::now();
                anchor = probe.discover(&automation, owner);
                diagnostics::record(format!(
                    "anchor_discover found={} duration_ms={}",
                    anchor.is_some(),
                    started.elapsed().as_millis()
                ));
                missing_scans = if anchor.is_some() || recovering {
                    0
                } else {
                    missing_scans + 1
                };
                last_scan = Instant::now();
            }
            had_anchor = anchor.is_some();
            was_recovering = recovering;
            let frame_after = native::host_frame(owner);
            let sample = AnchorSample {
                owner,
                generation,
                frame: sampled_frame.filter(|frame| Some(*frame) == frame_after),
                point: anchor,
            };
            let mut m = shared.inner.lock().unwrap();
            commit_anchor_sample(
                &mut m,
                sample,
                native::host_frame(owner),
                Instant::now(),
                recovery_until,
            );
        }
        thread::sleep(Duration::from_millis(250));
    }
}

struct AnchorSample {
    owner: isize,
    generation: u64,
    frame: Option<(i32, i32, i32, i32)>,
    point: Option<(i32, i32)>,
}

fn commit_anchor_sample(
    m: &mut Model,
    sample: AnchorSample,
    current_frame: Option<(i32, i32, i32, i32)>,
    now: Instant,
    until: Instant,
) -> bool {
    if m.owner != sample.owner
        || m.anchor_generation != sample.generation
        || sample.frame.is_none()
        || sample.frame != current_frame
    {
        return false;
    }
    if m.anchor.is_some() {
        let (point, misses) = domain::follow_anchor_during_recovery(
            m.anchor,
            m.missing_anchor_samples,
            sample.point,
            now,
            until,
        );
        m.anchor = point;
        m.missing_anchor_samples = misses;
    } else {
        let (pending, samples, ready) =
            domain::stable_anchor(m.pending_anchor, m.anchor_samples, sample.point);
        m.pending_anchor = pending;
        m.anchor_samples = samples;
        m.anchor = ready;
    }
    if sample.point.is_some() && m.anchor.is_some() {
        m.anchor_frame = sample.frame;
    }
    if m.anchor.is_none() {
        m.anchor_frame = None;
    }
    true
}

fn finish_drag(m: &mut Model) -> Option<Settings> {
    let Some(drag) = m.drag.take() else {
        return None;
    };
    diagnostics::record(format!(
        "drag_end start={:?} requested={:?} actual={:?}",
        drag.window,
        drag.last,
        native::position(m.windows[0])
    ));
    m.last_capsule = Some((drag.last.0, drag.last.1, drag.size.0, drag.size.1));
    if m.settings.always_on_top {
        m.settings.global_position = native::global_position(drag.last);
        return Some(m.settings.clone());
    }
    {
        let offset = domain::drag_window_offset(
            (m.settings.offset_x, m.settings.offset_y),
            drag.window,
            drag.last,
            drag.start_dpi,
            native::dpi(m.windows[0]),
            m.anchor
                .map(native::dpi_at)
                .unwrap_or_else(|| native::dpi_at(drag.window)),
        );
        m.settings.offset_x = offset.0;
        m.settings.offset_y = offset.1;
        Some(m.settings.clone())
    }
}

#[tauri::command]
fn get_state(state: State<'_, Arc<Shared>>) -> Value {
    let m = state.inner.lock().unwrap();
    let freshness = domain::quota_freshness(
        m.snapshot.as_ref().map(|s| s.fetched_at),
        m.quota_failed,
        epoch_now(),
    );
    let unavailable = freshness == "unavailable";
    let snapshot = if unavailable {
        None
    } else {
        m.snapshot.as_ref()
    };
    let weekly_exhausted = snapshot
        .and_then(|s| s.weekly.as_ref())
        .is_some_and(|w| w.remaining <= 0.0);
    let five = snapshot.and_then(|s| s.five_hour.as_ref());
    let weekly = snapshot.and_then(|s| s.weekly.as_ref());
    let remaining = if weekly_exhausted {
        Some(0.0)
    } else {
        five.or(weekly).map(|w| w.remaining)
    };
    let credits = if unavailable {
        Vec::new()
    } else {
        active_credits(&m)
    };
    json!({
        "theme": m.settings.theme,
        "capsule": domain::capsule_text(remaining),
        "fiveHour": five.map(|w| w.remaining),
        "weekly": weekly.map(|w| w.remaining),
        "fiveReset": domain::format_five_hour_reset(five.and_then(|w| w.resets_at)),
        "weekReset": domain::format_weekly_reset(weekly.and_then(|w| w.resets_at)),
        "weeklyExhausted": weekly_exhausted,
        "progressBand": domain::progress_band(five.map(|w| w.remaining).unwrap_or(0.0)),
        "credits": credits.iter().map(|c| json!({"id":c.id,"expiresAt":c.expires_at})).collect::<Vec<_>>(),
        "creditCount": domain::credit_count(snapshot, epoch_now()),
        "quotaStatus": m.quota_status,
        "creditOpen": m.credit_open,
        "startup": m.settings.start_with_windows,
        "topmost": m.settings.always_on_top,
        "freshness": freshness,
        "fetchedAt": snapshot.map(|s| s.fetched_at),
        "diagnosticsHealthy": diagnostics::healthy(),
        "defaultCursor": diagnostics::default_cursor(),
        "menuRequestId": m.menu_request_id,
    })
}

#[tauri::command]
fn report_capsule_layout(
    frame: [f64; 4],
    capsule: [f64; 4],
    viewport: [f64; 3],
    window: tauri::WebviewWindow,
) {
    diagnostics::record(format!(
        "capsule_layout frame={},{},{},{} capsule={},{},{},{} viewport={},{},{}",
        frame[0],
        frame[1],
        frame[2],
        frame[3],
        capsule[0],
        capsule[1],
        capsule[2],
        capsule[3],
        viewport[0],
        viewport[1],
        viewport[2]
    ));
    if let Ok(hwnd) = window.hwnd() {
        native::log_window(window.label(), hwnd.0 as isize);
    }
}

#[tauri::command]
fn window_ready(window: tauri::WebviewWindow, state: State<'_, Arc<Shared>>) -> bool {
    let Some(index) = ["capsule", "detail", "credit", "menu"]
        .iter()
        .position(|label| *label == window.label())
    else {
        return false;
    };
    let Ok(hwnd) = window.hwnd() else {
        return false;
    };
    let mut m = state.inner.lock().unwrap();
    if m.windows[index] != hwnd.0 as isize {
        return false;
    }
    if native::guard_webview_children(hwnd.0 as isize).is_err() {
        return false;
    }
    m.ready[index] = true;
    m.ready_timeout[index] = false;
    diagnostics::record(format!(
        "window_ready label={} hwnd={}",
        window.label(),
        hwnd.0 as isize
    ));
    native::log_window(window.label(), hwnd.0 as isize);
    true
}

#[tauri::command]
fn cycle_theme(delta: i32, state: State<'_, Arc<Shared>>, app: AppHandle) {
    let mut m = state.inner.lock().unwrap();
    let now = Instant::now();
    if !domain::accept_theme_wheel(m.last_theme_change, now) {
        return;
    }
    m.last_theme_change = Some(now);
    m.settings.theme = domain::cycle_theme(m.settings.theme, delta);
    let settings = m.settings.clone();
    drop(m);
    let _ = settings::save(&settings);
    let _ = app.emit("state-updated", ());
}

#[tauri::command]
fn reset_position(from_capsule: Option<bool>, state: State<'_, Arc<Shared>>) {
    let mut m = state.inner.lock().unwrap();
    let settings = reset_position_model(&mut m, from_capsule.unwrap_or(false));
    drop(m);
    if let Some(settings) = settings {
        let _ = settings::save(&settings);
    }
}

fn reset_position_model(m: &mut Model, from_capsule: bool) -> Option<Settings> {
    if from_capsule && m.settings.always_on_top {
        return None;
    }
    if m.settings.always_on_top {
        let point = m
            .last_capsule
            .map(|(x, y, w, h)| (x + w / 2, y + h / 2))
            .or_else(native::cursor)
            .unwrap_or((0, 0));
        if let Some(monitor) = native::monitor_at(point) {
            let (x, y, _, _) = domain::global_frame(None, monitor.area, monitor.dpi);
            m.settings.global_position = native::global_position((x, y));
        }
    } else {
        domain::reset_position(&mut m.settings);
    }
    Some(m.settings.clone())
}

#[tauri::command]
fn toggle_startup(state: State<'_, Arc<Shared>>, app: AppHandle) -> bool {
    let desired = !state.inner.lock().unwrap().settings.start_with_windows;
    if settings::set_startup(desired).is_ok() {
        let mut m = state.inner.lock().unwrap();
        m.settings.start_with_windows = desired;
        let settings = m.settings.clone();
        drop(m);
        let _ = settings::save(&settings);
        let _ = app.emit("state-updated", ());
    }
    state.inner.lock().unwrap().settings.start_with_windows
}

#[tauri::command]
fn toggle_topmost(state: State<'_, Arc<Shared>>, app: AppHandle) -> Result<bool, String> {
    let (desired, owner, windows, previous) = {
        let m = state.inner.lock().unwrap();
        (
            !m.settings.always_on_top,
            m.owner,
            m.windows,
            m.settings.clone(),
        )
    };
    diagnostics::record(format!("topmost_request enabled={desired} owner={owner}"));
    if let Err(error) = apply_badge_mode(windows, owner, desired) {
        let _ = apply_badge_mode(windows, owner, !desired);
        diagnostics::record(format!("topmost_failed {error}"));
        return Err(error);
    }
    let mut m = state.inner.lock().unwrap();
    m.settings.always_on_top = desired;
    hide_flyouts(&mut m);
    m.capsule_visible = native::is_visible(windows[0]);
    if desired && m.settings.global_position.is_none() {
        if let Some((x, y)) = native::position(windows[0]) {
            m.settings.global_position = native::global_position((x, y));
        }
    }
    let settings = m.settings.clone();
    drop(m);
    if let Err(error) = settings::save(&settings) {
        state.inner.lock().unwrap().settings = previous;
        let _ = apply_badge_mode(windows, owner, !desired);
        diagnostics::record(format!("topmost_save_failed {error}"));
        return Err(error);
    }
    diagnostics::record(format!(
        "topmost_applied enabled={desired} owner={} visible={}",
        native::owner(windows[0]),
        native::is_visible(windows[0])
    ));
    let _ = app.emit("state-updated", ());
    Ok(desired)
}

#[tauri::command]
fn hide_menu(request_id: u64, state: State<'_, Arc<Shared>>) {
    close_menu(&mut state.inner.lock().unwrap(), request_id);
}

fn close_menu(m: &mut Model, request_id: u64) {
    if request_id != m.menu_request_id || (!m.menu_visible && m.menu_target.is_none()) {
        return;
    }
    diagnostics::record("menu_hide command");
    native::hide(m.windows[3]);
    m.menu_visible = false;
    m.menu_target = None;
    m.menu_requested_at = None;
    m.menu_left_at = None;
    m.menu_shown_at = None;
    m.menu_observe_at = None;
    m.detail_hover_blocked = true;
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MenuSource {
    Tray,
    Capsule,
}

fn request_menu(m: &mut Model, point: (i32, i32), source: MenuSource) {
    m.menu_request_id = m.menu_request_id.wrapping_add(1);
    let dpi = native::dpi_at(point);
    let (width, height) = (domain::dip_to_px(190.0, dpi), domain::dip_to_px(148.0, dpi));
    let (x, y) = native::clamp_to_work_area(
        point.0 - width + domain::dip_to_px(16.0, dpi),
        point.1 - height + domain::dip_to_px(8.0, dpi),
        width,
        height,
        point,
    );
    let target = if let (Some(capsule), Some(monitor)) = (m.last_capsule, native::monitor_at(point))
    {
        domain::place_popup(
            if source == MenuSource::Capsule {
                capsule
            } else {
                (point.0, point.1, 1, 1)
            },
            (width, height),
            monitor.area,
            &[capsule],
            domain::dip_to_px(6.0, dpi),
        )
    } else {
        Some((x, y, width, height))
    };
    hide_flyouts(m);
    m.menu_target = target;
    m.menu_requested_at = Some(Instant::now());
    m.menu_origin = Some(point);
    m.menu_from_capsule = source == MenuSource::Capsule;
    m.menu_left_at = None;
    m.menu_entered = false;
    m.menu_shown_at = None;
    m.menu_observe_at = None;
    m.menu_resize_failed_logged = false;
    diagnostics::record(format!(
        "menu_request source={} request_id={} origin={},{} target={x},{y},{width},{height}",
        if source == MenuSource::Tray {
            "tray"
        } else {
            "capsule"
        },
        m.menu_request_id,
        point.0,
        point.1
    ));
}

#[tauri::command]
fn open_capsule_menu(state: State<'_, Arc<Shared>>) {
    if let Some(point) = native::cursor() {
        request_menu(&mut state.inner.lock().unwrap(), point, MenuSource::Capsule);
    }
}

#[tauri::command]
fn exit_app(app: AppHandle) {
    app.exit(0);
}

#[tauri::command]
fn toggle_credit(state: State<'_, Arc<Shared>>, app: AppHandle) -> bool {
    let mut m = state.inner.lock().unwrap();
    if !m.detail_visible || active_credits(&m).is_empty() {
        return false;
    }
    m.credit_open = !m.credit_open;
    if !m.credit_open {
        native::hide(m.windows[2]);
        m.credit_visible = false;
    }
    let open = m.credit_open;
    drop(m);
    let _ = app.emit("state-updated", ());
    open
}

#[tauri::command]
fn start_drag(state: State<'_, Arc<Shared>>) {
    let mut m = state.inner.lock().unwrap();
    if !m.capsule_visible {
        return;
    }
    let (Some(cursor), Some(window), Some(size)) = (
        native::cursor(),
        native::position(m.windows[0]),
        native::client_size(m.windows[0]),
    ) else {
        return;
    };
    hide_flyouts(&mut m);
    m.drag = Some(Drag {
        cursor,
        window,
        last: window,
        size,
        start_dpi: native::dpi(m.windows[0]),
    });
    diagnostics::record(format!(
        "drag_start cursor={cursor:?} window={window:?} size={size:?}"
    ));
}

#[tauri::command]
fn stop_drag(state: State<'_, Arc<Shared>>) {
    let mut m = state.inner.lock().unwrap();
    let settings = finish_drag(&mut m);
    drop(m);
    if let Some(settings) = settings {
        let _ = settings::save(&settings);
    }
}

fn main() {
    use windows::core::w;
    use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS};
    use windows::Win32::System::Threading::CreateMutexW;
    let mutex = unsafe { CreateMutexW(None, true, w!("Local\\CodexBadge.SingleInstance")) }
        .expect("single-instance mutex failed");
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        unsafe {
            let _ = CloseHandle(mutex);
        }
        return;
    }
    diagnostics::init();
    tauri::Builder::default()
        .setup(|app| {
            let shared = Arc::new(Shared::default());
            {
                let mut m = shared.inner.lock().unwrap();
                m.settings = settings::load();
            }
            if let Some(icon) = app.default_window_icon().cloned() {
                let tray_shared = shared.clone();
                TrayIconBuilder::new()
                    .icon(icon)
                    .tooltip("Codex Badge")
                    .show_menu_on_left_click(false)
                    .on_tray_icon_event(move |_tray, event| {
                        if let TrayIconEvent::Click {
                            button: MouseButton::Right,
                            button_state,
                            position,
                            ..
                        } = event
                        {
                            diagnostics::record(format!(
                                "tray_right state={button_state:?} position={},{}",
                                position.x, position.y
                            ));
                            if button_state != MouseButtonState::Up {
                                return;
                            }
                            let mut m = tray_shared.inner.lock().unwrap();
                            let cursor = (position.x as i32, position.y as i32);
                            request_menu(&mut m, cursor, MenuSource::Tray);
                        }
                    })
                    .build(app)?;
            }
            app.manage(shared.clone());
            thread::spawn({
                let s = shared.clone();
                let handle = app.handle().clone();
                move || track(s, handle)
            });
            thread::spawn({
                let s = shared.clone();
                move || probe_anchor(s)
            });
            let handle = app.handle().clone();
            thread::spawn(move || quota::run(shared, handle));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_state,
            report_capsule_layout,
            window_ready,
            cycle_theme,
            reset_position,
            toggle_startup,
            toggle_topmost,
            hide_menu,
            exit_app,
            toggle_credit,
            start_drag,
            stop_drag,
            open_capsule_menu
        ])
        .build(tauri::generate_context!())
        .expect("Codex Badge failed")
        .run(|_app, event| {
            if let tauri::RunEvent::ExitRequested {
                code: None, api, ..
            } = event
            {
                api.prevent_exit();
            }
        });
    unsafe {
        let _ = CloseHandle(mutex);
    }
}

#[cfg(test)]
mod stability_tests {
    use super::*;
    #[test]
    fn resize_rebases_existing_anchor_without_touching_global_settings() {
        let mut m = Model::default();
        m.anchor = Some((344, 900));
        m.anchor_frame = Some((0, 0, 1200, 900));
        m.last_host_frame = Some((0, 0, 1200, 900));
        m.settings.always_on_top = true;
        m.settings.global_position = Some(domain::GlobalPosition {
            device: "saved".into(),
            x_dip: 17.0,
            y_dip: 25.0,
        });
        update_host_frame(&mut m, Some((200, 100, 1400, 1000)));
        assert_eq!(m.anchor, Some((544, 1100)));
        assert_eq!(m.anchor_generation, 1);
        assert_eq!(
            m.settings
                .global_position
                .as_ref()
                .map(|p| (p.x_dip, p.y_dip)),
            Some((17.0, 25.0))
        );
        update_host_frame(&mut m, None);
        update_host_frame(&mut m, Some((200, 100, 1400, 1000)));
        assert_eq!(m.anchor_generation, 1);
        assert_eq!(m.anchor, Some((544, 1100)));
    }

    #[test]
    fn uia_result_for_new_frame_is_never_translated_a_second_time() {
        let mut m = Model::default();
        m.last_host_frame = Some((0, 0, 1200, 900));
        m.anchor = Some((544, 1100));
        m.anchor_frame = Some((200, 100, 1400, 1000));
        update_host_frame(&mut m, Some((200, 100, 1400, 1000)));
        assert_eq!(m.anchor, Some((544, 1100)));
    }

    #[test]
    fn stale_anchor_samples_leave_current_point_and_frame_unchanged() {
        let mut m = Model::default();
        m.owner = 42;
        m.anchor_generation = 3;
        m.anchor = Some((344, 900));
        let old = (0, 0, 1200, 900);
        let new = (200, 100, 1400, 1000);
        m.anchor_frame = Some(old);
        let now = Instant::now();
        for (owner, generation, frame, current) in [
            (43, 3, Some(new), Some(new)),
            (42, 2, Some(new), Some(new)),
            (42, 3, Some(old), Some(new)),
            (42, 3, None, None),
        ] {
            assert!(!commit_anchor_sample(
                &mut m,
                AnchorSample {
                    owner,
                    generation,
                    frame,
                    point: Some((544, 1100))
                },
                current,
                now,
                now + Duration::from_secs(2),
            ));
            assert_eq!(m.anchor, Some((344, 900)));
            assert_eq!(m.anchor_frame, Some(old));
        }
    }

    #[test]
    fn fresh_sample_committed_before_geometry_update_keeps_its_frame() {
        let mut m = Model::default();
        m.owner = 42;
        m.anchor = Some((344, 900));
        m.anchor_frame = Some((0, 0, 1200, 900));
        m.last_host_frame = m.anchor_frame;
        let new = (200, 100, 1400, 1000);
        let now = Instant::now();
        assert!(commit_anchor_sample(
            &mut m,
            AnchorSample {
                owner: 42,
                generation: 0,
                frame: Some(new),
                point: Some((544, 1100))
            },
            Some(new),
            now,
            now + Duration::from_secs(2),
        ));
        update_host_frame(&mut m, Some(new));
        assert_eq!(m.anchor, Some((544, 1100)));
        assert_eq!(m.anchor_frame, Some(new));
    }

    #[test]
    fn backend_rejects_a_stale_normal_double_click_after_topmost_was_enabled() {
        let mut m = Model::default();
        m.settings.always_on_top = true;
        m.settings.global_position = Some(domain::GlobalPosition {
            device: "saved".into(),
            x_dip: 17.0,
            y_dip: 25.0,
        });
        assert!(reset_position_model(&mut m, true).is_none());
        assert_eq!(
            m.settings
                .global_position
                .as_ref()
                .map(|p| (p.x_dip, p.y_dip)),
            Some((17.0, 25.0))
        );
        m.settings.always_on_top = false;
        m.settings.offset_x = 42.0;
        m.settings.offset_y = -9.0;
        assert!(reset_position_model(&mut m, true).is_some());
        assert_eq!((m.settings.offset_x, m.settings.offset_y), (0.0, 0.0));
        assert_eq!(
            m.settings
                .global_position
                .as_ref()
                .map(|p| (p.x_dip, p.y_dip)),
            Some((17.0, 25.0))
        );
    }
    #[test]
    fn global_capsule_menu_avoids_the_capsule_at_its_opening_point() {
        let mut m = Model::default();
        m.settings.always_on_top = true;
        m.last_capsule = Some((300, 300, 71, 30));
        request_menu(&mut m, (330, 313), MenuSource::Capsule);
        assert!(!domain::rects_intersect(
            m.menu_target.unwrap(),
            m.last_capsule.unwrap()
        ));
    }
    #[test]
    fn failed_badge_frame_does_not_skip_tray_processing() {
        let calls = std::cell::RefCell::new(Vec::new());
        run_tracking_frame(
            || {
                calls.borrow_mut().push("badge_failed");
            },
            || {
                calls.borrow_mut().push("tray");
            },
        );
        assert_eq!(*calls.borrow(), ["badge_failed", "tray"]);
    }
    #[test]
    fn forgetting_capsule_invalidates_hwnd_bound_cache_not_saved_position() {
        let mut m = Model::default();
        m.windows[0] = 123;
        m.ready[0] = true;
        m.last_capsule = Some((1, 2, 71, 30));
        m.created_at[0] = Some(Instant::now());
        forget_windows(&mut m, 0..3);
        assert_eq!(m.last_capsule, None);
        assert_eq!(m.windows[0], 0);
        assert!(!m.ready[0]);
        assert_eq!(m.created_at[0], None);
    }
    #[test]
    fn failed_initialization_cleans_only_new_resources() {
        let removed = std::cell::Cell::new(false);
        assert!(finish_initialization(Err::<(), _>("denied"), true, || removed.set(true)).is_err());
        assert!(removed.get());
        removed.set(false);
        assert!(finish_initialization(Ok::<(), &str>(()), true, || removed.set(true)).is_ok());
        assert!(!removed.get());
        assert!(
            finish_initialization(Err::<(), _>("denied"), false, || removed.set(true)).is_err()
        );
        assert!(!removed.get());
    }
    #[test]
    fn popup_configs_never_request_initial_focus() {
        let config: tauri::Config =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(config.app.windows.len(), 4);
        assert!(config
            .app
            .windows
            .iter()
            .all(|w| !w.focus && !w.focusable && !w.visible && !w.create));
    }
    #[test]
    fn normal_backend_opens_capsule_menu_and_old_close_cannot_hide_new_request() {
        let mut m = Model::default();
        request_menu(&mut m, (300, 300), MenuSource::Capsule);
        assert!(m.menu_target.is_some());
        for _ in 0..50 {
            request_menu(&mut m, (300, 300), MenuSource::Tray);
        }
        assert_eq!(m.menu_request_id, 51);
        assert!(!m.menu_entered);
        close_menu(&mut m, 50);
        assert!(m.menu_target.is_some());
        close_menu(&mut m, 51);
        assert!(m.menu_target.is_none());
    }

    #[test]
    fn pending_menu_prevents_hover_detail_and_close_requires_leave_then_reenter() {
        let mut m = Model::default();
        request_menu(&mut m, (300, 300), MenuSource::Tray);
        assert!(!detail_hover_inside(&mut m, true, false, false));
        m.menu_visible = true;
        m.menu_target = None;
        assert!(!detail_hover_inside(&mut m, true, false, false));
        let request_id = m.menu_request_id;
        close_menu(&mut m, request_id);
        assert!(!detail_hover_inside(&mut m, true, false, false));
        assert!(!detail_hover_inside(&mut m, false, false, false));
        assert!(detail_hover_inside(&mut m, true, false, false));
    }

    #[test]
    fn automatic_menu_close_also_blocks_detail_until_hover_rearms() {
        let now = Instant::now();
        let mut m = Model::default();
        m.menu_visible = true;
        m.menu_entered = true;
        m.menu_shown_at = Some(now - Duration::from_secs(2));
        m.menu_left_at = Some(now - Duration::from_millis(301));
        update_menu_departure(&mut m, false, now);
        assert!(!m.menu_visible);
        assert!(!detail_hover_inside(&mut m, true, false, false));
        assert!(!detail_hover_inside(&mut m, false, false, false));
        assert!(detail_hover_inside(&mut m, true, false, false));
    }

    #[test]
    fn failed_menu_show_expires_without_timing_out_open_or_newer_menu() {
        let mut m = Model::default();
        request_menu(&mut m, (300, 300), MenuSource::Tray);
        let first = m.menu_requested_at.unwrap();
        assert!(!expire_pending_menu(&mut m, first + Duration::from_secs(9)));
        assert!(m.menu_target.is_some());
        assert!(expire_pending_menu(&mut m, first + Duration::from_secs(10)));
        assert!(m.menu_target.is_none());
        assert!(!detail_hover_inside(&mut m, true, false, false));
        assert!(!detail_hover_inside(&mut m, false, false, false));
        assert!(detail_hover_inside(&mut m, true, false, false));

        request_menu(&mut m, (300, 300), MenuSource::Tray);
        m.menu_requested_at = Some(first + Duration::from_secs(8));
        assert!(!expire_pending_menu(
            &mut m,
            first + Duration::from_secs(11)
        ));
        assert!(m.menu_target.is_some());
        m.menu_target = None;
        m.menu_visible = true;
        assert!(!expire_pending_menu(
            &mut m,
            first + Duration::from_secs(60)
        ));
        assert!(
            m.menu_visible,
            "displayed menus have no pending-request deadline"
        );
    }

    #[test]
    fn hover_detail_comparison_can_disable_only_the_detail_trigger() {
        let mut m = Model::default();
        m.hover_detail_enabled = false;
        assert!(!detail_hover_inside(&mut m, true, false, false));
        m.hover_detail_enabled = true;
        assert!(detail_hover_inside(&mut m, true, false, false));
    }

    #[test]
    fn host_exit_closes_capsule_menu_but_waiting_tray_menu_remains_available() {
        for source in [MenuSource::Capsule, MenuSource::Tray] {
            let mut m = Model::default();
            request_menu(&mut m, (300, 300), source);
            m.windows[0] = 1;
            m.menu_visible = true;
            release_badge_model(&mut m);
            assert_eq!(m.windows[0], 0);
            assert_eq!(m.menu_visible, source == MenuSource::Tray);
            assert_eq!(m.menu_target.is_some(), source == MenuSource::Tray);
        }
    }

    #[test]
    fn capsule_menu_keeps_open_on_capsule_and_gap_without_keeping_broad_empty_space() {
        let capsule = (300, 300, 71, 30);
        let menu = (181, 146, 190, 148);
        assert!(domain::menu_pointer_inside((320, 310), menu, Some(capsule)));
        assert!(domain::menu_pointer_inside((320, 297), menu, Some(capsule)));
        assert!(domain::menu_pointer_inside((200, 200), menu, Some(capsule)));
        assert!(!domain::menu_pointer_inside(
            (200, 310),
            menu,
            Some(capsule)
        ));
        assert!(
            !domain::menu_pointer_inside((320, 297), menu, None),
            "tray has no capsule corridor"
        );
        let side_menu = (377, 300, 190, 148);
        assert!(domain::menu_pointer_inside(
            (374, 310),
            side_menu,
            Some(capsule)
        ));
        let now = Instant::now();
        let mut m = Model::default();
        m.menu_visible = true;
        m.menu_shown_at = Some(now);
        update_menu_departure(
            &mut m,
            domain::menu_pointer_inside((320, 310), menu, Some(capsule)),
            now + Duration::from_secs(5),
        );
        assert!(m.menu_visible, "no timeout while pointer rests on capsule");
        update_menu_departure(&mut m, false, now + Duration::from_secs(6));
        assert!(m.menu_visible);
        update_menu_departure(&mut m, false, now + Duration::from_millis(6301));
        assert!(!m.menu_visible);
    }

    #[test]
    fn disabling_global_mode_obeys_live_hidden_minimized_missing_and_visible_owner() {
        let _window_guard = native::WINDOW_TEST_LOCK.lock().unwrap();
        use windows::core::w;
        use windows::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, ShowWindow, SW_SHOWMINNOACTIVE, WS_EX_NOACTIVATE,
            WS_EX_TOOLWINDOW, WS_POPUP,
        };
        struct TestWindow(windows::Win32::Foundation::HWND);
        impl TestWindow {
            fn new() -> Self {
                Self(unsafe {
                    CreateWindowExW(
                        WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                        w!("STATIC"),
                        w!("Badge mode regression"),
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
                    .unwrap()
                })
            }
            fn raw(&self) -> isize {
                self.0 .0 as isize
            }
        }
        impl Drop for TestWindow {
            fn drop(&mut self) {
                unsafe {
                    let _ = DestroyWindow(self.0);
                }
            }
        }
        let host = TestWindow::new();
        let pill = TestWindow::new();
        for owner in [host.raw(), 0] {
            native::show(pill.raw());
            apply_badge_mode([pill.raw(), 0, 0, 0], owner, true).unwrap();
            assert!(native::is_visible(pill.raw()));
            apply_badge_mode([pill.raw(), 0, 0, 0], owner, false).unwrap();
            assert!(
                !native::is_visible(pill.raw()),
                "normal mode must not restore old visibility"
            );
            assert!(!native::is_topmost(pill.raw()));
            assert_eq!(native::owner(pill.raw()), owner);
        }
        unsafe {
            let _ = ShowWindow(host.0, SW_SHOWMINNOACTIVE);
        }
        assert!(native::is_minimized(host.raw()));
        native::show(pill.raw());
        apply_badge_mode([pill.raw(), 0, 0, 0], host.raw(), true).unwrap();
        apply_badge_mode([pill.raw(), 0, 0, 0], host.raw(), false).unwrap();
        assert!(!native::is_visible(pill.raw()));
        native::show(host.raw());
        assert!(!native::is_minimized(host.raw()));
        native::show(pill.raw());
        apply_badge_mode([pill.raw(), 0, 0, 0], host.raw(), true).unwrap();
        apply_badge_mode([pill.raw(), 0, 0, 0], host.raw(), false).unwrap();
        assert!(native::is_visible(pill.raw()));
    }
}
