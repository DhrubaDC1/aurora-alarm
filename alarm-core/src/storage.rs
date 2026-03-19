use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Local, NaiveTime, Utc};
use directories::ProjectDirs;
use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::{Alarm, AlarmId, AlarmState, AppSnapshot, DaemonStatus, RepeatRule, Settings};

const SCHEMA_VERSION: i64 = 2;

#[derive(Debug, Clone)]
pub struct AuroraPaths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub state_dir: PathBuf,
    pub log_dir: PathBuf,
    pub db_path: PathBuf,
}

impl AuroraPaths {
    pub fn discover() -> Result<Self> {
        let dirs = ProjectDirs::from("io", "codex", "Aurora Alarm")
            .context("failed to resolve XDG project directories")?;
        let config_dir = dirs.config_dir().to_path_buf();
        let data_dir = dirs.data_local_dir().to_path_buf();
        let state_dir = dirs
            .state_dir()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| data_dir.join("state"));
        let log_dir = state_dir.join("logs");
        let db_path = data_dir.join("aurora-alarm.sqlite3");

        fs::create_dir_all(&config_dir).context("failed to create config directory")?;
        fs::create_dir_all(&data_dir).context("failed to create data directory")?;
        fs::create_dir_all(&state_dir).context("failed to create state directory")?;
        fs::create_dir_all(&log_dir).context("failed to create log directory")?;

        Ok(Self {
            config_dir,
            data_dir,
            state_dir,
            log_dir,
            db_path,
        })
    }

    pub fn from_root(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref();
        let config_dir = root.join("config");
        let data_dir = root.join("data");
        let state_dir = root.join("state");
        let log_dir = state_dir.join("logs");
        let db_path = data_dir.join("aurora-alarm.sqlite3");

        fs::create_dir_all(&config_dir).context("failed to create config directory")?;
        fs::create_dir_all(&data_dir).context("failed to create data directory")?;
        fs::create_dir_all(&state_dir).context("failed to create state directory")?;
        fs::create_dir_all(&log_dir).context("failed to create log directory")?;

        Ok(Self {
            config_dir,
            data_dir,
            state_dir,
            log_dir,
            db_path,
        })
    }
}

pub struct Storage {
    conn: Connection,
}

impl Storage {
    pub fn open(paths: &AuroraPaths) -> Result<Self> {
        let conn = Connection::open(&paths.db_path).context("failed to open SQLite database")?;
        let storage = Self { conn };
        storage.migrate()?;
        storage.ensure_default_settings()?;
        Ok(storage)
    }

    fn migrate(&self) -> Result<()> {
        self.conn.execute_batch("PRAGMA foreign_keys = ON;")?;

        if self.has_legacy_json_schema()? {
            self.migrate_legacy_json_schema()?;
        }

        self.create_schema()?;
        self.set_schema_version(SCHEMA_VERSION)?;

        Ok(())
    }

