use chrono::{Datelike, Local, TimeZone, Timelike};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    let mut out: String = s.chars().take(n.saturating_sub(1)).collect();
    out.push('…');
    out
}

pub fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn age(ms: i64) -> String {
    if ms <= 0 {
        return String::new();
    }
    let d = now_ms() - ms;
    if d < 0 {
        return "now".into();
    }
    if d < 60_000 {
        "now".into()
    } else if d < 3_600_000 {
        format!("{}m", d / 60_000)
    } else if d < 86_400_000 {
        format!("{}h", d / 3_600_000)
    } else if d < 604_800_000 {
        format!("{}d", d / 86_400_000)
    } else {
        let dt = Local.timestamp_millis_opt(ms).single();
        dt.map(|t| t.format("%-d %b").to_string())
            .unwrap_or_else(|| "—".into())
    }
}

pub fn due_label(ms: i64) -> String {
    if ms <= 0 {
        return String::new();
    }
    let now = now_ms();
    let dt = match Local.timestamp_millis_opt(ms).single() {
        Some(d) => d,
        None => return String::new(),
    };
    if ms < now {
        let d = now - ms;
        if d < 3_600_000 {
            return format!("{}m late", (d / 60_000).max(1));
        }
        if d < 86_400_000 {
            return format!("{}h late", d / 3_600_000);
        }
        return format!("{}d late", d / 86_400_000);
    }
    let t = format!("{:02}:{:02}", dt.hour(), dt.minute());
    let today = Local::now().date_naive();
    if dt.date_naive() == today {
        t
    } else if dt.date_naive() == today.succ_opt().unwrap_or(today) {
        format!("tomorrow {}", t)
    } else if (dt.date_naive() - today).num_days() < 7 {
        format!("{} {}", dt.format("%a"), t)
    } else {
        dt.format("%-d %b").to_string()
    }
}

pub fn parse_when(text: &str) -> Option<i64> {
    let text = text.trim().to_lowercase();
    if text.is_empty() {
        return None;
    }
    let now = Local::now();
    if let Some(rest) = text.strip_prefix('+') {
        let split = rest.find(|c: char| !c.is_ascii_digit() && c != '.')?;
        let n: f64 = rest[..split].parse().ok()?;
        let unit = rest[split..].trim();
        let ms = match unit {
            "m" | "min" | "mins" => 60_000.0,
            "h" | "hr" | "hrs" => 3_600_000.0,
            "d" | "day" | "days" => 86_400_000.0,
            "w" | "week" | "weeks" => 604_800_000.0,
            _ => return None,
        };
        return Some(now_ms() + (n * ms) as i64);
    }
    let (day_offset, text) = if let Some(r) = text.strip_prefix("tomorrow ") {
        (1, r)
    } else if let Some(r) = text.strip_prefix("today ") {
        (0, r)
    } else {
        (0, text.as_str())
    };
    let make = |y: i32, mo: u32, d: u32, h: u32, mi: u32| {
        Local
            .with_ymd_and_hms(y, mo, d, h, mi, 0)
            .single()
            .map(|t| t.timestamp_millis())
    };
    if let Ok(t) = chrono::NaiveTime::parse_from_str(text, "%H:%M") {
        let mut date = now.date_naive() + chrono::Duration::days(day_offset);
        if day_offset == 0 && t <= now.time() {
            date += chrono::Duration::days(1);
        }
        return make(date.year(), date.month(), date.day(), t.hour(), t.minute());
    }
    for fmt in ["%Y-%m-%d %H:%M", "%Y-%m-%d", "%d.%m %H:%M", "%d.%m"] {
        if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(text, fmt) {
            return make(dt.year(), dt.month(), dt.day(), dt.hour(), dt.minute());
        }
    }
    None
}

pub fn notify(title: &str, body: &str) {
    let _ = std::process::Command::new("notify-send")
        .args(["-a", "Tasks", "-i", "tasks-rs", "-u", "normal", title, body])
        .spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_times_parse() {
        let now = now_ms();
        let one_hour = parse_when("+1h").expect("+1h");
        assert!(one_hour >= now + 3_500_000 && one_hour <= now + 3_700_000);
        assert!(parse_when("+2d").is_some());
        assert!(parse_when("tomorrow 09:00").is_some());
    }

    #[test]
    fn shortens_words() {
        assert_eq!(truncate("abcdef", 4), "abc…");
        assert_eq!(truncate("abc", 4), "abc");
    }
}
