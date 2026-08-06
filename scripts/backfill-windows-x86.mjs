#!/usr/bin/env node

import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";

const repoRoot = path.resolve(import.meta.dirname, "..");
const windowsX86Target = "i686-pc-windows-msvc";
const extensionRoots = [
  "extensions/ipc",
  "extensions/remote-desktop",
  "extensions/mcp-helper",
  "extensions/acp-agent",
  "extensions/wasm",
  "extensions/language",
  "extensions/language-bundle",
];

try {
  main();
} catch (error) {
  console.error(`error: ${error.message}`);
  process.exit(1);
}

function main() {
  const [command, ...argv] = process.argv.slice(2);
  if (command === "matrix") {
    writeMatrix(argv[0] || "all");
    return;
  }
  if (command === "merge") {
    mergeReleaseAssets(parseOptions(argv));
    return;
  }
  throw new Error("usage: backfill-windows-x86.mjs matrix [extension|all] | merge [options]");
}

function writeMatrix(selector) {
  const marketplace = readJson(path.join(repoRoot, "manifest.json"));
  const releasesById = new Map(
    (marketplace.extensions || []).map((extension) => [extension.id, extension]),
  );
  const include = [];

  for (const metadata of loadAllExtensionMetadata()) {
    if (!metadata.targets.includes(windowsX86Target)) continue;
    if (selector !== "all" && metadata.id !== selector) continue;

    const release = releasesById.get(metadata.id);
    if (!release?.version || !release?.release_tag) {
      throw new Error(`missing marketplace version or release tag for ${metadata.id}`);
    }
    include.push({
      extension: metadata.id,
      package: metadata.package || "",
      manifest_path: metadata.manifest_path || "",
      kind: metadata.kind,
      language: metadata.language || "rust",
      version: release.version,
      release_tag: release.release_tag,
      target: windowsX86Target,
      package_file: artifactFileName(metadata, windowsX86Target),
    });
  }

  include.sort((left, right) => left.extension.localeCompare(right.extension));
  if (include.length === 0) {
    throw new Error(
      selector === "all"
        ? `no extensions declare ${windowsX86Target}`
        : `${selector} does not declare ${windowsX86Target}`,
    );
  }
  process.stdout.write(JSON.stringify({ include }));
}

