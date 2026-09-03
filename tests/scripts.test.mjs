import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { execFileSync } from "node:child_process";

const repoRoot = path.resolve(import.meta.dirname, "..");

test("CNB release synchronization pushes tags before syncing release assets", () => {
  const workflow = fs.readFileSync(
    path.join(repoRoot, ".github/workflows/sync-cnb-release-assets.yml"),
    "utf8",
  );

  assert.match(workflow, /uses: actions\/checkout@v4/);
  assert.match(workflow, /fetch-depth: 0/);
  assert.match(workflow, /ref: \$\{\{ inputs\.tag \}\}/);
  assert.match(
    workflow,
    /git remote add cnb "https:\/\/cnb\.cool\/\$\{CNB_REPOSITORY\}\.git"/,
  );
  assert.match(workflow, /git ls-remote --tags cnb/);
  assert.match(workflow, /git for-each-ref --format='%\(refname\)' refs\/tags/);
  assert.match(workflow, /git push cnb ":refs\/tags\/\$\{tag_name\}"/);
  assert.match(workflow, /git push cnb --tags/);
  assert.match(workflow, /\.\/mpgrm releases sync/);

  const deleteMovedTag = workflow.indexOf('git push cnb ":refs/tags/${tag_name}"');
  const pushTags = workflow.indexOf("git push cnb --tags");
  const syncAssets = workflow.indexOf("./mpgrm releases sync");
  assert.ok(deleteMovedTag >= 0 && deleteMovedTag < pushTags);
  assert.ok(pushTags >= 0 && pushTags < syncAssets);
});

