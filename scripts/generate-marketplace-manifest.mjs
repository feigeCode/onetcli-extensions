import fs from "node:fs";
import path from "node:path";

const artifactDir = process.env.ARTIFACT_DIR || "artifacts";
const version = requiredEnv("EXTENSION_VERSION");
const releaseTag = requiredEnv("RELEASE_TAG");
const extensionId = requiredEnv("EXTENSION_ID");
const metadata = loadExtensionMetadata(extensionId);
const driverJson = JSON.parse(
  fs.readFileSync(path.join(metadata.path, "driver.json"), "utf8"),
);

const targets = selectedTargets(metadata.targets);

const checksums = readChecksums(path.join(artifactDir, "sha256sums.txt"));
const artifacts = {};

for (const target of targets) {
  const fileName = `${extensionId}-driver-${target}.tar.gz`;
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
  name: driverJson.name || extensionId,
  version,
  release_tag: releaseTag,
  description: driverJson.description || "",
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
  const roots = ["extensions/ipc", "extensions/wasm", "extensions/language"];
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
