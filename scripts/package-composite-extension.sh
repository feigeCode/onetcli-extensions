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

SOURCE_DIR="${REPO_DIR}/extensions/composite/${EXTENSION_ID}"
if [ ! -d "$SOURCE_DIR" ]; then
  SOURCE_DIR="${REPO_DIR}/extensions/wasm/${EXTENSION_ID}"
fi
BUILD_METADATA="${SOURCE_DIR}/extension.build.json"
if [ ! -f "$BUILD_METADATA" ]; then
  echo "Missing composite extension build metadata: ${BUILD_METADATA}" >&2
  exit 1
fi

LANGUAGE="$(node -e 'const fs = require("fs"); const data = JSON.parse(fs.readFileSync(process.argv[1], "utf8")); process.stdout.write(data.language || "static");' "$BUILD_METADATA")"
BIN_NAME="$(node -e 'const fs = require("fs"); const data = JSON.parse(fs.readFileSync(process.argv[1], "utf8")); process.stdout.write(data.binary || `${data.id}.wasm`);' "$BUILD_METADATA")"
PACKAGE_NAME="$(node -e 'const fs = require("fs"); const data = JSON.parse(fs.readFileSync(process.argv[1], "utf8")); process.stdout.write(data.package || data.id);' "$BUILD_METADATA")"
MODULE_PATH="$(node -e 'const fs = require("fs"); const data = JSON.parse(fs.readFileSync(process.argv[1], "utf8")); const runtime = data.runtime?.wasm?.[0]; process.stdout.write(runtime?.module || "");' "${SOURCE_DIR}/extension.json")"
PACKAGE_ROOT="${REPO_DIR}/target/extension-packages/${TARGET}"
EXTENSION_DIR="${PACKAGE_ROOT}/${EXTENSION_ID}"
ARCHIVE_NAME="${EXTENSION_ID}-composite-${TARGET}.tar.gz"

case "$LANGUAGE" in
  rust)
    if [ "$TARGET" = "universal" ]; then
      echo "Native Rust composite extensions require a platform target" >&2
      exit 1
    fi
    ;;
  rust-wasm|static)
    if [ "$TARGET" != "universal" ]; then
      echo "${LANGUAGE} composite extensions must use the universal target, got: ${TARGET}" >&2
      exit 1
    fi
    ;;
  *)
    echo "Unsupported composite extension language: ${LANGUAGE}" >&2
    exit 1
    ;;
esac

SOURCE_WASM=""
if [ "$LANGUAGE" = "rust-wasm" ] && [ -n "$MODULE_PATH" ]; then
  SOURCE_CANDIDATES=()
  if [ -n "${CARGO_TARGET_DIR:-}" ]; then
    SOURCE_CANDIDATES+=("${CARGO_TARGET_DIR}/wasm32-wasip2/release/${BIN_NAME}")
  fi
  SOURCE_CANDIDATES+=("${REPO_DIR}/target/wasm32-wasip2/release/${BIN_NAME}")
  SOURCE_CANDIDATES+=("${SOURCE_DIR}/${MODULE_PATH}")
  for CANDIDATE in "${SOURCE_CANDIDATES[@]}"; do
    if [ -f "$CANDIDATE" ]; then
      SOURCE_WASM="$CANDIDATE"
      break
    fi
  done
  if [ ! -f "$SOURCE_WASM" ]; then
    echo "Missing composite WASM module. Checked:" >&2
    printf '  %s\n' "${SOURCE_CANDIDATES[@]}" >&2
    echo "Run: cargo build --release -p ${PACKAGE_NAME} --target wasm32-wasip2" >&2
    exit 1
  fi
fi

