use crate::domain::Snapshot;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CelebrationState {
    pub welcome_done: bool,
    pub pending_reset: Option<ResetEvent>,
    pub last_reset: Option<ResetKey>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResetKey {
    account: u64,
    resets_at: i64,
    consumed_credit: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResetEvent {
    pub key: ResetKey,
    detected_at: i64,
}

impl CelebrationState {
    pub fn existing_install() -> Self {
        Self {
            welcome_done: true,
            ..Self::default()
        }
    }
    pub fn queue(&mut self, key: ResetKey, now: i64) -> bool {
        if self.last_reset.as_ref() == Some(&key) {
            return false;
        }
        self.last_reset = Some(key.clone());
        self.pending_reset = Some(ResetEvent {
            key,
            detected_at: now,
        });
        true
    }
    pub fn take(&mut self, account: Option<u64>, now: i64) -> Option<&'static str> {
        if !self.welcome_done {
            self.welcome_done = true;
            self.pending_reset = None;
            return Some("welcome");
        }
        let pending = self.pending_reset.take()?;
        (Some(pending.key.account) == account
            && (0..=86400).contains(&now.saturating_sub(pending.detected_at)))
        .then_some("reset")
    }
}

#[derive(Clone)]
struct ResetCandidate {
    snapshot: Snapshot,
    key: ResetKey,
    original_reset: i64,
    five_hour_before: f64,
}
#[derive(Clone, Default)]
pub struct ResetDetector {
    previous: Option<(Snapshot, u64)>,
    candidate: Option<ResetCandidate>,
}
fn valid_snapshot(snapshot: &Snapshot) -> bool {
    let windows = [(&snapshot.five_hour, 300), (&snapshot.weekly, 10080)];
    windows.iter().all(|(window, minutes)| {
        window.as_ref().is_some_and(|w| {
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
        let previous = self.previous.replace((snapshot.clone(), account));
        let Some((before, old_account)) = previous else {
            return None;
        };
        if account != old_account
            || !(1..=180).contains(&snapshot.fetched_at.saturating_sub(before.fetched_at))
            || !valid_snapshot(&before)
            || !valid_snapshot(snapshot)
        {
            self.candidate = None;
            return None;
        }
        let old = before.weekly.as_ref().unwrap();
        let current = snapshot.weekly.as_ref().unwrap();
        if let Some(candidate) = self.candidate.take() {
            let target = candidate.snapshot.weekly.as_ref().unwrap();
            let card_reset = candidate.key.consumed_credit.is_some();
            let five = snapshot.five_hour.as_ref().unwrap().remaining;
            let confirmed = candidate.original_reset > snapshot.fetched_at.saturating_add(120)
                && current.resets_at == target.resets_at
                && current.remaining >= target.remaining - 5.0
                && same_inventory(&candidate.snapshot, snapshot)
                && (!card_reset || candidate.five_hour_before >= 99.0 || five >= 94.0);
            if confirmed {
                let wait = if card_reset { 30 } else { 60 };
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
        }
        let old_reset = old.resets_at.unwrap();
        let new_reset = current.resets_at.unwrap();
        if old_reset <= snapshot.fetched_at.saturating_add(120)
            || new_reset < old_reset
            || current.remaining < 99.0
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
        let five_hour_before = before.five_hour.as_ref().unwrap().remaining;
        let eligible = if consumed_credit.is_some() {
            gain >= 5.0
                && new_reset >= old_reset.saturating_add(120)
                && (five_hour_before >= 99.0
                    || snapshot.five_hour.as_ref().unwrap().remaining >= 99.0)
        } else {
            gain >= 15.0 && same_inventory(&before, snapshot)
        };
        if eligible {
            self.candidate = Some(ResetCandidate {
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
    #[test]
    fn welcome_once_and_existing_install_never_welcomes() {
        let mut state = CelebrationState::default();
        assert_eq!(state.take(None, 0), Some("welcome"));
        let mut state: CelebrationState =
            serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();
        assert!(state.take(None, 0).is_none());
        assert!(CelebrationState::existing_install().take(None, 0).is_none());
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
        assert_eq!(state.take(Some(1), 220), Some("reset"));
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
            (400, 20000, Some(1)),
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
        for (card, delay) in [(false, 60), (true, 30)] {
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
            (95.0, 9000, true),
            (95.0, 20000, true),
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