    fn has_table(&self, table: &str) -> Result<bool> {
        Ok(self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
            [table],
            |row| row.get::<_, i64>(0),
        )? != 0)
    }

    fn table_has_column(&self, table: &str, column: &str) -> Result<bool> {
        let pragma = format!("PRAGMA table_info({table})");
        let mut stmt = self.conn.prepare(&pragma)?;
        let columns = stmt.query_map([], |row| row.get::<_, String>(1))?;
        for found in columns {
            if found? == column {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn has_legacy_json_schema(&self) -> Result<bool> {
        if self.has_table("schema_meta")? {
            return Ok(false);
        }

        let alarms_legacy = self.has_table("alarms")? && self.table_has_column("alarms", "json")?;
        let settings_legacy =
            self.has_table("settings")? && self.table_has_column("settings", "json")?;
        Ok(alarms_legacy || settings_legacy)
    }

    fn migrate_legacy_json_schema(&self) -> Result<()> {
        let alarms = if self.has_table("alarms")? && self.table_has_column("alarms", "json")? {
            let mut stmt = self.conn.prepare("SELECT json FROM alarms ORDER BY id")?;
            let rows = stmt
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows.into_iter()
                .map(|json| {
                    let alarm = serde_json::from_str::<Alarm>(&json)
                        .context("failed to deserialize legacy alarm")?;
                    alarm.normalized()
                })
                .collect::<Result<Vec<_>>>()?
        } else {
            Vec::new()
        };

        let settings =
            if self.has_table("settings")? && self.table_has_column("settings", "json")? {
                self.conn
                    .query_row("SELECT json FROM settings WHERE id = 1", [], |row| {
                        row.get::<_, String>(0)
                    })
                    .optional()?
                    .map(|json| {
                        serde_json::from_str::<Settings>(&json)
                            .context("failed to deserialize legacy settings")
                            .map(Settings::normalized)
                    })
                    .transpose()?
                    .unwrap_or_default()
            } else {
                Settings::default()
            };

        self.conn.execute_batch(
            "
            DROP TABLE IF EXISTS alarms;
            DROP TABLE IF EXISTS settings;
            DROP TABLE IF EXISTS schema_meta;
            ",
        )?;

        self.create_schema()?;
        self.save_settings(&settings)?;
        for alarm in alarms {
            self.save_alarm(&alarm)?;
        }

        Ok(())
    }

    fn create_schema(&self) -> Result<()> {
        self.conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS schema_meta (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS alarms (
                id TEXT PRIMARY KEY,
                label TEXT NOT NULL,
                time_local TEXT NOT NULL,
                repeat_rule_kind TEXT NOT NULL,
                repeat_rule_days TEXT,
                sound_id TEXT NOT NULL,
                volume INTEGER NOT NULL,
                enabled INTEGER NOT NULL,
                snooze_minutes INTEGER NOT NULL,
                state TEXT NOT NULL,
                next_trigger_at TEXT,
                last_triggered_at TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS settings (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                theme TEXT NOT NULL,
                autostart INTEGER NOT NULL,
                launch_minimized INTEGER NOT NULL,
                grace_window_minutes INTEGER NOT NULL,
                default_snooze_minutes INTEGER NOT NULL
            );
            ",
        )?;
        Ok(())
    }

    fn set_schema_version(&self, version: i64) -> Result<()> {
        self.conn.execute(
            "INSERT INTO schema_meta (key, value) VALUES ('schema_version', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [version.to_string()],
        )?;
        Ok(())
    }

    fn ensure_default_settings(&self) -> Result<()> {
        let settings_count: i64 =
            self.conn
                .query_row("SELECT COUNT(*) FROM settings", [], |row| row.get(0))?;
        if settings_count == 0 {
            self.save_settings(&Settings::default())?;
        }
        Ok(())
    }

    pub fn load_alarms(&self) -> Result<Vec<Alarm>> {
        let mut stmt = self.conn.prepare(
            "SELECT
                id,
                label,
                time_local,
                repeat_rule_kind,
                repeat_rule_days,
                sound_id,
                volume,
                enabled,
                snooze_minutes,
                state,
                next_trigger_at,
                last_triggered_at,
                created_at,
                updated_at
             FROM alarms
             ORDER BY time_local, label, id",
        )?;
        let alarms = stmt
            .query_map([], decode_alarm_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .map(Alarm::normalized)
            .collect::<Result<Vec<_>>>()
            .context("failed to load alarms")?;
        Ok(alarms)
    }

    pub fn load_settings(&self) -> Result<Settings> {
        let settings = self
            .conn
            .query_row(
                "SELECT theme, autostart, launch_minimized, grace_window_minutes, default_snooze_minutes
                 FROM settings
                 WHERE id = 1",
                [],
                decode_settings_row,
            )
            .optional()?
            .unwrap_or_default()
            .normalized();
        settings.validate()?;
        Ok(settings)
    }

    pub fn save_alarm(&self, alarm: &Alarm) -> Result<()> {
        let alarm = alarm.clone().normalized()?;
        let (repeat_rule_kind, repeat_rule_days) = encode_repeat_rule(&alarm.repeat_rule)?;
        self.conn.execute(
            "INSERT INTO alarms (
                id,
                label,
                time_local,
                repeat_rule_kind,
                repeat_rule_days,
                sound_id,
                volume,
                enabled,
                snooze_minutes,
                state,
                next_trigger_at,
                last_triggered_at,
                created_at,
                updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
             ON CONFLICT(id) DO UPDATE SET
                label = excluded.label,
                time_local = excluded.time_local,
                repeat_rule_kind = excluded.repeat_rule_kind,
                repeat_rule_days = excluded.repeat_rule_days,
                sound_id = excluded.sound_id,
                volume = excluded.volume,
                enabled = excluded.enabled,
                snooze_minutes = excluded.snooze_minutes,
                state = excluded.state,
                next_trigger_at = excluded.next_trigger_at,
                last_triggered_at = excluded.last_triggered_at,
                created_at = excluded.created_at,
                updated_at = excluded.updated_at",
            params![
                alarm.id.to_string(),
                alarm.label,
                alarm.time_local.format("%H:%M:%S").to_string(),
                repeat_rule_kind,
                repeat_rule_days,
                alarm.sound_id,
                i64::from(alarm.volume),
                bool_to_int(alarm.enabled),
                i64::from(alarm.snooze_minutes),
                encode_alarm_state(alarm.state),
                alarm.next_trigger_at.map(encode_datetime),
                alarm.last_triggered_at.map(encode_datetime),
                encode_datetime(alarm.created_at),
                encode_datetime(alarm.updated_at),
            ],
        )?;
        Ok(())
    }

    pub fn save_settings(&self, settings: &Settings) -> Result<()> {
        let settings = settings.clone().normalized();
        settings.validate()?;
        self.conn.execute(
            "INSERT INTO settings (
                id,
                theme,
                autostart,
                launch_minimized,
                grace_window_minutes,
                default_snooze_minutes
             ) VALUES (1, ?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET
                theme = excluded.theme,
                autostart = excluded.autostart,
                launch_minimized = excluded.launch_minimized,
                grace_window_minutes = excluded.grace_window_minutes,
                default_snooze_minutes = excluded.default_snooze_minutes",
            params![
                settings.theme,
                bool_to_int(settings.autostart),
                bool_to_int(settings.launch_minimized),
                i64::from(settings.grace_window_minutes),
                i64::from(settings.default_snooze_minutes),
            ],
        )?;
        Ok(())
    }

    pub fn delete_alarm(&self, id: AlarmId) -> Result<()> {
        self.conn
            .execute("DELETE FROM alarms WHERE id = ?1", params![id.to_string()])?;
        Ok(())
    }

    pub fn snapshot(
        &self,
        status: DaemonStatus,
        generated_at: DateTime<Local>,
    ) -> Result<AppSnapshot> {
        Ok(AppSnapshot {
            generated_at,
            alarms: self.load_alarms()?,
            status,
            settings: self.load_settings()?,
        })
    }
}

fn decode_alarm_row(row: &Row<'_>) -> rusqlite::Result<Alarm> {
    Ok(Alarm {
        id: parse_alarm_id(row.get::<_, String>(0)?)?,
        label: row.get(1)?,
        time_local: parse_time(row.get::<_, String>(2)?)?,
        repeat_rule: decode_repeat_rule(
            row.get::<_, String>(3)?,
            row.get::<_, Option<String>>(4)?,
        )?,
        sound_id: row.get(5)?,
        volume: row.get::<_, i64>(6)? as u8,
        enabled: int_to_bool(row.get::<_, i64>(7)?),
        snooze_minutes: row.get::<_, i64>(8)? as u16,
        state: decode_alarm_state(row.get::<_, String>(9)?)?,
        next_trigger_at: row
            .get::<_, Option<String>>(10)?
            .map(parse_datetime)
            .transpose()?,
        last_triggered_at: row
            .get::<_, Option<String>>(11)?
            .map(parse_datetime)
            .transpose()?,
        created_at: parse_datetime(row.get::<_, String>(12)?)?,
        updated_at: parse_datetime(row.get::<_, String>(13)?)?,
    })
}

fn decode_settings_row(row: &Row<'_>) -> rusqlite::Result<Settings> {
    Ok(Settings {
        theme: row.get(0)?,
        autostart: int_to_bool(row.get::<_, i64>(1)?),
        launch_minimized: int_to_bool(row.get::<_, i64>(2)?),
        grace_window_minutes: row.get::<_, i64>(3)? as u16,
        default_snooze_minutes: row.get::<_, i64>(4)? as u16,
    })
}

fn encode_repeat_rule(rule: &RepeatRule) -> Result<(String, Option<String>)> {
    match rule {
        RepeatRule::Once => Ok(("once".into(), None)),
        RepeatRule::Weekdays => Ok(("weekdays".into(), None)),
        RepeatRule::CustomDays(days) => Ok((
            "custom".into(),
            Some(serde_json::to_string(days).context("failed to encode repeat rule days")?),
        )),
    }
}

fn decode_repeat_rule(kind: String, days: Option<String>) -> rusqlite::Result<RepeatRule> {
    match kind.as_str() {
        "once" => Ok(RepeatRule::Once),
        "weekdays" => Ok(RepeatRule::Weekdays),
        "custom" => {
            let payload = days.unwrap_or_else(|| "[]".into());
            let parsed = serde_json::from_str(&payload).map_err(json_decode_error)?;
            Ok(RepeatRule::CustomDays(parsed))
        }
        other => Err(rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            format!("unknown repeat rule kind `{other}`").into(),
        )),
    }
}

fn encode_alarm_state(state: AlarmState) -> &'static str {
    match state {
        AlarmState::Idle => "idle",
        AlarmState::Scheduled => "scheduled",
        AlarmState::Ringing => "ringing",
        AlarmState::Snoozed => "snoozed",
        AlarmState::Missed => "missed",
    }
}

fn decode_alarm_state(state: String) -> rusqlite::Result<AlarmState> {
    match state.as_str() {
        "idle" => Ok(AlarmState::Idle),
        "scheduled" => Ok(AlarmState::Scheduled),
        "ringing" => Ok(AlarmState::Ringing),
        "snoozed" => Ok(AlarmState::Snoozed),
        "missed" => Ok(AlarmState::Missed),
        other => Err(rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            format!("unknown alarm state `{other}`").into(),
        )),
    }
}

fn encode_datetime(datetime: DateTime<Local>) -> String {
    datetime.with_timezone(&Utc).to_rfc3339()
}

fn parse_datetime(value: String) -> rusqlite::Result<DateTime<Local>> {
    let parsed = DateTime::parse_from_rfc3339(&value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, error.into())
    })?;
    Ok(parsed.with_timezone(&Local))
}

