# Migration Guide

## Overview

| Version | Theme | Key changes |
|---------|-------|-------------|
| **3.x** | AI-powered CLI | Built-in AI backends, git hooks, flat config |
| **4.x** | Config restructure + MCP | Nested config, MCP server, AI backends removed (commands kept as thin wrappers) |
| **5.x** | Release-only CLI | CLI commands stripped to release engineering; all git/PR/review workflows move to MCP tools |
| **6.x** | MCP-first workflows | PR, worktree, and breaking-commit tools added to MCP server; `sr init` improved |
| **7.x** | Config redesign | Entire config structure rewritten; 6 top-level sections; MCP server removed; agentspec removed; file snapshot/rollback removed |
| **7.1** | Build stage + roll-forward recovery | New `hooks.build` phase runs after bump before tag; declared artifacts validated before tagging; `sr-manifest.json` proves completion; idempotent uploads; reconciliation warns (never blocks); `--force` flag removed — recovery is push a new commit |
| **8.x** | Reconciler model + typed publishers + three verbs | `plan` / `prepare` / `release`; typed publishers (cargo/npm/docker/pypi/go/custom); workspace-aware publishes; shell hooks removed (builds live in CI); `sr-manifest.json` removed; monorepos collapse to one global version; literal paths only (no globs) |

---

## Migrating from 7.x to 8.x (breaking)

**8.x is a scope reduction**: `sr` stops running user shell commands and becomes a pure release-state reconciler. The VCS + registries are the only state; sr reads them and applies what's missing.

### Verb changes

| 7.x | 8.x |
|---|---|
| `sr status` | **`sr plan`** (Terraform-style resource diff; same data, richer output) |
| `sr release` | `sr release` (unchanged entrypoint; internally a reconciler now) |
| *(new)* | **`sr prepare`** — bump version files + write changelog on disk, no commit/tag. Use between `sr plan` and `sr release` in multi-job CI so artifact builds pick up the new version. |
| `sr release -p <name>` | **removed** — `sr release` is whole-repo; monorepos share one version. |

### Config: removed fields

```yaml
# 7.x — all of these are gone in 8.x
packages:
  - path: .
    independent: true          # ← removed (one global version for all)
    tag_prefix: "cli-v"        # ← removed (git.tag_prefix is the only prefix)
    hooks:                     # ← removed entirely
      pre_release: [...]
      build: [...]
      post_release: [...]
```

### Config: new `publish:` (typed)

```yaml
# 7.x
packages:
  - path: .
    hooks:
      post_release:
        - cargo publish

# 8.x — typed publisher
packages:
  - path: .
    publish:
      type: cargo
```

Supported publisher types: `cargo`, `npm`, `docker`, `pypi`, `go`, `custom`. Each queries its registry's API to decide if work is needed. See [examples/](../../../examples/) for one complete config per ecosystem.

### PyPI artifact resolution

The `pypi` publisher resolves each workspace member's wheel + sdist from a **single shared `dist/` directory** under the package path — it does not `cd` into members and rely on `uv publish`'s cwd glob. This matches `uv build --all`'s actual output layout (one workspace-root `dist/` for the whole workspace), and matches `poetry build` / `python -m build` when run at the root.

```yaml
# Single package — artifacts at `./dist/<name>-<version>*`
packages:
  - path: .
    publish:
      type: pypi

# Workspace — `uv build --all` writes every member to `./dist/`; sr resolves
# per member by filename (PEP 625 stem + version) and calls
# `uv publish <files>` once per member.
packages:
  - path: .
    publish:
      type: pypi
      workspace: true
```

`dist_dir:` overrides the default `dist/` location (e.g. `.dist`, `build/wheels`). Artifacts are matched by PEP 625 filename stem and exact version, so mixed-package dist dirs are safe — sr only picks up the files that belong to the current member.

Fallback order: `uv publish <files>` when `uv` is on `$PATH`, else `twine upload <files>`. Both accept positional file arguments; shell expansion is not involved.

