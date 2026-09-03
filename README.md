# navop-extensions

中文版本: [README.zh-CN.md](README.zh-CN.md)

First-party extension repository for `Navop`.

This repository builds and publishes official extension packages independently
from the main `Navop` application. The host app owns the extension runtime,
marketplace client, update client, and SDK/runtime contracts. This repository
owns concrete official extensions, release artifacts, the repository-maintained
marketplace manifest, and Cloudflare R2 upload automation.

## Current Contents

```text
extensions/
  ipc/
    duckdb/       Rust DuckDB IPC database driver
    redis/        Rust Redis native IPC sidecar driver
    mongodb-modern/ Rust MongoDB 4.2+ native IPC sidecar manifest
    mongodb-legacy/ Rust MongoDB 3.6 native IPC sidecar manifest
    mongodb-legacy-3-2/ Rust MongoDB 3.2–3.4 EOL native IPC sidecar manifest
    iotdb/        Go Apache IoTDB IPC database driver
    dm/           Go Dameng DM IPC database driver
    kingbase/     Go KingbaseES IPC database driver
    gbase8s/      Java GBase 8s IPC database driver
    oceanbase/    Go OceanBase IPC database driver
    opengauss/    Rust openGauss IPC database driver
    oracle-go/    Go Oracle IPC database driver (pure Go, godror)
  remote-desktop/
    rdp/          Rust RDP remote desktop provider
    rdp-helper/   Rust RDP helper binary (Cargo workspace)
    vnc/          Rust VNC remote desktop provider
    vnc-helper/   Rust VNC helper binary (Cargo workspace)
cmd/
  dm-ipc-driver/
  iotdb-ipc-driver/
  kingbase-ipc-driver/
  oceanbase-ipc-driver/
  oracle-go-ipc-driver/
java/
  gbase8s-ipc-driver/
internal/
  dbipc/          shared Go IPC database server runtime
  drivers/        driver-specific Go implementations (dm, iotdb, kingbase, oceanbase, oracle)
  ipc/            Go IPC framing and socket utilities
  runner/         Go IPC process runner
manifest.json     lightweight marketplace index
scripts/
  build-go-driver.sh
  build-java-driver.sh
  changed-extensions.mjs
  generate-marketplace-manifest.mjs
  install-local-drivers.sh
  install-local-remote-desktop-providers.sh
  install-local-composite-extensions.sh
  package-driver.sh
  package-remote-desktop-provider.sh
  release-driver.mjs
  verify-package.sh
  verify-remote-desktop-provider-package.sh
tests/
  scripts.test.mjs
docs/
  superpowers/plans/  implementation plans used by agentic development workflows
.codex/
  skills/ipc-driver-development/
```

The duplicated root-level `ipc-driver-development/` skill directory is not used.
Keep driver-development guidance under
`.codex/skills/ipc-driver-development/`.

Implementation plans under `docs/superpowers/plans/` capture approved,
task-oriented work plans for extension changes. Treat them as development
records: they may describe local findings, verification steps, and follow-up
implementation tasks, but they are not extension runtime inputs.

## Database Driver Matrix

