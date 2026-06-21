import fs from "node:fs";
import path from "node:path";

const artifactDir = process.env.ARTIFACT_DIR || "artifacts";
const version = requiredEnv("EXTENSION_VERSION");
const releaseTag = requiredEnv("RELEASE_TAG");
const extensionId = requiredEnv("EXTENSION_ID");
const metadata = loadExtensionMetadata(extensionId);
const sourceManifest = loadSourceManifest(metadata);

const targets = selectedTargets(metadata.targets);

const checksums = readChecksums(path.join(artifactDir, "sha256sums.txt"));
const artifacts = {};

for (const target of targets) {
  const fileName = artifactFileName(metadata, target);
  artifacts[target] = {
    file: fileName,
    sha256: checksumFor(checksums, fileName),
  };
}

const extensionManifest = {
  schema_version: 2,
  release_version: releaseTag,
};

const extensionEntry = {
  id: extensionId,
  kind: metadata.kind,
  name: sourceManifest.name || extensionId,
  version,
  release_tag: releaseTag,
  description: sourceManifest.description || "",
  artifacts,
};
extensionManifest.extensions = [extensionEntry];

fs.mkdirSync(artifactDir, { recursive: true });
fs.writeFileSync(
  path.join(artifactDir, "extension-manifest.json"),
  `${JSON.stringify(extensionManifest, null, 2)}\n`,
);

function requiredEnv(name) {
  const value = process.env[name];
  if (!value || !value.trim()) {
    throw new Error(`${name} is required`);
  }
  return value.trim();
}

function readChecksums(filePath) {
  const lines = fs.readFileSync(filePath, "utf8").trim().split(/\n/).filter(Boolean);
  return new Map(lines.map((line) => {
    const [sha256, fileName] = line.trim().split(/\s+/, 2);
    return [fileName, sha256];
  }));
}

function checksumFor(checksums, fileName) {
  const sha256 = checksums.get(fileName);
  if (!sha256 || !/^[0-9a-f]{64}$/.test(sha256)) {
    throw new Error(`missing checksum for ${fileName}`);
  }
  return sha256;
}

function loadExtensionMetadata(id) {
  const roots = ["extensions/ipc", "extensions/remote-desktop", "extensions/wasm", "extensions/language"];
  for (const root of roots) {
    const file = path.join(root, id, "extension.build.json");
    if (!fs.existsSync(file)) continue;
    const data = JSON.parse(fs.readFileSync(file, "utf8"));
    if (!data.id || !data.kind || !data.path || !Array.isArray(data.targets)) {
      throw new Error(`invalid extension build metadata: ${file}`);
    }
    return data;
  }
  throw new Error(`unknown extension id: ${id}`);
}

function loadSourceManifest(metadata) {
  const fileName = manifestFileName(metadata.kind);
  return JSON.parse(fs.readFileSync(path.join(metadata.path, fileName), "utf8"));
}

function manifestFileName(kind) {
  switch (kind) {
    case "database_driver":
      return "driver.json";
    case "remote_desktop_provider":
      return "remote_desktop_provider.json";
    default:
      throw new Error(`unsupported extension kind for marketplace manifest: ${kind}`);
  }
}

function artifactFileName(metadata, target) {
  switch (metadata.kind) {
    case "database_driver":
      return `${metadata.id}-driver-${target}.tar.gz`;
    case "remote_desktop_provider":
      return `${metadata.id}-remote-desktop-provider-${target}.tar.gz`;
    default:
      throw new Error(`unsupported extension kind for artifact naming: ${metadata.kind}`);
  }
}

function selectedTargets(defaultTargets) {
  const rawTargets = process.env.TARGETS;
  if (!rawTargets || !rawTargets.trim()) {
    return defaultTargets;
  }

  const requested = [...new Set(rawTargets.split(",").map((target) => target.trim()).filter(Boolean))];
  const known = new Set(defaultTargets);
  const unknown = requested.filter((target) => !known.has(target));
  if (unknown.length > 0) {
    throw new Error(`unknown target(s) for ${extensionId}: ${unknown.join(", ")}`);
  }
  if (requested.length === 0) {
    throw new Error("TARGETS did not contain any targets");
  }
  return requested;
}
