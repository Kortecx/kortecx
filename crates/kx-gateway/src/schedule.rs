//! T-APP-TRIGGER-TARGET: next-fire computation for CRON triggers.
//!
//! A `CRON` trigger's `schedule_spec` is now interpreted as EITHER:
//!
//! - a **legacy interval in seconds** — an all-ASCII-digit string (e.g. `"300"`).
//!   This is the pre-existing behavior, preserved with ZERO migration: an existing
//!   `triggers.db` row keeps firing exactly as before. A 5-field cron expression
//!   always contains spaces, so the two shapes are unambiguous.
//! - a **standard 5-field Unix crontab expression** (`min hour dom month dow`, e.g.
//!   `"0 9 * * 1-5"`), evaluated in the trigger's `timezone` (an IANA name such as
//!   `America/New_York`; empty ⇒ `UTC`) with DST-correct arithmetic via `chrono-tz`.
//!
//! The returned watermark is a wall-clock **ms-since-epoch (UTC)** — the store + the
//! cron ticker treat it as an opaque, monotone deadline (`next_fire_unix_ms`). This
//! helper is PURE (no I/O, no `now()` — `now_ms` is passed in) so register-time
//! seeding and tick-time advance can never diverge.

use std::str::FromStr;

use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use croner::Cron;

/// Why a `schedule_spec` / `timezone` could not be resolved into a next-fire.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub(crate) enum ScheduleError {
    /// The cron expression is not a valid 5-field Unix crontab pattern.
    #[error("invalid cron expression: {0}")]
    BadCron(String),
    /// The timezone is not a known IANA zone name.
    #[error("unknown timezone '{0}' (use an IANA name like 'America/New_York' or 'UTC')")]
    BadTimezone(String),
    /// A legacy interval spec of zero (or overflowing) seconds.
    #[error("cron interval must be > 0 seconds")]
    ZeroInterval,
    /// The cron expression matches no future instant (e.g. Feb-30).
    #[error("cron expression has no upcoming occurrence")]
    NoOccurrence,
    /// `now_ms` is outside the representable `DateTime` range.
    #[error("clock out of range")]
    ClockOutOfRange,
}

