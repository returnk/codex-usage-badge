use super::*;
use serde_json::json;
use std::collections::VecDeque;

fn fixture(time: i64, used: f64, cards: Value) -> Sample {
    Sample::new(
        json!({"type":"chatgpt","email":"one@example.test","planType":"plus"}),
        json!({"rateLimits":{"limitId":"codex","planType":"plus","primary":{"usedPercent":used,"windowDurationMins":300,"resetsAt":time+9000},"secondary":null},"rateLimitResetCredits":{"availableCount":5,"credits":cards}}),
        time,
    ).unwrap()
}
fn cards(time: i64) -> Value {
    json!([
        {"id":"card-a","resetType":"codexRateLimits","status":"available","expiresAt":time+1200},
        {"id":"card-b","resetType":"codexRateLimits","status":"available","expiresAt":time+1500}
    ])
}
fn temp_store() -> Store {
    Store::at(
        std::env::temp_dir()
            .join(format!("codexbadge-reset-test-{}", uuid().unwrap()))
            .join("reset.json"),
    )
}
struct MockRpc {
    responses: VecDeque<Result<Value, String>>,
    writes: Vec<Value>,
    clock: i64,
}
impl MockRpc {
    fn new(sample: &Sample, outcome: Result<Value, &str>) -> Self {
        let mut rpc = Self {
            responses: VecDeque::new(),
            writes: vec![],
            clock: 0,
        };
        rpc.read(sample);
        rpc.responses.push_back(outcome.map_err(str::to_string));
        rpc.read(sample);
        rpc
    }
    fn read(&mut self, sample: &Sample) {
        self.responses.extend([
            Ok(json!({"account":sample.account.clone()})),
            Ok(sample.limits.clone()),
            Ok(json!({"account":sample.account.clone()})),
        ]);
    }
}
impl Rpc for MockRpc {
    fn current_time(&self) -> i64 {
        self.clock
    }
    fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        if method == CONSUME {
            self.writes.push(params);
        }
        self.responses.pop_front().expect("unexpected RPC")
    }
}
fn grant(store: &Store, sample: &Sample) {
    store.set_enabled(&sample.scope, true).unwrap();
}

#[test]
fn default_off_and_unknown_account_never_consume() {
    let store = temp_store();
    let sample = fixture(10000, 50.0, cards(10000));
    let mut rpc = MockRpc::new(&sample, Ok(json!({"outcome":"reset"})));
    assert!(
        !process(&store, &mut rpc, &sample, &|| true, 10000)
            .unwrap()
            .enabled
    );
    assert!(rpc.writes.is_empty());
    grant(&store, &sample);
    let mut other = sample.clone();
    other.account["email"] = json!("two@example.test");
    let other = Sample::new(other.account, other.limits, 10000).unwrap();
    assert!(
        !process(&store, &mut rpc, &other, &|| true, 10000)
            .unwrap()
            .enabled
    );
    assert!(rpc.writes.is_empty());
}

#[test]
fn explicit_earliest_card_and_success_never_spends_second_before_original_expiry() {
    let store = temp_store();
    let sample = fixture(10000, 50.0, cards(10000));
    grant(&store, &sample);
    let mut rpc = MockRpc::new(&sample, Ok(json!({"outcome":"reset"})));
    let view = process(&store, &mut rpc, &sample, &|| true, 10000).unwrap();
    assert!(view.status.contains("同步"));
    assert_eq!(rpc.writes[0]["creditId"], "card-a");
    assert_eq!(rpc.writes[0]["idempotencyKey"].as_str().unwrap().len(), 36);
    let fresh_store = Store::at(store.path.clone());
    fresh_store.set_enabled(&sample.scope, false).unwrap();
    grant(&fresh_store, &sample);
    let mut next = fixture(10060, 50.0, cards(10000));
    next.limits["rateLimitResetCredits"]["credits"] = json!([cards(10000)[1]]);
    process(&fresh_store, &mut rpc, &next, &|| true, 10060).unwrap();
    assert_eq!(rpc.writes.len(), 1);
    assert!(
        store.load().unwrap().accounts[&account_slot(&sample.scope).unwrap()]
            .last
            .as_ref()
            .unwrap()
            .notified
    );
}

