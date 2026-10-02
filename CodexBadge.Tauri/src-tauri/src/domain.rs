use chrono::{Local, TimeZone};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Theme {
    #[serde(
        alias = "codex-blue",
        alias = "frost-light",
        alias = "graphite-dark",
        alias = "privacy",
        alias = "light",
        alias = "dark"
    )]
    System,
    #[serde(alias = "transparent")]
    Glass,
}

#[derive(Clone, Debug, PartialEq)]
pub struct QuotaWindow {
    pub remaining: f64,
    pub minutes: u32,
    pub resets_at: Option<i64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResetCredit {
    pub id: String,
    pub expires_at: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub five_hour: Option<QuotaWindow>,
    pub weekly: Option<QuotaWindow>,
    pub credits: Vec<ResetCredit>,
    pub available_credits: Option<u64>,
    pub fetched_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub theme: Theme,
    pub offset_x: f64,
    pub offset_y: f64,
    pub start_with_windows: bool,
    #[serde(default)]
    pub always_on_top: bool,
    #[serde(default)]
    pub notifications_enabled: bool,
    #[serde(default)]
    pub reminder_state: crate::reminders::ReminderTracker,
    #[serde(default = "crate::celebration::CelebrationState::existing_install")]
    pub celebration_state: crate::celebration::CelebrationState,
    #[serde(default, deserialize_with = "read_global_position")]
    pub global_position: Option<GlobalPosition>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GlobalPosition {
    pub device: String,
    pub x_dip: f64,
    pub y_dip: f64,
}

fn read_global_position<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<GlobalPosition>, D::Error> {
    let value = Value::deserialize(deserializer)?;
    Ok(serde_json::from_value::<GlobalPosition>(value)
        .ok()
        .filter(|p| {
            !p.device.is_empty()
                && p.device.len() <= 128
                && p.x_dip.is_finite()
                && p.y_dip.is_finite()
                && p.x_dip.abs() < 1_000_000.0
                && p.y_dip.abs() < 1_000_000.0
        }))
}

pub fn global_frame(
    saved: Option<&GlobalPosition>,
    area: (i32, i32, i32, i32),
    dpi: u32,
) -> (i32, i32, i32, i32) {
    let w = dip_to_px(72.0, dpi);
    let h = dip_to_px(34.0, dpi);
    let point = saved
        .map(|p| {
            (
                area.0 + dip_to_px(p.x_dip, dpi),
                area.1 + dip_to_px(p.y_dip, dpi),
            )
        })
        .unwrap_or((
            area.2 - w - dip_to_px(16.0, dpi),
            area.3 - h - dip_to_px(16.0, dpi),
        ));
    let (x, y) = clamp_to_work_area(point, (w, h), area);
    (x, y, w, h)
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: Theme::Glass,
            offset_x: 0.0,
            offset_y: 0.0,
            start_with_windows: false,
            always_on_top: false,
            notifications_enabled: false,
            reminder_state: Default::default(),
            celebration_state: Default::default(),
            global_position: None,
        }
    }
}

pub fn parse_quota(result: &Value, now: i64) -> Snapshot {
    let limits = quota_limits(result);
    let mut five_hour = None;
    let mut weekly = None;
    for key in ["primary", "secondary"] {
        let Some(value) = limits.and_then(|v| v.get(key)) else {
            continue;
        };
        let (Some(used), Some(minutes)) = (
            value.get("usedPercent").and_then(Value::as_f64),
            value.get("windowDurationMins").and_then(Value::as_u64),
        ) else {
            continue;
        };
        if !used.is_finite() || !(0.0..=100.0).contains(&used) {
            continue;
        }
        let window = QuotaWindow {
            remaining: (100.0 - used).clamp(0.0, 100.0),
            minutes: minutes as u32,
            resets_at: value.get("resetsAt").and_then(Value::as_i64),
        };
        match minutes {
            300 if five_hour.is_none() => five_hour = Some(window),
            10_080 if weekly.is_none() => weekly = Some(window),
            _ => {}
        }
    }
    let mut credits: Vec<ResetCredit> = result
        .get("rateLimitResetCredits")
        .and_then(|v| v.get("credits"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|v| {
            if v.get("status")?.as_str()? != "available" {
                return None;
            }
            let expires_at = v.get("expiresAt")?.as_i64()?;
            if expires_at <= now {
                return None;
            }
            Some(ResetCredit {
                id: v.get("id")?.as_str()?.to_string(),
                expires_at,
            })
        })
        .collect();
    credits.sort_by_key(|credit| credit.expires_at);
    Snapshot {
        five_hour,
        weekly,
        credits,
        available_credits: result.get("rateLimitResetCredits").and_then(|value| {
            value
                .get("availableCount")
                .and_then(Value::as_u64)
                .or_else(|| {
                    value
                        .get("credits")
                        .and_then(Value::as_array)
                        .map(|values| {
                            values
                                .iter()
                                .filter(|v| {
                                    v.get("status").and_then(Value::as_str) == Some("available")
                                        && v.get("expiresAt")
                                            .and_then(Value::as_i64)
                                            .is_some_and(|expiry| expiry > now)
                                })
                                .count() as u64
                        })
                })
        }),
        fetched_at: now,
    }
}

pub fn quota_limits(result: &Value) -> Option<&Value> {
    match result.get("rateLimitsByLimitId").and_then(Value::as_object) {
        Some(all) => all.get("codex").filter(|v| v.is_object()),
        None => result.get("rateLimits").filter(|v| v.is_object()),
    }
}

pub fn quota_response_valid(result: &Value) -> bool {
    let Some(limits) = quota_limits(result) else {
        return false;
    };
    ["primary", "secondary"].iter().all(|key| {
        limits.get(key).filter(|v| !v.is_null()).is_none_or(|w| {
            w.get("usedPercent")
                .and_then(Value::as_f64)
                .is_some_and(|v| v.is_finite() && (0.0..=100.0).contains(&v))
                && w.get("windowDurationMins")
                    .and_then(Value::as_u64)
                    .is_some_and(|v| v > 0 && v <= u32::MAX as u64)
        })
    })
}

pub fn quota_mode(snapshot: Option<&Snapshot>) -> &'static str {
    match snapshot {
        Some(s) if s.five_hour.is_some() => "five-hour",
        Some(s) if s.weekly.is_some() => "weekly",
        _ => "none",
    }
}

