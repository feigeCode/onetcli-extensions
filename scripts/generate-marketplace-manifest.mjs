import fs from "node:fs";
import path from "node:path";

const artifactDir = process.env.ARTIFACT_DIR || "artifacts";
const version = requiredEnv("EXTENSION_VERSION");
const releaseTag = requiredEnv("RELEASE_TAG");
const extensionId = requiredEnv("EXTENSION_ID");
const repository = requiredEnv("GITHUB_REPOSITORY");

const targets = [
  "aarch64-apple-darwin",
  "x86_64-apple-darwin",
  "x86_64-unknown-linux-gnu",
  "x86_64-pc-windows-msvc",
];

const checksums = readChecksums(path.join(artifactDir, "sha256sums.txt"));
const assetUrls = {};
const fallbackAssetUrls = {};
const sha256s = {};

for (const target of targets) {
  const fileName = `duckdb-driver-${target}.tar.gz`;
  assetUrls[target] = `${extensionId}/${version}/${fileName}`;
  fallbackAssetUrls[target] = `https://github.com/${repository}/releases/download/${releaseTag}/${fileName}`;
  sha256s[target] = checksumFor(checksums, fileName);
}

const manifest = {
  schema_version: 1,
  release_version: releaseTag,
  extensions: [],
};

const currentEntry = {
  id: extensionId,
  kind: "database_driver",
  name: "DuckDB",
  version,
  description: "DuckDB embedded analytical database IPC driver",
  asset_urls: assetUrls,
  fallback_asset_urls: fallbackAssetUrls,
  sha256s,
};

fs.mkdirSync(artifactDir, { recursive: true });
fs.mkdirSync("manifest/entries", { recursive: true });
fs.writeFileSync(
  `manifest/entries/${extensionId}.json`,
  `${JSON.stringify(currentEntry, null, 2)}\n`,
);

manifest.extensions = fs
  .readdirSync("manifest/entries")
  .filter((fileName) => fileName.endsWith(".json"))
  .sort()
  .map((fileName) =>
    JSON.parse(fs.readFileSync(path.join("manifest/entries", fileName), "utf8")),
  );

fs.writeFileSync(
  path.join(artifactDir, "extension-manifest.json"),
  `${JSON.stringify(manifest, null, 2)}\n`,
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
