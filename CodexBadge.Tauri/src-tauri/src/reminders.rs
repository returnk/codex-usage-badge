use crate::domain::Snapshot;
use chrono::{Local, TimeZone};
use serde::{Deserialize, Serialize};

pub struct Hint {
    pub level: &'static str,
    pub messages: Vec<String>,
}

#[cfg(test)]
fn hints(snapshot: Option<&Snapshot>, fresh: bool, now: i64) -> Hint {
    ReminderTracker::default().hints(snapshot, fresh, now, None)
}

pub fn countdown(epoch: Option<i64>, now: i64) -> String {
    let Some(epoch) = epoch else {
        return String::new();
    };
    if epoch <= now {
        return "等待额度更新".into();
    }
    let minutes = epoch.saturating_sub(now).saturating_add(59) / 60;
    format!("约{}小时{}分钟后重置", minutes / 60, minutes % 60)
}

pub fn updated_at(epoch: Option<i64>) -> String {
    epoch
        .and_then(|v| Local.timestamp_opt(v, 0).single())
        .map(|v| v.format("%H:%M").to_string())
        .unwrap_or_default()
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct ReminderTracker {
    credits: Vec<CreditTracker>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
struct CreditTracker {
    id: String,
    expires_at: i64,
    views: [u8; 2],
    notified: [bool; 2],
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreditReminder {
    id: String,
    expires_at: i64,
    stage: usize,
}

impl CreditReminder {
    pub fn valid(&self, snapshot: Option<&Snapshot>, fresh: bool, now: i64) -> bool {
        fresh
            && self.expires_at > now
            && snapshot.is_some_and(|s| {
                s.credits
                    .iter()
                    .any(|c| c.id == self.id && c.expires_at == self.expires_at)
            })
    }
}

fn credit_message(seconds: i64) -> String {
    let time = if seconds < 3600 {
        "重置卡将在1小时内过期".into()
    } else {
        format!("{} 小时后重置卡过期", seconds.saturating_add(3599) / 3600)
    };
    time
}

fn credit_candidates(snapshot: &Snapshot, now: i64) -> impl Iterator<Item = CreditReminder> + '_ {
    snapshot.credits.iter().filter_map(move |credit| {
        let seconds = credit.expires_at.saturating_sub(now);
        (seconds > 0 && seconds <= 48 * 3600).then(|| CreditReminder {
            id: credit.id.clone(),
            expires_at: credit.expires_at,
            stage: usize::from(seconds <= 24 * 3600),
        })
    })
}

impl ReminderTracker {
    pub fn credit_hint(
        &self,
        snapshot: Option<&Snapshot>,
        fresh: bool,
        now: i64,
    ) -> Option<CreditReminder> {
        credit_candidates(snapshot.filter(|_| fresh)?, now)
            .filter(|credit| {
                self.credits
                    .iter()
                    .find(|state| state.id == credit.id && state.expires_at == credit.expires_at)
                    .is_none_or(|state| state.views[credit.stage] < [1, 2][credit.stage])
            })
            .min_by_key(|credit| credit.expires_at)
    }

    pub fn hints(
        &self,
        snapshot: Option<&Snapshot>,
        fresh: bool,
        now: i64,
        shown: Option<&CreditReminder>,
    ) -> Hint {
        let mut hint = Hint {
            level: "none",
            messages: Vec::new(),
        };
        let held = shown.filter(|credit| credit.valid(snapshot, fresh, now));
        if let Some(credit) = held
            .cloned()
            .or_else(|| self.credit_hint(snapshot, fresh, now))
        {
            if credit.expires_at.saturating_sub(now) <= 24 * 3600 {
                hint.level = "urgent";
            } else if hint.level == "none" {
                hint.level = "notice";
            }
            hint.messages
                .push(credit_message(credit.expires_at.saturating_sub(now)));
        }
        hint
    }

    fn credit_state(&mut self, credit: &CreditReminder) -> &mut CreditTracker {
        let index = self
            .credits
            .iter()
            .position(|state| state.id == credit.id && state.expires_at == credit.expires_at)
            .unwrap_or_else(|| {
                self.credits.push(CreditTracker {
                    id: credit.id.clone(),
                    expires_at: credit.expires_at,
                    ..Default::default()
                });
                self.credits.len() - 1
            });
        &mut self.credits[index]
    }

    pub fn viewed(&mut self, credit: &CreditReminder) -> bool {
        let views = &mut self.credit_state(credit).views[credit.stage];
        if *views >= [1, 2][credit.stage] {
            return false;
        }
        *views += 1;
        true
    }

    /// Only fresh successful observations while enabled can advance notification state.
    /// Reset timestamps identify windows; wall-clock expiry never implies recovery.
    pub fn observe(
        &mut self,
        snapshot: Option<&Snapshot>,
        fresh: bool,
        enabled: bool,
        now: i64,
    ) -> Option<String> {
        let snapshot = snapshot.filter(|_| fresh && enabled)?;
        self.credits.retain(|state| state.expires_at > now);
        let mut events = Vec::new();
        for credit in credit_candidates(snapshot, now) {
            let state = self.credit_state(&credit);
            if !state.notified[credit.stage] {
                state.notified[credit.stage] = true;
                events.push(credit_message(credit.expires_at.saturating_sub(now)));
            }
        }
        if events.is_empty() {
            None
        } else {
            Some(events.join("；"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{QuotaWindow, ResetCredit};

    #[test]
    fn quota_changes_never_create_dots_text_or_system_notifications() {
        let mut tracker = ReminderTracker::default();
        for value in [0.0, 9.0, 100.0] {
            let sample = snapshot(Some(value), Some(value), None);
            assert_eq!(tracker.hints(Some(&sample), true, 0, None).level, "none");
            assert!(tracker.observe(Some(&sample), true, true, 0).is_none());
        }
    }

    #[test]
    fn expiry_text_is_plain_without_faces() {
        assert_eq!(credit_message(8 * 3600), "8 小时后重置卡过期");
        assert_eq!(credit_message(60), "重置卡将在1小时内过期");
    }

    #[test]
    fn credit_views_are_limited_per_stage_and_survive_restart() {
        let mut tracker = ReminderTracker::default();
        let orange = snapshot(None, None, Some(48 * 3600));
        let shown = tracker.credit_hint(Some(&orange), true, 0).unwrap();
        assert!(tracker.viewed(&shown));
        assert!(tracker.credit_hint(Some(&orange), true, 0).is_none());
        assert!(
            tracker.hints(Some(&orange), true, 0, Some(&shown)).messages[0].contains("48 小时")
        );
        let mut tracker: ReminderTracker =
            serde_json::from_str(&serde_json::to_string(&tracker).unwrap()).unwrap();
        assert!(tracker.credit_hint(Some(&orange), true, 0).is_none());
        let red = tracker.credit_hint(Some(&orange), true, 24 * 3600).unwrap();
        assert!(tracker.viewed(&red));
        assert!(tracker
            .credit_hint(Some(&orange), true, 24 * 3600)
            .is_some());
        assert!(tracker.viewed(&red));
        assert!(!tracker.viewed(&red));
        assert!(tracker
            .credit_hint(Some(&orange), true, 24 * 3600)
            .is_none());
        let mut next = orange.clone();
        next.credits[0].id = "new".into();
        assert!(tracker.credit_hint(Some(&next), true, 24 * 3600).is_some());
        assert!(tracker.credit_hint(Some(&next), false, 24 * 3600).is_none());
        assert!(tracker.credit_hint(Some(&next), true, 48 * 3600).is_none());
    }

    #[test]
    fn windows_notifications_are_independent_of_views_and_skip_missed_stage() {
        let mut tracker = ReminderTracker::default();
        let credit = snapshot(Some(0.0), None, Some(48 * 3600));
        let shown = tracker.credit_hint(Some(&credit), true, 0).unwrap();
        tracker.viewed(&shown);
        assert!(tracker.observe(Some(&credit), true, false, 0).is_none());
        assert!(tracker.observe(Some(&credit), false, true, 0).is_none());
        assert!(tracker
            .observe(Some(&credit), true, true, 0)
            .unwrap()
            .contains("48 小时"));
        assert!(tracker.observe(Some(&credit), true, true, 0).is_none());
        assert!(tracker
            .observe(Some(&credit), true, true, 40 * 3600)
            .unwrap()
            .contains("8 小时"));
        let mut tracker: ReminderTracker =
            serde_json::from_str(&serde_json::to_string(&tracker).unwrap()).unwrap();
        assert!(tracker
            .observe(Some(&credit), true, true, 40 * 3600)
            .is_none());
        assert!(ReminderTracker::default()
            .observe(Some(&credit), true, true, 40 * 3600)
            .unwrap()
            .contains("8 小时"));
    }

    #[test]
    fn five_hour_usage_alone_never_warns_or_notifies() {
        let low = snapshot(Some(0.0), None, None);
        assert_eq!(hints(Some(&low), true, 0).level, "none");
        let mut tracker = ReminderTracker::default();
        assert!(tracker.observe(Some(&low), true, true, 0).is_none());
    }

    #[test]
    fn reset_credit_does_not_warn_before_48_hours() {
        assert_eq!(
            hints(Some(&snapshot(None, None, Some(48 * 3600 + 1))), true, 0).level,
            "none"
        );
    }

    fn snapshot(five: Option<f64>, week: Option<f64>, expiry: Option<i64>) -> Snapshot {
        let window = |remaining, minutes| QuotaWindow {
            remaining,
            minutes,
            resets_at: Some(100),
        };
        Snapshot {
            five_hour: five.map(|v| window(v, 300)),
            weekly: week.map(|v| window(v, 10080)),
            credits: expiry
                .map(|expires_at| {
                    vec![ResetCredit {
                        id: "a".into(),
                        expires_at,
                    }]
                })
                .unwrap_or_default(),
            available_credits: expiry.map(|_| 1),
            fetched_at: 0,
        }
    }

    #[test]
    fn exact_expiry_boundaries_and_expired_filter() {
        for (seconds, level) in [
            (48 * 3600 + 1, "none"),
            (48 * 3600, "notice"),
            (24 * 3600, "urgent"),
            (1, "urgent"),
            (0, "none"),
            (-1, "none"),
        ] {
            assert_eq!(
                hints(Some(&snapshot(None, None, Some(seconds))), true, 0).level,
                level
            );
        }
    }
    #[test]
    fn quota_first_and_credit_advice() {
        let hint = hints(Some(&snapshot(Some(20.0), Some(10.0), Some(3600))), true, 0);
        assert_eq!(hint.level, "urgent");
        assert_eq!(hint.messages.len(), 1);
        assert!(hint.messages.iter().any(|v| v.contains("重置卡过期")));
        assert_eq!(
            hints(Some(&snapshot(Some(20.01), Some(10.01), None)), true, 0).level,
            "none"
        );
    }
    #[test]
    fn stale_unknown_and_countdown() {
        assert_eq!(
            hints(Some(&snapshot(Some(0.0), Some(0.0), Some(1))), false, 0).level,
            "none"
        );
        assert_eq!(hints(None, true, 0).level, "none");
        assert_eq!(countdown(None, 0), "");
        assert_eq!(countdown(Some(3660), 0), "约1小时1分钟后重置");
        assert_eq!(countdown(Some(0), 0), "等待额度更新");
        assert_eq!(updated_at(None), "");
    }
}
