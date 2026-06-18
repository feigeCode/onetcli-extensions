import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { execFileSync } from "node:child_process";

const repoRoot = path.resolve(import.meta.dirname, "..");

test("go ipc driver metadata excludes GBase8s", () => {
  const ids = fs
    .readdirSync(path.join(repoRoot, "extensions/ipc"))
    .filter((id) => {
      const metadataPath = path.join(repoRoot, "extensions/ipc", id, "extension.build.json");
      if (!fs.existsSync(metadataPath)) return false;
      const metadata = JSON.parse(fs.readFileSync(metadataPath, "utf8"));
      return metadata.language === "go";
    })
    .sort();

  assert.deepEqual(ids, ["dm", "kingbase"]);
});

test("Go IPC driver manifests expose the full shared method surface", () => {
  const expectedMethods = [
    "$/ping",
    "shutdown",
    "conn/test",
    "conn/open",
    "conn/close",
    "conn/ping",
    "conn/use",
    "query/start",
    "cursor/fetch",
    "cursor/close",
    "cursor/cancel",
    "exec/run",
    "exec/batch",
    "tx/begin",
    "tx/commit",
    "tx/rollback",
    "tx/savepoint",
    "tx/release",
    "ddl/build",
    "ddl/build_create_table",
    "ddl/build_alter_table",
    "ddl/build_drop",
    "data/export",
    "data/import_begin",
    "data/import_chunk",
    "data/import_commit",
    "data/import_abort",
    "stream/read",
    "stream/close",
    "schema/databases",
    "schema/schemas",
    "schema/objects",
    "schema/columns",
    "schema/indexes",
    "schema/foreign_keys",
    "schema/checks",
    "schema/views",
    "schema/functions",
    "schema/procedures",
    "schema/triggers",
    "schema/sequences",
    "schema/types",
    "schema/view_definition",
    "schema/dump_ddl",
  ];

  for (const id of ["dm", "kingbase"]) {
    const driverJson = JSON.parse(
      fs.readFileSync(path.join(repoRoot, "extensions/ipc", id, "driver.json"), "utf8"),
    );
    assert.deepEqual(driverJson.methods, expectedMethods, `${id} methods drifted`);
  }
});

test("GBase8s Java IPC driver manifest exposes the full method surface", () => {
  const metadata = JSON.parse(
    fs.readFileSync(path.join(repoRoot, "extensions/ipc/gbase8s/extension.build.json"), "utf8"),
  );
  assert.equal(metadata.language, "java");
  assert.equal(metadata.package, "java/gbase8s-ipc-driver");
  assert.equal(metadata.binary, "gbase8s-ipc-driver");
  assert.equal(metadata.jar, "gbase8s-ipc-driver.jar");

  const driverJson = JSON.parse(
    fs.readFileSync(path.join(repoRoot, "extensions/ipc/gbase8s/driver.json"), "utf8"),
  );
  for (const method of [
    "tx/begin",
    "tx/commit",
    "tx/rollback",
    "tx/savepoint",
    "tx/release",
    "ddl/build",
    "ddl/build_create_table",
    "ddl/build_alter_table",
    "ddl/build_drop",
    "data/export",
    "data/import_begin",
    "data/import_chunk",
    "data/import_commit",
    "data/import_abort",
    "stream/read",
    "stream/close",
    "schema/dump_ddl",
  ]) {
    assert.ok(driverJson.methods.includes(method), `gbase8s methods missing ${method}`);
  }
});

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

test("verify-package accepts non-DuckDB driver packages", () => {
  const workdir = makeTempDir();
  createPackageFixture(workdir, {
    id: "iotdb",
    binary: "iotdb_driver",
    binaryContents: "fake iotdb binary\n",
  });

  const archivePath = execFileSync(
    "bash",
    [
      path.join(workdir, "scripts/package-driver.sh"),
      "iotdb",
      "x86_64-unknown-linux-gnu",
      path.join(workdir, "artifacts"),
      "0.1.0",
    ],
    { cwd: workdir, encoding: "utf8" },
  ).trim();

  const output = execFileSync("bash", [path.join(workdir, "scripts/verify-package.sh"), archivePath], {
    cwd: workdir,
    encoding: "utf8",
  });
  assert.match(output, /Package verification ok:/);
});

