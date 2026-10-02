use crate::{diagnostics, domain, Shared};
use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter};
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};

fn celebration_account_key(account: Option<u64>, response: &Value) -> Option<u64> {
    use std::hash::{Hash, Hasher};
    let account = account?;
    let (source, limits) = if let Some(all) = response
        .get("rateLimitsByLimitId")
        .and_then(Value::as_object)
    {
        ("by_id", all.get("codex")?)
    } else {
        ("legacy", response.get("rateLimits")?)
    };
    let plan = limits.get("planType")?.as_str()?;
    if plan.is_empty() || plan == "unknown" {
        return None;
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (
        account,
        source,
        plan,
        limits.get("limitId"),
        limits.get("normalModelSlug"),
    )
        .hash(&mut hasher);
    Some(hasher.finish())
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn codex_exe() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("CODEX_BADGE_CLI") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
    }
    if let Some(dirs) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&dirs) {
            let path = dir.join("codex.exe");
            if path.is_file() {
                return Some(path);
            }
        }
    }
    let base = PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("OpenAI/Codex/bin");
    let mut found = Vec::new();
    for entry in fs::read_dir(base).ok()?.flatten() {
        let path = entry.path().join("codex.exe");
        if path.is_file() {
            found.push(path);
        }
    }
    found.sort_by_key(|path| fs::metadata(path).and_then(|m| m.modified()).ok());
    found.pop()
}

struct Server {
    child: Child,
    job: Option<HANDLE>,
    input: ChildStdin,
    lines: mpsc::Receiver<String>,
    next_id: u64,
    pending_update: bool,
    shared: Arc<Shared>,
    generation: u64,
}

impl Server {
    fn start(shared: Arc<Shared>) -> Result<Self, String> {
        let generation = shared.quota_generation.load(Ordering::Acquire);
        let path = codex_exe().ok_or("codex.exe not found")?;
        diagnostics::record(format!("app_server_start cli={}", path.display()));
        let mut child = Command::new(path)
            .args(["app-server", "--stdio"])
            .creation_flags(0x0800_0000)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| e.to_string())?;
        let job = unsafe { CreateJobObjectW(None, None) }
            .ok()
            .and_then(|job| {
                let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
                limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                let assigned = unsafe {
                    SetInformationJobObject(
                        job,
                        JobObjectExtendedLimitInformation,
                        &limits as *const _ as *const _,
                        std::mem::size_of_val(&limits) as u32,
                    )
                    .and_then(|_| AssignProcessToJobObject(job, HANDLE(child.as_raw_handle())))
                }
                .is_ok();
                if assigned {
                    Some(job)
                } else {
                    unsafe {
                        let _ = CloseHandle(job);
                    }
                    None
                }
            });
        let input = child.stdin.take().ok_or("missing stdin")?;
        let output = child.stdout.take().ok_or("missing stdout")?;
        let errors = child.stderr.take().ok_or("missing stderr")?;
        let (sender, lines) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                match line {
                    Ok(line) => {
                        if sender.send(line).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });
        thread::spawn(move || {
            let mut recorded = false;
            for _ in BufReader::new(errors).lines() {
                if !recorded {
                    diagnostics::record("app_server_stderr present=true contents_omitted=true");
                    recorded = true;
                }
            }
        });
        let mut server = Self {
            child,
            job,
            input,
            lines,
            next_id: 0,
            pending_update: false,
            shared,
            generation,
        };
        server.request("initialize", json!({"clientInfo":{"name":"codex_badge_tauri","title":"Codex Badge","version":env!("CARGO_PKG_VERSION")}}))?;
        server.send(&json!({"method":"initialized","params":{}}))?;
        let account = server.request("account/read", json!({"refreshToken":false}))?;
        if account.get("account").is_none_or(Value::is_null)
            && account.get("requiresOpenaiAuth").and_then(Value::as_bool) == Some(true)
        {
            return Err("authentication unavailable".into());
        }
        Ok(server)
    }

    fn send(&mut self, value: &Value) -> Result<(), String> {
        writeln!(self.input, "{value}")
            .and_then(|_| self.input.flush())
            .map_err(|e| e.to_string())
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let started = Instant::now();
        let result = self.request_inner(method, params);
        diagnostics::record(format!(
            "{} generation={}",
            rpc_diagnostic(
                method,
                result.as_ref().map(|_| ()).map_err(|error| error.as_str()),
                started.elapsed().as_millis(),
            ),
            self.generation
        ));
        result
    }

    fn request_inner(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.next_id += 1;
        let id = self.next_id;
        self.send(&json!({"id":id,"method":method,"params":params}))?;
        let deadline =
            Instant::now() + Duration::from_secs(if method == "initialize" { 10 } else { 30 });
        receive_response(
            &self.lines,
            &self.shared.quota_active,
            &self.shared.quota_generation,
            self.generation,
            id,
            deadline,
            &mut self.pending_update,
        )
    }

    fn update_notification(&self, timeout: Duration) -> Result<bool, String> {
        match self.lines.recv_timeout(timeout) {
            Ok(line) => Ok(serde_json::from_str::<Value>(&line)
                .ok()
                .and_then(|v| v.get("method").and_then(Value::as_str).map(str::to_owned))
                .is_some_and(|method| {
                    matches!(
                        method.as_str(),
                        "account/rateLimits/updated" | "account/updated"
                    )
                })),
            Err(mpsc::RecvTimeoutError::Timeout) => Ok(false),
            Err(_) => Err("app-server output closed".into()),
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(job) = self.job {
            unsafe {
                let _ = CloseHandle(job);
            }
        }
    }
}

