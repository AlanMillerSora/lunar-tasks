use chrono::{Datelike, Local, NaiveTime, TimeZone, Timelike, Weekday};
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

fn parse_hhmm(text: &str) -> Option<NaiveTime> {
    NaiveTime::parse_from_str(text.trim(), "%H:%M").ok()
}

fn ru_day_offset(word: &str) -> Option<i64> {
    match word {
        "сегодня" => Some(0),
        "завтра" => Some(1),
        "послезавтра" => Some(2),
        _ => None,
    }
}

fn ru_weekday(word: &str) -> Option<Weekday> {
    match word {
        "пн" | "понедельник" => Some(Weekday::Mon),
        "вт" | "вторник" => Some(Weekday::Tue),
        "ср" | "среда" => Some(Weekday::Wed),
        "чт" | "четверг" => Some(Weekday::Thu),
        "пт" | "пятница" => Some(Weekday::Fri),
        "сб" | "суббота" => Some(Weekday::Sat),
        "вс" | "воскресенье" => Some(Weekday::Sun),
        _ => None,
    }
}

fn ru_unit_ms(unit: &str) -> Option<i64> {
    match unit {
        "минута" | "минуту" | "минуты" | "минут" | "мин" => Some(60_000),
        "час" | "часа" | "часов" => Some(3_600_000),
        "день" | "дня" | "дней" => Some(86_400_000),
        "неделя" | "неделю" | "недели" | "недель" => Some(604_800_000),
        _ => None,
    }
}

