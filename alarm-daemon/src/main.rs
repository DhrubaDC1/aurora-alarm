use std::future::pending;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use alarm_core::{
    ActiveAlarm, Alarm, AlarmDraft, AlarmId, AlarmState, AlarmStatus, AppSnapshot, AuroraPaths,
    DBUS_PATH, DBUS_SERVICE, DaemonStatus, Storage, describe_next_alarm, next_occurrence_after,
};
use anyhow::{Context, Result};
use chrono::{DateTime, Local};
use notify_rust::Notification;
use rodio::{
    OutputStreamBuilder, Sink,
    source::{SineWave, Source},
};
use tokio::sync::Mutex;
use tokio::time::{MissedTickBehavior, interval};
use zbus::{connection, interface};

#[derive(Default)]
struct RuntimeState {
    active_alarm: Option<ActiveAlarm>,
    tone_stop: Option<Arc<AtomicBool>>,
    notifications_available: bool,
}

#[derive(Clone)]
struct AlarmService {
    paths: AuroraPaths,
    runtime: Arc<Mutex<RuntimeState>>,
}

#[interface(name = "io.codex.AuroraAlarm")]
impl AlarmService {
    async fn get_snapshot_json(&self) -> zbus::fdo::Result<String> {
        let alarms = self.with_storage(|storage| storage.load_alarms()).map_err(to_dbus_error)?;
        let settings = self
            .with_storage(|storage| storage.load_settings())
            .map_err(to_dbus_error)?;
        let status = self.current_status(&alarms).await.map_err(to_dbus_error)?;
        let snapshot = AppSnapshot {
            generated_at: Local::now(),
            alarms,
            status,
            settings,
        };
        serde_json::to_string(&snapshot).map_err(to_dbus_error)
    }

    fn create_alarm_json(&self, draft_json: &str) -> zbus::fdo::Result<String> {
        let draft = serde_json::from_str::<AlarmDraft>(draft_json).map_err(to_dbus_error)?;
        self.with_storage(|storage| {
            let mut alarm = draft.into_alarm(Local::now());
            alarm.next_trigger_at = next_occurrence_after(&alarm, Local::now());
            alarm.state = if alarm.next_trigger_at.is_some() {
                AlarmState::Scheduled
            } else {
                AlarmState::Idle
            };
            storage.save_alarm(&alarm)?;
            Ok(serde_json::to_string(&alarm)?)
        })
        .map_err(to_dbus_error)
    }

    fn update_alarm_json(&self, id: &str, alarm_json: &str) -> zbus::fdo::Result<String> {
        let alarm_id = parse_alarm_id(id).map_err(to_dbus_error)?;
        let mut updated = serde_json::from_str::<Alarm>(alarm_json).map_err(to_dbus_error)?;
        updated.id = alarm_id;
        updated.updated_at = Local::now();
        updated.next_trigger_at = next_occurrence_after(&updated, Local::now());
        updated.state = if updated.state == AlarmState::Ringing {
            AlarmState::Ringing
        } else if updated.next_trigger_at.is_some() && updated.enabled {
            AlarmState::Scheduled
        } else {
            AlarmState::Idle
        };

        self.with_storage(|storage| {
            storage.save_alarm(&updated)?;
            Ok(serde_json::to_string(&updated)?)
        })
        .map_err(to_dbus_error)
    }

    fn delete_alarm(&self, id: &str) -> zbus::fdo::Result<()> {
        let alarm_id = parse_alarm_id(id).map_err(to_dbus_error)?;
        self.with_storage(|storage| storage.delete_alarm(alarm_id))
            .map_err(to_dbus_error)
    }

