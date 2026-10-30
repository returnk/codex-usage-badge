//! Local opt-in redemptions. This record is separate from display settings: a
//! settings migration or quota-cache write must never erase a pending redemption.
use crate::domain;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::windows::fs::OpenOptionsExt;
use std::path::PathBuf;

pub const CONSUME: &str = "account/rateLimitResetCredit/consume";
const LEAD: i64 = 30 * 60;

pub trait Rpc {
    fn request(&mut self, method: &str, params: Value) -> Result<Value, String>;
    fn current_time(&self) -> i64 {
        0
    }
}

#[derive(Clone)]
pub struct Sample {
    account: Value,
    limits: Value,
    pub scope: String,
    pub label: String,
}

impl Sample {
    pub fn new(account: Value, limits: Value, _time: i64) -> Option<Self> {
        if account.get("type")?.as_str()? != "chatgpt" {
            return None;
        }
        let email = account.get("email")?.as_str()?;
        if email.is_empty() || email.len() > 320 || email.chars().any(char::is_control) {
            return None;
        }
        let plan = account.get("planType")?.as_str()?;
        domain::plan_label(&account)?;
        let bucket = domain::quota_limits(&limits)?;
        if bucket.get("planType")?.as_str()? != plan || !domain::quota_response_valid(&limits) {
            return None;
        }
        let scope = serde_json::to_string(&(
            "expiring-reset-v1",
            &account,
            plan,
            limits.get("rateLimitsByLimitId").is_some(),
            bucket.get("limitId"),
            bucket.get("normalModelSlug"),
        ))
        .ok()?;
        Some(Self {
            label: format!("{} ({})", email, plan.to_ascii_uppercase()),
            account,
            limits,
            scope,
        })
    }

    pub fn read(rpc: &mut impl Rpc, time: i64) -> Result<Self, String> {
        let before = rpc.request("account/read", json!({"refreshToken":false}))?;
        let limits = rpc.request("account/rateLimits/read", json!({}))?;
        let after = rpc.request("account/read", json!({"refreshToken":false}))?;
        if before.get("account") != after.get("account") {
            return Err("账号读取期间发生变化，自动用卡暂停".into());
        }
        Self::new(
            before.get("account").cloned().unwrap_or(Value::Null),
            limits,
            time,
        )
        .ok_or_else(|| "无法确认适用的 ChatGPT 账号及计划，自动用卡暂停".into())
    }

    fn windows(&self, time: i64) -> Option<String> {
        let bucket = domain::quota_limits(&self.limits)?;
        let mut used = false;
        let mut windows = Vec::new();
        for key in ["primary", "secondary"] {
            let Some(w) = bucket.get(key).filter(|v| !v.is_null()) else {
                continue;
            };
            if !matches!(w.get("windowDurationMins")?.as_u64()?, 300 | 10080)
                || w.get("resetsAt")?.as_i64()? <= time
            {
                return None;
            }
            used |= w.get("usedPercent")?.as_f64()? > 0.0;
            windows.push(w);
        }
        (used && !windows.is_empty()).then(|| serde_json::to_string(&windows).unwrap())
    }

    fn cards(&self, time: i64) -> Vec<domain::ResetCredit> {
        if self
            .limits
            .pointer("/rateLimitResetCredits/availableCount")
            .and_then(Value::as_u64)
            == Some(0)
        {
            return vec![];
        }
        let mut cards: Vec<_> = self
            .limits
            .pointer("/rateLimitResetCredits/credits")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|c| {
                if c.get("status")?.as_str()? != "available"
                    || c.get("resetType")?.as_str()? != "codexRateLimits"
                {
                    return None;
                }
                let id = c.get("id")?.as_str()?;
                let expires_at = c.get("expiresAt")?.as_i64()?;
                (!id.trim().is_empty() && id.len() <= 4096 && expires_at > time).then(|| {
                    domain::ResetCredit {
                        id: id.into(),
                        expires_at,
                    }
                })
            })
            .collect();
        cards.sort_by(|a, b| (a.expires_at, &a.id).cmp(&(b.expires_at, &b.id)));
        // Duplicate IDs with conflicting expiries are not a verifiable target.
        cards.retain(|c| {
            self.limits
                .pointer("/rateLimitResetCredits/credits")
                .and_then(Value::as_array)
                .is_some_and(|all| {
                    all.iter()
                        .filter(|v| v.get("id").and_then(Value::as_str) == Some(&c.id))
                        .count()
                        == 1
                })
        });
        cards
    }
}