test("latest CNB sync workflow reuses the per-tag sync workflow per matrix entry", () => {
  const workflow = fs.readFileSync(
    path.join(repoRoot, ".github/workflows/sync-cnb-latest.yml"),
    "utf8",
  );
  const syncWorkflow = fs.readFileSync(
    path.join(repoRoot, ".github/workflows/sync-cnb-release-assets.yml"),
    "utf8",
  );

  assert.match(workflow, /^name: Sync Latest Extension Releases to CNB/m);
  assert.match(workflow, /\n  workflow_dispatch:\n/);
  assert.match(workflow, /group: extension-cnb-sync-\$\{\{ matrix\.tag \}\}/);
  assert.match(workflow, /cancel-in-progress: false/);
  assert.match(workflow, /permissions:\n  actions: read\n  contents: read/);
  assert.match(workflow, /scripts\/discover-latest-extension-releases\.mjs/);
  assert.match(workflow, /has_releases="\$\(node -e/);
  assert.match(workflow, /fail-fast: false/);
  assert.match(workflow, /uses: \.\/\.github\/workflows\/sync-cnb-release-assets\.yml/);
  assert.match(workflow, /with:\n      tag: \$\{\{ matrix\.tag \}\}/);
  assert.match(workflow, /secrets: inherit/);

  assert.match(syncWorkflow, /\n  workflow_call:\n/);
  assert.match(syncWorkflow, /workflow_call:\n    inputs:\n      tag:/);
});

test("discover-latest-extension-releases picks the latest stable release per extension", () => {
  const script = path.join(repoRoot, "scripts/discover-latest-extension-releases.mjs");
  const fixture = [
    { tagName: "dm-v0.1.4", publishedAt: "2026-07-01T00:00:00Z", isDraft: false, isPrerelease: false },
    { tagName: "dm-v0.1.5", publishedAt: "2026-08-02T11:51:47Z", isDraft: false, isPrerelease: false },
    { tagName: "dm-v9.9.9", publishedAt: "2026-09-01T00:00:00Z", isDraft: true, isPrerelease: false },
    { tagName: "rdp-v0.3.6", publishedAt: "2026-08-11T09:39:46Z", isDraft: false, isPrerelease: false },
    { tagName: "rdp-v0.4.0-rc.1", publishedAt: "2026-08-30T00:00:00Z", isDraft: false, isPrerelease: true },
    { tagName: "claude-acp-v0.1.0", publishedAt: "2026-07-05T09:02:17Z", isDraft: false, isPrerelease: false },
    { tagName: "opencode-acp-v1.0.0-rc.1", publishedAt: "2026-07-09T05:27:10Z", isDraft: false, isPrerelease: true },
  ];
  const workdir = fs.mkdtempSync(path.join(os.tmpdir(), "navop-discover-"));
  try {
    const fixturePath = path.join(workdir, "releases.json");
    fs.writeFileSync(fixturePath, JSON.stringify(fixture));
    const output = execFileSync("node", [script], {
      encoding: "utf8",
      env: { ...process.env, GH_RELEASES_JSON: fixturePath },
    });
    const matrix = JSON.parse(output);
    const byId = new Map(matrix.include.map((item) => [item.id, item]));

    assert.strictEqual(byId.get("dm").tag, "dm-v0.1.5");
    assert.strictEqual(byId.get("dm").version, "0.1.5");
    assert.strictEqual(byId.get("claude-acp").tag, "claude-acp-v0.1.0");
    assert.strictEqual(byId.get("rdp").tag, "rdp-v0.3.6");
    assert.strictEqual(byId.get("opencode-acp").tag, "opencode-acp-v1.0.0-rc.1");
    assert.strictEqual(byId.get("opencode-acp").version, "1.0.0-rc.1");
    assert.equal(byId.has("astro"), false);
  } finally {
    fs.rmSync(workdir, { recursive: true, force: true });
  }
});

test("go ipc driver metadata excludes Java drivers", () => {
  const ids = fs
    .readdirSync(path.join(repoRoot, "extensions/ipc"))
    .filter((id) => {
      const metadataPath = path.join(repoRoot, "extensions/ipc", id, "extension.build.json");
      if (!fs.existsSync(metadataPath)) return false;
      const metadata = JSON.parse(fs.readFileSync(metadataPath, "utf8"));
      return metadata.language === "go";
    })
    .sort();

  assert.deepEqual(ids, ["dm", "iotdb", "kingbase", "oceanbase", "oracle-go"]);
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
    "schema/object_view",
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

  for (const id of ["dm", "kingbase", "oceanbase", "oracle-go"]) {
    const driverJson = JSON.parse(
      fs.readFileSync(path.join(repoRoot, "extensions/ipc", id, "driver.json"), "utf8"),
    );
    assert.deepEqual(driverJson.methods, expectedMethods, `${id} methods drifted`);
  }
});

test("Go IPC driver metadata declares all cross-compiled release targets", () => {
  const expectedTargets = [
    "x86_64-apple-darwin",
    "aarch64-apple-darwin",
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
    "x86_64-pc-windows-msvc",
    "i686-pc-windows-msvc",
  ];

  for (const id of ["dm", "iotdb", "kingbase", "oceanbase", "oracle-go"]) {
    const metadata = JSON.parse(
      fs.readFileSync(path.join(repoRoot, "extensions/ipc", id, "extension.build.json"), "utf8"),
    );
    assert.equal(metadata.language, "go");
    assert.deepEqual(metadata.targets, expectedTargets, `${id} target list drifted`);
  }
});

test("native Windows extensions declare the Windows x86 release target", () => {
  const extensionPaths = [
    "extensions/ipc/dm",
    "extensions/ipc/iotdb",
    "extensions/ipc/kingbase",
    "extensions/ipc/oceanbase",
    "extensions/ipc/oracle-go",
    "extensions/ipc/redis",
    "extensions/ipc/mongodb-legacy",
    "extensions/ipc/mongodb-modern",
    "extensions/ipc/mongodb-legacy-3-2",
    "extensions/ipc/opengauss",
    "extensions/ipc/duckdb",
    "extensions/remote-desktop/vnc",
    "extensions/remote-desktop/rdp",
  ];

  for (const extensionPath of extensionPaths) {
    const metadata = JSON.parse(
      fs.readFileSync(path.join(repoRoot, extensionPath, "extension.build.json"), "utf8"),
    );
    assert.ok(
      metadata.targets.includes("x86_64-pc-windows-msvc"),
      `${metadata.id} is missing x86_64-pc-windows-msvc`,
    );
    assert.ok(
      metadata.targets.includes("i686-pc-windows-msvc"),
      `${metadata.id} is missing i686-pc-windows-msvc`,
    );
  }
});

test("ACP agents declare both Windows x64 and Windows x86 release targets", () => {
  for (const id of ["claude-acp", "codex-acp", "opencode-acp"]) {
    const metadata = JSON.parse(
      fs.readFileSync(
        path.join(repoRoot, "extensions/acp-agent", id, "extension.build.json"),
        "utf8",
      ),
    );
    assert.ok(
      metadata.targets.includes("x86_64-pc-windows-msvc"),
      `${id} is missing x86_64-pc-windows-msvc`,
    );
    assert.ok(
      metadata.targets.includes("i686-pc-windows-msvc"),
      `${id} is missing i686-pc-windows-msvc`,
    );
    assert.equal(
      new Set(metadata.targets).size,
      metadata.targets.length,
      `${id} declares duplicate release targets`,
    );
  }
});

test("architecture-independent extensions remain universal for Windows x86 fallback", () => {
  const universalLanguages = new Set([
    "java",
    "rust-wasm",
    "static",
    "tree-sitter-wasm",
    "tree-sitter-wasm-bundle",
  ]);
  const roots = [
    "extensions/ipc",
    "extensions/wasm",
    "extensions/language",
    "extensions/language-bundle",
  ];
  const checked = [];

  for (const root of roots) {
    for (const id of fs.readdirSync(path.join(repoRoot, root))) {
      const metadataPath = path.join(repoRoot, root, id, "extension.build.json");
      if (!fs.existsSync(metadataPath)) continue;
      const metadata = JSON.parse(fs.readFileSync(metadataPath, "utf8"));
      if (!universalLanguages.has(metadata.language)) continue;
      assert.deepEqual(
        metadata.targets,
        ["universal"],
        `${metadata.id} must use the architecture-independent universal artifact`,
      );
      checked.push(metadata.id);
    }
  }

  assert.ok(checked.includes("gbase8s"));
  assert.ok(checked.includes("oscar"));
  assert.ok(checked.includes("dbeaver-importer"));
  assert.ok(checked.includes("notepad-plus-plus-editor"));
  assert.ok(checked.includes("java"));
  assert.ok(checked.includes("tree-sitter-languages"));
});

test("IPC driver metadata declares Linux ARM64 release target", () => {
  const ids = fs
    .readdirSync(path.join(repoRoot, "extensions/ipc"))
    .filter((id) =>
      fs.existsSync(path.join(repoRoot, "extensions/ipc", id, "extension.build.json")),
    )
    .sort();

  for (const id of ids) {
    const metadata = JSON.parse(
      fs.readFileSync(path.join(repoRoot, "extensions/ipc", id, "extension.build.json"), "utf8"),
    );
    assert.ok(
      metadata.targets.includes("universal") || metadata.targets.includes("aarch64-unknown-linux-gnu"),
      `${id} is missing aarch64-unknown-linux-gnu or universal target`,
    );
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
  assert.deepEqual(metadata.targets, ["universal"]);

  const driverJson = JSON.parse(
    fs.readFileSync(path.join(repoRoot, "extensions/ipc/gbase8s/driver.json"), "utf8"),
  );
  assert.equal(driverJson.entry.command, "./gbase8s-ipc-driver");
  assert.equal(driverJson.entry.commands.windows, "./gbase8s-ipc-driver.cmd");
  assert.equal(driverJson.entry.env_from_config.GBASE8S_JDK_HOME, "extra_params.jdk_home");
  assert.equal(driverJson.dialect.identifier_quote_left, "");
  assert.equal(driverJson.dialect.identifier_quote_right, "");
  assert.ok(
    fs.existsSync(
      path.join(
        repoRoot,
        "java/gbase8s-ipc-driver/bin/lib/gbasedbtjdbc_3.5.0_2ZY3_1_89a58a.jar",
      ),
    ),
    "gbase8s should include the official JDBC jar by default",
  );

  const connectionForm = driverJson.ui.form.forms.find((form) => form.kind === "Connection");
  const advancedTab = connectionForm.tabs.find((tab) => tab.id === "advanced");
  assert.ok(advancedTab, "gbase8s connection form should expose an advanced tab");
  assert.deepEqual(
    advancedTab.fields.map((field) => field.id),
    ["GBASEDBTSERVER", "PROTOCOL", "jdk_home", "jdbc_jar", "driver_class"],
  );
  assert.equal(
    advancedTab.fields.find((field) => field.id === "jdbc_jar").default_value,
    "lib/gbasedbtjdbc_3.5.0_2ZY3_1_89a58a.jar",
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
    "schema/object_view",
    "schema/dump_ddl",
  ]) {
    assert.ok(driverJson.methods.includes(method), `gbase8s methods missing ${method}`);
  }
});

test("Oscar Java IPC driver manifest exposes configurable JDBC settings", () => {
  const metadata = JSON.parse(
    fs.readFileSync(path.join(repoRoot, "extensions/ipc/oscar/extension.build.json"), "utf8"),
  );
  assert.equal(metadata.language, "java");
  assert.equal(metadata.package, "java/oscar-ipc-driver");
  assert.equal(metadata.binary, "oscar-ipc-driver");
  assert.equal(metadata.jar, "oscar-ipc-driver.jar");
  assert.deepEqual(metadata.targets, ["universal"]);

  const driverJson = JSON.parse(
    fs.readFileSync(path.join(repoRoot, "extensions/ipc/oscar/driver.json"), "utf8"),
  );
  assert.equal(driverJson.id, "oscar");
  assert.equal(driverJson.category, "domestic_database");
  assert.equal(driverJson.entry.command, "./oscar-ipc-driver");
  assert.equal(driverJson.entry.commands.windows, "./oscar-ipc-driver.cmd");
  assert.equal(driverJson.entry.env_from_config.OSCAR_JDK_HOME, "extra_params.jdk_home");
  assert.equal(driverJson.transport.name, "oscar-driver.sock");
  assert.equal(driverJson.ui.default_port, 2003);
  assert.equal(driverJson.dialect.identifier_quote_left, "\"");
  assert.equal(driverJson.dialect.identifier_quote_right, "\"");

  const connectionForm = driverJson.ui.form.forms.find((form) => form.kind === "Connection");
  const advancedTab = connectionForm.tabs.find((tab) => tab.id === "advanced");
  assert.ok(advancedTab, "oscar connection form should expose an advanced tab");
  assert.deepEqual(
    advancedTab.fields.map((field) => field.id),
    ["jdk_home", "jdbc_url", "jdbc_jar", "driver_class"],
  );
  assert.equal(
    advancedTab.fields.find((field) => field.id === "driver_class").default_value,
    "com.oscar.Driver",
  );
});

test("GBase8s Java IPC driver does not declare driver-owned table data", () => {
  const driverJson = JSON.parse(
    fs.readFileSync(path.join(repoRoot, "extensions/ipc/gbase8s/driver.json"), "utf8"),
  );

  assert.equal(driverJson.query?.table_data_method, undefined);
  assert.ok(!driverJson.methods.includes("gbase8s/table_data"));
  assert.ok(!driverJson.methods.includes("x/gbase8s/table_data"));
});

test("IPC driver build metadata declares release and R2 manifest routing", () => {
  const ids = fs
    .readdirSync(path.join(repoRoot, "extensions/ipc"))
    .filter((id) =>
      fs.existsSync(path.join(repoRoot, "extensions/ipc", id, "extension.build.json")),
    )
    .sort();

  for (const id of ids) {
    const metadata = JSON.parse(
      fs.readFileSync(path.join(repoRoot, "extensions/ipc", id, "extension.build.json"), "utf8"),
    );
    assert.equal(metadata.releaseTagPrefix, `${id}-v`, `${id} releaseTagPrefix drifted`);
    assert.equal(metadata.r2Prefix, `extensions/${id}`, `${id} r2Prefix drifted`);
  }
});

test("native Redis and MongoDB drivers keep standalone manifests and release metadata", () => {
  const expected = [
    ["redis", "redis-driver", "extensions/ipc/redis"],
    ["mongodb-modern", "mongodb-modern-driver", "extensions/ipc/mongodb-modern"],
    ["mongodb-legacy", "mongodb-legacy-driver", "extensions/ipc/mongodb-legacy"],
    [
      "mongodb-legacy-3-2",
      "mongodb-legacy-3-2-driver",
      "extensions/ipc/mongodb-legacy-3-2",
    ],
  ];

  for (const [id, binary, extensionPath] of expected) {
    const metadata = JSON.parse(
      fs.readFileSync(path.join(repoRoot, extensionPath, "extension.build.json"), "utf8"),
    );
    const manifest = JSON.parse(
      fs.readFileSync(path.join(repoRoot, extensionPath, "driver.json"), "utf8"),
    );
    assert.equal(metadata.kind, "database_driver");
    assert.equal(metadata.language, "rust");
    assert.equal(metadata.binary, binary);
    assert.equal(metadata.releaseTagPrefix, `${id}-v`);
    assert.equal(metadata.r2Prefix, `extensions/${id}`);
    assert.equal(manifest.id, id);
    assert.equal(manifest.api === "redis" || manifest.api === "mongodb", true);
    assert.equal(manifest.entry.command, `./${binary}`);
    assert.ok(manifest.methods.length > 0, `${id} should declare wire methods`);
  }

  const redis = JSON.parse(
    fs.readFileSync(path.join(repoRoot, "extensions/ipc/redis/driver.json"), "utf8"),
  );
  assert.deepEqual(redis.methods, [
    "conn/open",
    "conn/close",
    "redis/command",
    "redis/pipeline",
    "event/open",
    "redis/pubsub_control",
    "event/read",
    "event/close",
  ]);

  for (const id of ["mongodb-modern", "mongodb-legacy", "mongodb-legacy-3-2"]) {
    const mongo = JSON.parse(
      fs.readFileSync(path.join(repoRoot, `extensions/ipc/${id}/driver.json`), "utf8"),
    );
    assert.deepEqual(mongo.methods, [
      "conn/open",
      "conn/close",
      "mongodb/command",
      "mongodb/find",
      "blob/read",
      "blob/close",
    ]);
  }
});

test("MongoDB variants use separate SDK generations and honest compatibility ranges", () => {
  const cargo = fs.readFileSync(path.join(repoRoot, "drivers/mongodb-driver/Cargo.toml"), "utf8");
  const legacy32Cargo = fs.readFileSync(
    path.join(repoRoot, "drivers/mongodb-legacy-3-2-driver/Cargo.toml"),
    "utf8",
  );
  assert.match(cargo, /mongodb-modern\s*=\s*\{[^\n]*package\s*=\s*"mongodb"[^\n]*version\s*=\s*"=3\.8\.0"/);
  assert.match(cargo, /mongodb-legacy\s*=\s*\{[^\n]*package\s*=\s*"mongodb"[^\n]*version\s*=\s*"=2\.8\.2"/);
  assert.doesNotMatch(cargo, /mongodb-legacy32/);
  assert.match(legacy32Cargo, /mongodb-legacy32\s*=\s*\{[^\n]*package\s*=\s*"mongodb"[^\n]*version\s*=\s*"=0\.3\.10"/);
  assert.ok(fs.existsSync(path.join(repoRoot, "drivers/mongodb-driver/src/modern.rs")));
  assert.ok(fs.existsSync(path.join(repoRoot, "drivers/mongodb-driver/src/legacy.rs")));
  assert.ok(
    fs.existsSync(path.join(repoRoot, "drivers/mongodb-legacy-3-2-driver/src/main.rs")),
  );
  assert.ok(
    fs.existsSync(path.join(repoRoot, "drivers/mongodb-legacy-3-2-driver/Cargo.lock")),
  );
  const workspaceCargo = fs.readFileSync(path.join(repoRoot, "Cargo.toml"), "utf8");
  assert.doesNotMatch(workspaceCargo, /drivers\/mongodb-legacy-3-2-driver/);
  const releaseScript = fs.readFileSync(
    path.join(repoRoot, "scripts/release-driver.mjs"),
    "utf8",
  );
  assert.match(
    releaseScript,
    /metadata\.manifest_path \? \{ CARGO_TARGET_DIR: path\.join\(repoRoot, "target"\) \} : \{\}/,
  );
  const releaseWorkflow = fs.readFileSync(
    path.join(repoRoot, ".github/workflows/release.yml"),
    "utf8",
  );
  assert.match(
    releaseWorkflow,
    /export CARGO_TARGET_DIR="\$\{GITHUB_WORKSPACE\}\/target"/,
  );

  const modern = JSON.parse(
    fs.readFileSync(path.join(repoRoot, "extensions/ipc/mongodb-modern/driver.json"), "utf8"),
  );
  const legacy = JSON.parse(
    fs.readFileSync(path.join(repoRoot, "extensions/ipc/mongodb-legacy/driver.json"), "utf8"),
  );
  const legacy32 = JSON.parse(
    fs.readFileSync(
      path.join(repoRoot, "extensions/ipc/mongodb-legacy-3-2/driver.json"),
      "utf8",
    ),
  );
  assert.deepEqual(modern.compatibility.server, { min: "4.2" });
  assert.deepEqual(legacy.compatibility.server, { min: "3.6", max: "4.0" });
  assert.deepEqual(legacy32.compatibility.server, { min: "3.2", max: "3.4" });
});

test("extension descriptions are bilingual, detailed, and synchronized", () => {
  const globalManifest = JSON.parse(fs.readFileSync(path.join(repoRoot, "manifest.json"), "utf8"));
  const globalEntries = new Map(globalManifest.extensions.map((extension) => [extension.id, extension]));

  for (const metadata of collectExtensionMetadata()) {
    const language = languageName(metadata);
    const sourceManifest = JSON.parse(
      fs.readFileSync(path.join(repoRoot, metadata.path, sourceManifestFileName(metadata.kind)), "utf8"),
    );

    const description = sourceManifest.description;
    assert.equal(typeof description, "string", `${metadata.id} should define a description`);
    assert.ok(
      description.includes(language),
      `${metadata.id} source manifest description should mention ${language}`,
    );
    assert.match(description, /[\u3400-\u9fff]/, `${metadata.id} description should include Chinese`);
    assert.match(description, /[A-Za-z]{4}/, `${metadata.id} description should include English`);
    assert.ok(description.length >= 100, `${metadata.id} description should be sufficiently detailed`);
    assert.ok(
      globalEntries.get(sourceManifest.id)?.description?.includes(language),
      `${metadata.id} global manifest description should mention ${language}`,
    );
    assert.equal(
      globalEntries.get(sourceManifest.id)?.description,
      description,
      `${metadata.id} global manifest description should match its source manifest`,
    );
  }
});

test("Redis marketplace entry tracks the released native driver", () => {
  const globalManifest = JSON.parse(fs.readFileSync(path.join(repoRoot, "manifest.json"), "utf8"));
  const redisDriverManifest = JSON.parse(
    fs.readFileSync(path.join(repoRoot, "extensions/ipc/redis/driver.json"), "utf8"),
  );
  const entry = globalManifest.extensions.find((extension) => extension.id === "redis");

  assert.ok(entry, "redis should be present in the marketplace manifest");
  assert.equal(
    entry.version,
    redisDriverManifest.version,
    "redis marketplace version should match its native driver manifest",
  );
  assert.equal(
    entry.release_tag,
    `redis-v${redisDriverManifest.version}`,
    "redis marketplace release tag should match its native driver version",
  );
});

test("WASM connection importers use the shared host connection-import WIT", () => {
  const sharedWit = path.join(repoRoot, "wit/connection-import.wit");
  assert.ok(fs.existsSync(sharedWit), "repository should vendor a single shared connection-import WIT");

  const wasmRoot = path.join(repoRoot, "extensions/wasm");
  for (const id of fs.existsSync(wasmRoot) ? fs.readdirSync(wasmRoot) : []) {
    assert.equal(
      fs.existsSync(path.join(wasmRoot, id, "wit/connection-import.wit")),
      false,
      `${id} should reference the shared repository WIT instead of carrying a private copy`,
    );
  }

  const hostWitCandidates = [
    path.resolve(
      repoRoot,
      "../onetcli/.worktrees/connection-import-center/crates/extension-api/wit/connection-import.wit",
    ),
    path.resolve(repoRoot, "../onetcli/crates/extension-api/wit/connection-import.wit"),
  ];
  const hostWit = hostWitCandidates.find((candidate) => fs.existsSync(candidate));
  if (hostWit) {
    assert.equal(
      fs.readFileSync(sharedWit, "utf8"),
      fs.readFileSync(hostWit, "utf8"),
      `shared connection-import WIT drifted from ${path.relative(repoRoot, hostWit)}`,
    );
  }
});

test("Xshell importer is registered as a composite WASM importer", () => {
  const globalManifest = JSON.parse(fs.readFileSync(path.join(repoRoot, "manifest.json"), "utf8"));
  const entry = globalManifest.extensions.find((extension) => extension.id === "com.onetcli.importer.xshell");

  assert.equal(entry?.kind, "composite");
  assert.equal(entry?.manifest, "xshell-importer/manifest.json");

  const sourceManifest = JSON.parse(
    fs.readFileSync(path.join(repoRoot, "extensions/wasm/xshell-importer/extension.json"), "utf8"),
  );
  const importer = sourceManifest.contributes.connectionImporters[0];

  assert.equal(importer.id, "xshell");
  assert.deepEqual(importer.outputKinds, ["ssh"]);
  assert.deepEqual(importer.platforms, ["macos", "windows", "linux"]);
  assert.equal(
    importer.manualFilePick?.prompt,
    "选择 Xshell .xsh/.xts 文件",
    "Xshell importer should expose manual .xsh/.xts file selection",
  );
  assert.equal(importer.manualFilePick?.supportsDirectories, true);
  assert.equal(importer.manualFilePick?.directoryPrompt, "选择 Xshell Sessions 配置目录");
  assert.ok(
    importer.candidateFiles.some((candidate) => candidate.path.includes("NetSarang Computer/8/Xshell/Sessions")),
    "Xshell importer should declare the current NetSarang sessions directory",
  );
});

test("Notes PDF exporter has enough fuel for rendered Mermaid SVG", () => {
  const sourceManifest = JSON.parse(
    fs.readFileSync(path.join(repoRoot, "extensions/wasm/notes-pdf-exporter/extension.json"), "utf8"),
  );
  const runtime = sourceManifest.runtime.wasm.find((entry) => entry.id === "notes-pdf-exporter");

  assert.equal(sourceManifest.version, "0.1.2");
  assert.equal(runtime?.fuel_per_call, 1_000_000_000);
  assert.equal(runtime?.timeout_ms, 30_000);
});

test("Notes HTML exporter has enough resources for self-contained network images", () => {
  const sourceManifest = JSON.parse(
    fs.readFileSync(path.join(repoRoot, "extensions/wasm/notes-html-exporter/extension.json"), "utf8"),
  );
  const runtime = sourceManifest.runtime.wasm.find((entry) => entry.id === "notes-html-exporter");

  assert.equal(sourceManifest.version, "0.1.2");
  assert.equal(runtime?.fuel_per_call, 2_000_000_000);
  assert.equal(runtime?.max_memory_mb, 256);
  assert.equal(runtime?.timeout_ms, 30_000);
});

test("Notes Word exporter has enough fuel for bounded multi-image documents", () => {
  const sourceManifest = JSON.parse(
    fs.readFileSync(path.join(repoRoot, "extensions/wasm/notes-word-exporter/extension.json"), "utf8"),
  );
  const runtime = sourceManifest.runtime.wasm.find((entry) => entry.id === "notes-word-exporter");

  assert.equal(sourceManifest.version, "0.1.2");
  assert.equal(runtime?.fuel_per_call, 1_000_000_000);
  assert.equal(runtime?.timeout_ms, 20_000);
});

test("Navicat importer is registered as a composite WASM importer", () => {
  const globalManifest = JSON.parse(fs.readFileSync(path.join(repoRoot, "manifest.json"), "utf8"));
  const entry = globalManifest.extensions.find((extension) => extension.id === "com.onetcli.importer.navicat");

  assert.equal(entry?.kind, "composite");
  assert.equal(entry?.manifest, "navicat-importer/manifest.json");

  const sourceManifest = JSON.parse(
    fs.readFileSync(path.join(repoRoot, "extensions/wasm/navicat-importer/extension.json"), "utf8"),
  );
  const importer = sourceManifest.contributes.connectionImporters[0];

  assert.equal(importer.id, "navicat");
  assert.deepEqual(importer.outputKinds, ["database"]);
  assert.equal(
    importer.manualFilePick?.prompt,
    "选择 Navicat 导出的 connection.ncx 文件",
    "Navicat importer should define the manual file picker prompt in its manifest",
  );
  const candidatePaths = importer.candidateFiles.map((candidate) => candidate.path);
  assert.ok(
    candidatePaths.some((candidatePath) => candidatePath.includes("com.prect.NavicatPremium")),
    "Navicat importer should declare macOS PremiumSoft preference plist candidates",
  );
  assert.ok(
    candidatePaths.includes("~/Library/Application Support/PremiumSoft CyberTech/Navicat CC/Common/conn.plist"),
    "Navicat importer should declare macOS Navicat Premium Lite conn.plist candidate",
  );
  assert.ok(
    candidatePaths.includes("%APPDATA%/PremiumSoft CyberTech/Navicat CC/Common/conn.plist"),
    "Navicat importer should declare Windows Navicat Premium Lite conn.plist candidate",
  );
  assert.ok(importer.platforms.includes("windows"), "Navicat Lite candidate should enable Windows scanning");
  assert.ok(
    sourceManifest.permissions.includes(
      "fs:read:~/Library/Application Support/PremiumSoft CyberTech/Navicat CC/Common/conn.plist",
    ),
    "Navicat importer should permit macOS Lite conn.plist reads",
  );
  assert.ok(
    sourceManifest.permissions.includes(
      "fs:read:%APPDATA%/PremiumSoft CyberTech/Navicat CC/Common/conn.plist",
    ),
    "Navicat importer should permit Windows Lite conn.plist reads",
  );
});

test("composite marketplace ids match their installed extension manifest ids", () => {
  const globalManifest = JSON.parse(
    fs.readFileSync(path.join(repoRoot, "manifest.json"), "utf8"),
  );

  for (const entry of globalManifest.extensions.filter(
    (extension) => extension.kind === "composite",
  )) {
    const artifactSlug = entry.manifest.split("/")[0];
    const metadata = extensionBuildEntries().find((candidate) => candidate.id === artifactSlug);
    assert.ok(metadata, `missing build metadata for ${artifactSlug}`);
    const sourceManifest = JSON.parse(
      fs.readFileSync(
        path.join(repoRoot, metadata.path, "extension.json"),
        "utf8",
      ),
    );

    assert.equal(
      entry.id,
      sourceManifest.id,
      `${artifactSlug} marketplace id must match its installed extension id`,
    );
  }
});

test("TablePlus importer is registered as a composite WASM importer", () => {
  const globalManifest = JSON.parse(fs.readFileSync(path.join(repoRoot, "manifest.json"), "utf8"));
  const entry = globalManifest.extensions.find((extension) => extension.id === "com.onetcli.importer.tableplus");

  assert.equal(entry?.kind, "composite");
  assert.equal(entry?.manifest, "tableplus-importer/manifest.json");

  const sourceManifest = JSON.parse(
    fs.readFileSync(path.join(repoRoot, "extensions/wasm/tableplus-importer/extension.json"), "utf8"),
  );
  const importer = sourceManifest.contributes.connectionImporters[0];

  assert.equal(importer.id, "tableplus");
  assert.deepEqual(importer.outputKinds, ["database"]);
  assert.ok(
    importer.candidateFiles.some((candidate) => candidate.path.includes("com.tinyapp.TablePlus")),
    "TablePlus importer should declare TablePlus app support candidates",
  );
});

test("JetBrains importer is registered as a composite WASM importer", () => {
  const globalManifest = JSON.parse(fs.readFileSync(path.join(repoRoot, "manifest.json"), "utf8"));
  const entry = globalManifest.extensions.find((extension) => extension.id === "com.onetcli.importer.jetbrains");

  assert.equal(entry?.kind, "composite");
  assert.equal(entry?.manifest, "jetbrains-importer/manifest.json");

  const sourceManifest = JSON.parse(
    fs.readFileSync(path.join(repoRoot, "extensions/wasm/jetbrains-importer/extension.json"), "utf8"),
  );
  const importer = sourceManifest.contributes.connectionImporters[0];

  assert.equal(importer.id, "jetbrains");
  assert.deepEqual(importer.outputKinds, ["database"]);
  assert.ok(
    importer.candidateFiles.some((candidate) => candidate.path.includes("Application Support/JetBrains")),
    "JetBrains importer should declare JetBrains app support candidates",
  );
});

test("MongoDB Compass importer is registered as a composite WASM importer", () => {
  const globalManifest = JSON.parse(fs.readFileSync(path.join(repoRoot, "manifest.json"), "utf8"));
  const entry = globalManifest.extensions.find((extension) => extension.id === "com.onetcli.importer.mongodb-compass");

  assert.equal(entry?.kind, "composite");
  assert.equal(entry?.manifest, "mongodb-compass-importer/manifest.json");

  const sourceManifest = JSON.parse(
    fs.readFileSync(path.join(repoRoot, "extensions/wasm/mongodb-compass-importer/extension.json"), "utf8"),
  );
  const importer = sourceManifest.contributes.connectionImporters[0];

  assert.equal(importer.id, "mongodb-compass");
  assert.deepEqual(importer.outputKinds, ["database"]);
  assert.ok(
    importer.candidateFiles.some((candidate) => candidate.path.includes("MongoDB Compass/Connections")),
    "MongoDB Compass importer should declare Compass Connections candidates",
  );
});

test("OpenSSH config importer is registered as a composite WASM importer", () => {
  const globalManifest = JSON.parse(fs.readFileSync(path.join(repoRoot, "manifest.json"), "utf8"));
  const entry = globalManifest.extensions.find((extension) => extension.id === "com.onetcli.importer.openssh-config");

  assert.equal(entry?.kind, "composite");
  assert.equal(entry?.manifest, "openssh-config-importer/manifest.json");

  const sourceManifest = JSON.parse(
    fs.readFileSync(path.join(repoRoot, "extensions/wasm/openssh-config-importer/extension.json"), "utf8"),
  );
  const importer = sourceManifest.contributes.connectionImporters[0];

  assert.equal(importer.id, "openssh-config");
  assert.deepEqual(importer.outputKinds, ["ssh"]);
  const candidatePaths = importer.candidateFiles.map((candidate) => candidate.path);
  assert.ok(
    candidatePaths.some((candidatePath) => candidatePath.includes(".ssh/config")),
    "OpenSSH config importer should declare user ssh config candidates",
  );
  assert.ok(
    candidatePaths.some((candidatePath) => candidatePath.includes(".ssh/known_hosts")),
    "OpenSSH config importer should declare user known_hosts candidates",
  );
  assert.ok(
    sourceManifest.permissions.some((permission) => permission.includes(".ssh/known_hosts")),
    "OpenSSH config importer should permit known_hosts reads",
  );
});

test("WindTerm importer is registered as a composite WASM importer", () => {
  const globalManifest = JSON.parse(fs.readFileSync(path.join(repoRoot, "manifest.json"), "utf8"));
  const entry = globalManifest.extensions.find((extension) => extension.id === "com.onetcli.importer.windterm");

  assert.equal(entry?.kind, "composite");
  assert.equal(entry?.manifest, "windterm-importer/manifest.json");

  const sourceManifest = JSON.parse(
    fs.readFileSync(path.join(repoRoot, "extensions/wasm/windterm-importer/extension.json"), "utf8"),
  );
  const importer = sourceManifest.contributes.connectionImporters[0];

  assert.equal(importer.id, "windterm");
  assert.deepEqual(importer.outputKinds, ["ssh"]);
  assert.deepEqual(importer.platforms, ["macos", "windows", "linux"]);
  assert.equal(importer.manualFilePick?.prompt, "选择 WindTerm user.sessions 文件");
  assert.equal(importer.manualFilePick?.supportsDirectories, true);
  assert.equal(importer.manualFilePick?.directoryPrompt, "选择 WindTerm profiles 配置目录");
  assert.ok(
    importer.candidateFiles.some((candidate) => candidate.path === "~/.wind/profiles"),
    "WindTerm importer should discover Unix profile directories",
  );
  assert.ok(
    importer.candidateFiles.some((candidate) => candidate.path === "%USERPROFILE%/.wind/profiles"),
    "WindTerm importer should discover Windows profile directories",
  );
  assert.ok(sourceManifest.permissions.includes("fs:read:~/.wind/profiles"));
  assert.ok(sourceManifest.permissions.includes("fs:read:%USERPROFILE%/.wind/profiles"));
});

test("MobaXterm importer is registered as a composite WASM importer", () => {
  const globalManifest = JSON.parse(fs.readFileSync(path.join(repoRoot, "manifest.json"), "utf8"));
  const entry = globalManifest.extensions.find((extension) => extension.id === "com.onetcli.importer.mobaxterm");

  assert.equal(entry?.kind, "composite");
  assert.equal(entry?.manifest, "mobaxterm-importer/manifest.json");

  const sourceManifest = JSON.parse(
    fs.readFileSync(path.join(repoRoot, "extensions/wasm/mobaxterm-importer/extension.json"), "utf8"),
  );
  const importer = sourceManifest.contributes.connectionImporters[0];

  assert.equal(importer.id, "mobaxterm");
  assert.equal(importer.runtimeId, "mobaxterm-importer");
  assert.deepEqual(importer.outputKinds, ["ssh"]);
  assert.deepEqual(importer.platforms, ["macos", "windows", "linux"]);
  assert.equal(importer.manualFilePick?.prompt, "选择 MobaXterm.ini / .mxtsessions 文件");
  assert.equal(importer.manualFilePick?.supportsDirectories, true);
  assert.equal(importer.manualFilePick?.directoryPrompt, "选择 MobaXterm 配置目录");
  assert.ok(
    importer.candidateFiles.some((candidate) => candidate.path === "%USERPROFILE%/Documents/MobaXterm"),
    "MobaXterm importer should discover Windows Documents configuration directories",
  );
  assert.ok(
    importer.candidateFiles.some((candidate) => candidate.path === "%APPDATA%/MobaXterm"),
    "MobaXterm importer should discover Windows AppData configuration directories",
  );
  assert.ok(sourceManifest.permissions.includes("fs:read:%USERPROFILE%/Documents/MobaXterm"));
  assert.ok(sourceManifest.permissions.includes("fs:read:%APPDATA%/MobaXterm"));
});

test("SecureCRT importer is registered as a composite WASM importer", () => {
  const globalManifest = JSON.parse(fs.readFileSync(path.join(repoRoot, "manifest.json"), "utf8"));
  const entry = globalManifest.extensions.find((extension) => extension.id === "com.onetcli.importer.securecrt");

  assert.equal(entry?.kind, "composite");
  assert.equal(entry?.manifest, "securecrt-importer/manifest.json");

  const buildMetadata = JSON.parse(
    fs.readFileSync(path.join(repoRoot, "extensions/wasm/securecrt-importer/extension.build.json"), "utf8"),
  );
  const sourceManifest = JSON.parse(
    fs.readFileSync(path.join(repoRoot, "extensions/wasm/securecrt-importer/extension.json"), "utf8"),
  );
  const importer = sourceManifest.contributes.connectionImporters[0];
  const candidatePaths = importer.candidateFiles.map((candidate) => candidate.path);

  assert.equal(buildMetadata.kind, "composite");
  assert.equal(buildMetadata.language, "rust-wasm");
  assert.equal(buildMetadata.package, "securecrt_importer_wasm");
  assert.equal(buildMetadata.binary, "securecrt_importer_wasm.wasm");
  assert.deepEqual(buildMetadata.targets, ["universal"]);
  assert.equal(importer.id, "securecrt");
  assert.deepEqual(importer.outputKinds, [
    "ssh",
    "quick-command",
    "workspace",
  ]);
  assert.deepEqual(importer.platforms, ["macos", "windows", "linux"]);
  assert.equal(
    importer.manualFilePick?.prompt,
    "选择 SecureCRT XML 导出文件、会话 .ini、Button Bar .ini 或 Command Manager __Commands__.ini 文件",
    "SecureCRT importer should expose manual XML/session/Button Bar/Command Manager selection",
  );
  assert.equal(importer.manualFilePick?.supportsDirectories, true);
  assert.equal(
    importer.manualFilePick?.directoryPrompt,
    "选择 SecureCRT 的 Config 配置根目录，以保留 Sessions 连接分组和 Commands 快捷命令分组",
  );
  assert.ok(
    candidatePaths.some((candidatePath) =>
      candidatePath.includes("~/Library/Application Support/VanDyke/SecureCRT/Config"),
    ),
    "SecureCRT importer should discover the current macOS config directory",
  );
  assert.ok(
    candidatePaths.includes("%APPDATA%/VanDyke/Config"),
    "SecureCRT importer should discover the Windows config directory",
  );
  assert.ok(
    candidatePaths.includes("~/.vandyke/Config"),
    "SecureCRT importer should discover the Linux config directory",
  );
  assert.ok(
    sourceManifest.permissions.includes("fs:read:%APPDATA%/VanDyke/Config"),
    "SecureCRT importer should permit Windows config reads",
  );
});

test("Redis desktop importer is registered as a composite WASM importer", () => {
  const globalManifest = JSON.parse(fs.readFileSync(path.join(repoRoot, "manifest.json"), "utf8"));
  const entry = globalManifest.extensions.find((extension) => extension.id === "com.onetcli.importer.redis-desktop");

  assert.equal(entry?.kind, "composite");
  assert.equal(entry?.manifest, "redis-desktop-importer/manifest.json");

  const sourceManifest = JSON.parse(
    fs.readFileSync(path.join(repoRoot, "extensions/wasm/redis-desktop-importer/extension.json"), "utf8"),
  );
  const importer = sourceManifest.contributes.connectionImporters[0];

  assert.equal(importer.id, "redis-desktop");
  assert.deepEqual(importer.outputKinds, ["database"]);
  assert.ok(
    importer.candidateFiles.some((candidate) => candidate.path.includes("com.hepengju.redis/store.json")),
    "Redis desktop importer should declare com.hepengju.redis store candidates",
  );
});

test("Notepad-- and Zed editors are registered as static composite extensions", () => {
  const globalManifest = JSON.parse(
    fs.readFileSync(path.join(repoRoot, "manifest.json"), "utf8"),
  );
  const expected = [
    {
      id: "com.onetcli.editor.notepad-minus-minus",
      name: "Notepad-- External Editor",
      version: "0.1.2",
      releaseTag: "notepad-minus-minus-editor-v0.1.2",
      manifest: "notepad-minus-minus-editor/manifest.json",
    },
    {
      id: "com.onetcli.editor.zed",
      name: "Zed External Editor",
      version: "0.1.2",
      releaseTag: "zed-editor-v0.1.2",
      manifest: "zed-editor/manifest.json",
    },
  ];

  for (const item of expected) {
    const entry = globalManifest.extensions.find(
      (extension) => extension.id === item.id,
    );
    assert.equal(entry?.kind, "composite");
    assert.equal(entry?.name, item.name);
    assert.equal(entry?.version, item.version);
    assert.equal(entry?.release_tag, item.releaseTag);
    assert.equal(entry?.manifest, item.manifest);
  }
});

test("Notepad-- and Zed editor manifests declare platform-specific commands", () => {
  const readManifest = (id) => JSON.parse(
    fs.readFileSync(
      path.join(repoRoot, `extensions/wasm/${id}/extension.json`),
      "utf8",
    ),
  );
  const notepadManifest = readManifest("notepad-minus-minus-editor");
  const zedManifest = readManifest("zed-editor");
  const [notepadMacos, notepadWindows] = notepadManifest.contributes.remoteFileEditors;
  const [zedMacos, zedLinux, zedWindows] = zedManifest.contributes.remoteFileEditors;

  assert.equal(notepadMacos.id, "notepad-minus-minus");
  assert.deepEqual(notepadMacos.platforms, ["macos"]);
  assert.deepEqual(notepadMacos.fileMasks, ["*"]);
  assert.equal(notepadMacos.priority, 100);
  assert.equal(notepadMacos.command.launchMode, "macos_open");
  assert.deepEqual(notepadMacos.command.programCandidates, [
    "/Applications/Notepad--.app/Contents/MacOS/Notepad--",
  ]);
  assert.deepEqual(notepadMacos.command.args, ["{file}"]);

  assert.equal(notepadWindows.id, "notepad-minus-minus-windows");
  assert.deepEqual(notepadWindows.platforms, ["windows"]);
  assert.equal(notepadWindows.command.launchMode, undefined);
  assert.deepEqual(notepadWindows.command.programCandidates, [
    "${env:ProgramFiles}\\Notepad--\\Notepad--.exe",
  ]);
  assert.deepEqual(notepadWindows.fileMasks, ["*"]);
  assert.equal(notepadWindows.priority, 100);
  assert.deepEqual(notepadWindows.command.args, ["{file}"]);

  assert.equal(zedMacos.id, "zed-macos");
  assert.deepEqual(zedMacos.platforms, ["macos"]);
  assert.equal(zedMacos.command.launchMode, "macos_open");
  assert.deepEqual(zedMacos.command.programCandidates, [
    "/Applications/Zed.app/Contents/MacOS/zed",
  ]);
  assert.equal(zedLinux.id, "zed-linux");
  assert.deepEqual(zedLinux.platforms, ["linux"]);
  assert.equal(zedLinux.command.launchMode, undefined);
  assert.deepEqual(zedLinux.command.programCandidates, ["zed"]);
  assert.deepEqual(zedMacos.fileMasks, ["*"]);
  assert.deepEqual(zedLinux.fileMasks, ["*"]);
  assert.equal(zedMacos.priority, 90);
  assert.equal(zedLinux.priority, 90);
  assert.deepEqual(zedMacos.command.args, ["{file}"]);
  assert.deepEqual(zedLinux.command.args, ["{file}"]);

  assert.equal(zedWindows.id, "zed-windows");
  assert.deepEqual(zedWindows.platforms, ["windows"]);
  assert.equal(zedWindows.command.launchMode, undefined);
  assert.deepEqual(zedWindows.command.programCandidates, [
    "${env:ProgramFiles}\\Zed\\Zed.exe",
  ]);
  assert.deepEqual(zedWindows.fileMasks, ["*"]);
  assert.equal(zedWindows.priority, 90);
  assert.deepEqual(zedWindows.command.args, ["{file}"]);
});

test("R2 upload workflow handles composite extension assets", () => {
  const workflow = fs.readFileSync(path.join(repoRoot, ".github/workflows/upload-r2.yml"), "utf8");
  const compositeAssetName = "${process.env.EXTENSION_ID}-composite-${target}.tar.gz";
  const compositePackagePattern = "${{ steps.release.outputs.extension_id }}-composite-*.tar.gz";

  assert.ok(
    workflow.includes(compositePackagePattern),
    "Download GitHub Release assets should request composite packages",
  );
  assert.equal(
    workflow.split(compositeAssetName).length - 1,
    2,
    "Verify and upload steps should both generate composite package names",
  );
});

test("R2 upload workflow handles language extension assets", () => {
  const workflow = fs.readFileSync(path.join(repoRoot, ".github/workflows/upload-r2.yml"), "utf8");
  const languageAssetName = "${process.env.EXTENSION_ID}-language-${target}.tar.gz";
  const languagePackagePattern = "${{ steps.release.outputs.extension_id }}-language-*.tar.gz";

  assert.ok(
    workflow.includes(languagePackagePattern),
    "Download GitHub Release assets should request language packages",
  );
  assert.equal(
    workflow.split(languageAssetName).length - 1,
    2,
    "Verify and upload steps should both generate language package names",
  );
});

test("Windows x86 backfill matrix resolves every supported published extension", () => {
  const output = execFileSync(
    "node",
    [path.join(repoRoot, "scripts/backfill-windows-x86.mjs"), "matrix", "all"],
    { cwd: repoRoot, encoding: "utf8" },
  );
  const matrix = JSON.parse(output);

  assert.equal(matrix.include.length, 17);
  assert.deepEqual(
    matrix.include.map((entry) => entry.extension),
    [
      "claude-acp",
      "codex-acp",
      "dm",
      "duckdb",
      "elasticsearch",
      "iotdb",
      "kingbase",
      "mongodb-legacy",
      "mongodb-legacy-3-2",
      "mongodb-modern",
      "oceanbase",
      "opencode-acp",
      "opengauss",
      "oracle-go",
      "rdp",
      "redis",
      "vnc",
    ],
  );
  assert.ok(matrix.include.every((entry) => entry.target === "i686-pc-windows-msvc"));
  assert.ok(matrix.include.every((entry) => entry.version && entry.release_tag));
});

test("Windows x86 backfill merge preserves old targets and adds the new target", () => {
  const workdir = fs.mkdtempSync(path.join(os.tmpdir(), "navop-x86-backfill-"));
  const existingDir = path.join(workdir, "existing");
  const outputDir = path.join(workdir, "merged");
  const newPackageDir = path.join(workdir, "new");
  fs.mkdirSync(existingDir, { recursive: true });
  fs.mkdirSync(newPackageDir, { recursive: true });

  const metadata = JSON.parse(
    fs.readFileSync(path.join(repoRoot, "extensions/ipc/dm/extension.build.json"), "utf8"),
  );
  const oldTargets = metadata.targets.filter((target) => target !== "i686-pc-windows-msvc");
  const artifacts = {};
  const checksumLines = [];
  for (const target of oldTargets) {
    const file = `dm-driver-${target}.tar.gz`;
    const contents = Buffer.from(`old package ${target}`);
    fs.writeFileSync(path.join(existingDir, file), contents);
    const sha256 = createHash("sha256").update(contents).digest("hex");
    artifacts[target] = { file, sha256 };
    checksumLines.push(`${sha256}  ${file}`);
  }
  fs.writeFileSync(path.join(existingDir, "sha256sums.txt"), `${checksumLines.join("\n")}\n`);
  fs.writeFileSync(
    path.join(existingDir, "extension-manifest.json"),
    `${JSON.stringify({
      schema_version: 2,
      release_version: "dm-v0.1.5",
      extensions: [{
        id: "dm",
        kind: "database_driver",
        name: "Dameng DM",
        version: "0.1.5",
        release_tag: "dm-v0.1.5",
        description: "",
        artifacts,
      }],
    }, null, 2)}\n`,
  );
  const newPackage = path.join(
    newPackageDir,
    "dm-driver-i686-pc-windows-msvc.tar.gz",
  );
  fs.writeFileSync(newPackage, "new Windows x86 package");

  const result = JSON.parse(execFileSync(
    "node",
    [
      path.join(repoRoot, "scripts/backfill-windows-x86.mjs"),
      "merge",
      "--extension", "dm",
      "--version", "0.1.5",
      "--release-tag", "dm-v0.1.5",
      "--existing-dir", existingDir,
      "--new-package", newPackage,
      "--output-dir", outputDir,
    ],
    { cwd: repoRoot, encoding: "utf8" },
  ));
  const merged = JSON.parse(
    fs.readFileSync(path.join(outputDir, "extension-manifest.json"), "utf8"),
  );

  assert.equal(result.package_action, "upload");
  assert.equal(result.target_count, metadata.targets.length);
  assert.deepEqual(Object.keys(merged.extensions[0].artifacts), metadata.targets);
  assert.ok(fs.statSync(path.join(outputDir, result.package_file)).size > 0);
  assert.equal(
    fs.readFileSync(path.join(outputDir, "sha256sums.txt"), "utf8").trim().split("\n").length,
    metadata.targets.length,
  );
});

test("Windows x86 backfill refuses to overwrite a mismatched existing package", () => {
  const script = fs.readFileSync(
    path.join(repoRoot, "scripts/backfill-windows-x86.mjs"),
    "utf8",
  );
  assert.match(script, /already exists with a different checksum; rerun with --force/);
  assert.match(script, /merged manifest dropped old target/);
  assert.match(script, /old manifest is missing target/);
});

test("temporary Windows x86 backfill workflow is manual, dry-run by default, and guarded", () => {
  const workflow = fs.readFileSync(
    path.join(repoRoot, ".github/workflows/backfill-windows-x86.yml"),
    "utf8",
  );
  const r2Workflow = fs.readFileSync(
    path.join(repoRoot, ".github/workflows/upload-r2.yml"),
    "utf8",
  );

  assert.match(workflow, /^name: Backfill Windows x86 extension artifacts/m);
  assert.match(workflow, /\n  workflow_dispatch:\n/);
  assert.doesNotMatch(workflow, /\n  workflow_run:/);
  assert.doesNotMatch(workflow, /\n  push:/);
  assert.match(workflow, /publish_release:[\s\S]*default: false/);
  assert.match(workflow, /publish_r2:[\s\S]*default: false/);
  assert.match(workflow, /force:[\s\S]*default: false/);
  assert.match(workflow, /runs-on: windows-2022/);
  assert.match(workflow, /i686-pc-windows-msvc/);
  assert.match(workflow, /scripts\/verify-package\.sh/);
  assert.match(workflow, /scripts\/verify-remote-desktop-provider-package\.sh/);
  assert.match(workflow, /scripts\/verify-acp-agent-package\.sh/);
  assert.match(workflow, /gh release download "\$RELEASE_TAG"/);
  assert.match(workflow, /scripts\/backfill-windows-x86\.mjs merge/);
  assert.match(workflow, /\n  merge:\n/);
  assert.match(workflow, /\n  publish-release:\n/);
  assert.match(workflow, /package_action="\$\(node -e/);
  assert.match(workflow, /upload\)/);
  assert.match(workflow, /replace\)/);
  assert.match(workflow, /skip\)/);
  assert.match(workflow, /--clobber/);
  assert.match(workflow, /max-parallel: 1/);
  assert.match(workflow, /merge:\n[\s\S]*permissions:\n      contents: read/);
  assert.match(workflow, /publish-release:\n[\s\S]*permissions:\n      contents: write/);
  assert.match(workflow, /uses: \.\/\.github\/workflows\/upload-r2\.yml/);
  assert.match(workflow, /with:\n      tag: \$\{\{ matrix\.release_tag \}\}/);
  assert.match(workflow, /secrets: inherit/);
  assert.match(workflow, /group: windows-x86-extension-backfill/);
  assert.match(workflow, /cancel-in-progress: false/);
  assert.match(r2Workflow, /\n  workflow_call:\n/);
  assert.match(r2Workflow, /workflow_call:\n    inputs:\n      tag:/);
  assert.match(r2Workflow, /github\.event_name == 'workflow_call'/);
  assert.match(r2Workflow, /extension manifest artifact mismatch/);
});

test("RDP helper keeps native TLS backend for RDP compatibility", () => {
  const cargoToml = fs.readFileSync(
    path.join(repoRoot, "extensions/remote-desktop/rdp-helper/Cargo.toml"),
    "utf8",
  );

  assert.match(cargoToml, /ironrdp-client\s*=\s*\{[^}]*default-features\s*=\s*false/s);
  assert.match(cargoToml, /ironrdp-client\s*=\s*\{[^}]*"native-tls"/s);
  assert.match(cargoToml, /native-tls\s*=\s*"=0\.2\.14"/);
  assert.doesNotMatch(cargoToml, /"rustls"/);
});

test("IPC driver form fields include host-required defaults", () => {
  const ids = fs
    .readdirSync(path.join(repoRoot, "extensions/ipc"))
    .filter((id) => fs.existsSync(path.join(repoRoot, "extensions/ipc", id, "driver.json")))
    .sort();
  const requiredKeys = [
    "default_value",
    "placeholder_i18n_key",
    "help_i18n_key",
    "options",
    "options_source",
    "visible_when",
    "default_when",
    "disabled_when_editing",
    "rows",
    "min",
    "max",
  ];

  for (const id of ids) {
    const driverJson = JSON.parse(
      fs.readFileSync(path.join(repoRoot, "extensions/ipc", id, "driver.json"), "utf8"),
    );
    for (const form of driverJson.ui?.form?.forms || []) {
      for (const tab of form.tabs || []) {
        for (const field of tab.fields || []) {
          for (const key of requiredKeys) {
            assert.ok(Object.hasOwn(field, key), `${id} field ${field.id} missing ${key}`);
          }
        }
      }
    }
  }
});

test("DuckDB connection form declares static defaults", () => {
  const driverJson = JSON.parse(
    fs.readFileSync(path.join(repoRoot, "extensions/ipc/duckdb/driver.json"), "utf8"),
  );
  const connectionForm = driverJson.ui.form.forms.find((form) => form.kind === "Connection");
  const generalTab = connectionForm.tabs.find((tab) => tab.id === "general");

  assert.equal(
    generalTab.fields.find((field) => field.id === "name").default_value,
    "Local DuckDB",
  );
  assert.equal(
    generalTab.fields.find((field) => field.id === "database").default_value,
    "main",
  );
});

test("IPC driver locales define every manifest i18n key", () => {
  const ids = fs
    .readdirSync(path.join(repoRoot, "extensions/ipc"))
    .filter((id) => fs.existsSync(path.join(repoRoot, "extensions/ipc", id, "driver.json")))
    .sort();

  for (const id of ids) {
    const driverJson = JSON.parse(
      fs.readFileSync(path.join(repoRoot, "extensions/ipc", id, "driver.json"), "utf8"),
    );
    const keys = new Set(
      [...collectI18nKeys(driverJson)].filter(
        (key) => key.startsWith("database.") || key.startsWith("common."),
      ),
    );
    if (keys.size === 0) continue;

    const localesDir = path.join(repoRoot, "extensions/ipc", id, driverJson.ui?.locales_dir || "locales");
    for (const locale of ["en.yml", "zh-CN.yml", "zh-HK.yml"]) {
      const localePath = path.join(localesDir, locale);
      assert.ok(fs.existsSync(localePath), `${id} missing locale ${locale}`);
      const localeText = fs.readFileSync(localePath, "utf8");
      for (const key of keys) {
        assert.ok(
          localeDefinesKey(localeText, key),
          `${id} ${locale} missing i18n key ${key}`,
        );
      }
    }
  }
});

test("Oracle Go IPC driver uses an external-driver id that cannot collide with built-in Oracle", () => {
  const driverDir = path.join(repoRoot, "extensions/ipc/oracle-go");
  const metadata = JSON.parse(
    fs.readFileSync(path.join(driverDir, "extension.build.json"), "utf8"),
  );
  const driverJson = JSON.parse(fs.readFileSync(path.join(driverDir, "driver.json"), "utf8"));

  assert.equal(fs.existsSync(path.join(repoRoot, "extensions/ipc/oracle")), false);
  assert.equal(metadata.id, "oracle-go");
  assert.equal(metadata.path, "extensions/ipc/oracle-go");
  assert.equal(metadata.binary, "oracle-go-ipc-driver");
  assert.equal(metadata.releaseTagPrefix, "oracle-go-v");
  assert.equal(metadata.r2Prefix, "extensions/oracle-go");
  assert.equal(driverJson.id, "oracle-go");
  assert.equal(driverJson.entry.command, "./oracle-go-ipc-driver");
  assert.equal(driverJson.transport.name, "oracle-go-driver.sock");
});

test("Oracle Go connection form does not expose a generic database field", () => {
  const driverJson = JSON.parse(
    fs.readFileSync(path.join(repoRoot, "extensions/ipc/oracle-go/driver.json"), "utf8"),
  );
  const connectionForm = driverJson.ui?.form?.forms?.find((form) => form.kind === "Connection");
  assert.ok(connectionForm, "oracle-go should declare a Connection form");
  const fieldIds = connectionForm.tabs.flatMap((tab) =>
    (tab.fields || []).map((field) => field.id),
  );

  assert.equal(fieldIds.includes("database"), false);
  assert.deepEqual(
    fieldIds.filter((id) => id === "service_name" || id === "sid"),
    ["service_name", "sid"],
  );
});

test("Oracle Go declares schemas as database-level nodes", () => {
  const driverJson = JSON.parse(
    fs.readFileSync(path.join(repoRoot, "extensions/ipc/oracle-go/driver.json"), "utf8"),
  );

  assert.equal(driverJson.dialect.uses_schema_as_database, true);
  assert.equal(driverJson.capabilities.uses_schema_as_database, true);
});

test("Oracle Go and OceanBase plugin locales define all packaged UI i18n keys", () => {
  for (const id of ["oracle-go", "oceanbase"]) {
    const driverJson = JSON.parse(
      fs.readFileSync(path.join(repoRoot, "extensions/ipc", id, "driver.json"), "utf8"),
    );
    const keys = [...collectI18nKeys(driverJson)].filter(isPackagedUiI18nKey);
    assert.ok(keys.length > 0, `${id} should declare packaged i18n keys`);

    const localesDir = path.join(repoRoot, "extensions/ipc", id, driverJson.ui?.locales_dir || "locales");
    for (const locale of ["en.yml", "zh-CN.yml", "zh-HK.yml"]) {
      const localePath = path.join(localesDir, locale);
      assert.ok(fs.existsSync(localePath), `${id} missing locale ${locale}`);
      const localeText = fs.readFileSync(localePath, "utf8");
      for (const key of keys) {
        assert.ok(
          localeDefinesKey(localeText, key),
          `${id} ${locale} missing i18n key ${key}`,
        );
      }
    }
  }
});

test("IPC driver icon paths reference packaged files", () => {
  const ids = fs
    .readdirSync(path.join(repoRoot, "extensions/ipc"))
    .filter((id) => fs.existsSync(path.join(repoRoot, "extensions/ipc", id, "driver.json")))
    .sort();

  for (const id of ids) {
    const driverDir = path.join(repoRoot, "extensions/ipc", id);
    const driverJson = JSON.parse(fs.readFileSync(path.join(driverDir, "driver.json"), "utf8"));
    for (const key of ["icon", "icon_color"]) {
      const icon = driverJson.ui?.[key];
      if (typeof icon !== "string" || !isRelativeAssetPath(icon)) continue;
      assert.ok(fs.existsSync(path.join(driverDir, icon)), `${id} ui.${key} missing ${icon}`);
    }
  }
});

test("IPC driver categories keep domestic database routing manifest-driven", () => {
  const ids = fs
    .readdirSync(path.join(repoRoot, "extensions/ipc"))
    .filter((id) => fs.existsSync(path.join(repoRoot, "extensions/ipc", id, "driver.json")))
    .sort();
  const domesticIds = [];

  for (const id of ids) {
    const driverJson = JSON.parse(
      fs.readFileSync(path.join(repoRoot, "extensions/ipc", id, "driver.json"), "utf8"),
    );
    assert.ok(
      !Object.hasOwn(driverJson.ui || {}, "category"),
      `${id} category must be declared at manifest top level, not ui.category`,
    );
    if (driverJson.category === "domestic_database") {
      domesticIds.push(id);
    } else {
      assert.equal(
        driverJson.category,
        undefined,
        `${id} uses unsupported driver category ${driverJson.category}`,
      );
    }
  }

  assert.deepEqual(domesticIds, ["dm", "gbase8s", "kingbase", "oceanbase", "opengauss", "oscar"]);
});

test("IPC connection form extra params use raw extra parameter keys", () => {
  const ids = fs
    .readdirSync(path.join(repoRoot, "extensions/ipc"))
    .filter((id) => fs.existsSync(path.join(repoRoot, "extensions/ipc", id, "driver.json")))
    .sort();
  const basicFields = new Set([
    "name",
    "host",
    "port",
    "username",
    "password",
    "database",
    "remark",
    "service_name",
    "sid",
  ]);

  for (const id of ids) {
    const driverJson = JSON.parse(
      fs.readFileSync(path.join(repoRoot, "extensions/ipc", id, "driver.json"), "utf8"),
    );
    for (const form of driverJson.ui?.form?.forms || []) {
      for (const tab of form.tabs || []) {
        for (const field of tab.fields || []) {
          assert.ok(
            !field.id.startsWith("extra_params."),
            `${id} form field ${field.id} should be ${field.id.slice("extra_params.".length)}; non-basic connection form fields are already stored in extra_params`,
          );
          if (field.id === "external_driver_id") continue;
          if (basicFields.has(field.id)) continue;
          assert.ok(
            !field.id.includes("."),
            `${id} extra param form field ${field.id} should use the raw extra_params key without a dotted namespace`,
          );
        }
      }
    }
  }
});

test("IPC driver connection forms declare host-managed SSH and remark tabs", () => {
  const ids = fs
    .readdirSync(path.join(repoRoot, "extensions/ipc"))
    .filter((id) => fs.existsSync(path.join(repoRoot, "extensions/ipc", id, "driver.json")))
    .sort();

  for (const id of ids) {
    const driverJson = JSON.parse(
      fs.readFileSync(path.join(repoRoot, "extensions/ipc", id, "driver.json"), "utf8"),
    );
    if (driverJson.api && driverJson.api !== "database") continue;
    const connectionForm = driverJson.ui?.form?.forms?.find((form) => form.kind === "Connection");
    assert.ok(connectionForm, `${id} should declare a Connection form`);

    const tabs = connectionForm.tabs || [];
    for (const tabId of ["ssh", "remark"]) {
      const tab = tabs.find((candidate) => candidate.id === tabId);
      assert.ok(tab, `${id} should declare the host-managed ${tabId} tab`);
      assert.deepEqual(
        tab.fields,
        [],
        `${id} ${tabId} tab should let the host provide its managed fields`,
      );
    }
  }
});

test("IPC driver manifests expose context menu actions for supported object workflows", () => {
  const ids = fs
    .readdirSync(path.join(repoRoot, "extensions/ipc"))
    .filter((id) => fs.existsSync(path.join(repoRoot, "extensions/ipc", id, "driver.json")))
    .sort();

  for (const id of ids) {
    const driverJson = JSON.parse(
      fs.readFileSync(path.join(repoRoot, "extensions/ipc", id, "driver.json"), "utf8"),
    );
    if (driverJson.api && driverJson.api !== "database") continue;
    const actions = driverJson.ui?.form?.actions?.actions;
    assert.ok(Array.isArray(actions), `${id} should declare ui.form.actions.actions`);

    assertHasAction(actions, id, "CloseConnection", "Connection");
    assertHasAction(actions, id, "DeleteConnection", "Connection");

    if (driverJson.methods.includes("exec/batch")) {
      assertHasAction(actions, id, "RunSqlFile", "Connection");
      assertHasAction(actions, id, "RunSqlFile", "Database");
      assertHasAction(actions, id, "RunSqlFile", "Schema");
    }
    if (driverJson.methods.includes("ddl/build_create_table")) {
      assertHasAction(actions, id, "DesignTable", "Schema");
      assertHasAction(actions, id, "DesignTable", "TablesFolder");
      assertHasAction(actions, id, "DesignTable", "Table");
    }
    if (driverJson.methods.includes("data/export")) {
      assertHasAction(actions, id, "OpenTableData", "Table");
      assertHasAction(actions, id, "ExportData", "Table");
      assertHasAction(actions, id, "OpenViewData", "View");
    }
    if (driverJson.methods.includes("data/import_begin")) {
      assertHasAction(actions, id, "ImportData", "Table");
    }
    if (driverJson.methods.includes("schema/dump_ddl")) {
      assertHasAction(actions, id, "DumpSqlStructure", "Database");
      assertHasAction(actions, id, "DumpSqlStructure", "Schema");
      assertHasAction(actions, id, "DumpSqlStructure", "Table");
      assertHasAction(actions, id, "DumpSqlData", "Table");
      assertHasAction(actions, id, "DumpSqlStructureAndData", "Table");
    }
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
    fs.readFileSync(path.join(workdir, "unpacked/driver.json"), "utf8"),
  );
  assert.equal(driverJson.version, "1.2.3");
  assert.equal(driverJson.entry.command, "./duckdb_driver");
  assert.equal(
    fs.readFileSync(path.join(workdir, "unpacked/duckdb_driver"), "utf8"),
    "fake binary\n",
  );
});

test("package-driver allows native drivers without locales", () => {
  const workdir = makeTempDir();
  createPackageFixture(workdir, {
    id: "redis",
    binary: "redis-driver",
    package: "redis-driver",
    withoutLocales: true,
    driverJson: {
      id: "redis",
      api: "redis",
      version: "0.0.0",
      entry: {},
      methods: ["conn/open"],
    },
  });

  const archivePath = execFileSync(
    "bash",
    [
      path.join(workdir, "scripts/package-driver.sh"),
      "redis",
      "x86_64-unknown-linux-gnu",
      path.join(workdir, "artifacts"),
      "1.2.3",
    ],
    { cwd: workdir, encoding: "utf8" },
  ).trim();

  execFileSync("bash", [path.join(workdir, "scripts/verify-package.sh"), archivePath], {
    cwd: workdir,
    encoding: "utf8",
  });
  execFileSync("tar", ["xzf", archivePath, "-C", path.join(workdir, "unpacked")]);
  assert.equal(fs.existsSync(path.join(workdir, "unpacked/locales")), false);
});

test("package-remote-desktop-provider creates an RDP provider package", () => {
  const workdir = makeTempDir();
  createRemoteDesktopProviderFixture(workdir);

  const archivePath = execFileSync(
    "bash",
    [
      path.join(workdir, "scripts/package-remote-desktop-provider.sh"),
      "rdp",
      "x86_64-unknown-linux-gnu",
      path.join(workdir, "artifacts"),
      "1.2.3",
    ],
    { cwd: workdir, encoding: "utf8" },
  ).trim();

  assert.equal(
    path.basename(archivePath),
    "rdp-remote-desktop-provider-x86_64-unknown-linux-gnu.tar.gz",
  );
  execFileSync("tar", ["xzf", archivePath, "-C", path.join(workdir, "unpacked")]);

  const manifest = JSON.parse(
    fs.readFileSync(path.join(workdir, "unpacked/remote_desktop_provider.json"), "utf8"),
  );
  assert.equal(manifest.version, "1.2.3");
  assert.equal(manifest.entry.command, "./navop-rdp-helper");
  assert.equal(
    fs.readFileSync(path.join(workdir, "unpacked/navop-rdp-helper"), "utf8"),
    "fake rdp helper\n",
  );

  const output = execFileSync(
    "bash",
    [path.join(workdir, "scripts/verify-remote-desktop-provider-package.sh"), archivePath],
    { cwd: workdir, encoding: "utf8" },
  );
  assert.match(output, /Verified/);
});

test("package-mcp-helper creates a Public MCP helper package", () => {
  const workdir = makeTempDir();
  createMcpHelperFixture(workdir);

  const archivePath = execFileSync(
    "bash",
    [
      path.join(workdir, "scripts/package-mcp-helper.sh"),
      "onetcli-public-mcp",
      "x86_64-unknown-linux-gnu",
      path.join(workdir, "artifacts"),
      "1.2.3",
    ],
    { cwd: workdir, encoding: "utf8" },
  ).trim();

  assert.equal(
    path.basename(archivePath),
    "onetcli-public-mcp-mcp-helper-x86_64-unknown-linux-gnu.tar.gz",
  );
  execFileSync("tar", ["xzf", archivePath, "-C", path.join(workdir, "unpacked")]);

  const manifest = JSON.parse(
    fs.readFileSync(path.join(workdir, "unpacked/mcp_helper.json"), "utf8"),
  );
  assert.equal(manifest.version, "1.2.3");
  assert.equal(manifest.entry.command, "./onetcli-public-mcp");
  assert.equal(
    fs.readFileSync(path.join(workdir, "unpacked/onetcli-public-mcp"), "utf8"),
    "fake onetcli-public-mcp helper\n",
  );

  execFileSync(
    "bash",
    [path.join(workdir, "scripts/verify-mcp-helper-package.sh"), archivePath],
    { cwd: workdir, encoding: "utf8" },
  );
});

test("package-acp-agent creates a Codex ACP agent package", () => {
  const workdir = makeTempDir();
  createAcpAgentFixture(workdir, {
    id: "codex-acp",
    binary: "codex-acp",
    packageName: "@agentclientprotocol/codex-acp@1.0.1",
  });

  const archivePath = execFileSync(
    "bash",
    [
      path.join(workdir, "scripts/package-acp-agent.sh"),
      "codex-acp",
      "x86_64-unknown-linux-gnu",
      path.join(workdir, "artifacts"),
      "1.2.3",
    ],
    { cwd: workdir, encoding: "utf8" },
  ).trim();

  assert.equal(
    path.basename(archivePath),
    "codex-acp-acp-agent-x86_64-unknown-linux-gnu.tar.gz",
  );
  execFileSync("tar", ["xzf", archivePath, "-C", path.join(workdir, "unpacked")]);

  const manifest = JSON.parse(
    fs.readFileSync(path.join(workdir, "unpacked/acp_agent.json"), "utf8"),
  );
  assert.equal(manifest.version, "1.2.3");
  assert.equal(manifest.agents[0].transport.type, "stdio");
  assert.equal(manifest.agents[0].transport.command, "bin/codex-acp");
  assert.deepEqual(manifest.agents[0].transport.args, []);
  assert.match(
    fs.readFileSync(path.join(workdir, "unpacked/bin/codex-acp"), "utf8"),
    /@agentclientprotocol\/codex-acp@1\.0\.1/,
  );

  execFileSync(
    "bash",
    [path.join(workdir, "scripts/verify-acp-agent-package.sh"), archivePath],
    { cwd: workdir, encoding: "utf8" },
  );
});

test("OpenCode ACP extension launches local opencode acp", () => {
  const metadata = JSON.parse(
    fs.readFileSync(
      path.join(repoRoot, "extensions/acp-agent/opencode-acp/extension.build.json"),
      "utf8",
    ),
  );
  const manifest = JSON.parse(
    fs.readFileSync(
      path.join(repoRoot, "extensions/acp-agent/opencode-acp/acp_agent.json"),
      "utf8",
    ),
  );

  assert.equal(metadata.id, "opencode-acp");
  assert.equal(metadata.kind, "acp_agent");
  assert.equal(metadata.language, "shell");
  assert.equal(metadata.binary, "opencode-acp");
  assert.equal(manifest.id, "opencode-acp");
  assert.equal(manifest.name, "OpenCode");
  assert.equal(manifest.agents[0].transport.type, "stdio");
  assert.equal(manifest.agents[0].transport.command, "bin/opencode-acp");
  assert.deepEqual(manifest.agents[0].transport.args, []);

  const unixLauncher = fs.readFileSync(
    path.join(repoRoot, "extensions/acp-agent/opencode-acp/bin/opencode-acp"),
    "utf8",
  );
  const windowsLauncher = fs.readFileSync(
    path.join(repoRoot, "extensions/acp-agent/opencode-acp/bin/opencode-acp.cmd"),
    "utf8",
  );
  assert.match(unixLauncher, /acp "\$@"/);
  assert.match(windowsLauncher, /opencode acp %\*/);
  assert.doesNotMatch(unixLauncher, /npm/);
  assert.doesNotMatch(windowsLauncher, /npm/);
});

test("OpenCode ACP launcher finds nvm-installed opencode without shell PATH", () => {
  const workdir = makeTempDir();
  const binDir = path.join(workdir, ".nvm/versions/node/v99.0.0/bin");
  fs.mkdirSync(binDir, { recursive: true });
  fs.writeFileSync(
    path.join(binDir, "opencode"),
    `#!/usr/bin/env node\n`,
    { mode: 0o755 },
  );
  fs.writeFileSync(
    path.join(binDir, "node"),
    `#!/usr/bin/env sh\nshift\nprintf '%s\\n' "$@" > "$HOME/opencode-args.txt"\n`,
    { mode: 0o755 },
  );

  execFileSync(
    path.join(repoRoot, "extensions/acp-agent/opencode-acp/bin/opencode-acp"),
    {
      cwd: workdir,
      env: {
        HOME: workdir,
        PATH: "/usr/bin:/bin",
      },
      encoding: "utf8",
    },
  );

  assert.equal(fs.readFileSync(path.join(workdir, "opencode-args.txt"), "utf8"), "acp\n");
});

test("package-composite-extension creates a DBeaver importer package", () => {
  const workdir = makeTempDir();
  createDbeaverImporterFixture(workdir);

  const archivePath = execFileSync(
    "bash",
    [
      path.join(workdir, "scripts/package-composite-extension.sh"),
      "dbeaver-importer",
      "universal",
      path.join(workdir, "artifacts"),
      "1.2.3",
    ],
    { cwd: workdir, encoding: "utf8" },
  ).trim();

  assert.equal(
    path.basename(archivePath),
    "dbeaver-importer-composite-universal.tar.gz",
  );
  execFileSync("tar", ["xzf", archivePath, "-C", path.join(workdir, "unpacked")]);

  const manifest = JSON.parse(
    fs.readFileSync(path.join(workdir, "unpacked/extension.json"), "utf8"),
  );
  assert.equal(manifest.version, "1.2.3");
  assert.equal(manifest.contributes.connectionImporters.length, 1);
  assert.equal(manifest.contributes.connectionImporters[0].id, "dbeaver");
  assert.equal(
    fs.readFileSync(path.join(workdir, "unpacked/wasm/dbeaver_importer_wasm.wasm"), "utf8"),
    "fake wasm\n",
  );

  execFileSync(
    "bash",
    [path.join(workdir, "scripts/verify-composite-package.sh"), archivePath],
    { cwd: workdir, encoding: "utf8" },
  );
});

test("package-composite-extension creates a static remote editor package", () => {
  const workdir = makeTempDir();
  createStaticRemoteEditorFixture(workdir);

  const archivePath = execFileSync(
    "bash",
    [
      path.join(workdir, "scripts/package-composite-extension.sh"),
      "notepad-plus-plus-editor",
      "universal",
      path.join(workdir, "artifacts"),
      "1.2.3",
    ],
    { cwd: workdir, encoding: "utf8" },
  ).trim();

  assert.equal(
    path.basename(archivePath),
    "notepad-plus-plus-editor-composite-universal.tar.gz",
  );
  execFileSync("tar", ["xzf", archivePath, "-C", path.join(workdir, "unpacked")]);
  const manifest = JSON.parse(
    fs.readFileSync(path.join(workdir, "unpacked/extension.json"), "utf8"),
  );
  assert.equal(manifest.version, "1.2.3");
  assert.equal(manifest.runtime, undefined);
  assert.equal(manifest.contributes.remoteFileEditors[0].displayName, "Notepad++");
  execFileSync(
    "bash",
    [path.join(workdir, "scripts/verify-composite-package.sh"), archivePath],
    { cwd: workdir, encoding: "utf8" },
  );
});

test("package-composite-extension creates a native shell provider package", () => {
  const workdir = makeTempDir();
  createNativeCompositeFixture(workdir);

  const archivePath = execFileSync(
    "bash",
    [
      path.join(workdir, "scripts/package-composite-extension.sh"),
      "elasticsearch",
      "x86_64-unknown-linux-gnu",
      path.join(workdir, "artifacts"),
      "1.2.3",
    ],
    { cwd: workdir, encoding: "utf8" },
  ).trim();

  execFileSync("tar", ["xzf", archivePath, "-C", path.join(workdir, "unpacked")]);
  const manifest = JSON.parse(
    fs.readFileSync(path.join(workdir, "unpacked/extension.json"), "utf8"),
  );
  assert.equal(manifest.version, "1.2.3");
  assert.equal(manifest.runtime.ipc[0].entry.command, "./bin/elasticsearch-provider");
  assert.equal(manifest.contributes.connections[0].id, "elasticsearch");
  assert.equal(manifest.contributes.connections[0].shellViewId, "explorer");
  assert.ok(manifest.permissions.includes("spawn:./bin/elasticsearch-provider"));
  assert.equal(
    fs.readFileSync(path.join(workdir, "unpacked/bin/elasticsearch-provider"), "utf8"),
    "fake elasticsearch provider\n",
  );
  assert.match(
    fs.readFileSync(path.join(workdir, "unpacked/ui/explorer.js"), "utf8"),
    /ElasticsearchExplorer/,
  );
  execFileSync(
    "bash",
    [path.join(workdir, "scripts/verify-composite-package.sh"), archivePath],
    { cwd: workdir, encoding: "utf8" },
  );
});

test("Elasticsearch shell consumes the host-opened connection resource", () => {
  const source = fs.readFileSync(
    path.join(repoRoot, "extensions/composite/elasticsearch/ui/explorer.js"),
    "utf8",
  );

  assert.match(source, /current\(\)/);
  assert.match(source, /connection\.resource\.handle/);
  assert.doesNotMatch(source, /await open\(/);
  assert.doesNotMatch(source, /credentialRefs/);
});

test("package-composite-extension rewrites native Windows commands and permissions", () => {
  const workdir = makeTempDir();
  createNativeCompositeFixture(workdir, { target: "x86_64-pc-windows-msvc" });

  const archivePath = execFileSync(
    "bash",
    [
      path.join(workdir, "scripts/package-composite-extension.sh"),
      "elasticsearch",
      "x86_64-pc-windows-msvc",
      path.join(workdir, "artifacts"),
      "1.2.3",
    ],
    { cwd: workdir, encoding: "utf8" },
  ).trim();

  execFileSync("tar", ["xzf", archivePath, "-C", path.join(workdir, "unpacked")]);
  const manifest = JSON.parse(
    fs.readFileSync(path.join(workdir, "unpacked/extension.json"), "utf8"),
  );
  assert.equal(manifest.runtime.ipc[0].entry.command, "./bin/elasticsearch-provider.exe");
  assert.ok(manifest.permissions.includes("spawn:./bin/elasticsearch-provider.exe"));
  assert.ok(!manifest.permissions.includes("spawn:./bin/elasticsearch-provider"));
  assert.ok(fs.existsSync(path.join(workdir, "unpacked/bin/elasticsearch-provider.exe")));
});

test("package-language-extension creates a Tree-sitter language package", () => {
  const workdir = makeTempDir();
  createLanguageExtensionFixture(workdir, {
    id: "rust",
    version: "0.0.0",
    fileExtensions: ["rs"],
  });

  const archivePath = execFileSync(
    "bash",
    [
      path.join(workdir, "scripts/package-language-extension.sh"),
      "rust",
      "universal",
      path.join(workdir, "artifacts"),
      "1.2.3",
    ],
    { cwd: workdir, encoding: "utf8" },
  ).trim();

  assert.equal(path.basename(archivePath), "rust-language-universal.tar.gz");
  execFileSync("tar", ["xzf", archivePath, "-C", path.join(workdir, "unpacked")]);

  const manifest = JSON.parse(
    fs.readFileSync(path.join(workdir, "unpacked/manifest.json"), "utf8"),
  );
  assert.equal(manifest.name, "rust");
  assert.equal(manifest.version, "1.2.3");
  assert.deepEqual(manifest.file_extensions, ["rs"]);
  assert.equal(
    fs.readFileSync(path.join(workdir, "unpacked/parser.wasm"), "utf8"),
    "fake parser wasm\n",
  );
  assert.equal(
    fs.readFileSync(path.join(workdir, "unpacked/highlights.scm"), "utf8"),
    "(identifier) @variable\n",
  );

  const output = execFileSync(
    "bash",
    [path.join(workdir, "scripts/verify-language-package.sh"), archivePath],
    { cwd: workdir, encoding: "utf8" },
  );
  assert.match(output, /Verified/);
});

test("package-language-bundle-extension creates a Tree-sitter language bundle package", () => {
  const workdir = makeTempDir();
  createLanguageBundleFixture(workdir, {
    id: "tree-sitter-languages",
    version: "0.1.0",
    languages: [
      { id: "rust", version: "0.24.0", fileExtensions: ["rs"] },
      { id: "javascript", version: "0.23.1", fileExtensions: ["js", "mjs"] },
    ],
  });

  const archivePath = execFileSync(
    "bash",
    [
      path.join(workdir, "scripts/package-language-bundle-extension.sh"),
      "tree-sitter-languages",
      "universal",
      path.join(workdir, "artifacts"),
      "0.1.0",
    ],
    { cwd: workdir, encoding: "utf8" },
  ).trim();

  assert.equal(
    path.basename(archivePath),
    "tree-sitter-languages-language-bundle-universal.tar.gz",
  );
  execFileSync("tar", ["xzf", archivePath, "-C", path.join(workdir, "unpacked")]);

  const bundleManifest = JSON.parse(
    fs.readFileSync(path.join(workdir, "unpacked/manifest.json"), "utf8"),
  );
  assert.equal(bundleManifest.id, "tree-sitter-languages");
  assert.equal(bundleManifest.version, "0.1.0");
  assert.deepEqual(bundleManifest.languages, ["javascript", "rust"]);

  const rustManifest = JSON.parse(
    fs.readFileSync(path.join(workdir, "unpacked/rust/manifest.json"), "utf8"),
  );
  assert.equal(rustManifest.name, "rust");
  assert.equal(rustManifest.version, "0.24.0");
  assert.deepEqual(rustManifest.file_extensions, ["rs"]);
  assert.equal(
    fs.readFileSync(path.join(workdir, "unpacked/rust/parser.wasm"), "utf8"),
    "fake rust parser wasm\n",
  );
  assert.equal(
    fs.readFileSync(path.join(workdir, "unpacked/javascript/parser.wasm"), "utf8"),
    "fake javascript parser wasm\n",
  );

  const output = execFileSync(
    "bash",
    [path.join(workdir, "scripts/verify-language-bundle-package.sh"), archivePath],
    { cwd: workdir, encoding: "utf8" },
  );
  assert.match(output, /Verified language bundle/);
});

test("verify-language-bundle-package rejects empty bundles", () => {
  const workdir = makeTempDir();
  copyScript("verify-language-bundle-package.sh", workdir);
  const archivePath = path.join(workdir, "empty-language-bundle.tar.gz");
  writeJson(path.join(workdir, "bundle-root/manifest.json"), {
    id: "tree-sitter-languages",
    name: "Tree-sitter Languages",
    version: "0.1.0",
    languages: [],
  });
  execFileSync("tar", ["czf", archivePath, "-C", path.join(workdir, "bundle-root"), "."]);

  assert.throws(
    () => execFileSync(
      "bash",
      [path.join(workdir, "scripts/verify-language-bundle-package.sh"), archivePath],
      { cwd: workdir, encoding: "utf8", stdio: "pipe" },
    ),
    /language bundle must contain at least one language/,
  );
});

test("package-acp-agent selects a cmd launcher for Windows x86 packages", () => {
  const workdir = makeTempDir();
  createAcpAgentFixture(workdir, {
    id: "claude-acp",
    binary: "claude-agent-acp",
    packageName: "@agentclientprotocol/claude-agent-acp@0.52.0",
  });

  const archivePath = execFileSync(
    "bash",
    [
      path.join(workdir, "scripts/package-acp-agent.sh"),
      "claude-acp",
      "i686-pc-windows-msvc",
      path.join(workdir, "artifacts"),
      "1.2.3",
    ],
    { cwd: workdir, encoding: "utf8" },
  ).trim();

  assert.equal(path.basename(archivePath), "claude-acp-acp-agent-i686-pc-windows-msvc.tar.gz");
  execFileSync("tar", ["xzf", archivePath, "-C", path.join(workdir, "unpacked")]);

  const manifest = JSON.parse(
    fs.readFileSync(path.join(workdir, "unpacked/acp_agent.json"), "utf8"),
  );
  assert.equal(manifest.agents[0].transport.command, "bin/claude-agent-acp.cmd");
  assert.equal(
    fs.readFileSync(path.join(workdir, "unpacked/bin/claude-agent-acp.cmd"), "utf8"),
    "@echo off\r\nnpm exec --yes -- @agentclientprotocol/claude-agent-acp@0.52.0 %*\r\n",
  );
  const output = execFileSync(
    "bash",
    [path.join(workdir, "scripts/verify-acp-agent-package.sh"), archivePath],
    { cwd: workdir, encoding: "utf8" },
  );
  assert.match(output, /Verified ACP agent package/);
});

test("package-remote-desktop-provider finds manifest-path helper target output", () => {
  const workdir = makeTempDir();
  createRemoteDesktopProviderFixture(workdir, {
    manifestPath: "extensions/remote-desktop/rdp-helper/Cargo.toml",
    targetRoot: "extensions/remote-desktop/rdp-helper/target",
  });

  const archivePath = execFileSync(
    "bash",
    [
      path.join(workdir, "scripts/package-remote-desktop-provider.sh"),
      "rdp",
      "x86_64-unknown-linux-gnu",
      path.join(workdir, "artifacts"),
      "1.2.3",
    ],
    { cwd: workdir, encoding: "utf8" },
  ).trim();

  execFileSync("tar", ["xzf", archivePath, "-C", path.join(workdir, "unpacked")]);
  assert.equal(
    fs.readFileSync(path.join(workdir, "unpacked/navop-rdp-helper"), "utf8"),
    "fake rdp helper\n",
  );
});

test("package-remote-desktop-provider finds helper output in CARGO_TARGET_DIR", () => {
  const workdir = makeTempDir();
  const targetDir = path.join(workdir, "short-target");
  createRemoteDesktopProviderFixture(workdir, {
    manifestPath: "extensions/remote-desktop/rdp-helper/Cargo.toml",
    targetRoot: "short-target",
  });

  const archivePath = execFileSync(
    "bash",
    [
      path.join(workdir, "scripts/package-remote-desktop-provider.sh"),
      "rdp",
      "x86_64-unknown-linux-gnu",
      path.join(workdir, "artifacts"),
      "1.2.3",
    ],
    {
      cwd: workdir,
      encoding: "utf8",
      env: { ...process.env, CARGO_TARGET_DIR: targetDir },
    },
  ).trim();

  execFileSync("tar", ["xzf", archivePath, "-C", path.join(workdir, "unpacked")]);
  assert.equal(
    fs.readFileSync(path.join(workdir, "unpacked/navop-rdp-helper"), "utf8"),
    "fake rdp helper\n",
  );
});

test("package-driver includes declared icon resources", () => {
  const workdir = makeTempDir();
  createPackageFixture(workdir, {
    driverJson: {
      id: "duckdb",
      version: "0.0.0",
      entry: {},
      ui: {
        icon: "icons/duckdb.svg",
        icon_color: "icons/duckdb-color.svg",
      },
    },
    icons: {
      "duckdb.svg": "<svg>mono</svg>\n",
      "duckdb-color.svg": "<svg>color</svg>\n",
    },
  });

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

  execFileSync("tar", ["xzf", archivePath, "-C", path.join(workdir, "unpacked")]);

  assert.equal(
    fs.readFileSync(path.join(workdir, "unpacked/icons/duckdb.svg"), "utf8"),
    "<svg>mono</svg>\n",
  );
  assert.equal(
    fs.readFileSync(path.join(workdir, "unpacked/icons/duckdb-color.svg"), "utf8"),
    "<svg>color</svg>\n",
  );
  execFileSync("bash", [path.join(workdir, "scripts/verify-package.sh"), archivePath], {
    cwd: workdir,
    encoding: "utf8",
  });
});

test("package-driver only includes release lib directory for Java drivers", () => {
  const workdir = makeTempDir();
  createPackageFixture(workdir);
  fs.mkdirSync(path.join(workdir, "target/x86_64-unknown-linux-gnu/release/lib"), {
    recursive: true,
  });
  fs.writeFileSync(
    path.join(workdir, "target/x86_64-unknown-linux-gnu/release/lib/gbase8s-ipc-driver.jar"),
    "java jar\n",
  );

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

  execFileSync("tar", ["xzf", archivePath, "-C", path.join(workdir, "unpacked")]);
  assert.equal(fs.existsSync(path.join(workdir, "unpacked/lib")), false);
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
    fs.readFileSync(path.join(workdir, "unpacked/duckdb.dll"), "utf8"),
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
    fs.readFileSync(path.join(workdir, "unpacked/driver.json"), "utf8"),
  );
  assert.equal(driverJson.entry.command, "./dm-ipc-driver");
  assert.equal(
    fs.readFileSync(path.join(workdir, "unpacked/dm-ipc-driver"), "utf8"),
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
      path.join(workdir, "unpacked/lib/gbase8s-ipc-driver.jar"),
      "utf8",
    ),
    "fake jar\n",
  );
  const driverJson = JSON.parse(
    fs.readFileSync(path.join(workdir, "unpacked/driver.json"), "utf8"),
  );
  assert.equal(driverJson.entry.command, "./gbase8s-ipc-driver");
});

test("package-driver uses a cmd launcher for Java IPC drivers on Windows", () => {
  const workdir = makeTempDir();
  createPackageFixture(workdir, {
    id: "gbase8s",
    binary: "gbase8s-ipc-driver",
    language: "java",
    package: "java/gbase8s-ipc-driver",
  });
  fs.mkdirSync(path.join(workdir, "target/x86_64-pc-windows-msvc/release/lib"), {
    recursive: true,
  });
  fs.writeFileSync(
    path.join(workdir, "target/x86_64-pc-windows-msvc/release/gbase8s-ipc-driver.cmd"),
    "@echo off\r\n",
  );
  fs.writeFileSync(
    path.join(workdir, "target/x86_64-pc-windows-msvc/release/lib/gbase8s-ipc-driver.jar"),
    "fake jar\n",
  );

  const archivePath = execFileSync(
    "bash",
    [
      path.join(workdir, "scripts/package-driver.sh"),
      "gbase8s",
      "x86_64-pc-windows-msvc",
      path.join(workdir, "artifacts"),
      "0.1.0",
    ],
    { cwd: workdir, encoding: "utf8" },
  ).trim();

  execFileSync("tar", ["xzf", archivePath, "-C", path.join(workdir, "unpacked")]);
  assert.equal(
    fs.readFileSync(path.join(workdir, "unpacked/gbase8s-ipc-driver.cmd"), "utf8"),
    "@echo off\r\n",
  );
  const driverJson = JSON.parse(
    fs.readFileSync(path.join(workdir, "unpacked/driver.json"), "utf8"),
  );
  assert.equal(driverJson.entry.command, "./gbase8s-ipc-driver.cmd");
});

test("package-driver includes both Java launchers for universal packages", () => {
  const workdir = makeTempDir();
  createPackageFixture(workdir, {
    id: "gbase8s",
    binary: "gbase8s-ipc-driver",
    binaryContents: "#!/usr/bin/env sh\n",
    language: "java",
    package: "java/gbase8s-ipc-driver",
  });
  fs.mkdirSync(path.join(workdir, "target/universal/release/lib"), {
    recursive: true,
  });
  fs.writeFileSync(
    path.join(workdir, "target/universal/release/gbase8s-ipc-driver"),
    "#!/usr/bin/env sh\n",
  );
  fs.writeFileSync(
    path.join(workdir, "target/universal/release/gbase8s-ipc-driver.cmd"),
    "@echo off\r\n",
  );
  fs.writeFileSync(
    path.join(workdir, "target/universal/release/lib/gbase8s-ipc-driver.jar"),
    "fake jar\n",
  );

  const archivePath = execFileSync(
    "bash",
    [
      path.join(workdir, "scripts/package-driver.sh"),
      "gbase8s",
      "universal",
      path.join(workdir, "artifacts"),
      "0.1.0",
    ],
    { cwd: workdir, encoding: "utf8" },
  ).trim();

  assert.equal(path.basename(archivePath), "gbase8s-driver-universal.tar.gz");
  execFileSync("tar", ["xzf", archivePath, "-C", path.join(workdir, "unpacked")]);
  assert.equal(
    fs.readFileSync(path.join(workdir, "unpacked/gbase8s-ipc-driver"), "utf8"),
    "#!/usr/bin/env sh\n",
  );
  assert.equal(
    fs.readFileSync(path.join(workdir, "unpacked/gbase8s-ipc-driver.cmd"), "utf8"),
    "@echo off\r\n",
  );
  const driverJson = JSON.parse(
    fs.readFileSync(path.join(workdir, "unpacked/driver.json"), "utf8"),
  );
  assert.equal(driverJson.entry.command, "./gbase8s-ipc-driver");
  assert.equal(driverJson.entry.commands.default, "./gbase8s-ipc-driver");
  assert.equal(driverJson.entry.commands.windows, "./gbase8s-ipc-driver.cmd");
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
  fs.mkdirSync(path.join(workdir, "java/gbase8s-ipc-driver/bin/lib"), { recursive: true });
  fs.writeFileSync(
    path.join(workdir, "java/gbase8s-ipc-driver/target/gbase8s-ipc-driver-0.1.0-all.jar"),
    "fake shaded jar\n",
  );
  fs.writeFileSync(
    path.join(workdir, "java/gbase8s-ipc-driver/bin/gbase8s-ipc-driver"),
    "#!/usr/bin/env sh\n",
  );
  fs.writeFileSync(
    path.join(workdir, "java/gbase8s-ipc-driver/bin/lib/gbasedbtjdbc.jar"),
    "fake gbase jdbc jar\n",
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
  assert.equal(
    fs.readFileSync(
      path.join(workdir, "target/x86_64-unknown-linux-gnu/release/lib/gbasedbtjdbc.jar"),
      "utf8",
    ),
    "fake gbase jdbc jar\n",
  );
  assert.ok(
    fs.existsSync(path.join(workdir, "target/x86_64-unknown-linux-gnu/release/gbase8s-ipc-driver")),
  );
});

test("build-java-driver rebuilds stale shaded jars before staging", () => {
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
    targets: ["universal"],
  });
  const projectDir = path.join(workdir, "java/gbase8s-ipc-driver");
  fs.mkdirSync(path.join(projectDir, "target"), { recursive: true });
  fs.mkdirSync(path.join(projectDir, "bin"), { recursive: true });
  fs.writeFileSync(path.join(projectDir, "pom.xml"), "<project />\n");
  fs.writeFileSync(
    path.join(projectDir, "target/gbase8s-ipc-driver-0.1.0-all.jar"),
    "stale shaded jar\n",
  );
  fs.writeFileSync(path.join(projectDir, "bin/gbase8s-ipc-driver"), "#!/usr/bin/env sh\n");
  fs.writeFileSync(path.join(projectDir, "bin/gbase8s-ipc-driver.cmd"), "@echo off\r\n");

  const binDir = path.join(workdir, "fake-bin");
  fs.mkdirSync(binDir, { recursive: true });
  const mvnPath = path.join(binDir, "mvn");
  fs.writeFileSync(
    mvnPath,
    [
      "#!/usr/bin/env sh",
      "set -eu",
      "project=''",
      "while [ \"$#\" -gt 0 ]; do",
      "  if [ \"$1\" = '-f' ]; then",
      "    project=\"$2\"",
      "    shift 2",
      "  else",
      "    shift",
      "  fi",
      "done",
      "target_dir=\"$(dirname \"$project\")/target\"",
      "mkdir -p \"$target_dir\"",
      "printf 'fresh shaded jar\\n' > \"$target_dir/gbase8s-ipc-driver-0.1.2-all.jar\"",
      "",
    ].join("\n"),
  );
  fs.chmodSync(mvnPath, 0o755);

  execFileSync(
    "bash",
    [path.join(workdir, "scripts/build-java-driver.sh"), "gbase8s", "universal"],
    {
      cwd: workdir,
      env: {
        ...process.env,
        PATH: `${binDir}${path.delimiter}${process.env.PATH}`,
      },
    },
  );

  assert.equal(
    fs.readFileSync(
      path.join(workdir, "target/universal/release/lib/gbase8s-ipc-driver.jar"),
      "utf8",
    ),
    "fresh shaded jar\n",
  );
});

