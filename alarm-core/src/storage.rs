use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::{DateTime, Local};
use directories::ProjectDirs;
use rusqlite::{Connection, params};

use crate::{
    Alarm, AlarmDraft, AlarmId, AlarmState, AppSnapshot, DaemonStatus, Settings,
    next_occurrence_after,
};

#[derive(Debug, Clone)]
pub struct AuroraPaths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub db_path: PathBuf,
}

impl AuroraPaths {
    pub fn discover() -> Result<Self> {
        let dirs = ProjectDirs::from("io", "codex", "Aurora Alarm")
            .context("failed to resolve XDG project directories")?;
        let config_dir = dirs.config_dir().to_path_buf();
        let data_dir = dirs.data_local_dir().to_path_buf();
        let db_path = data_dir.join("aurora-alarm.sqlite3");

        fs::create_dir_all(&config_dir).context("failed to create config directory")?;
        fs::create_dir_all(&data_dir).context("failed to create data directory")?;

        Ok(Self {
            config_dir,
            data_dir,
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
        storage.ensure_seed_data()?;
        Ok(storage)
    }

    fn migrate(&self) -> Result<()> {
        self.conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS alarms (
                id TEXT PRIMARY KEY,
                json TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS settings (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                json TEXT NOT NULL
            );
            ",
        )?;

        Ok(())
    }

    fn ensure_seed_data(&self) -> Result<()> {
        let settings_count: i64 =
            self.conn
                .query_row("SELECT COUNT(*) FROM settings", [], |row| row.get(0))?;
        if settings_count == 0 {
            self.save_settings(&Settings::default())?;
        }

        let alarm_count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM alarms", [], |row| row.get(0))?;
        if alarm_count == 0 {
            let now = Local::now();
            for draft in [
                AlarmDraft {
                    label: "Gentle Wake".into(),
                    ..AlarmDraft::default()
                },
                AlarmDraft {
                    label: "Tea Break".into(),
                    time_local: chrono::NaiveTime::from_hms_opt(15, 30, 0).expect("valid seed"),
                    repeat_rule: crate::RepeatRule::CustomDays(vec![
                        chrono::Weekday::Mon,
                        chrono::Weekday::Tue,
                        chrono::Weekday::Wed,
                        chrono::Weekday::Thu,
                        chrono::Weekday::Fri,
                    ]),
                    enabled: false,
                    ..AlarmDraft::default()
                },
            ] {
                let mut alarm = draft.into_alarm(now);
                alarm.next_trigger_at = next_occurrence_after(&alarm, now);
                alarm.state = if alarm.next_trigger_at.is_some() {
                    AlarmState::Scheduled
                } else {
                    AlarmState::Idle
                };
                self.save_alarm(&alarm)?;
            }
        }

        Ok(())
    }

    pub fn load_alarms(&self) -> Result<Vec<Alarm>> {
        let mut stmt = self.conn.prepare("SELECT json FROM alarms ORDER BY id")?;
        let alarms = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .map(|json| serde_json::from_str::<Alarm>(&json))
            .collect::<Result<Vec<_>, _>>()
            .context("failed to deserialize alarms")?;
        Ok(alarms)
    }

    pub fn load_settings(&self) -> Result<Settings> {
        let json = self
            .conn
            .query_row("SELECT json FROM settings WHERE id = 1", [], |row| {
                row.get::<_, String>(0)
            })?;
        Ok(serde_json::from_str(&json).context("failed to deserialize settings")?)
    }

    pub fn save_alarm(&self, alarm: &Alarm) -> Result<()> {
        let json = serde_json::to_string(alarm)?;
        self.conn.execute(
            "INSERT INTO alarms (id, json) VALUES (?1, ?2)
             ON CONFLICT(id) DO UPDATE SET json = excluded.json",
            params![alarm.id.to_string(), json],
        )?;
        Ok(())
    }

    pub fn save_settings(&self, settings: &Settings) -> Result<()> {
        let json = serde_json::to_string(settings)?;
        self.conn.execute(
            "INSERT INTO settings (id, json) VALUES (1, ?1)
             ON CONFLICT(id) DO UPDATE SET json = excluded.json",
            params![json],
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