/// Whether `spec` is a **legacy interval-seconds** spec (non-empty, all ASCII digits).
/// A 5-field cron expression always contains spaces, so this is an exact discriminator.
pub(crate) fn is_interval_spec(spec: &str) -> bool {
    let s = spec.trim();
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// Resolve an IANA timezone name; an empty string ⇒ `UTC`.
pub(crate) fn parse_tz(timezone: &str) -> Result<Tz, ScheduleError> {
    let t = timezone.trim();
    if t.is_empty() {
        return Ok(Tz::UTC);
    }
    Tz::from_str(t).map_err(|_| ScheduleError::BadTimezone(timezone.to_string()))
}

/// Compute the next-fire watermark (ms since epoch, UTC) strictly AFTER `now_ms`.
///
/// - legacy interval spec ⇒ `now_ms + interval*1000`;
/// - 5-field cron spec ⇒ the next occurrence in `timezone`, DST-correct.
///
/// # Errors
/// [`ScheduleError`] on a malformed interval / cron expression / timezone. Callers
/// surface this at REGISTER time (a synchronous `invalid_argument`) so a typo is never
/// a silent never-firing trigger.
pub(crate) fn next_fire(
    schedule_spec: &str,
    timezone: &str,
    now_ms: u64,
) -> Result<u64, ScheduleError> {
    if is_interval_spec(schedule_spec) {
        let secs: u64 = schedule_spec
            .trim()
            .parse()
            .map_err(|_| ScheduleError::ZeroInterval)?;
        if secs == 0 {
            return Err(ScheduleError::ZeroInterval);
        }
        return Ok(now_ms.saturating_add(secs.saturating_mul(1000)));
    }
    // A 5-field crontab expression evaluated in `timezone`. croner's default parser
    // accepts EXACTLY 5 fields (a 6-field seconds pattern is rejected) — standard Unix
    // crontab semantics.
    let tz = parse_tz(timezone)?;
    let cron = Cron::new(schedule_spec.trim())
        .parse()
        .map_err(|e| ScheduleError::BadCron(e.to_string()))?;
    let now_utc: DateTime<Utc> = DateTime::from_timestamp_millis(
        i64::try_from(now_ms).map_err(|_| ScheduleError::ClockOutOfRange)?,
    )
    .ok_or(ScheduleError::ClockOutOfRange)?;
    let now_tz = now_utc.with_timezone(&tz);
    // `inclusive = false` ⇒ strictly after now (matches the interval path's forward step).
    let next = cron
        .find_next_occurrence(&now_tz, false)
        .map_err(|_| ScheduleError::NoOccurrence)?;
    let next_ms = next.with_timezone(&Utc).timestamp_millis();
    u64::try_from(next_ms).map_err(|_| ScheduleError::ClockOutOfRange)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone; // `Utc.with_ymd_and_hms` is a TimeZone-trait method.

    // A fixed reference instant: 2026-01-15T00:00:00Z = 1_768_435_200_000 ms.
    const REF_MS: u64 = 1_768_435_200_000;

    fn utc_ms(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> u64 {
        let dt = Utc.with_ymd_and_hms(y, mo, d, h, mi, 0).single().unwrap();
        u64::try_from(dt.timestamp_millis()).unwrap()
    }

    #[test]
    fn legacy_interval_is_preserved() {
        assert!(is_interval_spec("300"));
        assert!(is_interval_spec("  60 "));
        assert!(!is_interval_spec("0 9 * * *"));
        assert!(!is_interval_spec(""));
        // 300s past the reference instant.
        assert_eq!(next_fire("300", "", REF_MS).unwrap(), REF_MS + 300_000);
        // timezone is irrelevant for an interval.
        assert_eq!(
            next_fire("300", "America/New_York", REF_MS).unwrap(),
            REF_MS + 300_000
        );
    }

    #[test]
    fn zero_or_bad_interval_rejected() {
        assert_eq!(next_fire("0", "", REF_MS), Err(ScheduleError::ZeroInterval));
    }

    #[test]
    fn cron_daily_utc() {
        // 09:00 every day, UTC. From 2026-01-15T00:00Z the next is 2026-01-15T09:00Z.
        let got = next_fire("0 9 * * *", "UTC", REF_MS).unwrap();
        assert_eq!(got, utc_ms(2026, 1, 15, 9, 0));
        // Empty timezone defaults to UTC (same answer).
        assert_eq!(next_fire("0 9 * * *", "", REF_MS).unwrap(), got);
    }

    #[test]
    fn cron_is_strictly_after_now() {
        // At exactly 09:00Z, the NEXT 09:00 daily fire is the following day (inclusive=false).
        let at_nine = utc_ms(2026, 1, 15, 9, 0);
        let got = next_fire("0 9 * * *", "UTC", at_nine).unwrap();
        assert_eq!(got, utc_ms(2026, 1, 16, 9, 0));
    }

    #[test]
    fn cron_respects_timezone_offset() {
        // 09:00 in America/New_York (EST = UTC-5 in January) == 14:00Z.
        let got = next_fire("0 9 * * *", "America/New_York", REF_MS).unwrap();
        assert_eq!(got, utc_ms(2026, 1, 15, 14, 0));
    }

    #[test]
    fn cron_dst_spring_forward_is_valid_and_monotone() {
        // US spring-forward 2026: Sun 2026-03-08, clocks jump 02:00→03:00 (EST→EDT).
        // A daily 02:30 job on that day hits the non-existent local hour; the helper
        // must still yield a VALID future instant (no panic), strictly after now, and
        // resume normal offset after the transition.
        let before = utc_ms(2026, 3, 8, 6, 0); // 01:00 EST on the DST morning
        let got = next_fire("30 2 * * *", "America/New_York", before).unwrap();
        assert!(got > before, "next fire must be strictly in the future");
        assert!(
            got < before + 2 * 24 * 60 * 60 * 1000,
            "a daily job fires within ~2 days even across the gap"
        );
        // The day AFTER the transition, 02:30 EDT (UTC-4) == 06:30Z — normal resumption.
        let after = utc_ms(2026, 3, 9, 0, 0);
        let next = next_fire("30 2 * * *", "America/New_York", after).unwrap();
        assert_eq!(next, utc_ms(2026, 3, 9, 6, 30));
    }

    #[test]
    fn bad_cron_and_bad_timezone_rejected() {
        assert!(matches!(
            next_fire("not a cron", "UTC", REF_MS),
            Err(ScheduleError::BadCron(_))
        ));
        // A 6-field (seconds) pattern is rejected by the default 5-field parser.
        assert!(matches!(
            next_fire("0 0 9 * * *", "UTC", REF_MS),
            Err(ScheduleError::BadCron(_))
        ));
        assert_eq!(
            next_fire("0 9 * * *", "Mars/Phobos", REF_MS),
            Err(ScheduleError::BadTimezone("Mars/Phobos".into()))
        );
    }

    #[test]
    fn parse_tz_empty_is_utc() {
        assert_eq!(parse_tz("").unwrap(), Tz::UTC);
        assert_eq!(parse_tz("  ").unwrap(), Tz::UTC);
        assert!(parse_tz("America/New_York").is_ok());
        assert!(parse_tz("nope").is_err());
    }
}
