#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -ne 1 ]; then
  echo "Usage: $0 <composite-package.tar.gz>" >&2
  exit 2
fi

PACKAGE="$1"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

tar xzf "$PACKAGE" -C "$TMP_DIR"

MANIFEST="${TMP_DIR}/extension.json"
if [ ! -f "$MANIFEST" ]; then
  echo "Missing root-level extension.json" >&2
  exit 1
fi

node <<'NODE' "$MANIFEST" "$TMP_DIR"
const fs = require("fs");
const path = require("path");

const manifestPath = process.argv[2];
const packageRoot = process.argv[3];
const manifest = JSON.parse(fs.readFileSync(manifestPath, "utf8"));

for (const key of ["id", "name", "version", "contributes"]) {
  if (!manifest[key]) {
    console.error(`extension.json missing ${key}`);
    process.exit(1);
  }
}

if (Object.keys(manifest.contributes).length === 0) {
  console.error("extension.json contributes must not be empty");
  process.exit(1);
}

const wasmRuntimes = manifest.runtime?.wasm || [];
for (const runtime of wasmRuntimes) {
  if (!runtime.id || !runtime.module || runtime.kind !== "component") {
    console.error("runtime.wasm entries must declare id, module, and kind=component");
    process.exit(1);
  }
  if (path.isAbsolute(runtime.module) || runtime.module.includes("..")) {
    console.error(`runtime.wasm.module must stay inside package: ${runtime.module}`);
    process.exit(1);
  }
  const modulePath = path.join(packageRoot, runtime.module);
  if (!fs.existsSync(modulePath)) {
    console.error(`runtime.wasm.module not found: ${runtime.module}`);
    process.exit(1);
  }
}