fn parse_ru_relative(rest: &str) -> Option<i64> {
    let mut words = rest.split_whitespace();
    let n: i64 = words.next()?.parse().ok()?;
    let unit = words.next()?;
    if words.next().is_some() || n < 0 {
        return None;
    }
    let ms = ru_unit_ms(unit)?;
    Some(now_ms() + n.checked_mul(ms)?)
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
    let make = |y: i32, mo: u32, d: u32, h: u32, mi: u32| {
        Local
            .with_ymd_and_hms(y, mo, d, h, mi, 0)
            .single()
            .map(|t| t.timestamp_millis())
    };

    let mut parts = text.splitn(2, char::is_whitespace);
    let first = parts.next().unwrap_or("");
    let rest = parts.next().unwrap_or("").trim();

    if first == "через" {
        return parse_ru_relative(rest);
    }

    if let Some(day_offset) = ru_day_offset(first) {
        let time = if rest.is_empty() {
            NaiveTime::from_hms_opt(9, 0, 0)?
        } else {
            parse_hhmm(rest)?
        };
        let mut date = now.date_naive() + chrono::Duration::days(day_offset);
        if day_offset == 0 && time <= now.time() {
            date += chrono::Duration::days(1);
        }
        return make(
            date.year(),
            date.month(),
            date.day(),
            time.hour(),
            time.minute(),
        );
    }

    if let Some(weekday) = ru_weekday(first) {
        let time = if rest.is_empty() {
            NaiveTime::from_hms_opt(9, 0, 0)?
        } else {
            parse_hhmm(rest)?
        };
        let today = now.date_naive();
        let days_ahead = (weekday.num_days_from_monday() as i64
            - today.weekday().num_days_from_monday() as i64)
            .rem_euclid(7);
        let mut date = today + chrono::Duration::days(days_ahead);
        // Today's weekday only counts if its time is still ahead; otherwise
        // use the next occurrence, a week from now.
        if days_ahead == 0 && time <= now.time() {
            date += chrono::Duration::days(7);
        }
        return make(
            date.year(),
            date.month(),
            date.day(),
            time.hour(),
            time.minute(),
        );
    }

    let (day_offset, text) = if let Some(r) = text.strip_prefix("tomorrow ") {
        (1, r)
    } else if let Some(r) = text.strip_prefix("today ") {
        (0, r)
    } else {
        (0, text.as_str())
    };
    if let Ok(t) = NaiveTime::parse_from_str(text, "%H:%M") {
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

#[derive(Clone, Debug, PartialEq)]
pub struct QuickParse {
    pub title: String,
    pub priority: u8,
    pub due: Option<i64>,
}

fn priority_token(word: &str) -> Option<u8> {
    match word.to_ascii_lowercase().as_str() {
        "!p1" | "!1" | "!high" => Some(1),
        "!p2" | "!2" | "!med" | "!medium" => Some(2),
        "!p3" | "!3" | "!low" => Some(3),
        _ => None,
    }
}

pub fn parse_quick(text: &str) -> QuickParse {
    let mut priority = 0u8;
    let mut words: Vec<&str> = Vec::new();
    for word in text.split_whitespace() {
        if let Some(p) = priority_token(word) {
            priority = p;
        } else {
            words.push(word);
        }
    }

    let cleaned = words.join(" ");
    let mut title = cleaned.clone();
    let mut due = None;

    // Keep trailing tags in the title; they should not block date detection.
    let mut tag_start = words.len();
    while tag_start > 0 && words[tag_start - 1].starts_with('#') {
        tag_start -= 1;
    }
    let core = &words[..tag_start];
    let tags = &words[tag_start..];

    let max = core.len().min(4);
    for n in (1..=max).rev() {
        let candidate = core[core.len() - n..].join(" ");
        if let Some(ms) = parse_when(&candidate) {
            if n == core.len() {
                // The whole text is a date; keep it as the title instead.
                break;
            }
            let mut kept: Vec<&str> = core[..core.len() - n].to_vec();
            kept.extend_from_slice(tags);
            title = kept.join(" ");
            due = Some(ms);
            break;
        }
    }

    QuickParse {
        title: one_line(&title),
        priority,
        due,
    }
}

pub fn notify(title: &str, body: &str) {
    let _ = std::process::Command::new("notify-send")
        .args([
            "-a",
            "Lunar Tasks",
            "-i",
            "tasks-rs",
            "-u",
            "normal",
            title,
            body,
        ])
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

    fn dt_of(ms: i64) -> chrono::DateTime<Local> {
        Local.timestamp_millis_opt(ms).single().expect("local time")
    }

    fn assert_approx(actual: i64, expected: i64, tolerance: i64) {
        assert!(
            (actual - expected).abs() <= tolerance,
            "expected ~{expected}, got {actual}"
        );
    }

    #[test]
    fn russian_day_words() {
        let today = Local::now().date_naive();

        let ms = parse_when("завтра 18:30").expect("завтра 18:30");
        let dt = dt_of(ms);
        assert_eq!(dt.date_naive(), today + chrono::Duration::days(1));
        assert_eq!((dt.hour(), dt.minute()), (18, 30));

        let ms = parse_when("послезавтра 09:00").expect("послезавтра 09:00");
        let dt = dt_of(ms);
        assert_eq!(dt.date_naive(), today + chrono::Duration::days(2));
        assert_eq!((dt.hour(), dt.minute()), (9, 0));

        // Bare day words default to 09:00.
        let ms = parse_when("Завтра").expect("bare завтра");
        let dt = dt_of(ms);
        assert_eq!(dt.date_naive(), today + chrono::Duration::days(1));
        assert_eq!((dt.hour(), dt.minute()), (9, 0));
    }

    #[test]
    fn russian_today_rolls_to_tomorrow_when_past() {
        let before = Local::now();
        let ms = parse_when("сегодня 09:00").expect("сегодня 09:00");
        let dt = dt_of(ms);
        assert_eq!((dt.hour(), dt.minute()), (9, 0));
        assert!(ms > before.timestamp_millis());

        let nine = NaiveTime::from_hms_opt(9, 0, 0).expect("09:00");
        let expected = if nine <= before.time() {
            before.date_naive() + chrono::Duration::days(1)
        } else {
            before.date_naive()
        };
        assert_eq!(dt.date_naive(), expected);
    }

    #[test]
    fn russian_relative_offsets() {
        let now = now_ms();
        let cases = [
            ("через 30 минут", 30 * 60_000),
            ("через 5 мин", 5 * 60_000),
            ("через 2 часа", 2 * 3_600_000),
            ("через 1 час", 3_600_000),
            ("через 1 день", 86_400_000),
            ("через 3 дня", 3 * 86_400_000),
            ("через 2 недели", 2 * 604_800_000),
            ("ЧЕРЕЗ 1 ЧАС", 3_600_000),
        ];
        for (input, delta) in cases {
            let ms = parse_when(input).unwrap_or_else(|| panic!("{input}"));
            assert_approx(ms, now + delta, 5_000);
        }
        assert!(parse_when("через много минут").is_none());
        assert!(parse_when("через 2 попугая").is_none());
        assert!(parse_when("через").is_none());
    }

    #[test]
    fn russian_weekdays_next_occurrence() {
        let now = now_ms();
        let cases = [
            ("пн 10:00", Weekday::Mon),
            ("вт 10:00", Weekday::Tue),
            ("ср 10:00", Weekday::Wed),
            ("чт 10:00", Weekday::Thu),
            ("пт 10:00", Weekday::Fri),
            ("сб 10:00", Weekday::Sat),
            ("вс 10:00", Weekday::Sun),
            ("ПН 10:00", Weekday::Mon),
            ("понедельник 10:00", Weekday::Mon),
            ("вторник", Weekday::Tue),
            ("среда 18:45", Weekday::Wed),
            ("четверг", Weekday::Thu),
            ("пятница 08:15", Weekday::Fri),
            ("суббота", Weekday::Sat),
            ("воскресенье 20:00", Weekday::Sun),
        ];
        for (input, weekday) in cases {
            let ms = parse_when(input).unwrap_or_else(|| panic!("{input}"));
            let dt = dt_of(ms);
            assert_eq!(dt.weekday(), weekday, "{input}");
            assert!(ms > now, "{input} must be strictly in the future");
            assert!(
                ms - now <= 7 * 86_400_000 + 60_000,
                "{input} should be within a week"
            );
        }

        let ms = parse_when("среда 18:45").expect("среда 18:45");
        let dt = dt_of(ms);
        assert_eq!((dt.hour(), dt.minute()), (18, 45));

        let ms = parse_when("четверг").expect("четверг");
        let dt = dt_of(ms);
        assert_eq!((dt.hour(), dt.minute()), (9, 0));
    }

    #[test]
    fn russian_today_weekday_rolls_a_week_when_past() {
        let now = Local::now();
        let name = match now.weekday() {
            Weekday::Mon => "пн",
            Weekday::Tue => "вт",
            Weekday::Wed => "ср",
            Weekday::Thu => "чт",
            Weekday::Fri => "пт",
            Weekday::Sat => "сб",
            Weekday::Sun => "вс",
        };
        let ms = parse_when(&format!("{name} 00:00")).expect("today weekday");
        let dt = dt_of(ms);
        // The time already passed today (or is exactly now), so this weekday
        // must not resolve to today.
        assert_eq!(dt.weekday(), now.weekday());
        assert_ne!(dt.date_naive(), now.date_naive());
        assert!(ms > now.timestamp_millis());
        assert!(ms - now.timestamp_millis() <= 8 * 86_400_000);
        assert_eq!((dt.hour(), dt.minute()), (0, 0));
    }

    #[test]
    fn garbage_dates_return_none() {
        for input in [
            "",
            "   ",
            "через",
            "через много минут",
            "через 2 попугая",
            "завтра 25:99",
            "понедельник 99:99",
            "не дата",
        ] {
            assert!(parse_when(input).is_none(), "{input:?} should not parse");
        }
    }

    #[test]
    fn quick_priority_tokens() {
        let q = parse_quick("купить хлеб !p1");
        assert_eq!(q.title, "купить хлеб");
        assert_eq!(q.priority, 1);
        assert_eq!(q.due, None);

        let q = parse_quick("!p2 позвонить маме !high");
        assert_eq!(q.title, "позвонить маме");
        assert_eq!(q.priority, 1);
        assert_eq!(q.due, None);

        assert_eq!(parse_quick("!p1 !p2").priority, 2);
        assert_eq!(parse_quick("!high !low").priority, 3);
        assert_eq!(parse_quick("!1 !MEDIUM").priority, 2);
        assert_eq!(parse_quick("!P3").priority, 3);
        assert_eq!(parse_quick("!3").priority, 3);
        assert_eq!(parse_quick("задача").priority, 0);
    }

    #[test]
    fn quick_trailing_date() {
        let today = Local::now().date_naive();
        let q = parse_quick("купить хлеб завтра 18:00");
        assert_eq!(q.title, "купить хлеб");
        assert_eq!(q.priority, 0);
        let dt = dt_of(q.due.expect("due"));
        assert_eq!(dt.date_naive(), today + chrono::Duration::days(1));
        assert_eq!((dt.hour(), dt.minute()), (18, 0));
    }

    #[test]
    fn quick_keeps_tags_and_removes_priority() {
        let today = Local::now().date_naive();
        let q = parse_quick("купить хлеб завтра 18:00 #home !p1");
        assert_eq!(q.title, "купить хлеб #home");
        assert_eq!(q.priority, 1);
        let dt = dt_of(q.due.expect("due"));
        assert_eq!(dt.date_naive(), today + chrono::Duration::days(1));
        assert_eq!((dt.hour(), dt.minute()), (18, 0));

        let q = parse_quick("позвонить #work");
        assert_eq!(q.title, "позвонить #work");
        assert_eq!(q.priority, 0);
        assert_eq!(q.due, None);
    }

    #[test]
    fn quick_falls_back_when_date_is_the_whole_text() {
        let q = parse_quick("завтра");
        assert_eq!(q.title, "завтра");
        assert_eq!(q.priority, 0);
        assert_eq!(q.due, None);

        let q = parse_quick("!p1 завтра");
        assert_eq!(q.title, "завтра");
        assert_eq!(q.priority, 1);
        assert_eq!(q.due, None);
    }

    #[test]
    fn quick_collapses_spaces_and_leaves_non_dates() {
        let q = parse_quick("  купить   хлеб   ");
        assert_eq!(q.title, "купить хлеб");
        assert_eq!(q.due, None);

        let q = parse_quick("купить хлеб в магазине");
        assert_eq!(q.title, "купить хлеб в магазине");
        assert_eq!(q.due, None);
        assert_eq!(q.priority, 0);
    }

    #[test]
    fn quick_parses_longer_suffixes_and_english() {
        let now = now_ms();
        let q = parse_quick("созвон через 30 минут");
        assert_eq!(q.title, "созвон");
        assert_approx(q.due.expect("due"), now + 30 * 60_000, 5_000);

        let q = parse_quick("buy milk tomorrow 09:00");
        assert_eq!(q.title, "buy milk");
        assert!(q.due.is_some());
    }
}
