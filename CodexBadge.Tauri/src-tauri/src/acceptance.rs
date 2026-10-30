//! Isolated desktop acceptance harness: no production mutex, tray, production settings writes,
//! account mutations or host tracking. The actual create_window/render/input path is reused.
use super::*;

#[test]
#[ignore = "interactive desktop preview with isolated WebView profile"]
fn desktop_preview() {
    // Theme selection and playback acknowledgement write only into this sandbox.
    let preview_data = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../artifacts/desktop-preview-data");
    std::env::set_var("LOCALAPPDATA", &preview_data);
    diagnostics::init();
    let profile = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../artifacts/desktop-preview-profile");
    let mut context = tauri::generate_context!();
    for config in &mut context.config_mut().app.windows {
        config.data_directory = Some(profile.clone());
    }
    let shared = Arc::new(Shared::default());
    {
        let mut m = shared.inner.lock().unwrap();
        m.settings.always_on_top = true;
        if let Ok(theme) = std::env::var("BADGE_PREVIEW_THEME") {
            m.settings.theme = serde_json::from_value(json!(theme)).expect("preview theme");
        }
        m.settings.celebration_state = celebration::CelebrationState::existing_install();
        m.quota_mode = "weekly".into();
        m.plan_label = Some("PRO".into());
        m.quota_status.clear();
        m.snapshot = Some(domain::parse_quota(
            &json!({"rateLimits":{"primary":{"usedPercent":31,"windowDurationMins":10080,"resetsAt":epoch_now()+86400}},"rateLimitResetCredits":{"availableCount":1,"credits":[{"id":"simulation","status":"available","expiresAt":epoch_now()+86400*4}]}}),
            epoch_now(),
        ));
        if std::env::var("BADGE_PREVIEW_RESET").as_deref() == Ok("1") {
            m.account_key = Some(1);
            let before = m.snapshot.clone().unwrap();
            m.settings.celebration_state.observe(&before, Some(1));
        }
    }
    if std::env::var("BADGE_PREVIEW_LIVE_ACCOUNT").as_deref() == Ok("1") {
        let (plan, snapshot) = quota::live_readonly_sample().expect("read-only live quota");
        let mut m = shared.inner.lock().unwrap();
        m.plan_label = plan;
        m.quota_mode = domain::quota_mode(Some(&snapshot)).into();
        m.snapshot = Some(snapshot);
    }
    let setup_shared = shared.clone();
    let app = tauri::Builder::default()
        .any_thread()
        .setup(move |app| {
            app.manage(setup_shared.clone());
            app.manage(update_window::Controller::default());
            app.manage(authorization_window::Controller::default());
            if std::env::var("BADGE_PREVIEW_AUTHORIZATION").as_deref()==Ok("1") {
                let handle=app.handle().clone();
                thread::spawn(move || {
                    let decision=authorization_window::confirm(&handle,"demo@example.test (PLUS)");
                    println!("authorization_preview decision={decision:?}; no permit saved, no RPC sent");
                    authorization_window::finish(&handle,None);
                });
            }
            // A normal owner window makes nonactivating tool windows visible to
            // desktop inspection tools and supplies a reproducible blur target.
            let background = r#"<!doctype html><meta charset="utf-8"><style>
                html{height:100%;background:linear-gradient(120deg,#c5e9ff,#b8c9f3 45%,#eec5cd 75%,#f5d5ab)}
                body{margin:0;height:100%;font:14px Segoe UI;color:#29445f;background:repeating-linear-gradient(90deg,transparent 0 18px,#ffffff60 18px 20px)}
                </style><div style="position:absolute;bottom:24px;left:30px">重置烟花 · 原生窗口预览<br><small>模拟额度 · 原有主题 · 独立数据目录</small></div>"#;
            let background = if std::env::var("BADGE_PREVIEW_BACKGROUND").as_deref() == Ok("dark") {
                background.replace("#c5e9ff,#b8c9f3 45%,#eec5cd 75%,#f5d5ab", "#05080e,#13233e 45%,#332039 75%,#090d14")
                    .replace("color:#29445f", "color:#e4edf9")
            } else { background.to_string() };
            let backdrop = tauri::WebviewWindowBuilder::new(app, "preview-backdrop",
                tauri::WebviewUrl::External("about:blank".parse().unwrap()))
                .title("Codex Badge Reset Preview")
                .inner_size(620.0, 380.0).position(540.0, 300.0)
                .always_on_top(true)
                .build()?;
            backdrop.eval(format!("document.documentElement.innerHTML={}", serde_json::to_string(&background).unwrap()))?;
            let preview_owner = backdrop.hwnd()?.0 as isize;
            native::move_only(preview_owner, 540, 300);
            for label in ["capsule", "detail", "credit"] {
                create_window(app.handle(), &setup_shared, label).map_err(std::io::Error::other)?;
                let window = app.get_webview_window(label).unwrap();
                native::bind_owner(window.hwnd()?.0 as isize, preview_owner);
            }
            let preview_menu = if std::env::var("BADGE_PREVIEW_MENU").as_deref() == Ok("1") {
                Some(
                    popup::create(
                        windows::Win32::Foundation::HWND(
                            setup_shared.inner.lock().unwrap().windows[0] as _,
                        ),
                        Arc::new(tray::Checks {
                            palette: std::sync::atomic::AtomicUsize::new(
                                std::env::var("BADGE_PREVIEW_PALETTE")
                                    .ok()
                                    .and_then(|v| v.parse().ok())
                                    .unwrap_or(0),
                            ),
                            update_available: AtomicBool::new(true),
                            ..tray::Checks::default()
                        }),
                        Arc::new(|_| {}),
                        Arc::new(AtomicBool::new(false)),
                    )
                    .map_err(std::io::Error::other)?,
                )
            } else {
                None
            };
            if std::env::var("BADGE_PREVIEW_UPDATE").as_deref() == Ok("1") {
                update_window::preview(app.handle()).map_err(std::io::Error::other)?;
            }
            let handle = app.handle().clone();
            let s = setup_shared.clone();
            thread::spawn(move || {
                let duration = std::env::var("BADGE_PREVIEW_SECONDS")
                    .ok()
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(45)
                    .min(180);
                for tick in 0..duration * 5 {
                    let s = s.clone();
                    let a = handle.clone();
                    let _ = handle.run_on_main_thread(move || {
                        if let Some(menu) = preview_menu {
                            popup::show(
                                menu,
                                popup::Open {
                                    point: (970, 450),
                                    avoid: None,
                                    settings: tick >= 25,
                                    tray_anchor: None,
                                },
                            );
                        }
                        if std::env::var("BADGE_PREVIEW_UPDATE").as_deref()==Ok("1") && tick%25==0 {
                            let c=a.state::<update_window::Controller>();let u=c.0.lock().unwrap();
                            println!("update_preview tick={tick} session={} shown={} visible={} checking={} known={:?}",u.session,u.shown_session,u.visible,u.checking,u.known_version);
                        }
                        let mut m = s.inner.lock().unwrap();
                        if std::env::var("BADGE_PREVIEW_RESET").as_deref() == Ok("1") && [75, 90, 150, 165, 180].contains(&tick) {
                            let mut snapshot = m.snapshot.clone().unwrap();
                            snapshot.fetched_at = epoch_now();
                            snapshot.weekly.as_mut().unwrap().remaining = if tick == 150 { 69.0 } else { 100.0 };
                            m.settings.celebration_state.observe(&snapshot, Some(1));
                            m.snapshot = Some(snapshot);
                            println!("preview_reset tick={tick} pending={}", m.settings.celebration_state.peek(Some(1), epoch_now()).is_some());
                            let _ = a.emit("state-updated", ());
                        }
                        if std::env::var("BADGE_PREVIEW_HOVER").as_deref() == Ok("1") {
                            if tick == 0 {
                                m.settings.global_position = native::global_position((600, 390));
                            }
                            let previous = (m.detail_visible, m.detail_session);
                            layout(&mut m, &a);
                            if previous != (m.detail_visible, m.detail_session) {
                                println!("preview_hover tick={tick} visible={} session={}", m.detail_visible, m.detail_session);
                                let _ = a.emit("state-updated", ());
                            }
                            if s.quota_refresh.swap(false, Ordering::AcqRel) {
                                println!("preview_hover_refresh tick={tick}");
                            }
                            return;
                        }
                        let dpi = native::dpi(m.windows[1]);
                        let p = |v| domain::dip_to_px(v, dpi);
                        let area = native::monitor_at((600, 450)).unwrap().area;
                        let detail = (
                            600,
                            450,
                            p(270.0),
                            p(m.detail_content_height.unwrap_or(150.0)),
                        );
                        if m.ready[1] {
                            resize_move(
                                &a,
                                "detail",
                                m.windows[1],
                                detail.0,
                                detail.1,
                                detail.2,
                                detail.3,
                            );
                            native::show(m.windows[1]);
                            m.detail_visible = true;
                            m.detail_session = 1;
                        }
                        if m.ready[0] {
                            let (w,h)=if std::env::var("BADGE_PREVIEW_ROUND").as_deref()==Ok("1") {
                                m.settings.always_on_top=false;
                                let _=a.emit("state-updated",());
                                (p(28.8),p(28.8))
                            } else {(p(72.0),p(34.0))};
                            resize_move(&a, "capsule", m.windows[0], 600, 390, w, h);
                            native::round_capsule(m.windows[0], w, h);
                            native::show(m.windows[0]);
                            m.capsule_visible = true;
                        }
                        if tick == 50 {
                            m.credit_open = true;
                            let _ = a.emit("state-updated", ());
                        }
                        if m.ready[2] && m.credit_open {
                            if let Some(rect) = domain::place_vertical_popup(
                                detail,
                                (p(205.0), p(38.0)),
                                area,
                                &[detail, (600, 390, p(72.0), p(34.0))],
                                p(6.0),
                            ) {
                                resize_move(
                                    &a,
                                    "credit",
                                    m.windows[2],
                                    rect.0,
                                    rect.1,
                                    rect.2,
                                    rect.3,
                                );
                                native::show(m.windows[2]);
                                m.credit_visible = true;
                            }
                        }
                    });
                    thread::sleep(Duration::from_millis(200));
                }
                handle.exit(0);
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            authorization_window::get_authorization_state,
            authorization_window::authorization_ready,
            authorization_window::answer_authorization,
            get_state,
            window_ready,
            report_detail_height,
            report_capsule_layout,
            sync_webview_scale,
            set_input_regions,
            cycle_theme,
            toggle_credit,
            update_window::get_update_state,
            update_window::update_ready,
            update_window::close_update,
            update_window::retry_update,
            update_window::drag_update,
            update_window::move_update_drag,
            update_window::end_update_drag,
            updates::open_release,
            celebration_window::request_celebration,
            celebration_window::celebration_ready,
            celebration_window::celebration_started,
            celebration_window::celebration_failed
        ])
        .build(context)
        .expect("isolated preview builds");
    app.run_return(|_, _| {});
    assert!(shared.inner.lock().unwrap().detail_content_height.is_some());
}
