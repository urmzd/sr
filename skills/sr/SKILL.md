---
name: sr
description: Release-state reconciler — declarative change + release management. Single binary, one global version per repo, typed publishers (cargo/npm/docker/pypi/go), no user shell hooks.
metadata:
  argument-hint: [command]
---

# sr — Release-state reconciler

`sr` treats releases as state to reconcile. The VCS + registries are the actual state; `sr.yaml` + conventional commits are the desired state; `sr` computes the diff and applies.

## Steps

1. Ensure `sr.yaml` exists. If not, run `sr init` (or `sr init <example>` — see `sr init --list`).
2. If `$ARGUMENTS` is provided, run `sr $ARGUMENTS`.
3. Default flow: preview with `sr plan`, then execute with `sr release`.

## Three verbs

| Command | What it does |
|---------|-------------|
| `sr plan` | Preview — next version, tag, Terraform-style resource diff. No side effects. |
| `sr prepare` | Write bumped version files + changelog to disk. No commit, tag, or push. Use when CI needs the bumped manifest before a build step (so binaries embed the correct version). |
| `sr release` | Full apply — commit, tag, push, create GH release, upload, publish. Idempotent. |

## Flags

| Flag | Usage |
|---|---|
| `sr plan --format json` | Machine-readable plan + diff |
| `sr release --dry-run` | Preview without side effects (equivalent to `sr plan`) |
| `sr release -c <channel>` | Release via named channel (canary, rc, stable) |
| `sr release --prerelease <id>` | Produce 1.2.0-alpha.1 |
| `sr release --sign-tags` | GPG/SSH tag signing |
| `sr release --draft` | Draft GitHub release |
| `sr release --artifacts <path>` | Upload a literal path as a release asset |
| `sr release --stage-files <path>` | Stage extra file in release commit |
| `sr prepare --prerelease <id>` | Bump to a prerelease version |
| `sr config --resolved` | Show config with defaults applied |
| `sr init <example>` | Scaffold from bundled example (`sr init --list`) |

## Monorepos

One global version for every package. Every `packages[].version_files` is bumped to the same version on each release. For workspace ecosystems, one root entry with `publish.workspace: true` covers every member (cargo/npm/pnpm/uv).

Monorepo-specific targeting (`-p/--package`) does not exist — `sr release` is whole-repo.

## Publishers

Typed. `publish: { type: cargo | npm | docker | pypi | go | custom }`. Built-ins query the registry before publishing; already-published versions are skipped. No user shell required.

## Release execution order

1. Parse commits → determine bump
2. Bump version files (every package's `version_files`) + write changelog
3. Validate artifacts — every `artifacts` path must exist on disk
4. Commit release files
5. Create + push annotated tag
6. Update floating major tag (e.g. `v3`)
7. Create or update GitHub release
8. Upload artifacts
9. Run typed publishers (cargo/npm/etc.) — each skips if already published

Every stage has a strict `is_complete` check. Re-running on a converged release is a noop. Partial failures recover by re-running — there's no state file.

## Environment

- `GH_TOKEN` / `GITHUB_TOKEN` — required for GitHub releases
- `SR_VERSION` / `SR_TAG` — set when `publish: custom` commands run
- Registry tokens (`CARGO_REGISTRY_TOKEN`, `NODE_AUTH_TOKEN`, `UV_PUBLISH_TOKEN`, etc.) — consumed by the typed publishers

## Exit codes

- `0` — success (JSON printed to stdout)
- `1` — error (config, git, VCS, publisher failure)
- `2` — no releasable changes
