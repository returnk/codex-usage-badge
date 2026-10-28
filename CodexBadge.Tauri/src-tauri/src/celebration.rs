use crate::domain::{QuotaWindow, Snapshot};
use serde::{Deserialize, Serialize};

const HISTORY_TTL: i64 = 7 * 86400;
const EVENT_TTL: i64 = 86400;
const CONFIRM_SECONDS: i64 = 3;
// Two quick reads, then normal polling, for at most five minutes. This is a
// local evidence budget, not a promise about the service's synchronization time.
const CANDIDATE_TTL: i64 = 300;

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    recovery_from: Option<i64>,
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
    fn correlation(&self) -> u64 {
        stable_fingerprint(&serde_json::to_vec(self).unwrap())
    }
    fn event_id(&self) -> String {
        format!("reset-{:016x}", self.correlation())
    }
}
fn recent(now: i64, then: i64, ttl: i64) -> bool {
    (0..=ttl).contains(&now.saturating_sub(then))
}
impl CelebrationState {
    // Coalesce notifications into the existing confirmation/normal schedule;
    // a pending candidate must not turn repeated notifications into 1s polling.
    pub fn refresh_delay(&self, account: Option<u64>, now: i64, notified: bool) -> u64 {
        if self
            .histories
            .iter()
            .any(|h| Some(h.account) == account && h.detector.candidate.is_some())
        {
            self.confirmation_delay(account, now).unwrap_or(60)
        } else if notified {
            1
        } else {
            60
        }
    }
    pub fn confirmation_delay(&self, account: Option<u64>, now: i64) -> Option<u64> {
        let candidate = self
            .histories
            .iter()
            .find(|h| Some(h.account) == account)?
            .detector
            .candidate
            .as_ref()?;
        if !recent(now, candidate.snapshot.fetched_at, CANDIDATE_TTL)
            || candidate.quick_reads >= 2
            || candidate.interrupted
        {
            return None;
        }
        Some(
            (candidate
                .last_check_at
                .unwrap_or(candidate.snapshot.fetched_at)
                .saturating_add(CONFIRM_SECONDS)
                .saturating_sub(now))
            .clamp(1, 60) as u64,
        )
    }
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
            if history
                .detector
                .candidate
                .as_ref()
                .is_some_and(|c| now.saturating_sub(c.snapshot.fetched_at) > CANDIDATE_TTL)
            {
                let c = history.detector.candidate.take().unwrap();
                c.record(
                    "celebration_rejected",
                    "candidate_timeout",
                    &c.snapshot,
                    now,
                );
            }
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
            if let Some(candidate) = &mut history.detector.candidate {
                candidate.interrupted = true;
                candidate.record(
                    "celebration_waiting",
                    "read_interrupted",
                    &candidate.snapshot,
                    crate::epoch_now(),
                );
            }
        }
    }
    pub fn account_changed(&mut self) {
        for history in &mut self.histories {
            if let Some(candidate) = history.detector.candidate.take() {
                candidate.record(
                    "celebration_rejected",
                    "account_changed",
                    &candidate.snapshot,
                    crate::epoch_now(),
                );
            }
        }
    }
    pub fn observe(&mut self, snapshot: &Snapshot, account: Option<u64>) {
        self.prune(snapshot.fetched_at);
        let Some(account) = account else {
            self.interrupt();
            return;
        };
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
            crate::diagnostics::record(format!("celebration_queue reason=duplicate_event correlation={} identity={} accepted=false", key.correlation(), key.account));
            return false;
        }
        crate::diagnostics::record(format!(
            "celebration_queue correlation={} identity={} accepted=true",
            key.correlation(),
            key.account
        ));
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
        if let Some(played) = next.handled.last().filter(|e| e.key.event_id() == id) {
            crate::diagnostics::record(format!(
                "celebration_acknowledged correlation={} identity={}",
                played.key.correlation(),
                played.key.account
            ));
        }
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
    #[serde(default)]
    five_hour: bool,
    #[serde(default)]
    quick_reads: u8,
    #[serde(default)]
    last_check_at: Option<i64>,
    #[serde(default)]
    interrupted: bool,
    #[serde(default)]
    resumed_at: Option<i64>,
}
impl ResetCandidate {
    fn record(&self, event: &str, reason: &str, current: &Snapshot, now: i64) {
        let value = |s: &Snapshot| {
            window(s, self.five_hour)
                .map(|w| w.remaining)
                .unwrap_or(-1.0)
        };
        let reset = |s: &Snapshot| {
            window(s, self.five_hour)
                .and_then(|w| w.resets_at)
                .unwrap_or(0)
        };
        let count = |s: &Snapshot| {
            s.available_credits
                .map(|n| n.to_string())
                .unwrap_or_else(|| "none".into())
        };
        crate::diagnostics::record(format!(
            "{event} reason={reason} correlation={} identity={} window_minutes={} before_at={} candidate_at={} sampled_at={} decision_at={} original_reset={} candidate_reset={} current_reset={} before_remaining={} candidate_remaining={} current_remaining={} before_count={} current_count={} before_details={} current_details={} quick_reads={} retain={}",
            self.key.correlation(), self.key.account, if self.five_hour {300} else {10080},
            self.before.fetched_at, self.snapshot.fetched_at, current.fetched_at, now, self.original_reset,
            reset(&self.snapshot), reset(current), value(&self.before), value(&self.snapshot), value(current),
            count(&self.before), count(current), self.before.credits.len(), current.credits.len(), self.quick_reads,
            event == "celebration_waiting"
        ));
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ResetDetector {
    previous: Option<(Snapshot, u64)>,
    candidate: Option<ResetCandidate>,
}
fn valid_snapshot(snapshot: &Snapshot) -> bool {
    let windows = [(&snapshot.five_hour, 300), (&snapshot.weekly, 10080)];
    (snapshot.five_hour.is_some() || snapshot.weekly.is_some())
        && windows.iter().all(|(window, minutes)| {
            window.is_none()
                || window.as_ref().is_some_and(|w| {
                    w.minutes == *minutes
                        && w.remaining.is_finite()
                        && (0.0..=100.0).contains(&w.remaining)
                        && w.resets_at.is_some_and(|at| at > snapshot.fetched_at)
                })
        })
}
fn window(snapshot: &Snapshot, five_hour: bool) -> Option<&QuotaWindow> {
    if five_hour {
        snapshot.five_hour.as_ref()
    } else {
        snapshot.weekly.as_ref()
    }
}
impl ResetDetector {
    pub fn clear(&mut self) {
        *self = Self::default();
    }
    pub fn observe(&mut self, snapshot: &Snapshot, account: Option<u64>) -> Option<ResetKey> {
        let Some(account) = account else {
            if let Some(c) = &self.candidate {
                c.record(
                    "celebration_rejected",
                    "account_changed",
                    snapshot,
                    snapshot.fetched_at,
                );
            }
            self.clear();
            return None;
        };
        if self.candidate.as_ref().is_some_and(|c| {
            snapshot.fetched_at.saturating_sub(c.snapshot.fetched_at) > CANDIDATE_TTL
        }) {
            let c = self.candidate.take().unwrap();
            c.record(
                "celebration_rejected",
                "candidate_timeout",
                snapshot,
                snapshot.fetched_at,
            );
        }
        if !valid_snapshot(snapshot) {
            if let Some(c) = &mut self.candidate {
                c.interrupted = true;
                c.record(
                    "celebration_waiting",
                    "invalid_snapshot",
                    snapshot,
                    snapshot.fetched_at,
                );
            } else {
                crate::diagnostics::record(format!(
                    "celebration_observation reason=invalid_snapshot identity={} sampled_at={}",
                    account, snapshot.fetched_at
                ));
            }
            return None;
        }
        // RPC replies and notifications can repeat the exact sample within one
        // second. They add no evidence and must not erase a pending recovery.
        if self.previous.as_ref().is_some_and(|(before, old_account)| {
            *old_account == account
                && (before == snapshot || snapshot.fetched_at < before.fetched_at)
        }) {
            return None;
        }
        let previous = self.previous.replace((snapshot.clone(), account));
        let Some((before, old_account)) = previous else {
            crate::diagnostics::record(format!(
                "celebration_observation reason=baseline identity={} sampled_at={}",
                account, snapshot.fetched_at
            ));
            return None;
        };
        if account != old_account
            || before.five_hour.is_some() != snapshot.five_hour.is_some()
            || before.weekly.is_some() != snapshot.weekly.is_some()
            || !(0..=HISTORY_TTL).contains(&snapshot.fetched_at.saturating_sub(before.fetched_at))
            || !valid_snapshot(&before)
            || !valid_snapshot(snapshot)
        {
            let reason = if account != old_account {
                "account_changed"
            } else if !recent(snapshot.fetched_at, before.fetched_at, HISTORY_TTL) {
                "history_expired"
            } else {
                "window_changed"
            };
            if let Some(c) = &self.candidate {
                c.record(
                    "celebration_rejected",
                    reason,
                    snapshot,
                    snapshot.fetched_at,
                );
            }
            self.candidate = None;
            return None;
        }
        if let Some(mut candidate) = self.candidate.take() {
            let Some(target) = window(&candidate.snapshot, candidate.five_hour) else {
                return None;
            };
            let Some(current) = window(snapshot, candidate.five_hour) else {
                return None;
            };
            let Some(original) = window(&candidate.before, candidate.five_hour) else {
                return None;
            };
            let card_reset = candidate.key.consumed_credit.is_some();
            let cancelled = if candidate.original_reset
                <= snapshot
                    .fetched_at
                    .saturating_add(if card_reset { 120 } else { 300 })
            {
                Some("natural_boundary")
            } else if current.resets_at < original.resets_at {
                Some("period_incompatible")
            } else if current.resets_at == original.resets_at
                && current.remaining <= original.remaining
            {
                Some("quota_rollback")
            } else {
                None
            };
            if let Some(reason) = cancelled {
                candidate.record(
                    "celebration_rejected",
                    reason,
                    snapshot,
                    snapshot.fetched_at,
                );
                return None;
            }
            let confirmed = (current.resets_at > original.resets_at
                && (target.resets_at > original.resets_at
                    || current.remaining > original.remaining))
                || (current.remaining > original.remaining
                    && current.remaining >= target.remaining - 5.0);
            let due = candidate
                .last_check_at
                .unwrap_or(candidate.snapshot.fetched_at)
                .saturating_add(CONFIRM_SECONDS);
            if snapshot.fetched_at >= due && candidate.quick_reads < 2 {
                candidate.quick_reads += 1;
                candidate.last_check_at = Some(snapshot.fetched_at);
            }
            if candidate.interrupted {
                candidate.interrupted = false;
                candidate.resumed_at = Some(snapshot.fetched_at);
            }
            let confirmation_start = candidate
                .resumed_at
                .unwrap_or(candidate.snapshot.fetched_at);
            if confirmed {
                if snapshot.fetched_at.saturating_sub(confirmation_start) < CONFIRM_SECONDS {
                    candidate.record(
                        "celebration_waiting",
                        "confirmation_delay",
                        snapshot,
                        snapshot.fetched_at,
                    );
                    self.candidate = Some(candidate);
                    return None;
                }
                candidate.record(
                    "celebration_reset_confirmed",
                    if card_reset {
                        "consumed_live_credit"
                    } else {
                        "early_quota_recovery"
                    },
                    snapshot,
                    snapshot.fetched_at,
                );
                return Some(candidate.key);
            }
            candidate.record(
                "celebration_waiting",
                "evidence_pending",
                snapshot,
                snapshot.fetched_at,
            );
            self.candidate = Some(candidate);
            return None;
        }
        let consumed = before.credits.iter().find(|card| {
            !card.id.is_empty()
                && card.expires_at > snapshot.fetched_at.saturating_add(120)
                && !snapshot.credits.iter().any(|next| next.id == card.id)
        });
        let consumed_credit = consumed.filter(|card| {
            before.available_credits.is_some()
                && before.available_credits == Some(before.credits.len() as u64)
                && snapshot.available_credits == Some(snapshot.credits.len() as u64)
                && before.available_credits
                    == snapshot.available_credits.and_then(|n| n.checked_add(1))
                && before
                    .credits
                    .iter()
                    .filter(|c| c.id != card.id)
                    .all(|c| snapshot.credits.contains(c))
        });
        let card_reset = consumed_credit.is_some();
        let recovered = [false, true].into_iter().find_map(|five_hour| {
            let old = window(&before, five_hour)?;
            let current = window(snapshot, five_hour)?;
            let old_reset = old.resets_at?;
            let gain = current.remaining - old.remaining;
            // A small change needs an actual return to full. Larger recoveries
            // keep tolerating work performed between reads. Scheduled expiry is excluded.
            let full_recovery = current.remaining >= 99.99 && gain > 0.0;
            let large_recovery = current.remaining >= 95.0 && gain >= 10.0;
            let spent_card_recovery = card_reset
                && current.remaining >= 99.0
                && gain >= 5.0
                && current.resets_at >= old.resets_at.map(|at| at.saturating_add(120));
            let eligible = old_reset > snapshot.fetched_at.saturating_add(if card_reset {120} else {300})
                && current.resets_at >= old.resets_at
                && (full_recovery || large_recovery || spent_card_recovery)
                // An hourly-only change in a dual-window account must leave the
                // weekly quota full, rather than mistaking an ordinary hourly refill for a reset.
                && (!five_hour || snapshot.weekly.as_ref().is_none_or(|w| w.remaining >= 99.99)
                    && before.weekly.as_ref().is_none_or(|w| w.resets_at.is_some_and(|at| at > snapshot.fetched_at + 300)));
            eligible.then_some((five_hour, old_reset, current.resets_at.unwrap()))
        });
        if let Some((five_hour, old_reset, new_reset)) = recovered {
            let candidate = ResetCandidate {
                before: before.clone(),
                snapshot: snapshot.clone(),
                original_reset: old_reset,
                five_hour,
                quick_reads: 0,
                last_check_at: None,
                interrupted: false,
                resumed_at: None,
                key: ResetKey {
                    account,
                    resets_at: new_reset,
                    consumed_credit: consumed_credit.map(|card| card.id.clone()),
                    recovery_from: Some(before.fetched_at),
                },
            };
            candidate.record(
                "celebration_candidate",
                "early_quota_recovery",
                snapshot,
                snapshot.fetched_at,
            );
            self.candidate = Some(candidate);
        } else {
            for five_hour in [false, true] {
                if let (Some(old), Some(current)) =
                    (window(&before, five_hour), window(snapshot, five_hour))
                {
                    let reason = if old.resets_at.is_some_and(|at| {
                        at <= snapshot
                            .fetched_at
                            .saturating_add(if card_reset { 120 } else { 300 })
                    }) {
                        "natural_boundary"
                    } else if current.resets_at < old.resets_at {
                        "period_incompatible"
                    } else {
                        "below_threshold"
                    };
                    crate::diagnostics::record(format!("celebration_observation reason={} identity={} window_minutes={} before_at={} sampled_at={} original_reset={} current_reset={} before_remaining={} current_remaining={}", reason, account, current.minutes, before.fetched_at, snapshot.fetched_at, old.resets_at.unwrap_or(0), current.resets_at.unwrap_or(0), old.remaining, current.remaining));
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recovery_with_count_only_partial_or_unknown_cards_confirms() {
        for mode in 0..3 {
            let mut d = ResetDetector::default();
            let mut before = sample(100, 99.0, 9000, true);
            let mut after = sample(160, 100.0, 9000, true);
            for s in [&mut before, &mut after] {
                match mode {
                    0 => s.credits.clear(),
                    1 => s.available_credits = Some(4),
                    _ => {
                        s.available_credits = None;
                        s.credits.clear();
                    }
                }
            }
            d.observe(&before, Some(1));
            d.observe(&after, Some(1));
            after.fetched_at = 163;
            after.weekly.as_mut().unwrap().remaining = 99.5;
            assert!(d.observe(&after, Some(1)).is_some(), "mode={mode}");
        }
    }
    #[test]
    fn staged_card_updates_and_adjusted_deadline_keep_original_recovery() {
        for reset in [9000, 19999, 21000] {
            let mut state = CelebrationState::existing_install();
            state.observe(&sample(100, 20.0, 9000, true), Some(1));
            state.observe(&sample(160, 100.0, 20000, true), Some(1));
            state = restored(&state);
            state.observe(&sample(163, 98.0, reset, false), Some(1));
            assert_eq!(
                played(&mut state, Some(1), 163),
                Some("reset"),
                "reset={reset}"
            );
            state = restored(&state);
            assert_eq!(played(&mut state, Some(1), 164), None);
        }
    }
    #[test]
    fn uncertain_confirmation_waits_without_replacing_recovery_baseline() {
        let mut state = CelebrationState::existing_install();
        state.observe(&sample(100, 20.0, 9000, true), Some(1));
        state.observe(&sample(160, 100.0, 9000, true), Some(1));
        state.observe(&sample(163, 90.0, 9000, false), Some(1));
        assert!(state.peek(Some(1), 163).is_none());
        assert_eq!(
            state.histories[0]
                .detector
                .candidate
                .as_ref()
                .unwrap()
                .before
                .fetched_at,
            100
        );
        state = restored(&state);
        state.observe(&sample(166, 89.0, 9000, false), Some(1));
        assert_eq!(state.confirmation_delay(Some(1), 166), None);
        assert_eq!(state.refresh_delay(Some(1), 166, true), 60);
        state.observe(&sample(226, 88.0, 20000, false), Some(1));
        assert_eq!(played(&mut state, Some(1), 226), Some("reset"));
    }
    #[test]
    fn waiting_candidate_expires_and_does_not_restart_from_original_baseline() {
        let mut state = CelebrationState::existing_install();
        state.observe(&sample(100, 20.0, 9000, true), Some(1));
        state.observe(&sample(160, 100.0, 9000, true), Some(1));
        for at in [163, 166, 226, 286, 346, 406, 460] {
            state.observe(&sample(at, 90.0, 9000, true), Some(1));
            assert!(state.histories[0].detector.candidate.is_some());
        }
        state.observe(&sample(461, 90.0, 9000, true), Some(1));
        assert!(state.histories[0].detector.candidate.is_none());
        assert!(state.peek(Some(1), 461).is_none());
        assert_eq!(state.confirmation_delay(Some(1), 461), None);
    }
    #[test]
    fn invalid_quota_read_preserves_candidate_until_fresh_confirmation() {
        let mut state = CelebrationState::existing_install();
        state.observe(&sample(100, 20.0, 9000, true), Some(1));
        state.observe(&sample(160, 100.0, 20000, true), Some(1));
        let mut invalid = sample(163, 100.0, 20000, true);
        invalid.weekly.as_mut().unwrap().remaining = f64::NAN;
        state.observe(&invalid, Some(1));
        assert_eq!(
            state.histories[0]
                .detector
                .candidate
                .as_ref()
                .unwrap()
                .before
                .fetched_at,
            100
        );
        state = restored(&state);
        state.observe(&sample(220, 98.0, 20000, false), Some(1));
        assert!(state.peek(Some(1), 220).is_none());
        state.observe(&sample(223, 97.0, 20001, false), Some(1));
        assert_eq!(played(&mut state, Some(1), 223), Some("reset"));
    }
    #[test]
    fn notification_polling_and_restart_do_not_replenish_quick_reads() {
        let mut state = CelebrationState::existing_install();
        assert_eq!(state.refresh_delay(Some(1), 100, false), 60);
        assert_eq!(state.refresh_delay(Some(1), 100, true), 1);
        state.observe(&sample(100, 20.0, 9000, true), Some(1));
        state.observe(&sample(160, 100.0, 9000, true), Some(1));
        for at in [160, 161, 162] {
            assert_eq!(state.refresh_delay(Some(1), at, true), (163 - at) as u64);
        }
        state.observe(&sample(163, 90.0, 9000, true), Some(1));
        assert_eq!(state.refresh_delay(Some(1), 163, true), 3);
        state.observe(&sample(166, 90.0, 9000, true), Some(1));
        state = restored(&state);
        for at in [166, 167, 180, 226] {
            assert_eq!(state.refresh_delay(Some(1), at, true), 60);
        }
        state.interrupt();
        state.observe(&sample(226, 90.0, 9000, true), Some(1));
        assert_eq!(state.refresh_delay(Some(1), 226, true), 60);
    }
    #[test]
    fn older_settings_candidate_and_handled_keys_remain_compatible() {
        let mut state = CelebrationState::existing_install();
        state.observe(&sample(100, 20.0, 9000, true), Some(1));
        state.observe(&sample(160, 100.0, 20000, true), Some(1));
        let mut json = serde_json::to_value(&state).unwrap();
        let candidate = json["histories"][0]["detector"]["candidate"]
            .as_object_mut()
            .unwrap();
        for key in ["quick_reads", "last_check_at", "interrupted", "resumed_at"] {
            candidate.remove(key);
        }
        candidate.insert("five_hour_before".into(), serde_json::json!(100.0));
        let mut state: CelebrationState = serde_json::from_value(json).unwrap();
        state.observe(&sample(163, 98.0, 20001, false), Some(1));
        assert_eq!(played(&mut state, Some(1), 163), Some("reset"));
        let handled_id = state.handled[0].key.event_id();
        state = restored(&state);
        assert_eq!(state.handled[0].key.event_id(), handled_id);
        let key = state.handled[0].key.clone();
        assert!(!state.queue(key, 164));
    }
    #[test]
    fn account_change_cancels_candidate_and_another_account_cannot_confirm_it() {
        let mut state = CelebrationState::existing_install();
        state.observe(&sample(100, 20.0, 9000, true), Some(1));
        state.observe(&sample(160, 100.0, 20000, true), Some(1));
        state.account_changed();
        state.observe(&sample(163, 98.0, 20000, true), Some(2));
        state.observe(&sample(166, 97.0, 20000, true), Some(2));
        assert!(state.peek(Some(1), 166).is_none());
        assert!(state.peek(Some(2), 166).is_none());
        state.observe(&sample(169, 97.0, 20000, true), Some(1));
        assert!(state.peek(Some(1), 169).is_none());
    }
    #[test]
    fn consumed_card_still_confirms_after_some_of_the_restored_quota_is_used() {
        let mut state = CelebrationState::existing_install();
        state.observe(&sample(100, 92.0, 9000, true), Some(1));
        state.observe(&sample(160, 99.0, 20000, false), Some(1));
        assert_eq!(state.confirmation_delay(Some(1), 160), Some(3));
        state.observe(&sample(163, 98.0, 20000, false), Some(1));
        assert_eq!(played(&mut state, Some(1), 163), Some("reset"));
        assert_eq!(played(&mut state, Some(1), 164), None);
    }
    #[test]
    fn duplicate_same_second_read_preserves_pending_confirmation() {
        for remaining in [100.0, 99.9] {
            let mut d = ResetDetector::default();
            d.observe(&sample(100, 20.0, 9000, true), Some(1));
            d.observe(&sample(160, 100.0, 9000, true), Some(1));
            assert!(d
                .observe(&sample(160, remaining, 9000, true), Some(1))
                .is_none());
            assert!(d
                .observe(&sample(163, remaining, 9000, true), Some(1))
                .is_some());
        }
    }
    #[test]
    fn small_restores_confirm_after_three_seconds_in_each_supported_window() {
        for card in [false, true] {
            for mode in 0..3 {
                let mut before = sample(100, 99.5, 9000, true);
                let mut after = sample(160, 100.0, 9000, !card);
                if mode != 0 {
                    before.five_hour.as_mut().unwrap().remaining = 99.5;
                    before.weekly.as_mut().unwrap().remaining = 100.0;
                }
                if mode == 2 {
                    before.weekly = None;
                    after.weekly = None;
                }
                let mut d = ResetDetector::default();
                d.observe(&before, Some(1));
                assert!(d.observe(&after, Some(1)).is_none());
                after.fetched_at = 162;
                assert!(d.observe(&after, Some(1)).is_none());
                after.fetched_at = 163;
                assert!(
                    d.observe(&after, Some(1)).is_some(),
                    "card={card} mode={mode}"
                );
            }
        }
    }
    #[test]
    fn unchanged_full_quota_and_natural_hourly_expiry_do_not_celebrate() {
        for hourly_expired in [false, true] {
            let mut before = sample(100, 100.0, 9000, true);
            if hourly_expired {
                before.five_hour.as_mut().unwrap().remaining = 10.0;
                before.five_hour.as_mut().unwrap().resets_at = Some(150);
            }
            let mut after = sample(160, 100.0, 9000, true);
            let mut d = ResetDetector::default();
            d.observe(&before, Some(1));
            d.observe(&after, Some(1));
            after.fetched_at = 163;
            assert!(d.observe(&after, Some(1)).is_none());
        }
    }
    #[test]
    fn confirmed_new_reset_boundary_survives_immediate_heavy_use() {
        let mut d = ResetDetector::default();
        d.observe(&sample(100, 20.0, 9000, true), Some(1));
        d.observe(&sample(160, 100.0, 20000, true), Some(1));
        assert!(d
            .observe(&sample(163, 90.0, 20000, true), Some(1))
            .is_some());
    }
    #[test]
    fn separate_recoveries_with_the_same_reset_deadline_can_each_play_once() {
        let mut state = CelebrationState::existing_install();
        for (at, remaining) in [(100, 95.0), (160, 100.0), (163, 100.0)] {
            state.observe(&sample(at, remaining, 9000, true), Some(1));
        }
        assert_eq!(played(&mut state, Some(1), 163), Some("reset"));
        assert_eq!(played(&mut state, Some(1), 164), None);
        state = restored(&state);
        for (at, remaining) in [(180, 97.0), (200, 100.0), (203, 100.0)] {
            state.observe(&sample(at, remaining, 9000, true), Some(1));
        }
        assert_eq!(played(&mut state, Some(1), 203), Some("reset"));
        assert_eq!(played(&mut state, Some(1), 204), None);
    }
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
                    recovery_from: None,
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
        for invalid_read in [false, true] {
            let mut state = CelebrationState::existing_install();
            state.observe(&sample(100, 20.0, 9000, true), Some(1));
            state.observe(&sample(160, 100.0, 20000, true), Some(1));
            if invalid_read {
                let mut incomplete = sample(161, 100.0, 20000, true);
                incomplete.weekly.as_mut().unwrap().resets_at = None;
                state.observe(&incomplete, Some(1));
            } else {
                state.interrupt();
            }
            assert_eq!(state.confirmation_delay(Some(1), 161), None);
            state = restored(&state);
            state.observe(&sample(220, 100.0, 20000, true), Some(1));
            assert_eq!(state.confirmation_delay(Some(1), 220), Some(3));
            assert!(state.peek(Some(1), 220).is_none());
            state.observe(&sample(222, 98.0, 20000, true), Some(1));
            assert!(state.peek(Some(1), 222).is_none());
            state.observe(&sample(223, 98.0, 20000, true), Some(1));
            assert!(state.peek(Some(1), 223).is_some());
        }
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
                recovery_from: None,
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
                recovery_from: None,
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
    fn normal_five_hour_recovery_is_rejected_and_expiring_card_does_not_veto_weekly_recovery() {
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
            .is_some());
    }
    #[test]
    fn confirmation_waits_and_keeps_candidate_across_fast_updates() {
        for (card, delay) in [(false, 3), (true, 3)] {
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
    fn moderate_corrections_without_a_return_to_full_are_rejected() {
        for (remaining, reset, card) in
            [(94.0, 9000, true), (79.0, 20000, true), (99.5, 9000, true)]
        {
            let mut d = ResetDetector::default();
            d.observe(&sample(100, 98.0, 9000, true), Some(1));
            d.observe(&sample(160, remaining, reset, card), Some(1));
            assert!(d
                .observe(&sample(220, remaining, reset, card), Some(1))
                .is_none());
        }
    }
    #[test]
    fn weekly_recovery_allows_partial_cards_and_unrestored_hourly_but_rejects_invalid_window() {
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
            assert_eq!(
                d.observe(&after, Some(1)).is_some(),
                mode != 3,
                "mode={mode}"
            );
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
    fn candidate_is_cancelled_by_identity_window_or_quota_rollback() {
        for mode in 0..6 {
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
                2 => confirm.weekly = None,
                3 => confirm.weekly.as_mut().unwrap().minutes = 43200,
                4 => confirm = sample(220, 20.0, 9000, true),
                _ => d.clear(), // Connection failure clears pending evidence.
            }
            assert!(d.observe(&confirm, account).is_none());
            confirm.fetched_at = 280;
            assert!(d.observe(&confirm, account).is_none());
        }
    }
    #[test]
    fn expiring_inventory_during_official_confirmation_does_not_cancel_quota_evidence() {
        let mut before = sample(100, 20.0, 9000, true);
        before.credits[0].expires_at = 200;
        let mut candidate = sample(160, 100.0, 20000, true);
        candidate.credits[0].expires_at = 200;
        let mut d = ResetDetector::default();
        d.observe(&before, Some(1));
        d.observe(&candidate, Some(1));
        assert!(d
            .observe(&sample(220, 100.0, 20000, false), Some(1))
            .is_some());
    }
    #[test]
    fn fast_notifications_keep_the_original_confirmation_deadline_and_account() {
        let mut state = CelebrationState::existing_install();
        state.observe(&sample(100, 20.0, 9000, true), Some(1));
        state.observe(&sample(160, 100.0, 20000, true), Some(1));
        assert_eq!(state.confirmation_delay(Some(1), 160), Some(3));
        assert_eq!(state.confirmation_delay(Some(2), 160), None);
        state.observe(&sample(161, 100.0, 20000, true), Some(1));
        assert_eq!(state.confirmation_delay(Some(1), 161), Some(2));
        state = restored(&state);
        assert_eq!(state.confirmation_delay(Some(1), 162), Some(1));
        state.observe(&sample(163, 99.0, 20000, true), Some(1));
        assert_eq!(state.confirmation_delay(Some(1), 163), None);
        assert_eq!(played(&mut state, Some(1), 163), Some("reset"));
    }
}
