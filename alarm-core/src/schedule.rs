use chrono::{
    DateTime, Datelike, Days, Duration, Local, LocalResult, NaiveDate, NaiveDateTime, TimeZone,
};

use crate::{Alarm, AlarmState, RepeatRule};

pub fn next_occurrence_after(alarm: &Alarm, now: DateTime<Local>) -> Option<DateTime<Local>> {
    if !alarm.enabled {
        return None;
    }

    if let AlarmState::Ringing = alarm.state {
        return alarm.next_trigger_at;
    }

    if let Some(snoozed_until) = alarm.next_trigger_at
        && alarm.state == AlarmState::Snoozed
        && snoozed_until > now
    {
        return Some(snoozed_until);
    }

    match &alarm.repeat_rule {
        RepeatRule::Once => first_match(alarm, now.date_naive(), now),
        RepeatRule::Weekdays => next_matching_weekday(alarm, now, &[0, 1, 2, 3, 4]),
        RepeatRule::CustomDays(days) => {
            let weekday_ids = days
                .iter()
                .map(|day| day.num_days_from_monday())
                .collect::<Vec<_>>();
            next_matching_weekday(alarm, now, &weekday_ids)
        }
    }
}

fn next_matching_weekday(
    alarm: &Alarm,
    now: DateTime<Local>,
    weekday_ids: &[u32],
) -> Option<DateTime<Local>> {
    for offset in 0..14 {
        let date = now.date_naive().checked_add_days(Days::new(offset))?;
        if weekday_ids.contains(&date.weekday().num_days_from_monday())
            && let Some(candidate) = first_match(alarm, date, now)
        {
            return Some(candidate);
        }
    }

    None
}

fn first_match(alarm: &Alarm, date: NaiveDate, now: DateTime<Local>) -> Option<DateTime<Local>> {
    let naive = NaiveDateTime::new(date, alarm.time_local);
    let candidate = resolve_local_candidate(naive)?;

    if candidate > now {
        Some(candidate)
    } else {
        None
    }
}

fn resolve_local_candidate(naive: NaiveDateTime) -> Option<DateTime<Local>> {
    match Local.from_local_datetime(&naive) {
        LocalResult::Single(dt) => Some(dt),
        LocalResult::Ambiguous(first, second) => Some(first.min(second)),
        LocalResult::None => {
            for minute_offset in 1..=180 {
                let shifted = naive.checked_add_signed(Duration::minutes(minute_offset))?;
                if shifted.date() != naive.date() {
                    break;
                }

                match Local.from_local_datetime(&shifted) {
                    LocalResult::Single(dt) => return Some(dt),
                    LocalResult::Ambiguous(first, second) => return Some(first.min(second)),
                    LocalResult::None => continue,
                }
            }

            None
        }
    }
}

pub fn describe_next_alarm(alarm: &Alarm, now: DateTime<Local>) -> Option<String> {
    let trigger = next_occurrence_after(alarm, now)?;
    Some(trigger.format("%a %H:%M").to_string())
}

#[cfg(test)]
mod tests {
    use chrono::{Datelike, Local, NaiveTime, TimeZone, Timelike, Weekday};

    use crate::{AlarmDraft, AlarmState, RepeatRule};

    use super::next_occurrence_after;

    #[test]
    fn once_alarm_rolls_forward_only_if_future() {
        let now = Local
            .with_ymd_and_hms(2026, 3, 19, 8, 0, 0)
            .single()
            .expect("fixed test time");
        let alarm = AlarmDraft {
            repeat_rule: RepeatRule::Once,
            time_local: NaiveTime::from_hms_opt(8, 5, 0).unwrap(),
            ..AlarmDraft::default()
        }
        .into_alarm(now)
        .expect("valid alarm");

        let next = next_occurrence_after(&alarm, now).expect("next alarm");
        assert_eq!(next.hour(), 8);
        assert_eq!(next.minute(), 5);
    }

    #[test]
    fn weekday_alarm_skips_weekend() {
        let now = Local
            .with_ymd_and_hms(2026, 3, 20, 22, 0, 0)
            .single()
            .expect("fixed friday");
        assert_eq!(now.weekday(), Weekday::Fri);

        let alarm = AlarmDraft {
            repeat_rule: RepeatRule::Weekdays,
            time_local: NaiveTime::from_hms_opt(7, 30, 0).unwrap(),
            ..AlarmDraft::default()
        }
        .into_alarm(now)
        .expect("valid alarm");

        let next = next_occurrence_after(&alarm, now).expect("next alarm");
        assert_eq!(next.weekday(), Weekday::Mon);
        assert_eq!(next.hour(), 7);
    }

    #[test]
    fn disabled_alarm_has_no_next_occurrence() {
        let now = Local
            .with_ymd_and_hms(2026, 3, 19, 8, 0, 0)
            .single()
            .expect("fixed test time");
        let alarm = AlarmDraft {
            enabled: false,
            ..AlarmDraft::default()
        }
        .into_alarm(now)
        .expect("valid alarm");

        assert!(next_occurrence_after(&alarm, now).is_none());
    }

    #[test]
    fn snoozed_alarm_keeps_future_snooze_until() {
        let now = Local
            .with_ymd_and_hms(2026, 3, 19, 8, 0, 0)
            .single()
            .expect("fixed test time");
        let snoozed_until = Local
            .with_ymd_and_hms(2026, 3, 19, 8, 12, 0)
            .single()
            .expect("fixed snooze time");
        let mut alarm = AlarmDraft::default().into_alarm(now).expect("valid alarm");
        alarm.state = AlarmState::Snoozed;
        alarm.next_trigger_at = Some(snoozed_until);

        let next = next_occurrence_after(&alarm, now).expect("next alarm");
        assert_eq!(next, snoozed_until);
    }
}
