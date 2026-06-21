#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -ne 4 ]; then
  echo "Usage: $0 <provider-id> <target-triple> <artifact-dir> <version>" >&2
  exit 2
fi

PROVIDER_ID="$1"
TARGET="$2"
ARTIFACT_DIR="$3"
VERSION="$4"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"

SOURCE_DIR="${REPO_DIR}/extensions/remote-desktop/${PROVIDER_ID}"
BUILD_METADATA="${SOURCE_DIR}/extension.build.json"
if [ ! -f "$BUILD_METADATA" ]; then
  echo "Missing provider build metadata: ${BUILD_METADATA}" >&2
  exit 1
fi

BIN_STEM="$(node -e 'const fs = require("fs"); const data = JSON.parse(fs.readFileSync(process.argv[1], "utf8")); process.stdout.write(data.binary || data.id);' "$BUILD_METADATA")"
MANIFEST_PATH="$(node -e 'const fs = require("fs"); const data = JSON.parse(fs.readFileSync(process.argv[1], "utf8")); process.stdout.write(data.manifest_path || "");' "$BUILD_METADATA")"
BIN_NAME="$BIN_STEM"
if [[ "$TARGET" == *windows* ]]; then
  BIN_NAME="${BIN_STEM}.exe"
fi

PACKAGE_ROOT="${REPO_DIR}/target/extension-packages/${TARGET}"
PROVIDER_DIR="${PACKAGE_ROOT}/${PROVIDER_ID}"
ARCHIVE_NAME="${PROVIDER_ID}-remote-desktop-provider-${TARGET}.tar.gz"

SOURCE_BIN=""
SOURCE_CANDIDATES=()
if [ -n "${CARGO_TARGET_DIR:-}" ]; then
  SOURCE_CANDIDATES+=("${CARGO_TARGET_DIR}/${TARGET}/release/${BIN_NAME}")
fi
SOURCE_CANDIDATES+=("${REPO_DIR}/target/${TARGET}/release/${BIN_NAME}")
if [ -n "$MANIFEST_PATH" ]; then
  MANIFEST_DIR="${REPO_DIR}/$(dirname "$MANIFEST_PATH")"
  if [ -d "$MANIFEST_DIR" ]; then
    SOURCE_CANDIDATES+=("${MANIFEST_DIR}/target/${TARGET}/release/${BIN_NAME}")
  fi
fi
for CANDIDATE in "${SOURCE_CANDIDATES[@]}"; do
  if [ -f "$CANDIDATE" ]; then
    SOURCE_BIN="$CANDIDATE"
    break
  fi
done

if [ ! -f "$SOURCE_BIN" ]; then
  echo "Missing provider binary. Checked:" >&2
  printf '  %s\n' "${SOURCE_CANDIDATES[@]}" >&2
  if [ -n "$MANIFEST_PATH" ]; then
    echo "Run: cargo build --release --manifest-path ${MANIFEST_PATH} --target ${TARGET}" >&2
  else
    echo "Run: cargo build --release -p ${BIN_STEM} --target ${TARGET}" >&2
  fi
  exit 1
fi

rm -rf "$PROVIDER_DIR"
mkdir -p "$PROVIDER_DIR" "$ARTIFACT_DIR"
cp "$SOURCE_BIN" "${PROVIDER_DIR}/${BIN_NAME}"

MANIFEST_SOURCE="${SOURCE_DIR}/remote_desktop_provider.json"
MANIFEST_TARGET="${PROVIDER_DIR}/remote_desktop_provider.json"
MANIFEST_SOURCE="$MANIFEST_SOURCE" \
MANIFEST_TARGET="$MANIFEST_TARGET" \
VERSION="$VERSION" \
BIN_NAME="$BIN_NAME" \
node <<'NODE'
const fs = require("fs");
const source = process.env.MANIFEST_SOURCE;
const target = process.env.MANIFEST_TARGET;
const version = process.env.VERSION;
const binName = process.env.BIN_NAME;
const manifest = JSON.parse(fs.readFileSync(source, "utf8"));
manifest.version = version;
manifest.entry = manifest.entry || {};
manifest.entry.command = `./${binName}`;
fs.writeFileSync(target, `${JSON.stringify(manifest, null, 2)}\n`);
NODE

if [[ "$TARGET" != *windows* ]]; then
  chmod +x "${PROVIDER_DIR}/${BIN_NAME}"
fi

tar czf "${ARTIFACT_DIR}/${ARCHIVE_NAME}" -C "$PROVIDER_DIR" .
echo "${ARTIFACT_DIR}/${ARCHIVE_NAME}"
