# obconnector-go

[English](README.md)

`obconnector-go` 是一个面向 Go `database/sql` 的 OceanBase 驱动实验项目，目标是在纯 Go、无 cgo、无本地动态库依赖的前提下，连接 OceanBase 租户（Oracle 模式和 MySQL 模式）。

第一个里程碑：通过 OceanBase 租户认证，并执行：

```sql
select 1 from dual
```

本项目不依赖 OBCI、Oracle Instant Client、LibOBClient 或 OBConnector-C 动态库。

## 当前状态

驱动注册两个 `database/sql` driver name：

- `oceanbase`
- `oboracle`

已实现的能力：

- OceanBase 握手扩展（Oracle 和 MySQL 模式）。
- `mysql_native_password` 认证。
- `COM_QUERY` 查询路径。
- 流式读取文本协议结果集。
- `QueryContext` / `ExecContext`。
- `RowsAffected()`。
- 基础事务：`BeginTx`、`Commit`、`Rollback`。
- `Prepare` / `PrepareContext` 兼容路径。
- 客户端侧 `?` 参数插值兼容层。
- Oracle 模式 `:1` 风格参数插值。
- 服务端错误包解析，保留错误码、SQLSTATE 和 ORA 消息。
- `database/sql` 连接池生命周期接口：`IsValid`、`ResetSession`。
- 基础列类型元数据。
- `cmd/obping` 连接验证和回归测试工具。
- TLS 加密连接支持（含自定义 CA 证书）。
- BulkInsert 批量写入辅助函数。

## obping — 连接验证与回归测试工具

`cmd/obping` 是命令行工具，用于测试连接、运行集成测试和回归验证。

### 快速开始

```bash
go run ./cmd/obping \
  -host 127.0.0.1 \
  -port 2881 \
  -user '<user@tenant#cluster>' \
  -password '<password>' \
  -trace
```

或使用 DSN：

```bash
go run ./cmd/obping \
  -dsn 'oceanbase://<user>:<password>@<host>:<port>/?timeout=5s'
```

### 全部参数

| 参数 | 默认值 | 说明 |
|------|--------|------|
| `-dsn` | `""` | 完整 DSN（覆盖单个参数） |
| `-host` | `127.0.0.1` | OceanBase 或 OBProxy 主机地址 |
| `-port` | `2881` | OceanBase 或 OBProxy 端口 |
| `-user` | `""` | OceanBase 用户（`user@tenant#cluster`） |
| `-password` | `""` | OceanBase 密码 |
| `-database` | `""` | 数据库/模式名 |
| `-timeout` | `10s` | 连接和查询超时 |
| `-tls` | `false` | 启用 TLS 加密 |
| `-tls-ca` | `""` | CA 证书路径 |
| `-oracle-mode` | `false` | 强制 Oracle 模式 |
| `-mysql-mode` | `false` | 强制 MySQL 模式 |
| `-trace` | `false` | 输出握手和查询日志到 stderr |
| `-preset` | `""` | 客户端标识预设 |
| `-ob20` | `false` | 启用 OB 2.0 协议封装 |
| `-query` | `"select 1 from dual"` | 要执行的查询 |
| `-max-rows` | `20` | 最大打印行数 |
| `-attr` | — | 连接属性 `key=value`（可重复） |
| `-init` | — | 认证后执行的 SQL（可重复） |

#### 测试参数

| 参数 | 说明 |
|------|------|
| `-tx-test` | 基础事务测试（BEGIN / COMMIT / ROLLBACK） |
| `-exec-test` | DDL/DML 冒烟测试 |
| `-param-test` | 参数化查询测试 |
| `-pool-test` | 连接池生命周期测试 |
| `-bulk-test` | BulkInsert 批量写入测试 |
| `-full-test` | **综合集成测试（全部上述测试组合）** |

### DSN

推荐 URL 风格 DSN：

```text
oceanbase://<user>:<password>@<host>:<port>/<database>?<参数>
```

也兼容实验中的 opaque DSN：

```text
oceanbase:<user>:<password>@<host>:<port>/<database>?<参数>
```

#### DSN 参数

