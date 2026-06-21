#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -ne 1 ]; then
  echo "Usage: $0 <package.tar.gz>" >&2
  exit 2
fi

PACKAGE="$1"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

tar xzf "$PACKAGE" -C "$TMP_DIR"

MANIFEST="${TMP_DIR}/remote_desktop_provider.json"
if [ ! -f "$MANIFEST" ]; then
  echo "Missing root-level remote_desktop_provider.json" >&2
  exit 1
fi

COMMAND="$(node -e 'const fs = require("fs"); const p = process.argv[1]; const data = JSON.parse(fs.readFileSync(p, "utf8")); process.stdout.write(data.entry && data.entry.command || "");' "$MANIFEST")"
if [ -z "$COMMAND" ]; then
  echo "remote_desktop_provider.json entry.command is empty" >&2
  exit 1
fi

COMMAND_PATH="${TMP_DIR}/${COMMAND#./}"
if [ ! -f "$COMMAND_PATH" ]; then
  echo "provider binary referenced by entry.command does not exist: ${COMMAND}" >&2
  exit 1
fi

node <<'NODE' "$MANIFEST"
const fs = require("fs");
const manifest = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
const required = ["id", "name", "version", "protocol", "entry", "capabilities"];
for (const key of required) {
  if (!manifest[key]) {
    console.error(`remote_desktop_provider.json missing ${key}`);
    process.exit(1);
  }
}
if (!["rdp", "vnc"].includes(manifest.protocol)) {
  console.error(`unsupported protocol: ${manifest.protocol}`);
  process.exit(1);
}
NODE

echo "Verified ${PACKAGE}"
