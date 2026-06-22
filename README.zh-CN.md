# onetcli-extensions

English version: [README.md](README.md)

`onetcli` 官方一方扩展仓库。

本仓库用于独立构建和发布官方扩展包，不跟随主应用 `onetcli` 的发布流程。主应用继续负责扩展运行时、扩展市场客户端、更新客户端以及 SDK/运行时协议；本仓库负责官方扩展源码、发布产物、仓库维护的扩展市场 manifest 和 Cloudflare R2 上传自动化。

## 当前内容

```text
extensions/
  ipc/
    duckdb/       Rust DuckDB IPC 数据库驱动
    iotdb/        Go Apache IoTDB IPC 数据库驱动
    dm/           Go 达梦 DM IPC 数据库驱动
    kingbase/     Go KingbaseES IPC 数据库驱动
    gbase8s/      Java GBase 8s IPC 数据库驱动
    oceanbase/    Go OceanBase IPC 数据库驱动
    opengauss/    Rust openGauss IPC 数据库驱动
    oracle-go/    Go Oracle IPC 数据库驱动（纯 Go，godror）
  remote-desktop/
    rdp/          Rust RDP 远程桌面 provider
    rdp-helper/   Rust RDP helper 二进制（Cargo workspace）
    vnc/          Rust VNC 远程桌面 provider
    vnc-helper/   Rust VNC helper 二进制（Cargo workspace）
cmd/
  dm-ipc-driver/
  iotdb-ipc-driver/
  kingbase-ipc-driver/
  oceanbase-ipc-driver/
  oracle-go-ipc-driver/
java/
  gbase8s-ipc-driver/
internal/
  dbipc/          Go IPC 数据库驱动共享运行时
  drivers/        驱动专属 Go 实现（dm、iotdb、kingbase、oceanbase、oracle）
  ipc/            Go IPC 帧协议与 socket 工具
  runner/         Go IPC 进程 runner
manifest.json     轻量市场索引
scripts/
  build-go-driver.sh
  build-java-driver.sh
  changed-extensions.mjs
  generate-marketplace-manifest.mjs
  install-local-drivers.sh
  install-local-remote-desktop-providers.sh
  package-driver.sh
  package-remote-desktop-provider.sh
  release-driver.mjs
  verify-package.sh
  verify-remote-desktop-provider-package.sh
tests/
  scripts.test.mjs
.codex/
  skills/ipc-driver-development/
```

根目录下重复的 `ipc-driver-development/` skill 目录不再使用。驱动开发说明只保留在 `.codex/skills/ipc-driver-development/`。

## 数据库驱动矩阵

| 驱动 | 运行时 | 构建元数据 | Manifest | 说明 |
| --- | --- | --- | --- | --- |
| DuckDB | Rust | `extensions/ipc/duckdb/extension.build.json` | `extensions/ipc/duckdb/driver.json` | 嵌入式单文件分析数据库驱动。Cargo workspace member。 |
| Apache IoTDB | Go | `extensions/ipc/iotdb/extension.build.json` | `extensions/ipc/iotdb/driver.json` | 时序数据库驱动。使用 `cmd/iotdb-ipc-driver` 和 `internal/drivers/iotdb`。 |
| 达梦 DM | Go | `extensions/ipc/dm/extension.build.json` | `extensions/ipc/dm/driver.json` | 复用 `internal/dbipc` 共享运行时，并使用 `dm_driver` build tag。 |
| KingbaseES | Go | `extensions/ipc/kingbase/extension.build.json` | `extensions/ipc/kingbase/driver.json` | 复用 `internal/dbipc` 共享运行时，并使用 `kingbase_driver` build tag。 |
| GBase 8s | Java | `extensions/ipc/gbase8s/extension.build.json` | `extensions/ipc/gbase8s/driver.json` | 使用 `java/gbase8s-ipc-driver`。如果存在 `java/gbase8s-ipc-driver/bin/lib/gbase8s-ipc-driver.jar`，需要保留。仅 universal（跨平台）target。 |
| OceanBase | Go | `extensions/ipc/oceanbase/extension.build.json` | `extensions/ipc/oceanbase/driver.json` | 复用 `internal/dbipc` 共享运行时，并使用 `oceanbase_driver` build tag。 |
| openGauss | Rust | `extensions/ipc/opengauss/extension.build.json` | `extensions/ipc/opengauss/driver.json` | Cargo workspace member。使用 `tokio-opengauss` 异步驱动。 |
| Oracle Go | Go | `extensions/ipc/oracle-go/extension.build.json` | `extensions/ipc/oracle-go/driver.json` | 纯 Go Oracle 驱动，使用 `oracle_go_driver` build tag。 |