test("build-java-driver stages cmd launcher for Windows targets", () => {
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
    targets: ["x86_64-pc-windows-msvc"],
  });
  fs.mkdirSync(path.join(workdir, "java/gbase8s-ipc-driver/target"), { recursive: true });
  fs.mkdirSync(path.join(workdir, "java/gbase8s-ipc-driver/bin"), { recursive: true });
  fs.writeFileSync(
    path.join(workdir, "java/gbase8s-ipc-driver/target/gbase8s-ipc-driver-0.1.0-all.jar"),
    "fake shaded jar\n",
  );
  fs.writeFileSync(
    path.join(workdir, "java/gbase8s-ipc-driver/bin/gbase8s-ipc-driver.cmd"),
    "@echo off\r\n",
  );

  execFileSync(
    "bash",
    [
      path.join(workdir, "scripts/build-java-driver.sh"),
      "gbase8s",
      "x86_64-pc-windows-msvc",
    ],
    { cwd: workdir },
  );

  assert.equal(
    fs.readFileSync(
      path.join(workdir, "target/x86_64-pc-windows-msvc/release/lib/gbase8s-ipc-driver.jar"),
      "utf8",
    ),
    "fake shaded jar\n",
  );
  assert.equal(
    fs.readFileSync(
      path.join(workdir, "target/x86_64-pc-windows-msvc/release/gbase8s-ipc-driver.cmd"),
      "utf8",
    ),
    "@echo off\r\n",
  );
});

