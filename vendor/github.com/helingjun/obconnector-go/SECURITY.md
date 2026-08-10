# Security Policy

## Supported Versions

`obconnector-go` is currently pre-1.0. Security fixes are applied to the default branch until a release policy is established.

## Reporting a Vulnerability

Please do not open public issues for suspected vulnerabilities.

Report security concerns privately to the maintainers through GitHub's private vulnerability reporting feature when it is enabled for this repository. If that feature is unavailable, contact the repository owner privately and include:

- A description of the issue and affected versions or commits.
- Steps to reproduce or a minimal proof of concept.
- Any relevant logs with credentials, hostnames, tenant names, and tokens redacted.

## Credential Handling

Never commit real database passwords, tenant names, hostnames, packet captures, TLS keys, or environment files. Use placeholders in examples and redact sensitive values before sharing logs.
