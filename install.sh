#!/usr/bin/env sh
# Install Tasks locally: binary, desktop entry, icon and systemd units.
# Run from the repository root after reviewing the script.
set -eu

HERE=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
cd "$HERE"

BIN_DIR=${XDG_BIN_HOME:-$HOME/.local/bin}
APP_DIR=${XDG_DATA_HOME:-$HOME/.local/share}/applications
ICON_DIR=${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/scalable/apps
UNIT_DIR=${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user

if ! command -v cargo >/dev/null 2>&1; then
    echo "cargo is required to build tasks-rs" >&2
    exit 1
fi

cargo build --release

mkdir -p "$APP_DIR" "$ICON_DIR" "$UNIT_DIR"
install -Dm755 target/release/tasks-rs "$BIN_DIR/tasks-rs"
install -Dm644 assets/tasks-rs.svg "$ICON_DIR/tasks-rs.svg"

sed "s|@BIN@|$BIN_DIR/tasks-rs|g" \
    contrib/tasks-rs.desktop > "$APP_DIR/tasks-rs.desktop.tmp"
install -Dm644 "$APP_DIR/tasks-rs.desktop.tmp" "$APP_DIR/tasks-rs.desktop"
rm -f "$APP_DIR/tasks-rs.desktop.tmp"

for unit in tasks-rs.service tasks-rs-remind.service; do
    sed "s|@BIN@|$BIN_DIR/tasks-rs|g" "contrib/$unit" > "$UNIT_DIR/$unit.tmp"
    install -Dm644 "$UNIT_DIR/$unit.tmp" "$UNIT_DIR/$unit"
    rm -f "$UNIT_DIR/$unit.tmp"
done
install -Dm644 contrib/tasks-rs-remind.timer "$UNIT_DIR/tasks-rs-remind.timer"

if [ "${TASKS_RS_NO_SYSTEMD:-0}" != "1" ] && command -v systemctl >/dev/null 2>&1; then
    systemctl --user daemon-reload || true
    systemctl --user enable --now tasks-rs.service || true
    systemctl --user enable --now tasks-rs-remind.timer || true
fi

cat <<MSG
Lunar Tasks installed:
  $BIN_DIR/tasks-rs
  $APP_DIR/tasks-rs.desktop
  $ICON_DIR/tasks-rs.svg

Run it with:
  $BIN_DIR/tasks-rs

Optional Hyprland rule/bind examples are in README.md.
MSG