test("build-java-driver stages both launchers for universal targets", () => {
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
    targets: ["universal"],
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
  fs.writeFileSync(
    path.join(workdir, "java/gbase8s-ipc-driver/bin/gbase8s-ipc-driver.cmd"),
    "@echo off\r\n",
  );

  execFileSync(
    "bash",
    [
      path.join(workdir, "scripts/build-java-driver.sh"),
      "gbase8s",
      "universal",
    ],
    { cwd: workdir },
  );

  assert.ok(
    fs.existsSync(path.join(workdir, "target/universal/release/gbase8s-ipc-driver")),
  );
  assert.ok(
    fs.existsSync(path.join(workdir, "target/universal/release/gbase8s-ipc-driver.cmd")),
  );
  assert.equal(
    fs.readFileSync(
      path.join(workdir, "target/universal/release/lib/gbase8s-ipc-driver.jar"),
      "utf8",
    ),
    "fake shaded jar\n",
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

test("build-go-driver cross-compiles Windows x86 executables", () => {
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
    targets: ["i686-pc-windows-msvc"],
  });

  execFileSync(
    "bash",
    [
      path.join(workdir, "scripts/build-go-driver.sh"),
      "testdb",
      "i686-pc-windows-msvc",
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
    fs.existsSync(
      path.join(workdir, "target/i686-pc-windows-msvc/release/test-ipc-driver.exe"),
    ),
  );
});

test("build-go-driver prefers vendored Go driver dependencies", () => {
  const workdir = makeTempDir();
  copyScript("build-go-driver.sh", workdir);
  fs.writeFileSync(
    path.join(workdir, "go.mod"),
    [
      "module example.com/go-driver-fixture",
      "",
      "go 1.23",
      "",
      "require gitee.com/chunanyong/dm v1.8.23",
      "",
    ].join("\n"),
  );
  fs.mkdirSync(path.join(workdir, "cmd/dm-ipc-driver"), { recursive: true });
  fs.writeFileSync(
    path.join(workdir, "cmd/dm-ipc-driver/main.go"),
    [
      "package main",
      "",
      'import _ "gitee.com/chunanyong/dm"',
      "",
      "func main() {}",
      "",
    ].join("\n"),
  );
  fs.mkdirSync(path.join(workdir, "vendor/gitee.com/chunanyong/dm"), { recursive: true });
  fs.writeFileSync(
    path.join(workdir, "vendor/gitee.com/chunanyong/dm/dm.go"),
    "package dm\n",
  );
  fs.writeFileSync(
    path.join(workdir, "vendor/modules.txt"),
    [
      "# gitee.com/chunanyong/dm v1.8.23",
      "## explicit; go 1.23",
      "gitee.com/chunanyong/dm",
      "",
    ].join("\n"),
  );
  writeJson(path.join(workdir, "extensions/ipc/dm/extension.build.json"), {
    id: "dm",
    kind: "database_driver",
    language: "go",
    package: "./cmd/dm-ipc-driver",
    binary: "dm-ipc-driver",
    path: "extensions/ipc/dm",
    targets: ["x86_64-unknown-linux-gnu"],
  });

  execFileSync(
    "bash",
    [
      path.join(workdir, "scripts/build-go-driver.sh"),
      "dm",
      "x86_64-unknown-linux-gnu",
    ],
    {
      cwd: workdir,
      env: {
        ...process.env,
        DM_DRIVER_PATH: path.join(workdir, "missing-dm-driver"),
        GOCACHE: path.join(workdir, "go-cache"),
        CGO_ENABLED: "0",
      },
    },
  );

  assert.ok(
    fs.existsSync(path.join(workdir, "target/x86_64-unknown-linux-gnu/release/dm-ipc-driver")),
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
    targets: [
      "x86_64-unknown-linux-gnu",
      "x86_64-pc-windows-msvc",
      "i686-pc-windows-msvc",
    ],
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
        manifest_path: "",
        kind: "database_driver",
        language: "rust",
        os: "ubuntu-latest",
        windows_x86: true,
      },
    ],
  });
});

