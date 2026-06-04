# oxmonad

Xmonad inspired window manager written in rust and wayland only.

## Building

oxmonad links against system libraries for rendering (`libpixman-1`) and
keyboard handling (`libxkbcommon`). The TTY/DRM backend (running directly on a
virtual terminal, see below) additionally needs the GPU, input, and session
libraries.

### Arch Linux

```sh
sudo pacman -S --needed pixman libxkbcommon mesa libdrm libinput seatd
```

| Library             | Arch package   | Needed for           |
| ------------------- | -------------- | -------------------- |
| `libpixman-1`       | `pixman`       | software rendering   |
| `libxkbcommon`      | `libxkbcommon` | keyboard handling     |
| `libgbm` / EGL/GLES | `mesa`         | TTY/DRM rendering    |
| `libdrm`            | `libdrm`       | TTY/DRM mode setting |
| `libinput`          | `libinput`     | TTY input            |
| `libseat`           | `seatd`        | TTY session/seat     |

Then build with:

```sh
cargo build
```

## Running

oxmonad has three backends. By default it auto-detects: if a host display is
available (`WAYLAND_DISPLAY` or `DISPLAY` is set) it runs **nested**, otherwise
it assumes a bare VT and uses the **TTY/DRM** backend. Override with
`OXMONAD_BACKEND=tty|winit|headless`.

### Nested (winit)

Run inside an existing X11 or Wayland session for development — oxmonad opens as
a window. Just run it from a terminal in your desktop session:

```sh
cargo run            # OXMONAD_BACKEND=winit
```

It prints the Wayland socket it created (`OXMONAD_WAYLAND_DISPLAY=wayland-N`);
point clients at it, e.g. `WAYLAND_DISPLAY=wayland-N foot`.

### On a TTY (DRM/KMS), like weston/sway

This drives the GPU directly with no host compositor. One-time setup:

```sh
sudo systemctl enable --now seatd     # session/seat manager
sudo usermod -aG seat "$USER"         # then log out and back in
```

Then switch to a bare virtual terminal (e.g. Ctrl+Alt+F3), log in, and run:

```sh
OXMONAD_BACKEND=tty ./target/debug/oxmonad
```

It takes over the screen, scanning out to every connected output. Launch
clients into it using the printed socket name, e.g.
`WAYLAND_DISPLAY=wayland-N foot`. VT switching (Ctrl+Alt+F-keys) pauses and
restores the session automatically.

This backend is intentionally minimal: a single GPU, no monitor hotplug.

### As a display-manager session (LightDM)

To pick oxmonad from a login greeter, install the session wrapper and Wayland
session entry under [`contrib/lightdm/`](contrib/lightdm/README.md). On a
logind system this needs no extra seat setup — the display manager hands
oxmonad an active seat.
