# Running oxmonad as a LightDM session

oxmonad is a window manager for the [river](https://codeberg.org/river/river)
compositor (0.4 or later): river owns the display and input, oxmonad owns the
policy. These files register "river running oxmonad" as a selectable Wayland
session in a display manager such as LightDM.

| File              | Installs to                                   | Purpose                                         |
| ----------------- | --------------------------------------------- | ----------------------------------------------- |
| `oxmonad-session` | `/usr/local/bin/oxmonad-session`              | Session wrapper; runs `river -c oxmonad`.       |
| `oxmonad.desktop` | `/usr/share/wayland-sessions/oxmonad.desktop` | The session entry shown by the greeter.         |

## Install

The quick way — builds with the `deploy` profile and installs the binary, the
session wrapper and the greeter entry (asks for sudo):

```sh
deploy/install.sh            # or: deploy/install.sh /opt/oxmonad
```

It is safe to run while oxmonad is running: it then restarts the oxmonad of
the session it runs in (`oxmonad --restart`, like Mod+q), keeping every
window in place. Or step by step:

```sh
# river itself (Arch Linux).
sudo pacman -S --needed river

# Build and install oxmonad.
cargo build --release
sudo install -Dm755 target/release/oxmonad /usr/local/bin/oxmonad

# Install the session wrapper and the greeter entry.
sudo install -Dm755 contrib/lightdm/oxmonad-session /usr/local/bin/oxmonad-session
sudo install -Dm644 contrib/lightdm/oxmonad.desktop \
    /usr/share/wayland-sessions/oxmonad.desktop
```

Log out and pick **oxmonad** from the greeter's session menu.

## Notes

- **Seat access** is river's business: under a logind session started by the
  display manager it gets the GPU and input devices without further setup.
  Without logind, enable `seatd` and add yourself to the `seat` group.
- **Nothing starts automatically**: after logging in you get an empty desktop
  with the bar. Press **Super + Shift + Return** for a terminal (`alacritty` by
  default) or **Super + p** for the launcher (`tofi`). Install them first, or
  change `terminal` / the bindings in `~/.config/oxmonad/config.toml`.
- **Quitting**: **Super + Shift + q** ends the river session and returns to the
  greeter.
- **Logs**: oxmonad logs to stderr, which river passes on; check `journalctl`
  or `~/.cache/lightdm/`. Set `RUST_LOG=oxmonad=debug` in the wrapper for more.
