#!/usr/bin/env sh
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
BIN_DIR="${HOME}/.local/bin"
APP_DIR="${HOME}/.local/share/applications"
ICON_PNG_DIR="${HOME}/.local/share/icons/hicolor/256x256/apps"
ICON_SVG_DIR="${HOME}/.local/share/icons/hicolor/scalable/apps"

mkdir -p "$BIN_DIR" "$APP_DIR" "$ICON_PNG_DIR" "$ICON_SVG_DIR"
install -m 755 "${SCRIPT_DIR}/voltshot" "${BIN_DIR}/voltshot"
install -m 644 "${SCRIPT_DIR}/voltshot.png" "${ICON_PNG_DIR}/voltshot.png"
install -m 644 "${SCRIPT_DIR}/icon.svg" "${ICON_SVG_DIR}/voltshot.svg"
sed "s|^Exec=voltshot$|Exec=${BIN_DIR}/voltshot|" "${SCRIPT_DIR}/voltshot.desktop" > "${APP_DIR}/voltshot.desktop"
chmod 644 "${APP_DIR}/voltshot.desktop"

if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$APP_DIR" >/dev/null 2>&1 || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -q -t -f "${HOME}/.local/share/icons/hicolor" >/dev/null 2>&1 || true
fi

echo "VoltShot installed to ${BIN_DIR}/voltshot"
echo "Ensure ${BIN_DIR} is in your PATH, or launch VoltShot from your application menu."
