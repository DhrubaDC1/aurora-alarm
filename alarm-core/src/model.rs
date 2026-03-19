use chrono::{DateTime, Local, NaiveTime, Weekday};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub type AlarmId = Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Alarm {
    pub id: AlarmId,
    pub label: String,
    pub time_local: NaiveTime,
    pub repeat_rule: RepeatRule,
    pub sound_id: String,
    pub volume: u8,
    pub enabled: bool,
    pub snooze_minutes: u16,
    pub state: AlarmState,
    pub next_trigger_at: Option<DateTime<Local>>,
    pub last_triggered_at: Option<DateTime<Local>>,
    pub created_at: DateTime<Local>,
    pub updated_at: DateTime<Local>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlarmDraft {
    pub label: String,
    pub time_local: NaiveTime,
    pub repeat_rule: RepeatRule,
    pub sound_id: String,
    pub volume: u8,
    pub enabled: bool,
    pub snooze_minutes: u16,
}

impl Default for AlarmDraft {
    fn default() -> Self {
        Self {
            label: "Alarm".into(),
            time_local: NaiveTime::from_hms_opt(7, 0, 0).expect("valid default time"),
            repeat_rule: RepeatRule::Weekdays,
            sound_id: "default".into(),
            volume: 80,
            enabled: true,
            snooze_minutes: 10,
        }
    }
}

impl AlarmDraft {
    pub fn into_alarm(self, now: DateTime<Local>) -> Alarm {
        Alarm {
            id: Uuid::new_v4(),
            label: self.label,
            time_local: self.time_local,
            repeat_rule: self.repeat_rule,
            sound_id: self.sound_id,
            volume: self.volume.min(100),
            enabled: self.enabled,
            snooze_minutes: self.snooze_minutes.max(1),
            state: AlarmState::Idle,
            next_trigger_at: None,
            last_triggered_at: None,
            created_at: now,
            updated_at: now,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum RepeatRule {
    Once,
    Weekdays,
    CustomDays(Vec<Weekday>),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum AlarmState {
    Idle,
    Scheduled,
    Ringing,
    Snoozed,
    Missed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum AlarmStatus {
    Quiet,
    Upcoming,
    Ringing,
    Snoozed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveAlarm {
    pub alarm_id: AlarmId,
    pub label: String,
    pub state: AlarmState,
    pub due_at: DateTime<Local>,
    pub snoozed_until: Option<DateTime<Local>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonStatus {
    pub status: AlarmStatus,
    pub next_alarm_at: Option<DateTime<Local>>,
    pub active_alarm: Option<ActiveAlarm>,
    pub tray_available: bool,
    pub notifications_available: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSnapshot {
    pub generated_at: DateTime<Local>,
    pub alarms: Vec<Alarm>,
    pub status: DaemonStatus,
    pub settings: Settings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub theme: String,
    pub autostart: bool,
    pub launch_minimized: bool,
    pub grace_window_minutes: u16,
    pub default_snooze_minutes: u16,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: "aurora".into(),
            autostart: true,
            launch_minimized: false,
            grace_window_minutes: 15,
            default_snooze_minutes: 10,
        }
    }
}