fn receive_response(
    lines: &mpsc::Receiver<String>,
    active: &AtomicBool,
    generation: &AtomicU64,
    expected_generation: u64,
    id: u64,
    deadline: Instant,
    pending_update: &mut bool,
) -> Result<Value, String> {
    loop {
        if !active.load(Ordering::Acquire)
            || generation.load(Ordering::Acquire) != expected_generation
        {
            return Err("service paused".into());
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("request timed out".into());
        }
        let line = match lines.recv_timeout(remaining.min(Duration::from_millis(250))) {
            Ok(line) => line,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(_) => return Err("app-server output closed".into()),
        };
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if value
            .get("method")
            .and_then(Value::as_str)
            .is_some_and(|method| {
                matches!(method, "account/rateLimits/updated" | "account/updated")
            })
        {
            *pending_update = true;
            continue;
        }
        if value.get("id").and_then(Value::as_u64) != Some(id) {
            continue;
        }
        if let Some(error) = value.get("error") {
            return Err(error.to_string());
        }
        return value.get("result").cloned().ok_or("missing result".into());
    }
}

pub fn run(shared: Arc<Shared>, app: AppHandle) {
    let mut failures = 0usize;
    loop {
        while !shared.quota_active.load(Ordering::Acquire) {
            failures = 0;
            thread::sleep(Duration::from_millis(250));
        }
        set_status(&shared, &app, "正在连接额度服务");
        let generation = shared.quota_generation.load(Ordering::Acquire);
        let mut server = match Server::start(shared.clone()) {
            Ok(server) => server,
            Err(error) => {
                if shared.quota_generation.load(Ordering::Acquire) != generation {
                    continue;
                }
                fail(&shared, &app, &error);
                backoff(&shared, &mut failures);
                continue;
            }
        };
        let mut next_read = Instant::now();
        loop {
            if !shared.quota_active.load(Ordering::Acquire)
                || shared.quota_generation.load(Ordering::Acquire) != server.generation
            {
                break;
            }
            if shared.quota_refresh.swap(false, Ordering::AcqRel) {
                next_read = Instant::now();
            }
            if Instant::now() >= next_read {
                // Identify the account for every sample, so a switch never resembles a reset.
                let account = match server.request("account/read", json!({"refreshToken":false})) {
                    Ok(value) => value.get("account").filter(|v| !v.is_null()).cloned(),
                    Err(error) => {
                        invalidate_account(&shared);
                        fail(&shared, &app, &error);
                        break;
                    }
                };
                let Some(account) = account else {
                    invalidate_account(&shared);
                    fail(&shared, &app, "authentication unavailable");
                    break;
                };
                let identity = account_identity(&account);
                {
                    let mut m = shared.inner.lock().unwrap();
                    select_account(&mut m, identity);
                    m.plan_label = domain::plan_label(&account);
                }
                let _ = app.emit("state-updated", ());
                match server.request("account/rateLimits/read", json!({})) {
                    Ok(value) => {
                        // Recheck identity after the quota request: never pair a new account's
                        // limits with the account read before a switch.
                        let checked = server.request("account/read", json!({"refreshToken":false}));
                        let matched = checked
                            .as_ref()
                            .ok()
                            .and_then(|v| v.get("account"))
                            .filter(|v| !v.is_null())
                            .is_some_and(|v| account_identity(v) == identity);
                        if !matched {
                            invalidate_account(&shared);
                            fail(&shared, &app, "authentication changed during quota read");
                            break;
                        }
                        let snapshot = domain::parse_quota(&value, now());
                        let account_key = celebration_account_key(Some(identity), &value);
                        diagnostics::record(format!(
                            "quota_snapshot generation={} snapshot_valid={}",
                            server.generation,
                            snapshot.five_hour.is_some() || snapshot.weekly.is_some()
                        ));
                        if !domain::quota_response_valid(&value) {
                            fail(&shared, &app, "quota response incomplete");
                            break;
                        }
                        if !shared.quota_active.load(Ordering::Acquire) {
                            break;
                        }
                        {
                            let mut m = shared.inner.lock().unwrap();
                            if shared.quota_generation.load(Ordering::Acquire) != server.generation
                                || !shared.quota_active.load(Ordering::Acquire)
                            {
                                break;
                            }
                            m.account_key = account_key;
                            let detector_before = m.reset_detector.clone();
                            if let Some(key) = m.reset_detector.observe(&snapshot, account_key) {
                                let before = m.settings.celebration_state.clone();
                                if m.settings.celebration_state.queue(key, snapshot.fetched_at)
                                    && crate::settings::save(&m.settings).is_err()
                                {
                                    m.settings.celebration_state = before;
                                    m.reset_detector = detector_before;
                                    diagnostics::record("celebration_event_save_failed");
                                }
                            }
                            m.quota_mode = domain::quota_mode(Some(&snapshot)).into();
                            m.snapshot = Some(snapshot);
                            m.quota_failed = false;
                            m.quota_status.clear();
                        }
                        let _ = app.emit("state-updated", ());
                        crate::deliver_reminder(&shared, &app);
                        failures = 0;
                        next_read = Instant::now() + Duration::from_secs(60);
                        if server.pending_update {
                            server.pending_update = false;
                            next_read = Instant::now() + Duration::from_secs(1);
                        }
                    }
                    Err(error) => {
                        if shared.quota_generation.load(Ordering::Acquire) != server.generation {
                            break;
                        }
                        fail(&shared, &app, &error);
                        break;
                    }
                }
            }
            let wait = next_read
                .saturating_duration_since(Instant::now())
                .min(Duration::from_secs(1));
            match server.update_notification(wait) {
                Ok(true) => next_read = next_read.min(Instant::now() + Duration::from_secs(1)),
                Ok(false) => {}
                Err(error) => {
                    fail(&shared, &app, &error);
                    break;
                }
            }
        }
        drop(server);
        diagnostics::record("app_server_stop");
        if shared.quota_generation.load(Ordering::Acquire) != generation {
            continue;
        }
        backoff(&shared, &mut failures);
    }
}

