#!/usr/bin/env node
import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";

const repoRoot = path.resolve(import.meta.dirname, "..");

main();

function main() {
  const args = parseArgs(process.argv.slice(2));
  if (args.help) {
    printUsage();
    return;
  }

  if (!args.extensionId || !args.version) {
    printUsage();
    process.exit(2);
  }

  const metadata = loadExtensionMetadata(args.extensionId);
  if (!["database_driver", "remote_desktop_provider", "mcp_helper", "acp_agent", "composite", "language", "language_bundle"].includes(metadata.kind)) {
    fail(`unsupported extension kind: ${metadata.kind}`);
  }

  const targets = selectedTargets(metadata, args.targets);
  const artifactDir = path.resolve(repoRoot, args.artifactDir);
  const releaseTag = args.releaseTag || `${metadata.releaseTagPrefix || `${metadata.id}-v`}${args.version}`;

  fs.mkdirSync(artifactDir, { recursive: true });

  console.log(`Releasing ${metadata.id} ${args.version}`);
  console.log(`Targets: ${targets.join(", ")}`);
  console.log(`Artifacts: ${path.relative(repoRoot, artifactDir) || "."}`);

  for (const target of targets) {
    if (!args.skipBuild) {
      buildDriver(metadata, target);
    } else {
      console.log(`Skipping build for ${metadata.id} (${target})`);
    }
    packageDriver(metadata, target, artifactDir, args.version);
    verifyPackage(metadata, target, artifactDir);
  }

  writeChecksums(metadata, targets, artifactDir);
  generateExtensionManifest(metadata, targets, artifactDir, args.version, releaseTag);
  writeReleaseMetadata(artifactDir, metadata.id, args.version, releaseTag);

  console.log("Release artifacts ready:");
  console.log(`  ${path.relative(repoRoot, artifactDir) || "."}`);
}

function parseArgs(argv) {
  const args = {
    artifactDir: "artifacts",
    targets: [],
    skipBuild: false,
    help: false,
    releaseTag: "",
  };
  const positionals = [];

  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    switch (arg) {
      case "-h":
      case "--help":
        args.help = true;
        break;
      case "--artifact-dir":
        args.artifactDir = requiredValue(argv, ++i, arg);
        break;
      case "--target":
        args.targets.push(...splitTargets(requiredValue(argv, ++i, arg)));
        break;
      case "--skip-build":
        args.skipBuild = true;
        break;
      case "--release-tag":
        args.releaseTag = requiredValue(argv, ++i, arg);
        break;
      default:
        if (arg.startsWith("--")) {
          fail(`unknown option: ${arg}`, 2);
        }
        positionals.push(arg);
        break;
    }
  }

  if (positionals.length > 2) {
    fail(`too many arguments: ${positionals.slice(2).join(" ")}`, 2);
  }

  args.extensionId = positionals[0] || "";
  args.version = positionals[1] || "";
  return args;
}

function printUsage() {
  console.log(`Usage: node scripts/release-driver.mjs <driver-id> <version> [options]

Build, package, verify, and assemble release artifacts for one extension.

Options:
  --target <target>       Build only this target. May be repeated or comma-separated.
  --artifact-dir <dir>    Output directory. Defaults to artifacts.
  --release-tag <tag>     Override the release tag. Defaults to <driver-id>-v<version>.
  --skip-build            Package already-staged binaries from target/<target>/release.
  -h, --help              Show this help.

Examples:
  node scripts/release-driver.mjs duckdb 1.0.0
  node scripts/release-driver.mjs dm 0.4.0 --target x86_64-unknown-linux-gnu
  node scripts/release-driver.mjs gbase8s 0.7.0 --artifact-dir artifacts/gbase8s-0.7.0
`);
}

function requiredValue(argv, index, option) {
  const value = argv[index];
  if (!value || value.startsWith("--")) {
    fail(`${option} requires a value`, 2);
  }
  return value;
}

function splitTargets(value) {
  return value.split(",").map((target) => target.trim()).filter(Boolean);
}

function loadExtensionMetadata(id) {
  const roots = ["extensions/ipc", "extensions/remote-desktop", "extensions/mcp-helper", "extensions/acp-agent", "extensions/composite", "extensions/wasm", "extensions/language", "extensions/language-bundle"];
  let file = "";
  for (const root of roots) {
    const candidate = path.join(repoRoot, root, id, "extension.build.json");
    if (fs.existsSync(candidate)) {
      file = candidate;
      break;
    }
  }
  if (!file) {
    fail(`unknown extension id: ${id}`);
  }
  const metadata = JSON.parse(fs.readFileSync(file, "utf8"));
  for (const key of ["id", "kind", "path", "targets"]) {
    if (!metadata[key]) {
      fail(`invalid extension build metadata ${file}: missing ${key}`);
    }
  }
  if (!Array.isArray(metadata.targets) || metadata.targets.length === 0) {
    fail(`invalid extension build metadata ${file}: targets must be a non-empty array`);
  }
  if (metadata.id !== id) {
    fail(`extension metadata id mismatch: expected ${id}, got ${metadata.id}`);
  }
  return metadata;
}

