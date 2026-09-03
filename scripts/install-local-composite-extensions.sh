#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
ARTIFACT_DIR="${REPO_DIR}/target/local-extension-artifacts"

fail() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

json_value() {
  local file="$1"
  local expression="$2"
  node -e "const fs = require('fs'); const data = JSON.parse(fs.readFileSync(process.argv[1], 'utf8')); const value = ${expression}; if (Array.isArray(value)) process.stdout.write(value.join(' ')); else process.stdout.write(value == null ? '' : String(value));" "$file"
}

platform_config_root() {
  case "$(uname -s)" in
    CYGWIN*|MINGW*|MSYS*)
      [ -n "${APPDATA:-}" ] || fail "APPDATA is not set"
      if command -v cygpath >/dev/null 2>&1; then cygpath -u "$APPDATA"; else printf '%s\n' "$APPDATA"; fi
      ;;
    *)
      [ -n "${HOME:-}" ] || fail "HOME is not set"
      printf '%s/.config\n' "$HOME"
      ;;
  esac
}

install_root() {
  if [ -n "${NAVOP_COMPOSITE_EXTENSION_DIR:-}" ]; then
    printf '%s\n' "$NAVOP_COMPOSITE_EXTENSION_DIR"
    return
  fi
  if [ -n "${ONETCLI_COMPOSITE_EXTENSION_DIR:-}" ]; then
    printf '%s\n' "$ONETCLI_COMPOSITE_EXTENSION_DIR"
    return
  fi
  local root current legacy
  root="$(platform_config_root)"
  current="${root}/navop"
  legacy="${root}/one-hub"
  if [ -d "$legacy" ] && [ ! -f "${current}/.one-hub-migration-complete" ]; then
    printf '%s/extensions/composite\n' "$legacy"
  else
    printf '%s/extensions/composite\n' "$current"
  fi
}

metadata_path() {
  local id="$1"
  for root in extensions/composite extensions/wasm; do
    if [ -f "${REPO_DIR}/${root}/${id}/extension.build.json" ]; then
      printf '%s\n' "${REPO_DIR}/${root}/${id}/extension.build.json"
      return
    fi
  done
  fail "unknown composite extension: ${id}"
}

extension_ids() {
  if [ -n "${1:-}" ]; then
    printf '%s\n' "$1"
    return
  fi
  for root in "${REPO_DIR}/extensions/composite" "${REPO_DIR}/extensions/wasm"; do
    [ -d "$root" ] || continue
    for metadata in "$root"/*/extension.build.json; do
      [ -f "$metadata" ] && basename "$(dirname "$metadata")"
    done
  done
}

target_for() {
  local metadata="$1"
  local host targets target
  host="$(rustc -vV | sed -n 's/^host: //p')"
  targets="$(json_value "$metadata" 'data.targets || []')"
  for target in $targets; do
    if [ "$target" = "$host" ]; then printf '%s\n' "$host"; return; fi
  done
  for target in $targets; do
    if [ "$target" = "universal" ]; then printf '%s\n' universal; return; fi
  done
  fail "$(json_value "$metadata" 'data.id') does not support ${host} or universal"
}

install_one() {
  local id="$1"
  local metadata source_dir manifest version target package_dir manifest_id destination root backup
  metadata="$(metadata_path "$id")"
  source_dir="$(dirname "$metadata")"
  manifest="${source_dir}/extension.json"
  version="$(json_value "$manifest" 'data.version')"
  target="$(target_for "$metadata")"
  node "${SCRIPT_DIR}/release-driver.mjs" "$id" "$version" --target "$target" --artifact-dir "$ARTIFACT_DIR"
  package_dir="${REPO_DIR}/target/extension-packages/${target}/${id}"
  manifest_id="$(json_value "${package_dir}/extension.json" 'data.id')"
  root="$(install_root)"
  destination="${root}/${manifest_id}"
  mkdir -p "$root"
  if [ -e "$destination" ]; then
    backup="${root}/.backups/${manifest_id}.$(date +%Y%m%d%H%M%S)"
    mkdir -p "$(dirname "$backup")"
    mv "$destination" "$backup"
  fi
  mkdir -p "$destination"
  cp -R "${package_dir}/." "${destination}/"
  printf 'Installed %s -> %s\n' "$id" "$destination"
}

main() {
  if [ "$#" -gt 1 ]; then
    fail "usage: scripts/install-local-composite-extensions.sh [extension-id]"
  fi
  while IFS= read -r id; do
    [ -n "$id" ] && install_one "$id"
  done < <(extension_ids "${1:-}")
}

main "$@"
