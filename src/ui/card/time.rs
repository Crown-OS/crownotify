//! Relative timestamps, as a card wears them.

use chrono::{DateTime, Local};

/// Renders how long ago `then` was, in the macOS notification vocabulary.
///
/// The scale is deliberately coarse and then jumps straight to a wall clock:
/// "17m" is useful, "3h" is not — by then what the user wants to know is *when*
/// it happened, not how long they have been ignoring it.
///
/// * under a minute (or in the future, which a clock adjustment can produce) —
///   `now`
/// * under an hour — `5m`
/// * earlier today — `9:41 AM`
/// * the previous calendar day — `Yesterday`
/// * older — `3 Aug`
pub fn relative_time(now: DateTime<Local>, then: DateTime<Local>) -> String {
    let seconds = now.signed_duration_since(then).num_seconds();
    if seconds < 60 {
        return "now".to_string();
    }
    if seconds < 3600 {
        return format!("{}m", seconds / 60);
    }
    let (today, that_day) = (now.date_naive(), then.date_naive());
    if today == that_day {
        then.format("%-I:%M %p").to_string()
    } else if today.pred_opt() == Some(that_day) {
        "Yesterday".to_string()
    } else {
        then.format("%-d %b").to_string()
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, TimeZone};

    use super::*;

    #[test]
    fn relative_timestamps_step_through_every_boundary() {
        let Some(now) = Local.with_ymd_and_hms(2025, 8, 9, 21, 30, 0).single() else {
            panic!("an unambiguous local time");
        };
        assert_eq!(relative_time(now, now), "now");
        assert_eq!(relative_time(now, now - Duration::seconds(59)), "now");
        assert_eq!(relative_time(now, now - Duration::seconds(60)), "1m");
        assert_eq!(relative_time(now, now - Duration::minutes(5)), "5m");
        assert_eq!(relative_time(now, now - Duration::minutes(59)), "59m");
        assert_eq!(relative_time(now, now - Duration::hours(12)), "9:30 AM");
        assert_eq!(relative_time(now, now - Duration::days(1)), "Yesterday");
        assert_eq!(relative_time(now, now - Duration::days(5)), "4 Aug");
    }

    #[test]
    fn a_timestamp_from_the_future_reads_as_now() {
        let now = Local::now();
        assert_eq!(relative_time(now, now + Duration::minutes(3)), "now");
    }
}