**Upgrading from 8.0.x to 8.0.y:** if you previously built wheels inside each workspace member's own `dist/` directory, either switch to `uv build --all` at the workspace root, or keep per-member builds and rely on the default behavior — `<package_path>/dist` resolves to the member's dist when `path:` points at the member.

### Config: literal paths only

```yaml
# 7.x — globs allowed
artifacts:
  - "target/release/sr-*"
stage_files:
  - "Cargo.lock"

# 8.x — literal paths only
artifacts:
  - target/release/sr-x86_64-unknown-linux-musl
  - target/release/sr-aarch64-apple-darwin
stage_files:
  - Cargo.lock
```

Workspace member discovery inside `Cargo.toml` / `package.json` / `pyproject.toml` still uses those tools' native globs — that's unchanged.

### `sr-manifest.json` removed

Previously sr uploaded `sr-manifest.json` to every release as a completion marker. 8.x relies on tag presence + release asset list + registry version queries to determine state. Existing releases with the old manifest keep working; no migration step needed.

### CI workflow changes

Builds that embed a version (Rust binaries, Python wheels, packed tarballs) must run between `sr prepare` and `sr release`:

```yaml
# 7.x — hooks.build ran inside sr
- uses: urmzd/sr@v7    # runs cargo build as hooks.build

# 8.x — build lives in CI
- uses: urmzd/sr@v8
  with: { mode: prepare }
- run: cargo build --release
- uses: urmzd/sr@v8    # uploads the binary
```

For multi-platform matrix builds (impossible under 7.x), see [examples/ci/cargo-multi-platform.yml](../../../examples/ci/cargo-multi-platform.yml).

### Action input changes

- `mode: plan | prepare | release` — new. Default `release`.
- `dry-run: true` — deprecated alias for `mode: plan`. Kept for back-compat.
- `package` input — removed.

### Field-by-field migration table

| 7.x | 8.x |
|---|---|
| `packages[].independent` | **removed** — one global version |
| `packages[].tag_prefix` | **removed** — use `git.tag_prefix` |
| `packages[].hooks.pre_release` | **removed** — run in CI before `sr release` |
| `packages[].hooks.build` | **removed** — run in CI between `sr prepare` and `sr release` |
| `packages[].hooks.post_release` | **removed** — use typed `publish:` or CI steps |
| `packages[].publish.command: "cargo publish"` | `publish: { type: cargo }` |
| `artifacts: ["dist/*"]` | `artifacts: [dist/app.tar.gz]` (literal) |
| `stage_files: ["*.lock"]` | `stage_files: [Cargo.lock]` (literal) |
| `sr release -p core` | `sr release` (whole-repo; no `-p`) |
| `sr status` | `sr plan` |
| `SR_VERSION` / `SR_TAG` (in user hooks) | set only for `publish: custom` commands |

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

### GitHub Action: v3 → v4

Update `@v3` → `@v4`:

```yaml
# Before
- uses: urmzd/sr@v3
# After
- uses: urmzd/sr@v4
```

**Inputs removed in v4:**

| v3 input | What happened |
|----------|---------------|
| `command` | Removed. v3 accepted any subcommand (`release`, `plan`, `commit`, `pr`, etc.). v4 always runs `sr release` or `sr status`. |
| `force` | Removed in v4 (restored in v5). |
| `config` | Removed. sr always reads `sr.yaml` from the repo root. |
| `artifacts` | Removed in v4 (restored in v5). v3 accepted newline/comma-separated globs. |
| `build-command` | Removed. Use `hooks.pre_release` in `sr.yaml` instead. |

**Execution model changes:**

| Aspect | v3 | v4 |
|--------|----|----|
| Command dispatch | Configurable — any sr subcommand via `command` input | Fixed — always `sr release` or `sr status --format json` |
| Arg passing | Array-based (`sr "${ARGS[@]}"`) | Simple string (`sr $CMD`) |
| Artifact handling | Parsed from newline/comma input, each passed as `--artifacts` | Not supported |
| Build command | Written to temp script, passed as `--build-command` | Removed |
| Logging | `::group::` blocks with verbose echo | Minimal, no grouping |
| Exit code 2 | Sets all outputs to empty strings | Only sets `released=false` |

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