国产数据库驱动在 `driver.json` 中声明 `"category": "domestic_database"`；host 侧应该使用 manifest 元数据做 UI 分组，不要硬编码具体驱动 id。

## 远程桌面 Provider 矩阵

| Provider | 运行时 | 构建元数据 | Manifest | 说明 |
| --- | --- | --- | --- | --- |
| RDP | Rust | `extensions/remote-desktop/rdp/extension.build.json` | `extensions/remote-desktop/rdp/remote_desktop_provider.json` | RDP 远程桌面 provider。二进制由 `extensions/remote-desktop/rdp-helper` 构建。 |
| VNC | Rust | `extensions/remote-desktop/vnc/extension.build.json` | `extensions/remote-desktop/vnc/remote_desktop_provider.json` | VNC 远程桌面 provider。二进制由 `extensions/remote-desktop/vnc-helper` 构建。 |

## 协议能力

每个驱动都在 `driver.json.methods` 中声明可调用方法，并且应该在 `init` 返回中暴露同一组方法。这个列表是运行时契约：只要声明了某个方法，二进制就必须路由它，或者有意返回类型化 unsupported 错误。

当前 IPC 驱动通过 legacy 固定方法暴露 schema metadata，例如：

- `schema/databases`
- `schema/schemas`
- `schema/objects`
- `schema/columns`
- `schema/indexes`
- `schema/views`
- `schema/functions`

需要自定义对象列表表头的驱动还会声明 `schema/object_view`。该方法是 connection-bound，返回 host 应渲染的完整表格形状：

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

如果 `schema/object_view` 未声明，或者某个 view 返回类型化 not-supported / method-not-found，host 会回退到 legacy schema 方法。行代表可点击数据库对象时，第一列应保持为对象名。

## SDK 依赖

Rust 驱动依赖 `feigeCode/onetcli` 中的这些 SDK crates：

- `extension-protocol`
- `extension-driver`
- `extension-host`

目前 `Cargo.toml` 指向 `dev` 分支，因为现有 `v0.4.8` tag 还不包含这些 crates。等 `onetcli` 发布包含 SDK crates 的正式 release tag 后，应将这些分支依赖替换为固定 tag 依赖。

Cargo workspace 目前包含 `extensions/ipc/duckdb` 和 `extensions/ipc/opengauss`。RDP 和 VNC helper 是独立的 Cargo 项目，分别位于 `extensions/remote-desktop/rdp-helper` 和 `extensions/remote-desktop/vnc-helper`。

## 本地开发

运行脚本和打包测试：

```bash
node --test tests/scripts.test.mjs
```

运行 Rust 驱动测试：

```bash
cargo test -p duckdb_driver -- --nocapture
cargo test -p opengauss_driver -- --nocapture
```

运行 Go 共享运行时测试：

```bash
GOCACHE=/private/tmp/onetcli-go-cache go test ./internal/dbipc
```

运行 Java 驱动测试：

```bash
mvn -f java/gbase8s-ipc-driver/pom.xml test
```

检查 Rust 格式：

```bash
cargo fmt --all --check
```

校验 GitHub Actions YAML：

```bash
ruby -e 'require "yaml"; YAML.load_file(".github/workflows/ci.yml"); YAML.load_file(".github/workflows/release.yml"); YAML.load_file(".github/workflows/upload-r2.yml"); puts "workflow yaml ok"'
```

## 构建和打包

所有扩展包都由 `extensions/ipc/<driver-id>/extension.build.json` 描述。构建元数据定义扩展 id、运行时语言、package 或 binary 名称、target triples、release tag 前缀和 R2 前缀。

为当前本机 target 构建并打包 DuckDB：

