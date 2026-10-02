//! Isolated desktop acceptance harness: no production mutex, tray, settings writes,
//! account mutations or host tracking. The actual create_window/render/input path is reused.
use super::*;

#[test]
#[ignore = "interactive desktop preview with isolated WebView profile"]
fn desktop_preview() {
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
        m.settings.celebration_state = celebration::CelebrationState::existing_install();
        m.quota_mode = "weekly".into();
        m.plan_label = Some("PRO".into());
        m.quota_status.clear();
        m.snapshot = Some(domain::parse_quota(
            &json!({"rateLimits":{"primary":{"usedPercent":31,"windowDurationMins":10080,"resetsAt":epoch_now()+86400}},"rateLimitResetCredits":{"availableCount":1,"credits":[{"id":"simulation","status":"available","expiresAt":epoch_now()+86400*4}]}}),
            epoch_now(),
        ));
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
            for label in ["capsule", "detail", "credit"] {
                create_window(app.handle(), &setup_shared, label).map_err(std::io::Error::other)?;
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
                            resize_move(&a, "capsule", m.windows[0], 600, 390, p(72.0), p(34.0));
                            native::round_capsule(m.windows[0], p(72.0), p(34.0));
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
            get_state,
            window_ready,
            report_detail_height,
            report_capsule_layout,
            set_input_regions,
            toggle_credit,
            update_window::get_update_state,
            update_window::update_ready,
            update_window::close_update,
            update_window::retry_update,
            update_window::drag_update,
            update_window::move_update_drag,
            update_window::end_update_drag,
            updates::open_release,
            celebration_window::request_celebration
        ])
        .build(context)
        .expect("isolated preview builds");
    app.run_return(|_, _| {});
    assert!(shared.inner.lock().unwrap().detail_content_height.is_some());
}