### GitHub Action: v4 → v5

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

**Inputs restored from v3:**

| Input | v3 | v4 | v5 |
|-------|:---:|:---:|:---:|
| `force` | `false` | Removed | `false` — adds `--force` |
| `artifacts` | Newline/comma-separated | Removed | Space-separated, each passed as `--artifacts "glob"` |

**New inputs in v5:**

| Input | Default | Description |
|-------|---------|-------------|
| `package` | `""` | Target a specific monorepo package |
| `channel` | `""` | Release channel (e.g. canary, rc, stable) |
| `prerelease` | `""` | Pre-release identifier (e.g. alpha, beta, rc) |
| `stage-files` | `""` | Additional file globs to stage in release commit (space-separated) |
| `sign-tags` | `false` | Sign tags with GPG/SSH |
| `draft` | `false` | Create GitHub release as a draft |

**Execution model changes:**

| Aspect | v4 | v5 |
|--------|----|----|
| Arg passing | Simple string (`sr $CMD`) | `eval sr $CMD` (supports quoted globs) |
| Dirty tree check | sr refuses if working tree is dirty | Removed — unrelated files are harmless |
| CLI flag coverage | Only `--dry-run` | All `sr release` flags exposed as inputs |

**Inputs unchanged across all versions:**

| Input | Default |
|-------|---------|
| `dry-run` | `false` |
| `github-token` | `${{ github.token }}` |
| `git-user-name` | `sr[bot]` |
| `git-user-email` | `sr[bot]@urmzd.com` |
| `sha256` | `""` |

**Outputs unchanged across all versions:**

| Output | Description |
|--------|-------------|
| `version` | Released version |
| `previous-version` | Previous version |
| `tag` | Git tag created |
| `bump` | Bump level (major/minor/patch) |
| `floating-tag` | Floating major tag (e.g. `v3`) |
| `commit-count` | Commits included |
| `released` | `true`/`false` |
| `json` | Full release metadata as JSON |

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

## Migrating from 5.x to 6.x

v6 is a non-breaking release that was triggered by a version bump. There are no
breaking changes for users. All v5 config, action inputs, and CLI commands work
unchanged.

### What's new in v6

**MCP tools added:**

| Tool | Description |
|------|-------------|
| `sr_commit` | Now supports `breaking` flag — auto-adds `!` suffix and `BREAKING CHANGE:` footer |
| `sr_pr_template` | Generates PR template from branch commits and diff stats |
| `sr_pr_create` | Creates GitHub PR via `gh` CLI (title, body, labels, draft) |
| `sr_worktree` | Creates worktrees under `.sr/worktrees/` with metadata tracking |
| `sr_worktree_list` | Lists all worktrees with descriptions and creation dates |
| `sr_worktree_remove` | Removes worktree and cleans up metadata |

**`sr init` improvements:**

- No longer fails if `sr.yaml` already exists — each step (sr.yaml, .mcp.json,
  .gitignore) runs independently and skips files that already exist
- Automatically adds `.sr/` to `.gitignore` (used for worktree metadata)

### GitHub Action: v5 → v6

Update `@v5` → `@v6`. All inputs and outputs are unchanged:

```yaml
# Before
- uses: urmzd/sr@v5

# After
- uses: urmzd/sr@v6
```

### Worktree management via `.sr/`

Worktrees created via MCP (`sr_worktree`) are stored under `.sr/worktrees/`
instead of sibling directories. Each worktree gets a metadata file at
`.sr/worktrees/<branch>.json` tracking its purpose, description, and creation
date. The `.sr/` directory is automatically gitignored by `sr init`.

---

## Migrating from 7.0 to 7.1

v7.1 is **additive in config, subtractive in CLI**. Existing `sr.yaml` files continue to work. New capability is opt-in via `hooks.build`. One CLI flag (`--force`) and one workflow input pattern (`workflow_dispatch` with `force`) go away — recovery is now strictly via pushing a new commit, never re-running the same release. sr also uploads `sr-manifest.json` as the final asset on every release for completion tracking.

