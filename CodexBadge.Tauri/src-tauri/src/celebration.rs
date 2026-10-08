use crate::domain::Snapshot;
use serde::{Deserialize, Serialize};

const HISTORY_TTL: i64 = 7 * 86400;
const EVENT_TTL: i64 = 86400;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CelebrationState {
    pub welcome_done: bool,
    histories: Vec<AccountHistory>,
    pending: Vec<ResetEvent>,
    handled: Vec<ResetEvent>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResetKey {
    account: u64,
    resets_at: i64,
    consumed_credit: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ResetEvent {
    key: ResetKey,
    detected_at: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct AccountHistory {
    account: u64,
    detector: ResetDetector,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Playback {
    pub id: String,
    pub kind: &'static str,
}
// Versioned FNV-1a identity, independent of Rust's unspecified DefaultHasher.
// This is local correlation only, never an authentication/security decision.
pub fn stable_fingerprint(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    })
}
impl ResetKey {
    fn event_id(&self) -> String {
        format!(
            "reset-{:016x}",
            stable_fingerprint(&serde_json::to_vec(self).unwrap())
        )
    }
}
fn recent(now: i64, then: i64, ttl: i64) -> bool {
    (0..=ttl).contains(&now.saturating_sub(then))
}
impl CelebrationState {
    pub fn existing_install() -> Self {
        Self {
            welcome_done: true,
            ..Self::default()
        }
    }
    fn prune(&mut self, now: i64) {
        self.histories.retain(|h| {
            h.detector
                .previous
                .as_ref()
                .is_some_and(|(s, _)| recent(now, s.fetched_at, HISTORY_TTL))
        });
        for history in &mut self.histories {
            if history.detector.candidate.as_ref().is_some_and(|c| {
                !valid_snapshot(&c.snapshot)
                    || !valid_snapshot(&c.before)
                    || c.key.account != history.account
            }) {
                history.detector.candidate = None;
            }
        }
        let before = self.pending.len();
        self.pending
            .retain(|e| recent(now, e.detected_at, EVENT_TTL));
        self.handled
            .retain(|e| recent(now, e.detected_at, HISTORY_TTL));
        if self.pending.len() < before {
            crate::diagnostics::record("celebration_expired");
        }
    }
    pub fn interrupt(&mut self) {
        for history in &mut self.histories {
            if let Some(candidate) = history.detector.candidate.take() {
                history.detector.previous = Some((candidate.before, history.account));
            }
        }
    }
    pub fn observe(&mut self, snapshot: &Snapshot, account: Option<u64>) {
        self.prune(snapshot.fetched_at);
        let Some(account) = account else {
            self.interrupt();
            return;
        };
        if !valid_snapshot(snapshot) {
            self.interrupt();
            crate::diagnostics::record("celebration_rejected reason=invalid_snapshot");
            return;
        }
        let index = self
            .histories
            .iter()
            .position(|h| h.account == account)
            .unwrap_or_else(|| {
                if self.histories.len() >= 16 {
                    self.histories.remove(0);
                }
                self.histories.push(AccountHistory {
                    account,
                    detector: ResetDetector::default(),
                });
                self.histories.len() - 1
            });
        if let Some(key) = self.histories[index]
            .detector
            .observe(snapshot, Some(account))
        {
            self.queue(key, snapshot.fetched_at);
        }
    }
    pub fn queue(&mut self, key: ResetKey, now: i64) -> bool {
        self.prune(now);
        if self
            .pending
            .iter()
            .chain(&self.handled)
            .any(|e| e.key == key)
        {
            return false;
        }
        self.pending.push(ResetEvent {
            key,
            detected_at: now,
        });
        true
    }
    pub fn peek(&self, account: Option<u64>, now: i64) -> Option<Playback> {
        if !self.welcome_done {
            return Some(Playback {
                id: "welcome".into(),
                kind: "welcome",
            });
        }
        self.pending
            .iter()
            .find(|e| Some(e.key.account) == account && recent(now, e.detected_at, EVENT_TTL))
            .map(|e| Playback {
                id: e.key.event_id(),
                kind: "reset",
            })
    }
    // Consume only after rendering starts AND the acknowledgement is durable.
    pub fn acknowledge(
        &mut self,
        id: &str,
        account: Option<u64>,
        now: i64,
        save: impl FnOnce(&Self) -> Result<(), String>,
    ) -> Result<bool, String> {
        let Some(event) = self.peek(account, now).filter(|e| e.id == id) else {
            return Ok(false);
        };
        let mut next = self.clone();
        next.prune(now);
        if event.kind == "welcome" {
            next.welcome_done = true;
        } else if let Some(index) = next
            .pending
            .iter()
            .position(|e| e.key.event_id() == id && Some(e.key.account) == account)
        {
            let played = next.pending.remove(index);
            next.handled.push(played);
        }
        save(&next)?;
        *self = next;
        Ok(true)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ResetCandidate {
    before: Snapshot,
    snapshot: Snapshot,
    key: ResetKey,
    original_reset: i64,
    five_hour_before: Option<f64>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ResetDetector {
    previous: Option<(Snapshot, u64)>,
    candidate: Option<ResetCandidate>,
}
fn valid_snapshot(snapshot: &Snapshot) -> bool {
    let windows = [(&snapshot.five_hour, 300), (&snapshot.weekly, 10080)];
    windows.iter().all(|(window, minutes)| {
        (window.is_none() && *minutes == 300)
            || window.as_ref().is_some_and(|w| {
                w.minutes == *minutes
                    && w.remaining.is_finite()
                    && (0.0..=100.0).contains(&w.remaining)
                    && w.resets_at.is_some_and(|at| at > snapshot.fetched_at)
            })
    }) && snapshot.available_credits == Some(snapshot.credits.len() as u64)
        && snapshot.credits.iter().enumerate().all(|(index, card)| {
            !card.id.is_empty()
                && card.expires_at > snapshot.fetched_at
                && !snapshot.credits[..index]
                    .iter()
                    .any(|other| other.id == card.id)
        })
}
fn same_inventory(a: &Snapshot, b: &Snapshot) -> bool {
    a.available_credits == b.available_credits
        && a.credits.len() == b.credits.len()
        && a.credits.iter().all(|card| b.credits.contains(card))
}
impl ResetDetector {
    pub fn clear(&mut self) {
        *self = Self::default();
    }
    pub fn observe(&mut self, snapshot: &Snapshot, account: Option<u64>) -> Option<ResetKey> {
        let Some(account) = account else {
            self.clear();
            return None;
        };
        if !valid_snapshot(snapshot) {
            self.candidate = None;
            return None;
        }
        let previous = self.previous.replace((snapshot.clone(), account));
        let Some((before, old_account)) = previous else {
            return None;
        };
        if account != old_account
            || before.five_hour.is_some() != snapshot.five_hour.is_some()
            || !(1..=HISTORY_TTL).contains(&snapshot.fetched_at.saturating_sub(before.fetched_at))
            || !valid_snapshot(&before)
            || !valid_snapshot(snapshot)
        {
            self.candidate = None;
            return None;
        }
        let old = before.weekly.as_ref().unwrap();
        let current = snapshot.weekly.as_ref().unwrap();
        if let Some(candidate) = self.candidate.take() {
            if !recent(snapshot.fetched_at, before.fetched_at, 180) {
                crate::diagnostics::record("celebration_rejected reason=confirmation_gap");
                self.previous = Some((candidate.before, account));
                return self.observe(snapshot, Some(account));
            }
            let target = candidate.snapshot.weekly.as_ref().unwrap();
            let card_reset = candidate.key.consumed_credit.is_some();
            let five = snapshot.five_hour.as_ref().map(|w| w.remaining);
            let confirmed = candidate.original_reset
                > snapshot
                    .fetched_at
                    .saturating_add(if card_reset { 120 } else { 300 })
                && current.resets_at == target.resets_at
                && current.remaining >= target.remaining - 5.0
                && current.remaining >= 94.0
                && same_inventory(&candidate.snapshot, snapshot)
                && (!card_reset
                    || candidate.five_hour_before.is_none_or(|v| v >= 99.0)
                    || five.is_some_and(|v| v >= 94.0));
            if confirmed {
                let wait = 60;
                if snapshot
                    .fetched_at
                    .saturating_sub(candidate.snapshot.fetched_at)
                    < wait
                {
                    self.candidate = Some(candidate);
                    return None;
                }
                crate::diagnostics::record(if card_reset {
                    "celebration_reset_confirmed reason=consumed_live_credit"
                } else {
                    "celebration_reset_confirmed reason=early_weekly_recovery"
                });
                return Some(candidate.key);
            }
            crate::diagnostics::record("celebration_rejected reason=confirmation_changed");
        }
        let old_reset = old.resets_at.unwrap();
        let new_reset = current.resets_at.unwrap();
        if old_reset <= snapshot.fetched_at.saturating_add(120)
            || new_reset < old_reset
            || current.remaining < 95.0
        {
            return None;
        }
        let consumed = before.credits.iter().find(|card| {
            card.expires_at > snapshot.fetched_at.saturating_add(120)
                && !snapshot.credits.iter().any(|next| next.id == card.id)
        });
        let consumed_credit = consumed.filter(|card| {
            before.available_credits == snapshot.available_credits.and_then(|n| n.checked_add(1))
                && before
                    .credits
                    .iter()
                    .filter(|c| c.id != card.id)
                    .all(|c| snapshot.credits.contains(c))
        });
        let gain = current.remaining - old.remaining;
        let five_hour_before = before.five_hour.as_ref().map(|w| w.remaining);
        let eligible = if consumed_credit.is_some() {
            current.remaining >= 99.0
                && gain >= 5.0
                && new_reset >= old_reset.saturating_add(120)
                && (five_hour_before.is_none_or(|v| v >= 99.0)
                    || snapshot
                        .five_hour
                        .as_ref()
                        .is_some_and(|w| w.remaining >= 99.0))
        } else {
            gain >= 10.0
                && old_reset > snapshot.fetched_at.saturating_add(300)
                && same_inventory(&before, snapshot)
        };
        if eligible {
            crate::diagnostics::record("celebration_candidate");
            self.candidate = Some(ResetCandidate {
                before: before.clone(),
                snapshot: snapshot.clone(),
                original_reset: old_reset,
                five_hour_before,
                key: ResetKey {
                    account,
                    resets_at: new_reset,
                    consumed_credit: consumed_credit.map(|card| card.id.clone()),
                },
            });
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn played(
        state: &mut CelebrationState,
        account: Option<u64>,
        now: i64,
    ) -> Option<&'static str> {
        let event = state.peek(account, now)?;
        state
            .acknowledge(&event.id, account, now, |_| Ok(()))
            .unwrap()
            .then_some(event.kind)
    }
    #[test]
    fn weekly_only_reset_confirms_but_window_disappearance_does_not() {
        for consumed in [false, true] {
            let mut detector = ResetDetector::default();
            let mut before = sample(100, 20.0, 9000, true);
            before.five_hour = None;
            let mut after = sample(160, 100.0, 20000, !consumed);
            after.five_hour = None;
            let mut confirm = after.clone();
            confirm.fetched_at = 220;
            confirm.weekly.as_mut().unwrap().remaining = 98.0;
            detector.observe(&before, Some(1));
            assert!(detector.observe(&after, Some(1)).is_none());
            assert!(detector.observe(&confirm, Some(1)).is_some());
            let mut detector = ResetDetector::default();
            before.five_hour = sample(100, 20.0, 9000, true).five_hour;
            detector.observe(&before, Some(1));
            detector.observe(&after, Some(1));
            assert!(detector.observe(&confirm, Some(1)).is_none());
        }
    }
    use crate::domain::{QuotaWindow, ResetCredit};
    fn sample(at: i64, remaining: f64, reset: i64, card: bool) -> Snapshot {
        Snapshot {
            five_hour: Some(QuotaWindow {
                remaining: 100.0,
                minutes: 300,
                resets_at: Some(1000),
            }),
            weekly: Some(QuotaWindow {
                remaining,
                minutes: 10080,
                resets_at: Some(reset),
            }),
            credits: if card {
                vec![ResetCredit {
                    id: "card".into(),
                    expires_at: 10000,
                }]
            } else {
                vec![]
            },
            available_credits: Some(u64::from(card)),
            fetched_at: at,
        }
    }
    fn restored(state: &CelebrationState) -> CelebrationState {
        serde_json::from_slice(&serde_json::to_vec(state).unwrap()).unwrap()
    }
    #[test]
    fn overnight_recovery_is_confirmed_per_actual_window_and_survives_restart() {
        for weekly_only in [false, true] {
            let mut state = CelebrationState::existing_install();
            let mut before = sample(100, 80.0, 500000, true);
            before.credits[0].expires_at = 600000;
            if weekly_only {
                before.five_hour = None;
            }
            state.observe(&before, Some(1));
            state = restored(&state);
            let mut after = before.clone();
            after.fetched_at = 7200;
            if let Some(five) = after.five_hour.as_mut() {
                five.resets_at = Some(10000);
            }
            after.weekly.as_mut().unwrap().remaining = 96.0;
            after.weekly.as_mut().unwrap().resets_at = Some(600000);
            state.observe(&after, Some(1));
            assert!(state.peek(Some(1), 7200).is_none());
            state = restored(&state);
            after.fetched_at += 60;
            after.weekly.as_mut().unwrap().remaining = 95.0;
            state.observe(&after, Some(1));
            let event = state.peek(Some(1), 7260).expect("confirmed across restart");
            assert_eq!(event.kind, "reset");
            assert_eq!(state.peek(Some(2), 7260), None);
            assert_eq!(state.peek(Some(1), 7260), Some(event.clone()));
            state
                .acknowledge(&event.id, Some(1), 7260, |_| Ok(()))
                .unwrap();
            state = restored(&state);
            assert!(state.peek(Some(1), 7261).is_none());
            let key = state.handled[0].key.clone();
            assert!(!state.queue(key, 7262));
        }
    }
    #[test]
    fn natural_reset_expired_history_clock_rollback_and_first_read_do_not_celebrate() {
        for at in [9001, 100 + HISTORY_TTL + 1, 99] {
            let mut state = CelebrationState::existing_install();
            state.observe(&sample(100, 20.0, 9000, true), Some(1));
            state = restored(&state);
            let mut next = sample(at, 100.0, at + 100000, false);
            next.five_hour = None;
            state.observe(&next, Some(1));
            next.fetched_at += 60;
            state.observe(&next, Some(1));
            assert!(state.peek(Some(1), next.fetched_at).is_none());
        }
        let mut state = CelebrationState::existing_install();
        state.observe(&sample(100, 100.0, 9000, true), Some(1));
        state.observe(&sample(160, 100.0, 9000, true), Some(1));
        assert!(state.peek(Some(1), 160).is_none());
    }
    #[test]
    fn failed_acknowledgement_keeps_event_and_other_accounts_pending() {
        let mut state = CelebrationState::existing_install();
        for account in [1, 2] {
            state.queue(
                ResetKey {
                    account,
                    resets_at: 20000,
                    consumed_credit: None,
                },
                220,
            );
        }
        let first = state.peek(Some(1), 221).unwrap();
        assert!(state
            .acknowledge(&first.id, Some(1), 221, |_| Err("disk full".into()))
            .is_err());
        assert_eq!(state.peek(Some(1), 221), Some(first.clone()));
        assert!(!state
            .acknowledge(&first.id, Some(2), 221, |_| Ok(()))
            .unwrap());
        state
            .acknowledge(&first.id, Some(1), 222, |_| Ok(()))
            .unwrap();
        assert_eq!(state.peek(Some(2), 223).unwrap().kind, "reset");
        assert!(state.peek(Some(2), 220 + EVENT_TTL + 1).is_none());
    }
    #[test]
    fn read_failure_restarts_confirmation_without_losing_recovery_evidence() {
        let mut state = CelebrationState::existing_install();
        state.observe(&sample(100, 20.0, 9000, true), Some(1));
        state.observe(&sample(160, 100.0, 20000, true), Some(1));
        state.interrupt();
        state = restored(&state);
        state.observe(&sample(220, 100.0, 20000, true), Some(1));
        assert!(state.peek(Some(1), 220).is_none());
        state.observe(&sample(280, 98.0, 20000, true), Some(1));
        assert!(state.peek(Some(1), 280).is_some());
    }
    #[test]
    fn stable_fingerprint_has_a_fixed_cross_version_contract() {
        assert_eq!(stable_fingerprint(b"hello"), 0xa430d84680aabd0b);
    }
    #[test]
    fn another_account_cannot_consume_a_pending_reset() {
        let mut state = CelebrationState::existing_install();
        state.queue(
            ResetKey {
                account: 1,
                resets_at: 20000,
                consumed_credit: None,
            },
            220,
        );
        assert_eq!(played(&mut state, Some(2), 221), None);
        assert_eq!(played(&mut state, Some(1), 222), Some("reset"));
    }
    #[test]
    fn welcome_does_not_discard_a_confirmed_reset() {
        let mut state = CelebrationState::default();
        state.queue(
            ResetKey {
                account: 1,
                resets_at: 20000,
                consumed_credit: None,
            },
            220,
        );
        assert_eq!(played(&mut state, Some(1), 221), Some("welcome"));
        assert_eq!(played(&mut state, Some(1), 222), Some("reset"));
    }
    #[test]
    fn welcome_once_and_existing_install_never_welcomes() {
        let mut state = CelebrationState::default();
        assert_eq!(played(&mut state, None, 0), Some("welcome"));
        let mut state: CelebrationState =
            serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();
        assert!(played(&mut state, None, 0).is_none());
        assert!(played(&mut CelebrationState::existing_install(), None, 0).is_none());
    }
    #[test]
    fn early_weekly_reset_requires_confirmation_and_repeats_do_not_reset() {
        let mut detector = ResetDetector::default();
        assert!(detector
            .observe(&sample(100, 20.0, 9000, true), Some(1))
            .is_none());
        assert!(detector
            .observe(&sample(160, 100.0, 20000, true), Some(1))
            .is_none());
        let key = detector
            .observe(&sample(220, 98.0, 20000, true), Some(1))
            .expect("confirmed early reset");
        let mut state = CelebrationState::existing_install();
        assert!(state.queue(key.clone(), 220));
        assert_eq!(played(&mut state, Some(1), 220), Some("reset"));
        assert!(!state.queue(key, 220));
        assert!(detector
            .observe(&sample(280, 97.0, 20000, true), Some(1))
            .is_none());
    }
    #[test]
    fn consumed_live_card_can_confirm_a_small_weekly_recovery() {
        let mut detector = ResetDetector::default();
        detector.observe(&sample(100, 92.0, 9000, true), Some(1));
        assert!(detector
            .observe(&sample(160, 100.0, 20000, false), Some(1))
            .is_none());
        assert!(detector
            .observe(&sample(220, 98.0, 20000, false), Some(1))
            .is_some());
    }
    #[test]
    fn natural_reset_missing_account_gap_and_transient_jump_do_not_celebrate() {
        for (at, reset, account) in [
            (9001, 20000, Some(1)),
            (100 + HISTORY_TTL + 1, 100 + HISTORY_TTL + 20000, Some(1)),
            (160, 20000, Some(2)),
            (160, 20000, None),
        ] {
            let mut detector = ResetDetector::default();
            detector.observe(&sample(100, 20.0, 9000, true), Some(1));
            assert!(detector
                .observe(&sample(at, 100.0, reset, true), account)
                .is_none());
            assert!(detector
                .observe(&sample(at + 60, 98.0, reset, true), account)
                .is_none());
        }
        let mut detector = ResetDetector::default();
        detector.observe(&sample(100, 20.0, 9000, true), Some(1));
        detector.observe(&sample(160, 100.0, 20000, true), Some(1));
        assert!(detector
            .observe(&sample(220, 20.0, 9000, true), Some(1))
            .is_none());
    }
    #[test]
    fn normal_five_hour_recovery_and_expiring_card_do_not_celebrate() {
        let mut before = sample(100, 92.0, 9000, true);
        before.five_hour.as_mut().unwrap().remaining = 0.0;
        let mut detector = ResetDetector::default();
        detector.observe(&before, Some(1));
        assert!(detector
            .observe(&sample(160, 92.0, 9000, true), Some(1))
            .is_none());
        assert!(detector
            .observe(&sample(220, 92.0, 9000, true), Some(1))
            .is_none());
        before.credits[0].expires_at = 150;
        let mut detector = ResetDetector::default();
        detector.observe(&before, Some(1));
        assert!(detector
            .observe(&sample(160, 100.0, 20000, false), Some(1))
            .is_none());
        assert!(detector
            .observe(&sample(220, 98.0, 20000, false), Some(1))
            .is_none());
    }
    #[test]
    fn confirmation_waits_and_keeps_candidate_across_fast_updates() {
        for (card, delay) in [(false, 60), (true, 60)] {
            let mut d = ResetDetector::default();
            d.observe(&sample(100, 20.0, 9000, true), Some(1));
            d.observe(&sample(160, 100.0, 20000, !card), Some(1));
            assert!(d
                .observe(&sample(161, 100.0, 20000, !card), Some(1))
                .is_none());
            assert!(d
                .observe(&sample(160 + delay - 1, 100.0, 20000, !card), Some(1))
                .is_none());
            assert!(d
                .observe(&sample(160 + delay, 98.0, 20000, !card), Some(1))
                .is_some());
        }
    }
    #[test]
    fn moderate_correction_and_card_without_boundary_change_are_rejected() {
        for (remaining, reset, card) in [
            (94.0, 9000, true),
            (79.0, 20000, true),
            (100.0, 9000, false),
        ] {
            let mut d = ResetDetector::default();
            d.observe(&sample(100, 70.0, 9000, true), Some(1));
            d.observe(&sample(160, remaining, reset, card), Some(1));
            assert!(d
                .observe(&sample(220, remaining, reset, card), Some(1))
                .is_none());
        }
    }
    #[test]
    fn incomplete_inventory_and_unrestored_five_hour_are_rejected() {
        for mode in 0..4 {
            let mut before = sample(100, 20.0, 9000, true);
            before.five_hour.as_mut().unwrap().remaining = 30.0;
            let mut after = sample(160, 100.0, 20000, false);
            match mode {
                0 => after.available_credits = None,
                1 => before.available_credits = Some(2),
                2 => after.five_hour.as_mut().unwrap().remaining = 30.0,
                _ => after.weekly.as_mut().unwrap().minutes = 43200,
            }
            let mut d = ResetDetector::default();
            d.observe(&before, Some(1));
            d.observe(&after, Some(1));
            after.fetched_at = 220;
            assert!(d.observe(&after, Some(1)).is_none());
        }
    }
    #[test]
    fn confirmation_near_original_boundary_is_rejected() {
        let mut d = ResetDetector::default();
        d.observe(&sample(100, 20.0, 300, true), Some(1));
        d.observe(&sample(160, 100.0, 20000, true), Some(1));
        assert!(d
            .observe(&sample(220, 100.0, 20000, true), Some(1))
            .is_none());
    }
    #[test]
    fn candidate_is_cancelled_by_changed_account_inventory_or_failed_read() {
        for mode in 0..7 {
            let mut d = ResetDetector::default();
            let mut before = sample(100, 20.0, 9000, true);
            before.five_hour.as_mut().unwrap().remaining = 30.0;
            d.observe(&before, Some(1));
            d.observe(&sample(160, 100.0, 20000, false), Some(1));
            let mut confirm = sample(220, 98.0, 20000, false);
            let mut account = Some(1);
            match mode {
                0 => account = Some(2), // Includes plan/source fingerprint changes.
                1 => account = None,
                2 => confirm = sample(220, 98.0, 20000, true),
                3 => confirm.five_hour.as_mut().unwrap().remaining = 30.0,
                4 => confirm.available_credits = None,
                5 => confirm.weekly.as_mut().unwrap().remaining = 93.0,
                _ => d.clear(), // Connection failure clears pending evidence.
            }
            assert!(d.observe(&confirm, account).is_none());
            confirm.fetched_at = 280;
            assert!(d.observe(&confirm, account).is_none());
        }
    }
    #[test]
    fn expiring_inventory_during_official_confirmation_cancels_candidate() {
        let mut before = sample(100, 20.0, 9000, true);
        before.credits[0].expires_at = 200;
        let mut candidate = sample(160, 100.0, 20000, true);
        candidate.credits[0].expires_at = 200;
        let mut d = ResetDetector::default();
        d.observe(&before, Some(1));
        d.observe(&candidate, Some(1));
        assert!(d
            .observe(&sample(220, 100.0, 20000, false), Some(1))
            .is_none());
    }
}
