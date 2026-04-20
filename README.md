<p align="center">
  <h1 align="center">sr</h1>
  <p align="center">
    Release engineering CLI — automated semantic versioning from conventional commits.
    <br /><br />
    <a href="https://github.com/urmzd/sr/releases">Download</a>
    &middot;
    <a href="https://github.com/urmzd/sr/issues">Report Bug</a>
    &middot;
    <a href="https://github.com/urmzd/sr/blob/main/action.yml">GitHub Action</a>
  </p>
</p>

<p align="center">
  <a href="https://github.com/urmzd/sr/actions/workflows/ci.yml"><img src="https://github.com/urmzd/sr/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://crates.io/crates/sr-cli"><img src="https://img.shields.io/crates/v/sr-cli" alt="crates.io"></a>
  &nbsp;
  <a href="LICENSE"><img src="https://img.shields.io/github/license/urmzd/sr" alt="License"></a>
</p>

## Contents

- [Why?](#why)
- [Quick Start](#quick-start)
- [Installation](#installation)
- [Prerequisites](#prerequisites)
- [GitHub Enterprise Server (GHES)](#github-enterprise-server-ghes)
- [Branch Protection](#branch-protection)
- [Three verbs: plan, prepare, release](#three-verbs-plan-prepare-release)
- [Publishers](#publishers)
- [CLI Reference](#cli-reference)
- [Configuration](#configuration)
- [FAQ / Troubleshooting](#faq--troubleshooting)
- [Architecture](#architecture)
- [Design Philosophy](#design-philosophy)
- [Development](#development)
- [Contributing](#contributing)
- [Agent Skill](#agent-skill)
- [License](#license)

## Why?

Most release tools require Node.js, a pile of plugins, and still only handle the tagging step. **sr** is a single static binary that treats releases as state to reconcile — declare desired state in `sr.yaml`, let commits describe the diff, apply.

- **Terraform-shaped verbs** — `sr plan` previews, `sr prepare` writes manifests + changelog, `sr release` applies. Idempotent; safe to re-run.
- **Typed publishers** — built-in cargo / npm / docker / pypi / go. Each queries its registry before publishing, skips when already there.
- **Workspace-aware** — cargo / npm / pnpm / yarn / uv monorepos publish every member in one go; one tag, one version.
- **Release channels** — named channels (canary, rc, stable) for trunk-based promotion.
- **Agent skill** — ships as a portable [Agent Skill](https://agentskills.io) for Claude Code, Gemini CLI, Cursor, and other AI tools.
- **Single static binary** — no runtime, no plugins, no async runtime.

## Quick Start

```bash
# Initialize config. Pass an example name to scaffold from a template.
sr init
sr init --list                # show bundled templates
sr init pnpm-workspace        # write a specific example

# Preview the next release (version, tag, resource diff)
sr plan
sr plan --format json

# Bump manifest files + write changelog (no commit, no tag)
sr prepare

# Execute the release (bump if needed, commit, tag, push, release, publish)
sr release
sr release --dry-run

# Set up shell completions (bash)
sr completions bash >> ~/.bashrc
```

Most users run just `sr release` in CI. Use `sr prepare` when you need pre-built artifacts to embed the new version — see [`examples/ci/`](examples/ci/).

## Installation

### Shell installer (Linux/macOS)

```sh
curl -fsSL https://raw.githubusercontent.com/urmzd/sr/main/install.sh | sh
```

The installer automatically adds `~/.local/bin` to your `PATH` in your shell profile (`.zshrc`, `.bashrc`, or `config.fish`).

### GitHub Action (recommended)

```yaml
- uses: urmzd/sr@v8
  with:
    github-token: ${{ secrets.GITHUB_TOKEN }}
```

Minimal — release on every push to `main`:

```yaml
name: Release
on:
  push:
    branches: [main]

jobs:
  release:
    runs-on: ubuntu-latest
    permissions:
      contents: write
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0
      - uses: urmzd/sr@v8
```

Plan-only on pull requests (preview the next version without cutting a release):

```yaml
      - uses: urmzd/sr@v8
        with:
          mode: plan
```

Use outputs in subsequent steps:

```yaml
      - uses: urmzd/sr@v8
        id: sr
      - if: steps.sr.outputs.released == 'true'
        run: echo "Released ${{ steps.sr.outputs.version }}"
```

Verify the downloaded sr binary with a SHA256 checksum:

```yaml
      - uses: urmzd/sr@v8
        with:
          sha256: "abc123..."
```

For maximum security, pin the action to a full-length commit SHA:

```yaml
      - uses: urmzd/sr@<commit-sha>
        with:
          sha256: "abc123..."
```

Manual re-trigger with `workflow_dispatch` (useful when a previous release partially failed — re-runs reconcile any missing state idempotently, no special flag needed):

```yaml
name: Release
on:
  push:
    branches: [main]
  workflow_dispatch:

jobs:
  release:
    runs-on: ubuntu-latest
    permissions:
      contents: write
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0
      - uses: urmzd/sr@v8
```

#### Inputs

| Input | Description | Default |
|-------|-------------|---------|
| `mode` | `plan` \| `prepare` \| `release`. Default `release`. | `release` |
| `dry-run` | Deprecated alias for `mode: plan`. | `false` |
| `github-token` | GitHub token for creating releases | `${{ github.token }}` |
| `git-user-name` | Git author/committer name for the release commit and tag. Pass empty to let `sr.yaml` (`git.user.name`) or the repo's git config take over | `sr-releaser[bot]` |
| `git-user-email` | Git author/committer email for the release commit and tag. Pass empty to let `sr.yaml` (`git.user.email`) or the repo's git config take over | `sr-releaser[bot]@users.noreply.github.com` |
| `artifacts` | Literal paths to artifact files to upload (space-separated) | `""` |
| `channel` | Release channel (e.g. canary, rc, stable) | `""` |
| `prerelease` | Pre-release identifier (e.g. alpha, beta, rc) | `""` |
| `stage-files` | Additional literal paths to stage in the release commit (space-separated) | `""` |
| `sign-tags` | Sign tags with GPG/SSH | `false` |
| `draft` | Create GitHub release as a draft | `false` |
| `sha256` | Expected SHA256 checksum of the sr binary (hex string) | `""` |

#### Outputs

| Output | Description |
|--------|-------------|
| `version` | The released version (empty if no release) |
| `previous-version` | The previous version before this release (empty if first release) |
| `tag` | The git tag created for this release (empty if no release) |
| `bump` | The bump level applied (`major`/`minor`/`patch`, empty if no release) |
| `floating-tag` | The floating major tag (e.g. `v3`, empty if disabled or no release) |
| `commit-count` | Number of commits included in this release |
| `released` | Whether a release was created (`true`/`false`) |
| `json` | Full release metadata as JSON (empty if no release) |

### Binary download

Download the latest release for your platform from
[Releases](https://github.com/urmzd/sr/releases):

| Target | File |
|--------|------|
| Linux x86_64 (glibc) | `sr-x86_64-unknown-linux-gnu` |
| Linux aarch64 (glibc) | `sr-aarch64-unknown-linux-gnu` |
| Linux x86_64 (musl/static) | `sr-x86_64-unknown-linux-musl` |
| Linux aarch64 (musl/static) | `sr-aarch64-unknown-linux-musl` |
| macOS x86_64 | `sr-x86_64-apple-darwin` |
| macOS aarch64 | `sr-aarch64-apple-darwin` |
| Windows x86_64 | `sr-x86_64-pc-windows-msvc.exe` |

The MUSL variants are statically linked and work on any Linux distribution (Alpine, Debian, RHEL, etc.). Prefer these for maximum compatibility.

```bash
mkdir -p ~/.local/bin
chmod +x sr-* && mv sr-* ~/.local/bin/sr
```

Ensure `~/.local/bin` is on your `$PATH`.

### Build from source

```bash
cargo install --path crates/sr-cli
```

## Prerequisites

`sr release` calls the GitHub REST API directly — no external tools are needed. Authentication is via an environment variable:

```bash
export GH_TOKEN=ghp_xxxxxxxxxxxx   # or GITHUB_TOKEN
```

The GitHub Action sets this automatically via the `github-token` input. Dry-run mode (`sr release --dry-run`) works without a token.

## GitHub Enterprise Server (GHES)

`sr` works with GitHub Enterprise Server out of the box. The hostname is auto-detected from your git remote URL — changelog links, compare URLs, and API calls will point to the correct host automatically.

### Setup

Set your `GH_TOKEN` (or `GITHUB_TOKEN`) environment variable with a token that has access to your GHES instance:

```bash
export GH_TOKEN=ghp_xxxxxxxxxxxx
```

No additional host configuration is needed — `sr` derives the API base URL from the git remote hostname automatically (e.g. `ghes.example.com` → `https://ghes.example.com/api/v3`).

### How it works

1. `sr` reads the `origin` remote URL and extracts the hostname (e.g. `ghes.example.com`).
2. Changelog links and compare URLs use `https://<hostname>/owner/repo/...` instead of hardcoded `github.com`.
3. REST API calls are routed to `https://<hostname>/api/v3/...` automatically.

## Branch Protection

If your repository requires signed commits or restricts direct pushes to the release branch, use a **GitHub App** to authenticate `sr`. Commits pushed with a GitHub App installation token are automatically signed by GitHub and can bypass branch rulesets.

### Setup

**1. Create a GitHub App**

- Go to **GitHub Settings → Developer settings → GitHub Apps → New GitHub App**
- Name: e.g. `sr-bot`
- Homepage URL: your repo URL
- Uncheck **Webhook → Active**
- Repository permissions: **Contents → Read & write**
- Where can this app be installed: **Only on this account**
- Create the app, then **Generate a private key**
- Install the app on your repositories

**2. Store secrets**

Add these as repository or organization secrets:

| Secret | Value |
|--------|-------|
| `SR_APP_ID` | The App ID (from the App's settings page) |
| `SR_APP_PRIVATE_KEY` | The downloaded `.pem` file contents |

**3. Configure repository rulesets**

> Use **repository rulesets**, not legacy branch protection. Legacy branch protection does not support GitHub App bypass for signed commit requirements.

- Go to **repo Settings → Rules → Rulesets → New ruleset**
- Target branch: `main`
- Enable: **Require signed commits**, **Require a pull request before merging**
- Add your GitHub App to the **Bypass list**

### Workflow example

```yaml
jobs:
  release:
    runs-on: ubuntu-latest
    permissions:
      contents: write
    steps:
      - name: Generate App token
        id: app-token
        uses: actions/create-github-app-token@v1
        with:
          app-id: ${{ secrets.SR_APP_ID }}
          private-key: ${{ secrets.SR_APP_PRIVATE_KEY }}

      - uses: actions/checkout@v4
        with:
          fetch-depth: 0
          token: ${{ steps.app-token.outputs.token }}

      - uses: urmzd/sr@v8
        with:
          github-token: ${{ steps.app-token.outputs.token }}
```

## Three verbs: plan, prepare, release

`sr` is a release-state reconciler, not a task runner. Three verbs:

| Verb | Reads | Writes |
|---|---|---|
| `sr plan` | VCS + registries | — (preview only) |
| `sr prepare` | config + commits | manifest files + changelog (no git) |
| `sr release` | everything | commit, tag, push, release, upload, publish |

sr does not run user shell commands. Artifact builds happen in CI between `sr prepare` and `sr release` so binaries / wheels / packed tarballs embed the newly-bumped version.

### Single-job release

For repos where `cargo publish` / `npm publish` builds and uploads internally, one verb is enough:

```yaml
- uses: urmzd/sr@v8
```

### Multi-platform binaries (prepare → build matrix → release)

When you need pre-built binaries for multiple targets, split into three jobs:

```yaml
jobs:
  prepare:
    runs-on: ubuntu-latest
    outputs:
      version: ${{ steps.sr.outputs.version }}
    steps:
      - uses: actions/checkout@v4
      - uses: urmzd/sr@v8
        id: sr
        with: { mode: prepare }
      - uses: actions/upload-artifact@v4
        with:
          name: prepared-manifests
          path: "**/Cargo.toml CHANGELOG.md"

  build:
    needs: prepare
    strategy:
      matrix: { target: [x86_64-linux, aarch64-linux, x86_64-darwin, aarch64-darwin] }
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions/download-artifact@v4
        with: { name: prepared-manifests, path: . }
      - run: cargo build --release --target ${{ matrix.target }}
      # Binary now has the correct version baked in from the bumped Cargo.toml.
      - uses: actions/upload-artifact@v4

  release:
    needs: [prepare, build]
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions/download-artifact@v4
        with: { path: . }
      - uses: urmzd/sr@v8
```

Full worked examples per ecosystem live in [`examples/ci/`](examples/ci/).

## Publishers

Every package's `publish:` is a typed variant — sr handles the registry check + publish command internally, so users never write shell.

```yaml
packages:
  - path: .
    version_files: [Cargo.toml]
    publish:
      type: cargo                 # cargo publish to crates.io

  - path: packages/web
    version_files: [packages/web/package.json]
    publish:
      type: npm                   # npm publish; auto-detects pnpm / yarn
      workspace: true             # pnpm publish -r / npm publish --workspaces

  - path: services/api
    publish:
      type: docker
      image: ghcr.io/urmzd/api
      platforms: [linux/amd64, linux/arm64]
```

Supported types: `cargo`, `npm`, `docker`, `pypi`, `go`, `custom`. Each publisher queries its registry's API to decide if work is needed (e.g. `GET https://crates.io/api/v1/crates/<name>/<version>` — 200 means already published, skip). Re-running `sr release` on an already-published package is a noop. See [`examples/`](examples/) for one complete config per ecosystem.

### JSON output schema

All three verbs emit the same flat JSON to stdout on success:

```json
{
  "version": "1.2.3",
  "previous_version": "1.2.2",
  "tag": "v1.2.3",
  "bump": "patch",
  "floating_tag": "v1",
  "commit_count": 4
}
```

`sr plan` additionally includes a `resources` array (Terraform-style resource diff). Diagnostic messages go to stderr; stdout is always clean JSON (or empty on exit code 2, "no releasable changes").

## CLI Reference

### Commands

| Command | Description |
|---------|-------------|
| `sr plan` | Preview the next release — version, tag, resource diff. No side effects. |
| `sr prepare` | Bump version files + write changelog to disk. No commit, tag, or push. |
| `sr release` | Execute the release — commit, tag, push, create GH release, upload, publish. Idempotent. |
| `sr config` | Validate and display resolved configuration. |
| `sr init [example]` | Create `sr.yaml`. Pass an example name to scaffold from a template (`sr init --list`). |
| `sr completions` | Generate shell completions (bash, zsh, fish, powershell, elvish). |
| `sr update` | Update sr to the latest version. |
| `sr migrate` | Show migration guide. |

### Common flags

```bash
sr plan --format json           # machine-readable plan output
sr prepare --prerelease alpha   # bump to a prerelease (1.2.0-alpha.1)
sr release --dry-run            # preview without making changes
sr release -c canary            # release via named channel
sr release --prerelease rc      # produce 1.2.0-rc.1
sr release --sign-tags          # sign tags with GPG/SSH (git tag -s)
sr release --draft              # create GitHub release as a draft
sr release --artifacts dist/app.tar.gz   # upload literal path as release asset
sr release --stage-files Cargo.lock      # stage additional files in the release commit
sr config --resolved            # show config with defaults applied
sr init pnpm-workspace          # scaffold from a bundled example
sr init --list                  # list available examples
sr init --force                 # overwrite existing config
```

### Exit codes

| Code | Meaning |
|------|---------|
| `0` | Success. The planned/released metadata is printed to stdout as JSON. |
| `1` | Real error — configuration issue, git failure, VCS provider error, publish failure, etc. |
| `2` | No releasable changes — no new commits or no releasable commit types since the last tag. |

### Recovery from a broken release

The pipeline is idempotent. Re-running `sr release` after any mid-flight failure picks up exactly where it left off — tag created but release object missing? The next run creates the release object and skips tag creation. Assets uploaded but publish failed? The next run skips the upload and retries the publish.

No state files, no local checkpoints. Actual state lives in git + GitHub + registries; sr reads and converges. See [Architecture](#architecture) for the reconciler contract.

## Configuration

`sr` looks for `sr.yaml` in the repository root. All fields are optional and have sensible defaults.

Running `sr init` generates a fully-commented `sr.yaml` with every available option documented inline.

### Configuration reference

The config has 6 top-level sections — `git`, `commit`, `changelog`, `channels`, `vcs`, and `packages`:

#### `git`

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `git.tag_prefix` | `string` | `"v"` | Prefix for git tags (e.g. `v1.0.0`) |
| `git.floating_tag` | `bool` | `true` | Create floating major version tags (e.g. `v3` always points to the latest `v3.x.x` release) |
| `git.sign_tags` | `bool` | `false` | Sign annotated tags with GPG/SSH |
| `git.v0_protection` | `bool` | `true` | Prevent a breaking change from bumping `0.x` to `1.0.0` — stays at `0.x` |
| `git.user.name` | `string?` | `null` | Git author/committer name for the release commit and tag. When unset, sr uses the repo's git config (or the env fallback `SR_GIT_USER_NAME`) |
| `git.user.email` | `string?` | `null` | Git author/committer email. When unset, sr uses the repo's git config (or the env fallback `SR_GIT_USER_EMAIL`) |
| `git.skip_patterns` | `string[]` | `["[skip release]", "[skip sr]"]` | Substrings that, when present in a commit message, exclude that commit from release planning and the changelog |

#### `commit`

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `commit.types` | `object` | See below | Commit types grouped by bump level: `minor`, `patch`, `none` |
| `commit.types.minor` | `string[]` | `["feat"]` | Types that trigger a minor bump |
| `commit.types.patch` | `string[]` | `["fix", "perf", "refactor"]` | Types that trigger a patch bump |
| `commit.types.none` | `string[]` | `["docs", "revert", "chore", "ci", "test", "build", "style"]` | Types that do not trigger a release |

#### `changelog`

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `changelog.file` | `string?` | `"CHANGELOG.md"` | Path to the changelog file. Omit to skip changelog generation |
| `changelog.template` | `string?` | `null` | Path to a custom minijinja template file for changelog rendering |
| `changelog.groups` | `ChangelogGroup[]` | See below | Ordered list of changelog sections, each mapping type names to a heading |
| `changelog.groups[].name` | `string` | — (required) | Section heading name |
| `changelog.groups[].content` | `string[]` | — (required) | Commit types that appear in this section. Use `"breaking"` for breaking changes |

#### `channels`

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `channels.default` | `string` | `"stable"` | Default channel name used when no `--channel` flag is given |
| `channels.branch` | `string` | `"main"` | The trunk branch that triggers releases (all channels release from this branch) |
| `channels.content` | `Channel[]` | `[{name: "stable"}]` | Array of channel definitions |
| `channels.content[].name` | `string` | — (required) | Channel name (e.g. `canary`, `rc`, `stable`) |
| `channels.content[].prerelease` | `string?` | `null` | Pre-release identifier (e.g. `"canary"`, `"rc"`). None = stable |
| `channels.content[].draft` | `bool` | `false` | Create GitHub release as draft |

#### `vcs`

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `vcs.github.release_name_template` | `string?` | `null` | Minijinja template for the GitHub release name. Variables: `version`, `tag_name`, `tag_prefix` |

#### `packages`

Monorepos list one entry per package. Every package shares the same global version — `packages[]` describes *where to write versions, what to upload, and how to publish*, not *how to version*.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `packages[].path` | `string` | — (required) | Directory path relative to repo root. Used for per-package changelog sections and as the working directory for typed publishers. |
| `packages[].version_files` | `string[]` | `[]` (autodetected) | Manifest files to bump. **Literal paths, not globs.** |
| `packages[].version_files_strict` | `bool` | `false` | Fail on unsupported version file formats. |
| `packages[].stage_files` | `string[]` | `[]` | Additional literal paths to stage in the release commit (e.g. `["Cargo.lock"]`). |
| `packages[].artifacts` | `string[]` | `[]` | Literal paths to files to upload as release assets. Every entry must exist on disk before the tag is created. |
| `packages[].changelog` | `ChangelogConfig?` | inherits top-level | Changelog config override for this package. |
| `packages[].publish` | `PublishConfig?` | `null` | Publish target. See [Publishers](#publishers). |

#### `packages[].publish`

Typed enum — pick the registry type and sr handles the check + publish command. No user shell required.

| Type | Fields | Notes |
|---|---|---|
| `cargo` | `features: string[]`, `registry: string?`, `workspace: bool` | `cargo publish -p <name>`. `workspace: true` iterates `[workspace].members`. |
| `npm` | `registry: string?`, `access: "public"\|"restricted"?`, `workspace: bool` | Auto-detects pnpm / yarn / npm by lockfile. `workspace: true` uses `pnpm publish -r` / `npm publish --workspaces` / `yarn workspaces foreach`. |
| `docker` | `image: string`, `platforms: string[]`, `dockerfile: string?` | `docker buildx build --push` with multi-platform support. |
| `pypi` | `repository: string?`, `workspace: bool` | Auto-detects `uv` vs `twine`. `workspace: true` iterates `[tool.uv.workspace].members`. |
| `go` | — | No-op. Go modules publish via git tag, which sr already cuts. |
| `custom` | `command: string`, `check: string?`, `cwd: string?` | Escape hatch for registries without built-in support (helm, private Maven, etc.). |

### Example config

```yaml
# sr.yaml

git:
  tag_prefix: "v"
  floating_tag: true
  sign_tags: false
  v0_protection: true
  # Override the release commit/tag identity. When omitted, sr uses the
  # repo's git config (or SR_GIT_USER_NAME / SR_GIT_USER_EMAIL env vars).
  # user:
  #   name: "sr-releaser[bot]"
  #   email: "sr-releaser[bot]@users.noreply.github.com"
  skip_patterns:
    - "[skip release]"
    - "[skip sr]"

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
    - name: stable

# Optional: pre-release or draft channels
# channels:
#   default: stable
#   branch: main
#   content:
#     - name: canary
#       prerelease: canary
#     - name: rc
#       prerelease: rc
#       draft: true
#     - name: stable

vcs:
  github:
    release_name_template: "{{ tag_name }}"

packages:
  - path: .
    version_files:
      - Cargo.toml
    stage_files:
      - Cargo.lock
    # artifacts: literal paths, built in CI between `sr prepare` and `sr release`
    # artifacts:
    #   - release-assets/sr-x86_64-unknown-linux-musl
    #   - release-assets/sr-aarch64-apple-darwin
    publish:
      type: cargo
      workspace: true    # iterates every [workspace].members crate
```

More complete examples (pnpm, uv, docker, multi-language, custom) live in [`examples/`](examples/).

### Supported version files

| Filename | Key updated | Method | Notes |
|---|---|---|---|
| `Cargo.toml` | `package.version` or `workspace.package.version` | TOML parser | Preserves formatting/comments. Also updates `[workspace.dependencies]` entries that have both `path` and `version` fields. **Auto-discovers workspace members** |
| `package.json` | `version` | JSON parser | Pretty-printed output with trailing newline. **Auto-discovers npm workspace members** |
| `pyproject.toml` | `project.version` or `tool.poetry.version` | TOML parser | Preserves formatting/comments. Supports both PEP 621 and Poetry layouts. **Auto-discovers uv workspace members** |
| `pom.xml` | First `<version>` after `</parent>` (or `</modelVersion>`) | Regex | Skips the `<parent>` block to avoid changing the parent version |
| `build.gradle` | `version = '...'` or `version = "..."` | Regex | Only replaces the first match (avoids changing dependency versions) |
| `build.gradle.kts` | `version = "..."` | Regex | Only replaces the first match |
| `*.go` | `var Version = "..."` or `const Version string = "..."` | Regex | Matches the first `Version` variable/constant declaration |

#### Workspace auto-discovery

When bumping a workspace root, `sr` automatically finds and bumps all member manifests — no need to list them individually in `version_files`:

| Ecosystem | Root indicator | Members discovered via |
|-----------|---------------|----------------------|
| **Cargo** | `[workspace]` with `members` | `workspace.members` globs → member `Cargo.toml` files (skips `version.workspace = true`) |
| **npm** | `workspaces` array in `package.json` | `workspaces` globs → member `package.json` files (skips members without `version`) |
| **uv** | `[tool.uv.workspace]` with `members` | `tool.uv.workspace.members` globs → member `pyproject.toml` files (skips members without `version`) |

For example, a Cargo workspace only needs the root listed:

```yaml
packages:
  - path: .
    version_files:
      - Cargo.toml    # automatically bumps all workspace member Cargo.toml files
```

### Environment variables

| Variable | Context | Description |
|----------|---------|-------------|
| `GH_TOKEN` / `GITHUB_TOKEN` | Release | GitHub API token for creating releases and uploading artifacts. Not needed for `--dry-run` |
| `SR_GIT_USER_NAME` | Release | Fallback git author/committer name. Consulted only when neither `--git-user-name` nor `git.user.name` in `sr.yaml` is set |
| `SR_GIT_USER_EMAIL` | Release | Fallback git author/committer email. Same precedence as `SR_GIT_USER_NAME` |
| `SR_VERSION` | Release hooks | The new version string (e.g. `1.2.3`), set for `pre_release` and `post_release` hooks |
| `SR_TAG` | Release hooks | The new tag name (e.g. `v1.2.3`), set for `pre_release` and `post_release` hooks |

### Commit types

Commit types are grouped by their bump level under `commit.types`:

```yaml
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
```

The commit pattern is derived automatically from the type names. Any commit type not listed is silently ignored.

Breaking changes are detected in two ways per the [Conventional Commits](https://www.conventionalcommits.org/) spec:

1. **`!` suffix** — e.g. `feat!: new API` or `fix(core)!: rename method`
2. **`BREAKING CHANGE:` footer** — a line starting with `BREAKING CHANGE:` or `BREAKING-CHANGE:` in the commit body

Either form triggers a `major` bump regardless of the type's configured bump level.

#### Default commit-type mapping

| Type | Bump | Notes |
|------|------|-------|
| `feat` | minor | |
| `fix` | patch | |
| `perf` | patch | |
| `refactor` | patch | |
| `docs` | none | |
| `revert` | none | |
| `chore` | none | |
| `ci` | none | |
| `test` | none | |
| `build` | none | |
| `style` | none | |

Types in the `none` group do not trigger a release on their own. Changelog sections are configured separately under `changelog.groups`.

### Changelog behavior

When `changelog.file` is set:
- If the file doesn't exist, it's created with a `# Changelog` heading
- If it already exists, new entries are inserted after the first heading (prepended, not appended)
- Each entry has the format: `## <version> (<date>)`
- Sections appear in the order defined in `changelog.groups`
- Commits link to their full SHA on GitHub when the repo URL is available

### Changelog templates

Set `changelog.template` to a path pointing to a [minijinja](https://docs.rs/minijinja) (Jinja2-compatible) template file for full control over changelog output. When set, the default markdown format is bypassed entirely.

**Template context:**

| Variable | Type | Description |
|----------|------|-------------|
| `entries` | `ChangelogEntry[]` | Array of release entries (newest first) |
| `entries[].version` | `string` | Version string (e.g. `1.2.3`) |
| `entries[].date` | `string` | Release date (`YYYY-MM-DD`) |
| `entries[].commits` | `ConventionalCommit[]` | Array of commits in this release |
| `entries[].compare_url` | `string?` | GitHub compare URL (may be null) |
| `entries[].repo_url` | `string?` | Repository URL (may be null) |
| `entries[].commits[].sha` | `string` | Full commit SHA |
| `entries[].commits[].type` | `string` | Commit type (e.g. `feat`, `fix`) |
| `entries[].commits[].scope` | `string?` | Commit scope (may be null) |
| `entries[].commits[].description` | `string` | Commit description |
| `entries[].commits[].body` | `string?` | Commit body (may be null) |
| `entries[].commits[].breaking` | `bool` | Whether this is a breaking change |

**Example template:**

```yaml
changelog:
  file: CHANGELOG.md
  template: changelog.md.j2
```

`changelog.md.j2`:

```jinja
{% for entry in entries %}
## {{ entry.version }} ({{ entry.date }})
{% for c in entry.commits %}
- {% if c.scope %}**{{ c.scope }}**: {% endif %}{{ c.description }}
{% endfor %}
{% endfor %}
```

### Release execution order

1. **Parse commits** — determine version bump from commits since the last tag
2. **Bump version files** — every `packages[].version_files` entry across every package is rewritten on disk to the new version (workspace roots auto-expand to members)
3. **Write changelog** — `changelog.file` is updated (if configured)
4. **Validate artifacts** — every declared `artifacts` path must exist on disk (built in CI between `sr prepare` and `sr release`)
5. **Git commit** — bumped manifests + changelog + `stage_files` are committed as `chore(release): <tag> [skip ci]`
6. **Create and push tag** — annotated tag at HEAD (signed with GPG/SSH when `git.sign_tags: true`)
7. **Create/update floating tag** (if `git.floating_tag: true`)
8. **Create or update GitHub release** — PATCH-semantic update preserves existing assets on re-runs
9. **Upload artifacts** — MIME-type-aware uploads to the GitHub release (aggregated from every package)
10. **Publish** — typed publishers run per package; each queries its registry first and skips if already published

Every stage's `is_complete` check reads external state (tag existence, release object, asset basenames, registry versions) and short-circuits when converged. Re-running a completed release is a full noop.

### Release channels

Channels model trunk-based promotion — channels specify which branch they release from and optional pre-release identifiers:

```yaml
channels:
  default: stable
  branch: main
  content:
    - name: canary
      prerelease: canary
    - name: rc
      prerelease: rc
      draft: true
    - name: stable
```

```bash
sr release --channel canary     # 1.2.0-canary.1
sr release --channel rc         # 1.2.0-rc.1
sr release                      # 1.2.0 (stable, uses default channel)
```

**Channel fields:**

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `name` | `string` | — (required) | Channel name |
| `prerelease` | `string?` | `null` | Pre-release identifier. None = stable |
| `draft` | `bool` | `false` | Create GitHub release as draft |

### Pre-releases

Set `prerelease` on a channel to produce versions like `1.2.0-alpha.1` instead of `1.2.0`:

```yaml
channels:
  default: stable
  branch: main
  content:
    - name: alpha
      prerelease: alpha
    - name: stable
```

Or via CLI: `sr release --prerelease alpha`

**Behavior:**
- The version is based on the latest *stable* tag (pre-release tags are skipped when computing the base)
- The counter auto-increments by scanning existing tags: `1.2.0-alpha.1` → `1.2.0-alpha.2` → ...
- Switching identifiers resets the counter: `1.2.0-alpha.3` → `1.2.0-beta.1`
- The GitHub release is marked as a pre-release
- Floating tags are not updated for pre-releases
- Stable releases (`prerelease: null`) skip over pre-release tags entirely

### Monorepo support

One tag, one version, every package. Multiple packages in `packages[]` share the same global version — each one's `version_files` are bumped in lockstep on release.

```yaml
packages:
  - path: crates/core
    version_files: [crates/core/Cargo.toml]
    publish:
      type: cargo

  - path: crates/cli
    version_files: [crates/cli/Cargo.toml]
    stage_files: [crates/cli/Cargo.lock]
    publish:
      type: cargo
```

For workspace-aware ecosystems, one entry at the root is enough — `sr` walks the workspace:

```yaml
packages:
  - path: .
    version_files: [Cargo.toml]       # sr finds every [workspace].members crate
    stage_files: [Cargo.lock]
    publish:
      type: cargo
      workspace: true                  # publishes every member
```

Per-package changelog sections render automatically when more than one package has commits. The tag is always repo-wide (`git.tag_prefix` + semver); there are no per-package tags.

See [`examples/`](examples/) for cargo/npm/pnpm/uv workspace templates.

### Limitations

- **GitHub only** — the `VcsProvider` trait exists for extensibility, but only GitHub is implemented

## FAQ / Troubleshooting

<!-- fsrc src="docs/FAQ.md" -->
### Non-conventional commits are silently ignored

sr only understands commits that match the configured commit pattern (derived from type names defined in `commit.types`; follows [Conventional Commits](https://www.conventionalcommits.org/) by default). Commits that don't match — merge commits, JIRA-style messages, freeform text — are silently skipped during release planning. They won't trigger a version bump or appear in the changelog.

This means:
- **Merge commits** (`Merge pull request #123 from...`) — ignored, no impact
- **Squash merges with conventional titles** (`feat: add search`) — work perfectly
- **JIRA-style commits** (`PROJ-1234: fix login`) — ignored
- **Dependabot commits** (`Bump serde from 1.0 to 1.1`) — ignored
- **Freeform messages** (`fixed the bug`, `wip`) — ignored

If *all* commits since the last tag are non-conventional, sr exits with code 2 (no releasable changes).

### Skipping individual commits

Drop a skip token anywhere in a commit message to exclude it from release planning and the changelog:

```
feat: internal scratch work [skip release]
```

Out of the box, `[skip release]` and `[skip sr]` are recognized. Customize via `git.skip_patterns` in `sr.yaml`:

```yaml
git:
  skip_patterns:
    - "[skip release]"
    - "[skip sr]"
    - "DO-NOT-RELEASE"
```

Matching is a plain substring check against the full commit message, so the token can live in the subject or the body. sr also always filters its own `chore(release): …` commits regardless of configuration.

### Overriding the commit/tag author

By default sr uses whatever identity `git` resolves from its normal sources (`user.name` / `user.email` in the repo, `GIT_AUTHOR_*` env vars, etc.). To override without mutating the repo's git config, set either:

```yaml
# sr.yaml
git:
  user:
    name: "sr-releaser[bot]"
    email: "sr-releaser[bot]@users.noreply.github.com"
```

or pass the CLI flags on `sr release`:

```bash
sr release --git-user-name "sr-releaser[bot]" \
           --git-user-email "sr-releaser[bot]@users.noreply.github.com"
```

Precedence is **CLI flag > `sr.yaml` > `SR_GIT_USER_NAME` / `SR_GIT_USER_EMAIL` env > git's own resolution**. sr passes the identity via `git -c user.name=… -c user.email=…` per invocation, so persisted config is never rewritten.

### How merge strategies affect sr

sr reads the commit history from HEAD back to the latest tag. It doesn't care *how* commits landed on the branch — only what the commit messages say.

| Strategy | What sr sees | Impact |
|----------|-------------|--------|
| **Merge commit** (default) | The merge commit itself (`Merge pull request...`) + all individual commits from the branch | Merge commit is ignored (non-conventional). Individual commits are parsed normally. |
| **Squash merge** | A single commit with the PR title as the message | Works perfectly if the PR title is conventional (e.g. `feat: add search`). |
| **Rebase merge** | All individual commits replayed onto the branch | Each commit is parsed independently. Same as regular commits. |
| **Fast-forward** | All individual commits | Same as rebase. |

**Recommendation:** Squash merges with conventional PR titles give the cleanest release history — one commit per PR, one changelog entry per feature/fix.

### `sr release` exits with code 2

Exit code 2 means **no releasable commits** were found since the last tag. This is not an error — it means all commits since the last release are either non-bumping types (e.g. `chore`, `docs`, `ci`) or non-conventional messages that were skipped. To ship a release anyway, push a `feat:`/`fix:`/`perf:`/`refactor:` commit (an empty commit works: `git commit --allow-empty -m "fix: trigger release"`).

### Changelog is not generated

Set `changelog.file` in `sr.yaml` — changelog generation is opt-in:

```yaml
changelog:
  file: CHANGELOG.md
```

### Version files not updated

Ensure your manifest files are listed in `packages[].version_files` and match a [supported format](#supported-version-files).

### Tags are not signed

Set `git.sign_tags: true` in `sr.yaml` or pass `--sign-tags`. You must have a GPG or SSH signing key configured in git (`git config user.signingkey`).

### Migrating from v6.x

Run `sr migrate` to see the full migration guide, or read [migration.md](crates/sr-cli/docs/migration.md).
<!-- /fsrc -->

## Architecture

| Crate | Description |
|-------|-------------|
| [`sr-core`](crates/sr-core/) | Everything: config, release logic, git, GitHub API |
| [`sr-cli`](crates/sr-cli/) | CLI binary — command handlers, argument parsing |

`action.yml` in the repo root is the GitHub Action composite wrapper.

`sr` uses a pluggable `VcsProvider` trait and currently ships with GitHub support. GitLab, Bitbucket, and other providers can be added as separate crates implementing the same trait.

### Core traits

| Trait | Purpose |
|-------|---------|
| `GitRepository` | Tag discovery, commit listing, tag creation, push |
| `VcsProvider` | Remote release creation, updates, asset uploads, verification |
| `CommitParser` | Raw commit to conventional commit |
| `ChangelogFormatter` | Render changelog entries to text |
| `Publisher` | Registry-aware publish (cargo, npm, docker, pypi, go, custom) |
| `ReleaseStrategy` | Orchestrate plan / prepare / release |

## Design Philosophy

1. **VCS is state, commits are the diff.** Current state lives in git + GitHub + registries — never in an sr-managed file. The commits since the last tag define what changes we want to release. `sr` applies the diff.
2. **Reconciler, not task runner.** Every stage reads external state via `is_complete`, runs only when actual ≠ desired, and re-running a converged release is a noop. Partial failure recovery is automatic: re-run and `sr` picks up wherever reality diverges from the plan.
3. **No user shell hooks.** `sr` does not run arbitrary pre/post/build commands. Builds belong in CI between `sr prepare` and `sr release`; publishing is handled by typed registry publishers. The only user-shell escape hatch is `publish: custom`.
4. **Literal paths, not globs.** `artifacts`, `stage_files`, and `version_files` list exact filenames. Workspace member discovery inside Cargo.toml/package.json/pyproject.toml uses those tools' native manifest globs.
5. **Trunk-based flow.** Releases happen from a single branch; no release branches.
6. **Conventional commits as the versioning contract.** Commit messages drive the bump decision.
7. **Language-agnostic at the core.** `sr` knows git and semver; registry specifics live in the typed publishers.
8. **Skills-native.** AI assistants use sr through portable [Agent Skills](https://agentskills.io), not baked-in AI backends.

## Development

```bash
cargo test --workspace    # run tests
cargo clippy --workspace  # lint
cargo build               # build
```

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for development setup, code style, and PR guidelines.

## Agent Skill

This repo's conventions are available as portable agent skills in [`skills/`](skills/). Once installed, use `/sr` to plan, dry-run, or execute releases from conventional commits.

## License

[Apache-2.0](LICENSE)