test("package-driver creates a Go IPC driver package", () => {
  const workdir = makeTempDir();
  createPackageFixture(workdir, {
    id: "dm",
    binary: "dm-ipc-driver",
    binaryContents: "fake dm go binary\n",
    language: "go",
    package: "./cmd/dm-ipc-driver",
  });

  const archivePath = execFileSync(
    "bash",
    [
      path.join(workdir, "scripts/package-driver.sh"),
      "dm",
      "x86_64-unknown-linux-gnu",
      path.join(workdir, "artifacts"),
      "0.1.0",
    ],
    { cwd: workdir, encoding: "utf8" },
  ).trim();

  assert.equal(path.basename(archivePath), "dm-driver-x86_64-unknown-linux-gnu.tar.gz");
  execFileSync("tar", ["xzf", archivePath, "-C", path.join(workdir, "unpacked")]);

  const driverJson = JSON.parse(
    fs.readFileSync(path.join(workdir, "unpacked/dm/driver.json"), "utf8"),
  );
  assert.equal(driverJson.entry.command, "./dm-ipc-driver");
  assert.equal(
    fs.readFileSync(path.join(workdir, "unpacked/dm/dm-ipc-driver"), "utf8"),
    "fake dm go binary\n",
  );
});

test("package-driver includes Java IPC driver launcher and jar library", () => {
  const workdir = makeTempDir();
  createPackageFixture(workdir, {
    id: "gbase8s",
    binary: "gbase8s-ipc-driver",
    binaryContents: "#!/usr/bin/env sh\nexec java -jar \"$DIR/lib/gbase8s-ipc-driver.jar\" \"$@\"\n",
    language: "java",
    package: "java/gbase8s-ipc-driver",
  });
  fs.mkdirSync(path.join(workdir, "target/x86_64-unknown-linux-gnu/release/lib"), {
    recursive: true,
  });
  fs.writeFileSync(
    path.join(workdir, "target/x86_64-unknown-linux-gnu/release/lib/gbase8s-ipc-driver.jar"),
    "fake jar\n",
  );

  const archivePath = execFileSync(
    "bash",
    [
      path.join(workdir, "scripts/package-driver.sh"),
      "gbase8s",
      "x86_64-unknown-linux-gnu",
      path.join(workdir, "artifacts"),
      "0.1.0",
    ],
    { cwd: workdir, encoding: "utf8" },
  ).trim();

  execFileSync("tar", ["xzf", archivePath, "-C", path.join(workdir, "unpacked")]);
  assert.equal(
    fs.readFileSync(
      path.join(workdir, "unpacked/gbase8s/lib/gbase8s-ipc-driver.jar"),
      "utf8",
    ),
    "fake jar\n",
  );
  const driverJson = JSON.parse(
    fs.readFileSync(path.join(workdir, "unpacked/gbase8s/driver.json"), "utf8"),
  );
  assert.equal(driverJson.entry.command, "./gbase8s-ipc-driver");
});

test("build-java-driver stages launcher and shaded jar into target release directory", () => {
  const workdir = makeTempDir();
  copyScript("build-java-driver.sh", workdir);
  writeJson(path.join(workdir, "extensions/ipc/gbase8s/extension.build.json"), {
    id: "gbase8s",
    kind: "database_driver",
    language: "java",
    package: "java/gbase8s-ipc-driver",
    binary: "gbase8s-ipc-driver",
    jar: "gbase8s-ipc-driver.jar",
    path: "extensions/ipc/gbase8s",
    targets: ["x86_64-unknown-linux-gnu"],
  });
  fs.mkdirSync(path.join(workdir, "java/gbase8s-ipc-driver/target"), { recursive: true });
  fs.mkdirSync(path.join(workdir, "java/gbase8s-ipc-driver/bin"), { recursive: true });
  fs.writeFileSync(
    path.join(workdir, "java/gbase8s-ipc-driver/target/gbase8s-ipc-driver-0.1.0-all.jar"),
    "fake shaded jar\n",
  );
  fs.writeFileSync(
    path.join(workdir, "java/gbase8s-ipc-driver/bin/gbase8s-ipc-driver"),
    "#!/usr/bin/env sh\n",
  );

  execFileSync(
    "bash",
    [
      path.join(workdir, "scripts/build-java-driver.sh"),
      "gbase8s",
      "x86_64-unknown-linux-gnu",
    ],
    { cwd: workdir },
  );

  assert.equal(
    fs.readFileSync(
      path.join(workdir, "target/x86_64-unknown-linux-gnu/release/lib/gbase8s-ipc-driver.jar"),
      "utf8",
    ),
    "fake shaded jar\n",
  );
  assert.ok(
    fs.existsSync(path.join(workdir, "target/x86_64-unknown-linux-gnu/release/gbase8s-ipc-driver")),
  );
});

