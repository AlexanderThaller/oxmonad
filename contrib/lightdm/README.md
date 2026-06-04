# Running oxmonad as a LightDM session

These files register oxmonad as a selectable Wayland session in a display
manager such as LightDM.

| File             | Installs to                              | Purpose                                  |
| ---------------- | ---------------------------------------- | ---------------------------------------- |
| `oxmonad-session`| `/usr/local/bin/oxmonad-session`         | Session wrapper; forces the TTY backend. |
| `oxmonad.desktop`| `/usr/share/wayland-sessions/oxmonad.desktop` | The session entry shown by the greeter. |

## Install

```sh
# Build and install the compositor.
cargo build --release
sudo install -Dm755 target/release/oxmonad /usr/local/bin/oxmonad

# Install the session wrapper and the greeter entry.
sudo install -Dm755 contrib/lightdm/oxmonad-session /usr/local/bin/oxmonad-session
sudo install -Dm644 contrib/lightdm/oxmonad.desktop \
    /usr/share/wayland-sessions/oxmonad.desktop
```

Log out and pick **oxmonad** from the greeter's session menu.

## Notes

- **Seat access**: when LightDM logs you in via PAM/logind, it creates a logind
  session that owns the seat and active VT, and libseat gets GPU/input access
  automatically. The `seatd` + `seat`-group setup from the bare-VT instructions
  in the top-level README is **not** needed on a logind system. (Without
  logind, enable `seatd` and add yourself to the `seat` group instead.)
- **You need a client to launch**: oxmonad has no autostart yet. After logging
  in you get an empty desktop; press **Super + Shift + Return** to open the
  terminal (`alacritty` by default — see `src/config.rs`). Install it (and
  `tofi` for the launcher) before logging in, or change those constants and
  rebuild.
- **If LightDM misbehaves**: its Wayland-session support is workable but
  X-centric. Check `journalctl` / `~/.cache/lightdm/` and confirm oxmonad logs
  `opening primary GPU` and `DRM output online`. [greetd](https://git.sr.ht/~kennylevinsen/greetd)
  is a simpler alternative for launching Wayland compositors.
