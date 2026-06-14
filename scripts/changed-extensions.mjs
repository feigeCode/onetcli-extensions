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

const sharedChange = changedFiles.some((file) =>
  file.startsWith("scripts/") ||
  file.startsWith("crates/") ||
  file.startsWith(".github/workflows/"),
);

const changedIds = new Set();
if (sharedChange) {
  for (const extension of extensions) changedIds.add(extension.id);
} else {
  for (const file of changedFiles) {
    for (const extension of extensions) {
      if (file === extension.path || file.startsWith(`${extension.path}/`)) {
        changedIds.add(extension.id);
      }
    }
  }
}

const include = [];
for (const extension of extensions) {
  if (!changedIds.has(extension.id)) continue;
  for (const target of extension.targets) {
    include.push({
      extension: extension.id,
      package: extension.package || "",
      kind: extension.kind,
      target,
      os: runnerForTarget(target),
    });
  }
}

process.stdout.write(`${JSON.stringify({ include })}\n`);

function loadExtensions() {
  const roots = ["extensions/ipc", "extensions/wasm", "extensions/language"];
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

function runnerForTarget(target) {
  if (target === "universal") return "ubuntu-latest";
  if (target.includes("apple-darwin")) {
    return target.startsWith("x86_64") ? "macos-15-intel" : "macos-latest";
  }
  if (target.includes("windows")) return "windows-latest";
  return "ubuntu-latest";
}