test("build-go-driver builds a Go command into the target release directory", () => {
  const workdir = makeTempDir();
  copyScript("build-go-driver.sh", workdir);
  fs.writeFileSync(path.join(workdir, "go.mod"), "module example.com/go-driver-fixture\n\ngo 1.23\n");
  fs.mkdirSync(path.join(workdir, "cmd/test-ipc-driver"), { recursive: true });
  fs.writeFileSync(
    path.join(workdir, "cmd/test-ipc-driver/main.go"),
    "package main\n\nfunc main() {}\n",
  );
  writeJson(path.join(workdir, "extensions/ipc/testdb/extension.build.json"), {
    id: "testdb",
    kind: "database_driver",
    language: "go",
    package: "./cmd/test-ipc-driver",
    binary: "test-ipc-driver",
    path: "extensions/ipc/testdb",
    targets: ["x86_64-unknown-linux-gnu"],
  });

  execFileSync(
    "bash",
    [
      path.join(workdir, "scripts/build-go-driver.sh"),
      "testdb",
      "x86_64-unknown-linux-gnu",
    ],
    {
      cwd: workdir,
      env: {
        ...process.env,
        GOCACHE: path.join(workdir, "go-cache"),
        CGO_ENABLED: "0",
      },
    },
  );

  assert.ok(
    fs.existsSync(path.join(workdir, "target/x86_64-unknown-linux-gnu/release/test-ipc-driver")),
  );
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
    language: "go",
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
        language: "rust",
        target: "x86_64-unknown-linux-gnu",
        os: "ubuntu-latest",
      },
      {
        extension: "duckdb",
        package: "duckdb_driver",
        kind: "database_driver",
        language: "rust",
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
  writeJson(path.join(workdir, "extensions/ipc/duckdb/extension.build.json"), {
    id: "duckdb",
    kind: "database_driver",
    path: "extensions/ipc/duckdb",
    targets: [
      "aarch64-apple-darwin",
      "x86_64-apple-darwin",
      "x86_64-unknown-linux-gnu",
      "x86_64-pc-windows-msvc",
    ],
  });
  writeJson(path.join(workdir, "extensions/ipc/duckdb/driver.json"), {
    id: "duckdb",
    name: "DuckDB",
    description: "DuckDB embedded analytical database IPC driver",
  });

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

test("generate-marketplace-manifest uses selected extension metadata", () => {
  const workdir = makeTempDir();
  copyScript("generate-marketplace-manifest.mjs", workdir);
  fs.mkdirSync(path.join(workdir, "artifacts"), { recursive: true });
  writeJson(path.join(workdir, "extensions/ipc/iotdb/extension.build.json"), {
    id: "iotdb",
    kind: "database_driver",
    path: "extensions/ipc/iotdb",
    targets: ["x86_64-unknown-linux-gnu"],
  });
  writeJson(path.join(workdir, "extensions/ipc/iotdb/driver.json"), {
    id: "iotdb",
    name: "Apache IoTDB",
    description: "Apache IoTDB time-series database IPC driver",
  });
  const fileName = "iotdb-driver-x86_64-unknown-linux-gnu.tar.gz";
  fs.writeFileSync(
    path.join(workdir, "artifacts/sha256sums.txt"),
    `${createHash("sha256").update(fileName).digest("hex")}  ${fileName}\n`,
  );

  execFileSync("node", [path.join(workdir, "scripts/generate-marketplace-manifest.mjs")], {
    cwd: workdir,
    env: {
      ...process.env,
      ARTIFACT_DIR: "artifacts",
      EXTENSION_VERSION: "0.1.0",
      EXTENSION_ID: "iotdb",
      RELEASE_TAG: "iotdb-v0.1.0",
      GITHUB_REPOSITORY: "feigeCode/onetcli-extensions",
    },
  });

  const manifest = JSON.parse(
    fs.readFileSync(path.join(workdir, "artifacts/extension-manifest.json"), "utf8"),
  );
  assert.equal(manifest.extensions[0].id, "iotdb");
  assert.equal(manifest.extensions[0].name, "Apache IoTDB");
  assert.equal(
    manifest.extensions[0].asset_urls["x86_64-unknown-linux-gnu"],
    "iotdb/0.1.0/iotdb-driver-x86_64-unknown-linux-gnu.tar.gz",
  );
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

function createPackageFixture(workdir, options = {}) {
  const id = options.id || "duckdb";
  const binary = options.binary || "duckdb_driver";
  const binaryContents = options.binaryContents || "fake binary\n";
  const language = options.language || "rust";
  const packageName = options.package || `${id}_driver`;
  copyScript("package-driver.sh", workdir);
  copyScript("verify-package.sh", workdir);
  writeJson(path.join(workdir, `extensions/ipc/${id}/extension.build.json`), {
    id,
    kind: "database_driver",
    language,
    package: packageName,
    binary,
  });
  writeJson(path.join(workdir, `extensions/ipc/${id}/driver.json`), {
    id,
    version: "0.0.0",
    entry: {},
  });
  fs.mkdirSync(path.join(workdir, `extensions/ipc/${id}/locales`), { recursive: true });
  fs.writeFileSync(path.join(workdir, `extensions/ipc/${id}/locales/en.yml`), `name: ${id}\n`);
  fs.mkdirSync(path.join(workdir, "target/x86_64-unknown-linux-gnu/release"), { recursive: true });
  fs.writeFileSync(
    path.join(workdir, `target/x86_64-unknown-linux-gnu/release/${binary}`),
    binaryContents,
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
