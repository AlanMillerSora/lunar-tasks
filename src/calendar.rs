use chrono::{Datelike, Local, NaiveDate, TimeZone};

/// Weekday headers, Monday first.
pub const WEEKDAYS: [&str; 7] = ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"];

/// A fixed 6x7 grid for `month`, with leading and trailing blanks.
///
/// The first week always starts on a Monday, so the grid has exactly 42
/// entries regardless of where the month begins or how long it is.
pub fn month_grid(year: i32, month: u32) -> Vec<Option<NaiveDate>> {
    let mut grid = vec![None; 42];
    let Some(first) = NaiveDate::from_ymd_opt(year, month, 1) else {
        return grid;
    };
    let offset = first.weekday().num_days_from_monday() as usize;
    let mut date = first;
    let mut index = offset;
    while index < grid.len() && date.month() == month {
        grid[index] = Some(date);
        index += 1;
        let Some(next) = date.succ_opt() else {
            break;
        };
        date = next;
    }
    grid
}

/// Move `delta` months from `year`/`month`, wrapping the year as needed.
pub fn shift_month(year: i32, month: u32, delta: i32) -> (i32, u32) {
    let total = year * 12 + month as i32 - 1 + delta;
    (total.div_euclid(12), total.rem_euclid(12) as u32 + 1)
}

/// A heading such as `"October 2026"`.
pub fn month_label(year: i32, month: u32) -> String {
    NaiveDate::from_ymd_opt(year, month, 1)
        .map(|date| date.format("%B %Y").to_string())
        .unwrap_or_else(|| format!("{year}-{month:02}"))
}

/// The local calendar month containing the timestamp `ms`.
pub fn month_of(ms: i64) -> (i32, u32) {
    let dt = Local
        .timestamp_millis_opt(ms)
        .single()
        .unwrap_or_else(Local::now);
    (dt.year(), dt.month())
}

/// The local day of the month containing the timestamp `ms`.
#[allow(dead_code)] // Kept for date-picker affordances.
pub fn day_of(ms: i64) -> u32 {
    let dt = Local
        .timestamp_millis_opt(ms)
        .single()
        .unwrap_or_else(Local::now);
    dt.day()
}

/// A local timestamp in milliseconds, or `None` for an invalid or
/// nonexistent (DST gap) local time.
pub fn date_ms(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> Option<i64> {
    Local
        .with_ymd_and_hms(year, month, day, hour, minute, 0)
        .single()
        .map(|dt| dt.timestamp_millis())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_is_always_six_weeks() {
        for (year, month) in [(2026, 10), (2024, 2), (2026, 2), (2025, 12), (2026, 1)] {
            assert_eq!(month_grid(year, month).len(), 42);
        }
    }

    #[test]
    fn grid_starts_on_correct_monday_index() {
        let grid = month_grid(2026, 10);
        let first = NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();
        let expected = first.weekday().num_days_from_monday() as usize;
        // 1 October 2026 is a Thursday, i.e. the fourth Monday-first cell.
        assert_eq!(expected, 3);
        assert_eq!(grid[expected], Some(first));
        assert!(grid[..expected].iter().all(Option::is_none));
        assert_eq!(grid[expected + 1], Some(first.succ_opt().unwrap()));
    }

    #[test]
    fn grid_days_follow_the_month() {
        let grid = month_grid(2026, 10);
        let days: Vec<NaiveDate> = grid.into_iter().flatten().collect();
        assert_eq!(days.len(), 31);
        assert_eq!(
            days.first(),
            Some(&NaiveDate::from_ymd_opt(2026, 10, 1).unwrap())
        );
        assert_eq!(
            days.last(),
            Some(&NaiveDate::from_ymd_opt(2026, 10, 31).unwrap())
        );
    }

    #[test]
    fn leap_year_february_has_29_days() {
        let days = month_grid(2024, 2).into_iter().flatten().count();
        assert_eq!(days, 29);
    }

    #[test]
    fn shift_month_wraps_years() {
        assert_eq!(shift_month(2026, 12, 1), (2027, 1));
        assert_eq!(shift_month(2026, 1, -1), (2025, 12));
        assert_eq!(shift_month(2026, 6, 7), (2027, 1));
        assert_eq!(shift_month(2026, 6, -7), (2025, 11));
        assert_eq!(shift_month(2026, 3, 0), (2026, 3));
        assert_eq!(shift_month(2026, 1, -13), (2024, 12));
    }

    #[test]
    fn month_label_uses_full_names() {
        assert_eq!(month_label(2026, 10), "October 2026");
        assert_eq!(month_label(2025, 1), "January 2025");
    }

    #[test]
    fn date_ms_round_trips_month_and_day() {
        let ms = date_ms(2026, 10, 25, 12, 30).expect("valid local date");
        assert_eq!(month_of(ms), (2026, 10));
        assert_eq!(day_of(ms), 25);

        let leap = date_ms(2024, 2, 29, 12, 0).expect("leap day");
        assert_eq!(month_of(leap), (2024, 2));
        assert_eq!(day_of(leap), 29);

        assert!(date_ms(2025, 2, 29, 12, 0).is_none());
    }
}
