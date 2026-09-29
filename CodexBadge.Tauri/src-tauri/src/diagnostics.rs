use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, SyncSender};
use std::sync::OnceLock;
use std::thread;

static SENDER: OnceLock<SyncSender<String>> = OnceLock::new();
static HEALTHY: AtomicBool = AtomicBool::new(false);
static SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub fn healthy() -> bool {
    HEALTHY.load(Ordering::Acquire)
}

// Only known event names, categorical values and numeric fields cross the log boundary.
fn sanitize_event(event: &str) -> String {
    let mut words = event.split_whitespace();
    let name = words.next().unwrap_or("");
    if !matches!(
        name,
        "start"
            | "capsule_hover"
            | "popup_hide"
            | "overlay_message"
            | "host_geometry"
            | "native_show"
            | "native_topmost_failed"
            | "capsule_region"
            | "capsule_layout"
            | "window_observe"
            | "focus_observe"
            | "host_observe"
            | "window_ready"
            | "window_ready_timeout"
            | "window_create_failed"
            | "window_created"
            | "webview_browser_menu_disabled"
            | "tray_right"
            | "menu_request"
            | "menu_raise"
            | "menu_observe"
            | "menu_hide"
            | "menu_resize_pending"
            | "menu_create_failed"
            | "topmost_request"
            | "topmost_failed"
            | "topmost_save_failed"
            | "topmost_applied"
            | "drag_start"
            | "drag_end"
            | "anchor_discover"
            | "lifecycle_start"
            | "lifecycle_release"
            | "app_server_start"
            | "app_server_stop"
            | "app_server_stderr"
            | "quota_rpc"
            | "quota_snapshot"
            | "popup_no_space"
            | "frame_slow"
    ) {
        return "invalid_event".into();
    }
    let mut output = name.to_string();
    for word in words {
        let Some((key, value)) = word.split_once('=') else {
            continue;
        };
        let numeric = matches!(
            key,
            "hwnd"
                | "pid"
                | "tid"
                | "owner"
                | "style"
                | "exstyle"
                | "dpi"
                | "code"
                | "duration_ms"
                | "generation"
                | "request_id"
                | "focus"
                | "foreground"
                | "layout"
                | "rect"
                | "client"
                | "frame"
                | "capsule"
                | "viewport"
                | "position"
                | "target"
                | "origin"
                | "size"
                | "cursor"
                | "actual"
                | "requested"
                | "window"
                | "windows"
                | "thread"
                | "window_style"
                | "started"
                | "elapsed_ms"
        ) && !value.is_empty()
            && value
                .chars()
                .all(|c| c.is_ascii_digit() || "-.,[]()/x".contains(c));
        let category = matches!(
            key,
            "label" | "stage" | "category" | "reason" | "source" | "state" | "ime"
        ) && matches!(
            value,
            "capsule"
                | "detail"
                | "credit"
                | "menu"
                | "initialize"
                | "account/read"
                | "account/rateLimits/read"
                | "service"
                | "authentication"
                | "timeout"
                | "paused"
                | "protocol"
                | "transport"
                | "success"
                | "visible"
                | "iconic"
                | "hidden"
                | "missing"
                | "unknown"
                | "exit"
                | "tray"
                | "Up"
                | "Down"
                | "pointer_exit"
                | "command"
                | "none"
                | "mouseactivate"
                | "ncactivate"
        );
        let boolean = matches!(
            key,
            "visible"
                | "iconic"
                | "topmost"
                | "enabled"
                | "desired"
                | "observed"
                | "accepted"
                | "found"
                | "success"
                | "applied"
                | "cleared"
                | "native_region"
                | "hover_detail"
                | "default_cursor"
                | "entered"
                | "present"
                | "contents_omitted"
                | "snapshot_valid"
                | "retain"
                | "logger"
        ) && matches!(value, "true" | "false");
        if numeric || category || boolean || (key == "code" && value == "none") {
            output.push(' ');
            output.push_str(word);
        }
    }
    output
}

fn write_event(writer: &mut impl Write, event: &str, run: &str, seq: u64) -> std::io::Result<()> {
    writeln!(
        writer,
        "{} pid={} run={} seq={} {}",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f"),
        std::process::id(),
        run,
        seq,
        sanitize_event(event)
    )
}

fn path() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("CodexBadge/tauri-diagnostics.log"))
}

fn open_log(path: &PathBuf) -> std::io::Result<File> {
    fs::create_dir_all(path.parent().ok_or(std::io::ErrorKind::NotFound)?)?;
    if path.metadata().is_ok_and(|entry| entry.len() > 1_000_000) {
        let previous = path.with_extension("log.old");
        let _ = fs::remove_file(&previous);
        let _ = fs::rename(path, previous);
    }
    OpenOptions::new().create(true).append(true).open(path)
}

pub fn init() {
    let Some(path) = path() else {
        eprintln!("diagnostics_init_failed stage=environment code=none");
        return;
    };
    let mut file = match open_log(&path) {
        Ok(file) => file,
        Err(error) => {
            // Numeric error only; no profile paths, account data or raw errors.
            eprintln!(
                "diagnostics_init_failed stage=open code={:?}",
                error.raw_os_error()
            );
            return;
        }
    };
    let run = format!(
        "{}-{}",
        std::process::id(),
        chrono::Local::now().timestamp_millis()
    );
    let (sender, receiver) = mpsc::sync_channel(1024);
    if SENDER.set(sender).is_err() {
        return;
    }
    HEALTHY.store(true, Ordering::Release);
    thread::spawn(move || {
        while let Ok(event) = receiver.recv() {
            let seq = SEQUENCE.fetch_add(1, Ordering::Relaxed) + 1;
            if let Err(error) = write_event(&mut file, &event, &run, seq) {
                eprintln!("diagnostics_write_failed code={:?}", error.raw_os_error());
                break;
            }
            if file.metadata().is_ok_and(|entry| entry.len() > 1_000_000) {
                drop(file);
                match open_log(&path) {
                    Ok(next) => file = next,
                    Err(error) => {
                        eprintln!(
                            "diagnostics_write_failed stage=rotate code={:?}",
                            error.raw_os_error()
                        );
                        break;
                    }
                }
            }
        }
        HEALTHY.store(false, Ordering::Release);
    });
    record(format!(
        "start native_region={} hover_detail={} default_cursor={}",
        !no_native_region(),
        !no_hover_detail(),
        default_cursor()
    ));
}

pub fn no_native_region() -> bool {
    std::env::args_os().any(|arg| arg == "--diagnose-no-native-region")
}

pub fn no_hover_detail() -> bool {
    std::env::args_os().any(|arg| arg == "--diagnose-no-hover-detail")
}

pub fn default_cursor() -> bool {
    std::env::args_os().any(|arg| arg == "--diagnose-default-cursor")
}

pub fn record(event: impl Into<String>) {
    if let Some(sender) = SENDER.get() {
        let _ = sender.try_send(event.into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_boundary_drops_raw_errors_paths_and_untrusted_fields() {
        let event = sanitize_event("window_create_failed label=detail error=SECRET_TOKEN account@example.test path=C:\\Users\\secret token=123 code=-5 duration_ms=42");
        assert_eq!(
            event,
            "window_create_failed label=detail code=-5 duration_ms=42"
        );
        assert_eq!(sanitize_event("SECRET_TOKEN token=123"), "invalid_event");
    }

    #[test]
    fn log_write_failure_is_reported_to_caller() {
        struct Denied;
        impl std::io::Write for Denied {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::ErrorKind::PermissionDenied.into())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        assert!(write_event(&mut Denied, "start", "run", 1).is_err());
    }
}
