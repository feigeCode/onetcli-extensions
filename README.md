# onetcli-extensions

中文版本: [README.zh-CN.md](README.zh-CN.md)

First-party extension repository for `onetcli`.

This repository builds and publishes official extension packages independently
from the main `onetcli` application. The host app owns the extension runtime,
marketplace client, update client, and SDK/runtime contracts. This repository
owns concrete official extensions, release artifacts, the repository-maintained
marketplace manifest, and Cloudflare R2 upload automation.

## Current Contents

```text
extensions/
  ipc/
    duckdb/       Rust DuckDB IPC database driver
    iotdb/        Rust Apache IoTDB IPC database driver
    dm/           Go Dameng DM IPC database driver
    kingbase/     Go KingbaseES IPC database driver
    gbase8s/      Java GBase 8s IPC database driver
    opengauss/    Rust openGauss IPC database driver
java/
  gbase8s-ipc-driver/
internal/
  dbipc/          shared Go IPC database server runtime
manifest.json     lightweight marketplace index
scripts/
  build-go-driver.sh
  build-java-driver.sh
  changed-extensions.mjs
  generate-marketplace-manifest.mjs
  install-local-drivers.sh
  package-driver.sh
  release-driver.mjs
  verify-package.sh
tests/
  scripts.test.mjs
.codex/
  skills/ipc-driver-development/
```

The duplicated root-level `ipc-driver-development/` skill directory is not used.
Keep driver-development guidance under
`.codex/skills/ipc-driver-development/`.

## Driver Matrix

| Driver | Runtime | Package metadata | Manifest | Notes |
| --- | --- | --- | --- | --- |
| DuckDB | Rust | `extensions/ipc/duckdb/extension.build.json` | `extensions/ipc/duckdb/driver.json` | Embedded single-file database driver. |
| Apache IoTDB | Rust | `extensions/ipc/iotdb/extension.build.json` | `extensions/ipc/iotdb/driver.json` | Time-series database driver. |
| Dameng DM | Go | `extensions/ipc/dm/extension.build.json` | `extensions/ipc/dm/driver.json` | Uses shared `internal/dbipc` runtime and driver-specific build tags. |
| KingbaseES | Go | `extensions/ipc/kingbase/extension.build.json` | `extensions/ipc/kingbase/driver.json` | Uses shared `internal/dbipc` runtime and driver-specific build tags. |
| GBase 8s | Java | `extensions/ipc/gbase8s/extension.build.json` | `extensions/ipc/gbase8s/driver.json` | Uses `java/gbase8s-ipc-driver`. Preserve `java/gbase8s-ipc-driver/bin/lib/gbase8s-ipc-driver.jar` when present. |

Domestic database drivers declare `"category": "domestic_database"` in
`driver.json`; the host should use manifest metadata instead of hardcoded ids
for UI grouping.

## Protocol Surface

Each driver declares its callable methods in `driver.json.methods` and should
return the same method list from `init`. Treat this list as a runtime contract:
if a method is declared, the binary must route it or intentionally return a
typed unsupported error.

The current IPC drivers expose schema metadata through the legacy fixed methods
such as:

- `schema/databases`
- `schema/schemas`
- `schema/objects`
- `schema/columns`
- `schema/indexes`
- `schema/views`
- `schema/functions`

Drivers that customize object-list table headers also declare
`schema/object_view`. That method is connection-bound and returns the complete
render table shape:

```json
{
  "title": "Columns",
  "columns": [
    { "key": "name", "name": "Field", "width_px": 220 },
    { "key": "type", "name": "Type", "width_px": 160 },
    { "key": "nullable", "name": "Null?", "width_px": 72, "align": "right" }
  ],
  "rows": [
    ["id", "INTEGER", "false"],
    ["payload", "JSON", "true"]
  ]
}
```

If `schema/object_view` is absent or returns typed not-supported or
method-not-found for a view, the host falls back to the legacy schema methods.
Keep the first column as the object name when rows represent clickable database
objects.

## SDK Dependency

Rust drivers depend on these SDK crates from `feigeCode/onetcli`:

- `extension-protocol`
- `extension-driver`
- `extension-host`

At the moment, `Cargo.toml` points to the `dev-ipc` branch because the existing
`v0.4.8` tag does not contain those crates. After `onetcli` publishes a release
tag that includes the SDK crates, replace the branch dependencies with that
fixed tag.

## Local Development

Run script and packaging tests:

```bash
node --test tests/scripts.test.mjs
```

Run Rust driver tests:

```bash
cargo test -p duckdb_driver -- --nocapture
cargo test -p iotdb_driver -- --nocapture
```