#[test]
fn no_credit_and_missing_target_hold_across_polls_and_restart() {
    for outcome in ["noCredit", "missing"] {
        let store = temp_store();
        let sample = fixture(10000, 50.0, cards(10000));
        grant(&store, &sample);
        let mut rpc = MockRpc::new(&sample, Ok(json!({"outcome":"noCredit"})));
        if outcome == "missing" {
            let mut missing = sample.clone();
            missing.limits["rateLimitResetCredits"]["credits"] = json!([cards(10000)[1]]);
            rpc = MockRpc {
                responses: VecDeque::new(),
                writes: vec![],
                clock: 0,
            };
            rpc.read(&missing);
        }
        process(&store, &mut rpc, &sample, &|| true, 10000).unwrap();
        let count = rpc.writes.len();
        process(
            &Store::at(store.path.clone()),
            &mut rpc,
            &sample,
            &|| true,
            11000,
        )
        .unwrap();
        assert_eq!(rpc.writes.len(), count);
        assert_eq!(
            store.load().unwrap().accounts[&account_slot(&sample.scope).unwrap()]
                .attempt
                .as_ref()
                .unwrap()
                .credit_id,
            "card-a"
        );
    }
}

#[test]
fn unknown_reuses_original_key_at_60_and_300_seconds_then_pauses() {
    let store = temp_store();
    let sample = fixture(10000, 50.0, cards(10000));
    grant(&store, &sample);
    let mut rpc = MockRpc::new(&sample, Err("request timed out"));
    process(&store, &mut rpc, &sample, &|| true, 10000).unwrap();
    process(&store, &mut rpc, &sample, &|| true, 10059).unwrap();
    assert_eq!(rpc.writes.len(), 1);
    for time in [10060, 10300] {
        let mut retry = MockRpc::new(&sample, Err("app-server output closed"));
        process(
            &Store::at(store.path.clone()),
            &mut retry,
            &sample,
            &|| true,
            time,
        )
        .unwrap();
        assert_eq!(retry.writes, rpc.writes);
    }
    let view = process(&store, &mut rpc, &sample, &|| true, 10400).unwrap();
    assert!(view.status.contains("核查"));
    assert_eq!(rpc.writes.len(), 1);
}

#[test]
fn lost_response_then_already_redeemed_is_success_even_with_stale_quota() {
    let store = temp_store();
    let sample = fixture(10000, 50.0, cards(10000));
    grant(&store, &sample);
    let mut first = MockRpc::new(&sample, Err("request timed out"));
    process(&store, &mut first, &sample, &|| true, 10000).unwrap();
    let mut second = MockRpc::new(&sample, Ok(json!({"outcome":"alreadyRedeemed"})));
    assert!(process(&store, &mut second, &sample, &|| true, 10060)
        .unwrap()
        .status
        .contains("同步"));
    assert_eq!(first.writes, second.writes);
}

#[test]
fn nothing_to_reset_requires_changed_window_and_keeps_same_card() {
    let store = temp_store();
    let sample = fixture(10000, 50.0, cards(10000));
    grant(&store, &sample);
    let mut rpc = MockRpc::new(&sample, Ok(json!({"outcome":"nothingToReset"})));
    process(&store, &mut rpc, &sample, &|| true, 10000).unwrap();
    process(&store, &mut rpc, &sample, &|| true, 10060).unwrap();
    assert_eq!(rpc.writes.len(), 1);
    let next = fixture(10000, 60.0, cards(10000));
    let mut retry = MockRpc::new(&next, Ok(json!({"outcome":"reset"})));
    process(&store, &mut retry, &next, &|| true, 10120).unwrap();
    assert_eq!(retry.writes[0]["creditId"], "card-a");
    assert_ne!(
        rpc.writes[0]["idempotencyKey"],
        retry.writes[0]["idempotencyKey"]
    );
}

#[test]
fn full_count_only_unsupported_expired_or_outside_lead_never_consume() {
    let time = 10000;
    let samples = [
        fixture(time, 0.0, cards(time)),
        fixture(time, 10.0, Value::Null),
        fixture(
            time,
            10.0,
            json!([{"id":"x","status":"available","resetType":"other","expiresAt":time+10}]),
        ),
        fixture(time, 10.0, cards(time - 2000)),
        fixture(time, 10.0, cards(time + 2000)),
    ];
    for sample in samples {
        let store = temp_store();
        grant(&store, &sample);
        let mut rpc = MockRpc::new(&sample, Ok(json!({"outcome":"reset"})));
        process(&store, &mut rpc, &sample, &|| true, time).unwrap();
        assert!(rpc.writes.is_empty());
    }
    let mut sample = fixture(time, 50.0, cards(time));
    sample.limits["rateLimits"]["primary"]["windowDurationMins"] = json!(15);
    let store = temp_store();
    grant(&store, &sample);
    let mut rpc = MockRpc::new(&sample, Ok(json!({"outcome":"reset"})));
    process(&store, &mut rpc, &sample, &|| true, time).unwrap();
    assert!(rpc.writes.is_empty());
}