```bash
HOST_TRIPLE="$(rustc -vV | sed -n 's/^host: //p')"
cargo build --release -p duckdb_driver --target "$HOST_TRIPLE"
mkdir -p artifacts
bash scripts/package-driver.sh duckdb "$HOST_TRIPLE" artifacts 1.0.0
bash scripts/verify-package.sh "artifacts/duckdb-driver-${HOST_TRIPLE}.tar.gz"
```

构建并打包 Go 驱动：

```bash
HOST_TRIPLE="$(rustc -vV | sed -n 's/^host: //p')"
bash scripts/build-go-driver.sh dm "$HOST_TRIPLE"
mkdir -p artifacts
bash scripts/package-driver.sh dm "$HOST_TRIPLE" artifacts 0.1.0
```

构建并打包 Java GBase 8s 驱动：

```bash
HOST_TRIPLE="$(rustc -vV | sed -n 's/^host: //p')"
bash scripts/build-java-driver.sh gbase8s "$HOST_TRIPLE"
mkdir -p artifacts
bash scripts/package-driver.sh gbase8s "$HOST_TRIPLE" artifacts 0.1.0
```

构建并打包 Rust 远程桌面 provider：

```bash
HOST_TRIPLE="$(rustc -vV | sed -n 's/^host: //p')"
bash scripts/package-remote-desktop-provider.sh rdp "$HOST_TRIPLE" artifacts 0.1.0
bash scripts/verify-remote-desktop-provider-package.sh "artifacts/rdp-remote-desktop-provider-${HOST_TRIPLE}.tar.gz"
```

扩展包 archive 中包含扩展目录、`driver.json`、入口二进制或 launcher，以及 locales、icons、运行时库等资源。

构建、打包、校验并替换本地已安装驱动：

```bash
bash scripts/install-local-drivers.sh
bash scripts/install-local-drivers.sh dm
```

默认安装到 `$XDG_CONFIG_HOME/one-hub/extensions/database_drivers` 或
`$HOME/.config/one-hub/extensions/database_drivers`。如需改目标目录，可设置
`ONETCLI_DATABASE_DRIVER_DIR=/path/to/database_drivers`。

本地安装远程桌面 provider：

```bash
bash scripts/install-local-remote-desktop-providers.sh
bash scripts/install-local-remote-desktop-providers.sh rdp
```

默认安装到 `$XDG_CONFIG_HOME/one-hub/extensions/remote_desktop_providers` 或
`$HOME/.config/one-hub/extensions/remote_desktop_providers`。

本地准备某个 driver 的发版产物：

```bash
node scripts/release-driver.mjs duckdb 1.0.0
node scripts/release-driver.mjs dm 0.4.0 --target x86_64-unknown-linux-gnu
node scripts/release-driver.mjs gbase8s 0.7.0 --artifact-dir artifacts/gbase8s-0.7.0
```

发版脚本会读取 `extensions/ipc/<driver-id>/extension.build.json`，按运行时选择
对应构建命令，为每个选中的 target 打包并校验 archive，然后写出：

- `artifacts/<driver-id>-driver-<target>.tar.gz`
- `artifacts/sha256sums.txt`
- `artifacts/extension-manifest.json`
- `artifacts/release-metadata.json`

如果二进制已经提前放在 `target/<target>/release`，可以加 `--skip-build` 只做组包和
manifest 生成。

## 扩展市场 Manifest

仓库根目录的 `manifest.json` 是全局市场索引。它直接在本仓库中维护并提交，后续原样上传到
R2 的 `extensions/manifest.json`。

Release job 只生成一份插件级 manifest：

- `artifacts/extension-manifest.json`：当前扩展的插件级 manifest，会发布到该扩展的
  GitHub Release，包含各 target 的包文件名和 checksum。

插件级 manifest 根据以下输入生成：

- 扩展包文件名
- `artifacts/sha256sums.txt`
- release 环境变量

必需环境变量：

```text
ARTIFACT_DIR=artifacts
EXTENSION_VERSION=1.0.0
EXTENSION_ID=duckdb
RELEASE_TAG=duckdb-v1.0.0
```