pub fn plan_label(account: &Value) -> Option<String> {
    let plan = account.get("planType")?.as_str()?;
    matches!(
        plan,
        "free" | "go" | "plus" | "pro" | "team" | "business" | "enterprise" | "edu"
    )
    .then(|| plan.to_ascii_uppercase())
}

pub fn credit_count(snapshot: Option<&Snapshot>, now: i64) -> Option<u64> {
    let snapshot = snapshot?;
    snapshot.available_credits.map(|count| {
        count.saturating_sub(
            snapshot
                .credits
                .iter()
                .filter(|credit| credit.expires_at <= now)
                .count() as u64,
        )
    })
}

pub fn quota_freshness(fetched_at: Option<i64>, failed: bool, now: i64) -> &'static str {
    if fetched_at.is_none_or(|time| now - time > 1800) {
        "unavailable"
    } else if failed {
        "stale"
    } else {
        "fresh"
    }
}

pub fn retain_resources(present: bool, absent_since: Option<Instant>, now: Instant) -> bool {
    present || absent_since.is_some_and(|time| now.duration_since(time) < Duration::from_secs(2))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HostObservation {
    pub window: isize,
    pub visible: bool,
    pub iconic: bool,
    pub unknown: bool,
}

pub fn host_policy(
    host: HostObservation,
    global: bool,
    absent_since: Option<Instant>,
    now: Instant,
) -> (bool, bool) {
    let retain = host.unknown || retain_resources(host.window != 0, absent_since, now);
    (
        retain,
        retain && (global || (host.window != 0 && host.visible && !host.iconic)),
    )
}

pub fn progress_band(remaining: f64) -> &'static str {
    if remaining < 20.0 {
        "orange"
    } else if remaining < 40.0 {
        "yellow"
    } else {
        "green"
    }
}

pub fn cycle_theme(current: Theme, wheel_delta: i32) -> Theme {
    let themes = [Theme::Glass, Theme::System];
    if wheel_delta == 0 {
        return current;
    }
    let index = themes
        .iter()
        .position(|theme| *theme == current)
        .unwrap_or(0) as i32;
    themes[(index + wheel_delta.signum()).rem_euclid(2) as usize]
}

pub fn accept_theme_wheel(previous: Option<Instant>, now: Instant) -> bool {
    previous.is_none_or(|at| now.duration_since(at) >= Duration::from_millis(140))
}

pub fn popup_departure(
    left_at: Option<Instant>,
    cursor_inside: bool,
    now: Instant,
) -> (Option<Instant>, bool) {
    if cursor_inside {
        (None, false)
    } else {
        let left_at = left_at.or(Some(now));
        let close = left_at.is_some_and(|at| now.duration_since(at) >= Duration::from_millis(300));
        (left_at, close)
    }
}

pub fn menu_departure(
    entered: bool,
    shown_at: Instant,
    left_at: Option<Instant>,
    cursor_inside: bool,
    now: Instant,
) -> (bool, Option<Instant>, bool) {
    if cursor_inside {
        return (true, None, false);
    }
    if !entered {
        return (
            false,
            None,
            now.duration_since(shown_at) >= Duration::from_millis(1200),
        );
    }
    let (left_at, close) = popup_departure(left_at, false, now);
    (true, left_at, close)
}

pub fn capsule_text(remaining: Option<f64>) -> String {
    remaining
        .map(|percent| format!("{:.0}%", percent.round()))
        .unwrap_or_else(|| "--".into())
}

pub fn reset_position(settings: &mut Settings) {
    settings.offset_x = 0.0;
    settings.offset_y = 0.0;
}
pub fn badge_owner(codex_owner: isize, always_on_top: bool) -> isize {
    if always_on_top {
        0
    } else {
        codex_owner
    }
}
pub fn dip_to_px(dip: f64, dpi: u32) -> i32 {
    (dip * dpi.max(96) as f64 / 96.0).round() as i32
}
pub fn capsule_origin(anchor: (i32, i32), dpi: u32, offset: (f64, f64)) -> (i32, i32) {
    let width = dip_to_px(30.0, dpi);
    let inset = dip_to_px(2.0, dpi);
    (
        anchor.0 - width / 2 + inset + dip_to_px(offset.0, dpi),
        anchor.1 - width - dip_to_px(10.0, dpi) + inset + dip_to_px(offset.1, dpi),
    )
}
pub fn capsule_window_rect(x: i32, y: i32, dpi: u32) -> (i32, i32, i32, i32) {
    let inset = dip_to_px(2.0, dpi);
    (
        x - inset,
        y - inset,
        dip_to_px(30.0, dpi),
        dip_to_px(30.0, dpi),
    )
}
pub fn capsule_frame(
    settings: &Settings,
    anchor: Option<(i32, i32)>,
    dpi: u32,
    last: Option<(i32, i32, i32, i32)>,
) -> Option<(i32, i32, i32, i32)> {
    if settings.always_on_top {
        return last;
    }
    anchor
        .map(|point| {
            let (x, y) = capsule_origin(point, dpi, (settings.offset_x, settings.offset_y));
            capsule_window_rect(x, y, dpi)
        })
        .or_else(|| settings.always_on_top.then_some(last).flatten())
}
pub fn clamp_to_work_area(
    position: (i32, i32),
    size: (i32, i32),
    area: (i32, i32, i32, i32),
) -> (i32, i32) {
    (
        position.0.clamp(area.0, (area.2 - size.0).max(area.0)),
        position.1.clamp(area.1, (area.3 - size.1).max(area.1)),
    )
}