fn account_identity(account: &Value) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    account.to_string().hash(&mut hasher);
    hasher.finish()
}

fn select_account(m: &mut crate::Model, identity: u64) {
    if m.quota_account != Some(identity) {
        if m.quota_account.is_some() {
            m.settings.reminder_state = Default::default();
        }
        m.snapshot = None;
        m.quota_mode = "none".into();
        m.plan_label = None;
        m.account_key = None;
        m.reset_detector.clear();
        m.detail_credit_hint = None;
        m.credit_open = false;
        m.detail_content_height = None;
    }
    m.quota_account = Some(identity);
}

fn invalidate_account(shared: &Shared) {
    let mut m = shared.inner.lock().unwrap();
    m.snapshot = None;
    m.plan_label = None;
    m.account_key = None;
    m.detail_credit_hint = None;
    m.credit_open = false;
    m.reset_detector.clear();
}

fn set_status(shared: &Shared, app: &AppHandle, status: &str) {
    shared.inner.lock().unwrap().quota_status = status.into();
    let _ = app.emit("state-updated", ());
}

fn fail(shared: &Shared, app: &AppHandle, error: &str) {
    diagnostics::record(rpc_diagnostic("service", Err(error), 0));
    let status = match error_category(error).0 {
        "authentication" => "暂未取得 Codex 登录状态，正在重试",
        "timeout" => "额度连接超时，正在重试",
        "paused" => "等待 Codex 打开",
        "protocol" => "额度响应不完整，正在重试",
        _ => "额度服务暂不可用，正在重试",
    };
    {
        let mut m = shared.inner.lock().unwrap();
        m.quota_failed = true;
        m.reset_detector.clear();
        m.account_key = None;
        m.quota_status = status.into();
    }
    let _ = app.emit("state-updated", ());
}