### What changed in the pipeline

Old order (v7.0):

```
pre_release hooks → bump → commit → tag → push → create release → upload artifacts → post_release hooks
```

New order (v7.1):

```
pre_release hooks → bump → build → validate → commit → tag → push → create release → upload artifacts → post_release hooks → upload manifest
```

Two new stages:

- **`build`**: runs configured `hooks.build` commands after version files have been bumped on disk. Binaries built here embed the new version.
- **`validate`**: when `hooks.build` is non-empty, every `artifacts:` glob must resolve to ≥1 file, else the release aborts before tag creation. Guarantees the tag invariant: a tag on remote implies the declared artifacts exist.

Two new behaviors:

- **Idempotent upload**: `UploadArtifacts` skips any asset whose basename is already present on the release. Safe to re-run.
- **Reconciliation warning**: at the start of `sr release`, sr inspects the latest remote tag's `sr-manifest.json`. If it declares artifacts that aren't present on the release, sr prints a warning and **proceeds** — the broken release stays as a dangling record, and the new release rolls forward on top. Tags with no manifest (legacy or aborted-before-manifest) pass silently. There is no `--force` flag; recovery is by pushing a new commit.

### Opt-in `hooks.build` (recommended)

Move build commands from external CI steps (or from `hooks.pre_release`, where they would have embedded the pre-bump version) into `hooks.build`:

```yaml
# v7.0 (still works, but binaries embed old version)
packages:
  - path: .
    artifacts: ["release-assets/*.tar.gz"]
    hooks:
      pre_release:
        - cargo build --release  # runs BEFORE version bump — wrong version
      post_release:
        - cargo publish

# v7.1 (build embeds new version, sr validates output)
packages:
  - path: .
    artifacts: ["release-assets/*.tar.gz"]
    hooks:
      pre_release:
        - cargo test  # validations that can abort the release
      build:
        - cargo build --release  # runs AFTER bump — embeds new version
        - mkdir -p release-assets
        - tar czf release-assets/sr.tar.gz target/release/sr
      post_release:
        - cargo publish
```

If `hooks.build` is empty, pipeline behavior is identical to v7.0 — no validation, no enforcement.

### GitHub Actions workflow changes

Workflows pin `urmzd/sr@v7` (floating major) — they pick up v7.1 automatically. Two universal cleanup steps for any sr-using workflow:

1. **Drop the `force` input from `workflow_dispatch`.** It no longer does anything; the action's `force:` input is gone. If you have a workflow_dispatch trigger purely for recovery, you can remove it entirely — recovery is just a normal `git push` of a fix commit.
2. **Drop the `force: ${{ inputs.force }}` parameter** anywhere it appears in `with:` blocks for `urmzd/sr@v7`.

Per-pattern guidance:

**Pattern A — build + sr in same job (teasr, fsrc, agentspec, github-insights, urmzd.com, zigbee-skill).** Move the build steps from the workflow into `hooks.build` in `sr.yaml`. The workflow reduces to a single `urmzd/sr@v7` step.

```yaml
# Before: separate build step in the workflow
- run: cargo build --release
- run: tar czf release-assets/app.tar.gz target/release/app
- uses: urmzd/sr@v7

# After: build lives in sr.yaml hooks.build; workflow just invokes sr
- uses: urmzd/sr@v7
```

**Pattern B — parallel matrix build → consolidate → sr (sr itself).** Keep the matrix. `hooks.build` runs in one process on one runner and cannot replace a cross-runner matrix. Leave `hooks.build` empty; ValidateArtifacts stays inactive (same safety level as v7.0). Reconciliation still applies.

**Pattern C — sr-only, no build (streamsafe, lazyspeak.nvim, mnemonist).** No change.

**Pattern D — post-release build (incipit, linear-gp, saige).** This pattern is structurally unreachable by sr's validation: sr completes (no artifacts declared → validate skipped → manifest uploaded as "complete"), then the workflow's post-release build runs outside sr. If that build fails, sr has no record and reconciliation won't trigger on the next release. Two options: (a) move the build into `hooks.build` on the same runner as sr, or (b) accept that this pattern has no binary-presence guarantee.