Run Go runtime tests:

```bash
GOCACHE=/private/tmp/onetcli-go-cache go test ./internal/dbipc
```

Run Java driver tests:

```bash
mvn -f java/gbase8s-ipc-driver/pom.xml test
```

Check Rust formatting:

```bash
cargo fmt --all --check
```

Validate GitHub Actions YAML:

```bash
ruby -e 'require "yaml"; YAML.load_file(".github/workflows/ci.yml"); YAML.load_file(".github/workflows/release.yml"); YAML.load_file(".github/workflows/upload-r2.yml"); puts "workflow yaml ok"'
```

## Build And Package

All extension packages are described by
`extensions/ipc/<driver-id>/extension.build.json`. The build metadata defines
the extension id, runtime language, package or binary name, target triples,
release tag prefix, and R2 prefix.

Build and package DuckDB for the local host target:

```bash
HOST_TRIPLE="$(rustc -vV | sed -n 's/^host: //p')"
cargo build --release -p duckdb_driver --target "$HOST_TRIPLE"
mkdir -p artifacts
bash scripts/package-driver.sh duckdb "$HOST_TRIPLE" artifacts 1.0.0
bash scripts/verify-package.sh "artifacts/duckdb-driver-${HOST_TRIPLE}.tar.gz"
```

Build and package a Go driver:

```bash
HOST_TRIPLE="$(rustc -vV | sed -n 's/^host: //p')"
bash scripts/build-go-driver.sh dm "$HOST_TRIPLE"
mkdir -p artifacts
bash scripts/package-driver.sh dm "$HOST_TRIPLE" artifacts 0.1.0
```

Build and package the Java GBase 8s driver:

```bash
HOST_TRIPLE="$(rustc -vV | sed -n 's/^host: //p')"
bash scripts/build-java-driver.sh gbase8s "$HOST_TRIPLE"
mkdir -p artifacts
bash scripts/package-driver.sh gbase8s "$HOST_TRIPLE" artifacts 0.1.0
```

Package archives contain the extension directory with `driver.json`, the entry
binary or launcher, and packaged resources such as locales, icons, and runtime
libraries.

Build, package, verify, and replace installed local drivers:

```bash
bash scripts/install-local-drivers.sh
bash scripts/install-local-drivers.sh dm
```

By default this installs into
`$XDG_CONFIG_HOME/one-hub/extensions/database_drivers` or
`$HOME/.config/one-hub/extensions/database_drivers`. Override the target with
`ONETCLI_DATABASE_DRIVER_DIR=/path/to/database_drivers`.

Prepare release artifacts for one driver locally:

```bash
node scripts/release-driver.mjs duckdb 1.0.0
node scripts/release-driver.mjs dm 0.4.0 --target x86_64-unknown-linux-gnu
node scripts/release-driver.mjs gbase8s 0.7.0 --artifact-dir artifacts/gbase8s-0.7.0
```

The release script reads `extensions/ipc/<driver-id>/extension.build.json`,
builds each selected target with the runtime-specific build command, packages
and verifies each archive, then writes:

- `artifacts/<driver-id>-driver-<target>.tar.gz`
- `artifacts/sha256sums.txt`
- `artifacts/extension-manifest.json`
- `artifacts/release-metadata.json`

Use `--skip-build` when binaries have already been staged under
`target/<target>/release`.

## Marketplace Manifest

The repository root `manifest.json` is the global marketplace index. It is
maintained and committed directly in this repository, then uploaded unchanged to
R2 as `extensions/manifest.json`.

Release jobs generate one plugin manifest:

- `artifacts/extension-manifest.json`: the current extension manifest published
  to that extension's GitHub Release. It contains target artifact file names and
  checksums.

The plugin manifest is generated from:

- package filenames
- `artifacts/sha256sums.txt`
- release environment variables

Required environment variables:

```text
ARTIFACT_DIR=artifacts
EXTENSION_VERSION=1.0.0
EXTENSION_ID=duckdb
RELEASE_TAG=duckdb-v1.0.0
```

The extension-scoped GitHub Release keeps `extension-manifest.json` as the
current extension's plugin manifest. After the Release workflow succeeds, the
upload workflow serializes marketplace publication, uploads that plugin manifest
to R2 at `extensions/<id>/manifest.json`, and uploads the committed root
`manifest.json` to R2 at `extensions/manifest.json` with `no-cache`.

The global marketplace entry is schema v2 and contains metadata plus a manifest
path, not artifact files or download URLs:

```json
{
  "id": "duckdb",
  "kind": "database_driver",
  "name": "DuckDB",
  "version": "1.0.0",
  "release_tag": "duckdb-v1.0.0",
  "description": "DuckDB embedded analytical database IPC driver",
  "file_extensions": [],
  "manifest": "duckdb/manifest.json"
}
```

The plugin manifest is also schema v2 and contains artifact file names plus
checksums, not full download URLs:

```json
{
  "schema_version": 2,
  "release_version": "duckdb-v1.0.0",
  "extensions": [{
    "id": "duckdb",
    "kind": "database_driver",
    "name": "DuckDB",
    "version": "1.0.0",
    "release_tag": "duckdb-v1.0.0",
    "artifacts": {
      "x86_64-unknown-linux-gnu": {
        "file": "duckdb-driver-x86_64-unknown-linux-gnu.tar.gz",
        "sha256": "<sha256>"
      }
    }
  }]
}
```

The `onetcli` client owns download source policy. It first loads the global
marketplace index, then loads the selected extension's plugin manifest. For R2,
package URLs are resolved from the plugin manifest directory using
`<version>/<file>`. If the R2 plugin manifest or package is unavailable, the
client derives GitHub Release fallback URLs from its configured GitHub manifest
base, the entry's `release_tag`, and the plugin manifest or artifact file name.

## CI

`.github/workflows/ci.yml` detects changed extensions and builds only affected
release units.

Current selection rules:

- Changes under `extensions/ipc/<driver-id>/**` build that driver.
- Changes under shared runtime, scripts, workflow, or packaging paths build all
  known extensions.
- Each target triple is one matrix entry.

## Release

Extension releases are extension-scoped:

```bash
git tag duckdb-v1.0.0
git push origin duckdb-v1.0.0
```

The Release workflow:

1. Resolves the extension id and version from the tag.
2. Builds every target listed in `extension.build.json`.
3. Packages and verifies each archive.
4. Generates checksums.
5. Generates the current extension plugin manifest.
6. Publishes a GitHub Release with packages, checksums, and the current
   extension `extension-manifest.json`.

Manual release is also available through `workflow_dispatch` with:

- `extension`, for example `duckdb`
- `version`, for example `1.0.0`

## R2 Upload

`.github/workflows/upload-r2.yml` runs after a successful Release workflow or
can be triggered manually with a release tag.

Repository secrets:

```text
CLOUDFLARE_ACCOUNT_ID
CLOUDFLARE_R2_ACCESS_KEY_ID
CLOUDFLARE_R2_SECRET_ACCESS_KEY
CLOUDFLARE_R2_BUCKET
```

The upload workflow is serialized with the `extension-marketplace-publish`
concurrency group. For DuckDB `1.0.0`, R2 receives:

```text
extensions/duckdb/1.0.0/<package>.tar.gz
extensions/duckdb/manifest.json
extensions/manifest.json
```

Versioned packages are uploaded with immutable caching. Plugin manifests and the
global marketplace index are uploaded with `no-cache`. The global manifest is
the repository-maintained root `manifest.json`, uploaded unchanged to
`extensions/manifest.json`.

## Adding Another IPC Driver

Add a new directory under `extensions/ipc/<driver-id>` with:

```text
driver.json
extension.build.json
locales/
icons/
```

Runtime-specific code lives in the appropriate local package:

- Rust drivers usually live under `extensions/ipc/<driver-id>/src` and are root
  Cargo workspace members.
- Go drivers can reuse `internal/dbipc` and add a command under `cmd/`.
- Java drivers can use a package under `java/`.

Create metadata similar to:

```json
{
  "id": "postgres",
  "kind": "database_driver",
  "language": "go",
  "package": "./cmd/postgres-ipc-driver",
  "binary": "postgres-ipc-driver",
  "path": "extensions/ipc/postgres",
  "targets": [
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
    "x86_64-pc-windows-msvc"
  ],
  "releaseTagPrefix": "postgres-v",
  "r2Prefix": "extensions/postgres"
}
```

No workflow changes should be needed for another IPC database driver if it uses
the existing metadata and package shape.

## Host App Integration

The main `onetcli` repository should consume the published global marketplace
manifest from R2 first. Each global entry points to an extension plugin manifest
such as `duckdb/manifest.json`; the host loads that file before selecting a
platform artifact. GitHub fallback is extension-scoped: the host derives
`https://github.com/feigeCode/onetcli-extensions/releases/download/<release_tag>/extension-manifest.json`
for the plugin manifest, then derives package fallback URLs from the same
release tag and artifact file name.

Do not make the main application release depend on this repository's extension
builds. The main app owns runtime consumption; this repository owns extension
production and publication.