fn error_category(error: &str) -> (&'static str, Option<i64>) {
    let json = serde_json::from_str::<Value>(error).ok();
    let code = json
        .as_ref()
        .and_then(|value| value.get("code"))
        .and_then(Value::as_i64);
    let message = error.to_ascii_lowercase();
    let category = if message.contains("auth") || message.contains("token") {
        "authentication"
    } else if message.contains("timed out") || message.contains("timeout") {
        "timeout"
    } else if message.contains("paused") {
        "paused"
    } else if message.contains("incomplete") || message.contains("missing result") || code.is_some()
    {
        "protocol"
    } else {
        "transport"
    };
    (category, code)
}

fn rpc_diagnostic(stage: &str, result: Result<(), &str>, duration_ms: u128) -> String {
    let (category, code) = result
        .err()
        .map(error_category)
        .unwrap_or(("success", None));
    format!(
        "quota_rpc stage={stage} category={category} code={} duration_ms={duration_ms}",
        code.map(|value| value.to_string())
            .unwrap_or_else(|| "none".into())
    )
}

fn backoff(shared: &Shared, failures: &mut usize) {
    let delay = [15, 30, 60, 300][(*failures).min(3)];
    *failures += 1;
    let deadline = Instant::now() + Duration::from_secs(delay);
    while Instant::now() < deadline && shared.quota_active.load(Ordering::Acquire) {
        if shared.quota_refresh.swap(false, Ordering::AcqRel) {
            break;
        }
        thread::sleep(Duration::from_millis(250));
    }
}

