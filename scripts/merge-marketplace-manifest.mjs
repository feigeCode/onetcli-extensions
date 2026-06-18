import fs from "node:fs";
import path from "node:path";

const currentPath = requiredEnv("CURRENT_MANIFEST");
const outputPath = requiredEnv("OUTPUT_MANIFEST");
const existingPath = process.env.EXISTING_MANIFEST || "";

const current = readManifest(currentPath, true);
const existing = existingPath && fs.existsSync(existingPath)
  ? readManifest(existingPath, false)
  : { schema_version: current.schema_version || 1, release_version: "", extensions: [] };

const entries = new Map();
for (const entry of existing.extensions || []) {
  if (entry && entry.id) entries.set(entry.id, entry);
}
for (const entry of current.extensions || []) {
  if (!entry || !entry.id) {
    throw new Error(`current manifest contains an extension without id: ${currentPath}`);
  }
  entries.set(entry.id, entry);
}

const merged = {
  schema_version: current.schema_version || existing.schema_version || 1,
  release_version: current.release_version || existing.release_version || "",
  extensions: Array.from(entries.values()).sort((a, b) => a.id.localeCompare(b.id)),
};

fs.mkdirSync(path.dirname(outputPath), { recursive: true });
fs.writeFileSync(outputPath, `${JSON.stringify(merged, null, 2)}\n`);

function requiredEnv(name) {
  const value = process.env[name];
  if (!value || !value.trim()) {
    throw new Error(`${name} is required`);
  }
  return value.trim();
}

function readManifest(filePath, requireExtension) {
  const manifest = JSON.parse(fs.readFileSync(filePath, "utf8"));
  if (!Array.isArray(manifest.extensions)) {
    throw new Error(`manifest.extensions must be an array: ${filePath}`);
  }
  if (requireExtension && manifest.extensions.length === 0) {
    throw new Error(`manifest must contain at least one extension: ${filePath}`);
  }
  return manifest;
}
