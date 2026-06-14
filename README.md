# onetcli-extensions

中文版本: [README.zh-CN.md](README.zh-CN.md)

First-party extension repository for onetcli.

This repository builds and publishes extension packages independently from the
main `onetcli` application. The host app keeps the extension runtime,
marketplace client, update client, and SDK/runtime contracts. This repository
owns concrete official extensions, release artifacts, marketplace manifest
entries, and Cloudflare R2 upload automation.

## Current Contents

```text
extensions/
  ipc/
    duckdb/
      extension.build.json
      driver.json
      locales/
      src/
manifest/
  entries/
scripts/
  changed-extensions.mjs
  generate-marketplace-manifest.mjs
  package-driver.sh
  verify-package.sh
tests/
  scripts.test.mjs
```

The first extension is the DuckDB IPC database driver at
`extensions/ipc/duckdb`.

## SDK Dependency

The DuckDB driver depends on these SDK crates from `feigeCode/onetcli`:

- `extension-protocol`
- `extension-driver`
- `extension-host`

At the moment, `Cargo.toml` points to the `dev-ipc` branch because the existing
`v0.4.8` tag does not contain those crates. After `onetcli` publishes a release
tag that includes the SDK crates, replace the branch dependencies with that
fixed tag.

## Local Development

Run all script tests:

```bash
node --test tests/scripts.test.mjs
```

Run DuckDB driver tests:

```bash
cargo test -p duckdb_driver -- --nocapture
```

Check formatting:

```bash
cargo fmt --all --check
```

Validate GitHub Actions YAML:

```bash
ruby -e 'require "yaml"; YAML.load_file(".github/workflows/ci.yml"); YAML.load_file(".github/workflows/release.yml"); YAML.load_file(".github/workflows/upload-r2.yml"); puts "workflow yaml ok"'
```

## Build And Package DuckDB

Build the package for the local host target:

```bash
HOST_TRIPLE="$(rustc -vV | sed -n 's/^host: //p')"
cargo build --release -p duckdb_driver --target "$HOST_TRIPLE"
mkdir -p artifacts
bash scripts/package-driver.sh duckdb "$HOST_TRIPLE" artifacts 1.0.0
bash scripts/verify-package.sh "artifacts/duckdb-driver-${HOST_TRIPLE}.tar.gz"
```

The package archive contains:

```text
duckdb/
  driver.json
  duckdb_driver
  locales/
```

On Windows, the binary entry is `duckdb_driver.exe`.

## Marketplace Manifest

Release jobs generate `artifacts/extension-manifest.json` from:

- package filenames
- `artifacts/sha256sums.txt`
- `manifest/entries/*.json`
- release environment variables

Required environment variables:

```text
ARTIFACT_DIR=artifacts
EXTENSION_VERSION=1.0.0
EXTENSION_ID=duckdb
RELEASE_TAG=duckdb-v1.0.0
GITHUB_REPOSITORY=feigeCode/onetcli-extensions
```

The manifest includes relative primary R2 asset URLs and absolute GitHub
Release fallback URLs. Because the R2 manifest is published at
`/extensions/manifest.json`, a DuckDB primary package path is written as:

```text
duckdb/1.0.0/duckdb-driver-x86_64-unknown-linux-gnu.tar.gz
```

The `onetcli` client resolves that path against the manifest directory.

## CI

`.github/workflows/ci.yml` detects changed extensions and builds only affected
release units.

Current selection rules:

- Changes under `extensions/ipc/duckdb/**` build DuckDB only.
- Changes under `scripts/**`, `crates/**`, or `.github/workflows/**` build all
  known extensions.
- Each target triple is one matrix entry.

## Release

Extension releases are extension-scoped. For DuckDB:

```bash
git tag duckdb-v1.0.0
git push origin duckdb-v1.0.0
```

The Release workflow:

1. Resolves the extension id and version from the tag.
2. Builds every target listed in `extension.build.json`.
3. Packages and verifies each archive.
4. Generates checksums.
5. Generates `extension-manifest.json`.
6. Publishes a GitHub Release with packages, checksums, and manifest.

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

For DuckDB `1.0.0`, R2 receives:

```text
extensions/duckdb/1.0.0/<package>.tar.gz
extensions/duckdb/latest/<package>.tar.gz
extensions/manifest.json
```

Versioned packages are uploaded with immutable caching. The global manifest is
uploaded with `no-cache`.

## Adding Another IPC Driver

Add a new directory under `extensions/ipc/<driver-id>` with:

```text
Cargo.toml
driver.json
extension.build.json
locales/
src/
```

Add the package to the root workspace members and create metadata similar to:

```json
{
  "id": "postgres",
  "kind": "database_driver",
  "package": "postgres_driver",
  "binary": "postgres_driver",
  "path": "extensions/ipc/postgres",
  "targets": [
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "x86_64-unknown-linux-gnu",
    "x86_64-pc-windows-msvc"
  ],
  "releaseTagPrefix": "postgres-v",
  "r2Prefix": "extensions/postgres"
}
```

No workflow changes should be needed for another IPC database driver if it uses
the same package shape.

## Host App Integration

The main `onetcli` repository should consume the published marketplace manifest
from R2 first and use GitHub Releases in this repository as fallback:

```text
https://github.com/feigeCode/onetcli-extensions/releases/latest/download/extension-manifest.json
```

Do not make the main application release depend on this repository's extension
builds. The main app owns runtime consumption; this repository owns extension
production and publication.