#[derive(Serialize, Deserialize)]
struct Database {
    version: u32,
    accounts: BTreeMap<String, Account>,
}
impl Default for Database {
    fn default() -> Self {
        Self {
            version: 1,
            accounts: BTreeMap::new(),
        }
    }
}
#[derive(Default, Serialize, Deserialize)]
struct Account {
    permit_scope: String,
    enabled: bool,
    paused: bool,
    next_card_after: i64,
    attempt: Option<Attempt>,
    history: Vec<Attempt>,
    last: Option<ResultEvent>,
}
#[derive(Clone, Serialize, Deserialize)]
struct Attempt {
    credit_id: String,
    expires_at: i64,
    key: String,
    started: i64,
    recoveries: u8,
    state: AttemptState,
    windows: String,
}
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum AttemptState {
    Pending,
    Success,
    Nothing,
    NoCredit,
    Missing,
    Uncertain,
    Rejected,
}
#[derive(Serialize, Deserialize)]
struct ResultEvent {
    time: i64,
    message: String,
    notified: bool,
}
impl Database {
    fn valid(&self) -> bool {
        self.version == 1
            && self.accounts.iter().all(|(scope, a)| {
                !scope.is_empty()
                    && !a.permit_scope.is_empty()
                    && account_slot(&a.permit_scope).as_ref() == Ok(scope)
                    && a.attempt.iter().chain(a.history.iter()).all(|t| {
                        !t.credit_id.trim().is_empty()
                            && t.key.len() == 36
                            && t.recoveries <= 2
                            && t.started > 0
                            && t.expires_at > t.started
                            && !t.windows.is_empty()
                            && a.next_card_after >= t.expires_at
                    })
                    && a.attempt
                        .as_ref()
                        .is_none_or(|t| a.next_card_after == t.expires_at)
            })
    }
    fn background(&self) -> bool {
        self.accounts.values().any(|a| {
            a.enabled
                || a.attempt
                    .as_ref()
                    .is_some_and(|t| t.state == AttemptState::Pending)
        })
    }
}

pub struct Store {
    path: PathBuf,
}
impl Store {
    pub fn local() -> Result<Self, String> {
        Ok(Self::at(
            PathBuf::from(std::env::var_os("LOCALAPPDATA").ok_or("无法取得本地数据目录")?)
                .join("CodexBadge/expiring-reset.json"),
        ))
    }
    fn at(path: PathBuf) -> Self {
        Self { path }
    }
    fn lock(&self) -> Result<File, String> {
        fs::create_dir_all(self.path.parent().ok_or("执行记录目录无效")?)
            .map_err(|_| "无法创建执行记录目录")?;
        // Windows share mode 0 covers processes and login sessions of this user.
        // Holding the file handle also serializes permit changes with redemption.
        OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .share_mode(0)
            .open(self.path.with_extension("lock"))
            .map_err(|_| "执行记录被占用或不可写，自动用卡暂停".into())
    }
    fn load(&self) -> Result<Database, String> {
        let bytes = match fs::read(&self.path) {
            Ok(v) => v,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Database::default()),
            Err(_) => return Err("执行记录无法读取，自动用卡暂停".into()),
        };
        serde_json::from_slice::<Database>(&bytes)
            .ok()
            .filter(Database::valid)
            .ok_or_else(|| "执行记录损坏或版本不兼容，请核查；自动用卡暂停".into())
    }
    fn save(&self, db: &Database) -> Result<(), String> {
        if !db.valid() {
            return Err("执行记录校验失败，自动用卡暂停".into());
        }
        let save = || -> Result<(), Box<dyn std::error::Error>> {
            let mut file = File::create(self.path.with_extension("tmp"))?;
            file.write_all(&serde_json::to_vec_pretty(db)?)?;
            file.sync_all()?;
            drop(file);
            fs::rename(self.path.with_extension("tmp"), &self.path)?;
            Ok(())
        };
        save().map_err(|_| "执行记录保存失败，自动用卡暂停".into())
    }
    pub fn background(&self) -> Result<bool, String> {
        let _lock = self.lock()?;
        Ok(self.load()?.background())
    }
    pub fn set_enabled(&self, scope: &str, enabled: bool) -> Result<(), String> {
        let _lock = self.lock()?;
        let mut db = self.load()?;
        let a = db.accounts.entry(account_slot(scope)?).or_default();
        if enabled {
            a.permit_scope = scope.into();
        }
        a.enabled = enabled;
        // Re-authorizing never clears unresolved results or the card-phase guard.
        self.save(&db)
    }
}