function mergeReleaseAssets(options) {
  const extensionId = requiredOption(options, "extension");
  const version = requiredOption(options, "version");
  const releaseTag = requiredOption(options, "release-tag");
  const existingDir = path.resolve(requiredOption(options, "existing-dir"));
  const newPackage = path.resolve(requiredOption(options, "new-package"));
  const outputDir = path.resolve(requiredOption(options, "output-dir"));
  const force = options.force === true;

  if (pathsOverlap(existingDir, outputDir)) {
    throw new Error("output-dir must not overlap existing-dir");
  }
  if (isPathInside(outputDir, newPackage)) {
    throw new Error("new-package must not be inside output-dir");
  }

  const metadata = loadExtensionMetadata(extensionId);
  if (!metadata.targets.includes(windowsX86Target)) {
    throw new Error(`${extensionId} does not declare ${windowsX86Target}`);
  }

  const expectedNewPackage = artifactFileName(metadata, windowsX86Target);
  if (path.basename(newPackage) !== expectedNewPackage) {
    throw new Error(
      `unexpected Windows x86 package name: expected ${expectedNewPackage}, got ${path.basename(newPackage)}`,
    );
  }
  requireNonEmptyFile(newPackage);

  const oldManifestPath = path.join(existingDir, "extension-manifest.json");
  const oldChecksumsPath = path.join(existingDir, "sha256sums.txt");
  const oldManifest = readJson(oldManifestPath);
  const oldExtension = validateReleaseManifest(oldManifest, extensionId, version, releaseTag);
  const oldChecksums = readChecksums(oldChecksumsPath);

  for (const [target, artifact] of Object.entries(oldExtension.artifacts)) {
    if (!metadata.targets.includes(target)) {
      throw new Error(`old manifest contains undeclared target ${target}`);
    }
    const expectedFile = artifactFileName(metadata, target);
    if (artifact.file !== expectedFile) {
      throw new Error(`old manifest file mismatch for ${target}: expected ${expectedFile}`);
    }
    const filePath = path.join(existingDir, expectedFile);
    requireNonEmptyFile(filePath);
    const actualSha256 = sha256File(filePath);
    if (artifact.sha256 !== actualSha256) {
      throw new Error(`old manifest checksum mismatch for ${expectedFile}`);
    }
    if (oldChecksums.get(expectedFile) !== actualSha256) {
      throw new Error(`old checksum file mismatch for ${expectedFile}`);
    }
  }

  for (const target of metadata.targets) {
    if (target === windowsX86Target) continue;
    if (!oldExtension.artifacts[target]) {
      throw new Error(`old manifest is missing target ${target}`);
    }
  }

  const existingNewPackage = path.join(existingDir, expectedNewPackage);
  const newSha256 = sha256File(newPackage);
  let packageAction = "upload";
  if (fs.existsSync(existingNewPackage)) {
    requireNonEmptyFile(existingNewPackage);
    const existingSha256 = sha256File(existingNewPackage);
    if (existingSha256 === newSha256) {
      packageAction = "skip";
    } else if (force) {
      packageAction = "replace";
    } else {
      throw new Error(
        `${expectedNewPackage} already exists with a different checksum; rerun with --force to replace it`,
      );
    }
  }

  fs.rmSync(outputDir, { recursive: true, force: true });
  fs.mkdirSync(outputDir, { recursive: true });
  for (const target of metadata.targets) {
    const fileName = artifactFileName(metadata, target);
    const source = target === windowsX86Target
      ? newPackage
      : path.join(existingDir, fileName);
    requireNonEmptyFile(source);
    fs.copyFileSync(source, path.join(outputDir, fileName));
  }

  const checksumLines = metadata.targets.map((target) => {
    const fileName = artifactFileName(metadata, target);
    return `${sha256File(path.join(outputDir, fileName))}  ${fileName}`;
  });
  fs.writeFileSync(path.join(outputDir, "sha256sums.txt"), `${checksumLines.join("\n")}\n`);

  const generated = spawnSync(
    process.execPath,
    [path.join(repoRoot, "scripts/generate-marketplace-manifest.mjs")],
    {
      cwd: repoRoot,
      env: {
        ...process.env,
        ARTIFACT_DIR: outputDir,
        EXTENSION_ID: extensionId,
        EXTENSION_VERSION: version,
        RELEASE_TAG: releaseTag,
        TARGETS: metadata.targets.join(","),
      },
      encoding: "utf8",
    },
  );
  if (generated.error) throw generated.error;
  if (generated.status !== 0) {
    throw new Error(
      `marketplace manifest generation failed: ${(generated.stderr || generated.stdout).trim()}`,
    );
  }

  const mergedManifest = readJson(path.join(outputDir, "extension-manifest.json"));
  const mergedExtension = validateReleaseManifest(
    mergedManifest,
    extensionId,
    version,
    releaseTag,
  );
  for (const target of Object.keys(oldExtension.artifacts)) {
    if (!mergedExtension.artifacts[target]) {
      throw new Error(`merged manifest dropped old target ${target}`);
    }
  }
  for (const target of metadata.targets) {
    const artifact = mergedExtension.artifacts[target];
    if (!artifact) throw new Error(`merged manifest is missing target ${target}`);
    const expectedFile = artifactFileName(metadata, target);
    if (artifact.file !== expectedFile) {
      throw new Error(`merged manifest file mismatch for ${target}`);
    }
    if (artifact.sha256 !== sha256File(path.join(outputDir, expectedFile))) {
      throw new Error(`merged manifest checksum mismatch for ${expectedFile}`);
    }
  }

  process.stdout.write(JSON.stringify({
    extension: extensionId,
    version,
    release_tag: releaseTag,
    package_file: expectedNewPackage,
    package_action: packageAction,
    target_count: metadata.targets.length,
  }));
}

function parseOptions(argv) {
  const options = {};
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === "--force") {
      options.force = true;
      continue;
    }
    if (!argument.startsWith("--")) {
      throw new Error(`unexpected argument: ${argument}`);
    }
    const name = argument.slice(2);
    const value = argv[++index];
    if (!value || value.startsWith("--")) {
      throw new Error(`${argument} requires a value`);
    }
    options[name] = value;
  }
  return options;
}