test("changed-extensions emits one Ubuntu test entry for Go extensions", () => {
  const workdir = makeTempDir();
  copyScript("changed-extensions.mjs", workdir);
  writeJson(path.join(workdir, "extensions/ipc/dm/extension.build.json"), {
    id: "dm",
    kind: "database_driver",
    language: "go",
    package: "./cmd/dm-ipc-driver",
    path: "extensions/ipc/dm",
    targets: [
      "x86_64-apple-darwin",
      "aarch64-apple-darwin",
      "x86_64-unknown-linux-gnu",
      "aarch64-unknown-linux-gnu",
      "x86_64-pc-windows-msvc",
      "i686-pc-windows-msvc",
    ],
  });
  fs.writeFileSync(path.join(workdir, "README.md"), "base\n");
  git(workdir, "init");
  git(workdir, "add", ".");
  git(workdir, "commit", "-m", "base");
  const base = git(workdir, "rev-parse", "HEAD").trim();
  fs.mkdirSync(path.join(workdir, "extensions/ipc/dm"), { recursive: true });
  fs.writeFileSync(path.join(workdir, "extensions/ipc/dm/driver.json"), "{}\n");
  git(workdir, "add", ".");
  git(workdir, "commit", "-m", "change dm");
  const head = git(workdir, "rev-parse", "HEAD").trim();

  const output = execFileSync(
    "node",
    [path.join(workdir, "scripts/changed-extensions.mjs"), base, head],
    { cwd: workdir, encoding: "utf8" },
  );

  const matrix = JSON.parse(output);
  assert.deepEqual(matrix.include, [
    {
      extension: "dm",
      package: "./cmd/dm-ipc-driver",
      manifest_path: "",
      kind: "database_driver",
      language: "go",
      os: "ubuntu-latest",
      windows_x86: true,
    },
  ]);
});

test("changed-extensions includes Rust WASM extensions for workspace Rust changes", () => {
  const workdir = makeTempDir();
  copyScript("changed-extensions.mjs", workdir);
  writeJson(path.join(workdir, "extensions/ipc/duckdb/extension.build.json"), {
    id: "duckdb",
    kind: "database_driver",
    language: "rust",
    package: "duckdb_driver",
    path: "extensions/ipc/duckdb",
    targets: ["x86_64-unknown-linux-gnu"],
  });
  writeJson(path.join(workdir, "extensions/wasm/dbeaver-importer/extension.build.json"), {
    id: "dbeaver-importer",
    kind: "composite",
    language: "rust-wasm",
    package: "dbeaver_importer_wasm",
    path: "extensions/wasm/dbeaver-importer",
    targets: ["universal"],
  });
  fs.writeFileSync(path.join(workdir, "Cargo.toml"), "[workspace]\n");
  git(workdir, "init");
  git(workdir, "add", ".");
  git(workdir, "commit", "-m", "base");
  const base = git(workdir, "rev-parse", "HEAD").trim();
  fs.writeFileSync(path.join(workdir, "Cargo.toml"), "[workspace]\nresolver = \"2\"\n");
  git(workdir, "add", ".");
  git(workdir, "commit", "-m", "change workspace");
  const head = git(workdir, "rev-parse", "HEAD").trim();

  const output = execFileSync(
    "node",
    [path.join(workdir, "scripts/changed-extensions.mjs"), base, head],
    { cwd: workdir, encoding: "utf8" },
  );

  assert.deepEqual(JSON.parse(output).include, [
    {
      extension: "duckdb",
      package: "duckdb_driver",
      manifest_path: "",
      kind: "database_driver",
      language: "rust",
      os: "ubuntu-latest",
      windows_x86: false,
    },
    {
      extension: "dbeaver-importer",
      package: "dbeaver_importer_wasm",
      manifest_path: "",
      kind: "composite",
      language: "rust-wasm",
      os: "ubuntu-latest",
      windows_x86: false,
    },
  ]);
});

test("changed-extensions maps declared source paths to the owning extension", () => {
  const workdir = makeTempDir();
  copyScript("changed-extensions.mjs", workdir);
  writeJson(path.join(workdir, "extensions/ipc/gbase8s/extension.build.json"), {
    id: "gbase8s",
    kind: "database_driver",
    language: "java",
    package: "java/gbase8s-ipc-driver",
    binary: "gbase8s-ipc-driver",
    path: "extensions/ipc/gbase8s",
    source_paths: ["java/gbase8s-ipc-driver"],
    targets: ["universal"],
  });
  writeJson(path.join(workdir, "extensions/ipc/duckdb/extension.build.json"), {
    id: "duckdb",
    kind: "database_driver",
    package: "duckdb_driver",
    path: "extensions/ipc/duckdb",
    targets: ["x86_64-unknown-linux-gnu"],
  });
  fs.mkdirSync(path.join(workdir, "java/gbase8s-ipc-driver/src"), { recursive: true });
  fs.writeFileSync(path.join(workdir, "java/gbase8s-ipc-driver/src/Main.java"), "class Main {}\n");
  git(workdir, "init");
  git(workdir, "add", ".");
  git(workdir, "commit", "-m", "base");
  const base = git(workdir, "rev-parse", "HEAD").trim();
  fs.writeFileSync(path.join(workdir, "java/gbase8s-ipc-driver/src/Main.java"), "class Main2 {}\n");
  git(workdir, "add", ".");
  git(workdir, "commit", "-m", "change gbase java");
  const head = git(workdir, "rev-parse", "HEAD").trim();

  const output = execFileSync(
    "node",
    [path.join(workdir, "scripts/changed-extensions.mjs"), base, head],
    { cwd: workdir, encoding: "utf8" },
  );

  assert.deepEqual(JSON.parse(output).include, [
    {
      extension: "gbase8s",
      package: "java/gbase8s-ipc-driver",
      manifest_path: "",
      kind: "database_driver",
      language: "java",
      os: "ubuntu-latest",
      windows_x86: false,
    },
  ]);
});

test("changed-extensions emits manifest-path metadata for standalone Rust helpers", () => {
  const workdir = makeTempDir();
  copyScript("changed-extensions.mjs", workdir);
  writeJson(path.join(workdir, "extensions/remote-desktop/rdp/extension.build.json"), {
    id: "rdp",
    kind: "remote_desktop_provider",
    package: "navop-rdp-helper",
    binary: "navop-rdp-helper",
    manifest_path: "extensions/remote-desktop/rdp-helper/Cargo.toml",
    path: "extensions/remote-desktop/rdp",
    source_paths: ["extensions/remote-desktop/rdp-helper"],
    targets: ["x86_64-unknown-linux-gnu", "i686-pc-windows-msvc"],
  });
  fs.mkdirSync(path.join(workdir, "extensions/remote-desktop/rdp-helper/src"), {
    recursive: true,
  });
  fs.writeFileSync(path.join(workdir, "extensions/remote-desktop/rdp-helper/src/main.rs"), "fn main() {}\n");
  git(workdir, "init");
  git(workdir, "add", ".");
  git(workdir, "commit", "-m", "base");
  const base = git(workdir, "rev-parse", "HEAD").trim();
  fs.writeFileSync(path.join(workdir, "extensions/remote-desktop/rdp-helper/src/main.rs"), "fn main() { println!(\"rdp\"); }\n");
  git(workdir, "add", ".");
  git(workdir, "commit", "-m", "change rdp helper");
  const head = git(workdir, "rev-parse", "HEAD").trim();

  const output = execFileSync(
    "node",
    [path.join(workdir, "scripts/changed-extensions.mjs"), base, head],
    { cwd: workdir, encoding: "utf8" },
  );

  assert.deepEqual(JSON.parse(output).include, [
    {
      extension: "rdp",
      package: "navop-rdp-helper",
      manifest_path: "extensions/remote-desktop/rdp-helper/Cargo.toml",
      kind: "remote_desktop_provider",
      language: "rust",
      os: "ubuntu-latest",
      windows_x86: true,
    },
  ]);
});

test("changed-extensions emits manifest-path metadata for MCP helpers", () => {
  const workdir = makeTempDir();
  copyScript("changed-extensions.mjs", workdir);
  writeJson(path.join(workdir, "extensions/mcp-helper/onetcli-public-mcp/extension.build.json"), {
    id: "onetcli-public-mcp",
    kind: "mcp_helper",
    package: "onetcli-public-mcp",
    binary: "onetcli-public-mcp",
    manifest_path: "extensions/mcp-helper/onetcli-public-mcp/Cargo.toml",
    path: "extensions/mcp-helper/onetcli-public-mcp",
    targets: ["x86_64-unknown-linux-gnu"],
  });
  fs.mkdirSync(path.join(workdir, "extensions/mcp-helper/onetcli-public-mcp/src"), {
    recursive: true,
  });
  fs.writeFileSync(
    path.join(workdir, "extensions/mcp-helper/onetcli-public-mcp/src/main.rs"),
    "fn main() {}\n",
  );
  git(workdir, "init");
  git(workdir, "add", ".");
  git(workdir, "commit", "-m", "base");
  const base = git(workdir, "rev-parse", "HEAD").trim();
  fs.writeFileSync(
    path.join(workdir, "extensions/mcp-helper/onetcli-public-mcp/src/main.rs"),
    "fn main() { println!(\"mcp\"); }\n",
  );
  git(workdir, "add", ".");
  git(workdir, "commit", "-m", "change mcp helper");
  const head = git(workdir, "rev-parse", "HEAD").trim();

  const output = execFileSync(
    "node",
    [path.join(workdir, "scripts/changed-extensions.mjs"), base, head],
    { cwd: workdir, encoding: "utf8" },
  );

  assert.deepEqual(JSON.parse(output).include, [
    {
      extension: "onetcli-public-mcp",
      package: "onetcli-public-mcp",
      manifest_path: "extensions/mcp-helper/onetcli-public-mcp/Cargo.toml",
      kind: "mcp_helper",
      language: "rust",
      os: "ubuntu-latest",
      windows_x86: false,
    },
  ]);
});

test("changed-extensions emits shell metadata for ACP agents", () => {
  const workdir = makeTempDir();
  copyScript("changed-extensions.mjs", workdir);
  writeJson(path.join(workdir, "extensions/acp-agent/codex-acp/extension.build.json"), {
    id: "codex-acp",
    kind: "acp_agent",
    language: "shell",
    package: "",
    binary: "codex-acp",
    path: "extensions/acp-agent/codex-acp",
    targets: [
      "x86_64-unknown-linux-gnu",
      "x86_64-pc-windows-msvc",
      "i686-pc-windows-msvc",
    ],
  });
  fs.mkdirSync(path.join(workdir, "extensions/acp-agent/codex-acp/bin"), {
    recursive: true,
  });
  fs.writeFileSync(
    path.join(workdir, "extensions/acp-agent/codex-acp/bin/codex-acp"),
    "#!/usr/bin/env sh\n",
  );
  git(workdir, "init");
  git(workdir, "add", ".");
  git(workdir, "commit", "-m", "base");
  const base = git(workdir, "rev-parse", "HEAD").trim();
  fs.writeFileSync(
    path.join(workdir, "extensions/acp-agent/codex-acp/bin/codex-acp"),
    "#!/usr/bin/env sh\nexec npm exec --yes -- @agentclientprotocol/codex-acp@1.0.1 \"$@\"\n",
  );
  git(workdir, "add", ".");
  git(workdir, "commit", "-m", "change codex acp");
  const head = git(workdir, "rev-parse", "HEAD").trim();

  const output = execFileSync(
    "node",
    [path.join(workdir, "scripts/changed-extensions.mjs"), base, head],
    { cwd: workdir, encoding: "utf8" },
  );

  assert.deepEqual(JSON.parse(output).include, [
    {
      extension: "codex-acp",
      package: "",
      manifest_path: "",
      kind: "acp_agent",
      language: "shell",
      os: "ubuntu-latest",
      windows_x86: true,
    },
  ]);
});

test("changed-extensions does not expand workflow-only changes into extension tests", () => {
  const workdir = makeTempDir();
  copyScript("changed-extensions.mjs", workdir);
  writeJson(path.join(workdir, "extensions/ipc/duckdb/extension.build.json"), {
    id: "duckdb",
    kind: "database_driver",
    package: "duckdb_driver",
    path: "extensions/ipc/duckdb",
    targets: ["x86_64-unknown-linux-gnu", "x86_64-pc-windows-msvc"],
  });
  fs.mkdirSync(path.join(workdir, ".github/workflows"), { recursive: true });
  fs.writeFileSync(path.join(workdir, ".github/workflows/ci.yml"), "name: CI\n");
  git(workdir, "init");
  git(workdir, "add", ".");
  git(workdir, "commit", "-m", "base");
  const base = git(workdir, "rev-parse", "HEAD").trim();
  fs.writeFileSync(path.join(workdir, ".github/workflows/ci.yml"), "name: CI changed\n");
  git(workdir, "add", ".");
  git(workdir, "commit", "-m", "change workflow");
  const head = git(workdir, "rev-parse", "HEAD").trim();

  const output = execFileSync(
    "node",
    [path.join(workdir, "scripts/changed-extensions.mjs"), base, head],
    { cwd: workdir, encoding: "utf8" },
  );

  assert.deepEqual(JSON.parse(output), { include: [] });
});

test("repository manifest is maintained as a lightweight marketplace index", () => {
  const manifest = JSON.parse(fs.readFileSync(path.join(repoRoot, "manifest.json"), "utf8"));
  const bundleManifest = JSON.parse(
    fs.readFileSync(
      path.join(repoRoot, "extensions/language-bundle/tree-sitter-languages/manifest.json"),
      "utf8",
    ),
  );
  const bundledLanguageIds = new Set(bundleManifest.languages || []);
  const entriesById = new Map(
    extensionBuildEntries()
      .filter((entry) => !bundledLanguageIds.has(entry.id))
      .map((entry) => [entry.id, entry]),
  );

  assert.equal(manifest.schema_version, 2);
  const expectedMarketplaceIds = [...entriesById.values()].map((buildEntry) => {
    const sourceManifest = JSON.parse(
      fs.readFileSync(
        path.join(repoRoot, buildEntry.path, manifestFileForKind(buildEntry.kind)),
        "utf8",
      ),
    );
    return sourceManifest.id;
  });
  assert.deepEqual(
    manifest.extensions.map((entry) => entry.id).sort(),
    expectedMarketplaceIds.sort(),
  );

  for (const entry of manifest.extensions) {
    const artifactSlug = entry.manifest.split("/")[0];
    const buildEntry = entriesById.get(artifactSlug);
    const sourceManifest = JSON.parse(
      fs.readFileSync(path.join(repoRoot, buildEntry.path, manifestFileForKind(entry.kind)), "utf8"),
    );
    assert.equal(entry.id, sourceManifest.id);
    assert.equal(entry.kind, buildEntry.kind);
    assert.equal(entry.name, sourceManifest.name || entry.id);
    assert.equal(entry.version, sourceManifest.version);
    assert.equal(entry.release_tag, `${artifactSlug}-v${entry.version}`);
    assert.equal(entry.manifest, `${artifactSlug}/manifest.json`);
    assert.equal(Object.hasOwn(entry, "artifacts"), false);
    assert.equal(Object.hasOwn(entry, "asset_urls"), false);
    assert.equal(Object.hasOwn(entry, "fallback_asset_urls"), false);
    assert.equal(Object.hasOwn(entry, "sha256s"), false);
  }
});

