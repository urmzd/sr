# Migration Guide

## Overview

| Version | Theme | Key changes |
|---------|-------|-------------|
| **3.x** | AI-powered CLI | Built-in AI backends, git hooks, flat config |
| **4.x** | Config restructure + MCP | Nested config, MCP server, AI backends removed (commands kept as thin wrappers) |
| **5.x** | Release-only CLI | CLI commands stripped to release engineering; all git/PR/review workflows move to MCP tools |

---

## Migrating from 3.x to 4.x

### Config restructured

The flat `sr.yaml` was reorganized into three sections — `commit`, `release`, `hooks`:

```yaml
# 3.x (flat)
branches: [main]
tag_prefix: "v"
commit_pattern: '...'
types: [...]
build_command: "cargo build --release"
pre_release_command: "cargo test"
hooks:
  commit-msg: [sr hook commit-msg]

# 4.x (grouped by concern)
commit:
  pattern: '...'
  types: [...]

release:
  branches: [main]
  tag_prefix: "v"
  version_files: [Cargo.toml]
  floating_tags: true

hooks:
  pre_release: ["cargo test"]
  post_release: ["./notify.sh"]
```

Field mappings:

| 3.x field | 4.x location |
|-----------|--------------|
| `branches` | `release.branches` |
| `tag_prefix` | `release.tag_prefix` |
| `commit_pattern` | `commit.pattern` |
| `breaking_section` | `commit.breaking_section` |
| `misc_section` | `commit.misc_section` |
| `types` | `commit.types` |
| `version_files` | `release.version_files` |
| `floating_tags` | `release.floating_tags` |
| `artifacts` | `release.artifacts` |
| `stage_files` | `release.stage_files` |
| `build_command` | Removed — use `hooks.pre_release` |
| `pre_release_command` | `hooks.pre_release` |
| `post_release_command` | `hooks.post_release` |
| `hooks.commit-msg` | Removed |
| `hooks.pre-commit` | Removed |

### AI backends removed

sr 3.x called AI providers directly. In 4.x, the AI module was removed. Commands
(`commit`, `pr`, `review`, `worktree`, `rebase`) were kept as thin non-AI wrappers
that accept explicit flags instead of generating content.

### MCP server added

`sr mcp serve` exposes git operations as tools for AI assistants (Claude Code,
Gemini CLI, etc.). `sr init` creates both `sr.yaml` and `.mcp.json`.

### Git hooks removed

sr 3.x installed git hooks (`commit-msg`, `pre-commit`) via `.githooks/`. These
no longer exist. Remove them:

```bash
rm -rf .githooks/
rm -f .git/hooks/commit-msg .git/hooks/pre-commit .git/hooks/pre-push
git config --unset core.hooksPath 2>/dev/null || true
```

The 4.x `hooks:` section uses lifecycle event names (`pre_release`, `post_release`)
instead of git hook names.

### Crate consolidation

5 crates consolidated to 2:

| 3.x crate | 4.x status |
|------------|------------|
| `sr-ai` | Removed |
| `sr-git` | Merged into `sr-core` |
| `sr-github` | Merged into `sr-core` |
| `sr-core` | All release logic, git, GitHub, config |
| `sr-cli` | CLI dispatch only |

### GitHub Action

Update `@v3` → `@v4`:

```yaml
# Before
- uses: urmzd/sr@v3
# After
- uses: urmzd/sr@v4
```

---

## Migrating from 4.x to 5.x

### CLI commands removed

The thin wrapper commands added in 4.x were removed. These workflows now live
entirely in the MCP server tools:

| Removed CLI command | MCP tool equivalent |
|---------------------|---------------------|
| `sr commit` | `sr_commit` (via `sr mcp serve`) |
| `sr pr` | `sr_pr_template` + `sr_pr_create` |
| `sr review` | AI assistant + `gh` CLI |
| `sr worktree` | `sr_worktree` + `sr_worktree_list` + `sr_worktree_remove` |
| `sr rebase` | `git rebase` directly |
| `sr plan` | `sr status` |
| `sr version` | `sr status --format json \| jq .next_version` |
| `sr changelog` | Generated automatically by `sr release` |

**Remaining CLI commands:** `release`, `status`, `config`, `init`, `mcp`, `migrate`,
`completions`, `update`.

### MCP tools expanded

New tools available via `sr mcp serve`:

| Tool | Description |
|------|-------------|
| `sr_commit` | Now supports `breaking` flag — auto-adds `!` and `BREAKING CHANGE:` footer |
| `sr_pr_template` | Generates PR template from branch commits/diff stats |
| `sr_pr_create` | Creates GitHub PR via `gh` CLI (title, body, labels, draft) |
| `sr_worktree` | Creates worktrees under `.sr/worktrees/` with metadata tracking |
| `sr_worktree_list` | Lists all worktrees with descriptions and creation dates |
| `sr_worktree_remove` | Removes worktree and cleans up metadata |

### Dirty working tree check removed

sr 4.x refused to release if the working tree had uncommitted changes. This check
was removed in 5.x because sr only stages files it explicitly modifies (version
files, changelog, stage_files) — it never runs `git add -A`. Unrelated files in
the working tree (downloaded CI artifacts, build outputs) are harmless and were
causing false failures.

### Action inputs expanded

The v5 action exposes all `sr release` CLI flags as inputs:

| New input | Description |
|-----------|-------------|
| `artifacts` | Glob patterns for artifact files to upload (space-separated) |
| `package` | Target a specific monorepo package |
| `channel` | Release channel (e.g. canary, rc, stable) |
| `prerelease` | Pre-release identifier (e.g. alpha, beta, rc) |
| `stage-files` | Additional files to stage in release commit (space-separated) |
| `sign-tags` | Sign tags with GPG/SSH |
| `draft` | Create GitHub release as a draft |

The v4 `command` input was removed. The v5 action always runs `sr release`
(or `sr status` with `dry-run: true`).

Update `@v4` → `@v5`:

```yaml
# Before
- uses: urmzd/sr@v4

# After — basic
- uses: urmzd/sr@v5

# After — with artifacts
- uses: urmzd/sr@v5
  with:
    github-token: ${{ steps.app-token.outputs.token }}
    artifacts: "release-assets/*"
```

### `sr init` improvements

- No longer fails if `sr.yaml` already exists — each step (sr.yaml, .mcp.json,
  .gitignore) runs independently
- Automatically adds `.sr/` to `.gitignore` (used for worktree metadata and cache)

### Worktree management via `.sr/`

Worktrees created via MCP (`sr_worktree`) are stored under `.sr/worktrees/`
instead of sibling directories. Each worktree gets a metadata file at
`.sr/worktrees/<branch>.json` tracking its purpose, description, and creation
date. The `.sr/` directory is automatically gitignored by `sr init`.

---

## Migrating from 3.x directly to 5.x

Follow both sections above in order:

1. **Remove git hooks** — delete `.githooks/`, unset `core.hooksPath`
2. **Restructure sr.yaml** — move flat fields into `commit:`, `release:`, `hooks:` sections
3. **Update action** — change `@v3` to `@v5`
4. **Update scripts** — replace `sr version`, `sr changelog`, `sr plan` with `sr status`

### CI/CD script migration

```bash
# 3.x
VERSION=$(sr version --short)
sr changelog --write
sr plan --format json

# 5.x
VERSION=$(sr status --format json | jq -r '.next_version')
# Changelog is written automatically by sr release
sr status --format json
```
