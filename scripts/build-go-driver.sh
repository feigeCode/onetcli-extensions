#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -ne 2 ]; then
  echo "Usage: $0 <extension-id> <target-triple>" >&2
  exit 2
fi

EXTENSION_ID="$1"
TARGET="$2"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
SOURCE_DIR="${REPO_DIR}/extensions/ipc/${EXTENSION_ID}"
BUILD_METADATA="${SOURCE_DIR}/extension.build.json"
BUILD_TMP_DIR="${TMPDIR:-/tmp}/onetcli-go-driver-build.$$"
VENDOR_DIR="${REPO_DIR}/vendor"

cleanup() {
  rm -rf "$BUILD_TMP_DIR"
}
trap cleanup EXIT

if [ ! -f "$BUILD_METADATA" ]; then
  echo "Missing extension build metadata: ${BUILD_METADATA}" >&2
  exit 1
fi

fail() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

read_metadata() {
  node -e "const fs = require('fs'); const data = JSON.parse(fs.readFileSync(process.argv[1], 'utf8')); const value = ${1}; if (Array.isArray(value)) process.stdout.write(value.join(' ')); else process.stdout.write(value || '');" "$BUILD_METADATA"
}

LANGUAGE="$(read_metadata 'data.language')"
if [ "$LANGUAGE" != "go" ]; then
  echo "Extension ${EXTENSION_ID} is not a Go driver" >&2
  exit 1
fi

CMD_PACKAGE="$(read_metadata 'data.package')"
BIN_STEM="$(read_metadata 'data.binary || `${data.id}-ipc-driver`')"
BUILD_TAGS="$(read_metadata 'data.build_tags || []')"

if [ -z "$CMD_PACKAGE" ]; then
  echo "Missing Go package in ${BUILD_METADATA}" >&2
  exit 1
fi

has_go_source() {
  local path="$1"
  [[ -d "$path" ]] && find "$path" -maxdepth 2 -name '*.go' -type f | grep -q .
}

default_module_path() {
  local import_path="$1"
  local gopath
  gopath="$(go env GOPATH 2>/dev/null || true)"
  IFS=':' read -r -a roots <<<"$gopath"
  for root in "${roots[@]}"; do
    if [[ -d "$root/src/$import_path" ]]; then
      printf '%s\n' "$root/src/$import_path"
      return 0
    fi
  done
  return 1
}

driver_path() {
  local env_name="$1"
  local import_path="$2"
  local value="${!env_name:-}"
  if [[ -n "$value" ]]; then
    printf '%s\n' "$value"
    return 0
  fi
  default_module_path "$import_path"
}

prepare_driver_module() {
  local label="$1"
  local import_path="$2"
  local source_path="$3"
  local replace_path="$BUILD_TMP_DIR/$label"

  [[ -d "$source_path" ]] || fail "$label driver path does not exist: $source_path"
  if ! has_go_source "$source_path"; then
    fail "$label driver path does not contain Go source files: $source_path"
  fi

  if [[ -f "$source_path/go.mod" ]]; then
    printf '%s\n' "$source_path"
    return 0
  fi

  mkdir -p "$replace_path"
  cp -R "$source_path"/. "$replace_path"/
  if [[ ! -f "$replace_path/go.mod" ]]; then
    printf 'module %s\n\ngo 1.23\n' "$import_path" >"$replace_path/go.mod"
  fi
  printf '%s\n' "$replace_path"
}

MODFILE_ARG=""
USE_VENDOR=false
if [[ -d "$VENDOR_DIR" ]]; then
  USE_VENDOR=true
fi

prepare_modfile() {
  local name="$1"
  local modfile="$BUILD_TMP_DIR/$name-build.mod"
  mkdir -p "$BUILD_TMP_DIR"
  cp "$REPO_DIR/go.mod" "$modfile"
  if [[ -f "$REPO_DIR/go.sum" ]]; then
    cp "$REPO_DIR/go.sum" "${modfile%.mod}.sum"
  fi
  MODFILE_ARG="-modfile=$modfile"
  printf '%s\n' "$modfile"
}

