use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;

use adw::prelude::*;
use alarm_core::{
    Alarm, AlarmDraft, AlarmState, AlarmStatus, AppSnapshot, DBUS_SERVICE, RepeatRule,
};
use anyhow::Result;
use chrono::{Local, NaiveTime, Timelike, Weekday};
use gtk::glib;
use gtk::{Align, Orientation};
use ksni::Tray;
use zbus::blocking::Connection;

#[zbus::proxy(
    interface = "io.codex.AuroraAlarm",
    default_service = "io.codex.AuroraAlarm",
    default_path = "/io/codex/AuroraAlarm",
    gen_blocking = true
)]
trait AlarmDaemon {
    fn get_snapshot_json(&self) -> zbus::Result<String>;
    fn create_alarm_json(&self, draft_json: &str) -> zbus::Result<String>;
    fn update_alarm_json(&self, id: &str, alarm_json: &str) -> zbus::Result<String>;
    fn delete_alarm(&self, id: &str) -> zbus::Result<()>;
    fn toggle_alarm(&self, id: &str, enabled: bool) -> zbus::Result<String>;
    fn dismiss_active(&self) -> zbus::Result<()>;
    fn snooze_active(&self, minutes: u16) -> zbus::Result<()>;
}

#[derive(Debug)]
enum UiMessage {
    Snapshot(Result<AppSnapshot, String>),
    Command(Result<(), String>),
}

#[derive(Debug, Clone)]
enum TrayCommand {
    ShowWindow,
    DismissActive,
    SnoozeActive,
}

#[derive(Debug, Clone)]
enum TrayUpdate {
    Replace(TrayVisualState),
}

#[derive(Debug, Clone)]
struct TrayVisualState {
    title: String,
    status: AlarmStatus,
    next_items: Vec<String>,
    has_active_alarm: bool,
}

impl Default for TrayVisualState {
    fn default() -> Self {
        Self {
            title: "Aurora Alarm".into(),
            status: AlarmStatus::Quiet,
            next_items: vec!["No upcoming alarms".into()],
            has_active_alarm: false,
        }
    }
}

#[derive(Clone)]
struct Ui {
    app: adw::Application,
    window: adw::ApplicationWindow,
    hero_clock: gtk::Label,
    hero_status: gtk::Label,
    banner: gtk::Label,
    alarm_box: gtk::Box,
    label_entry: gtk::Entry,
    hour_spin: gtk::SpinButton,
    minute_spin: gtk::SpinButton,
    repeat_combo: gtk::DropDown,
    snooze_button: gtk::Button,
    dismiss_button: gtk::Button,
    status_line: gtk::Label,
    sender: mpsc::Sender<UiMessage>,
    tray_sender: mpsc::Sender<TrayUpdate>,
    snooze_minutes: Arc<Mutex<u16>>,
}

fn main() -> glib::ExitCode {
    adw::init().expect("failed to initialize libadwaita");

    let app = adw::Application::builder()
        .application_id("io.codex.aurora-alarm")
        .build();

    app.connect_activate(build_ui);
    app.run()
}