#[cfg(test)]
pub(crate) fn live_readonly_sample() -> Result<(Option<String>, domain::Snapshot), String> {
    let shared = Arc::new(Shared::default());
    shared.quota_active.store(true, Ordering::Release);
    let mut server = Server::start(shared)?;
    let account = server.request("account/read", json!({"refreshToken":false}))?;
    let response = server.request("account/rateLimits/read", json!({}))?;
    let confirmed = server.request("account/read", json!({"refreshToken":false}))?;
    if account.get("account") != confirmed.get("account")
        || !domain::quota_response_valid(&response)
    {
        return Err("account changed or quota response incomplete".into());
    }
    Ok((
        account.get("account").and_then(domain::plan_label),
        domain::parse_quota(&response, now()),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn account_switch_discards_quota_and_unavailable_identity_keeps_last_mode_only() {
        let shared = Shared::default();
        let mut m = shared.inner.lock().unwrap();
        select_account(&mut m, 1);
        m.snapshot = Some(domain::parse_quota(
            &json!({"rateLimits":{"primary":{"usedPercent":31,"windowDurationMins":10080}}}),
            now(),
        ));
        m.quota_mode = "weekly".into();
        m.plan_label = Some("PRO".into());
        select_account(&mut m, 1);
        assert!(m.snapshot.is_some());
        drop(m);
        invalidate_account(&shared);
        let mut m = shared.inner.lock().unwrap();
        assert!(m.snapshot.is_none());
        assert_eq!(m.quota_mode, "weekly");
        assert!(m.plan_label.is_none());
        select_account(&mut m, 2);
        assert_eq!(m.quota_mode, "none");
        assert!(m.snapshot.is_none());
    }

    #[test]
    #[ignore = "read-only live account acceptance; no settings or quotas modified"]
    fn live_account_readonly() {
        let shared = Arc::new(Shared::default());
        shared.quota_active.store(true, Ordering::Release);
        let mut server = Server::start(shared).expect("connect read-only app server");
        let account = server
            .request("account/read", json!({"refreshToken":false}))
            .unwrap();
        let response = server
            .request("account/rateLimits/read", json!({}))
            .unwrap();
        assert!(domain::quota_response_valid(&response));
        let snapshot = domain::parse_quota(&response, now());
        println!(
            "live_readonly plan={} mode={} five={} weekly={}",
            account
                .get("account")
                .and_then(domain::plan_label)
                .unwrap_or("unknown".into()),
            domain::quota_mode(Some(&snapshot)),
            snapshot.five_hour.is_some(),
            snapshot.weekly.is_some()
        );
    }
    #[test]
    fn jsonl_response_filters_ids_and_notifications_and_reports_closed_timeout_paused() {
        let active = std::sync::atomic::AtomicBool::new(true);
        let generation = std::sync::atomic::AtomicU64::new(7);
        let (tx, rx) = mpsc::channel();
        for line in [
            "garbage",
            r#"{"id":99,"result":{"wrong":true}}"#,
            r#"{"method":"account/rateLimits/updated","params":{"secret":"ignored"}}"#,
            r#"{"id":2,"result":{"ok":true}}"#,
        ] {
            tx.send(line.into()).unwrap();
        }
        let mut update = false;
        assert_eq!(
            receive_response(
                &rx,
                &active,
                &generation,
                7,
                2,
                Instant::now() + Duration::from_millis(20),
                &mut update
            )
            .unwrap(),
            json!({"ok":true})
        );
        assert!(update);
        assert_eq!(
            receive_response(&rx, &active, &generation, 7, 3, Instant::now(), &mut update)
                .unwrap_err(),
            "request timed out"
        );
        active.store(false, Ordering::Release);
        assert_eq!(
            receive_response(&rx, &active, &generation, 7, 3, Instant::now(), &mut update)
                .unwrap_err(),
            "service paused"
        );
        active.store(true, Ordering::Release);
        generation.store(8, Ordering::Release);
        assert_eq!(
            receive_response(&rx, &active, &generation, 7, 3, Instant::now(), &mut update)
                .unwrap_err(),
            "service paused"
        );
        drop(tx);
        assert_eq!(
            receive_response(
                &rx,
                &active,
                &generation,
                8,
                3,
                Instant::now() + Duration::from_millis(20),
                &mut update
            )
            .unwrap_err(),
            "app-server output closed"
        );
    }

    #[test]
    fn matching_rpc_error_and_missing_result_are_not_successful_snapshots() {
        let active = std::sync::atomic::AtomicBool::new(true);
        let generation = std::sync::atomic::AtomicU64::new(1);
        for stage in ["initialize", "account/read", "account/rateLimits/read"] {
            let (tx, rx) = mpsc::channel();
            tx.send(r#"{"id":1,"error":{"code":-32600,"message":"SECRET_TOKEN"}}"#.into())
                .unwrap();
            let error = receive_response(
                &rx,
                &active,
                &generation,
                1,
                1,
                Instant::now() + Duration::from_millis(20),
                &mut false,
            )
            .unwrap_err();
            let log = rpc_diagnostic(stage, Err(&error), 10);
            assert!(log.contains("code=-32600") && !log.contains("SECRET_TOKEN"));
            tx.send(r#"{"id":1}"#.into()).unwrap();
            assert_eq!(
                receive_response(
                    &rx,
                    &active,
                    &generation,
                    1,
                    1,
                    Instant::now() + Duration::from_millis(20),
                    &mut false
                )
                .unwrap_err(),
                "missing result"
            );
        }
    }
    #[test]
    fn diagnostic_keeps_rpc_code_without_account_or_token_values() {
        let log = rpc_diagnostic("account/read", Err("{\"code\":-32600,\"message\":\"authentication failed SECRET_TOKEN account@example.test\"}"), 120);
        assert!(log.contains("category=authentication"));
        assert!(log.contains("code=-32600"));
        assert!(!log.contains("SECRET_TOKEN"));
        assert!(!log.contains("account@example"));
    }
    #[test]
    fn timeout_is_distinct_from_authentication_error() {
        let log = rpc_diagnostic(
            "account/rateLimits/read",
            Err("timed out waiting on channel"),
            10000,
        );
        assert!(log.contains("category=timeout"));
        assert!(log.contains("duration_ms=10000"));
    }
    #[test]
    fn celebration_identity_requires_known_plan_and_codex_source() {
        let a = json!({"rateLimitsByLimitId":{"codex":{"planType":"plus"}}});
        let b = json!({"rateLimitsByLimitId":{"codex":{"planType":"pro"}}});
        assert_ne!(
            celebration_account_key(Some(1), &a),
            celebration_account_key(Some(1), &b)
        );
        assert_ne!(
            celebration_account_key(Some(1), &a),
            celebration_account_key(Some(2), &a)
        );
        assert!(celebration_account_key(Some(1), &json!({"rateLimits":{}})).is_none());
        assert!(celebration_account_key(
            Some(1),
            &json!({"rateLimitsByLimitId":{"other":{"planType":"plus"}}})
        )
        .is_none());
        assert!(celebration_account_key(None, &a).is_none());
        let legacy = json!({"rateLimits":{"planType":"plus"}});
        assert_ne!(
            celebration_account_key(Some(1), &a),
            celebration_account_key(Some(1), &legacy)
        );
    }
}
