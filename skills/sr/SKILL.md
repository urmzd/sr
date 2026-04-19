---
name: sr
description: Semantic release — automated versioning, changelog generation, and GitHub releases from conventional commits. Single binary, zero-config, language-agnostic.
metadata:
  argument-hint: [command]
---

# sr — Semantic Release

Use `sr` to manage the full release lifecycle from conventional commits.

## Steps

1. Ensure a `sr.yaml` config exists. If not, run `sr init`.
2. If `$ARGUMENTS` is provided, run `sr $ARGUMENTS` instead of the default flow.
3. Default flow: preview with `sr status`, then execute with `sr release`.

## Commands

| Command | Description |
|---------|-------------|
| `sr release` | Execute a release (tag + GitHub release) |
| `sr release --dry-run` | Simulate without side effects |
| `sr release --sign-tags` | Sign tags with GPG/SSH |
| `sr release --draft` | Create GitHub release as draft |
| `sr release -p <name>` | Release a specific monorepo package |
| `sr release -c <channel>` | Release via named channel (e.g. canary, rc) |
| `sr release --prerelease <id>` | Produce pre-release versions (e.g. 1.2.0-alpha.1) |
| `sr release --artifacts <glob>` | Upload artifacts matching glob |
| `sr release --stage-files <glob>` | Stage additional files in the release commit |
| `sr status` | Show branch, version, unreleased commits, and open PRs |
| `sr status --format json` | Machine-readable status output |
| `sr status -p <name>` | Status for a specific package |
| `sr config` | Validate and display resolved configuration |
| `sr config --resolved` | Show resolved config with defaults |
| `sr init` | Create default `sr.yaml` config file |
| `sr init --force` | Overwrite existing config |
| `sr completions <shell>` | Generate shell completions |
| `sr update` | Update sr to the latest version |
| `sr migrate` | Show migration guide |

## Monorepo

Use `-p/--package` to target a specific package when `packages` is configured in `sr.yaml`:

```bash
sr release -p core          # release only the core package
sr status -p cli            # status for cli package
```

## Release Execution Order

1. Pre-release command → 2. Bump version files → 3. Write changelog → 4. Build command → 5. Git commit → 6. Create/push tag (signed if configured) → 7. Floating tag → 8. Create/update GitHub release (draft if configured) → 9. Upload artifacts → 10. Verify release → 11. Post-release command

## Environment

- `GH_TOKEN` / `GITHUB_TOKEN` — Required for GitHub releases
- `SR_VERSION` / `SR_TAG` — Set during hook commands

## Exit Codes

- `0` — Success
- `1` — Error (config, git, VCS)
- `2` — No releasable changes