pub type Rect = (i32, i32, i32, i32);
pub fn menu_pointer_inside(point: (i32, i32), menu: Rect, capsule: Option<Rect>) -> bool {
    let contains = |rect: Rect| {
        point.0 >= rect.0
            && point.0 < rect.0 + rect.2
            && point.1 >= rect.1
            && point.1 < rect.1 + rect.3
    };
    if contains(menu) {
        return true;
    }
    let Some(capsule) = capsule else {
        return false;
    };
    if contains(capsule) {
        return true;
    }
    // Only bridge the shared edge span, not the large bounding box around both
    // windows. The normal 6-DIP gap is smaller than the capsule at every DPI.
    let x0 = capsule.0.max(menu.0);
    let x1 = (capsule.0 + capsule.2).min(menu.0 + menu.2);
    let y0 = (capsule.1 + capsule.3).min(menu.1 + menu.3);
    let y1 = capsule.1.max(menu.1);
    let vertical = y1 >= y0 && y1 - y0 <= capsule.3 && contains((x0, y0, x1 - x0, y1 - y0));
    let y0 = capsule.1.max(menu.1);
    let y1 = (capsule.1 + capsule.3).min(menu.1 + menu.3);
    let x0 = (capsule.0 + capsule.2).min(menu.0 + menu.2);
    let x1 = capsule.0.max(menu.0);
    vertical || (x1 >= x0 && x1 - x0 <= capsule.3 && contains((x0, y0, x1 - x0, y1 - y0)))
}

pub fn rects_intersect(a: Rect, b: Rect) -> bool {
    a.0 < b.0 + b.2 && b.0 < a.0 + a.2 && a.1 < b.1 + b.3 && b.1 < a.1 + a.3
}

pub fn place_vertical_popup(
    anchor: Rect,
    size: (i32, i32),
    area: Rect,
    avoid: &[Rect],
    gap: i32,
) -> Option<Rect> {
    let width = size.0.min(area.2 - area.0);
    if width < 80 {
        return None;
    }
    let x = (anchor.0 + anchor.2 - width).clamp(area.0, area.2 - width);
    let above = anchor.1 - gap - area.1;
    let below = area.3 - anchor.1 - anchor.3 - gap;
    let candidate = |top: bool, height: i32| {
        let y = if top {
            anchor.1 - gap - height
        } else {
            anchor.1 + anchor.3 + gap
        };
        let rect = (x, y, width, height);
        (height >= 32
            && y >= area.1
            && y + height <= area.3
            && avoid.iter().all(|other| !rects_intersect(rect, *other)))
        .then_some(rect)
    };
    if above >= size.1 {
        if let Some(rect) = candidate(true, size.1) {
            return Some(rect);
        }
    }
    if below >= size.1 {
        if let Some(rect) = candidate(false, size.1) {
            return Some(rect);
        }
    }
    let top = above >= below;
    candidate(top, size.1.min(if top { above } else { below }))
        .or_else(|| candidate(!top, size.1.min(if top { below } else { above })))
}

#[cfg(test)]
mod vertical_popup_tests {
    use super::*;
    #[test]
    fn credit_popup_prefers_above_then_below_and_never_the_side() {
        let anchor = (200, 300, 270, 148);
        assert_eq!(
            place_vertical_popup(anchor, (205, 36), (0, 0, 1000, 800), &[anchor], 6),
            Some((265, 258, 205, 36))
        );
        let top = (200, 10, 270, 148);
        assert_eq!(
            place_vertical_popup(top, (205, 36), (0, 0, 1000, 800), &[top], 6),
            Some((265, 164, 205, 36))
        );
        let edge = (900, 300, 100, 148);
        assert_eq!(
            place_vertical_popup(edge, (205, 36), (0, 0, 1000, 800), &[edge], 6),
            Some((795, 258, 205, 36))
        );
        assert!(
            place_vertical_popup((0, 0, 270, 100), (205, 36), (0, 0, 300, 120), &[], 6).is_none()
        );
    }
}

pub fn place_popup(
    anchor: Rect,
    size: (i32, i32),
    area: Rect,
    avoid: &[Rect],
    gap: i32,
) -> Option<Rect> {
    place_popup_at(anchor, size, area, avoid, gap, None)
}

pub fn place_detail_in_sidebar(
    anchor: Rect,
    size: (i32, i32),
    area: Rect,
    avoid: &[Rect],
    gap: i32,
    sidebar: Option<Rect>,
    navigation: Option<Rect>,
) -> Option<Rect> {
    if let (Some(sidebar), Some(nav)) = (sidebar, navigation) {
        let left = nav.0 + nav.2;
        let right = sidebar.0 + sidebar.2;
        if right - left >= size.0 + 2 * gap {
            let rect = (left + gap, anchor.1 + anchor.3 - size.1, size.0, size.1);
            if rect.0 >= area.0
                && rect.1 >= area.1
                && rect.0 + rect.2 <= area.2
                && rect.1 + rect.3 <= area.3
                && avoid.iter().all(|a| !rects_intersect(rect, *a))
            {
                return Some(rect);
            }
        }
    }
    place_popup(anchor, size, area, avoid, gap)
}

fn place_popup_at(
    anchor: Rect,
    size: (i32, i32),
    area: Rect,
    avoid: &[Rect],
    gap: i32,
    preferred_x: Option<i32>,
) -> Option<Rect> {
    let (x, y, w, h) = anchor;
    let right_x = avoid
        .iter()
        .filter(|rect| rect.1 < y + h && rect.1 + rect.3 > y + h - size.1)
        .fold(x + w + gap, |edge, rect| edge.max(rect.0 + rect.2 + gap));
    let fits = |rect: Rect| {
        rect.2 >= 80
            && rect.3 >= 32
            && rect.0 >= area.0
            && rect.1 >= area.1
            && rect.0 + rect.2 <= area.2
            && rect.1 + rect.3 <= area.3
            && avoid.iter().all(|a| !rects_intersect(rect, *a))
    };
    for point in [
        (right_x, y + h - size.1),
        (preferred_x.unwrap_or(x + w - size.0), y - size.1 - gap),
        (preferred_x.unwrap_or(x + w - size.0), y + h + gap),
        (x - size.0 - gap, y),
        (right_x, y),
    ] {
        let (px, py) = clamp_to_work_area(point, size, area);
        let candidate = (px, py, size.0, size.1);
        if fits(candidate) {
            return Some(candidate);
        }
    }
    // Constrained screens: prefer the largest usable side, then scroll the contents.
    let strips = [
        (area.0, area.1, area.2, y - gap),
        (area.0, y + h + gap, area.2, area.3),
        (area.0, area.1, x - gap, area.3),
        (right_x, area.1, area.2, area.3),
    ];
    strips
        .into_iter()
        .filter_map(|strip| {
            let sw = size.0.min(strip.2 - strip.0);
            let sh = size.1.min(strip.3 - strip.1);
            if sw <= 0 || sh <= 0 {
                return None;
            }
            let (px, py) = clamp_to_work_area(
                (preferred_x.unwrap_or(x + w - sw), y - sh - gap),
                (sw, sh),
                strip,
            );
            let rect = (px, py, sw, sh);
            fits(rect).then_some(rect)
        })
        .max_by_key(|rect| rect.2 as i64 * rect.3 as i64)
}

