//! Relative timestamps, as a card wears them.
//!
//! Written into a caller-owned `String` rather than returned, because the
//! surface re-renders every visible card's timestamp twice a second and a
//! freshly allocated `String` per card per tick is exactly the kind of cost the
//! retained text layouts exist to avoid.

use std::fmt::Write as _;

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
pub fn write_relative_time(out: &mut String, now: DateTime<Local>, then: DateTime<Local>) {
    out.clear();
    let seconds = now.signed_duration_since(then).num_seconds();
    if seconds < 60 {
        out.push_str("now");
        return;
    }
    if seconds < 3600 {
        let _ = write!(out, "{}m", seconds / 60);
        return;
    }

    let (today, that_day) = (now.date_naive(), then.date_naive());
    if today == that_day {
        let _ = write!(out, "{}", then.format("%-I:%M %p"));
    } else if today.pred_opt() == Some(that_day) {
        out.push_str("Yesterday");
    } else {
        let _ = write!(out, "{}", then.format("%-d %b"));
    }
}

/// [`write_relative_time`] into a fresh string. Convenience for tests and for
/// the rare caller that is not on the frame path.
pub fn relative_time(now: DateTime<Local>, then: DateTime<Local>) -> String {
    let mut out = String::new();
    write_relative_time(&mut out, now, then);
    out
}