    async fn toggle_alarm(&self, id: &str, enabled: bool) -> zbus::fdo::Result<String> {
        let alarm_id = parse_alarm_id(id).map_err(to_dbus_error)?;
        let runtime = self.runtime.clone();
        self.with_storage(|storage| {
            let mut alarm = storage
                .load_alarms()?
                .into_iter()
                .find(|alarm| alarm.id == alarm_id)
                .context("alarm not found")?;
            alarm.enabled = enabled;
            alarm.updated_at = Local::now();
            alarm.state = AlarmState::Idle;
            alarm.next_trigger_at = next_occurrence_after(&alarm, Local::now());
            if alarm.next_trigger_at.is_some() && alarm.enabled {
                alarm.state = AlarmState::Scheduled;
            }
            storage.save_alarm(&alarm)?;
            Ok(alarm)
        })
        .map(|alarm| async move {
            if !enabled {
                let mut runtime = runtime.lock().await;
                if runtime
                    .active_alarm
                    .as_ref()
                    .is_some_and(|active| active.alarm_id == alarm.id)
                {
                    stop_audio(&mut runtime);
                    runtime.active_alarm = None;
                }
            }
            serde_json::to_string(&alarm).map_err(anyhow::Error::from)
        })
        .map_err(to_dbus_error)?
        .await
        .map_err(to_dbus_error)
    }

    async fn dismiss_active(&self) -> zbus::fdo::Result<()> {
        let active = { self.runtime.lock().await.active_alarm.clone() };
        let Some(active) = active else {
            return Ok(());
        };

        self.with_storage(|storage| {
            if let Some(mut alarm) = storage
                .load_alarms()?
                .into_iter()
                .find(|alarm| alarm.id == active.alarm_id)
            {
                alarm.last_triggered_at = Some(Local::now());
                alarm.state = AlarmState::Idle;
                alarm.next_trigger_at = next_occurrence_after(&alarm, Local::now());
                if alarm.next_trigger_at.is_some() && alarm.enabled {
                    alarm.state = AlarmState::Scheduled;
                }
                storage.save_alarm(&alarm)?;
            }
            Ok(())
        })
        .map_err(to_dbus_error)?;

        let mut runtime = self.runtime.lock().await;
        stop_audio(&mut runtime);
        runtime.active_alarm = None;
        Ok(())
    }

    async fn snooze_active(&self, minutes: u16) -> zbus::fdo::Result<()> {
        let active = { self.runtime.lock().await.active_alarm.clone() };
        let Some(active) = active else {
            return Ok(());
        };

        let snoozed_until = Local::now() + chrono::Duration::minutes(i64::from(minutes.max(1)));

        self.with_storage(|storage| {
            if let Some(mut alarm) = storage
                .load_alarms()?
                .into_iter()
                .find(|alarm| alarm.id == active.alarm_id)
            {
                alarm.state = AlarmState::Snoozed;
                alarm.next_trigger_at = Some(snoozed_until);
                alarm.updated_at = Local::now();
                storage.save_alarm(&alarm)?;
            }
            Ok(())
        })
        .map_err(to_dbus_error)?;

        let mut runtime = self.runtime.lock().await;
        stop_audio(&mut runtime);
        runtime.active_alarm = Some(ActiveAlarm {
            alarm_id: active.alarm_id,
            label: active.label,
            state: AlarmState::Snoozed,
            due_at: active.due_at,
            snoozed_until: Some(snoozed_until),
        });
        Ok(())
    }
}

impl AlarmService {
    fn with_storage<T>(&self, f: impl FnOnce(&Storage) -> Result<T>) -> Result<T> {
        let storage = Storage::open(&self.paths)?;
        f(&storage)
    }

    async fn current_status(&self, alarms: &[Alarm]) -> Result<DaemonStatus> {
        let runtime = self.runtime.lock().await;
        let next_alarm_at = alarms
            .iter()
            .filter(|alarm| alarm.enabled)
            .filter_map(|alarm| alarm.next_trigger_at)
            .min();
        let status = match runtime.active_alarm.as_ref().map(|active| active.state) {
            Some(AlarmState::Ringing) => AlarmStatus::Ringing,
            Some(AlarmState::Snoozed) => AlarmStatus::Snoozed,
            _ if next_alarm_at.is_some() => AlarmStatus::Upcoming,
            _ => AlarmStatus::Quiet,
        };

        Ok(DaemonStatus {
            status,
            next_alarm_at,
            active_alarm: runtime.active_alarm.clone(),
            tray_available: false,
            notifications_available: runtime.notifications_available,
        })
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let paths = AuroraPaths::discover()?;
    let runtime = Arc::new(Mutex::new(RuntimeState {
        notifications_available: true,
        ..RuntimeState::default()
    }));
    let service = AlarmService {
        paths: paths.clone(),
        runtime: runtime.clone(),
    };

    tokio::spawn(run_scheduler(paths, runtime));

    let _conn = connection::Builder::session()?
        .name(DBUS_SERVICE)?
        .serve_at(DBUS_PATH, service)?
        .build()
        .await?;

    install_user_service_file().ok();
    pending::<()>().await;
    Ok(())
}

async fn run_scheduler(paths: AuroraPaths, runtime: Arc<Mutex<RuntimeState>>) {
    let mut ticker = interval(Duration::from_secs(1));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);