### Recovery: roll forward, never re-release the same commit

sr never re-releases the same commit. If a release breaks mid-pipeline (artifact upload died, post-release hook failed, CI runner dropped), the tag and partial release stay on GitHub as a dangling record. Recovery is to **push a new commit**:

```bash
# Scenario: sr uploaded 2 of 3 declared artifacts during v1.2.3, then CI runner died.
# To recover, push a fix (or an empty commit) and let sr cut v1.2.4 on top:
$ git commit --allow-empty -m "fix: trigger release after v1.2.3 partial"
$ git push
# CI runs sr release → warns about v1.2.3 being incomplete → proceeds → v1.2.4 ships.
# Floating major tag (v1) moves to v1.2.4. Users installing get the good release.
```

The broken v1.2.3 stays on GitHub as history, but no floating tag points to it and the next "latest" is v1.2.4.

There is **no** `--force` flag, no `workflow_dispatch` input for recovery, no `git checkout` step, no `cargo publish` retry. sr never tries to re-release the same version, so `cargo publish: already exists` never fires.

### Legacy releases

Releases created before v7.1 don't have `sr-manifest.json`. sr treats them as `Unknown` status, neither complete nor incomplete, and doesn't block future releases. The manifest-based reconciliation only applies to releases cut by v7.1+.

---

## Migrating from 6.x to 7.x

v7 is a **breaking release** — the entire `sr.yaml` config structure was redesigned.
The old config had everything under `commit` and `release` top-level sections. The
new config has 6 top-level sections: `git`, `commit`, `changelog`, `channels`, `vcs`,
`packages`.

### Breaking changes summary

| Area | What changed |
|------|-------------|
| `commit.pattern` | Removed — regex is derived automatically from type names |
| `commit.types` | Changed from flat list `[{name, bump, section}]` to grouped by bump level `{minor: [], patch: [], none: []}` |
| `commit.breaking_section` / `commit.misc_section` | Removed — now configured via `changelog.groups` |
| `release.changelog` | Moved from under `release` to top-level `changelog` section |
| `release.branches` | Removed — channels now specify which branch to release from |
| `release.tag_prefix`, `sign_tags`, `floating_tags` | Moved to `git` section; `floating_tags` renamed to `floating_tag`; new `v0_protection` field added |
| `release.draft`, `prerelease` | Moved into channel config (`channels.content[].draft`, `channels.content[].prerelease`) |
| `release.release_name_template` | Moved to `vcs.github.release_name_template` |
| `release.channels` (map) | Replaced by top-level `channels` (object with `default` + `content` array) |
| `release.versioning` | Replaced by `packages[].independent` (bool) |
| `release.version_files`, `artifacts`, `stage_files` | Moved to package config (`packages[].version_files`, etc.) |
| `hooks` (top-level) | Removed — only package-level `packages[].hooks.pre_release` / `post_release` |
| `packages[].name` | Removed — package name is now derived from `path` |
| Per-type fallback `pattern` | Removed |
| MCP server | Removed (`sr mcp serve` command gone, delete `.mcp.json`) |
| agentspec dependencies | Removed |
| File snapshot/rollback | Removed (unnecessary complexity) |

### Before / After config comparison

**v6 `sr.yaml`:**

```yaml
commit:
  pattern: '^(?P<type>\w+)(?:\((?P<scope>[^)]+)\))?(?P<breaking>!)?:\s+(?P<description>.+)'
  breaking_section: Breaking Changes
  misc_section: Miscellaneous
  types:
    - name: feat
      bump: minor
      section: Features
    - name: fix
      bump: patch
      section: Bug Fixes
    - name: perf
      bump: patch
      section: Performance
    - name: docs
      section: Documentation
    - name: refactor
      bump: patch
      section: Refactoring
    - name: revert
      section: Reverts
    - name: chore
    - name: ci
    - name: test
    - name: build
    - name: style

release:
  branches: [main]
  tag_prefix: "v"
  floating_tags: true
  sign_tags: false
  changelog:
    file: CHANGELOG.md
  version_files:
    - Cargo.toml
  stage_files:
    - Cargo.lock
  artifacts:
    - "target/release/sr-*"
  draft: false
  channels:
    canary:
      prerelease: canary
    stable: {}
  default_channel: stable

hooks:
  pre_release:
    - cargo build --release
  post_release:
    - cargo publish
```