#[cfg(test)]
fn place_detail_popup(
    capsule: Rect,
    size: (i32, i32),
    area: Rect,
    gap: i32,
    dpi: u32,
) -> Option<Rect> {
    // Keep the existing right anchor with a small, DPI-scaled visual inset.
    // Sidebar resizing needs no layout mode or guessed default width.
    let preferred = capsule.0 + capsule.2 - size.0 - dip_to_px(2.0, dpi);
    place_popup_at(capsule, size, area, &[capsule], gap, Some(preferred))
}
pub fn native_capsule_curve(height: i32) -> i32 {
    (height - 4).max(1)
}
pub fn px_to_dip(px: i32, dpi: u32) -> f64 {
    px as f64 * 96.0 / dpi.max(96) as f64
}

pub fn stable_anchor(
    previous: Option<(i32, i32)>,
    samples: u8,
    next: Option<(i32, i32)>,
) -> (Option<(i32, i32)>, u8, Option<(i32, i32)>) {
    match next {
        Some(point) if previous == Some(point) => (
            Some(point),
            samples.saturating_add(1),
            (samples >= 1).then_some(point),
        ),
        Some(point) => (Some(point), 1, None),
        None => (None, 0, None),
    }
}

pub fn follow_anchor(
    current: Option<(i32, i32)>,
    misses: u8,
    next: Option<(i32, i32)>,
) -> (Option<(i32, i32)>, u8) {
    match next {
        Some(point) => (Some(point), 0),
        None if misses < 7 => (current, misses + 1),
        None => (None, misses.saturating_add(1)),
    }
}

pub fn credit_popup_height(rows: usize) -> f64 {
    // CSS: outer inset 4 + borders 2 + padding 16 + first row 14;
    // subsequent rows add their line height 14 and the flex gap 4.
    36.0 + rows.saturating_sub(1) as f64 * 18.0
}

pub fn anchor_scan_interval(missing_scans: u32, recovering: bool) -> Duration {
    if recovering {
        return Duration::from_millis(250);
    }
    Duration::from_secs([2, 4, 8, 15, 30][missing_scans.min(4) as usize])
}

pub fn anchor_scan_due(
    now: Instant,
    last: Instant,
    until: Instant,
    was_recovering: bool,
    misses: u32,
) -> bool {
    (was_recovering && now >= until)
        || now.saturating_duration_since(last) >= anchor_scan_interval(misses, now < until)
}

pub fn follow_anchor_during_recovery(
    current: Option<(i32, i32)>,
    misses: u8,
    next: Option<(i32, i32)>,
    now: Instant,
    until: Instant,
) -> (Option<(i32, i32)>, u8) {
    if next.is_some() {
        return follow_anchor(current, misses, next);
    }
    (
        (now < until).then_some(current).flatten(),
        misses.saturating_add(1),
    )
}

pub fn drag_offset(old: (f64, f64), start: (i32, i32), end: (i32, i32), dpi: u32) -> (f64, f64) {
    (
        old.0 + px_to_dip(end.0 - start.0, dpi),
        old.1 + px_to_dip(end.1 - start.1, dpi),
    )
}

pub fn drag_window_offset(
    old: (f64, f64),
    start: (i32, i32),
    end: (i32, i32),
    start_dpi: u32,
    end_dpi: u32,
    anchor_dpi: u32,
) -> (f64, f64) {
    let first = dip_to_px(2.0, start_dpi);
    let last = dip_to_px(2.0, end_dpi);
    drag_offset(
        old,
        (start.0 + first, start.1 + first),
        (end.0 + last, end.1 + last),
        anchor_dpi,
    )
}

pub fn format_five_hour_reset(epoch: Option<i64>) -> String {
    epoch
        .and_then(|value| Local.timestamp_opt(value, 0).single())
        .map(|date| format!("将于 {} 重置", date.format("%H:%M")))
        .unwrap_or_else(|| "重置时间未知".into())
}

pub fn format_weekly_reset(epoch: Option<i64>) -> String {
    epoch
        .and_then(|value| Local.timestamp_opt(value, 0).single())
        .map(|date| format!("{} {} 重置", date.format("%-m/%-d"), date.format("%H:%M")))
        .unwrap_or_else(|| "重置时间未知".into())
}