    loop {
        ticker.tick().await;
        if let Err(error) = scheduler_tick(&paths, &runtime).await {
            eprintln!("scheduler tick failed: {error}");
        }
    }
}

async fn scheduler_tick(paths: &AuroraPaths, runtime: &Arc<Mutex<RuntimeState>>) -> Result<()> {
    let storage = Storage::open(paths)?;
    let settings = storage.load_settings()?;
    let now = Local::now();
    let alarms = storage.load_alarms()?;
    let mut changed = Vec::new();
    let mut fired_alarm: Option<Alarm> = None;

    for mut alarm in alarms {
        let original = serde_json::to_string(&alarm)?;
        let was_ringing = alarm.state == AlarmState::Ringing;

        refresh_alarm_schedule(&mut alarm, now);

        if alarm.enabled
            && alarm.next_trigger_at.is_some_and(|next| {
                next <= now
                    && now - next
                        <= chrono::Duration::minutes(i64::from(settings.grace_window_minutes))
            })
            && !was_ringing
        {
            alarm.state = AlarmState::Ringing;
            alarm.last_triggered_at = Some(now);
            alarm.next_trigger_at = Some(now);
            fired_alarm = Some(alarm.clone());
        } else if alarm.enabled
            && alarm.next_trigger_at.is_some_and(|next| {
                next < now
                    && now - next
                        > chrono::Duration::minutes(i64::from(settings.grace_window_minutes))
            })
            && !was_ringing
        {
            alarm.state = AlarmState::Missed;
            alarm.next_trigger_at = next_occurrence_after(&alarm, now);
            if alarm.next_trigger_at.is_some() && alarm.enabled {
                alarm.state = AlarmState::Scheduled;
            }
        }

        if serde_json::to_string(&alarm)? != original {
            storage.save_alarm(&alarm)?;
            changed.push(alarm);
        }
    }

    if let Some(alarm) = fired_alarm {
        trigger_alarm(runtime.clone(), alarm, now).await;
    } else {
        let runtime_guard = runtime.lock().await;
        if runtime_guard
            .active_alarm
            .as_ref()
            .is_some_and(|active| active.state == AlarmState::Snoozed)
            && runtime_guard
                .active_alarm
                .as_ref()
                .and_then(|active| active.snoozed_until)
                .is_some_and(|until| until <= now)
        {
            if let Some(active) = runtime_guard.active_alarm.clone() {
                drop(runtime_guard);
                if let Ok(storage) = Storage::open(paths) {
                    if let Ok(Some(alarm)) = storage
                        .load_alarms()
                        .map(|alarms| alarms.into_iter().find(|alarm| alarm.id == active.alarm_id))
                    {
                        trigger_alarm(runtime.clone(), alarm, now).await;
                    }
                }
            }
        }
    }

    if !changed.is_empty() {
        let storage = Storage::open(paths)?;
        let status = {
            let next_alarm_at = storage
                .load_alarms()?
                .iter()
                .filter(|alarm| alarm.enabled)
                .filter_map(|alarm| alarm.next_trigger_at)
                .min();
            let runtime = runtime.lock().await;
            DaemonStatus {
                status: match runtime.active_alarm.as_ref().map(|active| active.state) {
                    Some(AlarmState::Ringing) => AlarmStatus::Ringing,
                    Some(AlarmState::Snoozed) => AlarmStatus::Snoozed,
                    _ if next_alarm_at.is_some() => AlarmStatus::Upcoming,
                    _ => AlarmStatus::Quiet,
                },
                next_alarm_at,
                active_alarm: runtime.active_alarm.clone(),
                tray_available: false,
                notifications_available: runtime.notifications_available,
            }
        };
        let _snapshot: AppSnapshot = storage.snapshot(status, Local::now())?;
    }

    Ok(())
}

