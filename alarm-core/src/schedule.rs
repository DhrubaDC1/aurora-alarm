use chrono::{DateTime, Datelike, Days, Local, LocalResult, NaiveDate, NaiveDateTime, TimeZone};

use crate::{Alarm, AlarmState, RepeatRule};

pub fn next_occurrence_after(alarm: &Alarm, now: DateTime<Local>) -> Option<DateTime<Local>> {
    if !alarm.enabled {
        return None;
    }

    if let AlarmState::Ringing = alarm.state {
        return alarm.next_trigger_at;
    }

    if let Some(snoozed_until) = alarm.next_trigger_at {
        if alarm.state == AlarmState::Snoozed && snoozed_until > now {
            return Some(snoozed_until);
        }
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
        if weekday_ids.contains(&date.weekday().num_days_from_monday()) {
            if let Some(candidate) = first_match(alarm, date, now) {
                return Some(candidate);
            }
        }
    }

    None
}

fn first_match(alarm: &Alarm, date: NaiveDate, now: DateTime<Local>) -> Option<DateTime<Local>> {
    let naive = NaiveDateTime::new(date, alarm.time_local);
    let candidate = match Local.from_local_datetime(&naive) {
        LocalResult::Single(dt) => dt,
        LocalResult::Ambiguous(first, second) => first.min(second),
        LocalResult::None => return None,
    };

    if candidate > now {
        Some(candidate)
    } else {
        None
    }
}

pub fn describe_next_alarm(alarm: &Alarm, now: DateTime<Local>) -> Option<String> {
    let trigger = next_occurrence_after(alarm, now)?;
    Some(trigger.format("%a %H:%M").to_string())
}

#[cfg(test)]
mod tests {
    use chrono::{Datelike, Local, NaiveTime, TimeZone, Timelike, Weekday};

    use crate::{AlarmDraft, RepeatRule};

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
        .into_alarm(now);

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
        .into_alarm(now);

        let next = next_occurrence_after(&alarm, now).expect("next alarm");
        assert_eq!(next.weekday(), Weekday::Mon);
        assert_eq!(next.hour(), 7);
    }
}