SOURCE_BINARY=""
PACKAGE_COMMAND=""
if [ "$LANGUAGE" = "rust" ]; then
  EXE_SUFFIX=""
  case "$TARGET" in
    *-pc-windows-*) EXE_SUFFIX=".exe" ;;
  esac
  SOURCE_CANDIDATES=()
  if [ -n "${CARGO_TARGET_DIR:-}" ]; then
    SOURCE_CANDIDATES+=("${CARGO_TARGET_DIR}/${TARGET}/release/${BIN_NAME}${EXE_SUFFIX}")
  fi
  SOURCE_CANDIDATES+=("${REPO_DIR}/target/${TARGET}/release/${BIN_NAME}${EXE_SUFFIX}")
  for CANDIDATE in "${SOURCE_CANDIDATES[@]}"; do
    if [ -f "$CANDIDATE" ]; then
      SOURCE_BINARY="$CANDIDATE"
      break
    fi
  done
  if [ ! -f "$SOURCE_BINARY" ]; then
    echo "Missing native composite binary. Checked:" >&2
    printf '  %s\n' "${SOURCE_CANDIDATES[@]}" >&2
    exit 1
  fi
  PACKAGE_COMMAND="./bin/${BIN_NAME}${EXE_SUFFIX}"
fi

case "$MODULE_PATH" in
  /*|*..*)
    echo "extension.json runtime.wasm.module must stay inside package: ${MODULE_PATH}" >&2
    exit 1
    ;;
esac

rm -rf "$EXTENSION_DIR"
mkdir -p "$EXTENSION_DIR" "$ARTIFACT_DIR"

MANIFEST_SOURCE="${SOURCE_DIR}/extension.json"
MANIFEST_TARGET="${EXTENSION_DIR}/extension.json"
MANIFEST_SOURCE="$MANIFEST_SOURCE" \
MANIFEST_TARGET="$MANIFEST_TARGET" \
VERSION="$VERSION" \
LANGUAGE="$LANGUAGE" \
PACKAGE_COMMAND="$PACKAGE_COMMAND" \
node <<'NODE'
const fs = require("fs");
const source = process.env.MANIFEST_SOURCE;
const target = process.env.MANIFEST_TARGET;
const version = process.env.VERSION;
const manifest = JSON.parse(fs.readFileSync(source, "utf8"));
manifest.version = version;
if (process.env.LANGUAGE === "rust") {
  const runtimes = manifest.runtime?.ipc || [];
  if (runtimes.length !== 1) {
    throw new Error("native composite packages currently require exactly one runtime.ipc entry");
  }
  const previous = runtimes[0]?.entry?.command;
  if (!previous) throw new Error("native composite runtime is missing entry.command");
  const next = process.env.PACKAGE_COMMAND;
  runtimes[0].entry.command = next;
  manifest.permissions = (manifest.permissions || []).map((permission) =>
    permission === `spawn:${previous}` ? `spawn:${next}` : permission,
  );
  if (!manifest.permissions.includes(`spawn:${next}`)) {
    throw new Error(`native composite manifest must declare spawn:${previous}`);
  }
}
fs.writeFileSync(target, `${JSON.stringify(manifest, null, 2)}\n`);
NODE

if [ -n "$MODULE_PATH" ]; then
  mkdir -p "$(dirname "${EXTENSION_DIR}/${MODULE_PATH}")"
  cp "$SOURCE_WASM" "${EXTENSION_DIR}/${MODULE_PATH}"
fi

if [ "$LANGUAGE" = "rust" ]; then
  BINARY_TARGET="${EXTENSION_DIR}/${PACKAGE_COMMAND#./}"
  mkdir -p "$(dirname "$BINARY_TARGET")"
  cp "$SOURCE_BINARY" "$BINARY_TARGET"
  case "$TARGET" in
    *-pc-windows-*) ;;
    *) chmod +x "$BINARY_TARGET" ;;
  esac
fi

for RESOURCE_DIR in icons locales assets ui; do
  if [ -d "${SOURCE_DIR}/${RESOURCE_DIR}" ]; then
    cp -R "${SOURCE_DIR}/${RESOURCE_DIR}" "${EXTENSION_DIR}/${RESOURCE_DIR}"
  fi
done

tar czf "${ARTIFACT_DIR}/${ARCHIVE_NAME}" -C "$EXTENSION_DIR" .
echo "${ARTIFACT_DIR}/${ARCHIVE_NAME}"