fn refresh_alarm_schedule(alarm: &mut Alarm, now: DateTime<Local>) {
    if !alarm.enabled {
        alarm.state = AlarmState::Idle;
        alarm.next_trigger_at = None;
        return;
    }

    if alarm.state == AlarmState::Ringing {
        return;
    }

    if alarm.state == AlarmState::Snoozed && alarm.next_trigger_at.is_some_and(|next| next > now) {
        return;
    }

    alarm.next_trigger_at = next_occurrence_after(alarm, now);
    alarm.state = if alarm.next_trigger_at.is_some() {
        AlarmState::Scheduled
    } else {
        AlarmState::Idle
    };
}

async fn trigger_alarm(runtime: Arc<Mutex<RuntimeState>>, alarm: Alarm, now: DateTime<Local>) {
    {
        let mut runtime = runtime.lock().await;
        if runtime.active_alarm.as_ref().is_some_and(|active| {
            active.alarm_id == alarm.id && active.state == AlarmState::Ringing
        }) {
            return;
        }

        stop_audio(&mut runtime);
        let stop_flag = Arc::new(AtomicBool::new(false));
        spawn_audio_loop(stop_flag.clone(), alarm.volume);
        runtime.tone_stop = Some(stop_flag);
        runtime.active_alarm = Some(ActiveAlarm {
            alarm_id: alarm.id,
            label: alarm.label.clone(),
            state: AlarmState::Ringing,
            due_at: now,
            snoozed_until: None,
        });
    }

    let subtitle = describe_next_alarm(&alarm, now).unwrap_or_else(|| "Now".into());
    let _ = Notification::new()
        .summary(&format!("Aurora Alarm: {}", alarm.label))
        .body(&format!(
            "Alarm is ringing at {subtitle}. Dismiss or snooze from the app or tray."
        ))
        .show();
}

fn spawn_audio_loop(stop_flag: Arc<AtomicBool>, volume: u8) {
    thread::spawn(move || {
        let Ok(mut stream) = OutputStreamBuilder::open_default_stream() else {
            return;
        };
        stream.log_on_drop(false);
        let sink = Sink::connect_new(stream.mixer());
        sink.set_volume((volume as f32 / 100.0).clamp(0.1, 1.0));

        while !stop_flag.load(Ordering::Relaxed) {
            sink.append(
                SineWave::new(660.0)
                    .take_duration(Duration::from_millis(280))
                    .amplify(0.20),
            );
            sink.append(
                SineWave::new(880.0)
                    .take_duration(Duration::from_millis(280))
                    .amplify(0.25),
            );
            sink.sleep_until_end();
            thread::sleep(Duration::from_millis(180));
        }
    });
}

fn stop_audio(runtime: &mut RuntimeState) {
    if let Some(stop) = runtime.tone_stop.take() {
        stop.store(true, Ordering::Relaxed);
    }
}

fn parse_alarm_id(id: &str) -> Result<AlarmId> {
    Ok(id.parse()?)
}

fn to_dbus_error(error: impl Into<anyhow::Error>) -> zbus::fdo::Error {
    zbus::fdo::Error::Failed(error.into().to_string())
}

fn install_user_service_file() -> Result<()> {
    let paths = AuroraPaths::discover()?;
    let service_dir = paths.config_dir.join("systemd/user");
    std::fs::create_dir_all(&service_dir)?;
    let service_path = service_dir.join("aurora-alarm-daemon.service");
    let current_exe = std::env::current_exe().context("missing daemon executable path")?;
    let service = format!(
        "[Unit]\nDescription=Aurora Alarm Daemon\n\n[Service]\nExecStart={}\nRestart=on-failure\n\n[Install]\nWantedBy=default.target\n",
        current_exe.display()
    );
    std::fs::write(service_path, service)?;
    Ok(())
}
