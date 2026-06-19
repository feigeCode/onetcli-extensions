# 人大金仓 kingbase golang driver - A pure Go postgres driver for Go's  package

## 安装

	go get gitea.com/kingbase/gokb

## 实现

* SSL
* Handles bad connections for `database/sql`
* Scan `time.Time` correctly (i.e. `timestamp[tz]`, `time[tz]`, `date`)
* Scan binary blobs correctly (i.e. `bytea`)
* Package for `hstore` support
* COPY FROM support
* pq.ParseURL for converting urls to connection strings for sql.Open.
* Many libpq compatible environment variables
* Unix socket support
* Notifications: `LISTEN`/`NOTIFY`
* pgpass support
* GSS (Kerberos) auth

## 测试

`go test` is used for testing.  See [TESTS.md](TESTS.md) for more details.

## 修改

修改自 github.com/lib/pq