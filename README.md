# Aurora Alarm

Aurora Alarm is a native Linux alarm clock with a persistent user-session daemon,
a GTK/libadwaita control window, and tray-first desktop integration.

## Features

- native GTK4/libadwaita interface with a large clock and quick-add alarm flow
- persistent background daemon over D-Bus, so alarms continue after the window closes
- StatusNotifier/AppIndicator-compatible tray menu for opening the app, snoozing, and dismissing
- recurring alarms, weekday alarms, one-off alarms, and local SQLite storage
- desktop notifications and built-in tone playback

## Desktop Support

Aurora Alarm is designed for Linux desktops that support GTK4 plus a session bus.

- KDE Plasma: expected best tray experience
- GNOME: works best when AppIndicator support is present
- XFCE, Cinnamon, and similar desktops: tray behavior depends on the panel/tray host
- no tray host: the window and daemon still work, but the top-bar integration falls back

## Requirements

Build requirements:

- Rust toolchain with `cargo`
- `pkg-config`
- GTK4 development files
- libadwaita development files
- ALSA development files
- a running D-Bus session

Examples:

Arch Linux:

```bash
sudo pacman -S rustup pkgconf gtk4 libadwaita alsa-lib
rustup default stable
```

Ubuntu / Debian:

```bash
sudo apt update
sudo apt install build-essential curl pkg-config libgtk-4-dev libadwaita-1-dev libasound2-dev
curl https://sh.rustup.rs -sSf | sh
```

## Workspace

- `alarm-core`: domain types, recurrence logic, and SQLite persistence
- `alarm-daemon`: D-Bus service, scheduler loop, notifications, and tray state
- `alarm-app`: GTK/libadwaita window that talks to the daemon over D-Bus

## Install From Source

```bash
git clone https://github.com/YOUR_GITHUB_USERNAME/aurora-alarm.git
cd aurora-alarm
cargo build --release
mkdir -p ~/.local/bin
install -m755 target/release/alarm-daemon ~/.local/bin/alarm-daemon
install -m755 target/release/alarm-app ~/.local/bin/alarm-app
```

If `~/.local/bin` is not already on your `PATH`, add it in your shell profile.

## Run Manually

```bash
cargo run -p alarm-daemon
```

In another terminal:

```bash
cargo run -p alarm-app
```

## Enable As User Service

A template user service is included at
[`dist/systemd/aurora-alarm-daemon.service`](./dist/systemd/aurora-alarm-daemon.service).
Adjust the `ExecStart=` path for your installation location and install it with:

```bash
mkdir -p ~/.config/systemd/user
cp dist/systemd/aurora-alarm-daemon.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now aurora-alarm-daemon.service
```

Then launch the UI whenever you want:

```bash
~/.local/bin/alarm-app
```

## Data Locations

- config/service files: XDG config directory, typically `~/.config`
- alarm database: XDG data directory, typically `~/.local/share`

## Notes

- The daemon stores state under the user's XDG data/config directories.
- Tray support uses StatusNotifier/AppIndicator-compatible hosts when available.
- If no tray host exists, the app still works through the window and notifications.
- The current build uses a generated tone rather than bundled audio assets.