function selectedTargets(metadata, requestedTargets) {
  if (requestedTargets.length === 0) {
    return metadata.targets;
  }

  const known = new Set(metadata.targets);
  const selected = [...new Set(requestedTargets)];
  const unknown = selected.filter((target) => !known.has(target));
  if (unknown.length > 0) {
    fail(`${metadata.id} does not declare target(s): ${unknown.join(", ")}`);
  }
  return selected;
}

function buildDriver(metadata, target) {
  const language = metadata.language || "rust";
  console.log(`Building ${metadata.id} (${language}, ${target})`);

  if (language === "rust") {
    const packageName = metadata.package || metadata.binary || `${metadata.id}_driver`;
    const args = metadata.manifest_path
      ? ["build", "--release", "--manifest-path", metadata.manifest_path, "--target", target]
      : ["build", "--release", "-p", packageName, "--target", target];
    run("cargo", args, {
      env: {
        ...rustBuildEnv(target),
        ...(metadata.manifest_path ? { CARGO_TARGET_DIR: path.join(repoRoot, "target") } : {}),
      },
    });
    return;
  }

  if (language === "rust-wasm") {
    if (target !== "universal") {
      fail(`rust-wasm extensions must declare the universal target, got ${target}`);
    }
    const packageName = metadata.package || metadata.id;
    run("cargo", ["build", "--release", "-p", packageName, "--target", "wasm32-wasip2"]);
    return;
  }

  if (language === "static") {
    if (target !== "universal") {
      fail(`static extensions must declare the universal target, got ${target}`);
    }
    return;
  }

  if (language === "tree-sitter-wasm") {
    if (target !== "universal") {
      fail(`tree-sitter-wasm extensions must declare the universal target, got ${target}`);
    }
    const parserPath = path.join(repoRoot, metadata.path, "parser.wasm");
    if (!fs.existsSync(parserPath)) {
      fail(`missing Tree-sitter parser wasm: ${parserPath}`);
    }
    return;
  }

  if (language === "tree-sitter-wasm-bundle") {
    if (target !== "universal") {
      fail(`tree-sitter-wasm-bundle extensions must declare the universal target, got ${target}`);
    }
    validateLanguageBundleParsers(metadata);
    return;
  }

  if (language === "go") {
    run("bash", [scriptPath("build-go-driver.sh"), metadata.id, target]);
    return;
  }

  if (language === "java") {
    run("bash", [scriptPath("build-java-driver.sh"), metadata.id, target]);
    return;
  }

  fail(`unsupported driver language for ${metadata.id}: ${language}`);
}

function rustBuildEnv(target) {
  if (target !== "aarch64-unknown-linux-gnu") {
    return process.env;
  }
  return {
    ...process.env,
    CC_aarch64_unknown_linux_gnu: process.env.CC_aarch64_unknown_linux_gnu || "aarch64-linux-gnu-gcc",
    CXX_aarch64_unknown_linux_gnu: process.env.CXX_aarch64_unknown_linux_gnu || "aarch64-linux-gnu-g++",
    AR_aarch64_unknown_linux_gnu: process.env.AR_aarch64_unknown_linux_gnu || "aarch64-linux-gnu-ar",
    CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER:
      process.env.CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER || "aarch64-linux-gnu-gcc",
  };
}

function packageDriver(metadata, target, artifactDir, version) {
  console.log(`Packaging ${metadata.id} (${target})`);
  if (metadata.kind === "database_driver") {
    run("bash", [scriptPath("package-driver.sh"), metadata.id, target, artifactDir, version]);
    return;
  }
  if (metadata.kind === "mcp_helper") {
    run("bash", [scriptPath("package-mcp-helper.sh"), metadata.id, target, artifactDir, version]);
    return;
  }
  if (metadata.kind === "acp_agent") {
    run("bash", [scriptPath("package-acp-agent.sh"), metadata.id, target, artifactDir, version]);
    return;
  }
  if (metadata.kind === "composite") {
    run("bash", [
      scriptPath("package-composite-extension.sh"),
      metadata.id,
      target,
      artifactDir,
      version,
    ]);
    return;
  }
  if (metadata.kind === "language") {
    run("bash", [
      scriptPath("package-language-extension.sh"),
      metadata.id,
      target,
      artifactDir,
      version,
    ]);
    return;
  }
  if (metadata.kind === "language_bundle") {
    run("bash", [
      scriptPath("package-language-bundle-extension.sh"),
      metadata.id,
      target,
      artifactDir,
      version,
    ]);
    return;
  }
  run("bash", [
    scriptPath("package-remote-desktop-provider.sh"),
    metadata.id,
    target,
    artifactDir,
    version,
  ]);
}