fn build_ui(app: &adw::Application) {
    load_css();

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .default_width(1120)
        .default_height(820)
        .title("Aurora Alarm")
        .build();

    let root = gtk::Box::builder()
        .orientation(Orientation::Vertical)
        .spacing(18)
        .margin_top(24)
        .margin_bottom(24)
        .margin_start(24)
        .margin_end(24)
        .build();

    let hero = gtk::Frame::builder().css_classes(vec!["hero-card"]).build();
    let hero_content = gtk::Box::builder()
        .orientation(Orientation::Vertical)
        .spacing(8)
        .margin_top(22)
        .margin_bottom(22)
        .margin_start(22)
        .margin_end(22)
        .build();
    let eyebrow = gtk::Label::builder()
        .label("Aurora Alarm")
        .halign(Align::Start)
        .css_classes(vec!["dim-label"])
        .build();
    let hero_clock = gtk::Label::builder()
        .label("00:00")
        .halign(Align::Start)
        .css_classes(vec!["display-clock"])
        .build();
    let hero_status = gtk::Label::builder()
        .label("Connecting to daemon...")
        .halign(Align::Start)
        .wrap(true)
        .build();
    let banner = gtk::Label::builder()
        .label("Persistent Linux alarms with tray-first controls.")
        .halign(Align::Start)
        .css_classes(vec!["banner-chip"])
        .build();
    hero_content.append(&eyebrow);
    hero_content.append(&hero_clock);
    hero_content.append(&hero_status);
    hero_content.append(&banner);
    hero.set_child(Some(&hero_content));

    let content = gtk::Box::builder()
        .orientation(Orientation::Horizontal)
        .spacing(18)
        .vexpand(true)
        .build();

    let left_column = gtk::Box::builder()
        .orientation(Orientation::Vertical)
        .spacing(16)
        .hexpand(true)
        .vexpand(true)
        .build();

    let right_column = gtk::Box::builder()
        .orientation(Orientation::Vertical)
        .spacing(16)
        .build();
    right_column.set_size_request(320, -1);

    let (alarm_section, alarm_shell) = section_frame("Upcoming Alarms");
    let alarm_box = gtk::Box::builder()
        .orientation(Orientation::Vertical)
        .spacing(12)
        .build();
    alarm_shell.append(&alarm_box);

    let (composer, composer_shell) = section_frame("Quick Add");
    let composer_box = gtk::Box::builder()
        .orientation(Orientation::Vertical)
        .spacing(12)
        .margin_top(18)
        .margin_bottom(18)
        .margin_start(18)
        .margin_end(18)
        .build();
    let label_entry = gtk::Entry::builder()
        .placeholder_text("Label")
        .text("Aurora Wake")
        .build();
    let time_row = gtk::Box::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .build();
    let hour_spin = gtk::SpinButton::with_range(0.0, 23.0, 1.0);
    let minute_spin = gtk::SpinButton::with_range(0.0, 59.0, 1.0);
    hour_spin.set_value(f64::from(Local::now().time().hour()));
    minute_spin.set_value(f64::from((Local::now().time().minute() + 5) % 60));
    let repeat_combo = gtk::DropDown::from_strings(&["Once", "Weekdays", "Every day"]);
    repeat_combo.set_selected(1);
    time_row.append(&hour_spin);
    time_row.append(&gtk::Label::new(Some(":")));
    time_row.append(&minute_spin);

    let add_button = gtk::Button::with_label("Add Alarm");
    add_button.add_css_class("suggested-action");
    let snooze_button = gtk::Button::with_label("Snooze 10m");
    let dismiss_button = gtk::Button::with_label("Dismiss Active");
    let action_row = gtk::Box::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .build();
    action_row.append(&snooze_button);
    action_row.append(&dismiss_button);

    let status_line = gtk::Label::builder()
        .label("Window ready.")
        .halign(Align::Start)
        .wrap(true)
        .build();

    composer_box.append(&label_entry);
    composer_box.append(&time_row);
    composer_box.append(&repeat_combo);
    composer_box.append(&add_button);
    composer_box.append(&action_row);
    composer_box.append(&status_line);
    composer_shell.append(&composer_box);

    left_column.append(&hero);
    left_column.append(&alarm_section);
    right_column.append(&composer);
    content.append(&left_column);
    content.append(&right_column);
    root.append(&content);

    let scroller = gtk::ScrolledWindow::builder()
        .hexpand(true)
        .vexpand(true)
        .child(&root)
        .build();

    window.set_content(Some(&scroller));
    window.connect_close_request(|window| {
        window.set_visible(false);
        glib::Propagation::Stop
    });
    window.present();

    let (sender, receiver) = mpsc::channel::<UiMessage>();
    let (tray_command_tx, tray_command_rx) = mpsc::channel::<TrayCommand>();
    let (tray_sender, tray_receiver) = mpsc::channel::<TrayUpdate>();
    start_tray_thread(tray_command_tx, tray_receiver);
    let snooze_minutes = Arc::new(Mutex::new(10));

    let ui = Ui {
        app: app.clone(),
        window,
        hero_clock,
        hero_status,
        banner,
        alarm_box,
        label_entry,
        hour_spin,
        minute_spin,
        repeat_combo,
        snooze_button: snooze_button.clone(),
        dismiss_button: dismiss_button.clone(),
        status_line,
        sender,
        tray_sender,
        snooze_minutes,
    };

    let receiver_ui = ui.clone();
    glib::timeout_add_local(Duration::from_millis(120), move || {
        while let Ok(message) = receiver.try_recv() {
            match message {
                UiMessage::Snapshot(result) => render_snapshot(&receiver_ui, result),
                UiMessage::Command(result) => match result {
                    Ok(()) => {
                        receiver_ui.status_line.set_label("Command applied.");
                        refresh(&receiver_ui);
                    }
                    Err(error) => receiver_ui
                        .status_line
                        .set_label(&format!("Command failed: {error}")),
                },
            }
        }

        while let Ok(command) = tray_command_rx.try_recv() {
            match command {
                TrayCommand::ShowWindow => {
                    receiver_ui.window.present();
                    receiver_ui.app.activate();
                }
                TrayCommand::DismissActive => dismiss_active(&receiver_ui),
                TrayCommand::SnoozeActive => snooze_active(&receiver_ui),
            }
        }

        glib::ControlFlow::Continue
    });

    let clock_ui = ui.clone();
    glib::timeout_add_local(Duration::from_secs(1), move || {
        let now = Local::now();
        clock_ui
            .hero_clock
            .set_label(&now.format("%H:%M:%S").to_string());
        glib::ControlFlow::Continue
    });

    let refresh_ui = ui.clone();
    glib::timeout_add_local(Duration::from_secs(5), move || {
        refresh(&refresh_ui);
        glib::ControlFlow::Continue
    });

    let add_ui = ui.clone();
    add_button.connect_clicked(move |_| {
        let draft = build_draft(&add_ui);
        add_ui.status_line.set_label("Saving new alarm...");
        let sender = add_ui.sender.clone();
        thread::spawn(move || {
            let result = (|| -> Result<()> {
                let connection = daemon_connection()?;
                let proxy = AlarmDaemonProxyBlocking::new(&connection)?;
                proxy.create_alarm_json(&serde_json::to_string(&draft)?)?;
                Ok(())
            })();
            let _ = sender.send(UiMessage::Command(result.map_err(|err| err.to_string())));
        });
    });

    let snooze_ui = ui.clone();
    snooze_button.connect_clicked(move |_| snooze_active(&snooze_ui));
    let dismiss_ui = ui.clone();
    dismiss_button.connect_clicked(move |_| dismiss_active(&dismiss_ui));

    refresh(&ui);
}