#[derive(Clone)]
pub struct View {
    pub enabled: bool,
    pub background: bool,
    pub status: String,
    pub last: String,
    pub notification: Option<String>,
    pub snapshot: Option<domain::Snapshot>,
}
impl Default for View {
    fn default() -> Self {
        Self {
            enabled: false,
            background: false,
            status: "未开启".into(),
            last: String::new(),
            notification: None,
            snapshot: None,
        }
    }
}
impl View {
    pub fn hint_lines(&self) -> Vec<String> {
        let mut lines = vec![
            "到期前 30 分钟自动尝试，无需逐次确认。".into(),
            "电脑和 CodexBadge 须运行；不保证兑换成功。".into(),
            format!("状态：{}", self.status),
        ];
        if !self.last.is_empty() {
            lines.push(format!("最近：{}", self.last));
        }
        lines
    }
}

pub fn authorize(
    store: &Store,
    rpc: &mut impl Rpc,
    shown: &Sample,
    accepted: bool,
    time: i64,
) -> Result<bool, String> {
    if !accepted {
        return Ok(false);
    }
    let confirmed = Sample::read(rpc, time)?;
    if shown.scope != confirmed.scope {
        return Err("确认期间账号或计划变化，未开启；请重新授权".into());
    }
    store.set_enabled(&confirmed.scope, true)?;
    Ok(true)
}

pub fn show_error(message: &str) {
    use windows::core::{w, PCWSTR};
    use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONWARNING, MB_OK};
    let text: Vec<u16> = message.encode_utf16().chain(Some(0)).collect();
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(text.as_ptr()),
            w!("临期用卡"),
            MB_OK | MB_ICONWARNING,
        );
    }
}
fn event(a: &mut Account, message: &str, time: i64) {
    a.last = Some(ResultEvent {
        time,
        message: message.into(),
        notified: false,
    });
}
fn view(db: &Database, scope: &str, sample: &Sample, time: i64) -> View {
    let Some(a) = account_slot(scope)
        .ok()
        .and_then(|slot| db.accounts.get(&slot))
    else {
        return View {
            background: db.background(),
            status: "当前账号未开启".into(),
            ..View::default()
        };
    };
    let matched = a.permit_scope == scope;
    let status = if !matched {
        "当前账号或计划需重新授权"
    } else if a.paused {
        "自动用卡暂停，请核查最近结果"
    } else if let Some(t) = a
        .attempt
        .as_ref()
        .filter(|t| t.state == AttemptState::Pending)
    {
        let _ = t;
        "兑换结果待核实"
    } else if !a.enabled {
        "未开启"
    } else if a
        .attempt
        .as_ref()
        .is_some_and(|t| t.state == AttemptState::Success)
        && time < a.next_card_after
    {
        if sample.windows(time).is_none() {
            "已用卡"
        } else {
            "已用卡，额度同步中"
        }
    } else if time < a.next_card_after {
        if a.attempt
            .as_ref()
            .is_some_and(|t| t.state == AttemptState::Nothing)
        {
            "暂不可兑换：官方没有适用窗口"
        } else {
            "原卡到期前不换下一张"
        }
    } else if sample
        .limits
        .pointer("/rateLimitResetCredits/credits")
        .is_none_or(|v| !v.is_array())
        || (sample
            .limits
            .pointer("/rateLimitResetCredits/availableCount")
            .and_then(Value::as_u64)
            .is_some_and(|v| v > 0)
            && sample.cards(time).is_empty())
    {
        "缺少卡明细"
    } else if sample.windows(time).is_none() {
        "暂不可兑换：满额或无适用窗口"
    } else {
        "等待临期"
    };
    View {
        enabled: a.enabled && matched,
        background: db.background(),
        status: status.into(),
        last: a
            .last
            .as_ref()
            .map(|e| {
                format!(
                    "{} {}",
                    chrono::DateTime::from_timestamp(e.time, 0)
                        .map(|t| t
                            .with_timezone(&chrono::Local)
                            .format("%m/%d %H:%M")
                            .to_string())
                        .unwrap_or_default(),
                    e.message
                )
            })
            .unwrap_or_default(),
        notification: None,
        snapshot: Some(domain::parse_quota(&sample.limits, time)),
    }
}