扩展级 GitHub Release 会保留 `extension-manifest.json` 作为当前扩展的插件级 manifest。
Release workflow 成功后，上传 workflow 会串行执行市场发布，把这份插件 manifest 上传到
R2 的 `extensions/<id>/manifest.json`，再把已提交的根目录 `manifest.json` 用
`no-cache` 上传到 R2 的 `extensions/manifest.json`。

全局市场条目使用 schema v2，只记录扩展元数据和插件 manifest 路径，不记录 artifact
文件和下载 URL。DuckDB 条目示例：

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

插件级 manifest 同样使用 schema v2，记录 artifact 文件名和 checksum，但不记录完整下载 URL：

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

`onetcli` 客户端负责下载源策略：先加载全局市场索引，再加载用户选择扩展的插件级
manifest。R2 包地址按插件 manifest 所在目录拼 `<version>/<file>`；如果 R2 插件
manifest 或包不可用，则根据客户端配置的 GitHub manifest base、条目的 `release_tag`
以及插件 manifest 或 artifact 文件名推导 GitHub Release fallback 地址。

## CI

`.github/workflows/ci.yml` 会检测发生变化的扩展，并只构建受影响的发布单元。

当前选择规则：

- 修改 `extensions/ipc/<driver-id>/**` 时构建对应驱动。
- 修改共享运行时、脚本、workflow 或打包路径时构建所有已知扩展。
- 每个 target triple 对应一个 matrix entry。

## Release

扩展发布使用扩展级 tag：

```bash
git tag duckdb-v1.0.0
git push origin duckdb-v1.0.0
```

Release workflow 会执行：

1. 从 tag 解析扩展 id 和版本。
2. 构建 `extension.build.json` 中列出的所有 target。
3. 打包并校验每个 archive。
4. 生成 checksum。
5. 生成当前扩展的插件级 manifest。
6. 发布包含扩展包、checksum 和当前扩展 `extension-manifest.json` 的 GitHub Release。

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

上传 workflow 使用 `extension-marketplace-publish` concurrency group 串行执行。
以 DuckDB `1.0.0` 为例，R2 会收到：

```text
extensions/duckdb/1.0.0/<package>.tar.gz
extensions/duckdb/manifest.json
extensions/manifest.json
```

版本化扩展包使用 immutable cache。插件级 manifest 和全局市场索引使用 `no-cache`
上传。全局 manifest 是仓库维护的根目录 `manifest.json`，会原样上传到
`extensions/manifest.json`。

## 新增另一个 IPC 驱动

在 `extensions/ipc/<driver-id>` 下新增目录：

```text
driver.json
extension.build.json
locales/
icons/
```

运行时代码按语言放到对应本地 package：

- Rust 驱动通常放在 `extensions/ipc/<driver-id>/src`，并加入根 Cargo workspace members。
- Go 驱动可以复用 `internal/dbipc`，并在 `cmd/` 下新增命令。
- Java 驱动可以放在 `java/` 下。

创建类似的元数据：

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

如果新 IPC 数据库驱动使用现有 metadata 和包结构，通常不需要修改 workflow。

## 新增另一个远程桌面 Provider

在 `extensions/remote-desktop/<provider-id>` 下新增目录：

```text
remote_desktop_provider.json
extension.build.json
```

helper 二进制是 Rust Cargo 项目，位于 `extensions/remote-desktop/<provider-id>-helper`。`extension.build.json` 通过 `manifest_path` 引用 helper 的 `Cargo.toml`，并在 `source_paths` 中列出 helper 源码目录，以便 CI 变更检测能正确工作。

## 主应用集成

主仓库 `onetcli` 应优先从 R2 消费已发布的全局市场 manifest。全局条目会指向某个扩展的
插件级 manifest，例如 `duckdb/manifest.json`；宿主在选择平台 artifact 前先加载这个文件。
GitHub fallback 是扩展级的：宿主根据 `release_tag` 推导
`https://github.com/feigeCode/onetcli-extensions/releases/download/<release_tag>/extension-manifest.json`
作为插件 manifest fallback，再用同一个 release tag 和 artifact 文件名推导包 fallback 地址。

不要让主应用 release 依赖本仓库的扩展构建。主应用负责运行时消费；本仓库负责扩展生产和发布。
