# Migrating to sr 5.x

sr 5.x is a focused release engineering tool. All AI-powered workflow commands
(commit, pr, review, worktree, rebase, branch, ask, explain) have been removed.
Those workflows are now handled by your AI assistant via the MCP server
(`sr mcp serve`).

**Remaining commands:** `release`, `status`, `config`, `init`, `mcp`, `migrate`,
`completions`, `update`.

This guide covers migration from both 3.x and 4.x.

---

## 1. Remove git hooks

sr 3.x/4.x installed git hooks that called `sr hook ...`. These no longer exist.

```bash
# Remove hook files
rm -rf .githooks/
rm -f .git/hooks/commit-msg .git/hooks/pre-commit .git/hooks/pre-push

# Unset custom hooks path
git config --unset core.hooksPath 2>/dev/null || true
```

If you use husky, lefthook, or pre-commit, remove any lines referencing `sr hook`.

## 2. Update sr.yaml

### From 3.x (flat format)

The flat config must be restructured into nested sections:

```yaml
# 3.x (flat — no longer valid)
branches: [main]
tag_prefix: "v"
commit_pattern: '...'
types: [...]
build_command: "cargo build --release"
pre_release_command: "cargo test"
hooks:
  commit-msg: [sr hook commit-msg]

# 5.x (grouped by concern)
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

Key mappings:

| 3.x field | 5.x location |
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

### From 4.x

The config structure is unchanged. No migration needed for `sr.yaml`.

## 3. Update GitHub Action

Change the action reference from `@v3` or `@v4` to `@v5`:

```yaml
# Before
- uses: urmzd/sr@v3
- uses: urmzd/sr@v4

# After
- uses: urmzd/sr@v5
```

### New action inputs

The v5 action exposes all `sr release` CLI flags:

| Input | Description |
|-------|-------------|
| `artifacts` | Glob patterns for artifact files to upload (space-separated) |
| `package` | Target a specific monorepo package |
| `channel` | Release channel (e.g. canary, rc, stable) |
| `prerelease` | Pre-release identifier (e.g. alpha, beta, rc) |
| `stage-files` | Additional files to stage in the release commit (space-separated) |
| `sign-tags` | Sign tags with GPG/SSH |
| `draft` | Create GitHub release as a draft |

Example with artifacts:

```yaml
- uses: urmzd/sr@v5
  with:
    github-token: ${{ steps.app-token.outputs.token }}
    artifacts: "release-assets/*"
```

### Removed: `command` input

The v4 action accepted a `command` input. The v5 action always runs `sr release`
(or `sr status` with `dry-run: true`). All release options are exposed as
dedicated inputs.

## 4. Commands removed

All AI-powered commands have been removed. Use your AI assistant (Claude Code,
Copilot, etc.) or the MCP server (`sr mcp serve`) instead.

| Removed command | What to use instead |
|-----------------|---------------------|
| `sr ask` | AI assistant |
| `sr explain` | AI assistant + `git show` |
| `sr commit` | AI assistant or `sr mcp serve` |
| `sr rebase` | AI assistant or `git rebase` |
| `sr branch` | AI assistant or `git worktree` |
| `sr worktree` | AI assistant or `git worktree` |
| `sr pr` | AI assistant + `gh` CLI |
| `sr review` | AI assistant + `gh` CLI |
| `sr plan` | `sr status` |
| `sr version` | `sr status` or `sr status --format json \| jq .next_version` |
| `sr changelog` | Generated automatically by `sr release` |

## 5. CI/CD scripts

Update any scripts that called removed commands:

```bash
# 3.x/4.x
VERSION=$(sr version --short)
sr changelog --write
sr plan --format json

# 5.x
VERSION=$(sr status --format json | jq -r '.next_version')
# Changelog is written automatically by sr release
sr status --format json
```

## 6. Architecture

Consolidated from 5 crates (3.x) to 2. All logic lives in `sr-core`; `sr-cli`
is the thin CLI binary.

| 3.x crate | 5.x status |
|------------|------------|
| `sr-ai` | Removed |
| `sr-git` | Merged into `sr-core` |
| `sr-github` | Merged into `sr-core` |
| `sr-core` | All release logic, git, GitHub, config |
| `sr-cli` | CLI dispatch only |
