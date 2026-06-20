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

  assert.deepEqual(ids, ["dm", "kingbase", "oracle"]);
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

  for (const id of ["dm", "kingbase", "oracle"]) {
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
  ];

  for (const id of ["dm", "kingbase", "oracle"]) {
    const metadata = JSON.parse(
      fs.readFileSync(path.join(repoRoot, "extensions/ipc", id, "extension.build.json"), "utf8"),
    );
    assert.equal(metadata.language, "go");
    assert.deepEqual(metadata.targets, expectedTargets, `${id} target list drifted`);
  }
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

  assert.deepEqual(domesticIds, ["dm", "gbase8s", "kingbase", "opengauss"]);
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
        os: "ubuntu-latest",
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
      kind: "database_driver",
      language: "go",
      os: "ubuntu-latest",
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
      kind: "database_driver",
      language: "java",
      os: "ubuntu-latest",
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
  const ids = fs
    .readdirSync(path.join(repoRoot, "extensions/ipc"))
    .filter((id) =>
      fs.existsSync(path.join(repoRoot, "extensions/ipc", id, "extension.build.json")),
    )
    .sort();

  assert.equal(manifest.schema_version, 2);
  assert.deepEqual(
    manifest.extensions.map((entry) => entry.id).sort(),
    ids,
  );

  for (const entry of manifest.extensions) {
    const driverJson = JSON.parse(
      fs.readFileSync(path.join(repoRoot, "extensions/ipc", entry.id, "driver.json"), "utf8"),
    );
    assert.equal(entry.kind, "database_driver");
    assert.equal(entry.name, driverJson.name || entry.id);
    assert.equal(entry.version, driverJson.version);
    assert.equal(entry.release_tag, `${entry.id}-v${entry.version}`);
    assert.equal(entry.manifest, `${entry.id}/manifest.json`);
    assert.equal(Object.hasOwn(entry, "artifacts"), false);
    assert.equal(Object.hasOwn(entry, "asset_urls"), false);
    assert.equal(Object.hasOwn(entry, "fallback_asset_urls"), false);
    assert.equal(Object.hasOwn(entry, "sha256s"), false);
  }
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
});

test("CI workflow routes Rust, Go, and Java extension jobs by language", () => {
  const workflow = fs.readFileSync(path.join(repoRoot, ".github/workflows/ci.yml"), "utf8");
  const releaseWorkflow = fs.readFileSync(path.join(repoRoot, ".github/workflows/release.yml"), "utf8");

  assert.match(workflow, /name: Repository checks/);
  assert.match(workflow, /node --test tests\/scripts\.test\.mjs/);
  assert.match(workflow, /Validate workflow YAML/);
  assert.match(workflow, /matrix\.language == 'rust'/);
  assert.match(workflow, /matrix\.language == 'go'/);
  assert.match(workflow, /matrix\.language == 'java'/);
  assert.match(workflow, /actions\/setup-go@v5/);
  assert.match(workflow, /actions\/setup-java@v4/);
  assert.match(workflow, /run: cargo test -p \$\{\{ matrix\.package \}\} -- --nocapture/);
  assert.match(workflow, /run: go test \.\/\.\.\./);
  assert.match(workflow, /run: mvn -f "\$\{\{ matrix\.package \}\}\/pom\.xml" test/);
  assert.doesNotMatch(workflow, /name: Package/);
  assert.doesNotMatch(workflow, /cargo build --release/);
  assert.doesNotMatch(workflow, /scripts\/build-go-driver\.sh/);
  assert.doesNotMatch(workflow, /scripts\/build-java-driver\.sh/);
  assert.doesNotMatch(workflow, /scripts\/package-driver\.sh/);
  assert.doesNotMatch(workflow, /scripts\/verify-package\.sh/);
  assert.doesNotMatch(workflow, /aarch64-unknown-linux-gnu/);
  assert.match(releaseWorkflow, /if: \$\{\{ matrix\.language == 'java' \}\}\n\s+run: bash scripts\/build-java-driver\.sh/);
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
  const installRoot = path.join(workdir, "onetcli/extensions/database_drivers");
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
        ONETCLI_DATABASE_DRIVER_DIR: installRoot,
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

test("install-local-drivers installs all local drivers when no id is passed", () => {
  const workdir = makeTempDir();
  copyScript("install-local-drivers.sh", workdir);
  copyScript("package-driver.sh", workdir);
  copyScript("verify-package.sh", workdir);
  createRustDriverFixture(workdir, "duckdb", "duckdb_driver", "0.9.0");
  createRustDriverFixture(workdir, "iotdb", "iotdb_driver", "0.8.0");
  const installRoot = path.join(workdir, "onetcli/extensions/database_drivers");

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
  const installRoot = path.join(workdir, "onetcli/extensions/database_drivers");

  const output = execFileSync(
    "bash",
    [path.join(workdir, "scripts/install-local-drivers.sh"), "gbase8s"],
    {
      cwd: workdir,
      encoding: "utf8",
      env: {
        ...process.env,
        ONETCLI_DATABASE_DRIVER_DIR: installRoot,
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
    path: `extensions/ipc/${id}`,
    targets: ["x86_64-unknown-linux-gnu"],
    ...options.metadata,
  });
  writeJson(path.join(workdir, `extensions/ipc/${id}/driver.json`), options.driverJson || {
    id,
    version: "0.0.0",
    entry: {},
  });
  fs.mkdirSync(path.join(workdir, `extensions/ipc/${id}/locales`), { recursive: true });
  fs.writeFileSync(path.join(workdir, `extensions/ipc/${id}/locales/en.yml`), `name: ${id}\n`);
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
