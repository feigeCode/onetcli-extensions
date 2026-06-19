# Tests

## Running Tests

`go test` is used for testing. A running PostgreSQL
server is required, with the ability to log in. The
database to connect to test with is "pqgotest," on
"localhost" but these can be overridden using [environment
variables](https://www.postgresql.org/docs/9.3/static/libpq-envars.html).

Example:

	KBHOST=/run/kingbase go test

## Benchmarks

A benchmark suite can be run as part of the tests:

	go test -bench .

## Example setup (Docker)

Run a postgres container:

```
docker run --expose 54321:54321 kingbase
```

Run tests:

```
KBHOST=localhost KBPORT=54321 KBUSER=system KBSSLMODE=disable KBDATABASE=samples go test
```
