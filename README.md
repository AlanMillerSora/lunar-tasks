# Tasks (Rust + egui)

Native desktop task manager for Hyprland/Wayland, replacing the old
Quickshell/QML app. One resident tray daemon plus a lightweight egui window
that is spawned on demand. No QML engine, no Quickshell process.

```
SUPER + T  /  ~/.local/bin/tasks-rs        toggle the window
```

## Architecture

```
systemd --user  tasks-rs.service
        │
        └── tasks-rs --daemon                 ~9 MB RSS, <0.1 % CPU idle
              ├── StatusNotifierItem tray       (ksni, on the Quickshell bar)
              ├── $XDG_RUNTIME_DIR/tasks-rs.sock      CLI / single instance
              ├── reminders (every 10 s, notify-send)
              └── spawns  tasks-rs --gui  on Show/Toggle/Add
                            │
                            ├── $XDG_RUNTIME_DIR/tasks-rs-gui.sock
                            └── renders ~/.local/share/tasks-rs/state.json
```

The GUI owns the window and writes state; the daemon owns the tray, the single
instance lock, reminders and process lifecycle. Commands are line based over
Unix sockets (`toggle`, `show`, `hide`, `add`, `quit`, `status`, `reload`).

On Wayland, `Visible(false)` is not reliably honoured by compositors, so
closing (×, Esc, tray Hide) exits the GUI process while the daemon and tray stay
resident. The next Show/Toggle starts a fresh GUI and restores its size and
position. This gives deterministic hide/show behaviour and zero idle GUI cost.

## Commands

```bash
tasks-rs                 # toggle (default); starts the daemon if needed
tasks-rs show
tasks-rs hide            # closes the GUI, daemon stays
tasks-rs add "buy milk #home"
tasks-rs status          # daemon pid, GUI state, tray state
tasks-rs quit            # quit both GUI and daemon
tasks-rs --gui           # GUI only (second launch shows the existing window)
tasks-rs --daemon        # resident daemon (systemd service)
tasks-rs --remind        # one-shot reminder pass (systemd timer fallback)
```

## Install / autostart

Installed on this machine:

| file | purpose |
| --- | --- |
| `~/.local/bin/tasks-rs` | launcher (passes through commands) |
| `~/.local/share/applications/tasks-rs.desktop` | desktop entry, `StartupWMClass=tasks-rs` |
| `~/.local/share/icons/hicolor/scalable/apps/tasks-rs.svg` | app + tray icon |
| `~/.config/systemd/user/tasks-rs.service` | enabled at login (`default.target`) |
| `~/.config/systemd/user/tasks-rs-remind.{service,timer}` | reminder fallback, every minute |
| `~/.config/hypr/config/custom/tasks_rs.lua` | floating/pinned window + `SUPER + T` |

The service is started at login without a window, so there is no visible flash.
It recovers Wayland/Hyprland environment variables from `$XDG_RUNTIME_DIR` even
when systemd starts it before the compositor. The tray registers automatically
when the StatusNotifierWatcher (Quickshell) appears.

```bash
systemctl --user enable --now tasks-rs.service
systemctl --user enable --now tasks-rs-remind.timer
hyprctl reload
```

## State and window settings

- `~/.local/share/tasks-rs/state.json` — same v2 schema as before:
  `taskId/title/done/parentId/notes/created/expanded/due/remindAt/notified/deletedAt`.
- `~/.local/share/tasks-rs/settings.json` — width, height, x, y.
- Writes are debounced (300 ms), atomic (`state.json.tmp` + rename), and keep
  `state.json.bak`.
- If `state.json` is unreadable the app shows an error banner, disables editing
  and **never overwrites it with defaults**. Fix the file (or `.bak`) and press
  Retry.
- Dangling parents, self-parents and cycles are pruned on load; purging a parent
  removes its subtree.

Window defaults to 640×780 at the top-right of the focused monitor (2776,24 on
this 3440×1440 display). Size/position are saved on hide/quit and reapplied with
`hyprctl` (Wayland clients cannot position themselves).

## Features

- git-like graph: lane = nesting depth, rails, orthogonal branches;
- drag a node onto another row (or click the link icon) to link, with animated
  link formation and a toast; drag the ⠿ handle to reorder;
- `#tags` parsed from title and notes, tag chips, click-to-filter, `/ #tag`;
- parent chip `↳ parent` under the title, click to jump to the parent;
- expand/collapse per row (animated): full wrapped notes, `+ Subtask`, `Unlink`,
  `Mark done`, `Delete`; collapsed rows show a one-line note preview;
- hover affordances for due, edit, link, delete and drag; selected rows keep
  them visible;
- due chip always visible when set, red when overdue, reminder bell state;
- trash filter with counts, Restore and Delete forever (batch too), undo/redo
  (`Ctrl+Z` / `Ctrl+Shift+Z`), multi-select batch bar (`Ctrl+A`);
- filters All/Open/Done/Trash (`Ctrl+1..4`), fuzzy search, tag search, search
  clear chips, explicit empty states;
- reminders call `notify-send` while the daemon runs; the systemd timer is a
  fallback and skips itself when the daemon is alive;
- header shows completion, open/done counts and tray/daemon status.

## Measured on this machine

Wayland/Hyprland 0.56, 3440×1440, release build, idle after settling:

| process | RSS | PSS | anon | file | CPU idle |
| --- | ---: | ---: | ---: | ---: | ---: |
| `tasks-rs --daemon` | 9.2 MB | 6.8 MB | 1.1 MB | 8.1 MB | 0.00 % |
| `tasks-rs --gui` | 346.2 MB | 170.8 MB | 38.2 MB | 294.7 MB | 0.0–0.3 % |

The GUI's large RSS is shared GL/NVIDIA/Wayland file-backed memory (PSS nearly
half of the old QML app's 283 MB; anonymous memory is only ~38 MB). CPU is
0.0 % when unfocused and ≤0.3 % while the focused text caret blinks.
PSS varies slightly because the daemon shares D-Bus/zbus pages with other session
services.

## Build / test

```bash
cargo build --release
cargo test
```

Unit tests cover hashtag parsing, tag/text search, dangling-parent/cycle
pruning, atomic saves/backups, unreadable-state handling and time parsing.

Reminders can be tested manually by setting `remindAt` a few seconds in the past
on a temporary task, waiting ≤10 s for the daemon (or running `--remind` while
the daemon is stopped), then removing the task.