// Permits include plan and protocol compatibility; the safety ledger instead
// uses the signed-in email across those changes. Grouping workspaces sharing an
// email is deliberately conservative: it can delay redemption, never authorize it.
fn account_slot(scope: &str) -> Result<String, String> {
    let value: Value = serde_json::from_str(scope).map_err(|_| "账号许可格式无效")?;
    let email = value
        .get(1)
        .and_then(|a| a.get("email"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or("账号许可缺少身份")?;
    Ok(email.to_ascii_lowercase())
}

pub fn process(
    store: &Store,
    rpc: &mut impl Rpc,
    sample: &Sample,
    allowed: &impl Fn() -> bool,
    time: i64,
) -> Result<View, String> {
    let _lock = store.lock()?;
    let mut db = store.load()?;
    let slot = account_slot(&sample.scope)?;
    if let Some(a) = db.accounts.get_mut(&slot) {
        if let Some(t) = a
            .attempt
            .as_ref()
            .filter(|t| t.state == AttemptState::Pending)
        {
            if !a.enabled || !allowed() {
                a.attempt.as_mut().unwrap().state = AttemptState::Uncertain;
                a.paused = true;
                event(a, "已停止新兑换；此前请求结果未决，请到官方页面核查", time);
                store.save(&db)?;
            } else if t.expires_at <= time {
                a.attempt.as_mut().unwrap().state = AttemptState::Uncertain;
                a.paused = true;
                event(a, "兑换结果未能确认，卡已到期；请到官方页面核查", time);
                store.save(&db)?;
            }
        }
    }
    let target = db.accounts.get(&slot).and_then(|a| {
        if a.paused || a.permit_scope != sample.scope || !a.enabled || !allowed() {
            return None;
        }
        if let Some(t) = &a.attempt {
            if t.state == AttemptState::Pending {
                let due = t.started
                    + match t.recoveries {
                        0 => 60,
                        _ => 300,
                    };
                return (t.recoveries < 2 && time >= due && time < t.expires_at)
                    .then(|| (t.clone(), true));
            }
            if time < a.next_card_after {
                return (t.state == AttemptState::Nothing
                    && sample.windows(time).is_some_and(|w| w != t.windows))
                .then(|| (t.clone(), false));
            }
        }
        sample.windows(time)?;
        sample
            .cards(time)
            .into_iter()
            .find(|c| {
                c.expires_at <= time + LEAD
                    && !a
                        .attempt
                        .iter()
                        .chain(a.history.iter())
                        .any(|t| t.credit_id == c.id)
            })
            .map(|c| {
                (
                    Attempt {
                        credit_id: c.id,
                        expires_at: c.expires_at,
                        key: String::new(),
                        started: time,
                        recoveries: 0,
                        state: AttemptState::Pending,
                        windows: String::new(),
                    },
                    false,
                )
            })
    });
    let mut shown_sample = sample.clone();
    if let Some((mut attempt, recovery)) = target {
        // The store lock stays held across the second read and request. A second
        // process must reload both the permit and phase guard after acquiring it.
        let fresh = Sample::read(rpc, time)?;
        let time = time.max(rpc.current_time());
        if fresh.scope != sample.scope {
            return Err("账号或计划变化，自动用卡暂停".into());
        }
        shown_sample = fresh.clone();
        let a = db.accounts.get_mut(&slot).unwrap();
        if !fresh
            .cards(time)
            .iter()
            .any(|c| c.id == attempt.credit_id && c.expires_at == attempt.expires_at)
        {
            if attempt.key.is_empty() {
                attempt.key = uuid()?;
                attempt.windows = sample.windows(time).ok_or("额度窗口已变化")?;
            }
            attempt.state = if recovery {
                AttemptState::Uncertain
            } else {
                AttemptState::Missing
            };
            a.paused |= recovery;
            a.next_card_after = attempt.expires_at;
            a.attempt = Some(attempt);
            event(
                a,
                "目标卡已不在可核实明细中；原卡到期前不换卡，请核查官方状态",
                time,
            );
            store.save(&db)?;
        } else if allowed() && a.enabled && (recovery || fresh.windows(time).is_some()) {
            if !recovery {
                if a.attempt.as_ref().is_some_and(|t| {
                    t.state == AttemptState::Nothing
                        && fresh.windows(time).as_ref() == Some(&t.windows)
                }) {
                    return Ok(view(&db, &sample.scope, &fresh, time));
                }
                attempt.key = uuid()?;
                attempt.started = time;
                attempt.recoveries = 0;
                attempt.windows = fresh.windows(time).unwrap();
            } else {
                attempt.recoveries += 1;
            }
            attempt.state = AttemptState::Pending;
            if let Some(old) = a.attempt.take() {
                if old.credit_id != attempt.credit_id {
                    a.history.push(old);
                }
            }
            a.next_card_after = attempt.expires_at;
            a.attempt = Some(attempt.clone());
            event(a, "请求已准备，兑换结果待核实", time);
            a.last.as_mut().unwrap().notified = true;
            // Write and flush the exact ID/key and phase barrier BEFORE any send.
            store.save(&db)?;
            if !allowed() || rpc.current_time() >= attempt.expires_at {
                return Ok(view(&db, &sample.scope, &fresh, time));
            }
            let response = rpc.request(
                CONSUME,
                json!({"creditId":attempt.credit_id,"idempotencyKey":attempt.key}),
            );
            let a = db.accounts.get_mut(&slot).unwrap();
            let outcome = response
                .as_ref()
                .ok()
                .and_then(|v| v.get("outcome"))
                .and_then(Value::as_str);
            let rejected = response
                .as_ref()
                .err()
                .and_then(|e| serde_json::from_str::<Value>(e).ok())
                .and_then(|v| v.get("code").and_then(Value::as_i64))
                .is_some_and(|c| matches!(c, -32601 | -32602));
            let (state, message) = match outcome {
                Some("reset" | "alreadyRedeemed") => {
                    (AttemptState::Success, Some("已用卡，额度正在同步"))
                }
                Some("nothingToReset") => (AttemptState::Nothing, None),
                Some("noCredit") => (
                    AttemptState::NoCredit,
                    Some("官方未找到可用目标卡；原卡到期前不换卡，请核查"),
                ),
                _ if rejected => (
                    AttemptState::Rejected,
                    Some("当前 CLI 不支持自动用卡接口或参数，请更新后核查"),
                ),
                _ if attempt.recoveries >= 2 => (
                    AttemptState::Uncertain,
                    Some("兑换结果仍无法确认，已暂停自动用卡；请到官方页面核查"),
                ),
                _ => (AttemptState::Pending, None),
            };
            a.attempt.as_mut().unwrap().state = state;
            a.paused |= matches!(state, AttemptState::Uncertain | AttemptState::Rejected);
            if let Some(message) = message {
                event(a, message, time);
            }
            if state == AttemptState::Nothing {
                event(a, "官方暂无适用窗口，卡已保留，等待额度状态变化", time);
                a.last.as_mut().unwrap().notified = true;
            }
            // A success is durable before refreshing limits, even when stale.
            store.save(&db)?;
            if let Ok(after) = Sample::read(rpc, time) {
                if after.scope == sample.scope {
                    shown_sample = after;
                }
            }
            crate::diagnostics::record(format!(
                "auto_reset_result state={} correlation={}",
                match state {
                    AttemptState::Success => "success",
                    AttemptState::Pending => "unknown",
                    AttemptState::Nothing => "nothing",
                    AttemptState::NoCredit => "missing",
                    _ => "paused",
                },
                crate::celebration::stable_fingerprint(attempt.key.as_bytes())
            ));
        }
    }
    let mut result = view(&db, &sample.scope, &shown_sample, time);
    if let Some(a) = db.accounts.get_mut(&slot) {
        if let Some(event) = a.last.as_mut().filter(|e| !e.notified) {
            result.notification = Some(event.message.clone());
            event.notified = true;
            // At most once submission, including a crash or OS rejection. The
            // durable in-app result remains available regardless of Windows.
            store.save(&db)?;
        }
    }
    Ok(result)
}

fn uuid() -> Result<String, String> {
    windows::core::GUID::new()
        .map(|g| format!("{g:?}").to_ascii_lowercase())
        .map_err(|_| "无法创建请求幂等键".into())
}

#[cfg(test)]
#[path = "expiring_reset_tests.rs"]
mod tests;
