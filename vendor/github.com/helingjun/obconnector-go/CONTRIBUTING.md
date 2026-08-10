# Contributing

Thanks for helping improve `obconnector-go`.

## Development

Before sending changes, run:

```bash
gofmt -w .
go test ./...
go vet ./...
```

Keep the driver pure Go and avoid cgo or runtime dependencies on OBCI, Oracle Instant Client, LibOBClient, or other native client libraries.

## Clean-Room Protocol Work

This project may use public documentation, packet captures from owned test environments, and high-level behavioral observations from compatible clients. Do not copy code, comments, function structure, or implementation details from LGPL/MPL/incompatible driver sources.

Record protocol findings in `docs/protocol-notes.md`, with sensitive hostnames, tenant names, usernames, passwords, and packet payloads redacted.

## Git author metadata

Prefer a stable public email in commits (not a machine-local `*.local` address). The repository includes a `.mailmap` so historical commits and `git shortlog` can group contributors consistently.

## Pull Requests

Pull requests should include:

- A short description of the behavior change.
- Tests or a clear reason tests are not practical.
- Notes for any protocol observations or compatibility risks.
- Confirmation that no credentials, private hostnames, tenant names, or packet captures are included.