**v7 `sr.yaml`:**

```yaml
git:
  tag_prefix: "v"
  floating_tag: true
  sign_tags: false
  v0_protection: true

commit:
  types:
    minor:
      - feat
    patch:
      - fix
      - perf
      - refactor
    none:
      - docs
      - revert
      - chore
      - ci
      - test
      - build
      - style

changelog:
  file: CHANGELOG.md
  groups:
    - name: breaking
      content:
        - breaking
    - name: features
      content:
        - feat
    - name: bug-fixes
      content:
        - fix
    - name: performance
      content:
        - perf
    - name: misc
      content:
        - chore
        - ci
        - test
        - build
        - style

channels:
  default: stable
  branch: main
  content:
    - name: canary
      prerelease: canary
    - name: stable

vcs:
  github:
    release_name_template: "{{ tag_name }}"

packages:
  - path: .
    version_files:
      - Cargo.toml
    stage_files:
      - Cargo.lock
    artifacts:
      - "target/release/sr-*"
    hooks:
      pre_release:
        - cargo build --release
      post_release:
        - cargo publish
```

### Field-by-field migration guide

| v6 field | v7 equivalent |
|----------|---------------|
| `commit.pattern` | Removed — derived from type names automatically |
| `commit.breaking_section` | `changelog.groups[].name` where `content: [breaking]` |
| `commit.misc_section` | `changelog.groups[].name` for catch-all types |
| `commit.types[].name` | Key in `commit.types.minor[]`, `commit.types.patch[]`, or `commit.types.none[]` |
| `commit.types[].bump: minor` | Move type name to `commit.types.minor[]` |
| `commit.types[].bump: patch` | Move type name to `commit.types.patch[]` |
| `commit.types[].bump: null` | Move type name to `commit.types.none[]` |
| `commit.types[].section` | `changelog.groups[].name` with the type in `content` |
| `commit.types[].pattern` | Removed — no per-type fallback patterns |
| `release.branches` | `channels.content[].branch` (each channel specifies its branch) |
| `release.tag_prefix` | `git.tag_prefix` |
| `release.floating_tags` | `git.floating_tag` |
| `release.sign_tags` | `git.sign_tags` |
| `release.changelog.file` | `changelog.file` |
| `release.changelog.template` | `changelog.template` (now a file path, not inline string) |
| `release.version_files` | `packages[].version_files` |
| `release.artifacts` | `packages[].artifacts` |
| `release.stage_files` | `packages[].stage_files` |
| `release.draft` | `channels.content[].draft` |
| `release.prerelease` | `channels.content[].prerelease` |
| `release.release_name_template` | `vcs.github.release_name_template` |
| `release.channels` (map) | `channels.content` (array) |
| `release.default_channel` | `channels.default` |
| `release.versioning: independent` | `packages[].independent: true` (default) |
| `release.versioning: fixed` | `packages[].independent: false` |
| `hooks.pre_release` | `packages[].hooks.pre_release` |
| `hooks.post_release` | `packages[].hooks.post_release` |
| `packages[].name` | Removed — derived from `path` |

### Step-by-step migration

1. **Replace `commit` section** — convert the flat types list to the grouped format:
   ```yaml
   # Before
   commit:
     types:
       - name: feat
         bump: minor
       - name: fix
         bump: patch
       - name: chore

   # After
   commit:
     types:
       minor: [feat]
       patch: [fix]
       none: [chore]
   ```

2. **Move `release.changelog` to top-level `changelog`** — and replace `breaking_section`/`misc_section` with `groups`:
   ```yaml
   # Before
   release:
     changelog:
       file: CHANGELOG.md
   commit:
     breaking_section: "Breaking Changes"

   # After
   changelog:
     file: CHANGELOG.md
     groups:
       - name: breaking
         content: [breaking]
       - name: features
         content: [feat]
       - name: bug-fixes
         content: [fix]
       - name: misc
         content: [chore, ci, test, build, style]
   ```

