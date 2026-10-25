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

- `~/.local/share/tasks-rs/state.json` — v2 schema plus optional
  `priority/repeat/remind_lead/completed_at`:
  `taskId/title/done/parentId/notes/created/expanded/due/remindAt/notified/deletedAt`.
- `~/.local/share/tasks-rs/settings.json` — width, height, x, y, `compact`,
  `sort`, `reduce_motion`.
- Writes are debounced (300 ms), atomic (`state.json.tmp` + rename), and keep
  `state.json.bak`.
- If `state.json` is unreadable the app shows an error banner, disables editing
  and **never overwrites it with defaults**. Fix the file (or `.bak`) and press
  Retry.
- Dangling parents, self-parents and cycles are pruned on load; purging a parent
  removes its subtree.

Window defaults to 660×780 at the top-right of the focused monitor (a Hyprland
rule pins it to 640×560 on this 3440×1440 display). Size/position are saved on
hide/quit and reapplied with `hyprctl` (Wayland clients cannot position
themselves).

## Features

- quiet graphite panel driven by the current ColorEclipse palette, a compact
  tree mark, animated completion bar and segmented All/Open/Done/Trash filters;
- smart views Inbox / Today / Upcoming / Overdue / No date with grouped sections,
  per-section counts and collapsible headers;
- manual / due-date / priority sorting, persisted in settings;
- priorities P1–P3, visible as chips and in compact rows as a small colored rail;
- git-like graph: lane = nesting depth, smooth curved branches and no line
  between unrelated roots; the selected task's branch path is highlighted;
- drag a node onto another row (or click the link icon) to link without moving
  the row, with draw-on link animation and a toast; drag the ⠿ handle to reorder
  intentionally;
- `#tags` parsed from title and notes, deterministic tag colors, click-to-filter,
  `/ #tag`, plus a collapsible tag rail with per-tag counts;
- parent chip `↳ parent` under the title, click to jump to the parent;
- inline title and notes editing (double-click a title, or use the row menu);
- a `⋯` row menu for edit, full editor, due date, priority, subtask, link,
  duplicate (with or without subtasks), complete/reopen and delete;
- expand/collapse per row (animated): full wrapped notes, `Add subtask`,
  `Unlink`, `Mark done`, `Delete`; collapsed rows show an ellipsized one-line
  note preview;
- hover affordances for due, edit, link, delete and drag; selected rows keep
  them visible;
- compact-row mode (header `≡`) persisted in settings;
- focus mode (header `◎`) with one calm task card, subtask progress and
  keyboard navigation;
- due chip always visible when set, red when overdue, reminder bell state; the
  due dialog includes a month calendar, time presets, repeat schedule, custom
  reminder lead and a natural-language field;
- recurring tasks (daily / weekly / monthly) spawn their next occurrence when
  completed;
- smart capture in the command bar: `!p1`–`!p3`, `#tags`, and natural dates in
  English or Russian (`tomorrow 18:00`, `завтра 18:00`, `через 2 часа`,
  weekdays), with a live chip preview;
- trash filter with counts, Restore and Delete forever (batch too), undo/redo
  (`Ctrl+Z` / `Ctrl+Shift+Z`), multi-select batch bar (`Ctrl+A`), and an
  **Undo** action in the toast after destructive/batch actions;
- fuzzy search and tag search, clear chips, centered empty states with a small
  tree/check/bin motif;
- command palette (`Ctrl+K` / header `⌘`): views, filters, sorting, actions,
  data tools and fuzzy task jump;
- statistics window with open/done/overdue/trash totals and a 4-week completion
  heatmap;
- data tools: export Markdown/JSON, import `export.json`, restore
  `state.json.bak`, open the data folder;
- a compact command bar with focus ring, Enter keycap, contextual hint line,
  and fading toasts;
- keyboard navigation: `↑`/`↓` or `j`/`k` move, `Space` completes, `Enter`
  edits, `M` opens the row menu, `D` sets a due date, `1…5` switch smart views
  (`Alt+1…5` works while a field is focused), `Esc` closes;
- reduced-motion mode for the custom progress, expand, completion and link
  animations;
- drag-and-drop with a row ghost, insertion marker, target highlight and
  edge auto-scroll; completion pulses and collapses the row;
- reminders call `notify-send` while the daemon runs; the systemd timer is a
  fallback and skips itself when the daemon is alive;
- header shows completion, open/done/overdue counts and tray/daemon status.

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
pruning, atomic saves/backups, unreadable-state handling, calendar grids,
Markdown export, activity buckets, recurrence/next-due helpers and English/
Russian time parsing (39 tests).

Reminders can be tested manually by setting `remindAt` a few seconds in the past
on a temporary task, waiting ≤10 s for the daemon (or running `--remind` while
the daemon is stopped), then removing the task.
