#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat >&2 <<'EOF'
Usage: scripts/install-local-remote-desktop-providers.sh [provider-id]

Build, package, verify, and install remote desktop provider extensions into the
active Navop remote desktop provider directory. Passing a provider id installs
only that provider; omitting it installs every provider under
extensions/remote-desktop.

Environment:
  NAVOP_REMOTE_DESKTOP_PROVIDER_DIR    Override install root. The legacy
                                       ONETCLI_REMOTE_DESKTOP_PROVIDER_DIR is
                                       also supported. By default, the script
                                       follows Navop's one-hub migration state
                                       and installs below the active app config
                                       directory.
EOF
}

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
ARTIFACT_DIR="${REPO_DIR}/target/local-extension-artifacts"
INSTALL_ROOT=""

fail() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

platform_config_root() {
  local platform
  platform="$(uname -s)"

  case "$platform" in
    CYGWIN*|MINGW*|MSYS*)
      [ -n "${APPDATA:-}" ] || fail "APPDATA is not set"
      if command -v cygpath >/dev/null 2>&1; then
        cygpath -u "$APPDATA"
      else
        printf '%s\n' "$APPDATA"
      fi
      ;;
    *)
      [ -n "${HOME:-}" ] || fail "HOME is not set"
      printf '%s/.config\n' "$HOME"
      ;;
  esac
}

active_app_config_dir() {
  local root="$1"
  local current="${root}/navop"
  local legacy="${root}/one-hub"

  if [ -d "$legacy" ] && [ ! -f "${current}/.one-hub-migration-complete" ]; then
    printf '%s\n' "$legacy"
  else
    printf '%s\n' "$current"
  fi
}

remote_desktop_provider_dir() {
  local config_root active_config_dir

  if [ -n "${NAVOP_REMOTE_DESKTOP_PROVIDER_DIR:-}" ]; then
    printf '%s\n' "$NAVOP_REMOTE_DESKTOP_PROVIDER_DIR"
    return 0
  fi
  if [ -n "${ONETCLI_REMOTE_DESKTOP_PROVIDER_DIR:-}" ]; then
    printf '%s\n' "$ONETCLI_REMOTE_DESKTOP_PROVIDER_DIR"
    return 0
  fi

  config_root="$(platform_config_root)"
  active_config_dir="$(active_app_config_dir "$config_root")"
  printf '%s/extensions/remote_desktop_providers\n' "$active_config_dir"
}

json_value() {
  local file="$1"
  local expression="$2"
  node -e "const fs = require('fs'); const data = JSON.parse(fs.readFileSync(process.argv[1], 'utf8')); const value = ${expression}; if (Array.isArray(value)) process.stdout.write(value.join(' ')); else process.stdout.write(value == null ? '' : String(value));" "$file"
}

host_triple() {
  rustc -vV | sed -n 's/^host: //p'
}

provider_ids() {
  if [ "$#" -eq 1 ] && [ -n "$1" ]; then
    printf '%s\n' "$1"
    return 0
  fi

  find "${REPO_DIR}/extensions/remote-desktop" -mindepth 2 -maxdepth 2 -name extension.build.json -print \
    | sort \
    | while IFS= read -r metadata; do
        basename "$(dirname "$metadata")"
      done
}

target_for_provider() {
  local metadata="$1"
  local targets host
  targets="$(json_value "$metadata" 'data.targets || []')"
  host="$(host_triple)"
  for target in $targets; do
    if [ "$target" = "$host" ]; then
      printf '%s\n' "$host"
      return 0
    fi
  done
  fail "$(json_value "$metadata" 'data.id') does not declare target ${host}"
}

build_provider() {
  local id="$1"
  local target="$2"
  local metadata="$3"
  local package_name manifest_path

  package_name="$(json_value "$metadata" 'data.package || data.binary || data.id')"
  manifest_path="$(json_value "$metadata" 'data.manifest_path || ""')"

  printf 'Building %s (rust, %s)\n' "$id" "$target"
  if [ -n "$manifest_path" ]; then
    cargo build --release --manifest-path "${REPO_DIR}/${manifest_path}" --target "$target"
  else
    cargo build --release -p "$package_name" --target "$target"
  fi
}

package_provider() {
  local id="$1"
  local target="$2"
  local manifest_json="${REPO_DIR}/extensions/remote-desktop/${id}/remote_desktop_provider.json"
  local version

  [ -f "$manifest_json" ] || fail "missing provider manifest: ${manifest_json}"
  version="$(json_value "$manifest_json" 'data.version')"
  [ -n "$version" ] || fail "missing version in ${manifest_json}"

  mkdir -p "$ARTIFACT_DIR"
  printf 'Packaging %s %s\n' "$id" "$version"
  bash "${SCRIPT_DIR}/package-remote-desktop-provider.sh" "$id" "$target" "$ARTIFACT_DIR" "$version"
}

install_packaged_provider() {
  local id="$1"
  local target="$2"
  local packaged_dir="${REPO_DIR}/target/extension-packages/${target}/${id}"
  local dest_dir="${INSTALL_ROOT}/${id}"
  local backup_root="${INSTALL_ROOT}/.backups"
  local backup_dir base_backup counter

  [ -d "$packaged_dir" ] || fail "packaged provider directory does not exist: ${packaged_dir}"

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
    fail "failed to install ${id}; restored previous provider if a backup existed"
  fi

  printf 'Installed %s -> %s\n' "$id" "$dest_dir"
}

main() {
  local selected="${1:-}"
  local target id metadata archive

  if [ "$#" -gt 1 ]; then
    usage
    exit 2
  fi

  case "$selected" in
    -h|--help)
      usage
      exit 0
      ;;
  esac

  INSTALL_ROOT="$(remote_desktop_provider_dir)"
  printf 'Installing local remote desktop providers into %s\n' "$INSTALL_ROOT"

  while IFS= read -r id; do
    [ -n "$id" ] || continue
    metadata="${REPO_DIR}/extensions/remote-desktop/${id}/extension.build.json"
    [ -f "$metadata" ] || fail "missing extension build metadata: ${metadata}"
    target="$(target_for_provider "$metadata")"
    build_provider "$id" "$target" "$metadata"
    archive="$(package_provider "$id" "$target" | tail -n 1)"
    bash "${SCRIPT_DIR}/verify-remote-desktop-provider-package.sh" "$archive"
    install_packaged_provider "$id" "$target"
  done < <(provider_ids "$selected")
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  main "$@"
fi