3. **Move git settings to `git` section:**
   ```yaml
   # Before
   release:
     tag_prefix: "v"
     floating_tags: true
     sign_tags: false

   # After
   git:
     tag_prefix: "v"
     floating_tag: true   # renamed: floating_tags → floating_tag
     sign_tags: false
     v0_protection: true  # new field
   ```

4. **Convert `release.channels` map to `channels` array:**
   ```yaml
   # Before
   release:
     branches: [main]
     channels:
       stable: {}
       canary:
         prerelease: canary
     default_channel: stable

   # After
   channels:
     default: stable
     branch: main              # single trunk branch
     content:
       - name: stable
       - name: canary
         prerelease: canary
   ```

5. **Move `release.version_files`, `artifacts`, `stage_files`, and `hooks` under `packages`:**
   ```yaml
   # Before
   release:
     version_files: [Cargo.toml]
     artifacts: ["target/release/sr-*"]
     stage_files: [Cargo.lock]
   hooks:
     pre_release: [cargo build --release]
     post_release: [cargo publish]

   # After
   packages:
     - path: .
       version_files: [Cargo.toml]
       artifacts: ["target/release/sr-*"]
       stage_files: [Cargo.lock]
       hooks:
         pre_release: [cargo build --release]
         post_release: [cargo publish]
   ```

6. **Move `release.release_name_template` to `vcs`:**
   ```yaml
   # Before
   release:
     release_name_template: "{{ tag_name }}"

   # After
   vcs:
     github:
       release_name_template: "{{ tag_name }}"
   ```

7. **Remove `packages[].name`** — the name is now derived from `path` automatically.

8. **Delete `.mcp.json`** — the MCP server has been removed:
   ```bash
   rm .mcp.json
   ```

9. **Update the action** — change `@v6` to `@v7`:
   ```yaml
   # Before
   - uses: urmzd/sr@v6

   # After
   - uses: urmzd/sr@v7
   ```

### MCP server removed

The MCP server (`sr mcp serve`) and `.mcp.json` have been removed entirely.

| Removed | What to do |
|---------|------------|
| `sr mcp serve` | Delete `.mcp.json`; use the `sr` agent skill instead |
| `.mcp.json` | Delete from project root |

### GitHub Action: v6 → v7

The action inputs and outputs are unchanged. Only the version tag changes:

```yaml
# Before
- uses: urmzd/sr@v6

# After
- uses: urmzd/sr@v7
```

---

## Migrating from 3.x directly to 7.x

Follow the 3.x→4.x, 4.x→5.x, 5.x→6.x, and 6.x→7.x sections above in order,
or use this quick checklist:

1. **Remove git hooks** — delete `.githooks/`, unset `core.hooksPath`
2. **Restructure sr.yaml** — rewrite config using the new 6-section format
3. **Delete `.mcp.json`** — MCP server was removed in v7
4. **Update action** — change `@v3` to `@v7`
5. **Update scripts** — replace `sr version`, `sr changelog`, `sr plan` with `sr status`

### Action input migration: v3 → v7

| v3 input | v7 equivalent |
|----------|---------------|
| `command: release` | Default (no input needed) |
| `command: plan` | `dry-run: "true"` |
| `command: <other>` | Removed — use CLI or agent skills |
| `artifacts: "dist/*\nbin/*"` | `artifacts: "dist/* bin/*"` (space-separated) |
| `build-command: "make"` | Removed — use `packages[].hooks.pre_release` in sr.yaml |
| `config: custom.yaml` | Removed — always reads `sr.yaml` |
| `force: true` | `force: "true"` (same) |

### CI/CD script migration

```bash
# 3.x
VERSION=$(sr version --short)
sr changelog --write
sr plan --format json

# 7.x
VERSION=$(sr status --format json | jq -r '.next_version')
# Changelog is written automatically by sr release
sr status --format json
```
