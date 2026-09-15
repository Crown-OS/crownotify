# crownotify

The notification daemon for [CrownOS](https://github.com/Crown-OS). Implements
the freedesktop notification specification plus a CrownOS interface for rich
notification types — calls, music and chat.

It is the only real D-Bus surface in the CrownOS desktop, and the only repository
in the organization with integration tests.

**Status: Partial.** See [Known limitations](#known-limitations).

## Surface

| | |
|---|---|
| Layer | `Top` |
| Anchor | TOP, RIGHT, BOTTOM |
| Width | 400 |
| Namespace | `crownotify` |

## D-Bus

All on the **session** bus.

### Owned

| Name | Path |
|---|---|
| `org.freedesktop.Notifications` | `/org/freedesktop/Notifications` |
| `io.crownos.crownotify` | `/io/crownos/crownotify` |

Owning the freedesktop name means any application that raises a desktop
notification works with CrownOS out of the box. The CrownOS interface adds
`OpenNotificationCenter`, `CloseNotificationCenter` and
`Send{General,Call,Music,Chat}Notification`.

### Called

| Name | Methods |
|---|---|
| `io.crownos.crowncrate` | `PickupCall`, `DeclineCall` |

Pressing pickup or decline on a call toast calls through to
[`crowncrate-linux`](https://github.com/Crown-OS/crowncrate-linux).

> That daemon has **no D-Bus code at all** — the service does not exist yet.
> `crownotify` implements the calling side and tests it against a mock.

## Prerequisites

Any Wayland compositor supporting `wlr-layer-shell`, plus D-Bus.

Native dependencies (Arch):

```bash
sudo pacman -S --needed base-devel pkgconf dbus \
  wayland wayland-protocols libxkbcommon \
  vulkan-icd-loader mesa libglvnd fontconfig
```

Full list, including Debian/Ubuntu:
[Prerequisites](https://github.com/Crown-OS/crownos-documentations/blob/main/docs/10-getting-started/prerequisites.md).

## Build and run

> **You need the dev overlay first.** `crownotify` depends on
> `crownshell = "0.3"`, and 0.3 is **not published** — crates.io has only
> `crownshell` 0.1.0 and 0.2.0. A fresh clone fails at `cargo metadata` until
> Cargo is pointed at a local `crownshell` checkout.
>
> `crownos-setup`'s `./bootstrap.sh --dev` clones the repos side by side and
> writes a `[patch.crates-io]` overlay into a `.cargo/config.toml` one directory
> **above** them:
>
> ```
> ~/crownos/
> ├── .cargo/config.toml   # [patch.crates-io] crownshell = { path = "crownshell" }
> ├── crownotify/
> └── crownshell/
> ```
>
> Cargo walks up from the working directory to find that file, and the paths in
> it are relative to the file's own directory. No particular layout *inside* a
> repo is required.

```bash
cargo run
```

It will fail to start if another notification daemon already owns
`org.freedesktop.Notifications`.

### Tests

```bash
dbus-run-session -- cargo test -- --test-threads=1
```

Both parts matter. The tests register **real** well-known names on the session
bus, so they need one to exist, and they must not run concurrently — the internal
`OnceLock<Mutex<()>>` is per-binary and there are two test binaries. They also
conflict with a real `crownotify` already holding the name.

An ignored test runs against a real phone-bridge daemon rather than a mock:

```bash
cargo test -- --ignored real_crowncrate
```

## Threading

A `smol` thread hosts zbus. Notifications land in an
`Arc<Mutex<VecDeque<Notification>>>` inbox, and a `calloop::ping::Ping` wakes the
`crownshell` event loop and requests a frame — the pattern `crownshell`'s docs
recommend for getting a D-Bus signal onto the render thread.

## Known limitations

- **It ignores `notifications.ron` entirely**, including Do Not Disturb.
  `toggle_dnd()` exists and nothing calls it.
- **There is no notification centre.** `OpenNotificationCenter` and
  `CloseNotificationCenter` only log.
- **`CloseNotification` does nothing** and the three declared signals are never
  emitted.
- **`GetCapabilities` over-advertises** — it claims `action-icons`, `actions`,
  `body-images`, `sound` and `persistence`; none are implemented.
- **`models/audio.rs` and `models/display.rs` are empty structs**, though the
  `Notification` enum has variants for them.
- **Only `General` notifications expire.** Everything else lives until dismissed.
- `Notify` discards `app_icon` and `hints`.

## Contributing

See the organization-wide
[contribution guide](https://github.com/Crown-OS/crownos-documentations/blob/main/CONTRIBUTING.md).
Default branch here is **`main`**.

## License

Licensed under the [MIT License](LICENSE).