function verifyPackage(metadata, target, artifactDir) {
  console.log(`Verifying ${metadata.id} (${target})`);
  const script = verifyScriptName(metadata.kind);
  run("bash", [scriptPath(script), packagePath(artifactDir, metadata, target)]);
}

function verifyScriptName(kind) {
  switch (kind) {
    case "database_driver":
      return "verify-package.sh";
    case "remote_desktop_provider":
      return "verify-remote-desktop-provider-package.sh";
    case "mcp_helper":
      return "verify-mcp-helper-package.sh";
    case "acp_agent":
      return "verify-acp-agent-package.sh";
    case "composite":
      return "verify-composite-package.sh";
    case "language":
      return "verify-language-package.sh";
    case "language_bundle":
      return "verify-language-bundle-package.sh";
    default:
      fail(`unsupported extension kind: ${kind}`);
  }
}

function writeChecksums(metadata, targets, artifactDir) {
  const lines = targets.map((target) => {
    const filePath = packagePath(artifactDir, metadata, target);
    const sha256 = createHash("sha256").update(fs.readFileSync(filePath)).digest("hex");
    return `${sha256}  ${path.basename(filePath)}`;
  });
  fs.writeFileSync(path.join(artifactDir, "sha256sums.txt"), `${lines.join("\n")}\n`);
}

function generateExtensionManifest(metadata, targets, artifactDir, version, releaseTag) {
  run("node", [scriptPath("generate-marketplace-manifest.mjs")], {
    env: {
      ...process.env,
      ARTIFACT_DIR: path.relative(repoRoot, artifactDir) || ".",
      EXTENSION_ID: metadata.id,
      EXTENSION_VERSION: version,
      RELEASE_TAG: releaseTag,
      TARGETS: targets.join(","),
    },
  });
}

function writeReleaseMetadata(artifactDir, extensionId, version, releaseTag) {
  fs.writeFileSync(
    path.join(artifactDir, "release-metadata.json"),
    `${JSON.stringify({
      release_tag: releaseTag,
      extension_id: extensionId,
      extension_version: version,
    }, null, 2)}\n`,
  );
}

function packagePath(artifactDir, metadata, target) {
  if (metadata.kind === "database_driver") {
    return path.join(artifactDir, `${metadata.id}-driver-${target}.tar.gz`);
  }
  if (metadata.kind === "mcp_helper") {
    return path.join(artifactDir, `${metadata.id}-mcp-helper-${target}.tar.gz`);
  }
  if (metadata.kind === "acp_agent") {
    return path.join(artifactDir, `${metadata.id}-acp-agent-${target}.tar.gz`);
  }
  if (metadata.kind === "composite") {
    return path.join(artifactDir, `${metadata.id}-composite-${target}.tar.gz`);
  }
  if (metadata.kind === "language") {
    return path.join(artifactDir, `${metadata.id}-language-${target}.tar.gz`);
  }
  if (metadata.kind === "language_bundle") {
    return path.join(artifactDir, `${metadata.id}-language-bundle-${target}.tar.gz`);
  }
  return path.join(artifactDir, `${metadata.id}-remote-desktop-provider-${target}.tar.gz`);
}

function validateLanguageBundleParsers(metadata) {
  const manifestPath = path.join(repoRoot, metadata.path, "manifest.json");
  if (!fs.existsSync(manifestPath)) {
    fail(`missing Tree-sitter language bundle manifest: ${manifestPath}`);
  }
  const manifest = JSON.parse(fs.readFileSync(manifestPath, "utf8"));
  if (!Array.isArray(manifest.languages) || manifest.languages.length === 0) {
    fail(`language bundle manifest must declare non-empty languages: ${manifestPath}`);
  }
  for (const languageId of manifest.languages) {
    const parserPath = path.join(repoRoot, "extensions/language", languageId, "parser.wasm");
    if (!fs.existsSync(parserPath)) {
      fail(`missing Tree-sitter parser wasm for bundled language ${languageId}: ${parserPath}`);
    }
  }
}

function scriptPath(name) {
  return path.join(repoRoot, "scripts", name);
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: repoRoot,
    env: options.env || process.env,
    stdio: "inherit",
  });
  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    process.exit(result.status || 1);
  }
}

function fail(message, exitCode = 1) {
  console.error(`error: ${message}`);
  process.exit(exitCode);
}