| Driver | Runtime | Package metadata | Manifest | Notes |
| --- | --- | --- | --- | --- |
| DuckDB | Rust | `extensions/ipc/duckdb/extension.build.json` | `extensions/ipc/duckdb/driver.json` | Embedded single-file analytical database driver. Cargo workspace member. |
| Apache IoTDB | Go | `extensions/ipc/iotdb/extension.build.json` | `extensions/ipc/iotdb/driver.json` | Time-series database driver. Uses `cmd/iotdb-ipc-driver` and `internal/drivers/iotdb`. |
| Dameng DM | Go | `extensions/ipc/dm/extension.build.json` | `extensions/ipc/dm/driver.json` | Uses shared `internal/dbipc` runtime and `dm_driver` build tag. |
| KingbaseES | Go | `extensions/ipc/kingbase/extension.build.json` | `extensions/ipc/kingbase/driver.json` | Uses shared `internal/dbipc` runtime and `kingbase_driver` build tag. |
| GBase 8s | Java | `extensions/ipc/gbase8s/extension.build.json` | `extensions/ipc/gbase8s/driver.json` | Uses `java/gbase8s-ipc-driver`. Preserve `java/gbase8s-ipc-driver/bin/lib/gbase8s-ipc-driver.jar` when present. Universal (cross-platform) target only. |
| OceanBase | Go | `extensions/ipc/oceanbase/extension.build.json` | `extensions/ipc/oceanbase/driver.json` | Uses shared `internal/dbipc` runtime and `oceanbase_driver` build tag. |
| openGauss | Rust | `extensions/ipc/opengauss/extension.build.json` | `extensions/ipc/opengauss/driver.json` | Cargo workspace member. Uses `tokio-opengauss` async driver. |
| Oracle Go | Go | `extensions/ipc/oracle-go/extension.build.json` | `extensions/ipc/oracle-go/driver.json` | Pure Go Oracle driver using `oracle_go_driver` build tag. |
| Redis | Rust | `extensions/ipc/redis/extension.build.json` | `extensions/ipc/redis/driver.json` | Native API sidecar with binary-safe command/pipeline, blob, and bounded Pub/Sub support. |
| MongoDB Modern | Rust | `extensions/ipc/mongodb-modern/extension.build.json` | `extensions/ipc/mongodb-modern/driver.json` | Shares the IPC implementation package with the legacy variant, uses MongoDB Rust Driver 3.8, and targets MongoDB 4.2+. |
| MongoDB Legacy | Rust | `extensions/ipc/mongodb-legacy/extension.build.json` | `extensions/ipc/mongodb-legacy/driver.json` | Shares the IPC implementation package with the modern variant, uses MongoDB Rust Driver 2.8, and targets MongoDB 3.6. |
| MongoDB Legacy 3.2 | Rust | `extensions/ipc/mongodb-legacy-3-2/extension.build.json` | `extensions/ipc/mongodb-legacy-3-2/driver.json` | Uses the MongoDB Rust Driver 0.3 wire implementation for EOL MongoDB 3.2–3.4 servers; SRV and TLS are intentionally unsupported. |

Domestic database drivers declare `"category": "domestic_database"` in
`driver.json`; the host should use manifest metadata instead of hardcoded ids
for UI grouping.

## Remote Desktop Provider Matrix
| Provider | Runtime | Package metadata | Manifest | Notes |
| --- | --- | --- | --- | --- |
| RDP | Rust | `extensions/remote-desktop/rdp/extension.build.json` | `extensions/remote-desktop/rdp/remote_desktop_provider.json` | RDP remote desktop provider. Binary built from `extensions/remote-desktop/rdp-helper`. |
| VNC | Rust | `extensions/remote-desktop/vnc/extension.build.json` | `extensions/remote-desktop/vnc/remote_desktop_provider.json` | VNC remote desktop provider. Binary built from `extensions/remote-desktop/vnc-helper`. |

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

Rust drivers depend on these SDK crates from `feigeCode/navop`:

- `extension-protocol`
- `extension-driver`
- `extension-host`

At the moment, `Cargo.toml` points to the `dev` branch because the existing
`v0.4.8` tag does not contain those crates. After `Navop` publishes a release
tag that includes the SDK crates, replace the branch dependencies with that
fixed tag.

The Cargo workspace currently includes `extensions/ipc/duckdb`,
`extensions/ipc/opengauss`, `extensions/ipc/redis`, and the shared
`drivers/mongodb-driver` crate. The RDP and VNC helpers are independent Cargo
projects under `extensions/remote-desktop/rdp-helper` and
`extensions/remote-desktop/vnc-helper` respectively.

## Local Development

Run script and packaging tests:

```bash
node --test tests/scripts.test.mjs
```

Run Rust driver tests:

```bash
cargo test -p duckdb_driver -- --nocapture
cargo test -p opengauss_driver -- --nocapture
```

Run Go runtime tests:

```bash
GOCACHE=/private/tmp/navop-go-cache go test ./internal/dbipc
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
ruby -e 'require "yaml"; YAML.load_file(".github/workflows/ci.yml"); YAML.load_file(".github/workflows/release.yml"); YAML.load_file(".github/workflows/upload-r2.yml"); YAML.load_file(".github/workflows/backfill-windows-x86.yml"); YAML.load_file(".github/workflows/sync-cnb-release-assets.yml"); YAML.load_file(".github/workflows/sync-cnb-latest.yml"); puts "workflow yaml ok"'
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

Build and package a Rust remote desktop provider:

```bash
HOST_TRIPLE="$(rustc -vV | sed -n 's/^host: //p')"
bash scripts/package-remote-desktop-provider.sh rdp "$HOST_TRIPLE" artifacts 0.1.0
bash scripts/verify-remote-desktop-provider-package.sh "artifacts/rdp-remote-desktop-provider-${HOST_TRIPLE}.tar.gz"
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
`NAVOP_DATABASE_DRIVER_DIR=/path/to/database_drivers`. The legacy
`ONETCLI_DATABASE_DRIVER_DIR` variable remains supported for compatibility.

Install remote desktop providers locally:

```bash
bash scripts/install-local-remote-desktop-providers.sh
bash scripts/install-local-remote-desktop-providers.sh rdp
```

Build, package, verify, and install a native or universal composite extension:

```bash
bash scripts/install-local-composite-extensions.sh elasticsearch
```

Native composites publish one archive per target. Their root `extension.json`
may combine `runtime.ipc` with `contributes.shellViews`; packages contain the
platform provider under `bin/` and gpui-shell sources under `ui/`.

By default this follows Navop's config-directory migration state. On Unix it
uses `$HOME/.config`; on Windows it uses `%APPDATA%`. Within that config root it
installs below
`navop/extensions/remote_desktop_providers`, while continuing to use the legacy
`one-hub` directory if it still exists and Navop has not written the
`.one-hub-migration-complete` marker. Override the target with
`NAVOP_REMOTE_DESKTOP_PROVIDER_DIR=/path/to/remote_desktop_providers`; the legacy
`ONETCLI_REMOTE_DESKTOP_PROVIDER_DIR` variable remains supported.

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

The `Navop` client owns download source policy. It first loads the global
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

## CNB Sync

`.github/workflows/sync-cnb-release-assets.yml` mirrors a release's Git tags and
assets to the CNB mirror `navop-dev/navop-extensions`. The Release workflow
dispatches it for every new release, and `.github/workflows/sync-cnb-latest.yml`
reconciles the latest stable release of every extension on a daily schedule
(plus `workflow_dispatch`) so the Navop website can serve extension downloads
from CNB:

```text
https://cnb.cool/navop-dev/navop-extensions/-/releases/download/<release-tag>/<asset>
```

The latest-version workflow:

1. Runs `scripts/discover-latest-extension-releases.mjs`, which reads every
   `extension.build.json` `releaseTagPrefix` and resolves the newest stable
   GitHub release per extension into a build matrix.
2. Calls `sync-cnb-release-assets.yml` once per extension with
   `tag: <extension>-v<version>` and `secrets: inherit`.

CNB synchronization requires the `CNB_TOKEN` repository secret.

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
    "x86_64-pc-windows-msvc",
    "i686-pc-windows-msvc"
  ],
  "releaseTagPrefix": "postgres-v",
  "r2Prefix": "extensions/postgres"
}
```

No workflow changes should be needed for another IPC database driver if it uses
the existing metadata and package shape.

## Adding Another Remote Desktop Provider

Add a new directory under `extensions/remote-desktop/<provider-id>` with:

```text
remote_desktop_provider.json
extension.build.json
```

The helper binary is a Rust Cargo project under
`extensions/remote-desktop/<provider-id>-helper`. The `extension.build.json`
references the helper's `Cargo.toml` via `manifest_path` and lists the helper
source directory in `source_paths` so that CI change detection works correctly.

## Host App Integration

The main `Navop` repository should consume the published global marketplace
manifest from R2 first. Each global entry points to an extension plugin manifest
such as `duckdb/manifest.json`; the host loads that file before selecting a
platform artifact. GitHub fallback is extension-scoped: the host derives
`https://github.com/feigeCode/navop-extensions/releases/download/<release_tag>/extension-manifest.json`
for the plugin manifest, then derives package fallback URLs from the same
release tag and artifact file name.

Do not make the main application release depend on this repository's extension
builds. The main app owns runtime consumption; this repository owns extension
production and publication.
