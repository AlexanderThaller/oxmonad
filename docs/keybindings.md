# Keybindings

oxmonad's shortcuts are hardcoded in [`src/keybindings.rs`](src/keybindings.rs);
the program choices they spawn live in [`src/config.rs`](src/config.rs).

Every binding uses the **mod key** (`META`), which defaults to **Super** (the
"logo"/Windows key). Change the single `META` constant in `src/config.rs` to
rebind every shortcut to a different modifier (Alt or Control).

## Launching programs

| Shortcut             | Action                                              |
| -------------------- | --------------------------------------------------- |
| `Super` + `Shift` + `Return` | Spawn the terminal (`alacritty`)            |
| `Super` + `p`        | Open the application launcher (`tofi-run`)          |

## Focus & window movement

| Shortcut             | Action                                              |
| -------------------- | --------------------------------------------------- |
| `Super` + `j`        | Focus the next window in tiling order               |
| `Super` + `k`        | Focus the previous window in tiling order           |
| `Super` + `Return`   | Swap the focused window with the master window      |
| `Super` + `o`        | Move the focused window to the next monitor         |
| `Super` + `Shift` + `c` | Close the focused window                         |

## Layout & resizing

| Shortcut             | Action                                              |
| -------------------- | --------------------------------------------------- |
| `Super` + `h`        | Shrink the master area (narrower)                   |
| `Super` + `l`        | Expand the master area (wider)                      |
| `Super` + `Up`       | Expand the focused window vertically                |
| `Super` + `Down`     | Shrink the focused window vertically                |

## Session

| Shortcut             | Action                                              |
| -------------------- | --------------------------------------------------- |
| `Super` + `Shift` + `q` | Quit oxmonad                                     |

## Notes

- Focus follows the monitor: focusing or clicking a window makes its monitor the
  active one, so new windows and layout commands target that monitor.
- The focus, swap, and resize commands act on the **active monitor** only; each
  monitor tiles its own windows independently.
- `Super` + `o` sends the focused window to the next monitor (wrapping) as that
  monitor's new master and follows it with focus.
