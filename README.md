# Crownotify

Crownotify is a beautiful notification server for CrownOS and other Wayland desktops built on [CrownUI](https://crates.io/crates/crownui).

## Features

- **Rich cards.** Separate card designs for general notifications, progress, incoming calls, chat messages with inline replies, and now-playing music.
- **Interruption levels.** `passive`, `active`, `time-sensitive` and `critical`, taken from the freedesktop urgency or set directly by apps that know about CrownOS. Critical notifications always get through.
- **Per-app overrides.** Mute an app, force its level or timeout, turn grouping on or off, or give it its own accent colour.
- **App profiles.** Known apps such as Spotify get the right card and brand colour without having to ask for them.
- **Relayed notifications.** Notifications mirrored from a phone or tablet carry a device badge, and call actions go back to the phone through crownconnect.
- **Click to launch.** Clicking a notification from an app that isn't listening opens the app from its desktop entry.
- **Fault tolerant.** A malformed hint or config file is logged and ignored. If another server already owns the D-Bus name, CrownOS's own notifications keep working over crownos-ipc.

## Installation

### Arch Linux

Available in the [GitHub release](https://github.com/Crown-OS/crownotify/releases):

```sh
sudo pacman -U crownotify-*.pkg.tar.zst
```

It will be on the AUR soon. That is delayed because [AUR](https://aur.archlinux.org/register) registration is down.

### Build from source

Requires a recent stable Rust toolchain (edition 2024).

```sh
cargo build --release
install -Dm755 target/release/crownotify ~/.local/bin/crownotify
```

Clients that only need the wire protocol can depend on the library with `default-features = false`. That pulls in `crownotify::proto` without the daemon's rendering and D-Bus stack:

```toml
[dependencies]
crownotify = { git = "https://github.com/Crown-OS/crownotify", default-features = false }
```

## Run

Stop any other notification daemon (mako, dunst, swaync, …), then start crownotify from your compositor's autostart:

```sh
crownotify
```

Logging uses `env_logger` and defaults to `info`:

```sh
RUST_LOG=crownotify=debug crownotify
```

Check that it works:

```sh
notify-send "Hello" "from crownotify"
```

## Configuration

Crownotify reads `$XDG_CONFIG_HOME/crownotify/config.toml` (usually `~/.config/crownotify/config.toml`) once at startup. Every field is optional. A missing file means all defaults are used. A malformed file logs a warning and also falls back to the defaults.

```toml
appearance = "auto"          # auto | light | dark
default_timeout_ms = 5000
# critical_timeout_ms = 30000  # unset: critical notifications never expire
max_toasts = 5
max_history = 100

# Start in a focus mode. `do_not_disturb = true` does the same for the preset.
focus_mode = "work"

[focus.work]
minimum_level = "time-sensitive"   # passive | active | time-sensitive | critical
allowed_apps = ["thunderbird"]     # let through whatever their level
blocked = "queue"                  # queue: deliver when the mode ends; file: center only

# Overrides the built-in preset, which files everything below critical.
[focus.do-not-disturb]
blocked = "queue"

[apps.spotify]
muted = true                 # keep in the center, never toast
grouped = false
accent = "#1DB954"

[apps."org.signal.signal"]
level = "time-sensitive"
timeout_ms = 20000
```

App keys are the app's desktop entry or name, lower-cased.

## Sending notifications

### Over D-Bus

Any freedesktop client works. These CrownOS hints unlock the rich cards:

| Hint                                             | Effect                                                                                       |
| ------------------------------------------------ | -------------------------------------------------------------------------------------------- |
| `category` = `x-crownos.call`                    | Call card. Summary is the contact, body is the number. `x-crownos.call-id` sets the call id. |
| `category` = `x-crownos.chat`                    | Chat card with inline reply. `x-crownos.conversation-id` sets the thread.                    |
| `category` = `x-crownos.music`                   | Now-playing card. Summary is the title, body is the artist. `x-crownos.playing` is a bool.   |
| `value` (int 0–100)                              | Progress bar, or the music position.                                                         |
| `x-crownos.interruption-level`                   | `passive`, `active`, `time-sensitive` or `critical`. Takes precedence over `urgency`.        |
| `x-crownos.device-kind`, `x-crownos.device-name` | Marks the notification as relayed from another device.                                       |

```sh
notify-send -h string:category:x-crownos.music -h int:value:40 "Song" "Artist"
notify-send -h string:x-crownos.interruption-level:time-sensitive "Meeting" "Starts in 5 minutes"
```

### Over crownos-ipc

The CrownOS shell talks to the `crownotify` service defined in [`src/proto.rs`](src/proto.rs). It covers:

- **Commands:** open, close and toggle the center; set Do Not Disturb or a focus mode; dismiss all; invoke an action; close a notification.
- **Posting:** typed calls to post general, call, music, chat, app-profile and relayed notifications.
- **Events:** `NotificationClosed`, `ActionInvoked`, `ReplySubmitted`, `CenterVisibility`, `DoNotDisturbChanged` and `FocusModeChanged`.

## Development

```sh
cargo fmt -- --check && cargo clippy -- -D warnings && cargo test
```

Render every card kind offscreen to a PNG, with no compositor needed:

```sh
cargo run --example render -- /tmp/crownotify toasts   # or: center
```

## Dependencies

- A Wayland compositor with `wlr-layer-shell`, `wp_viewporter` and `linux-dmabuf` like [Hyprland](https://hypr.land/), [Sway](https://swaywm.org/), [Niri](https://github.com/niri-wm/niri)