#[test]
fn cancelled_before_send_and_identity_change_never_write() {
    let store = temp_store();
    let sample = fixture(10000, 50.0, cards(10000));
    grant(&store, &sample);
    let mut rpc = MockRpc::new(&sample, Ok(json!({"outcome":"reset"})));
    process(&store, &mut rpc, &sample, &|| false, 10000).unwrap();
    assert!(rpc.writes.is_empty());
    let mut other = sample.clone();
    other.account["email"] = json!("two@example.test");
    let mut rpc = MockRpc::new(&other, Ok(json!({"outcome":"reset"})));
    assert!(process(&store, &mut rpc, &sample, &|| true, 10000).is_err());
    assert!(rpc.writes.is_empty());
}

#[test]
fn corrupt_record_unwritable_record_and_concurrent_instance_fail_closed() {
    let store = temp_store();
    let sample = fixture(10000, 50.0, cards(10000));
    grant(&store, &sample);
    let mut rpc = MockRpc::new(&sample, Ok(json!({"outcome":"reset"})));
    let lock = store.lock().unwrap();
    assert!(process(
        &Store::at(store.path.clone()),
        &mut rpc,
        &sample,
        &|| true,
        10000
    )
    .is_err());
    drop(lock);
    std::fs::write(&store.path, "{}").unwrap();
    assert!(process(&store, &mut rpc, &sample, &|| true, 10000).is_err());
    assert!(rpc.writes.is_empty());
    let store = temp_store();
    grant(&store, &sample);
    std::fs::create_dir(store.path.with_extension("tmp")).unwrap();
    assert!(process(&store, &mut rpc, &sample, &|| true, 10000).is_err());
    assert!(rpc.writes.is_empty());
}

#[test]
fn confirmed_success_and_expired_pending_cannot_be_reset_by_toggle() {
    let store = temp_store();
    let sample = fixture(10000, 50.0, cards(10000));
    grant(&store, &sample);
    let mut rpc = MockRpc::new(&sample, Err("missing result"));
    process(&store, &mut rpc, &sample, &|| true, 10000).unwrap();
    store.set_enabled(&sample.scope, false).unwrap();
    grant(&store, &sample);
    let next = fixture(11300, 50.0, cards(11300));
    let view = process(&store, &mut rpc, &next, &|| true, 11300).unwrap();
    assert!(view.status.contains("核查"));
    assert_eq!(rpc.writes.len(), 1);
    store.set_enabled(&sample.scope, false).unwrap();
    grant(&store, &sample);
    process(&store, &mut rpc, &next, &|| true, 11400).unwrap();
    assert_eq!(rpc.writes.len(), 1);
}

#[test]
fn permission_revoked_during_second_read_prevents_send() {
    use std::cell::Cell;
    let store = temp_store();
    let sample = fixture(10000, 50.0, cards(10000));
    grant(&store, &sample);
    let checks = Cell::new(0);
    let allowed = || {
        let n = checks.get();
        checks.set(n + 1);
        n == 0
    };
    let mut rpc = MockRpc::new(&sample, Ok(json!({"outcome":"reset"})));
    process(&store, &mut rpc, &sample, &allowed, 10000).unwrap();
    assert!(rpc.writes.is_empty());
}

#[test]
fn success_allows_a_new_card_only_after_old_expiry_and_fresh_nonfull_state() {
    let store = temp_store();
    let sample = fixture(10000, 50.0, cards(10000));
    grant(&store, &sample);
    let mut first = MockRpc::new(&sample, Ok(json!({"outcome":"reset"})));
    process(&store, &mut first, &sample, &|| true, 10000).unwrap();
    let sample = fixture(11201, 50.0, cards(10000));
    let mut next = MockRpc::new(&sample, Ok(json!({"outcome":"reset"})));
    process(&store, &mut next, &sample, &|| true, 11201).unwrap();
    assert_eq!(next.writes[0]["creditId"], "card-b");
}

#[test]
fn reauthorization_after_plan_or_protocol_change_preserves_account_phase_guard() {
    let store = temp_store();
    let sample = fixture(10000, 50.0, cards(10000));
    grant(&store, &sample);
    let mut rpc = MockRpc::new(&sample, Ok(json!({"outcome":"reset"})));
    process(&store, &mut rpc, &sample, &|| true, 10000).unwrap();
    let mut changed = sample.clone();
    changed.account["planType"] = json!("pro");
    changed.limits["rateLimits"]["planType"] = json!("pro");
    changed.limits["rateLimitResetCredits"]["credits"] = json!([cards(10000)[1]]);
    let changed = Sample::new(changed.account, changed.limits, 10060).unwrap();
    grant(&store, &changed);
    let mut second = MockRpc::new(&changed, Ok(json!({"outcome":"reset"})));
    process(&store, &mut second, &changed, &|| true, 10060).unwrap();
    assert!(
        second.writes.is_empty(),
        "plan reauthorization must retain the account's original card barrier"
    );
    let mut by_id = changed.limits.clone();
    by_id["rateLimitsByLimitId"] = json!({"codex":by_id["rateLimits"].clone()});
    let by_id = Sample::new(changed.account.clone(), by_id, 10060).unwrap();
    grant(&store, &by_id);
    process(&store, &mut second, &by_id, &|| true, 10060).unwrap();
    assert!(second.writes.is_empty());
}

