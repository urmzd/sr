# Security Policy

## Supported Versions

| Version | Supported |
|---------|-----------|
| 6.x     | Yes       |
| < 6.0   | No        |

## Reporting a Vulnerability

If you discover a security vulnerability, please report it responsibly:

1. **Do not** open a public GitHub issue.
2. Email **theurmzd@gmail.com** with:
   - A description of the vulnerability
   - Steps to reproduce
   - Affected versions
   - Any potential impact assessment
3. You will receive an acknowledgment within 48 hours.

## Scope

sr is a release engineering CLI that interacts with:

- **Git repositories** — reads commit history, creates tags, writes changelogs
- **GitHub API** — creates releases, uploads artifacts (requires `GH_TOKEN`)
- **Local filesystem** — reads/writes `sr.yaml`, version files, changelogs
- **AI backends** — sends commit diffs to configured AI providers for commit/review/PR generation

### Security considerations

- **Token handling** — `GH_TOKEN` / `GITHUB_TOKEN` are read from environment variables, never written to disk or logs.
- **No network access by default** — sr only contacts GitHub when creating releases and AI backends when using AI commands. All other operations are local.
- **No runtime dependencies** — single static binary with no dynamic linking, no plugin system, no shell evaluation of user config values.
- **Config is declarative** — `sr.yaml` contains version/changelog/commit-type configuration only. No arbitrary code execution from config.

## Supply Chain

- Built with Rust (memory-safe, no garbage collector)
- CI builds are reproducible from tagged source
- Pre-built binaries are published as GitHub release assets with checksums