test("Tree-sitter language extensions cover every non-built-in host language", () => {
  const builtInLanguageIds = new Set(["bash", "sql"]);
  const expectedLanguageIds = [
    "astro",
    "c",
    "cmake",
    "cpp",
    "csharp",
    "css",
    "diff",
    "ejs",
    "elixir",
    "erb",
    "go",
    "graphql",
    "html",
    "java",
    "javascript",
    "jsdoc",
    "kotlin",
    "lua",
    "make",
    "markdown",
    "markdown_inline",
    "php",
    "proto",
    "python",
    "ruby",
    "rust",
    "scala",
    "svelte",
    "swift",
    "toml",
    "tsx",
    "typescript",
    "yaml",
    "zig",
  ];
  assert.equal(expectedLanguageIds.some((id) => builtInLanguageIds.has(id)), false);

  const manifest = JSON.parse(fs.readFileSync(path.join(repoRoot, "manifest.json"), "utf8"));
  const globalEntries = new Map(manifest.extensions.map((entry) => [entry.id, entry]));
  const languageRoot = path.join(repoRoot, "extensions/language");
  const actualLanguageIds = fs.existsSync(languageRoot)
    ? fs.readdirSync(languageRoot).filter((id) =>
        fs.existsSync(path.join(languageRoot, id, "extension.build.json")),
      ).sort()
    : [];

  assert.deepEqual(actualLanguageIds, expectedLanguageIds);

  const bundleEntry = globalEntries.get("tree-sitter-languages");
  assert.equal(bundleEntry?.kind, "language_bundle");
  assert.equal(bundleEntry?.manifest, "tree-sitter-languages/manifest.json");

  const bundledFileExtensions = new Set();
  for (const id of expectedLanguageIds) {
    const metadata = JSON.parse(
      fs.readFileSync(path.join(languageRoot, id, "extension.build.json"), "utf8"),
    );
    assert.equal(metadata.id, id);
    assert.equal(metadata.kind, "language");
    assert.equal(metadata.language, "tree-sitter-wasm");
    assert.equal(metadata.path, `extensions/language/${id}`);
    assert.deepEqual(metadata.targets, ["universal"]);
    assert.equal(metadata.releaseTagPrefix, `${id}-v`);
    assert.equal(metadata.r2Prefix, `extensions/${id}`);

    const sourceManifest = JSON.parse(
      fs.readFileSync(path.join(languageRoot, id, "manifest.json"), "utf8"),
    );
    assert.equal(sourceManifest.name, id);
    assert.equal(typeof sourceManifest.version, "string");
    assert.ok(sourceManifest.version.length > 0, `${id} manifest version should not be empty`);
    assert.ok(Array.isArray(sourceManifest.file_extensions), `${id} file_extensions should be an array`);
    for (const extension of sourceManifest.file_extensions) {
      bundledFileExtensions.add(extension);
    }
    assert.ok(
      fs.existsSync(path.join(languageRoot, id, "parser.wasm")),
      `${id} should include parser.wasm`,
    );

    assert.equal(globalEntries.has(id), false, `${id} should be represented by the bundle entry`);
  }
  assert.deepEqual(
    bundleEntry?.file_extensions,
    [...bundledFileExtensions].sort(),
  );
});

test("generate-marketplace-manifest writes only the current plugin manifest", () => {
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
    engines: { onetcli: ">=0.10.0" },
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
    },
  });

  const extensionManifest = JSON.parse(
    fs.readFileSync(path.join(workdir, "artifacts/extension-manifest.json"), "utf8"),
  );
  assert.equal(extensionManifest.schema_version, 2);
  assert.equal(extensionManifest.release_version, "duckdb-v1.2.3");
  assert.equal(extensionManifest.extensions.length, 1);
  assert.equal(extensionManifest.extensions[0].release_tag, "duckdb-v1.2.3");
  assert.deepEqual(extensionManifest.extensions[0].engines, {
    onetcli: ">=0.10.0",
  });
  assert.equal(
    extensionManifest.extensions[0].artifacts["x86_64-unknown-linux-gnu"].file,
    "duckdb-driver-x86_64-unknown-linux-gnu.tar.gz",
  );
  assert.match(
    extensionManifest.extensions[0].artifacts["x86_64-unknown-linux-gnu"].sha256,
    /^[0-9a-f]{64}$/,
  );
  assert.equal(fs.existsSync(path.join(workdir, "artifacts/marketplace-manifest.json")), false);
  assert.equal(fs.existsSync(path.join(workdir, "manifest/entries/duckdb.json")), false);
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
    },
  });

  const extensionManifest = JSON.parse(
    fs.readFileSync(path.join(workdir, "artifacts/extension-manifest.json"), "utf8"),
  );
  assert.equal(extensionManifest.extensions[0].id, "iotdb");
  assert.equal(extensionManifest.extensions[0].name, "Apache IoTDB");
  assert.equal(
    extensionManifest.extensions[0].artifacts["x86_64-unknown-linux-gnu"].file,
    "iotdb-driver-x86_64-unknown-linux-gnu.tar.gz",
  );
});

test("generate-marketplace-manifest supports remote desktop providers", () => {
  const workdir = makeTempDir();
  copyScript("generate-marketplace-manifest.mjs", workdir);
  fs.mkdirSync(path.join(workdir, "artifacts"), { recursive: true });
  writeJson(path.join(workdir, "extensions/remote-desktop/rdp/extension.build.json"), {
    id: "rdp",
    kind: "remote_desktop_provider",
    path: "extensions/remote-desktop/rdp",
    targets: ["x86_64-unknown-linux-gnu"],
  });
  writeJson(path.join(workdir, "extensions/remote-desktop/rdp/remote_desktop_provider.json"), {
    id: "rdp",
    name: "RDP",
    description: "RDP remote desktop provider",
  });
  const fileName = "rdp-remote-desktop-provider-x86_64-unknown-linux-gnu.tar.gz";
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
      EXTENSION_ID: "rdp",
      RELEASE_TAG: "rdp-v0.1.0",
    },
  });

  const extensionManifest = JSON.parse(
    fs.readFileSync(path.join(workdir, "artifacts/extension-manifest.json"), "utf8"),
  );
  assert.equal(extensionManifest.extensions[0].id, "rdp");
  assert.equal(extensionManifest.extensions[0].kind, "remote_desktop_provider");
  assert.equal(
    extensionManifest.extensions[0].artifacts["x86_64-unknown-linux-gnu"].file,
    fileName,
  );
});

test("generate-marketplace-manifest supports MCP helpers", () => {
  const workdir = makeTempDir();
  copyScript("generate-marketplace-manifest.mjs", workdir);
  fs.mkdirSync(path.join(workdir, "artifacts"), { recursive: true });
  writeJson(path.join(workdir, "extensions/mcp-helper/onetcli-public-mcp/extension.build.json"), {
    id: "onetcli-public-mcp",
    kind: "mcp_helper",
    path: "extensions/mcp-helper/onetcli-public-mcp",
    targets: ["x86_64-unknown-linux-gnu"],
  });
  writeJson(path.join(workdir, "extensions/mcp-helper/onetcli-public-mcp/mcp_helper.json"), {
    id: "onetcli-public-mcp",
    name: "Navop Public MCP Helper",
    description: "Public MCP stdio bridge",
  });
  const fileName = "onetcli-public-mcp-mcp-helper-x86_64-unknown-linux-gnu.tar.gz";
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
      EXTENSION_ID: "onetcli-public-mcp",
      RELEASE_TAG: "onetcli-public-mcp-v0.1.0",
    },
  });

  const extensionManifest = JSON.parse(
    fs.readFileSync(path.join(workdir, "artifacts/extension-manifest.json"), "utf8"),
  );
  assert.equal(extensionManifest.extensions[0].id, "onetcli-public-mcp");
  assert.equal(extensionManifest.extensions[0].kind, "mcp_helper");
  assert.equal(
    extensionManifest.extensions[0].artifacts["x86_64-unknown-linux-gnu"].file,
    fileName,
  );
});

test("generate-marketplace-manifest supports ACP agents", () => {
  const workdir = makeTempDir();
  copyScript("generate-marketplace-manifest.mjs", workdir);
  fs.mkdirSync(path.join(workdir, "artifacts"), { recursive: true });
  writeJson(path.join(workdir, "extensions/acp-agent/codex-acp/extension.build.json"), {
    id: "codex-acp",
    kind: "acp_agent",
    path: "extensions/acp-agent/codex-acp",
    targets: ["x86_64-unknown-linux-gnu"],
  });
  writeJson(path.join(workdir, "extensions/acp-agent/codex-acp/acp_agent.json"), {
    id: "codex-acp",
    name: "Codex",
    description: "Shell wrapper for Codex ACP",
  });
  const fileName = "codex-acp-acp-agent-x86_64-unknown-linux-gnu.tar.gz";
  fs.writeFileSync(
    path.join(workdir, "artifacts/sha256sums.txt"),
    `${createHash("sha256").update(fileName).digest("hex")}  ${fileName}\n`,
  );

  execFileSync("node", [path.join(workdir, "scripts/generate-marketplace-manifest.mjs")], {
    cwd: workdir,
    env: {
      ...process.env,
      ARTIFACT_DIR: "artifacts",
      EXTENSION_VERSION: "1.2.3",
      EXTENSION_ID: "codex-acp",
      RELEASE_TAG: "codex-acp-v1.2.3",
    },
  });

  const extensionManifest = JSON.parse(
    fs.readFileSync(path.join(workdir, "artifacts/extension-manifest.json"), "utf8"),
  );
  assert.equal(extensionManifest.extensions[0].id, "codex-acp");
  assert.equal(extensionManifest.extensions[0].kind, "acp_agent");
  assert.equal(
    extensionManifest.extensions[0].artifacts["x86_64-unknown-linux-gnu"].file,
    fileName,
  );
});

test("generate-marketplace-manifest supports language extensions", () => {
  const workdir = makeTempDir();
  copyScript("generate-marketplace-manifest.mjs", workdir);
  fs.mkdirSync(path.join(workdir, "artifacts"), { recursive: true });
  writeJson(path.join(workdir, "extensions/language/rust/extension.build.json"), {
    id: "rust",
    kind: "language",
    language: "tree-sitter-wasm",
    path: "extensions/language/rust",
    targets: ["universal"],
  });
  writeJson(path.join(workdir, "extensions/language/rust/manifest.json"), {
    name: "rust",
    version: "0.24.0",
    file_extensions: ["rs"],
  });
  const fileName = "rust-language-universal.tar.gz";
  fs.writeFileSync(
    path.join(workdir, "artifacts/sha256sums.txt"),
    `${createHash("sha256").update(fileName).digest("hex")}  ${fileName}\n`,
  );

  execFileSync("node", [path.join(workdir, "scripts/generate-marketplace-manifest.mjs")], {
    cwd: workdir,
    env: {
      ...process.env,
      ARTIFACT_DIR: "artifacts",
      EXTENSION_VERSION: "0.24.0",
      EXTENSION_ID: "rust",
      RELEASE_TAG: "rust-v0.24.0",
    },
  });

  const extensionManifest = JSON.parse(
    fs.readFileSync(path.join(workdir, "artifacts/extension-manifest.json"), "utf8"),
  );
  assert.equal(extensionManifest.extensions[0].id, "rust");
  assert.equal(extensionManifest.extensions[0].kind, "language");
  assert.deepEqual(extensionManifest.extensions[0].file_extensions, ["rs"]);
  assert.equal(extensionManifest.extensions[0].artifacts.universal.file, fileName);
});

test("generate-marketplace-manifest supports language bundles", () => {
  const workdir = makeTempDir();
  copyScript("generate-marketplace-manifest.mjs", workdir);
  fs.mkdirSync(path.join(workdir, "artifacts"), { recursive: true });
  writeJson(path.join(workdir, "extensions/language-bundle/tree-sitter-languages/extension.build.json"), {
    id: "tree-sitter-languages",
    kind: "language_bundle",
    language: "tree-sitter-wasm-bundle",
    path: "extensions/language-bundle/tree-sitter-languages",
    targets: ["universal"],
  });
  writeJson(path.join(workdir, "extensions/language-bundle/tree-sitter-languages/manifest.json"), {
    id: "tree-sitter-languages",
    name: "Tree-sitter Languages",
    version: "0.1.0",
    languages: ["javascript", "rust"],
    file_extensions: ["js", "mjs", "rs"],
  });
  const fileName = "tree-sitter-languages-language-bundle-universal.tar.gz";
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
      EXTENSION_ID: "tree-sitter-languages",
      RELEASE_TAG: "tree-sitter-languages-v0.1.0",
    },
  });

  const extensionManifest = JSON.parse(
    fs.readFileSync(path.join(workdir, "artifacts/extension-manifest.json"), "utf8"),
  );
  const entry = extensionManifest.extensions[0];
  assert.equal(entry.id, "tree-sitter-languages");
  assert.equal(entry.kind, "language_bundle");
  assert.deepEqual(entry.file_extensions, ["js", "mjs", "rs"]);
  assert.equal(entry.artifacts.universal.file, fileName);
});

test("upload-r2 workflow exports R2 credentials without AWS STS configuration", () => {
  const workflow = fs.readFileSync(path.join(repoRoot, ".github/workflows/upload-r2.yml"), "utf8");

  assert.doesNotMatch(workflow, /aws-actions\/configure-aws-credentials/);
  assert.match(workflow, /contents:\s+read/);
  assert.match(workflow, /concurrency:/);
  assert.match(workflow, /group:\s+extension-marketplace-publish/);
  assert.match(workflow, /cancel-in-progress:\s+false/);
  assert.match(workflow, /AWS_ACCESS_KEY_ID:\s+\$\{\{\s*secrets\.CLOUDFLARE_R2_ACCESS_KEY_ID\s*\}\}/);
  assert.match(
    workflow,
    /AWS_SECRET_ACCESS_KEY:\s+\$\{\{\s*secrets\.CLOUDFLARE_R2_SECRET_ACCESS_KEY\s*\}\}/,
  );
  assert.match(workflow, /AWS_DEFAULT_REGION:\s+auto\b/);
  assert.match(workflow, /upload_object "\$current_manifest" "\$\{R2_PREFIX\}\/manifest\.json"/);
  assert.match(workflow, /upload_object "manifest\.json" "extensions\/manifest\.json"/);
  assert.match(workflow, /"extensions\/remote-desktop"/);
  assert.match(workflow, /"extensions\/mcp-helper"/);
  assert.match(workflow, /"extensions\/acp-agent"/);
  assert.match(workflow, /remote_desktop_provider/);
  assert.match(workflow, /mcp_helper/);
  assert.match(workflow, /acp_agent/);
  assert.match(workflow, /\$\{process\.env\.EXTENSION_ID\}-remote-desktop-provider-\$\{target\}\.tar\.gz/);
  assert.match(workflow, /\$\{process\.env\.EXTENSION_ID\}-mcp-helper-\$\{target\}\.tar\.gz/);
  assert.match(workflow, /\$\{process\.env\.EXTENSION_ID\}-acp-agent-\$\{target\}\.tar\.gz/);
  assert.doesNotMatch(workflow, /merge-marketplace-manifest\.mjs/);
  assert.doesNotMatch(workflow, /r2-extension-manifest\.json/);
  assert.doesNotMatch(workflow, /CURRENT_MANIFEST=/);
  assert.doesNotMatch(workflow, /EXISTING_MANIFEST=/);
  assert.doesNotMatch(workflow, /\/latest\/\$\{file\}/);
  assert.doesNotMatch(workflow, /MANIFEST_RELEASE_TAG:\s+extensions-manifest/);
  assert.doesNotMatch(workflow, /gh release list/);
  assert.doesNotMatch(workflow, /gh release create "\$MANIFEST_RELEASE_TAG"/);
  assert.doesNotMatch(workflow, /gh release upload "\$MANIFEST_RELEASE_TAG"/);
  assert.doesNotMatch(workflow, /aws s3 cp "s3:\/\/\$\{CLOUDFLARE_R2_BUCKET\}\/extensions\/manifest\.json"/);
});