#[test]
fn second_read_crossing_expiry_never_sends() {
    let store = temp_store();
    let sample = fixture(10000, 50.0, cards(10000));
    grant(&store, &sample);
    let mut rpc = MockRpc::new(&sample, Ok(json!({"outcome":"reset"})));
    rpc.clock = 11200;
    process(&store, &mut rpc, &sample, &|| true, 10000).unwrap();
    assert!(rpc.writes.is_empty());
}

#[test]
fn stopping_unknown_request_does_not_replay_consume_and_preserves_unresolved_result() {
    let store = temp_store();
    let sample = fixture(10000, 50.0, cards(10000));
    grant(&store, &sample);
    let mut rpc = MockRpc::new(&sample, Err("timeout"));
    process(&store, &mut rpc, &sample, &|| true, 10000).unwrap();
    store.set_enabled(&sample.scope, false).unwrap();
    let view = process(&store, &mut rpc, &sample, &|| false, 10060).unwrap();
    assert!(view.last.contains("未决"));
    assert!(view.notification.as_ref().unwrap().contains("核查"));
    assert_eq!(rpc.writes.len(), 1);
    grant(&store, &sample);
    process(&store, &mut rpc, &sample, &|| true, 10300).unwrap();
    assert_eq!(rpc.writes.len(), 1);
}

#[test]
fn rejected_or_unanswered_first_consent_never_saves_permission() {
    let store = temp_store();
    let sample = fixture(10000, 50.0, cards(10000));
    let mut rpc = MockRpc::new(&sample, Ok(json!({"outcome":"reset"})));
    assert!(!authorize(&store, &mut rpc, &sample, false, 10000).unwrap());
    assert!(!store.background().unwrap());
    assert!(!store.path.exists());
    assert!(rpc.writes.is_empty());
}

#[test]
fn consent_rechecks_account_and_save_failure_does_not_enable() {
    let store = temp_store();
    let sample = fixture(10000, 50.0, cards(10000));
    let mut other = sample.clone();
    other.account["email"] = json!("other@example.test");
    let mut rpc = MockRpc::new(&other, Ok(json!({"outcome":"reset"})));
    assert!(authorize(&store, &mut rpc, &sample, true, 10000).is_err());
    assert!(!store.background().unwrap());
    fs::create_dir(store.path.with_extension("tmp")).unwrap();
    let mut rpc = MockRpc::new(&sample, Ok(json!({"outcome":"reset"})));
    assert!(authorize(&store, &mut rpc, &sample, true, 10000).is_err());
    assert!(!store.background().unwrap());
    assert!(rpc.writes.is_empty());
}

#[test]
fn crash_after_send_before_result_save_recovers_the_persisted_key() {
    struct FailSave<'a> {
        store: &'a Store,
        rpc: MockRpc,
    }
    impl Rpc for FailSave<'_> {
        fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
            if method == CONSUME {
                let db = self.store.load().unwrap();
                let t = db
                    .accounts
                    .values()
                    .next()
                    .unwrap()
                    .attempt
                    .as_ref()
                    .unwrap();
                assert_eq!(t.key, params["idempotencyKey"]);
                assert_eq!(t.credit_id, params["creditId"]);
                assert!(self.store.lock().is_err());
                fs::create_dir(self.store.path.with_extension("tmp")).unwrap();
            }
            self.rpc.request(method, params)
        }
    }
    let store = temp_store();
    let sample = fixture(10000, 50.0, cards(10000));
    grant(&store, &sample);
    let mut first = FailSave {
        store: &store,
        rpc: MockRpc::new(&sample, Ok(json!({"outcome":"reset"}))),
    };
    assert!(process(&store, &mut first, &sample, &|| true, 10000).is_err());
    fs::remove_dir(store.path.with_extension("tmp")).unwrap();
    let mut second = MockRpc::new(&sample, Ok(json!({"outcome":"alreadyRedeemed"})));
    process(&store, &mut second, &sample, &|| true, 10060).unwrap();
    assert_eq!(first.rpc.writes, second.writes);
}
