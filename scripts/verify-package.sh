#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -ne 1 ]; then
  echo "Usage: $0 <extension-package.tar.gz>" >&2
  exit 2
fi

ARCHIVE="$1"
TMP_DIR="$(mktemp -d)"
cleanup() {
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT

tar xzf "$ARCHIVE" -C "$TMP_DIR"

DRIVER_DIR="$TMP_DIR"
DRIVER_JSON="${DRIVER_DIR}/driver.json"
if [ ! -f "$DRIVER_JSON" ]; then
  echo "Missing root-level driver.json" >&2
  exit 1
fi

COMMAND="$(node -e 'const fs = require("fs"); const p = process.argv[1]; const data = JSON.parse(fs.readFileSync(p, "utf8")); process.stdout.write(data.entry && data.entry.command || "");' "$DRIVER_JSON")"
if [ -z "$COMMAND" ]; then
  echo "driver.json entry.command is empty" >&2
  exit 1
fi

BIN_PATH="${DRIVER_DIR}/${COMMAND#./}"
if [ ! -f "$BIN_PATH" ]; then
  echo "driver binary referenced by entry.command does not exist: ${COMMAND}" >&2
  exit 1
fi

if [ ! -d "${DRIVER_DIR}/locales" ]; then
  echo "Missing locales directory" >&2
  exit 1
fi

DRIVER_JSON="$DRIVER_JSON" \
DRIVER_DIR="$DRIVER_DIR" \
node <<'NODE'
const fs = require("fs");
const path = require("path");

const driverJson = process.env.DRIVER_JSON;
const driverDir = process.env.DRIVER_DIR;
const manifest = JSON.parse(fs.readFileSync(driverJson, "utf8"));
const ui = manifest.ui || {};

for (const key of ["icon", "icon_color"]) {
  const value = ui[key];
  if (typeof value !== "string" || value.trim() === "") continue;
  if (!value.includes("/") && !value.includes("\\")) continue;
  const iconPath = path.join(driverDir, value);
  if (!fs.existsSync(iconPath)) {
    console.error(`driver.json ui.${key} references missing file: ${value}`);
    process.exit(1);
  }
}
NODE

echo "Package verification ok: ${ARCHIVE}"
