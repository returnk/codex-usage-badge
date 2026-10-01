use crate::domain::{QuotaWindow, Snapshot};
use chrono::{Local, TimeZone};
use serde::{Deserialize, Serialize};

pub struct Hint {
    pub level: &'static str,
    pub messages: Vec<String>,
}

pub fn hints(snapshot: Option<&Snapshot>, fresh: bool, now: i64) -> Hint {
    let mut hint = Hint {
        level: "none",
        messages: Vec::new(),
    };
    let Some(snapshot) = snapshot.filter(|_| fresh) else {
        return hint;
    };
    for (window, threshold, label) in [
        (&snapshot.weekly, 10.0, "周额度"),
        (&snapshot.five_hour, 20.0, "五小时额度"),
    ] {
        if window
            .as_ref()
            .is_some_and(|v| valid_remaining(v.remaining) && v.remaining <= threshold)
        {
            if hint.level != "urgent" {
                hint.level = if window.as_ref().is_some_and(|v| v.remaining <= 0.0) {
                    "urgent"
                } else {
                    "notice"
                };
            }
            hint.messages
                .push(format!("{label}剩余不超过{threshold:.0}%"));
        }
    }
    let low = !hint.messages.is_empty();
    let earliest = snapshot
        .credits
        .iter()
        .filter(|v| v.expires_at > now)
        .map(|v| v.expires_at)
        .min();
    if let Some(expiry) = earliest {
        let seconds = expiry.saturating_sub(now);
        if seconds <= 72 * 3600 {
            hint.level = if hint.level == "urgent" || seconds <= 24 * 3600 {
                "urgent"
            } else {
                "notice"
            };
            hint.messages.push(if seconds <= 24 * 3600 {
                "最早的重置机会将在24小时内到期".into()
            } else {
                "最早的重置机会将在72小时内到期".into()
            });
        }
    }
    if low
        && (earliest.is_some()
            || crate::domain::credit_count(Some(snapshot), now).is_some_and(|n| n > 0))
    {
        hint.messages.push("有可用重置机会，可考虑使用".into());
    }
    hint
}

fn valid_remaining(value: f64) -> bool {
    value.is_finite() && (0.0..=100.0).contains(&value)
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
    five_hour: WindowTracker,
    weekly: WindowTracker,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
struct WindowTracker {
    low_active: bool,
    notified: bool,
    notified_reset: Option<i64>,
}

impl WindowTracker {
    fn observe(
        &mut self,
        window: Option<&QuotaWindow>,
        low: f64,
        recovery: f64,
        label: &str,
    ) -> Option<String> {
        let window = window.filter(|v| valid_remaining(v.remaining))?;
        if window.remaining <= low {
            if !self.notified || self.notified_reset != window.resets_at {
                self.notified = true;
                self.notified_reset = window.resets_at;
                self.low_active = true;
                return Some(format!("{label}剩余不超过{low:.0}%"));
            }
        } else if self.low_active && window.remaining >= recovery {
            self.low_active = false;
            return Some(format!("{label}已恢复，当前剩余{:.0}%", window.remaining));
        }
        None
    }
}

impl ReminderTracker {
    /// Only fresh successful observations while enabled can advance notification state.
    /// Reset timestamps identify windows; wall-clock expiry never implies recovery.
    pub fn observe(
        &mut self,
        snapshot: Option<&Snapshot>,
        fresh: bool,
        enabled: bool,
    ) -> Option<String> {
        let snapshot = snapshot.filter(|_| fresh && enabled)?;
        let events = [
            self.weekly
                .observe(snapshot.weekly.as_ref(), 10.0, 15.0, "周额度"),
            self.five_hour
                .observe(snapshot.five_hour.as_ref(), 20.0, 25.0, "五小时额度"),
        ];
        let events: Vec<_> = events.into_iter().flatten().collect();
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
    use crate::domain::ResetCredit;

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
            (72 * 3600 + 1, "none"),
            (72 * 3600, "notice"),
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
        assert!(hint.messages[0].contains("周额度"));
        assert!(hint.messages.iter().any(|v| v.contains("可考虑使用")));
        assert_eq!(
            hints(Some(&snapshot(Some(20.01), Some(10.01), None)), true, 0).level,
            "none"
        );
    }
    #[test]
    fn exhausted_quota_stays_red_when_other_hints_are_only_orange() {
        for (five, week) in [(Some(0.0), Some(9.0)), (Some(19.0), Some(0.0))] {
            assert_eq!(
                hints(Some(&snapshot(five, week, Some(48 * 3600))), true, 0).level,
                "urgent"
            );
        }
        assert_eq!(
            hints(Some(&snapshot(Some(19.0), Some(9.0), None)), true, 0).level,
            "notice"
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
    #[test]
    fn tracker_hysteresis_unknown_restart_and_merge() {
        let low = snapshot(Some(20.0), Some(10.0), None);
        let mut tracker = ReminderTracker::default();
        assert!(tracker.observe(Some(&low), true, false).is_none());
        let first = tracker.observe(Some(&low), true, true).unwrap();
        assert!(first.contains("五小时") && first.contains("周额度"));
        let mut tracker: ReminderTracker =
            serde_json::from_str(&serde_json::to_string(&tracker).unwrap()).unwrap();
        assert!(tracker.observe(Some(&low), true, true).is_none());
        assert!(tracker.observe(None, true, true).is_none());
        assert!(tracker
            .observe(Some(&snapshot(Some(100.0), Some(100.0), None)), false, true)
            .is_none());
        assert!(tracker
            .observe(Some(&snapshot(Some(24.9), Some(14.9), None)), true, true)
            .is_none());
        assert!(tracker
            .observe(Some(&snapshot(Some(25.0), Some(15.0), None)), true, true)
            .unwrap()
            .contains("恢复"));
        assert!(tracker
            .observe(Some(&snapshot(Some(100.0), Some(100.0), None)), true, true)
            .is_none());
        assert!(
            tracker.observe(Some(&low), true, true).is_none(),
            "same reset identity must not repeat low notification"
        );
        let mut next = low.clone();
        next.five_hour.as_mut().unwrap().resets_at = Some(200);
        assert!(tracker.observe(Some(&next), true, true).is_some());
    }
    #[test]
    fn passing_reset_epoch_does_not_recover() {
        let mut tracker = ReminderTracker::default();
        let low = snapshot(Some(0.0), None, None);
        assert!(tracker.observe(Some(&low), true, true).is_some());
        assert!(tracker.observe(Some(&low), true, true).is_none());
        assert!(tracker
            .observe(Some(&snapshot(None, None, None)), true, true)
            .is_none());
    }
}