case "$EXTENSION_ID" in
  dm)
    if [[ "$USE_VENDOR" != true ]] && source_path="$(driver_path DM_DRIVER_PATH gitee.com/chunanyong/dm 2>/dev/null)"; then
      dm_module_path="$(prepare_driver_module dm gitee.com/chunanyong/dm "$source_path")"
      modfile="$(prepare_modfile dm)"
      go mod edit -modfile="$modfile" "-replace=gitee.com/chunanyong/dm=$dm_module_path"
    fi
    ;;
  kingbase)
    if [[ "$USE_VENDOR" != true ]] && source_path="$(driver_path KINGBASE_DRIVER_PATH gitea.com/kingbase/gokb 2>/dev/null)"; then
      kingbase_module_path="$(prepare_driver_module kingbase gitea.com/kingbase/gokb "$source_path")"
      modfile="$(prepare_modfile kingbase)"
      go mod edit -modfile="$modfile" "-replace=gitea.com/kingbase/gokb=$kingbase_module_path"
    fi
    ;;
esac

case "$TARGET" in
  x86_64-unknown-linux-gnu)
    GOOS_VALUE="linux"
    GOARCH_VALUE="amd64"
    EXE_SUFFIX=""
    ;;
  aarch64-unknown-linux-gnu)
    GOOS_VALUE="linux"
    GOARCH_VALUE="arm64"
    EXE_SUFFIX=""
    ;;
  x86_64-apple-darwin)
    GOOS_VALUE="darwin"
    GOARCH_VALUE="amd64"
    EXE_SUFFIX=""
    ;;
  aarch64-apple-darwin)
    GOOS_VALUE="darwin"
    GOARCH_VALUE="arm64"
    EXE_SUFFIX=""
    ;;
  x86_64-pc-windows-msvc)
    GOOS_VALUE="windows"
    GOARCH_VALUE="amd64"
    EXE_SUFFIX=".exe"
    ;;
  *)
    echo "Unsupported Go target triple: ${TARGET}" >&2
    exit 1
    ;;
esac

OUT_DIR="${REPO_DIR}/target/${TARGET}/release"
mkdir -p "$OUT_DIR"

GO_BUILD_ARGS=()
if [ -n "$BUILD_TAGS" ]; then
  GO_BUILD_ARGS+=("-tags" "$BUILD_TAGS")
fi
if [[ "$USE_VENDOR" == true ]]; then
  GO_BUILD_ARGS+=("-mod=vendor")
elif [ -n "$MODFILE_ARG" ]; then
  GO_BUILD_ARGS+=("$MODFILE_ARG")
fi

if [ "${#GO_BUILD_ARGS[@]}" -gt 0 ]; then
  GOOS="$GOOS_VALUE" \
  GOARCH="$GOARCH_VALUE" \
  CGO_ENABLED="${CGO_ENABLED:-0}" \
  GOCACHE="${GOCACHE:-${TMPDIR:-/tmp}/onetcli-extensions-go-cache}" \
  GOPROXY="${GOPROXY:-direct}" \
  GOSUMDB="${GOSUMDB:-off}" \
  go build "${GO_BUILD_ARGS[@]}" -o "${OUT_DIR}/${BIN_STEM}${EXE_SUFFIX}" "$CMD_PACKAGE"
else
  GOOS="$GOOS_VALUE" \
  GOARCH="$GOARCH_VALUE" \
  CGO_ENABLED="${CGO_ENABLED:-0}" \
  GOCACHE="${GOCACHE:-${TMPDIR:-/tmp}/onetcli-extensions-go-cache}" \
  GOPROXY="${GOPROXY:-direct}" \
  GOSUMDB="${GOSUMDB:-off}" \
  go build -o "${OUT_DIR}/${BIN_STEM}${EXE_SUFFIX}" "$CMD_PACKAGE"
fi