fn parse_time(value: String) -> rusqlite::Result<NaiveTime> {
    NaiveTime::parse_from_str(&value, "%H:%M:%S").map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, error.into())
    })
}

fn parse_alarm_id(value: String) -> rusqlite::Result<AlarmId> {
    value.parse().map_err(|error: uuid::Error| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, error.into())
    })
}

fn bool_to_int(value: bool) -> i64 {
    if value { 1 } else { 0 }
}

fn int_to_bool(value: i64) -> bool {
    value != 0
}

fn json_decode_error(error: serde_json::Error) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, error.into())
}

#[cfg(test)]
mod tests {
    use chrono::{Local, NaiveTime, TimeZone, Weekday};
    use tempfile::tempdir;

    use super::*;
    use crate::AlarmDraft;

    #[test]
    fn fresh_storage_starts_with_default_settings_and_no_demo_alarms() {
        let dir = tempdir().expect("tempdir");
        let paths = AuroraPaths::from_root(dir.path()).expect("paths");
        let storage = Storage::open(&paths).expect("storage");

        assert!(storage.load_alarms().expect("alarms").is_empty());
        assert_eq!(storage.load_settings().expect("settings").theme, "aurora");
    }

    #[test]
    fn migrates_legacy_json_blob_storage() {
        let dir = tempdir().expect("tempdir");
        let paths = AuroraPaths::from_root(dir.path()).expect("paths");
        let now = Local
            .with_ymd_and_hms(2026, 3, 19, 8, 0, 0)
            .single()
            .expect("fixed time");

        let conn = Connection::open(&paths.db_path).expect("legacy db");
        conn.execute_batch(
            "
            CREATE TABLE alarms (
                id TEXT PRIMARY KEY,
                json TEXT NOT NULL
            );
            CREATE TABLE settings (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                json TEXT NOT NULL
            );
            ",
        )
        .expect("legacy schema");

        let alarm = AlarmDraft {
            label: "Morning".into(),
            time_local: NaiveTime::from_hms_opt(7, 15, 0).expect("time"),
            repeat_rule: RepeatRule::CustomDays(vec![Weekday::Mon, Weekday::Wed]),
            ..AlarmDraft::default()
        }
        .into_alarm(now)
        .expect("alarm");
        let settings = Settings {
            theme: " aurora ".into(),
            ..Settings::default()
        };

        conn.execute(
            "INSERT INTO alarms (id, json) VALUES (?1, ?2)",
            params![
                alarm.id.to_string(),
                serde_json::to_string(&alarm).expect("alarm json")
            ],
        )
        .expect("insert alarm");
        conn.execute(
            "INSERT INTO settings (id, json) VALUES (1, ?1)",
            [serde_json::to_string(&settings).expect("settings json")],
        )
        .expect("insert settings");
        drop(conn);

        let storage = Storage::open(&paths).expect("migrated storage");
        let alarms = storage.load_alarms().expect("alarms");
        let loaded_settings = storage.load_settings().expect("settings");

        assert_eq!(alarms.len(), 1);
        assert_eq!(alarms[0].label, "Morning");
        assert_eq!(loaded_settings.theme, "aurora");
        assert!(storage.has_table("schema_meta").expect("schema meta"));
    }
}
