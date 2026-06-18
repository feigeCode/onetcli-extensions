#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -ne 4 ]; then
  echo "Usage: $0 <extension-id> <target-triple> <artifact-dir> <version>" >&2
  exit 2
fi

EXTENSION_ID="$1"
TARGET="$2"
ARTIFACT_DIR="$3"
VERSION="$4"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"

SOURCE_DIR="${REPO_DIR}/extensions/ipc/${EXTENSION_ID}"
BUILD_METADATA="${SOURCE_DIR}/extension.build.json"
if [ ! -f "$BUILD_METADATA" ]; then
  echo "Missing extension build metadata: ${BUILD_METADATA}" >&2
  exit 1
fi

LANGUAGE="$(node -e 'const fs = require("fs"); const data = JSON.parse(fs.readFileSync(process.argv[1], "utf8")); process.stdout.write(data.language || "rust");' "$BUILD_METADATA")"
BIN_STEM="$(node -e 'const fs = require("fs"); const data = JSON.parse(fs.readFileSync(process.argv[1], "utf8")); process.stdout.write(data.binary || `${data.id}_driver`);' "$BUILD_METADATA")"
BIN_NAME="$BIN_STEM"
if [[ "$TARGET" == *windows* ]]; then
  BIN_NAME="${BIN_STEM}.exe"
fi

SOURCE_BIN="${REPO_DIR}/target/${TARGET}/release/${BIN_NAME}"
PACKAGE_ROOT="${REPO_DIR}/target/extension-packages/${TARGET}"
DRIVER_DIR="${PACKAGE_ROOT}/${EXTENSION_ID}"
ARCHIVE_NAME="${EXTENSION_ID}-driver-${TARGET}.tar.gz"

if [ ! -f "$SOURCE_BIN" ]; then
  echo "Missing driver binary: ${SOURCE_BIN}" >&2
  if [ "$LANGUAGE" = "go" ]; then
    echo "Run: bash scripts/build-go-driver.sh ${EXTENSION_ID} ${TARGET}" >&2
  elif [ "$LANGUAGE" = "java" ]; then
    echo "Run: bash scripts/build-java-driver.sh ${EXTENSION_ID} ${TARGET}" >&2
  else
    echo "Run: cargo build --release -p ${BIN_STEM} --target ${TARGET}" >&2
  fi
  exit 1
fi

rm -rf "$DRIVER_DIR"
mkdir -p "$DRIVER_DIR" "$ARTIFACT_DIR"
cp "$SOURCE_BIN" "${DRIVER_DIR}/${BIN_NAME}"
cp -R "${SOURCE_DIR}/locales" "${DRIVER_DIR}/locales"
if [ -d "${REPO_DIR}/target/${TARGET}/release/lib" ]; then
  cp -R "${REPO_DIR}/target/${TARGET}/release/lib" "${DRIVER_DIR}/lib"
fi

if [[ "$TARGET" == *windows* ]]; then
  RUNTIME_DLL="${REPO_DIR}/target/${TARGET}/release/deps/duckdb.dll"
  if [ -f "$RUNTIME_DLL" ]; then
    cp "$RUNTIME_DLL" "${DRIVER_DIR}/duckdb.dll"
  fi
fi

DRIVER_JSON_SOURCE="${SOURCE_DIR}/driver.json"
DRIVER_JSON_TARGET="${DRIVER_DIR}/driver.json"
DRIVER_JSON_SOURCE="$DRIVER_JSON_SOURCE" \
DRIVER_JSON_TARGET="$DRIVER_JSON_TARGET" \
VERSION="$VERSION" \
BIN_NAME="$BIN_NAME" \
node <<'NODE'
const fs = require("fs");
const source = process.env.DRIVER_JSON_SOURCE;
const target = process.env.DRIVER_JSON_TARGET;
const version = process.env.VERSION;
const binName = process.env.BIN_NAME;
const manifest = JSON.parse(fs.readFileSync(source, "utf8"));
manifest.version = version;
manifest.entry = manifest.entry || {};
manifest.entry.command = `./${binName}`;
fs.writeFileSync(target, `${JSON.stringify(manifest, null, 2)}\n`);
NODE

if [[ "$TARGET" != *windows* ]]; then
  chmod +x "${DRIVER_DIR}/${BIN_NAME}"
fi

tar czf "${ARTIFACT_DIR}/${ARCHIVE_NAME}" -C "$PACKAGE_ROOT" "$EXTENSION_ID"
echo "${ARTIFACT_DIR}/${ARCHIVE_NAME}"
