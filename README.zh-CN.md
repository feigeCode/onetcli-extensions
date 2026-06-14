# onetcli-extensions

English version: [README.md](README.md)

`onetcli` 官方一方扩展仓库。

本仓库用于独立构建和发布扩展包，不跟随主应用 `onetcli` 的发布流程。主应用继续负责扩展运行时、扩展市场客户端、更新客户端以及 SDK/运行时协议；本仓库负责官方扩展源码、发布产物、扩展市场 manifest 条目和 Cloudflare R2 上传自动化。

## 当前内容

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

当前第一个扩展是 DuckDB IPC 数据库驱动，位于 `extensions/ipc/duckdb`。

## SDK 依赖

DuckDB 驱动依赖 `feigeCode/onetcli` 中的这些 SDK crates：

- `extension-protocol`
- `extension-driver`
- `extension-host`

目前 `Cargo.toml` 指向 `dev-ipc` 分支，因为现有 `v0.4.8` tag 还不包含这些 crates。等 `onetcli` 发布包含 SDK crates 的正式 release tag 后，应将这些分支依赖替换为固定 tag 依赖。

## 本地开发

运行脚本测试：

```bash
node --test tests/scripts.test.mjs
```

运行 DuckDB 驱动测试：

```bash
cargo test -p duckdb_driver -- --nocapture
```

检查 Rust 格式：

```bash
cargo fmt --all --check
```

校验 GitHub Actions YAML：

```bash
ruby -e 'require "yaml"; YAML.load_file(".github/workflows/ci.yml"); YAML.load_file(".github/workflows/release.yml"); YAML.load_file(".github/workflows/upload-r2.yml"); puts "workflow yaml ok"'
```

## 构建和打包 DuckDB

为当前本机 target 构建扩展包：

```bash
HOST_TRIPLE="$(rustc -vV | sed -n 's/^host: //p')"
cargo build --release -p duckdb_driver --target "$HOST_TRIPLE"
mkdir -p artifacts
bash scripts/package-driver.sh duckdb "$HOST_TRIPLE" artifacts 1.0.0
bash scripts/verify-package.sh "artifacts/duckdb-driver-${HOST_TRIPLE}.tar.gz"
```

扩展包内容：

```text
duckdb/
  driver.json
  duckdb_driver
  locales/
```

Windows 平台的二进制入口是 `duckdb_driver.exe`。

## 扩展市场 Manifest

Release job 会根据以下输入生成 `artifacts/extension-manifest.json`：

- 扩展包文件名
- `artifacts/sha256sums.txt`
- `manifest/entries/*.json`
- release 环境变量

必需环境变量：

```text
ARTIFACT_DIR=artifacts
EXTENSION_VERSION=1.0.0
EXTENSION_ID=duckdb
RELEASE_TAG=duckdb-v1.0.0
GITHUB_REPOSITORY=feigeCode/onetcli-extensions
```

manifest 中的主 R2 下载地址使用相对路径，GitHub Release fallback 下载地址使用绝对 URL。因为 R2 manifest 发布在 `/extensions/manifest.json`，DuckDB 主扩展包路径会写成：

```text
duckdb/1.0.0/duckdb-driver-x86_64-unknown-linux-gnu.tar.gz
```

`onetcli` 客户端会将这个相对路径按 manifest 所在目录解析。

## CI

`.github/workflows/ci.yml` 会检测发生变化的扩展，并只构建受影响的发布单元。

当前选择规则：

- 只修改 `extensions/ipc/duckdb/**` 时，只构建 DuckDB。
- 修改 `scripts/**`、`crates/**` 或 `.github/workflows/**` 时，构建所有已知扩展。
- 每个 target triple 对应一个 matrix entry。

## Release

扩展发布使用扩展级 tag。DuckDB 示例：

```bash
git tag duckdb-v1.0.0
git push origin duckdb-v1.0.0
```

Release workflow 会执行：

1. 从 tag 解析扩展 id 和版本。
2. 构建 `extension.build.json` 中列出的所有 target。
3. 打包并校验每个 archive。
4. 生成 checksum。
5. 生成 `extension-manifest.json`。
6. 发布包含扩展包、checksum 和 manifest 的 GitHub Release。

也可以通过 `workflow_dispatch` 手动发布，参数包括：

- `extension`，例如 `duckdb`
- `version`，例如 `1.0.0`

## R2 上传

`.github/workflows/upload-r2.yml` 会在 Release workflow 成功后运行，也可以用 release tag 手动触发。

仓库 secrets：

```text
CLOUDFLARE_ACCOUNT_ID
CLOUDFLARE_R2_ACCESS_KEY_ID
CLOUDFLARE_R2_SECRET_ACCESS_KEY
CLOUDFLARE_R2_BUCKET
```

以 DuckDB `1.0.0` 为例，R2 会收到：

```text
extensions/duckdb/1.0.0/<package>.tar.gz
extensions/duckdb/latest/<package>.tar.gz
extensions/manifest.json
```

版本化扩展包使用 immutable cache。全局 manifest 使用 `no-cache`。

## 新增另一个 IPC 驱动

在 `extensions/ipc/<driver-id>` 下新增目录：

```text
Cargo.toml
driver.json
extension.build.json
locales/
src/
```

将包加入根 workspace members，并创建类似的元数据：

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

如果新 IPC 数据库驱动使用相同的包结构，通常不需要修改 workflow。

## 主应用集成

主仓库 `onetcli` 应优先从 R2 消费已发布的扩展市场 manifest，并将本仓库的 GitHub Release 作为 fallback：

```text
https://github.com/feigeCode/onetcli-extensions/releases/latest/download/extension-manifest.json
```

不要让主应用 release 依赖本仓库的扩展构建。主应用负责运行时消费；本仓库负责扩展生产和发布。
