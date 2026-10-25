# Tasks (Rust + egui)

Standalone replacement for the old Quickshell/QML Tasks app. One native binary,
no QML engine, no Quickshell process.

## Build / run

```bash
cargo build --release
~/.local/bin/tasks-rs          # or SUPER + T
```

The launcher is a toggle: if the window is already open it closes it (the app
saves on every change, with a 300 ms debounce).

## State

`~/.local/share/tasks-rs/state.json` — same v2 schema as before
(`taskId/title/done/parentId/notes/created/expanded/due/remindAt/notified/deletedAt`).
Atomic write (`state.json.tmp` + rename).

## Features

- git-like graph: lane = nesting depth, rails, orthogonal branches;
- drag a node onto another row to link, click a node to complete;
- one-line notes, due chip (click to edit), reminders;
- trash (soft delete + restore / delete forever), undo/redo (Ctrl+Z / Ctrl+Shift+Z);
- filters All/Open/Done/Trash, `/` fuzzy search, Enter to add;
- double-click a row to edit title/notes;
- animated strikethrough, link drag line, minimal dark theme from ColorEclipse.

## Reminders

While the window is open, reminders are checked every 20 s. When it is closed,
`tasks-rs-remind.timer` (systemd user, every minute) runs `tasks-rs --remind`
headlessly, sends `notify-send` and persists `notified`.

## Window

Floating 640×auto-height (clamped 380–940), top-right, pinned; rules in
`~/.config/hypr/config/custom/tasks_rs.lua`. Esc or clicking another window
closes it; focus loss is the "click outside".

## Not yet

Multi-select/batch actions, drag-reorder, inline title editing without the
dialog, recurrence, i18n. Memory measured: RSS ~345 MB / PSS ~170 MB /
anon ~38 MB, 0 % CPU idle (most RSS is shared GL/NVIDIA/Wayland memory).