const ipcRuntimes = manifest.runtime?.ipc || [];
const ipcIds = new Set();
for (const runtime of ipcRuntimes) {
  if (!runtime.id || !runtime.entry?.command) {
    console.error("runtime.ipc entries must declare id and entry.command");
    process.exit(1);
  }
  if (ipcIds.has(runtime.id)) {
    console.error(`duplicate runtime.ipc id: ${runtime.id}`);
    process.exit(1);
  }
  ipcIds.add(runtime.id);
  const command = runtime.entry.command;
  if (path.isAbsolute(command) || command.includes("..")) {
    console.error(`runtime.ipc entry.command must stay inside package: ${command}`);
    process.exit(1);
  }
  const commandPath = path.join(packageRoot, command.replace(/^\.\//, ""));
  if (!fs.existsSync(commandPath)) {
    console.error(`runtime.ipc entry.command not found: ${command}`);
    process.exit(1);
  }
  if (!(manifest.permissions || []).includes(`spawn:${command}`)) {
    console.error(`runtime.ipc entry.command is missing spawn permission: spawn:${command}`);
    process.exit(1);
  }
}

const shellViews = manifest.contributes?.shellViews || [];
const shellViewById = new Map();
for (const view of shellViews) {
  for (const key of ["id", "title", "entry"]) {
    if (!view[key]) {
      console.error(`shell view missing ${key}`);
      process.exit(1);
    }
  }
  if ((view.surface || "tab") !== "tab") {
    console.error(`shell view ${view.id} has unsupported surface`);
    process.exit(1);
  }
  if (path.isAbsolute(view.entry) || view.entry.includes("..")) {
    console.error(`shell view entry must stay inside package: ${view.entry}`);
    process.exit(1);
  }
  if (!fs.existsSync(path.join(packageRoot, view.entry))) {
    console.error(`shell view entry not found: ${view.entry}`);
    process.exit(1);
  }
  for (const runtimeId of Object.values(view.backends || {})) {
    if (!ipcIds.has(runtimeId)) {
      console.error(`shell view ${view.id} references unknown IPC runtime: ${runtimeId}`);
      process.exit(1);
    }
  }
  if (!Array.isArray(view.modules)) {
    console.error(`shell view ${view.id} modules must be an array`);
    process.exit(1);
  }
  shellViewById.set(view.id, view);
}
if (shellViews.length > 0) {
  if (!(manifest.permissions || []).includes("shell:exec")) {
    console.error("shell views require shell:exec permission");
    process.exit(1);
  }
  if (!manifest.engines?.gpui_shell || !manifest.api?.shell) {
    console.error("shell views require engines.gpui_shell and api.shell");
    process.exit(1);
  }
}

const connections = manifest.contributes?.connections || [];
const connectionIds = new Set();
for (const connection of connections) {
  for (const key of ["id", "label", "runtimeId", "resourceType", "form"]) {
    if (!connection[key]) {
      console.error(`connection contribution missing ${key}`);
      process.exit(1);
    }
  }
  if (connectionIds.has(connection.id)) {
    console.error(`duplicate connection contribution id: ${connection.id}`);
    process.exit(1);
  }
  connectionIds.add(connection.id);
  if (!ipcIds.has(connection.runtimeId)) {
    console.error(`connection ${connection.id} references unknown IPC runtime: ${connection.runtimeId}`);
    process.exit(1);
  }
  if (connection.shellViewId) {
    const view = shellViewById.get(connection.shellViewId);
    if (!view) {
      console.error(`connection ${connection.id} references unknown shell view: ${connection.shellViewId}`);
      process.exit(1);
    }
    if (view.singleton || !view.modules.includes("context") || !view.modules.includes("resource")) {
      console.error(`connection shell view ${view.id} must be non-singleton with context and resource modules`);
      process.exit(1);
    }
    if (!Object.values(view.backends || {}).includes(connection.runtimeId)) {
      console.error(`connection shell view ${view.id} must expose runtime ${connection.runtimeId}`);
      process.exit(1);
    }
  }
  const tabs = connection.form.tabs || [];
  const fieldIds = new Set();
  let hasSecrets = false;
  for (const tab of tabs) {
    if (!tab.id || !tab.label || !Array.isArray(tab.fields)) {
      console.error(`connection ${connection.id} has an invalid form tab`);
      process.exit(1);
    }
    for (const field of tab.fields) {
      if (!field.id || !field.label || !field.fieldType || fieldIds.has(field.id)) {
        console.error(`connection ${connection.id} has an invalid or duplicate form field`);
        process.exit(1);
      }
      fieldIds.add(field.id);
      hasSecrets ||= field.secret === true;
      if ((field.secret === true) !== (field.fieldType === "Password")) {
        console.error(`connection secret field ${field.id} must use Password and secret=true`);
        process.exit(1);
      }
    }
  }
  if (hasSecrets && !(manifest.permissions || []).includes("secrets:read:self.*")) {
    console.error(`connection ${connection.id} secrets require secrets:read:self.* permission`);
    process.exit(1);
  }
}

const importers = manifest.contributes?.connectionImporters || [];

for (const importer of importers) {
  for (const key of ["id", "runtimeId", "displayName"]) {
    if (!importer[key]) {
      console.error(`connection importer missing ${key}`);
      process.exit(1);
    }
  }
  if (!Array.isArray(importer.outputKinds) || importer.outputKinds.length === 0) {
    console.error(`connection importer ${importer.id} missing outputKinds`);
    process.exit(1);
  }
}

const editors = manifest.contributes?.remoteFileEditors || [];
for (const editor of editors) {
  for (const key of ["id", "displayName", "command"]) {
    if (!editor[key]) {
      console.error(`remote file editor missing ${key}`);
      process.exit(1);
    }
  }
  if (!Array.isArray(editor.command.programCandidates)
      || editor.command.programCandidates.length === 0) {
    console.error(`remote file editor ${editor.id} missing programCandidates`);
    process.exit(1);
  }
  if (editor.command.args && !Array.isArray(editor.command.args)) {
    console.error(`remote file editor ${editor.id} args must be an array`);
    process.exit(1);
  }
}

if (importers.length === 0 && editors.length === 0 && wasmRuntimes.length === 0
    && ipcRuntimes.length === 0 && shellViews.length === 0 && connections.length === 0) {
  console.error("extension.json has no supported composite contributions");
  process.exit(1);
}
NODE

echo "Verified ${PACKAGE}"
