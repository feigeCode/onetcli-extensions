import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";

const [baseSha, headSha] = process.argv.slice(2);
if (!baseSha || !headSha) {
  throw new Error("Usage: node scripts/changed-extensions.mjs <base-sha> <head-sha>");
}

const extensions = loadExtensions();
const changedFiles = execFileSync(
  "git",
  ["diff", "--name-only", baseSha, headSha],
  { encoding: "utf8" },
)
  .trim()
  .split(/\n/)
  .filter(Boolean);

const changedIds = new Set();
for (const file of changedFiles) {
  for (const extension of extensions) {
    if (extensionMatchesPath(extension, file)) {
      changedIds.add(extension.id);
    }
  }

  if (isRustSharedPath(file)) {
    addExtensionsByLanguage(changedIds, extensions, "rust");
  }
  if (isGoSharedPath(file)) {
    addExtensionsByLanguage(changedIds, extensions, "go");
  }
}

const include = [];
for (const extension of extensions) {
  if (!changedIds.has(extension.id)) continue;
  include.push({
    extension: extension.id,
    package: extension.package || "",
    manifest_path: extension.manifest_path || "",
    kind: extension.kind,
    language: extension.language || "rust",
    os: "ubuntu-latest",
  });
}

process.stdout.write(`${JSON.stringify({ include })}\n`);

function loadExtensions() {
  const roots = ["extensions/ipc", "extensions/remote-desktop", "extensions/wasm", "extensions/language"];
  const result = [];
  for (const root of roots) {
    if (!fs.existsSync(root)) continue;
    for (const name of fs.readdirSync(root)) {
      const file = path.join(root, name, "extension.build.json");
      if (!fs.existsSync(file)) continue;
      const data = JSON.parse(fs.readFileSync(file, "utf8"));
      if (!data.id || !data.path || !Array.isArray(data.targets)) {
        throw new Error(`invalid extension build metadata: ${file}`);
      }
      result.push(data);
    }
  }
  return result;
}

function extensionMatchesPath(extension, file) {
  return extensionSourcePaths(extension).some((sourcePath) =>
    file === sourcePath || file.startsWith(`${sourcePath}/`),
  );
}

function extensionSourcePaths(extension) {
  const paths = [extension.path, ...(extension.source_paths || [])];
  if (typeof extension.package === "string" && extension.package.includes("/")) {
    paths.push(extension.package.replace(/^\.\//, ""));
  }
  return [...new Set(paths.filter(Boolean).map((item) => item.replace(/\/$/, "")))];
}

function addExtensionsByLanguage(changedIds, extensions, language) {
  for (const extension of extensions) {
    if ((extension.language || "rust") === language) {
      changedIds.add(extension.id);
    }
  }
}

function isRustSharedPath(file) {
  return file === "Cargo.toml" || file === "Cargo.lock" || file === "rustfmt.toml";
}

function isGoSharedPath(file) {
  return (
    file === "go.mod" ||
    file === "go.sum" ||
    file.startsWith("internal/") ||
    file.startsWith("vendor/")
  );
}
