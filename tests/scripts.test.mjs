import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { execFileSync } from "node:child_process";

const repoRoot = path.resolve(import.meta.dirname, "..");

test("package-driver creates a DuckDB package with executable entry command", () => {
  const workdir = makeTempDir();
  createPackageFixture(workdir);

  const archivePath = execFileSync(
    "bash",
    [
      path.join(workdir, "scripts/package-driver.sh"),
      "duckdb",
      "x86_64-unknown-linux-gnu",
      path.join(workdir, "artifacts"),
      "1.2.3",
    ],
    { cwd: workdir, encoding: "utf8" },
  ).trim();

  assert.equal(path.basename(archivePath), "duckdb-driver-x86_64-unknown-linux-gnu.tar.gz");
  execFileSync("tar", ["xzf", archivePath, "-C", path.join(workdir, "unpacked")]);

  const driverJson = JSON.parse(
    fs.readFileSync(path.join(workdir, "unpacked/duckdb/driver.json"), "utf8"),
  );
  assert.equal(driverJson.version, "1.2.3");
  assert.equal(driverJson.entry.command, "./duckdb_driver");
  assert.equal(
    fs.readFileSync(path.join(workdir, "unpacked/duckdb/duckdb_driver"), "utf8"),
    "fake binary\n",
  );
});

test("package-driver includes downloaded DuckDB runtime library on Windows", () => {
  const workdir = makeTempDir();
  createPackageFixture(workdir);
  fs.mkdirSync(path.join(workdir, "target/x86_64-pc-windows-msvc/release/deps"), {
    recursive: true,
  });
  fs.writeFileSync(
    path.join(workdir, "target/x86_64-pc-windows-msvc/release/duckdb_driver.exe"),
    "fake windows binary\n",
  );
  fs.writeFileSync(
    path.join(workdir, "target/x86_64-pc-windows-msvc/release/deps/duckdb.dll"),
    "fake duckdb dll\n",
  );

  const archivePath = execFileSync(
    "bash",
    [
      path.join(workdir, "scripts/package-driver.sh"),
      "duckdb",
      "x86_64-pc-windows-msvc",
      path.join(workdir, "artifacts"),
      "1.2.3",
    ],
    { cwd: workdir, encoding: "utf8" },
  ).trim();

  execFileSync("tar", ["xzf", archivePath, "-C", path.join(workdir, "unpacked")]);
  assert.equal(
    fs.readFileSync(path.join(workdir, "unpacked/duckdb/duckdb.dll"), "utf8"),
    "fake duckdb dll\n",
  );
});

test("verify-package accepts a package containing driver.json, binary, and locales", () => {
  const workdir = makeTempDir();
  createPackageFixture(workdir);

  const archivePath = execFileSync(
    "bash",
    [
      path.join(workdir, "scripts/package-driver.sh"),
      "duckdb",
      "x86_64-unknown-linux-gnu",
      path.join(workdir, "artifacts"),
      "1.2.3",
    ],
    { cwd: workdir, encoding: "utf8" },
  ).trim();

  const output = execFileSync("bash", [path.join(workdir, "scripts/verify-package.sh"), archivePath], {
    cwd: workdir,
    encoding: "utf8",
  });
  assert.match(output, /Package verification ok:/);
});

test("changed-extensions emits matrix entries only for changed extension paths", () => {
  const workdir = makeTempDir();
  copyScript("changed-extensions.mjs", workdir);
  writeJson(path.join(workdir, "extensions/ipc/duckdb/extension.build.json"), {
    id: "duckdb",
    kind: "database_driver",
    package: "duckdb_driver",
    path: "extensions/ipc/duckdb",
    targets: ["x86_64-unknown-linux-gnu", "x86_64-pc-windows-msvc"],
  });
  writeJson(path.join(workdir, "extensions/ipc/postgres/extension.build.json"), {
    id: "postgres",
    kind: "database_driver",
    package: "postgres_driver",
    path: "extensions/ipc/postgres",
    targets: ["x86_64-unknown-linux-gnu"],
  });
  fs.writeFileSync(path.join(workdir, "extensions/ipc/duckdb/src.txt"), "one\n");
  fs.writeFileSync(path.join(workdir, "extensions/ipc/postgres/src.txt"), "one\n");
  git(workdir, "init");
  git(workdir, "add", ".");
  git(workdir, "commit", "-m", "initial");
  const baseSha = git(workdir, "rev-parse", "HEAD").trim();
  fs.writeFileSync(path.join(workdir, "extensions/ipc/duckdb/src.txt"), "two\n");
  git(workdir, "add", ".");
  git(workdir, "commit", "-m", "duckdb change");
  const headSha = git(workdir, "rev-parse", "HEAD").trim();

  const output = execFileSync(
    "node",
    [path.join(workdir, "scripts/changed-extensions.mjs"), baseSha, headSha],
    { cwd: workdir, encoding: "utf8" },
  );

  assert.deepEqual(JSON.parse(output), {
    include: [
      {
        extension: "duckdb",
        package: "duckdb_driver",
        kind: "database_driver",
        target: "x86_64-unknown-linux-gnu",
        os: "ubuntu-latest",
      },
      {
        extension: "duckdb",
        package: "duckdb_driver",
        kind: "database_driver",
        target: "x86_64-pc-windows-msvc",
        os: "windows-latest",
      },
    ],
  });
});