fn section_frame(title: &str) -> (gtk::Frame, gtk::Box) {
    let frame = gtk::Frame::builder()
        .css_classes(vec!["surface-card"])
        .build();
    let wrapper = gtk::Box::builder()
        .orientation(Orientation::Vertical)
        .spacing(12)
        .margin_top(18)
        .margin_bottom(18)
        .margin_start(18)
        .margin_end(18)
        .build();
    wrapper.append(
        &gtk::Label::builder()
            .label(title)
            .halign(Align::Start)
            .css_classes(vec!["heading"])
            .build(),
    );
    frame.set_child(Some(&wrapper));
    (frame, wrapper)
}

fn build_draft(ui: &Ui) -> AlarmDraft {
    let hour = ui.hour_spin.value_as_int().clamp(0, 23) as u32;
    let minute = ui.minute_spin.value_as_int().clamp(0, 59) as u32;
    let repeat_rule = match ui.repeat_combo.selected() {
        0 => RepeatRule::Once,
        2 => RepeatRule::CustomDays(vec![
            Weekday::Mon,
            Weekday::Tue,
            Weekday::Wed,
            Weekday::Thu,
            Weekday::Fri,
            Weekday::Sat,
            Weekday::Sun,
        ]),
        _ => RepeatRule::Weekdays,
    };

    AlarmDraft {
        label: if ui.label_entry.text().is_empty() {
            "Aurora Alarm".into()
        } else {
            ui.label_entry.text().to_string()
        },
        time_local: NaiveTime::from_hms_opt(hour, minute, 0).expect("valid UI time"),
        repeat_rule,
        sound_id: "default".into(),
        volume: 80,
        enabled: true,
        snooze_minutes: 10,
    }
}

