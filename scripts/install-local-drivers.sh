#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat >&2 <<'EOF'
Usage: scripts/install-local-drivers.sh [extension-id]

Build, package, verify, and install IPC database driver extensions into the
local one-hub database driver directory. Passing an extension id installs only
that driver; omitting it installs every driver under extensions/ipc.

Environment:
  ONETCLI_DATABASE_DRIVER_DIR  Override install root. Defaults to
                               $XDG_CONFIG_HOME/one-hub/extensions/database_drivers
                               or $HOME/.config/one-hub/extensions/database_drivers.
EOF
}

if [ "$#" -gt 1 ]; then
  usage
  exit 2
fi

case "${1:-}" in
  -h|--help)
    usage
    exit 0
    ;;
esac

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
ARTIFACT_DIR="${REPO_DIR}/target/local-extension-artifacts"
CONFIG_HOME="${XDG_CONFIG_HOME:-${HOME}/.config}"
INSTALL_ROOT="${ONETCLI_DATABASE_DRIVER_DIR:-${CONFIG_HOME}/one-hub/extensions/database_drivers}"

fail() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

json_value() {
  local file="$1"
  local expression="$2"
  node -e "const fs = require('fs'); const data = JSON.parse(fs.readFileSync(process.argv[1], 'utf8')); const value = ${expression}; if (Array.isArray(value)) process.stdout.write(value.join(' ')); else process.stdout.write(value == null ? '' : String(value));" "$file"
}

host_triple() {
  rustc -vV | sed -n 's/^host: //p'
}

driver_ids() {
  if [ "$#" -eq 1 ] && [ -n "$1" ]; then
    printf '%s\n' "$1"
    return 0
  fi

  find "${REPO_DIR}/extensions/ipc" -mindepth 2 -maxdepth 2 -name extension.build.json -print \
    | sort \
    | while IFS= read -r metadata; do
        basename "$(dirname "$metadata")"
      done
}

target_for_driver() {
  local metadata="$1"
  local targets host
  targets="$(json_value "$metadata" 'data.targets || []')"
  for target in $targets; do
    if [ "$target" = "universal" ]; then
      printf 'universal\n'
      return 0
    fi
  done

  host="$(host_triple)"
  for target in $targets; do
    if [ "$target" = "$host" ]; then
      printf '%s\n' "$host"
      return 0
    fi
  done
  fail "$(json_value "$metadata" 'data.id') does not declare target ${host}"
}

build_driver() {
  local id="$1"
  local target="$2"
  local metadata="$3"
  local language package_name

  language="$(json_value "$metadata" 'data.language || "rust"')"
  package_name="$(json_value "$metadata" 'data.package || data.binary || `${data.id}_driver`')"

  printf 'Building %s (%s, %s)\n' "$id" "$language" "$target"
  case "$language" in
    rust)
      cargo build --release -p "$package_name" --target "$target"
      ;;
    go)
      bash "${SCRIPT_DIR}/build-go-driver.sh" "$id" "$target"
      ;;
    java)
      bash "${SCRIPT_DIR}/build-java-driver.sh" "$id" "$target"
      ;;
    *)
      fail "unsupported driver language for ${id}: ${language}"
      ;;
  esac
}

package_driver() {
  local id="$1"
  local target="$2"
  local driver_json="${REPO_DIR}/extensions/ipc/${id}/driver.json"
  local version

  [ -f "$driver_json" ] || fail "missing driver manifest: ${driver_json}"
  version="$(json_value "$driver_json" 'data.version')"
  [ -n "$version" ] || fail "missing version in ${driver_json}"

  mkdir -p "$ARTIFACT_DIR"
  printf 'Packaging %s %s\n' "$id" "$version"
  bash "${SCRIPT_DIR}/package-driver.sh" "$id" "$target" "$ARTIFACT_DIR" "$version"
}

install_packaged_driver() {
  local id="$1"
  local target="$2"
  local packaged_dir="${REPO_DIR}/target/extension-packages/${target}/${id}"
  local dest_dir="${INSTALL_ROOT}/${id}"
  local backup_root="${INSTALL_ROOT}/.backups"
  local backup_dir base_backup counter

  [ -d "$packaged_dir" ] || fail "packaged driver directory does not exist: ${packaged_dir}"

  mkdir -p "$INSTALL_ROOT"
  if [ -e "$dest_dir" ]; then
    mkdir -p "$backup_root"
    base_backup="${backup_root}/${id}.backup.$(date +%Y%m%d%H%M%S)"
    backup_dir="$base_backup"
    counter=1
    while [ -e "$backup_dir" ]; do
      counter=$((counter + 1))
      backup_dir="${base_backup}.${counter}"
    done
    mv "$dest_dir" "$backup_dir"
  else
    backup_dir=""
  fi

  mkdir -p "$dest_dir"
  if ! cp -R "${packaged_dir}/." "${dest_dir}/"; then
    rm -rf "$dest_dir"
    if [ -n "$backup_dir" ]; then
      mv "$backup_dir" "$dest_dir"
    fi
    fail "failed to install ${id}; restored previous driver if a backup existed"
  fi

  printf 'Installed %s -> %s\n' "$id" "$dest_dir"
}

main() {
  local selected="${1:-}"
  local target id metadata archive

  printf 'Installing local drivers into %s\n' "$INSTALL_ROOT"

  while IFS= read -r id; do
    [ -n "$id" ] || continue
    metadata="${REPO_DIR}/extensions/ipc/${id}/extension.build.json"
    [ -f "$metadata" ] || fail "missing extension build metadata: ${metadata}"
    target="$(target_for_driver "$metadata")"
    build_driver "$id" "$target" "$metadata"
    archive="$(package_driver "$id" "$target" | tail -n 1)"
    bash "${SCRIPT_DIR}/verify-package.sh" "$archive"
    install_packaged_driver "$id" "$target"
  done < <(driver_ids "$selected")
}

main "$@"