test("release workflow keeps extension releases scoped to current extension", () => {
  const workflow = fs.readFileSync(path.join(repoRoot, ".github/workflows/release.yml"), "utf8");

  assert.doesNotMatch(workflow, /Merge previous GitHub marketplace manifests/);
  assert.doesNotMatch(workflow, /gh release list/);
  assert.doesNotMatch(workflow, /previous-github-manifests/);
  assert.match(workflow, /artifacts\/extension-manifest\.json/);
  assert.match(workflow, /"extensions\/mcp-helper"/);
  assert.match(workflow, /"extensions\/acp-agent"/);
  assert.match(workflow, /target === "aarch64-unknown-linux-gnu" && kind === "remote_desktop_provider"/);
  assert.match(workflow, /return "ubuntu-24\.04-arm"/);
  assert.match(workflow, /export CARGO_TARGET_DIR="\$\{RUNNER_TEMP\}\/cargo-target"/);
  assert.match(workflow, /export CMAKE_GENERATOR=Ninja/);
  assert.match(workflow, /choco install ninja -y --no-progress/);
  assert.match(workflow, /sudo apt-get install -y pkg-config libasound2-dev libssl-dev/);
  assert.match(workflow, /scripts\/package-mcp-helper\.sh/);
  assert.match(workflow, /scripts\/verify-mcp-helper-package\.sh/);
  assert.match(workflow, /scripts\/package-acp-agent\.sh/);
  assert.match(workflow, /scripts\/verify-acp-agent-package\.sh/);
  assert.match(workflow, /scripts\/package-composite-extension\.sh/);
  assert.match(workflow, /scripts\/verify-composite-package\.sh/);
  assert.match(workflow, /scripts\/package-language-extension\.sh/);
  assert.match(workflow, /scripts\/verify-language-package\.sh/);
  assert.match(workflow, /wasm32-wasip2/);
  assert.match(workflow, /matrix\.target == 'aarch64-unknown-linux-gnu' && matrix\.kind != 'remote_desktop_provider'/);
  assert.match(workflow, /export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc/);
  assert.doesNotMatch(workflow, /CMAKE_GENERATOR:\s+\$\{\{/);
  assert.doesNotMatch(workflow, /CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER:\s+\$\{\{/);
  assert.doesNotMatch(workflow, /sudo dpkg --add-architecture arm64/);
});

test("CI workflow routes Rust, Rust WASM, Go, and Java extension jobs by language", () => {
  const workflow = fs.readFileSync(path.join(repoRoot, ".github/workflows/ci.yml"), "utf8");
  const releaseWorkflow = fs.readFileSync(path.join(repoRoot, ".github/workflows/release.yml"), "utf8");

  assert.match(workflow, /name: Repository checks/);
  assert.match(workflow, /node --test tests\/scripts\.test\.mjs/);
  assert.match(workflow, /Validate workflow YAML/);
  assert.match(workflow, /matrix\.language == 'rust'/);
  assert.match(workflow, /matrix\.language == 'rust-wasm'/);
  assert.match(workflow, /matrix\.language == 'go'/);
  assert.match(workflow, /matrix\.language == 'java'/);
  assert.match(workflow, /actions\/setup-go@v5/);
  assert.match(workflow, /actions\/setup-java@v4/);
  assert.match(workflow, /matrix\.manifest_path != ''/);
  assert.match(workflow, /cargo test --manifest-path "\$\{\{ matrix\.manifest_path \}\}" -- --nocapture/);
  assert.match(workflow, /matrix\.manifest_path == '' && matrix\.package != ''/);
  assert.match(workflow, /cargo test -p "\$\{\{ matrix\.package \}\}" -- --nocapture/);
  assert.match(workflow, /run: go test \.\/\.\.\./);
  assert.match(workflow, /run: mvn -f "\$\{\{ matrix\.package \}\}\/pom\.xml" test/);
  assert.match(workflow, /name: Build Windows x86/);
  assert.match(workflow, /target: i686-pc-windows-msvc/);
  assert.match(workflow, /cargo build --release/);
  assert.match(workflow, /scripts\/build-go-driver\.sh/);
  assert.match(workflow, /scripts\/package-driver\.sh/);
  assert.match(workflow, /scripts\/verify-package\.sh/);
  assert.match(workflow, /scripts\/package-remote-desktop-provider\.sh/);
  assert.match(workflow, /scripts\/verify-remote-desktop-provider-package\.sh/);
  assert.match(workflow, /scripts\/package-acp-agent\.sh/);
  assert.match(workflow, /scripts\/verify-acp-agent-package\.sh/);
  assert.match(workflow, /acp-agent-i686-pc-windows-msvc\.tar\.gz/);
  assert.match(workflow, /matrix\.windows_x86 == true/);
  assert.doesNotMatch(workflow, /scripts\/build-java-driver\.sh/);
  assert.doesNotMatch(workflow, /aarch64-unknown-linux-gnu/);
  assert.match(releaseWorkflow, /if: \$\{\{ matrix\.language == 'java' \}\}\n\s+run: bash scripts\/build-java-driver\.sh/);
  assert.match(releaseWorkflow, /matrix\.language == 'rust-wasm'/);
  assert.match(releaseWorkflow, /cargo build --release -p "\$\{\{ matrix\.package \}\}" --target wasm32-wasip2/);
  assert.doesNotMatch(workflow, /DUCKDB_DOWNLOAD_LIB/);
  assert.match(releaseWorkflow, /if \(language === "go"\) return "ubuntu-latest";/);
  assert.match(
    releaseWorkflow,
    /matrix\.language == 'rust' && matrix\.target == 'aarch64-unknown-linux-gnu'/,
  );
  assert.match(releaseWorkflow, /gcc-aarch64-linux-gnu/);
  assert.match(releaseWorkflow, /g\+\+-aarch64-linux-gnu/);
  assert.match(releaseWorkflow, /CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER/);
  assert.match(releaseWorkflow, /CXX_aarch64_unknown_linux_gnu/);
  assert.doesNotMatch(releaseWorkflow, /DUCKDB_DOWNLOAD_LIB/);
});

test("Java workflows use a runner-available JDK while preserving Java 8 bytecode target", () => {
  const ciWorkflow = fs.readFileSync(path.join(repoRoot, ".github/workflows/ci.yml"), "utf8");
  const releaseWorkflow = fs.readFileSync(path.join(repoRoot, ".github/workflows/release.yml"), "utf8");

  assert.match(ciWorkflow, /java-version:\s+'11'/);
  assert.match(releaseWorkflow, /java-version:\s+'11'/);
  assert.match(
    fs.readFileSync(path.join(repoRoot, "java/gbase8s-ipc-driver/pom.xml"), "utf8"),
    /<maven\.compiler\.target>1\.8<\/maven\.compiler\.target>/,
  );
});

test("install-local-drivers builds and replaces one selected local driver", () => {
  const workdir = makeTempDir();
  copyScript("install-local-drivers.sh", workdir);
  copyScript("package-driver.sh", workdir);
  copyScript("verify-package.sh", workdir);
  createRustDriverFixture(workdir, "duckdb", "duckdb_driver", "0.9.0");
  createRustDriverFixture(workdir, "iotdb", "iotdb_driver", "0.9.0");
  const installRoot = path.join(workdir, "navop/extensions/database_drivers");
  fs.mkdirSync(path.join(installRoot, "duckdb"), { recursive: true });
  fs.writeFileSync(path.join(installRoot, "duckdb/old.txt"), "old duckdb\n");
  fs.mkdirSync(path.join(installRoot, "iotdb"), { recursive: true });
  fs.writeFileSync(path.join(installRoot, "iotdb/old.txt"), "old iotdb\n");

  const output = execFileSync(
    "bash",
    [path.join(workdir, "scripts/install-local-drivers.sh"), "duckdb"],
    {
      cwd: workdir,
      encoding: "utf8",
      env: {
        ...process.env,
        NAVOP_DATABASE_DRIVER_DIR: installRoot,
        PATH: `${createFakeRustToolchain(workdir)}${path.delimiter}${process.env.PATH}`,
      },
    },
  );

  assert.match(output, /Installed duckdb ->/);
  assert.equal(
    fs.readFileSync(path.join(installRoot, "duckdb/driver.json"), "utf8").includes('"version": "0.9.0"'),
    true,
  );
  assert.equal(
    fs.readFileSync(path.join(installRoot, "duckdb/duckdb_driver"), "utf8"),
    "fake duckdb binary\n",
  );
  assert.equal(fs.existsSync(path.join(installRoot, "duckdb/old.txt")), false);
  assert.equal(fs.readFileSync(path.join(installRoot, "iotdb/old.txt"), "utf8"), "old iotdb\n");
  const backups = fs
    .readdirSync(path.join(installRoot, ".backups"))
    .filter((name) => name.startsWith("duckdb.backup."));
  assert.equal(backups.length, 1);
  assert.equal(
    fs.readFileSync(path.join(installRoot, ".backups", backups[0], "old.txt"), "utf8"),
    "old duckdb\n",
  );
});

test("install-local-drivers defaults to the navop driver directory", () => {
  const script = fs.readFileSync(path.join(repoRoot, "scripts/install-local-drivers.sh"), "utf8");

  assert.match(script, /NAVOP_DATABASE_DRIVER_DIR/);
  assert.match(script, /ONETCLI_DATABASE_DRIVER_DIR/);
  assert.match(script, /\$XDG_CONFIG_HOME\/navop\/extensions\/database_drivers/);
  assert.match(script, /\$HOME\/\.config\/navop\/extensions\/database_drivers/);
  assert.match(script, /\$\{CONFIG_HOME\}\/navop\/extensions\/database_drivers/);
});

test("install-local-drivers installs all local drivers when no id is passed", () => {
  const workdir = makeTempDir();
  copyScript("install-local-drivers.sh", workdir);
  copyScript("package-driver.sh", workdir);
  copyScript("verify-package.sh", workdir);
  createRustDriverFixture(workdir, "duckdb", "duckdb_driver", "0.9.0");
  createRustDriverFixture(workdir, "iotdb", "iotdb_driver", "0.8.0");
  const installRoot = path.join(workdir, "navop/extensions/database_drivers");

  const output = execFileSync("bash", [path.join(workdir, "scripts/install-local-drivers.sh")], {
    cwd: workdir,
    encoding: "utf8",
    env: {
      ...process.env,
      ONETCLI_DATABASE_DRIVER_DIR: installRoot,
      PATH: `${createFakeRustToolchain(workdir)}${path.delimiter}${process.env.PATH}`,
    },
  });

  assert.match(output, /Installed duckdb ->/);
  assert.match(output, /Installed iotdb ->/);
  assert.ok(fs.existsSync(path.join(installRoot, "duckdb/driver.json")));
  assert.ok(fs.existsSync(path.join(installRoot, "iotdb/driver.json")));
});

test("install-local-drivers installs universal drivers without requiring rustc", () => {
  const workdir = makeTempDir();
  copyScript("install-local-drivers.sh", workdir);
  copyScript("package-driver.sh", workdir);
  copyScript("verify-package.sh", workdir);
  copyScript("build-java-driver.sh", workdir);
  createJavaDriverFixture(workdir, "gbase8s", "gbase8s-ipc-driver", "0.7.0");
  const installRoot = path.join(workdir, "navop/extensions/database_drivers");

  const output = execFileSync(
    "bash",
    [path.join(workdir, "scripts/install-local-drivers.sh"), "gbase8s"],
    {
      cwd: workdir,
      encoding: "utf8",
      env: {
        ...process.env,
        NAVOP_DATABASE_DRIVER_DIR: installRoot,
        PATH: `${createFailingRustc(workdir)}${path.delimiter}${process.env.PATH}`,
      },
    },
  );

  assert.match(output, /Building gbase8s \(java, universal\)/);
  assert.ok(fs.existsSync(path.join(installRoot, "gbase8s/driver.json")));
  assert.ok(fs.existsSync(path.join(installRoot, "gbase8s/gbase8s-ipc-driver")));
  assert.ok(fs.existsSync(path.join(installRoot, "gbase8s/gbase8s-ipc-driver.cmd")));
  assert.equal(
    fs.readFileSync(path.join(installRoot, "gbase8s/lib/gbase8s-ipc-driver.jar"), "utf8"),
    "fake shaded jar\n",
  );
});

test("install-local-remote-desktop-providers builds and replaces one selected provider", () => {
  const workdir = makeTempDir();
  copyScript("install-local-remote-desktop-providers.sh", workdir);
  copyScript("package-remote-desktop-provider.sh", workdir);
  copyScript("verify-remote-desktop-provider-package.sh", workdir);
  createRemoteDesktopProviderFixture(workdir, {
    id: "rdp",
    protocol: "rdp",
    version: "0.9.0",
    target: "aarch64-apple-darwin",
  });
  createRemoteDesktopProviderFixture(workdir, {
    id: "vnc",
    protocol: "vnc",
    version: "0.8.0",
    target: "aarch64-apple-darwin",
  });
  const installRoot = path.join(workdir, "navop/extensions/remote_desktop_providers");
  fs.mkdirSync(path.join(installRoot, "rdp"), { recursive: true });
  fs.writeFileSync(path.join(installRoot, "rdp/old.txt"), "old rdp\n");
  fs.mkdirSync(path.join(installRoot, "vnc"), { recursive: true });
  fs.writeFileSync(path.join(installRoot, "vnc/old.txt"), "old vnc\n");

  const output = execFileSync(
    "bash",
    [path.join(workdir, "scripts/install-local-remote-desktop-providers.sh"), "rdp"],
    {
      cwd: workdir,
      encoding: "utf8",
      env: {
        ...process.env,
        ONETCLI_REMOTE_DESKTOP_PROVIDER_DIR: installRoot,
        PATH: `${createFakeRustToolchain(workdir)}${path.delimiter}${process.env.PATH}`,
      },
    },
  );

  assert.match(output, /Installed rdp ->/);
  assert.equal(
    fs
      .readFileSync(path.join(installRoot, "rdp/remote_desktop_provider.json"), "utf8")
      .includes('"version": "0.9.0"'),
    true,
  );
  assert.equal(
    fs.readFileSync(path.join(installRoot, "rdp/navop-rdp-helper"), "utf8"),
    "fake rdp helper\n",
  );
  assert.equal(fs.existsSync(path.join(installRoot, "rdp/old.txt")), false);
  assert.equal(fs.readFileSync(path.join(installRoot, "vnc/old.txt"), "utf8"), "old vnc\n");
  const backups = fs
    .readdirSync(path.join(installRoot, ".backups"))
    .filter((name) => name.startsWith("rdp.backup."));
  assert.equal(backups.length, 1);
  assert.equal(
    fs.readFileSync(path.join(installRoot, ".backups", backups[0], "old.txt"), "utf8"),
    "old rdp\n",
  );
});

test("install-local-remote-desktop-providers follows the Navop config migration state", () => {
  const workdir = makeTempDir();
  copyScript("install-local-remote-desktop-providers.sh", workdir);
  const scriptPath = path.join(workdir, "scripts/install-local-remote-desktop-providers.sh");
  const defaultConfigRoot = path.join(workdir, ".config");
  const xdgConfigRoot = path.join(workdir, "xdg-config");
  const currentRoot = path.join(defaultConfigRoot, "navop");
  const legacyRoot = path.join(defaultConfigRoot, "one-hub");
  const providerSuffix = path.join("extensions", "remote_desktop_providers");
  const baseEnv = {
    ...process.env,
    HOME: workdir,
  };
  delete baseEnv.XDG_CONFIG_HOME;
  delete baseEnv.NAVOP_REMOTE_DESKTOP_PROVIDER_DIR;
  delete baseEnv.ONETCLI_REMOTE_DESKTOP_PROVIDER_DIR;

  const resolveProviderDir = (envOverrides = {}) =>
    execFileSync(
      "bash",
      ["-c", 'source "$1"; remote_desktop_provider_dir', "bash", scriptPath],
      {
        cwd: workdir,
        encoding: "utf8",
        env: {
          ...baseEnv,
          ...envOverrides,
        },
      },
    ).trim();

  assert.equal(
    resolveProviderDir(),
    path.join(defaultConfigRoot, "navop", providerSuffix),
  );
  assert.equal(
    resolveProviderDir({ XDG_CONFIG_HOME: xdgConfigRoot }),
    path.join(defaultConfigRoot, "navop", providerSuffix),
  );

  fs.mkdirSync(legacyRoot, { recursive: true });
  assert.equal(
    resolveProviderDir({ XDG_CONFIG_HOME: xdgConfigRoot }),
    path.join(legacyRoot, providerSuffix),
  );

  fs.mkdirSync(currentRoot, { recursive: true });
  fs.writeFileSync(path.join(currentRoot, ".one-hub-migration-complete"), "");
  assert.equal(
    resolveProviderDir({ XDG_CONFIG_HOME: xdgConfigRoot }),
    path.join(currentRoot, providerSuffix),
  );
});

test("install-local-remote-desktop-providers installs all local providers when no id is passed", () => {
  const workdir = makeTempDir();
  copyScript("install-local-remote-desktop-providers.sh", workdir);
  copyScript("package-remote-desktop-provider.sh", workdir);
  copyScript("verify-remote-desktop-provider-package.sh", workdir);
  createRemoteDesktopProviderFixture(workdir, {
    id: "rdp",
    protocol: "rdp",
    version: "0.9.0",
    target: "aarch64-apple-darwin",
  });
  createRemoteDesktopProviderFixture(workdir, {
    id: "vnc",
    protocol: "vnc",
    version: "0.8.0",
    target: "aarch64-apple-darwin",
  });
  const installRoot = path.join(workdir, "navop/extensions/remote_desktop_providers");

  const output = execFileSync(
    "bash",
    [path.join(workdir, "scripts/install-local-remote-desktop-providers.sh")],
    {
      cwd: workdir,
      encoding: "utf8",
      env: {
        ...process.env,
        NAVOP_REMOTE_DESKTOP_PROVIDER_DIR: installRoot,
        PATH: `${createFakeRustToolchain(workdir)}${path.delimiter}${process.env.PATH}`,
      },
    },
  );

  assert.match(output, /Installed rdp ->/);
  assert.match(output, /Installed vnc ->/);
  assert.ok(fs.existsSync(path.join(installRoot, "rdp/remote_desktop_provider.json")));
  assert.ok(fs.existsSync(path.join(installRoot, "vnc/remote_desktop_provider.json")));
});

test("install-local-mcp-helpers builds and replaces one selected helper", () => {
  const workdir = makeTempDir();
  copyScript("install-local-mcp-helpers.sh", workdir);
  copyScript("package-mcp-helper.sh", workdir);
  copyScript("verify-mcp-helper-package.sh", workdir);
  createMcpHelperFixture(workdir, {
    id: "onetcli-public-mcp",
    version: "0.9.0",
    target: "aarch64-apple-darwin",
  });
  const installRoot = path.join(workdir, "navop/extensions/mcp_helpers");
  fs.mkdirSync(path.join(installRoot, "onetcli-public-mcp"), { recursive: true });
  fs.writeFileSync(path.join(installRoot, "onetcli-public-mcp/old.txt"), "old helper\n");

  const output = execFileSync(
    "bash",
    [path.join(workdir, "scripts/install-local-mcp-helpers.sh"), "onetcli-public-mcp"],
    {
      cwd: workdir,
      encoding: "utf8",
      env: {
        ...process.env,
        NAVOP_MCP_HELPER_DIR: installRoot,
        PATH: `${createFakeRustToolchain(workdir)}${path.delimiter}${process.env.PATH}`,
      },
    },
  );

  assert.match(output, /Installed onetcli-public-mcp ->/);
  assert.equal(
    fs
      .readFileSync(path.join(installRoot, "onetcli-public-mcp/mcp_helper.json"), "utf8")
      .includes('"version": "0.9.0"'),
    true,
  );
  assert.equal(
    fs.readFileSync(path.join(installRoot, "onetcli-public-mcp/onetcli-public-mcp"), "utf8"),
    "fake onetcli-public-mcp helper\n",
  );
  assert.equal(fs.existsSync(path.join(installRoot, "onetcli-public-mcp/old.txt")), false);
});

test("install-local-mcp-helpers defaults to the one-hub helper directory", () => {
  const script = fs.readFileSync(
    path.join(repoRoot, "scripts/install-local-mcp-helpers.sh"),
    "utf8",
  );

  assert.match(script, /NAVOP_MCP_HELPER_DIR/);
  assert.match(script, /ONETCLI_MCP_HELPER_DIR/);
  assert.match(script, /\$XDG_CONFIG_HOME\/one-hub\/extensions\/mcp_helpers/);
  assert.match(script, /\$HOME\/\.config\/one-hub\/extensions\/mcp_helpers/);
  assert.match(script, /\$\{CONFIG_HOME\}\/one-hub\/extensions\/mcp_helpers/);
});

test("install-local-languages packages and replaces one selected language", () => {
  const workdir = makeTempDir();
  copyScript("install-local-languages.sh", workdir);
  createLanguageExtensionFixture(workdir, {
    id: "rust",
    version: "0.24.2",
    fileExtensions: ["rs"],
  });
  createLanguageExtensionFixture(workdir, {
    id: "python",
    version: "0.23.6",
    fileExtensions: ["py"],
  });
  const installRoot = path.join(workdir, "navop/extensions/languages");
  fs.mkdirSync(path.join(installRoot, "rust"), { recursive: true });
  fs.writeFileSync(path.join(installRoot, "rust/old.txt"), "old rust\n");
  fs.mkdirSync(path.join(installRoot, "python"), { recursive: true });
  fs.writeFileSync(path.join(installRoot, "python/old.txt"), "old python\n");

  const output = execFileSync(
    "bash",
    [path.join(workdir, "scripts/install-local-languages.sh"), "rust"],
    {
      cwd: workdir,
      encoding: "utf8",
      env: {
        ...process.env,
        NAVOP_LANGUAGE_DIR: installRoot,
      },
    },
  );

  assert.match(output, /Installed rust ->/);
  assert.equal(
    fs.readFileSync(path.join(installRoot, "rust/manifest.json"), "utf8").includes('"version": "0.24.2"'),
    true,
  );
  assert.equal(
    fs.readFileSync(path.join(installRoot, "rust/parser.wasm"), "utf8"),
    "fake parser wasm\n",
  );
  assert.equal(fs.existsSync(path.join(installRoot, "rust/old.txt")), false);
  assert.equal(fs.readFileSync(path.join(installRoot, "python/old.txt"), "utf8"), "old python\n");
  const backups = fs
    .readdirSync(path.join(installRoot, ".backups"))
    .filter((name) => name.startsWith("rust.backup."));
  assert.equal(backups.length, 1);
  assert.equal(
    fs.readFileSync(path.join(installRoot, ".backups", backups[0], "old.txt"), "utf8"),
    "old rust\n",
  );
});

test("install-local-languages defaults to the one-hub language directory", () => {
  const script = fs.readFileSync(
    path.join(repoRoot, "scripts/install-local-languages.sh"),
    "utf8",
  );

  assert.match(script, /NAVOP_LANGUAGE_DIR/);
  assert.match(script, /ONETCLI_LANGUAGE_DIR/);
  assert.match(script, /\$XDG_CONFIG_HOME\/one-hub\/extensions\/languages/);
  assert.match(script, /\$HOME\/\.config\/one-hub\/extensions\/languages/);
  assert.match(script, /\$\{CONFIG_HOME\}\/one-hub\/extensions\/languages/);
});

test("install-local-acp-agents packages and replaces one selected ACP agent", () => {
  const workdir = makeTempDir();
  copyScript("install-local-acp-agents.sh", workdir);
  copyScript("package-acp-agent.sh", workdir);
  copyScript("verify-acp-agent-package.sh", workdir);
  createAcpAgentFixture(workdir, {
    id: "codex-acp",
    binary: "codex-acp",
    version: "0.9.0",
    target: "aarch64-apple-darwin",
    packageName: "@agentclientprotocol/codex-acp@1.0.1",
  });
  const installRoot = path.join(workdir, "navop/extensions/acp_agents");
  fs.mkdirSync(path.join(installRoot, "codex-acp"), { recursive: true });
  fs.writeFileSync(path.join(installRoot, "codex-acp/old.txt"), "old codex\n");

  const output = execFileSync(
    "bash",
    [path.join(workdir, "scripts/install-local-acp-agents.sh"), "codex-acp"],
    {
      cwd: workdir,
      encoding: "utf8",
      env: {
        ...process.env,
        NAVOP_ACP_AGENT_DIR: installRoot,
        PATH: `${createFakeRustToolchain(workdir)}${path.delimiter}${process.env.PATH}`,
      },
    },
  );

  assert.match(output, /Installed codex-acp ->/);
  assert.equal(
    fs
      .readFileSync(path.join(installRoot, "codex-acp/acp_agent.json"), "utf8")
      .includes('"version": "0.9.0"'),
    true,
  );
  assert.match(
    fs.readFileSync(path.join(installRoot, "codex-acp/bin/codex-acp"), "utf8"),
    /@agentclientprotocol\/codex-acp@1\.0\.1/,
  );
  assert.equal(fs.existsSync(path.join(installRoot, "codex-acp/old.txt")), false);
  assert.equal(
    fs.existsSync(path.join(installRoot, ".backups")),
    false,
    "ACP agent backups must not live under the scanned acp_agents root",
  );
  const backups = fs
    .readdirSync(`${installRoot}.backups`)
    .filter((name) => name.startsWith("codex-acp.backup."));
  assert.equal(backups.length, 1);
  assert.equal(
    fs.readFileSync(path.join(`${installRoot}.backups`, backups[0], "old.txt"), "utf8"),
    "old codex\n",
  );
});

test("install-local-acp-agents defaults to the one-hub ACP agent directory", () => {
  const script = fs.readFileSync(
    path.join(repoRoot, "scripts/install-local-acp-agents.sh"),
    "utf8",
  );

  assert.match(script, /NAVOP_ACP_AGENT_DIR/);
  assert.match(script, /ONETCLI_ACP_AGENT_DIR/);
  assert.match(script, /\$XDG_CONFIG_HOME\/one-hub\/extensions\/acp_agents/);
  assert.match(script, /\$HOME\/\.config\/one-hub\/extensions\/acp_agents/);
  assert.match(script, /\$\{CONFIG_HOME\}\/one-hub\/extensions\/acp_agents/);
});

test("release-driver packages selected targets and writes release artifacts", () => {
  const workdir = makeTempDir();
  copyScript("release-driver.mjs", workdir);
  copyScript("package-driver.sh", workdir);
  copyScript("verify-package.sh", workdir);
  copyScript("generate-marketplace-manifest.mjs", workdir);
  createPackageFixture(workdir, {
    id: "duckdb",
    binary: "duckdb_driver",
    binaryContents: "fake duckdb release binary\n",
    metadata: {
      path: "extensions/ipc/duckdb",
      targets: ["x86_64-unknown-linux-gnu"],
      releaseTagPrefix: "duckdb-v",
      r2Prefix: "extensions/duckdb",
    },
    driverJson: {
      id: "duckdb",
      name: "DuckDB",
      version: "0.0.0",
      description: "DuckDB embedded analytical database IPC driver",
      entry: {},
    },
  });

  const output = execFileSync(
    "node",
    [
      path.join(workdir, "scripts/release-driver.mjs"),
      "duckdb",
      "1.2.3",
      "--target",
      "x86_64-unknown-linux-gnu",
      "--skip-build",
      "--artifact-dir",
      "artifacts",
    ],
    { cwd: workdir, encoding: "utf8" },
  );

  assert.match(output, /Packaging duckdb \(x86_64-unknown-linux-gnu\)/);
  assert.match(output, /Release artifacts ready:/);
  assert.ok(fs.existsSync(path.join(workdir, "artifacts/duckdb-driver-x86_64-unknown-linux-gnu.tar.gz")));
  assert.match(
    fs.readFileSync(path.join(workdir, "artifacts/sha256sums.txt"), "utf8"),
    /^[0-9a-f]{64}\s+duckdb-driver-x86_64-unknown-linux-gnu\.tar\.gz\n$/,
  );

  const extensionManifest = JSON.parse(
    fs.readFileSync(path.join(workdir, "artifacts/extension-manifest.json"), "utf8"),
  );
  assert.equal(extensionManifest.release_version, "duckdb-v1.2.3");
  assert.equal(extensionManifest.extensions[0].id, "duckdb");
  assert.equal(extensionManifest.extensions[0].version, "1.2.3");
  assert.equal(
    extensionManifest.extensions[0].artifacts["x86_64-unknown-linux-gnu"].file,
    "duckdb-driver-x86_64-unknown-linux-gnu.tar.gz",
  );

  const releaseMetadata = JSON.parse(
    fs.readFileSync(path.join(workdir, "artifacts/release-metadata.json"), "utf8"),
  );
  assert.deepEqual(releaseMetadata, {
    release_tag: "duckdb-v1.2.3",
    extension_id: "duckdb",
    extension_version: "1.2.3",
  });
});

test("release-driver delegates Go driver builds to the existing build script", () => {
  const workdir = makeTempDir();
  copyScript("release-driver.mjs", workdir);
  copyScript("package-driver.sh", workdir);
  copyScript("verify-package.sh", workdir);
  copyScript("generate-marketplace-manifest.mjs", workdir);
  createPackageFixture(workdir, {
    id: "dm",
    binary: "dm-ipc-driver",
    binaryContents: "fake dm go binary\n",
    language: "go",
    package: "./cmd/dm-ipc-driver",
    metadata: {
      path: "extensions/ipc/dm",
      targets: ["x86_64-unknown-linux-gnu"],
      releaseTagPrefix: "dm-v",
      r2Prefix: "extensions/dm",
    },
  });
  fs.writeFileSync(
    path.join(workdir, "scripts/build-go-driver.sh"),
    [
      "#!/usr/bin/env bash",
      "set -euo pipefail",
      "printf '%s %s\\n' \"$1\" \"$2\" >> build-go-driver.calls",
      "",
    ].join("\n"),
    { mode: 0o755 },
  );

  execFileSync(
    "node",
    [
      path.join(workdir, "scripts/release-driver.mjs"),
      "dm",
      "0.4.0",
      "--target",
      "x86_64-unknown-linux-gnu",
    ],
    { cwd: workdir, encoding: "utf8" },
  );

  assert.equal(
    fs.readFileSync(path.join(workdir, "build-go-driver.calls"), "utf8"),
    "dm x86_64-unknown-linux-gnu\n",
  );
  assert.ok(fs.existsSync(path.join(workdir, "artifacts/dm-driver-x86_64-unknown-linux-gnu.tar.gz")));
});

test("release-driver packages MCP helpers", () => {
  const workdir = makeTempDir();
  copyScript("release-driver.mjs", workdir);
  copyScript("package-mcp-helper.sh", workdir);
  copyScript("verify-mcp-helper-package.sh", workdir);
  copyScript("generate-marketplace-manifest.mjs", workdir);
  createMcpHelperFixture(workdir, {
    id: "onetcli-public-mcp",
    version: "0.0.0",
    target: "x86_64-unknown-linux-gnu",
  });

  const output = execFileSync(
    "node",
    [
      path.join(workdir, "scripts/release-driver.mjs"),
      "onetcli-public-mcp",
      "1.2.3",
      "--target",
      "x86_64-unknown-linux-gnu",
      "--skip-build",
      "--artifact-dir",
      "artifacts",
    ],
    { cwd: workdir, encoding: "utf8" },
  );

  assert.match(output, /Packaging onetcli-public-mcp \(x86_64-unknown-linux-gnu\)/);
  assert.ok(
    fs.existsSync(
      path.join(workdir, "artifacts/onetcli-public-mcp-mcp-helper-x86_64-unknown-linux-gnu.tar.gz"),
    ),
  );
  const extensionManifest = JSON.parse(
    fs.readFileSync(path.join(workdir, "artifacts/extension-manifest.json"), "utf8"),
  );
  assert.equal(extensionManifest.extensions[0].kind, "mcp_helper");
  assert.equal(
    extensionManifest.extensions[0].artifacts["x86_64-unknown-linux-gnu"].file,
    "onetcli-public-mcp-mcp-helper-x86_64-unknown-linux-gnu.tar.gz",
  );
});

test("release-driver packages ACP agents", () => {
  const workdir = makeTempDir();
  copyScript("release-driver.mjs", workdir);
  copyScript("package-acp-agent.sh", workdir);
  copyScript("verify-acp-agent-package.sh", workdir);
  copyScript("generate-marketplace-manifest.mjs", workdir);
  createAcpAgentFixture(workdir, {
    id: "codex-acp",
    binary: "codex-acp",
    version: "0.0.0",
    target: "x86_64-unknown-linux-gnu",
    packageName: "@agentclientprotocol/codex-acp@1.0.1",
  });

  const output = execFileSync(
    "node",
    [
      path.join(workdir, "scripts/release-driver.mjs"),
      "codex-acp",
      "1.2.3",
      "--target",
      "x86_64-unknown-linux-gnu",
      "--skip-build",
      "--artifact-dir",
      "artifacts",
    ],
    { cwd: workdir, encoding: "utf8" },
  );

  assert.match(output, /Packaging codex-acp \(x86_64-unknown-linux-gnu\)/);
  assert.ok(
    fs.existsSync(
      path.join(workdir, "artifacts/codex-acp-acp-agent-x86_64-unknown-linux-gnu.tar.gz"),
    ),
  );
  const extensionManifest = JSON.parse(
    fs.readFileSync(path.join(workdir, "artifacts/extension-manifest.json"), "utf8"),
  );
  assert.equal(extensionManifest.extensions[0].kind, "acp_agent");
  assert.equal(
    extensionManifest.extensions[0].artifacts["x86_64-unknown-linux-gnu"].file,
    "codex-acp-acp-agent-x86_64-unknown-linux-gnu.tar.gz",
  );
});

test("release-driver packages composite wasm extensions", () => {
  const workdir = makeTempDir();
  copyScript("release-driver.mjs", workdir);
  copyScript("package-composite-extension.sh", workdir);
  copyScript("verify-composite-package.sh", workdir);
  copyScript("generate-marketplace-manifest.mjs", workdir);
  createDbeaverImporterFixture(workdir, { version: "0.0.0" });

  const output = execFileSync(
    "node",
    [
      path.join(workdir, "scripts/release-driver.mjs"),
      "dbeaver-importer",
      "1.2.3",
      "--target",
      "universal",
      "--skip-build",
      "--artifact-dir",
      "artifacts",
    ],
    { cwd: workdir, encoding: "utf8" },
  );

  assert.match(output, /Packaging dbeaver-importer \(universal\)/);
  assert.ok(
    fs.existsSync(
      path.join(workdir, "artifacts/dbeaver-importer-composite-universal.tar.gz"),
    ),
  );
  const extensionManifest = JSON.parse(
    fs.readFileSync(path.join(workdir, "artifacts/extension-manifest.json"), "utf8"),
  );
  assert.equal(extensionManifest.extensions[0].kind, "composite");
  assert.equal(
    extensionManifest.extensions[0].artifacts.universal.file,
    "dbeaver-importer-composite-universal.tar.gz",
  );
});

test("release-driver packages native composite shell extensions", () => {
  const workdir = makeTempDir();
  copyScript("release-driver.mjs", workdir);
  copyScript("package-composite-extension.sh", workdir);
  copyScript("verify-composite-package.sh", workdir);
  copyScript("generate-marketplace-manifest.mjs", workdir);
  createNativeCompositeFixture(workdir);

  const output = execFileSync(
    "node",
    [
      path.join(workdir, "scripts/release-driver.mjs"),
      "elasticsearch",
      "1.2.3",
      "--target",
      "x86_64-unknown-linux-gnu",
      "--skip-build",
      "--artifact-dir",
      "artifacts",
    ],
    { cwd: workdir, encoding: "utf8" },
  );

  assert.match(output, /Packaging elasticsearch \(x86_64-unknown-linux-gnu\)/);
  assert.ok(
    fs.existsSync(
      path.join(workdir, "artifacts/elasticsearch-composite-x86_64-unknown-linux-gnu.tar.gz"),
    ),
  );
  const extensionManifest = JSON.parse(
    fs.readFileSync(path.join(workdir, "artifacts/extension-manifest.json"), "utf8"),
  );
  assert.equal(extensionManifest.extensions[0].kind, "composite");
  assert.equal(
    extensionManifest.extensions[0].artifacts["x86_64-unknown-linux-gnu"].file,
    "elasticsearch-composite-x86_64-unknown-linux-gnu.tar.gz",
  );
});

function makeTempDir() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "navop-extensions-test-"));
  fs.mkdirSync(path.join(dir, "unpacked"), { recursive: true });
  return dir;
}

function extensionBuildEntries() {
  const roots = ["extensions/ipc", "extensions/remote-desktop", "extensions/mcp-helper", "extensions/acp-agent", "extensions/composite", "extensions/wasm", "extensions/language", "extensions/language-bundle"];
  const entries = [];
  for (const root of roots) {
    if (!fs.existsSync(path.join(repoRoot, root))) continue;
    for (const id of fs.readdirSync(path.join(repoRoot, root))) {
      const metadataPath = path.join(repoRoot, root, id, "extension.build.json");
      if (fs.existsSync(metadataPath)) {
        entries.push(JSON.parse(fs.readFileSync(metadataPath, "utf8")));
      }
    }
  }
  return entries;
}

function manifestFileForKind(kind) {
  if (kind === "database_driver") return "driver.json";
  if (kind === "remote_desktop_provider") return "remote_desktop_provider.json";
  if (kind === "mcp_helper") return "mcp_helper.json";
  if (kind === "acp_agent") return "acp_agent.json";
  if (kind === "composite") return "extension.json";
  if (kind === "language") return "manifest.json";
  if (kind === "language_bundle") return "manifest.json";
  throw new Error(`unsupported manifest kind: ${kind}`);
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
    path: `extensions/ipc/${id}`,
    targets: ["x86_64-unknown-linux-gnu"],
    ...options.metadata,
  });
  writeJson(path.join(workdir, `extensions/ipc/${id}/driver.json`), options.driverJson || {
    id,
    version: "0.0.0",
    entry: {},
  });
  if (!options.withoutLocales) {
    fs.mkdirSync(path.join(workdir, `extensions/ipc/${id}/locales`), { recursive: true });
    fs.writeFileSync(path.join(workdir, `extensions/ipc/${id}/locales/en.yml`), `name: ${id}\n`);
  }
  if (options.icons) {
    fs.mkdirSync(path.join(workdir, `extensions/ipc/${id}/icons`), { recursive: true });
    for (const [name, contents] of Object.entries(options.icons)) {
      fs.writeFileSync(path.join(workdir, `extensions/ipc/${id}/icons`, name), contents);
    }
  }
  fs.mkdirSync(path.join(workdir, "target/x86_64-unknown-linux-gnu/release"), { recursive: true });
  fs.writeFileSync(
    path.join(workdir, `target/x86_64-unknown-linux-gnu/release/${binary}`),
    binaryContents,
  );
}