fn refresh(ui: &Ui) {
    ui.hero_status.set_label("Refreshing daemon state...");
    let sender = ui.sender.clone();
    thread::spawn(move || {
        let result = fetch_snapshot().map_err(|error| error.to_string());
        let _ = sender.send(UiMessage::Snapshot(result));
    });
}

fn fetch_snapshot() -> Result<AppSnapshot> {
    let connection = daemon_connection()?;
    let json = match AlarmDaemonProxyBlocking::new(&connection)?.get_snapshot_json() {
        Ok(json) => json,
        Err(_) => {
            try_launch_daemon().ok();
            thread::sleep(Duration::from_millis(500));
            let retry_connection = daemon_connection()?;
            let retry_proxy = AlarmDaemonProxyBlocking::new(&retry_connection)?;
            retry_proxy.get_snapshot_json()?
        }
    };
    Ok(serde_json::from_str(&json)?)
}

fn dismiss_active(ui: &Ui) {
    ui.status_line.set_label("Sending command to daemon...");
    let sender = ui.sender.clone();
    thread::spawn(move || {
        let result = (|| -> Result<()> {
            let connection = daemon_connection()?;
            let proxy = AlarmDaemonProxyBlocking::new(&connection)?;
            proxy.dismiss_active()?;
            Ok(())
        })();
        let _ = sender.send(UiMessage::Command(result.map_err(|err| err.to_string())));
    });
}

fn snooze_active(ui: &Ui) {
    ui.status_line.set_label("Sending command to daemon...");
    let sender = ui.sender.clone();
    let minutes = ui.snooze_minutes.lock().map(|value| *value).unwrap_or(10);
    thread::spawn(move || {
        let result = (|| -> Result<()> {
            let connection = daemon_connection()?;
            let proxy = AlarmDaemonProxyBlocking::new(&connection)?;
            proxy.snooze_active(minutes)?;
            Ok(())
        })();
        let _ = sender.send(UiMessage::Command(result.map_err(|err| err.to_string())));
    });
}