| 参数 | 值 | 说明 |
|------|-----|------|
| `timeout` | `5s` | 连接超时 |
| `trace` | `true`/`false` | 输出握手和查询日志 |
| `tls` | `true`/`skip-verify`/`false` | 启用 TLS |
| `tls.ca` | 文件路径 | CA 证书路径 |
| `oracleMode` | `true`/`false`/`auto` | 强制 Oracle/MySQL 模式或自动检测 |
| `ob20` | `true`/`false` | 启用 OB 2.0 协议 |
| `preset` | 预设名 | 客户端标识预设 |
| `collation` | uint8 | 握手 collation |
| `init` | SQL | 认证后执行初始化 SQL（可重复） |

### Oracle 租户全面测试

```bash
go run ./cmd/obping \
  -host <host> \
  -port 1521 \
  -user '<user>@<tenant>#<cluster>' \
  -password '<password>' \
  -oracle-mode \
  -full-test
```

`-oracle-mode` 在 OBProxy 下强制 Oracle 模式（因 OBProxy 返回的版本号不包含 "oracle"）。

### MySQL 租户全面测试（TLS）

```bash
go run ./cmd/obping \
  -host <host> \
  -port 3306 \
  -user '<user>' \
  -password '<password>' \
  -database <database> \
  -tls \
  -tls-ca /path/to/ca.pem \
  -mysql-mode \
  -full-test
```

`-tls` 启用加密连接，`-tls-ca` 指定 CA 证书路径。

### 完整测试清单

`-full-test` 按顺序运行 16 项测试：

| # | 测试 | 说明 |
|---|------|------|
| 1 | Ping | 基础连通性 |
| 2 | 数值 | 数值运算 |
| 3 | 字符串 | 字符串函数 |
| 4 | 时间戳 | `CURRENT_TIMESTAMP` |
| 5 | DDL | CREATE TABLE |
| 6 | INSERT | 插入 3 行 |
| 7 | SELECT | 多行查询 |
| 8 | 参数查询 | `?` 参数化查询（文本协议） |
| 9 | Prepared `?` | 服务端预处理（仅 Oracle） |
| 10 | Prepared `:1` | 服务端预处理（仅 Oracle） |
| 11 | UPDATE | 更新数据 |
| 12 | DELETE | 删除数据 |
| 13 | NULL | NULL 值处理 |
| 14 | 事务回滚 | UPDATE + ROLLBACK 验证 |
| 15 | 事务提交 | INSERT + COMMIT 验证 |
| 16 | 清理 | DROP 测试表 |

> **说明：** 第 9-10 项（预处理语句）使用 `COM_STMT_EXECUTE`，部分 OBProxy 版本不支持该协议（尤其是 MySQL 模式 OBProxy）。Oracle 模式下预处理语句正常工作。测试 9-10 失败不会影响后续测试。

### 回归测试

代码变更后，使用 `-full-test` 进行回归验证：

**Oracle 租户：**
```bash
obping -host <host> -port 1521 -user '<user>' -password '<password>' -oracle-mode -full-test
```

**MySQL 租户：**
```bash
obping -host <host> -port 3306 -user '<user>' -password '<password>' -database <database> -tls -tls-ca /path/to/ca.pem -mysql-mode -full-test
```

预期结果：`=== ALL TESTS PASSED ===`

## 在 Go 代码中使用驱动

```go
package main

import (
	"database/sql"
	_ "github.com/helingjun/obconnector-go"
)

func main() {
	db, err := sql.Open("oceanbase", "oceanbase://<user>:<password>@<host>:<port>/?timeout=5s")
	if err != nil {
		panic(err)
	}
	defer db.Close()

	var one string
	if err := db.QueryRow("select 1 from dual").Scan(&one); err != nil {
		panic(err)
	}
}
```

## 参数支持说明

当前 `?` 和 `:1` 参数支持是客户端侧插值兼容层，不是服务端 prepared statement。

支持的参数类型：

- `nil`
- `string`
- `int64`
- `float64`
- `bool`
- `[]byte`
- `time.Time`

插值逻辑会正确跳过字符串字面量、双引号标识符、行注释和块注释中的占位符。

## 安全和隐私

**请不要提交真实的：**

- 数据库密码。
- 私有主机名或内网 IP。
- 租户名、集群名、用户名。
- TLS 私钥、证书、token。
- 未脱敏的抓包文件或日志。

文档、Issue 和 PR 中请始终使用占位符（`<host>`、`<user>`、`<password>` 等）。

## 开发

提交前请运行：

```bash
gofmt -w .
go test ./...
go vet ./...
```

## 许可证

Apache-2.0。详见 `LICENSE`。