function requiredOption(options, name) {
  const value = options[name];
  if (!value || !value.trim()) throw new Error(`--${name} is required`);
  return value.trim();
}

function loadAllExtensionMetadata() {
  const metadata = [];
  for (const root of extensionRoots) {
    const absoluteRoot = path.join(repoRoot, root);
    if (!fs.existsSync(absoluteRoot)) continue;
    for (const id of fs.readdirSync(absoluteRoot).sort()) {
      const file = path.join(absoluteRoot, id, "extension.build.json");
      if (!fs.existsSync(file)) continue;
      metadata.push(validateMetadata(readJson(file), file));
    }
  }
  return metadata;
}

function loadExtensionMetadata(id) {
  const metadata = loadAllExtensionMetadata().find((entry) => entry.id === id);
  if (!metadata) throw new Error(`unknown extension id: ${id}`);
  return metadata;
}

function validateMetadata(metadata, file) {
  if (
    !metadata.id
    || !metadata.kind
    || !metadata.path
    || !Array.isArray(metadata.targets)
    || metadata.targets.length === 0
  ) {
    throw new Error(`invalid extension build metadata: ${file}`);
  }
  return metadata;
}

function validateReleaseManifest(manifest, extensionId, version, releaseTag) {
  if (!Array.isArray(manifest.extensions) || manifest.extensions.length !== 1) {
    throw new Error("extension manifest must contain exactly one extension");
  }
  const extension = manifest.extensions[0];
  if (extension.id !== extensionId) {
    throw new Error(`extension manifest id mismatch: expected ${extensionId}`);
  }
  if (extension.version !== version) {
    throw new Error(`extension manifest version mismatch: expected ${version}`);
  }
  if (extension.release_tag !== releaseTag || manifest.release_version !== releaseTag) {
    throw new Error(`extension manifest release tag mismatch: expected ${releaseTag}`);
  }
  if (!extension.artifacts || typeof extension.artifacts !== "object") {
    throw new Error("extension manifest artifacts are missing");
  }
  return extension;
}

function artifactFileName(metadata, target) {
  switch (metadata.kind) {
    case "database_driver":
      return `${metadata.id}-driver-${target}.tar.gz`;
    case "remote_desktop_provider":
      return `${metadata.id}-remote-desktop-provider-${target}.tar.gz`;
    case "mcp_helper":
      return `${metadata.id}-mcp-helper-${target}.tar.gz`;
    case "acp_agent":
      return `${metadata.id}-acp-agent-${target}.tar.gz`;
    case "composite":
      return `${metadata.id}-composite-${target}.tar.gz`;
    case "language":
      return `${metadata.id}-language-${target}.tar.gz`;
    case "language_bundle":
      return `${metadata.id}-language-bundle-${target}.tar.gz`;
    default:
      throw new Error(`unsupported extension kind: ${metadata.kind}`);
  }
}

function readChecksums(file) {
  requireNonEmptyFile(file);
  const checksums = new Map();
  for (const line of fs.readFileSync(file, "utf8").split(/\r?\n/).filter(Boolean)) {
    const match = line.match(/^([0-9a-f]{64})\s+(.+)$/);
    if (!match) throw new Error(`invalid checksum line: ${line}`);
    if (checksums.has(match[2])) throw new Error(`duplicate checksum entry: ${match[2]}`);
    checksums.set(match[2], match[1]);
  }
  return checksums;
}

function sha256File(file) {
  return createHash("sha256").update(fs.readFileSync(file)).digest("hex");
}

function requireNonEmptyFile(file) {
  if (!fs.statSync(file, { throwIfNoEntry: false })?.isFile() || fs.statSync(file).size === 0) {
    throw new Error(`missing or empty file: ${file}`);
  }
}

function pathsOverlap(left, right) {
  return isPathInside(left, right) || isPathInside(right, left);
}

function isPathInside(parent, candidate) {
  const relative = path.relative(parent, candidate);
  return relative === "" || (!relative.startsWith("..") && !path.isAbsolute(relative));
}

function readJson(file) {
  return JSON.parse(fs.readFileSync(file, "utf8"));
}