fn render_snapshot(ui: &Ui, result: Result<AppSnapshot, String>) {
    clear_box(&ui.alarm_box);

    match result {
        Ok(snapshot) => {
            let degraded = degraded_capabilities(&snapshot);
            let next_copy = snapshot
                .status
                .next_alarm_at
                .map(|next| format!("Next alarm at {}", next.format("%a %H:%M")))
                .unwrap_or_else(|| "No alarm scheduled".into());
            let active_copy = snapshot
                .status
                .active_alarm
                .as_ref()
                .map(|active| format!("{} is {}", active.label, state_copy(active.state)))
                .unwrap_or_else(|| next_copy.clone());

            ui.hero_status.set_label(&active_copy);
            ui.banner.set_label(&build_banner_text(
                snapshot.alarms.len(),
                &next_copy,
                &degraded,
            ));
            ui.dismiss_button
                .set_sensitive(snapshot.status.active_alarm.is_some());
            ui.snooze_button
                .set_sensitive(snapshot.status.active_alarm.is_some());
            if let Ok(mut snooze_minutes) = ui.snooze_minutes.lock() {
                *snooze_minutes = snapshot.settings.default_snooze_minutes;
            }
            ui.snooze_button.set_label(&format!(
                "Snooze {}m",
                snapshot.settings.default_snooze_minutes
            ));

            let mut alarms = snapshot.alarms;
            alarms.sort_by_key(|alarm| alarm.next_trigger_at);
            for alarm in alarms.iter() {
                ui.alarm_box.append(&alarm_card(
                    ui,
                    alarm.clone(),
                    snapshot.status.active_alarm.is_some(),
                ));
            }

            if alarms.is_empty() {
                ui.alarm_box.append(
                    &gtk::Label::builder()
                        .label("No alarms yet. Add your first alarm from the Quick Add panel.")
                        .halign(Align::Start)
                        .build(),
                );
            }

            let tray_state = TrayVisualState {
                title: next_copy,
                status: snapshot.status.status,
                next_items: alarms
                    .iter()
                    .filter_map(|alarm| {
                        alarm
                            .next_trigger_at
                            .map(|next| format!("{} · {}", alarm.label, next.format("%a %H:%M")))
                    })
                    .take(3)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .collect(),
                has_active_alarm: snapshot.status.active_alarm.is_some(),
            };
            let _ = ui.tray_sender.send(TrayUpdate::Replace(tray_state));
        }
        Err(error) => {
            ui.hero_status
                .set_label(&format!("Daemon unavailable: {error}"));
            ui.banner
                .set_label("The app is running without its background daemon. Installed service mode is recommended for reliability.");
            ui.dismiss_button.set_sensitive(false);
            ui.snooze_button.set_sensitive(false);
            ui.alarm_box.append(
                &gtk::Label::builder()
                    .label("The background daemon is not reachable. Start `cargo run -p alarm-daemon` for development, or enable the installed `aurora-alarm-daemon.service` for normal use.")
                    .wrap(true)
                    .halign(Align::Start)
                    .build(),
            );
            let _ = ui.tray_sender.send(TrayUpdate::Replace(TrayVisualState {
                title: "Daemon offline".into(),
                status: AlarmStatus::Quiet,
                next_items: vec!["Daemon not reachable".into()],
                has_active_alarm: false,
            }));
        }
    }
}

fn build_banner_text(alarm_count: usize, next_copy: &str, degraded: &[String]) -> String {
    if degraded.is_empty() {
        format!("{alarm_count} alarms loaded. {next_copy}")
    } else {
        format!(
            "{alarm_count} alarms loaded. {next_copy} Degraded: {}.",
            degraded.join(", ")
        )
    }
}

fn degraded_capabilities(snapshot: &AppSnapshot) -> Vec<String> {
    let mut degraded = Vec::new();
    if !snapshot.status.notifications_available {
        degraded.push("desktop notifications unavailable".into());
    }
    if !snapshot.status.audio_available {
        degraded.push("audio output unavailable".into());
    }
    degraded
}