test("generate-marketplace-manifest writes merged entry with relative R2 and GitHub fallback assets", () => {
  const workdir = makeTempDir();
  copyScript("generate-marketplace-manifest.mjs", workdir);
  fs.mkdirSync(path.join(workdir, "artifacts"), { recursive: true });

  const targets = [
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "x86_64-unknown-linux-gnu",
    "x86_64-pc-windows-msvc",
  ];
  const checksums = targets.map((target) => {
    const fileName = `duckdb-driver-${target}.tar.gz`;
    const sha256 = createHash("sha256").update(fileName).digest("hex");
    return `${sha256}  ${fileName}`;
  });
  fs.writeFileSync(path.join(workdir, "artifacts/sha256sums.txt"), `${checksums.join("\n")}\n`);

  execFileSync("node", [path.join(workdir, "scripts/generate-marketplace-manifest.mjs")], {
    cwd: workdir,
    env: {
      ...process.env,
      ARTIFACT_DIR: "artifacts",
      EXTENSION_VERSION: "1.2.3",
      EXTENSION_ID: "duckdb",
      RELEASE_TAG: "duckdb-v1.2.3",
      GITHUB_REPOSITORY: "feigeCode/onetcli-extensions",
    },
  });

  const manifest = JSON.parse(
    fs.readFileSync(path.join(workdir, "artifacts/extension-manifest.json"), "utf8"),
  );
  assert.equal(manifest.schema_version, 1);
  assert.equal(manifest.release_version, "duckdb-v1.2.3");
  assert.equal(manifest.extensions.length, 1);
  assert.equal(
    manifest.extensions[0].asset_urls["x86_64-unknown-linux-gnu"],
    "duckdb/1.2.3/duckdb-driver-x86_64-unknown-linux-gnu.tar.gz",
  );
  assert.equal(
    manifest.extensions[0].fallback_asset_urls["x86_64-unknown-linux-gnu"],
    "https://github.com/feigeCode/onetcli-extensions/releases/download/duckdb-v1.2.3/duckdb-driver-x86_64-unknown-linux-gnu.tar.gz",
  );
  assert.match(manifest.extensions[0].sha256s["x86_64-unknown-linux-gnu"], /^[0-9a-f]{64}$/);
});

test("upload-r2 workflow exports R2 credentials without AWS STS configuration", () => {
  const workflow = fs.readFileSync(path.join(repoRoot, ".github/workflows/upload-r2.yml"), "utf8");

  assert.doesNotMatch(workflow, /aws-actions\/configure-aws-credentials/);
  assert.match(workflow, /AWS_ACCESS_KEY_ID:\s+\$\{\{\s*secrets\.CLOUDFLARE_R2_ACCESS_KEY_ID\s*\}\}/);
  assert.match(
    workflow,
    /AWS_SECRET_ACCESS_KEY:\s+\$\{\{\s*secrets\.CLOUDFLARE_R2_SECRET_ACCESS_KEY\s*\}\}/,
  );
  assert.match(workflow, /AWS_DEFAULT_REGION:\s+auto\b/);
});

function makeTempDir() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "onetcli-extensions-test-"));
  fs.mkdirSync(path.join(dir, "unpacked"), { recursive: true });
  return dir;
}

function createPackageFixture(workdir) {
  copyScript("package-driver.sh", workdir);
  copyScript("verify-package.sh", workdir);
  writeJson(path.join(workdir, "extensions/ipc/duckdb/extension.build.json"), {
    id: "duckdb",
    binary: "duckdb_driver",
  });
  writeJson(path.join(workdir, "extensions/ipc/duckdb/driver.json"), {
    id: "duckdb",
    version: "0.0.0",
    entry: {},
  });
  fs.mkdirSync(path.join(workdir, "extensions/ipc/duckdb/locales"), { recursive: true });
  fs.writeFileSync(path.join(workdir, "extensions/ipc/duckdb/locales/en.yml"), "name: DuckDB\n");
  fs.mkdirSync(path.join(workdir, "target/x86_64-unknown-linux-gnu/release"), { recursive: true });
  fs.writeFileSync(
    path.join(workdir, "target/x86_64-unknown-linux-gnu/release/duckdb_driver"),
    "fake binary\n",
  );
}

function copyScript(name, workdir) {
  fs.mkdirSync(path.join(workdir, "scripts"), { recursive: true });
  fs.copyFileSync(path.join(repoRoot, "scripts", name), path.join(workdir, "scripts", name));
}

function writeJson(filePath, value) {
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
  fs.writeFileSync(filePath, `${JSON.stringify(value, null, 2)}\n`);
}

function git(workdir, ...args) {
  return execFileSync(
    "git",
    ["-c", "user.name=Test User", "-c", "user.email=test@example.com", ...args],
    { cwd: workdir, encoding: "utf8" },
  );
}