function createRemoteDesktopProviderFixture(workdir, options = {}) {
  const id = options.id || "rdp";
  const protocol = options.protocol || id;
  const binary = options.binary || `navop-${id}-helper`;
  const version = options.version || "0.0.0";
  const target = options.target || "x86_64-unknown-linux-gnu";
  copyScript("package-remote-desktop-provider.sh", workdir);
  copyScript("verify-remote-desktop-provider-package.sh", workdir);
  writeJson(path.join(workdir, `extensions/remote-desktop/${id}/extension.build.json`), {
    id,
    kind: "remote_desktop_provider",
    package: binary,
    binary,
    ...(options.manifestPath ? { manifest_path: options.manifestPath } : {}),
    path: `extensions/remote-desktop/${id}`,
    targets: [target],
  });
  writeJson(path.join(workdir, `extensions/remote-desktop/${id}/remote_desktop_provider.json`), {
    id,
    name: id.toUpperCase(),
    description: `${id.toUpperCase()} remote desktop provider`,
    version,
    protocol,
    entry: {},
    capabilities: {
      resize: "remote_resize",
      clipboard_text: true,
      cursor_shape: true,
      audio: false,
      file_transfer: false,
    },
  });
  const targetRoot = options.targetRoot || "target";
  fs.mkdirSync(path.join(workdir, targetRoot, `${target}/release`), {
    recursive: true,
  });
  fs.writeFileSync(
    path.join(workdir, targetRoot, `${target}/release/${binary}`),
    `fake ${id} helper\n`,
  );
}

function createMcpHelperFixture(workdir, options = {}) {
  const id = options.id || "onetcli-public-mcp";
  const binary = options.binary || id;
  const version = options.version || "0.0.0";
  const target = options.target || "x86_64-unknown-linux-gnu";
  copyScript("package-mcp-helper.sh", workdir);
  copyScript("verify-mcp-helper-package.sh", workdir);
  writeJson(path.join(workdir, `extensions/mcp-helper/${id}/extension.build.json`), {
    id,
    kind: "mcp_helper",
    package: binary,
    binary,
    ...(options.manifestPath ? { manifest_path: options.manifestPath } : {}),
    path: `extensions/mcp-helper/${id}`,
    targets: [target],
  });
  writeJson(path.join(workdir, `extensions/mcp-helper/${id}/mcp_helper.json`), {
    id,
    name: "Navop Public MCP Helper",
    description: "Public MCP stdio bridge",
    version,
    entry: {
      command: `./${binary}`,
    },
  });
  const targetRoot = options.targetRoot || "target";
  fs.mkdirSync(path.join(workdir, targetRoot, `${target}/release`), {
    recursive: true,
  });
  fs.writeFileSync(
    path.join(workdir, targetRoot, `${target}/release/${binary}`),
    `fake ${id} helper\n`,
    { mode: 0o755 },
  );
}

function createAcpAgentFixture(workdir, options = {}) {
  const id = options.id || "codex-acp";
  const binary = options.binary || id;
  const version = options.version || "0.0.0";
  const target = options.target || "x86_64-unknown-linux-gnu";
  const packageName = options.packageName || "@agentclientprotocol/codex-acp@1.0.1";
  copyScript("package-acp-agent.sh", workdir);
  copyScript("verify-acp-agent-package.sh", workdir);
  writeJson(path.join(workdir, `extensions/acp-agent/${id}/extension.build.json`), {
    id,
    kind: "acp_agent",
    language: "shell",
    package: "",
    binary,
    path: `extensions/acp-agent/${id}`,
    targets: [target],
  });
  writeJson(path.join(workdir, `extensions/acp-agent/${id}/acp_agent.json`), {
    id,
    name: id,
    description: "Shell wrapper for an ACP Registry agent",
    version,
    agents: [
      {
        id,
        name: id,
        transport: {
          type: "stdio",
          command: `bin/${binary}`,
          args: [],
          env: {},
        },
      },
    ],
  });
  fs.mkdirSync(path.join(workdir, `extensions/acp-agent/${id}/bin`), { recursive: true });
  fs.writeFileSync(
    path.join(workdir, `extensions/acp-agent/${id}/bin/${binary}`),
    `#!/usr/bin/env sh\nexec npm exec --yes -- ${packageName} "$@"\n`,
    { mode: 0o755 },
  );
  fs.writeFileSync(
    path.join(workdir, `extensions/acp-agent/${id}/bin/${binary}.cmd`),
    `@echo off\r\nnpm exec --yes -- ${packageName} %*\r\n`,
  );
}

function createDbeaverImporterFixture(workdir, options = {}) {
  const id = options.id || "dbeaver-importer";
  const version = options.version || "0.0.0";
  copyScript("package-composite-extension.sh", workdir);
  copyScript("verify-composite-package.sh", workdir);
  writeJson(path.join(workdir, `extensions/wasm/${id}/extension.build.json`), {
    id,
    kind: "composite",
    language: "rust-wasm",
    package: "dbeaver_importer_wasm",
    binary: "dbeaver_importer_wasm.wasm",
    path: `extensions/wasm/${id}`,
    targets: ["universal"],
  });
  writeJson(path.join(workdir, `extensions/wasm/${id}/extension.json`), {
    schema_version: 1,
    id: "com.onetcli.importer.dbeaver",
    name: "DBeaver Importer",
    description: "Rust WASM connection importer for DBeaver",
    version,
    engines: { onetcli: ">=0.7.0" },
    runtime: {
      wasm: [{
        id: "dbeaver-importer",
        module: "wasm/dbeaver_importer_wasm.wasm",
        kind: "component",
      }],
    },
    contributes: {
      connectionImporters: [{
        id: "dbeaver",
        runtimeId: "dbeaver-importer",
        displayName: "DBeaver",
        outputKinds: ["database"],
        platforms: ["macos", "windows"],
      }],
    },
  });
  fs.mkdirSync(path.join(workdir, `extensions/wasm/${id}/wasm`), { recursive: true });
  fs.writeFileSync(path.join(workdir, `extensions/wasm/${id}/wasm/dbeaver_importer_wasm.wasm`), "fake wasm\n");
}

function createNativeCompositeFixture(workdir, options = {}) {
  const target = options.target || "x86_64-unknown-linux-gnu";
  const executable = target.includes("windows")
    ? "elasticsearch-provider.exe"
    : "elasticsearch-provider";
  copyScript("package-composite-extension.sh", workdir);
  copyScript("verify-composite-package.sh", workdir);
  writeJson(path.join(workdir, "extensions/composite/elasticsearch/extension.build.json"), {
    id: "elasticsearch",
    kind: "composite",
    language: "rust",
    package: "elasticsearch-provider",
    binary: "elasticsearch-provider",
    path: "extensions/composite/elasticsearch",
    targets: [target],
  });
  writeJson(path.join(workdir, "extensions/composite/elasticsearch/extension.json"), {
    schema_version: 1,
    id: "com.navop.elasticsearch",
    name: "Elasticsearch",
    version: "0.0.0",
    engines: { onetcli: ">=0.15.2", gpui_shell: "0.2.0" },
    api: { extension: "1.0", shell: "1.0" },
    permissions: ["shell:exec", "spawn:./bin/elasticsearch-provider"],
    runtime: {
      ipc: [{ id: "main", entry: { command: "./bin/elasticsearch-provider" } }],
    },
    contributes: {
      connections: [{
        id: "elasticsearch",
        label: "Elasticsearch",
        runtimeId: "main",
        resourceType: "elasticsearch",
        shellViewId: "explorer",
        form: { tabs: [] },
      }],
      shellViews: [{
        id: "explorer",
        title: "Elasticsearch",
        entry: "ui/explorer.js",
        singleton: false,
        backends: { search: "main" },
        modules: ["context", "resource"],
      }],
    },
  });
  fs.mkdirSync(path.join(workdir, "extensions/composite/elasticsearch/ui"), { recursive: true });
  fs.writeFileSync(
    path.join(workdir, "extensions/composite/elasticsearch/ui/explorer.js"),
    "export default class ElasticsearchExplorer {}\n",
  );
  fs.mkdirSync(path.join(workdir, `target/${target}/release`), { recursive: true });
  fs.writeFileSync(
    path.join(workdir, `target/${target}/release/${executable}`),
    "fake elasticsearch provider\n",
    { mode: 0o755 },
  );
}

function createStaticRemoteEditorFixture(workdir) {
  const id = "notepad-plus-plus-editor";
  copyScript("package-composite-extension.sh", workdir);
  copyScript("verify-composite-package.sh", workdir);
  writeJson(path.join(workdir, `extensions/wasm/${id}/extension.build.json`), {
    id,
    kind: "composite",
    language: "static",
    path: `extensions/wasm/${id}`,
    targets: ["universal"],
  });
  writeJson(path.join(workdir, `extensions/wasm/${id}/extension.json`), {
    schema_version: 1,
    id: "com.onetcli.editor.notepad-plus-plus",
    name: "Notepad++ External Editor",
    description: "Use Notepad++ to edit SFTP remote files from Navop.",
    version: "0.0.0",
    engines: { onetcli: ">=0.8.6" },
    contributes: {
      remoteFileEditors: [{
        id: "notepad-plus-plus",
        displayName: "Notepad++",
        platforms: ["windows"],
        fileMasks: ["*"],
        priority: 100,
        command: {
          programCandidates: [
            "${env:ProgramFiles}\\Notepad++\\notepad++.exe",
            "${env:ProgramFiles(x86)}\\Notepad++\\notepad++.exe",
          ],
          args: ["{file}"],
        },
      }],
    },
  });
}

function createLanguageExtensionFixture(workdir, options = {}) {
  const id = options.id || "rust";
  const version = options.version || "0.0.0";
  const fileExtensions = options.fileExtensions || ["rs"];
  copyScript("package-language-extension.sh", workdir);
  copyScript("verify-language-package.sh", workdir);
  writeJson(path.join(workdir, `extensions/language/${id}/extension.build.json`), {
    id,
    kind: "language",
    language: "tree-sitter-wasm",
    path: `extensions/language/${id}`,
    targets: ["universal"],
  });
  writeJson(path.join(workdir, `extensions/language/${id}/manifest.json`), {
    name: id,
    version,
    file_extensions: fileExtensions,
  });
  fs.writeFileSync(
    path.join(workdir, `extensions/language/${id}/parser.wasm`),
    "fake parser wasm\n",
  );
  fs.writeFileSync(
    path.join(workdir, `extensions/language/${id}/highlights.scm`),
    "(identifier) @variable\n",
  );
}

function createLanguageBundleFixture(workdir, options = {}) {
  const id = options.id || "tree-sitter-languages";
  const version = options.version || "0.1.0";
  const languages = options.languages || [
    { id: "rust", version: "0.24.0", fileExtensions: ["rs"] },
    { id: "javascript", version: "0.23.1", fileExtensions: ["js"] },
  ];
  copyScript("package-language-bundle-extension.sh", workdir);
  copyScript("verify-language-bundle-package.sh", workdir);
  writeJson(path.join(workdir, `extensions/language-bundle/${id}/extension.build.json`), {
    id,
    kind: "language_bundle",
    language: "tree-sitter-wasm-bundle",
    path: `extensions/language-bundle/${id}`,
    targets: ["universal"],
    releaseTagPrefix: `${id}-v`,
    r2Prefix: `extensions/${id}`,
  });
  writeJson(path.join(workdir, `extensions/language-bundle/${id}/manifest.json`), {
    id,
    name: "Tree-sitter Languages",
    version,
    languages: languages.map((language) => language.id).sort(),
  });
  for (const language of languages) {
    writeJson(path.join(workdir, `extensions/language/${language.id}/manifest.json`), {
      name: language.id,
      version: language.version,
      file_extensions: language.fileExtensions,
    });
    fs.writeFileSync(
      path.join(workdir, `extensions/language/${language.id}/parser.wasm`),
      `fake ${language.id} parser wasm\n`,
    );
    fs.writeFileSync(
      path.join(workdir, `extensions/language/${language.id}/highlights.scm`),
      `(${language.id}_node) @variable\n`,
    );
  }
}

function copyScript(name, workdir) {
  fs.mkdirSync(path.join(workdir, "scripts"), { recursive: true });
  fs.copyFileSync(path.join(repoRoot, "scripts", name), path.join(workdir, "scripts", name));
}

function writeJson(filePath, value) {
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
  fs.writeFileSync(filePath, `${JSON.stringify(value, null, 2)}\n`);
}

function createRustDriverFixture(workdir, id, binary, version) {
  writeJson(path.join(workdir, `extensions/ipc/${id}/extension.build.json`), {
    id,
    kind: "database_driver",
    language: "rust",
    package: `${id}_driver`,
    binary,
    path: `extensions/ipc/${id}`,
    targets: ["aarch64-apple-darwin"],
  });
  writeJson(path.join(workdir, `extensions/ipc/${id}/driver.json`), {
    id,
    version,
    entry: {},
  });
  fs.mkdirSync(path.join(workdir, `extensions/ipc/${id}/locales`), { recursive: true });
  fs.writeFileSync(path.join(workdir, `extensions/ipc/${id}/locales/en.yml`), `name: ${id}\n`);
  fs.mkdirSync(path.join(workdir, "target/aarch64-apple-darwin/release"), {
    recursive: true,
  });
  fs.writeFileSync(
    path.join(workdir, `target/aarch64-apple-darwin/release/${binary}`),
    `fake ${id} binary\n`,
  );
}

function createJavaDriverFixture(workdir, id, binary, version) {
  writeJson(path.join(workdir, `extensions/ipc/${id}/extension.build.json`), {
    id,
    kind: "database_driver",
    language: "java",
    package: `java/${binary}`,
    binary,
    jar: `${binary}.jar`,
    path: `extensions/ipc/${id}`,
    targets: ["universal"],
  });
  writeJson(path.join(workdir, `extensions/ipc/${id}/driver.json`), {
    id,
    version,
    entry: {},
  });
  fs.mkdirSync(path.join(workdir, `extensions/ipc/${id}/locales`), { recursive: true });
  fs.writeFileSync(path.join(workdir, `extensions/ipc/${id}/locales/en.yml`), `name: ${id}\n`);
  fs.mkdirSync(path.join(workdir, `java/${binary}/target`), { recursive: true });
  fs.mkdirSync(path.join(workdir, `java/${binary}/bin`), { recursive: true });
  fs.writeFileSync(
    path.join(workdir, `java/${binary}/target/${binary}-0.7.0-all.jar`),
    "fake shaded jar\n",
  );
  fs.writeFileSync(path.join(workdir, `java/${binary}/bin/${binary}`), "#!/usr/bin/env sh\n");
  fs.writeFileSync(path.join(workdir, `java/${binary}/bin/${binary}.cmd`), "@echo off\r\n");
}

function createFakeRustToolchain(workdir) {
  const binDir = path.join(workdir, "fake-bin");
  fs.mkdirSync(binDir, { recursive: true });
  fs.writeFileSync(
    path.join(binDir, "rustc"),
    "#!/usr/bin/env bash\nif [ \"$1\" = \"-vV\" ]; then printf 'host: aarch64-apple-darwin\\n'; else exit 1; fi\n",
    { mode: 0o755 },
  );
  fs.writeFileSync(
    path.join(binDir, "cargo"),
    "#!/usr/bin/env bash\nif [ \"$1\" = \"build\" ]; then exit 0; fi\nexit 1\n",
    { mode: 0o755 },
  );
  return binDir;
}

function createFailingRustc(workdir) {
  const binDir = path.join(workdir, "failing-rustc");
  fs.mkdirSync(binDir, { recursive: true });
  fs.writeFileSync(
    path.join(binDir, "rustc"),
    "#!/usr/bin/env bash\nprintf 'rustc should not be called for universal drivers\\n' >&2\nexit 99\n",
    { mode: 0o755 },
  );
  return binDir;
}

function collectExtensionMetadata() {
  return ["extensions/ipc", "extensions/remote-desktop", "extensions/mcp-helper", "extensions/acp-agent", "extensions/composite", "extensions/wasm"].flatMap((root) =>
    fs.existsSync(path.join(repoRoot, root)) ?
    fs
      .readdirSync(path.join(repoRoot, root))
      .map((id) => path.join(repoRoot, root, id, "extension.build.json"))
      .filter((metadataPath) => fs.existsSync(metadataPath))
      .map((metadataPath) => JSON.parse(fs.readFileSync(metadataPath, "utf8")))
      : []
  ).sort((left, right) => left.id.localeCompare(right.id));
}

function languageName(metadata) {
  const language = metadata.language || "rust";
  switch (language) {
    case "go":
      return "Go";
    case "java":
      return "Java";
    case "rust":
      return "Rust";
    case "rust-wasm":
      return "Rust WASM";
    case "shell":
      return "Shell";
    case "static":
      return "Static";
    default:
      throw new Error(`unsupported extension language for ${metadata.id}: ${language}`);
  }
}

function sourceManifestFileName(kind) {
  switch (kind) {
    case "database_driver":
      return "driver.json";
    case "remote_desktop_provider":
      return "remote_desktop_provider.json";
    case "mcp_helper":
      return "mcp_helper.json";
    case "acp_agent":
      return "acp_agent.json";
    case "composite":
      return "extension.json";
    default:
      throw new Error(`unsupported extension kind: ${kind}`);
  }
}

function collectI18nKeys(value, keys = new Set()) {
  if (Array.isArray(value)) {
    for (const item of value) collectI18nKeys(item, keys);
    return keys;
  }
  if (!value || typeof value !== "object") return keys;
  for (const [key, item] of Object.entries(value)) {
    if (key.endsWith("_i18n_key") && typeof item === "string" && item.length > 0) {
      keys.add(item);
    } else {
      collectI18nKeys(item, keys);
    }
  }
  return keys;
}

function isPackagedUiI18nKey(key) {
  return key.startsWith("database.")
    || key.startsWith("common.")
    || key.startsWith("ConnectionForm.")
    || key.startsWith("ImportExport.")
    || key.startsWith("Table.")
    || key.startsWith("View.")
    || key.startsWith("Connection.");
}

function localeDefinesKey(localeText, key) {
  if (new RegExp(`^\\s*["']?${escapeRegExp(key)}["']?\\s*:`, "m").test(localeText)) {
    return true;
  }

  let indent = -1;
  for (const part of key.split(".")) {
    const match = localeText.match(new RegExp(`^(\\s*)${escapeRegExp(part)}\\s*:`, "m"));
    if (!match) return false;
    const nextIndent = match[1].length;
    if (nextIndent <= indent) return false;
    indent = nextIndent;
  }
  return true;
}

function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function isRelativeAssetPath(value) {
  return value.includes("/") || value.includes("\\");
}

function assertHasAction(actions, driverId, actionId, nodeType) {
  assert.ok(
    actions.some((action) =>
      action.id === actionId
      && action.targets?.some((target) => target.node_type === nodeType)
    ),
    `${driverId} should expose ${actionId} for ${nodeType}`,
  );
}

function git(workdir, ...args) {
  return execFileSync(
    "git",
    ["-c", "user.name=Test User", "-c", "user.email=test@example.com", ...args],
    { cwd: workdir, encoding: "utf8" },
  );
}