fn alarm_card(ui: &Ui, alarm: Alarm, _has_active_alarm: bool) -> gtk::Widget {
    let frame = gtk::Frame::builder()
        .css_classes(vec!["alarm-card"])
        .build();
    let content = gtk::Box::builder()
        .orientation(Orientation::Vertical)
        .spacing(10)
        .margin_top(16)
        .margin_bottom(16)
        .margin_start(16)
        .margin_end(16)
        .build();

    let head = gtk::Box::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .build();
    let label = gtk::Label::builder()
        .label(&alarm.label)
        .halign(Align::Start)
        .css_classes(vec!["heading"])
        .build();
    let time = gtk::Label::builder()
        .label(alarm.time_local.format("%H:%M").to_string())
        .halign(Align::End)
        .hexpand(true)
        .css_classes(vec!["time-pill"])
        .build();
    head.append(&label);
    head.append(&time);

    let next_line = gtk::Label::builder()
        .label(format!(
            "{} • {}",
            repeat_copy(&alarm.repeat_rule),
            alarm
                .next_trigger_at
                .map(|next| next.format("%a %d %b %H:%M").to_string())
                .unwrap_or_else(|| "Not scheduled".into())
        ))
        .halign(Align::Start)
        .wrap(true)
        .build();
    let state = gtk::Label::builder()
        .label(state_copy(alarm.state))
        .halign(Align::Start)
        .css_classes(vec!["dim-label"])
        .build();

    let actions = gtk::Box::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .build();
    let toggle = gtk::Button::with_label(if alarm.enabled { "Disable" } else { "Enable" });
    let delete = gtk::Button::with_label("Delete");
    actions.append(&toggle);
    actions.append(&delete);

    let toggle_ui = ui.clone();
    let toggle_alarm = alarm.clone();
    toggle.connect_clicked(move |_| {
        toggle_ui.status_line.set_label("Updating alarm...");
        let sender = toggle_ui.sender.clone();
        thread::spawn(move || {
            let result = (|| -> Result<()> {
                let connection = daemon_connection()?;
                let proxy = AlarmDaemonProxyBlocking::new(&connection)?;
                proxy.toggle_alarm(&toggle_alarm.id.to_string(), !toggle_alarm.enabled)?;
                Ok(())
            })();
            let _ = sender.send(UiMessage::Command(result.map_err(|err| err.to_string())));
        });
    });

    let delete_ui = ui.clone();
    let delete_alarm_id = alarm.id;
    delete.connect_clicked(move |_| {
        delete_ui.status_line.set_label("Deleting alarm...");
        let sender = delete_ui.sender.clone();
        thread::spawn(move || {
            let result = (|| -> Result<()> {
                let connection = daemon_connection()?;
                let proxy = AlarmDaemonProxyBlocking::new(&connection)?;
                proxy.delete_alarm(&delete_alarm_id.to_string())?;
                Ok(())
            })();
            let _ = sender.send(UiMessage::Command(result.map_err(|err| err.to_string())));
        });
    });

    content.append(&head);
    content.append(&next_line);
    content.append(&state);
    content.append(&actions);
    frame.set_child(Some(&content));
    frame.upcast()
}

fn repeat_copy(rule: &RepeatRule) -> String {
    match rule {
        RepeatRule::Once => "Once".into(),
        RepeatRule::Weekdays => "Weekdays".into(),
        RepeatRule::CustomDays(days) if days.len() == 7 => "Every day".into(),
        RepeatRule::CustomDays(days) => days
            .iter()
            .map(|day| format!("{day:?}"))
            .collect::<Vec<_>>()
            .join(", "),
    }
}

fn state_copy(state: AlarmState) -> &'static str {
    match state {
        AlarmState::Idle => "Idle",
        AlarmState::Scheduled => "Scheduled",
        AlarmState::Ringing => "Ringing",
        AlarmState::Snoozed => "Snoozed",
        AlarmState::Missed => "Missed",
    }
}

