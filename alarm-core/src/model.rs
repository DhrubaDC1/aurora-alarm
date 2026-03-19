use anyhow::{Result, bail, ensure};
use chrono::{DateTime, Local, NaiveTime, Weekday};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub type AlarmId = Uuid;

const DEFAULT_LABEL: &str = "Alarm";
const DEFAULT_SOUND_ID: &str = "default";
const MAX_LABEL_LEN: usize = 80;
const MAX_SNOOZE_MINUTES: u16 = 240;
const MAX_GRACE_WINDOW_MINUTES: u16 = 180;

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
            label: DEFAULT_LABEL.into(),
            time_local: NaiveTime::from_hms_opt(7, 0, 0).expect("valid default time"),
            repeat_rule: RepeatRule::Weekdays,
            sound_id: DEFAULT_SOUND_ID.into(),
            volume: 80,
            enabled: true,
            snooze_minutes: 10,
        }
    }
}

impl AlarmDraft {
    pub fn normalized(self) -> Result<Self> {
        let label = normalize_label(self.label);
        let sound_id = normalize_sound_id(self.sound_id);
        let repeat_rule = self.repeat_rule.normalized()?;
        let snooze_minutes = normalize_snooze_minutes(self.snooze_minutes);

        ensure!(
            label.len() <= MAX_LABEL_LEN,
            "alarm label must be at most {MAX_LABEL_LEN} characters"
        );

        Ok(Self {
            label,
            time_local: self.time_local,
            repeat_rule,
            sound_id,
            volume: self.volume.min(100),
            enabled: self.enabled,
            snooze_minutes,
        })
    }

    pub fn into_alarm(self, now: DateTime<Local>) -> Result<Alarm> {
        let draft = self.normalized()?;
        Ok(Alarm {
            id: Uuid::new_v4(),
            label: draft.label,
            time_local: draft.time_local,
            repeat_rule: draft.repeat_rule,
            sound_id: draft.sound_id,
            volume: draft.volume,
            enabled: draft.enabled,
            snooze_minutes: draft.snooze_minutes,
            state: AlarmState::Idle,
            next_trigger_at: None,
            last_triggered_at: None,
            created_at: now,
            updated_at: now,
        })
    }
}

impl Alarm {
    pub fn normalized(mut self) -> Result<Self> {
        self.label = normalize_label(self.label);
        self.sound_id = normalize_sound_id(self.sound_id);
        self.repeat_rule = self.repeat_rule.normalized()?;
        self.volume = self.volume.min(100);
        self.snooze_minutes = normalize_snooze_minutes(self.snooze_minutes);

        if !self.enabled {
            self.state = AlarmState::Idle;
            self.next_trigger_at = None;
        }

        self.validate()?;
        Ok(self)
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(!self.label.is_empty(), "alarm label must not be empty");
        ensure!(
            self.label.len() <= MAX_LABEL_LEN,
            "alarm label must be at most {MAX_LABEL_LEN} characters"
        );
        ensure!(
            !self.sound_id.is_empty(),
            "alarm sound id must not be empty"
        );
        ensure!(
            self.snooze_minutes >= 1 && self.snooze_minutes <= MAX_SNOOZE_MINUTES,
            "snooze duration must be between 1 and {MAX_SNOOZE_MINUTES} minutes"
        );
        self.repeat_rule.validate()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum RepeatRule {
    Once,
    Weekdays,
    CustomDays(Vec<Weekday>),
}

impl RepeatRule {
    pub fn normalized(self) -> Result<Self> {
        match self {
            Self::CustomDays(days) => {
                let mut normalized = days;
                normalized.sort_by_key(Weekday::num_days_from_monday);
                normalized.dedup_by_key(|day| day.num_days_from_monday());
                if normalized.is_empty() {
                    bail!("custom repeat rules must include at least one weekday");
                }
                Ok(Self::CustomDays(normalized))
            }
            rule => Ok(rule),
        }
    }

    pub fn validate(&self) -> Result<()> {
        if let Self::CustomDays(days) = self {
            ensure!(
                !days.is_empty(),
                "custom repeat rules must include at least one weekday"
            );
        }
        Ok(())
    }
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
    pub audio_available: bool,
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

impl Settings {
    pub fn normalized(mut self) -> Self {
        self.theme = normalize_theme(self.theme);
        self.grace_window_minutes = self.grace_window_minutes.clamp(1, MAX_GRACE_WINDOW_MINUTES);
        self.default_snooze_minutes = normalize_snooze_minutes(self.default_snooze_minutes);
        self
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(!self.theme.is_empty(), "theme must not be empty");
        ensure!(
            self.grace_window_minutes >= 1 && self.grace_window_minutes <= MAX_GRACE_WINDOW_MINUTES,
            "grace window must be between 1 and {MAX_GRACE_WINDOW_MINUTES} minutes"
        );
        ensure!(
            self.default_snooze_minutes >= 1 && self.default_snooze_minutes <= MAX_SNOOZE_MINUTES,
            "default snooze must be between 1 and {MAX_SNOOZE_MINUTES} minutes"
        );
        Ok(())
    }
}

fn normalize_label(label: String) -> String {
    let trimmed = label.trim();
    if trimmed.is_empty() {
        DEFAULT_LABEL.into()
    } else {
        trimmed.chars().take(MAX_LABEL_LEN).collect()
    }
}

fn normalize_sound_id(sound_id: String) -> String {
    let trimmed = sound_id.trim();
    if trimmed.is_empty() {
        DEFAULT_SOUND_ID.into()
    } else {
        trimmed.to_string()
    }
}

fn normalize_theme(theme: String) -> String {
    let trimmed = theme.trim();
    if trimmed.is_empty() {
        "aurora".into()
    } else {
        trimmed.to_string()
    }
}

fn normalize_snooze_minutes(minutes: u16) -> u16 {
    minutes.clamp(1, MAX_SNOOZE_MINUTES)
}