pub fn reset_display(epoch: Option<i64>, weekly: bool, now: i64) -> String {
    let Some(at) = epoch else {
        return "重置时间未知".into();
    };
    if at <= now {
        return "已到重置时间，等待额度更新".into();
    }
    let Some(date) = Local.timestamp_opt(at, 0).single() else {
        return "重置时间未知".into();
    };
    let today = Local.timestamp_opt(now, 0).single().map(|v| v.date_naive());
    if weekly || today != Some(date.date_naive()) {
        format_weekly_reset(Some(at))
    } else {
        format_five_hour_reset(Some(at))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn adapted_detail_and_credit_fit_screen_edges_at_supported_scales() {
        for dpi in [96, 120, 144] {
            let p = |v| dip_to_px(v, dpi);
            let area = (0, 0, p(1200.0), p(800.0));
            for height in [131.0, 170.0, 220.0] {
                for (x, y) in [(0, 0), (1100, 0), (0, 700), (1100, 700)] {
                    let capsule = (p(x as f64), p(y as f64), p(30.0), p(30.0));
                    let detail = place_detail_in_sidebar(
                        capsule,
                        (p(270.0), p(height)),
                        area,
                        &[capsule],
                        p(9.0),
                        None,
                        None,
                    )
                    .unwrap();
                    assert!(!rects_intersect(capsule, detail));
                    assert!(
                        detail.0 >= area.0
                            && detail.1 >= area.1
                            && detail.0 + detail.2 <= area.2
                            && detail.1 + detail.3 <= area.3
                    );
                    if let Some(credit) = place_vertical_popup(
                        detail,
                        (p(205.0), p(38.0)),
                        area,
                        &[detail, capsule],
                        p(6.0),
                    ) {
                        assert!(!rects_intersect(detail, credit));
                        assert!(!rects_intersect(capsule, credit));
                        assert!(
                            credit.0 >= area.0
                                && credit.1 >= area.1
                                && credit.0 + credit.2 <= area.2
                                && credit.1 + credit.3 <= area.3
                        );
                    }
                }
            }
        }
    }
    #[test]
    fn malformed_present_window_is_a_read_failure_and_past_reset_waits_for_update() {
        for source in [
            Value::Null,
            serde_json::json!("broken"),
            serde_json::json!([]),
        ] {
            assert!(!quota_response_valid(
                &serde_json::json!({"rateLimitsByLimitId":{"codex":source}})
            ));
        }
        let value = serde_json::json!({"rateLimits":{"primary":{"windowDurationMins":300},"secondary":{"usedPercent":31,"windowDurationMins":10080}}});
        assert!(!quota_response_valid(&value));
        assert_eq!(
            reset_display(Some(99), false, 100),
            "已到重置时间，等待额度更新"
        );
        assert_eq!(reset_display(None, true, 100), "重置时间未知");
        let weekly = serde_json::json!({"rateLimits":{"primary":null,"secondary":{"usedPercent":31,"windowDurationMins":10080}}});
        assert!(quota_response_valid(&weekly));
        assert_eq!(quota_mode(Some(&parse_quota(&weekly, 100))), "weekly");
    }
    #[test]
    fn invalid_percentage_and_unrelated_provider_do_not_manufacture_quota() {
        for used in [-1.0, 101.0] {
            let value = serde_json::json!({"rateLimits":{"primary":{"usedPercent":used,"windowDurationMins":300}}});
            assert!(parse_quota(&value, 100).five_hour.is_none());
        }
        let value = serde_json::json!({"rateLimitsByLimitId":{"other":{"primary":{"usedPercent":10,"windowDurationMins":300}}}});
        assert!(parse_quota(&value, 100).five_hour.is_none());
    }
    #[test]
    fn avatar_capsule_is_centered_and_fits_the_navigation_column() {
        for dpi in [96, 120, 144, 192] {
            let anchor = (dip_to_px(26.0, dpi), dip_to_px(850.0, dpi));
            let rect = capsule_frame(&Settings::default(), Some(anchor), dpi, None).unwrap();
            assert_eq!(rect.2, dip_to_px(30.0, dpi));
            assert_eq!(rect.3, rect.2);
            assert!((rect.0 + rect.2 / 2 - anchor.0).abs() <= 1);
            assert!(rect.0 >= dip_to_px(2.0, dpi));
            assert!(rect.0 + rect.2 <= dip_to_px(50.0, dpi));
            assert_eq!(rect.1 + rect.3, anchor.1 - dip_to_px(10.0, dpi));
        }
    }
    #[test]
    fn wide_navigation_and_left_aligned_avatar_leave_popup_space_on_the_right() {
        let nav = (0, 40, 100, 860);
        let capsule = (27, 810, 46, 30);
        for size in [(270, 148), (190, 148), (205, 72)] {
            let popup = place_popup(capsule, size, (8, 8, 1000, 900), &[capsule, nav], 6).unwrap();
            assert!(popup.0 >= 106);
            assert!(!rects_intersect(popup, nav));
            assert!(!rects_intersect(popup, capsule));
        }
    }
    #[test]
    fn detail_keeps_its_default_left_inset_when_the_sidebar_widens() {
        for dpi in [96, 120, 144, 168, 192] {
            let p = |v| dip_to_px(v, dpi);
            for body in [288.0, 340.0, 420.0] {
                let nav = (p(60.0), p(40.0), p(52.0), p(800.0));
                let sidebar = (p(60.0), p(40.0), p(52.0 + body), p(800.0));
                let capsule = (p(68.0), p(760.0), p(30.0), p(30.0));
                let rect = place_detail_in_sidebar(
                    capsule,
                    (p(270.0), p(148.0)),
                    (0, 0, p(1200.0), p(900.0)),
                    &[capsule, nav],
                    p(9.0),
                    Some(sidebar),
                    Some(nav),
                )
                .unwrap();
                let left = rect.0 - (nav.0 + nav.2);
                let right = sidebar.0 + sidebar.2 - (rect.0 + rect.2);
                assert_eq!(left, p(9.0), "dpi={dpi} body={body}");
                if body == 288.0 {
                    assert!((left - right).abs() <= 1);
                }
            }
        }
    }
    #[test]
    fn narrow_or_unknown_sidebar_retains_the_existing_safe_placement() {
        let nav = (0, 0, 52, 800);
        let capsule = (8, 700, 30, 30);
        let area = (0, 0, 1000, 900);
        let expected = place_popup(capsule, (270, 148), area, &[nav, capsule], 6);
        for sidebar in [None, Some((0, 0, 250, 800))] {
            assert_eq!(
                place_detail_in_sidebar(
                    capsule,
                    (270, 148),
                    area,
                    &[nav, capsule],
                    6,
                    sidebar,
                    Some(nav)
                ),
                expected
            );
        }
    }
    #[test]
    fn detail_prefers_right_up_and_avoids_the_navigation_column_at_every_dpi() {
        for dpi in [96, 120, 144, 192] {
            let p = |v| dip_to_px(v, dpi);
            let capsule = (p(3.0), p(810.0), p(46.0), p(30.0));
            let nav = (0, p(40.0), p(52.0), p(860.0));
            let area = (p(8.0), p(8.0), p(1000.0), p(892.0));
            let detail =
                place_popup(capsule, (p(270.0), p(148.0)), area, &[capsule, nav], p(6.0)).unwrap();
            assert!(detail.0 >= nav.0 + nav.2);
            assert!(detail.1 < capsule.1);
            assert!(!rects_intersect(detail, nav));
            assert!(!rects_intersect(detail, capsule));
            let credit = place_popup(
                detail,
                (p(205.0), p(72.0)),
                area,
                &[capsule, nav, detail],
                p(6.0),
            )
            .unwrap();
            assert!(
                !rects_intersect(credit, nav)
                    && !rects_intersect(credit, detail)
                    && !rects_intersect(credit, capsule)
            );
        }
    }
    #[test]
    fn constrained_detail_shrinks_without_covering_capsule_and_refuses_no_space() {
        let capsule = (140, 110, 71, 30);
        let detail = place_detail_popup(capsule, (270, 148), (0, 0, 220, 160), 6, 96);
        assert_eq!(detail, Some((0, 0, 220, 104)));
        assert!(!rects_intersect(detail.unwrap(), capsule));
        assert_eq!(
            place_detail_popup((0, 0, 71, 30), (270, 148), (0, 0, 71, 30), 6, 96),
            None
        );
    }
    #[test]
    fn edge_popups_stay_inside_work_area_and_never_cover_capsule() {
        for dpi in [96, 120, 144, 168] {
            let area = (-1920, 0, 0, 1080);
            let w = dip_to_px(71.0, dpi);
            let h = dip_to_px(30.0, dpi);
            for (x, y) in [
                (-1920, 0),
                (-w, 0),
                (-1920, 1080 - h),
                (-w, 1080 - h),
                (-960, 0),
                (-960, 1080 - h),
            ] {
                let capsule = (x, y, w, h);
                let detail = place_detail_popup(
                    capsule,
                    (dip_to_px(270.0, dpi), dip_to_px(148.0, dpi)),
                    area,
                    dip_to_px(6.0, dpi),
                    dpi,
                )
                .unwrap();
                assert!(!rects_intersect(detail, capsule));
                assert!(
                    detail.0 >= -1920
                        && detail.1 >= 0
                        && detail.0 + detail.2 <= 0
                        && detail.1 + detail.3 <= 1080
                );
                let credit = place_popup(
                    detail,
                    (dip_to_px(205.0, dpi), 2000),
                    area,
                    &[capsule, detail],
                    dip_to_px(6.0, dpi),
                )
                .unwrap();
                assert!(!rects_intersect(credit, capsule) && !rects_intersect(credit, detail));
                assert!(credit.1 + credit.3 <= 1080);
            }
        }
        assert_eq!(
            place_popup(
                (0, 0, 71, 30),
                (270, 148),
                (0, 0, 71, 30),
                &[(0, 0, 71, 30)],
                6
            ),
            None
        );
    }
    #[test]
    fn host_minimize_unknown_recreation_and_exit_have_distinct_actions() {
        let now = Instant::now();
        let iconic = HostObservation {
            window: 123,
            visible: true,
            iconic: true,
            unknown: false,
        };
        assert_eq!(host_policy(iconic, false, None, now), (true, false));
        assert_eq!(host_policy(iconic, true, None, now), (true, true));
        let missing = HostObservation::default();
        assert_eq!(
            host_policy(missing, true, Some(now), now + Duration::from_millis(1900)),
            (true, true)
        );
        assert_eq!(
            host_policy(missing, true, Some(now), now + Duration::from_millis(2100)),
            (false, false)
        );
        let unknown = HostObservation {
            unknown: true,
            ..missing
        };
        assert_eq!(
            host_policy(unknown, true, Some(now), now + Duration::from_secs(20)),
            (true, true)
        );
        assert_eq!(
            host_policy(unknown, false, Some(now), now + Duration::from_secs(20)),
            (true, false)
        );
        let restored = HostObservation {
            iconic: false,
            ..iconic
        };
        assert_eq!(host_policy(restored, false, None, now), (true, true));
        assert_eq!(host_policy(missing, true, None, now), (false, false));
    }
    #[test]
    fn credit_count_survives_missing_details_and_unknown_is_not_zero() {
        let snapshot = parse_quota(
            &serde_json::json!({"rateLimitResetCredits":{"availableCount":2,"credits":null}}),
            1000,
        );
        assert_eq!(credit_count(Some(&snapshot), 1000), Some(2));
        assert_eq!(
            credit_count(Some(&parse_quota(&serde_json::json!({}), 1000)), 1000),
            None
        );
        assert_eq!(credit_count(None, 1000), None);
    }

    #[test]
    fn known_credit_expiry_reduces_count_after_fetch() {
        let snapshot = parse_quota(
            &serde_json::json!({"rateLimitResetCredits":{"availableCount":2,"credits":[
                {"id":"first","status":"available","expiresAt":1100},
                {"id":"second","status":"available","expiresAt":1200}
            ]}}),
            1000,
        );
        assert_eq!(credit_count(Some(&snapshot), 1100), Some(1));
        assert_eq!(credit_count(Some(&snapshot), 1200), Some(0));
    }

    #[test]
    fn quota_freshness_marks_failures_and_expires_at_thirty_minutes() {
        assert_eq!(quota_freshness(None, false, 2000), "unavailable");
        assert_eq!(quota_freshness(Some(1000), false, 2000), "fresh");
        assert_eq!(quota_freshness(Some(1000), true, 2000), "stale");
        assert_eq!(quota_freshness(Some(1000), false, 2801), "unavailable");
    }

    #[test]
    fn lifecycle_retains_resources_during_short_window_recreation() {
        let now = Instant::now();
        assert!(retain_resources(true, None, now));
        assert!(retain_resources(
            false,
            Some(now),
            now + Duration::from_millis(1900)
        ));
        assert!(!retain_resources(
            false,
            Some(now),
            now + Duration::from_millis(2100)
        ));
        assert!(!retain_resources(false, None, now));
    }
    use serde_json::json;
    use std::time::{Duration, Instant};

    #[test]
    fn menu_allows_pointer_to_travel_from_tray_then_closes_after_exit() {
        let now = Instant::now();
        let (entered, left_at, close) = menu_departure(false, now, None, false, now);
        assert!(!entered);
        assert_eq!(left_at, None);
        assert!(!close);
        let (_, _, close) =
            menu_departure(false, now, None, false, now + Duration::from_millis(700));
        assert!(!close);
        let (entered, left_at, close) =
            menu_departure(false, now, None, true, now + Duration::from_millis(800));
        assert!(entered);
        assert_eq!(left_at, None);
        assert!(!close);
        let (_, left_at, close) =
            menu_departure(true, now, None, false, now + Duration::from_millis(850));
        assert!(!close);
        let (_, _, close) =
            menu_departure(true, now, left_at, false, now + Duration::from_millis(1151));
        assert!(close);
        let (_, _, close) = menu_departure(false, now, None, false, now + Duration::from_secs(2));
        assert!(close);
    }

    #[test]
    fn flyout_closes_from_cursor_geometry_even_if_hover_leave_is_lost() {
        let now = Instant::now();
        let (left_at, close) = popup_departure(None, false, now);
        assert!(!close);
        let (_, close) = popup_departure(left_at, false, now + Duration::from_millis(301));
        assert!(close);
        let (left_at, close) = popup_departure(left_at, true, now + Duration::from_millis(200));
        assert_eq!(left_at, None);
        assert!(!close);
    }

    #[test]
    fn rapid_wheel_events_change_theme_once_per_interval() {
        let now = Instant::now();
        assert!(accept_theme_wheel(None, now));
        assert!(!accept_theme_wheel(
            Some(now),
            now + Duration::from_millis(50)
        ));
        assert!(accept_theme_wheel(
            Some(now),
            now + Duration::from_millis(150)
        ));
    }

    #[test]
    fn used_percent_and_window_durations() {
        let snapshot = parse_quota(
            &json!({"rateLimits": {"primary": {"usedPercent": 17, "windowDurationMins": 10080, "resetsAt": 1790000000}, "secondary": {"usedPercent": 61, "windowDurationMins": 300, "resetsAt": 1790000123}}}),
            1789000000,
        );
        assert_eq!(snapshot.five_hour.unwrap().remaining, 39.0);
        assert_eq!(snapshot.weekly.unwrap().remaining, 83.0);
    }

    #[test]
    fn progress_thresholds() {
        assert_eq!(progress_band(0.0), "orange");
        assert_eq!(progress_band(19.99), "orange");
        assert_eq!(progress_band(20.0), "yellow");
        assert_eq!(progress_band(39.99), "yellow");
        assert_eq!(progress_band(40.0), "green");
    }

    #[test]
    fn credits_are_available_unexpired_and_sorted() {
        let snapshot = parse_quota(
            &json!({"rateLimitResetCredits": {"availableCount": 4, "credits": [
                {"id":"late","status":"available","expiresAt":2000},
                {"id":"expired","status":"available","expiresAt":999},
                {"id":"redeemed","status":"redeemed","expiresAt":1500},
                {"id":"early","status":"available","expiresAt":1500}
            ]}}),
            1000,
        );
        assert_eq!(
            snapshot
                .credits
                .iter()
                .map(|c| c.id.as_str())
                .collect::<Vec<_>>(),
            vec!["early", "late"]
        );
    }

    #[test]
    fn reset_time_has_local_date_and_minute() {
        assert!(format_five_hour_reset(Some(1_790_000_000)).starts_with("将于 "));
        assert!(format_five_hour_reset(Some(1_790_000_000)).contains(':'));
        let text = format_weekly_reset(Some(1_790_000_000));
        assert!(!text.contains('日'));
        assert!(text.contains('/') && text.contains(':'));
        assert_eq!(format_weekly_reset(None), "重置时间未知");
    }

    #[test]
    fn themes_cycle_glass_system_and_legacy_preferences_load() {
        assert_eq!(
            serde_json::to_value(cycle_theme(Theme::Glass, 120)).unwrap(),
            serde_json::json!("system")
        );
        let system: Theme = serde_json::from_str("\"system\"").unwrap();
        assert_eq!(cycle_theme(system, 120), Theme::Glass);
        assert_eq!(cycle_theme(Theme::Glass, -120), system);
        assert_eq!(cycle_theme(system, -120), Theme::Glass);
        assert_eq!(capsule_text(Some(83.0)), "83%");
        for (old, current) in [
            ("codex-blue", system),
            ("frost-light", system),
            ("graphite-dark", system),
            ("privacy", system),
            ("light", system),
            ("dark", system),
            ("system", system),
            ("transparent", Theme::Glass),
        ] {
            let value = format!(
                "{{\"theme\":\"{old}\",\"offsetX\":1.0,\"offsetY\":2.0,\"startWithWindows\":true}}"
            );
            let settings: Settings = serde_json::from_str(&value).unwrap();
            assert_eq!(settings.theme, current);
            assert_eq!((settings.offset_x, settings.offset_y), (1.0, 2.0));
        }
    }

    #[test]
    fn double_click_only_resets_position() {
        let mut settings = Settings {
            theme: Theme::Glass,
            offset_x: 22.0,
            offset_y: -10.0,
            start_with_windows: true,
            always_on_top: true,
            global_position: None,
            ..Settings::default()
        };
        reset_position(&mut settings);
        assert_eq!((settings.offset_x, settings.offset_y), (0.0, 0.0));
        assert_eq!(settings.theme, Theme::Glass);
        assert!(settings.start_with_windows);
        assert!(settings.always_on_top);
    }

    #[test]
    fn dpi_anchor_and_drag_offset() {
        for dpi in [96, 120, 144, 168] {
            let px = dip_to_px(24.0, dpi);
            assert_eq!(px_to_dip(px, dpi), 24.0);
            let native_curve = native_capsule_curve(px);
            assert!(native_curve < px);
            assert!(px - native_curve >= 2);
            let (outer_x, outer_y, outer_w, outer_h) = capsule_window_rect(400, 600, dpi);
            assert_eq!(outer_x, 400 - dip_to_px(2.0, dpi));
            assert_eq!(outer_y, 600 - dip_to_px(2.0, dpi));
            assert_eq!(outer_w, dip_to_px(30.0, dpi));
            assert_eq!(outer_h, dip_to_px(30.0, dpi));
            assert!((outer_w - 2 * dip_to_px(2.0, dpi) - dip_to_px(26.0, dpi)).abs() <= 1);
        }
        let (pending, samples, ready) = stable_anchor(None, 0, Some((344, 900)));
        assert_eq!((pending, samples, ready), (Some((344, 900)), 1, None));
        assert_eq!(
            stable_anchor(pending, samples, Some((344, 900))).2,
            Some((344, 900))
        );
        assert_eq!(stable_anchor(pending, samples, Some((400, 900))).2, None);
        assert_eq!(
            drag_offset((2.0, 0.0), (100, 100), (125, 90), 120),
            (22.0, -8.0)
        );
    }

    #[test]
    fn cross_dpi_drag_saves_visible_origin_without_transparent_inset_drift() {
        assert_eq!(
            drag_window_offset((0.0, 0.0), (100, 200), (350, 450), 96, 144, 96),
            (251.0, 251.0)
        );
        assert_eq!(
            drag_window_offset((0.0, 0.0), (100, 200), (350, 450), 144, 96, 120),
            (199.2, 199.2)
        );
    }

    #[test]
    fn default_capsule_uses_avatar_center_and_ten_dip_gap() {
        assert_eq!(capsule_origin((344, 1400), 96, (0.0, 0.0)), (331, 1362));
        assert_eq!(capsule_origin((430, 1750), 120, (0.0, 0.0)), (414, 1702));
        assert_eq!(capsule_origin((430, 1750), 120, (4.0, -2.0)), (419, 1699));
    }

    #[test]
    fn global_topmost_has_no_codex_owner() {
        assert_eq!(badge_owner(123, true), 0);
        assert_eq!(badge_owner(123, false), 123);
    }

    #[test]
    fn global_topmost_keeps_last_frame_when_running_codex_is_minimized() {
        let mut settings = Settings::default();
        let last = Some((500, 600, 89, 38));
        assert_eq!(capsule_frame(&settings, None, 120, last), None);
        settings.always_on_top = true;
        assert_eq!(capsule_frame(&settings, None, 120, last), last);
        assert_eq!(capsule_frame(&settings, None, 120, None), None);
        let anchored = capsule_frame(&settings, Some((430, 1750)), 120, last);
        assert_eq!(
            anchored, last,
            "global coordinates must not follow the anchor"
        );
    }

    #[test]
    fn global_position_restores_work_relative_dip_and_clamps_after_screen_change() {
        let saved = GlobalPosition {
            device: "DISPLAY2".into(),
            x_dip: 100.0,
            y_dip: 120.0,
        };
        for (dpi, want) in [
            (96, (-1820, 120, 72, 34)),
            (120, (-1795, 150, 90, 43)),
            (144, (-1770, 180, 108, 51)),
            (168, (-1745, 210, 126, 60)),
        ] {
            assert_eq!(global_frame(Some(&saved), (-1920, 0, 0, 1080), dpi), want);
        }
        assert_eq!(global_frame(None, (0, 0, 800, 600), 96), (712, 550, 72, 34));
        let offscreen = GlobalPosition {
            x_dip: 9999.0,
            y_dip: 9999.0,
            ..saved
        };
        assert_eq!(
            global_frame(Some(&offscreen), (0, 0, 800, 600), 96),
            (728, 566, 72, 34)
        );
    }

    #[test]
    fn invalid_global_position_does_not_reset_other_preferences() {
        for pos in [
            json!({"device":"DISPLAY1","xDip":null,"yDip":2}),
            json!({"device":"","xDip":0,"yDip":0}),
            json!({"device":"DISPLAY1","xDip":1e200,"yDip":0}),
        ] {
            let s: Settings = serde_json::from_value(json!({"theme":"system","offsetX":4,"offsetY":5,"startWithWindows":true,"alwaysOnTop":true,"globalPosition":pos})).unwrap();
            assert_eq!(s.theme, Theme::System);
            assert!(s.start_with_windows && s.always_on_top);
            assert!(s.global_position.is_none());
        }
    }

    #[test]
    fn clamp_capsule_within_work_area_even_after_monitor_removal() {
        assert_eq!(
            clamp_to_work_area((80, 90), (79, 30), (0, 0, 100, 100)),
            (21, 70)
        );
        assert_eq!(
            clamp_to_work_area((-100, -100), (79, 30), (0, 0, 100, 100)),
            (0, 0)
        );
        assert_eq!(
            clamp_to_work_area((500, 500), (120, 30), (0, 0, 100, 100)),
            (0, 70)
        );
    }

    #[test]
    fn visible_anchor_follows_motion_without_flicker() {
        assert_eq!(
            follow_anchor(Some((344, 900)), 0, Some((400, 900))),
            (Some((400, 900)), 0)
        );
        assert_eq!(
            follow_anchor(Some((400, 900)), 0, None),
            (Some((400, 900)), 1)
        );
        assert_eq!(follow_anchor(Some((400, 900)), 7, None), (None, 8));
    }

    #[test]
    fn transient_anchor_failure_keeps_position_through_the_one_second_layout_gap() {
        let mut point = Some((344, 900));
        let mut misses = 0;
        for _ in 0..4 {
            (point, misses) = follow_anchor(point, misses, None);
        }
        assert_eq!(point, Some((344, 900)));
        assert_eq!(
            follow_anchor(point, misses, Some((344, 1200))),
            (Some((344, 1200)), 0)
        );
    }

    #[test]
    fn credit_popup_size_fits_one_row_and_adds_only_each_row_and_gap() {
        assert_eq!(credit_popup_height(1), 36.0);
        assert_eq!(credit_popup_height(2), 54.0);
        assert_eq!(credit_popup_height(8), 162.0);
    }

    #[test]
    fn layout_recovery_retries_without_inheriting_missing_anchor_backoff() {
        for misses in [0, 1, 4, 100] {
            assert_eq!(
                anchor_scan_interval(misses, true),
                Duration::from_millis(250)
            );
        }
        assert!(anchor_scan_interval(100, false) >= Duration::from_secs(8));
    }

    #[test]
    fn anchor_is_retained_to_the_deadline_and_probe_runs_at_the_recovery_boundary() {
        let start = Instant::now();
        let until = start + Duration::from_secs(2);
        let mut point = Some((344, 900));
        let mut misses = 0;
        for i in 0..8 {
            (point, misses) = follow_anchor_during_recovery(
                point,
                misses,
                None,
                start + Duration::from_millis(i * 250),
                until,
            );
        }
        assert_eq!(
            point,
            Some((344, 900)),
            "1.75s is still inside the recovery window"
        );
        assert!(anchor_scan_due(
            until,
            start + Duration::from_millis(1750),
            until,
            true,
            0
        ));
        assert!(!anchor_scan_due(
            until + Duration::from_millis(10),
            until,
            until,
            false,
            0
        ));
        assert_eq!(
            follow_anchor_during_recovery(point, misses, Some((344, 1200)), until, until),
            (Some((344, 1200)), 0)
        );
        assert_eq!(
            follow_anchor_during_recovery(point, misses, None, until, until),
            (None, 9)
        );
    }

    #[test]
    fn recovery_boundary_does_not_wait_for_the_long_retry_delay() {
        let start = Instant::now();
        let until = start + Duration::from_secs(2);
        assert!(anchor_scan_due(
            until,
            start + Duration::from_millis(1750),
            until,
            true,
            0
        ));
    }
}