fn clear_box(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

fn daemon_connection() -> Result<Connection> {
    Ok(Connection::session()?)
}

fn try_launch_daemon() -> Result<()> {
    let current_exe = std::env::current_exe()?;
    let candidates = [
        current_exe.with_file_name("alarm-daemon"),
        current_exe
            .parent()
            .and_then(|dir| dir.parent())
            .map(|dir| dir.join("alarm-daemon"))
            .unwrap_or_else(|| current_exe.with_file_name("alarm-daemon")),
    ];

    for candidate in candidates {
        if candidate.exists() {
            Command::new(candidate)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()?;
            thread::sleep(Duration::from_millis(500));
            return Ok(());
        }
    }

    Ok(())
}

fn start_tray_thread(command_tx: mpsc::Sender<TrayCommand>, updates: mpsc::Receiver<TrayUpdate>) {
    thread::spawn(move || {
        use ksni::blocking::TrayMethods;

        let tray = AuroraTray {
            state: Arc::new(Mutex::new(TrayVisualState::default())),
            command_tx,
        };

        let Ok(handle) = tray.spawn() else {
            return;
        };

        while let Ok(update) = updates.recv() {
            match update {
                TrayUpdate::Replace(state) => {
                    let _ = handle.update(move |tray: &mut AuroraTray| {
                        if let Ok(mut tray_state) = tray.state.lock() {
                            *tray_state = state.clone();
                        }
                    });
                }
            }
        }
    });
}

struct AuroraTray {
    state: Arc<Mutex<TrayVisualState>>,
    command_tx: mpsc::Sender<TrayCommand>,
}

impl Tray for AuroraTray {
    fn id(&self) -> String {
        DBUS_SERVICE.into()
    }

    fn title(&self) -> String {
        self.state
            .lock()
            .map(|state| state.title.clone())
            .unwrap_or_else(|_| "Aurora Alarm".into())
    }

    fn icon_name(&self) -> String {
        let status = self
            .state
            .lock()
            .map(|state| state.status)
            .unwrap_or(AlarmStatus::Quiet);
        match status {
            AlarmStatus::Ringing => "alarm-symbolic".into(),
            AlarmStatus::Snoozed => "appointment-soon-symbolic".into(),
            AlarmStatus::Upcoming => "appointment-new-symbolic".into(),
            AlarmStatus::Quiet => "preferences-system-time-symbolic".into(),
        }
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::*;

        let snapshot = self
            .state
            .lock()
            .map(|state| state.clone())
            .unwrap_or_default();
        let mut items = vec![
            StandardItem {
                label: snapshot.title.clone(),
                enabled: false,
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
        ];

        items.extend(snapshot.next_items.into_iter().map(|item| {
            StandardItem {
                label: item,
                enabled: false,
                ..Default::default()
            }
            .into()
        }));

        items.push(MenuItem::Separator);

        if snapshot.has_active_alarm {
            let dismiss_tx = self.command_tx.clone();
            items.push(
                StandardItem {
                    label: "Dismiss Active".into(),
                    activate: Box::new(move |_| {
                        let _ = dismiss_tx.send(TrayCommand::DismissActive);
                    }),
                    ..Default::default()
                }
                .into(),
            );
            let snooze_tx = self.command_tx.clone();
            items.push(
                StandardItem {
                    label: "Snooze 10 Minutes".into(),
                    activate: Box::new(move |_| {
                        let _ = snooze_tx.send(TrayCommand::SnoozeActive);
                    }),
                    ..Default::default()
                }
                .into(),
            );
        }

        let show_tx = self.command_tx.clone();
        items.push(
            StandardItem {
                label: "Open Aurora Alarm".into(),
                activate: Box::new(move |_| {
                    let _ = show_tx.send(TrayCommand::ShowWindow);
                }),
                ..Default::default()
            }
            .into(),
        );

        items
    }

    fn status(&self) -> ksni::Status {
        let status = self
            .state
            .lock()
            .map(|state| state.status)
            .unwrap_or(AlarmStatus::Quiet);
        match status {
            AlarmStatus::Ringing => ksni::Status::NeedsAttention,
            _ => ksni::Status::Active,
        }
    }
}

fn load_css() {
    let provider = gtk::CssProvider::new();
    provider.load_from_data(
        "
        window {
            background: linear-gradient(155deg, #08111e 0%, #101f33 46%, #16293f 100%);
            color: #f3f7ff;
        }
        .hero-card, .surface-card, .alarm-card {
            background: rgba(7, 12, 21, 0.44);
            border-radius: 26px;
            border: 1px solid rgba(168, 212, 255, 0.12);
            box-shadow: 0 24px 60px rgba(2, 6, 14, 0.34);
        }
        .display-clock {
            font-size: 64px;
            font-weight: 700;
            letter-spacing: 2px;
        }
        .heading {
            font-size: 18px;
            font-weight: 700;
        }
        .dim-label {
            color: rgba(230, 238, 255, 0.65);
            letter-spacing: 0.16em;
            text-transform: uppercase;
        }
        .banner-chip {
            background: rgba(82, 168, 255, 0.16);
            border-radius: 999px;
            padding: 6px 12px;
        }
        .time-pill {
            color: #9fd2ff;
            font-weight: 700;
        }
        button {
            border-radius: 999px;
            padding: 8px 16px;
        }
        ",
    );

    gtk::style_context_add_provider_for_display(
        &gtk::gdk::Display::default().expect("display"),
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}
