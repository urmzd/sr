<p align="center">
  <h1 align="center">sr</h1>
  <p align="center">
    Release engineering CLI: automated semantic versioning from conventional commits.
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
  &nbsp;
  <a href="https://github.com/urmzd/sr/releases"><img src="https://img.shields.io/github/v/release/urmzd/sr" alt="Release"></a>
  &nbsp;
  <a href="https://crates.io/crates/sr-cli"><img src="https://img.shields.io/crates/v/sr-cli" alt="crates.io"></a>
  &nbsp;
  <a href="LICENSE"><img src="https://img.shields.io/github/license/urmzd/sr" alt="License"></a>
</p>

## Contents

- [Features](#features)
- [Installation](#installation)
- [Quick Start](#quick-start)
- [Examples](#examples)
- [Usage](#usage)
- [Configuration](#configuration)
- [FAQ](#faq)
- [Development](#development)
- [Documentation](#documentation)
- [Agent Skill](#agent-skill)
- [License](#license)

## Features

- **Terraform-shaped verbs**: `sr plan` previews, `sr prepare` writes manifests and changelog, `sr release` applies. Idempotent and safe to re-run.
- **Typed publishers**: built-in cargo, npm, docker, pypi and go. Each queries its registry before publishing and skips when the version is already there.
- **Workspace-aware**: cargo, npm, pnpm, yarn and uv monorepos publish every member in one go, with one tag and one version.
- **Release channels**: named channels (canary, rc, stable) for trunk-based promotion.
- **Self-healing releases**: state lives in git, GitHub and registries. Re-run after a failure and sr finishes whatever is missing.
- **Single static binary**: no runtime, no plugins, no async runtime.

## Installation

### Prerequisites

| Requirement | Notes |
|-------------|-------|
| `git` | sr shells out to git for tags, commits and pushes |
| `GH_TOKEN` or `GITHUB_TOKEN` | Needed by `sr release` to call the GitHub REST API. Not needed for `--dry-run` |

The GitHub Action sets the token from its `github-token` input.

### Script (macOS / Linux)

```sh
curl -fsSL https://raw.githubusercontent.com/urmzd/sr/main/install.sh | sh
```

The installer adds `~/.local/bin` to your `PATH` in your shell profile (`.zshrc`, `.bashrc`, or `config.fish`).

### GitHub Action

```yaml
- uses: urmzd/sr@v9
  with:
    github-token: ${{ secrets.GITHUB_TOKEN }}
```

See [GitHub Action](#github-action) for workflows, inputs and outputs.

### Manual

Download a pre-built binary from the [releases page](https://github.com/urmzd/sr/releases/latest):

| Target | File |
|--------|------|
| Linux x86_64 (glibc) | `sr-x86_64-unknown-linux-gnu` |
| Linux aarch64 (glibc) | `sr-aarch64-unknown-linux-gnu` |
| Linux x86_64 (musl/static) | `sr-x86_64-unknown-linux-musl` |
| Linux aarch64 (musl/static) | `sr-aarch64-unknown-linux-musl` |
| macOS x86_64 | `sr-x86_64-apple-darwin` |
| macOS aarch64 | `sr-aarch64-apple-darwin` |
| Windows x86_64 | `sr-x86_64-pc-windows-msvc.exe` |

Prefer the statically linked musl variants on Linux: they run on any distribution (Alpine, Debian, RHEL, etc.).

```bash
mkdir -p ~/.local/bin
chmod +x sr-* && mv sr-* ~/.local/bin/sr
```

### Build from source

```bash
cargo install --path crates/cli
```

## Quick Start

```bash
curl -fsSL https://raw.githubusercontent.com/urmzd/sr/main/install.sh | sh

# Initialize config. Pass an example name to scaffold from a template.
sr init
sr init --list  # show bundled templates
sr init pnpm-workspace  # write a specific example

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

Most users run only `sr release` in CI. Use `sr prepare` when pre-built artifacts must embed the new version (see [`examples/ci/`](examples/ci/)).

## Examples

Each file in [`examples/`](examples/) is a complete `sr.yaml` for one release scenario. Copy the closest match and adjust paths.

| Example | Description |
|---------|-------------|
| [`cargo-single`](examples/cargo-single.yaml) | Single Rust crate to crates.io |
| [`cargo-workspace`](examples/cargo-workspace.yaml) | Rust workspace; every member publishes at the shared version |
| [`npm-single`](examples/npm-single.yaml) | Single npm package to registry.npmjs.org |
| [`npm-workspace`](examples/npm-workspace.yaml) | npm workspaces (`npm publish --workspaces`) |
| [`pnpm-workspace`](examples/pnpm-workspace.yaml) | pnpm monorepo (`pnpm publish -r`) |
| [`uv-workspace`](examples/uv-workspace.yaml) | uv / Python monorepo to PyPI |
| [`go`](examples/go.yaml) | Go module (tag-only; no registry) |
| [`docker`](examples/docker.yaml) | Container image to any OCI registry |
| [`multi-language`](examples/multi-language.yaml) | Rust core + Node CLI, one tag |
| [`custom`](examples/custom.yaml) | Arbitrary publish command + state check |

See [`examples/ci/`](examples/ci/) for matching `.github/workflows/release.yml` files.

## Usage

### Three verbs: plan, prepare, release

| Verb | Reads | Writes |
|---|---|---|
| `sr plan` | VCS + registries | None (preview only) |
| `sr prepare` | config + commits | manifest files + changelog (no git) |
| `sr release` | everything | commit, tag, push, release, upload, publish |

sr does not run user shell commands. Build artifacts in CI between `sr prepare` and `sr release` so binaries, wheels and packed tarballs embed the bumped version.

#### Single-job release

When `cargo publish` / `npm publish` builds and uploads internally, one verb is enough:

```yaml
- uses: urmzd/sr@v9
```

#### Multi-platform binaries (prepare, build matrix, release)

For pre-built binaries on multiple targets, split into three jobs:

```yaml
jobs:
  prepare:
    runs-on: ubuntu-latest
    outputs:
      version: ${{ steps.sr.outputs.version }}
    steps:
      - uses: actions/checkout@v4
      - uses: urmzd/sr@v9
        id: sr
        with:
          mode: prepare
      - uses: actions/upload-artifact@v4
        with:
          name: prepared-manifests
          path: "**/Cargo.toml CHANGELOG.md"

  build:
    needs: prepare
    strategy:
      matrix:
        target: [x86_64-linux, aarch64-linux, x86_64-darwin, aarch64-darwin]
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions/download-artifact@v4
        with:
          name: prepared-manifests
          path: .
      - run: cargo build --release --target ${{ matrix.target }}
      # Binary now has the correct version baked in from the bumped Cargo.toml.
      - uses: actions/upload-artifact@v4

  release:
    needs: [prepare, build]
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions/download-artifact@v4
        with:
          path: .
      - uses: urmzd/sr@v9
```

Full worked examples per ecosystem live in [`examples/ci/`](examples/ci/).

### GitHub Action

Release on every push to `main`. The `workflow_dispatch` trigger lets you re-run a partially failed release; sr reconciles missing state idempotently, no special flag needed:

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
      - uses: urmzd/sr@v9
```

Preview the next version on pull requests without cutting a release:

```yaml
- uses: urmzd/sr@v9
  with:
    mode: plan
```

Use outputs in later steps:

```yaml
- uses: urmzd/sr@v9
  id: sr
- if: steps.sr.outputs.released == 'true'
  run: echo "Released ${{ steps.sr.outputs.version }}"
```

Verify the downloaded binary with a SHA256 checksum, and pin the action to a full-length commit SHA for maximum security:

```yaml
- uses: urmzd/sr@<commit-sha>
  with:
    sha256: "abc123..."
```

#### Inputs

| Input | Description | Default |
|-------|-------------|---------|
| `mode` | `plan` \| `prepare` \| `release` | `release` |
| `dry-run` | Deprecated alias for `mode: plan` | `false` |
| `github-token` | GitHub token for creating releases | `${{ github.token }}` |
| `git-user-name` | Git author/committer name for the release commit and tag. Pass empty to defer to `sr.yaml` (`git.user.name`) or the repo's git config | `sr-releaser[bot]` |
| `git-user-email` | Git author/committer email for the release commit and tag. Pass empty to defer to `sr.yaml` (`git.user.email`) or the repo's git config | `sr-releaser[bot]@users.noreply.github.com` |
| `artifacts` | Literal paths to artifact files to upload (space-separated) | `""` |
| `channel` | Release channel (e.g. canary, rc, stable) | `""` |
| `prerelease` | Pre-release identifier (e.g. alpha, beta, rc) | `""` |
| `stage-files` | Additional literal paths to stage in the release commit (space-separated) | `""` |
| `sign-tags` | Sign tags with GPG/SSH | `false` |
| `draft` | Create GitHub release as a draft | `false` |
| `base-ref` | Commit to release (ref or SHA), locked at plan time so queued runs each release their own commit | `${{ github.sha }}` |
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

### GitHub Enterprise Server

sr works with GHES with no extra configuration. It reads the `origin` remote hostname (e.g. `ghes.example.com`) and uses it for changelog links, compare URLs, and REST API calls (`https://ghes.example.com/api/v3`). Set `GH_TOKEN` (or `GITHUB_TOKEN`) to a token with access to your GHES instance.

### Branch protection

If the release branch requires signed commits or restricts direct pushes, authenticate sr with a **GitHub App**. GitHub signs commits pushed with an App installation token, and the App can bypass branch rulesets.

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

**4. Use the App token in the workflow**

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

      - uses: urmzd/sr@v9
        with:
          github-token: ${{ steps.app-token.outputs.token }}
```

### Commands

| Command | Description |
|---------|-------------|
| `sr plan` | Preview the next release: version, tag, resource diff. No side effects. |
| `sr prepare` | Bump version files + write changelog to disk. No commit, tag, or push. |
| `sr release` | Execute the release: commit, tag, push, create GH release, upload, publish. Idempotent. |
| `sr config` | Validate and display resolved configuration. |
| `sr init [example]` | Create `sr.yaml`. Pass an example name to scaffold from a template (`sr init --list`). |
| `sr completions` | Generate shell completions (bash, zsh, fish, powershell, elvish). |
| `sr update` | Update sr to the latest version. |
| `sr migrate` | Show migration guide. |

### Common flags

```bash
sr plan --format json  # machine-readable plan output
sr prepare --prerelease alpha  # bump to a prerelease (1.2.0-alpha.1)
sr release --dry-run  # preview without making changes
sr release -c canary  # release via named channel
sr release --prerelease rc  # produce 1.2.0-rc.1
sr release --sign-tags  # sign tags with GPG/SSH (git tag -s)
sr release --draft  # create GitHub release as a draft
sr release --artifacts dist/app.tar.gz  # upload literal path as release asset
sr release --stage-files Cargo.lock  # stage additional files in the release commit
sr release --base-ref "$GITHUB_SHA"  # lock the release to a specific commit
sr config --resolved  # show config with defaults applied
sr init pnpm-workspace  # scaffold from a bundled example
sr init --list  # list available examples
sr init --force  # overwrite existing config
```

### JSON output

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

`sr plan` also includes a `resources` array (Terraform-style resource diff). Diagnostics and publisher output (`npm publish`, `cargo publish`, custom commands) go to stderr; stdout is always clean JSON, or empty on exit code 2.

### Exit codes

| Code | Meaning |
|------|---------|
| `0` | Success. The planned/released metadata is printed to stdout as JSON. |
| `1` | Real error: configuration issue, git failure, VCS provider error, publish failure, etc. |
| `2` | No releasable changes: no new commits or no releasable commit types since the last tag, and the release at that tag is complete. |

### Recovery from a broken release

Re-run `sr release` after any mid-flight failure; it picks up exactly where it left off. Tag created but release object missing? The next run creates the release object and skips tag creation. Assets uploaded but publish failed? The next run skips the upload and retries the publish.

This holds after the version is tagged too. With no new commits `sr release` would normally exit `2`, but when the checkout is the latest tag's commit it first reads that release's actual state: the release object, the declared assets, and each publisher's registry check. Anything missing is completed for the tagged version (no new commit, no new tag) and the run exits `0` with that version's JSON. A complete release still exits `2`.

sr only resumes what it can observe on the released tree, which sets three limits:

- The checkout must be the tagged commit. Re-dispatch the workflow on the release branch rather than using "Re-run failed jobs", which checks out the commit from before the release commit.
- A `custom` publisher needs a `check` command to be retried on its own; without one its state is unknown.
- Draft releases are skipped, since GitHub cannot look a draft up by tag.

sr keeps no state files or local checkpoints.

### Limitations

- **GitHub only**: GitHub.com and GitHub Enterprise Server are the only supported VCS providers.

## Configuration

sr reads `sr.yaml` from the repository root. Every field is optional and has a default. Run `sr init` to generate a fully-commented `sr.yaml` with every option documented inline.

The config has six top-level sections: `git`, `commit`, `changelog`, `channels`, `vcs`, and `packages`.

### `git`

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `git.tag_prefix` | `string` | `"v"` | Prefix for git tags (e.g. `v1.0.0`) |
| `git.floating_tag` | `bool` | `true` | Create floating major version tags (e.g. `v3` always points to the latest `v3.x.x` release) |
| `git.sign_tags` | `bool` | `false` | Sign annotated tags with GPG/SSH |
| `git.v0_protection` | `bool` | `true` | Prevent a breaking change from bumping `0.x` to `1.0.0`; stays at `0.x` |
| `git.user.name` | `string?` | `null` | Git author/committer name for the release commit and tag. When unset, sr uses the repo's git config (or the env fallback `SR_GIT_USER_NAME`) |
| `git.user.email` | `string?` | `null` | Git author/committer email. When unset, sr uses the repo's git config (or the env fallback `SR_GIT_USER_EMAIL`) |
| `git.skip_patterns` | `string[]` | `["[skip release]", "[skip sr]"]` | Substrings that, when present in a commit message, exclude that commit from release planning and the changelog |

Identity precedence: `--git-user-name` / `--git-user-email` flags > `git.user` in `sr.yaml` > `SR_GIT_USER_NAME` / `SR_GIT_USER_EMAIL` env > git's own resolution. sr passes the identity via `git -c user.name=… -c user.email=…` per invocation and never rewrites persisted git config. `skip_patterns` is a plain substring match against the full commit message, so the token can live in the subject or the body.

### `commit`

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `commit.types` | `object` | See below | Commit types grouped by bump level: `minor`, `patch`, `none` |
| `commit.types.minor` | `string[]` | `["feat"]` | Types that trigger a minor bump |
| `commit.types.patch` | `string[]` | `["fix", "perf", "refactor"]` | Types that trigger a patch bump |
| `commit.types.none` | `string[]` | `["docs", "revert", "chore", "ci", "test", "build", "style"]` | Types that do not trigger a release |

sr derives the commit pattern from the type names and ignores any type not listed. Breaking changes are detected per the [Conventional Commits](https://www.conventionalcommits.org/) spec and trigger a `major` bump regardless of type:

1. **`!` suffix**: e.g. `feat!: new API` or `fix(core)!: rename method`
2. **`BREAKING CHANGE:` footer**: a line starting with `BREAKING CHANGE:` or `BREAKING-CHANGE:` in the commit body

### `changelog`

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `changelog.file` | `string?` | `"CHANGELOG.md"` | Path to the changelog file. Omit to skip changelog generation |
| `changelog.template` | `string?` | `null` | Path to a custom minijinja template file for changelog rendering |
| `changelog.groups` | `ChangelogGroup[]` | See below | Ordered list of changelog sections, each mapping type names to a heading |
| `changelog.groups[].name` | `string` | None (required) | Section heading name |
| `changelog.groups[].content` | `string[]` | None (required) | Commit types that appear in this section. Use `"breaking"` for breaking changes |

When `changelog.file` is set:

- If the file doesn't exist, it's created with a `# Changelog` heading
- If it exists, new entries are inserted after the first heading (prepended, not appended)
- Each entry has the format: `## <version> (<date>)`
- Sections appear in the order defined in `changelog.groups`
- Commits link to their full SHA on GitHub when the repo URL is available

#### Changelog templates

Set `changelog.template` to a [minijinja](https://docs.rs/minijinja) (Jinja2-compatible) template file for full control over changelog output. When set, it replaces the default markdown format entirely.

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

### `channels`

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `channels.default` | `string` | `"stable"` | Default channel name used when no `--channel` flag is given |
| `channels.branch` | `string` | `"main"` | The trunk branch that triggers releases (all channels release from this branch) |
| `channels.content` | `Channel[]` | `[{name: "stable"}]` | Array of channel definitions |
| `channels.content[].name` | `string` | None (required) | Channel name (e.g. `canary`, `rc`, `stable`) |
| `channels.content[].prerelease` | `string?` | `null` | Pre-release identifier (e.g. `"canary"`, `"rc"`). None = stable |
| `channels.content[].draft` | `bool` | `false` | Create GitHub release as draft |

Channels model trunk-based promotion from a single branch:

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
sr release --channel canary  # 1.2.0-canary.1
sr release --channel rc  # 1.2.0-rc.1
sr release  # 1.2.0 (stable, uses default channel)
```

#### Pre-releases

Set `prerelease` on a channel, or pass `sr release --prerelease alpha`, to produce versions like `1.2.0-alpha.1` instead of `1.2.0`:

- The version is based on the latest *stable* tag (pre-release tags are skipped when computing the base)
- The counter auto-increments by scanning existing tags: `1.2.0-alpha.1` → `1.2.0-alpha.2` → ...
- Switching identifiers resets the counter: `1.2.0-alpha.3` → `1.2.0-beta.1`
- The GitHub release is marked as a pre-release
- Floating tags are not updated for pre-releases
- Stable releases (`prerelease: null`) skip over pre-release tags entirely

### `vcs`

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `vcs.github.release_name_template` | `string?` | `null` | Minijinja template for the GitHub release name. Variables: `version`, `tag_name`, `tag_prefix` |

### `packages`

List one entry per package. Every package shares the same global version: `packages[]` declares *where to write versions, what to upload, and how to publish*, not *how to version*.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `packages[].path` | `string` | None (required) | Directory path relative to repo root. Used for per-package changelog sections and as the working directory for typed publishers. |
| `packages[].version_files` | `string[]` | `[]` (autodetected) | Manifest files to bump. **Literal paths, not globs.** |
| `packages[].version_files_strict` | `bool` | `false` | Fail on unsupported version file formats. |
| `packages[].stage_files` | `string[]` | `[]` | Additional literal paths to stage in the release commit (e.g. `["Cargo.lock"]`). |
| `packages[].artifacts` | `string[]` | `[]` | Literal paths to files to upload as release assets. Every entry must exist on disk before the tag is created. |
| `packages[].changelog` | `ChangelogConfig?` | inherits top-level | Changelog config override for this package. |
| `packages[].publish` | `PublishConfig?` | `null` | Publish target. See [Publishers](#publishers). |

#### Publishers

`packages[].publish` is a typed variant: pick the registry type and sr runs the registry check and publish command, with no user shell.

```yaml
packages:
  - path: .
    version_files: [Cargo.toml]
    publish:
      type: cargo  # cargo publish to crates.io

  - path: packages/web
    version_files: [packages/web/package.json]
    publish:
      type: npm  # npm publish; auto-detects pnpm / yarn
      workspace: true  # pnpm publish -r / npm publish --workspaces

  - path: services/api
    publish:
      type: docker
      image: ghcr.io/urmzd/api
      platforms: [linux/amd64, linux/arm64]
```

| Type | Fields | Notes |
|---|---|---|
| `cargo` | `features: string[]`, `registry: string?`, `workspace: bool` | `cargo publish -p <name>`. `workspace: true` iterates `[workspace].members`. |
| `npm` | `registry: string?`, `access: "public"\|"restricted"?`, `workspace: bool` | Auto-detects pnpm / yarn / npm by lockfile. `workspace: true` uses `pnpm publish -r` / `npm publish --workspaces` / `yarn workspaces foreach`. |
| `docker` | `image: string`, `platforms: string[]`, `dockerfile: string?` | `docker buildx build --push` with multi-platform support. |
| `pypi` | `repository: string?`, `workspace: bool`, `dist_dir: string?` | Auto-detects `uv` vs `twine`. `workspace: true` iterates `[tool.uv.workspace].members`; each member's wheel + sdist are resolved from `<package_path>/<dist_dir>` (default `dist/`, matching `uv build --all`). |
| `go` | None | No-op. Go modules publish via git tag, which sr already cuts. |
| `custom` | `command: string`, `check: string?`, `cwd: string?` | Escape hatch for registries without built-in support (helm, private Maven, etc.). |

Each publisher queries its registry to decide if work is needed (e.g. `GET https://crates.io/api/v1/crates/<name>/<version>`: 200 means already published, skip). Re-running `sr release` on an already-published package is a noop.

#### Monorepos

One tag, one version, every package. Each package's `version_files` are bumped in lockstep:

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

For workspace-aware ecosystems, one entry at the root is enough:

```yaml
packages:
  - path: .
    version_files: [Cargo.toml]  # sr finds every [workspace].members crate
    stage_files: [Cargo.lock]
    publish:
      type: cargo
      workspace: true  # publishes every member
```

Per-package changelog sections render automatically when more than one package has commits. The tag is always repo-wide (`git.tag_prefix` + semver); there are no per-package tags.

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

When bumping a workspace root, sr finds and bumps every member manifest, so list only the root in `version_files`:

| Ecosystem | Root indicator | Members discovered via |
|-----------|---------------|----------------------|
| **Cargo** | `[workspace]` with `members` | `workspace.members` globs → member `Cargo.toml` files (skips `version.workspace = true`) |
| **npm** | `workspaces` array in `package.json` | `workspaces` globs → member `package.json` files (skips members without `version`) |
| **uv** | `[tool.uv.workspace]` with `members` | `tool.uv.workspace.members` globs → member `pyproject.toml` files (skips members without `version`) |

sr keeps lock files and intra-workspace dependency requirements in sync at the same time. `Cargo.lock`, `uv.lock`, `poetry.lock` and `package-lock.json` are rewritten in place and staged in the release commit; Cargo path deps (`{ path = ..., version = ... }`) and npm sibling ranges (`^1.0.0`) are retargeted so a major bump doesn't leave the workspace unresolvable. This is plain data editing: no `cargo update`, `uv lock`, `poetry lock` or `npm install`, and no network access. Registry entries and their hashes are untouched. See the [FAQ](docs/FAQ.md#does-updating-lock-files-need-a-network-resolve) for the matching rules and verified format versions.

### Environment variables

| Variable | Context | Description |
|----------|---------|-------------|
| `GH_TOKEN` / `GITHUB_TOKEN` | Release | GitHub API token for creating releases and uploading artifacts. Not needed for `--dry-run` |
| `SR_GIT_USER_NAME` | Release | Fallback git author/committer name. Consulted only when neither `--git-user-name` nor `git.user.name` in `sr.yaml` is set |
| `SR_GIT_USER_EMAIL` | Release | Fallback git author/committer email. Same precedence as `SR_GIT_USER_NAME` |
| `SR_BASE_REF` | Plan / Prepare / Release | Fallback for `--base-ref`: the commit the release is locked to. Consulted only when the flag is not given; empty means lock to HEAD at plan time |
| `SR_VERSION` | Publish | The new version string (e.g. `1.2.3`), set in the environment of publisher commands, including `custom` |
| `SR_TAG` | Publish | The new tag name (e.g. `v1.2.3`), set in the environment of publisher commands, including `custom` |

### Release execution order

1. **Parse commits**: determine version bump from commits since the last tag
2. **Bump version files**: every `packages[].version_files` entry across every package is rewritten on disk to the new version (workspace roots auto-expand to members)
3. **Write changelog**: `changelog.file` is updated (if configured)
4. **Validate artifacts**: every declared `artifacts` path must exist on disk (built in CI between `sr prepare` and `sr release`)
5. **Git commit**: bumped manifests + changelog + `stage_files` are committed as `chore(release): <tag> [skip ci]`
6. **Create and push tag**: annotated tag at HEAD (signed with GPG/SSH when `git.sign_tags: true`)
7. **Create/update floating tag** (if `git.floating_tag: true`)
8. **Create or update GitHub release**: PATCH-semantic update preserves existing assets on re-runs
9. **Upload artifacts**: MIME-type-aware uploads to the GitHub release (aggregated from every package)
10. **Publish**: typed publishers run per package; each queries its registry first and skips if already published

Each stage checks external state (tag existence, release object, asset basenames, registry versions) and skips when already converged. Re-running a completed release is a full noop.

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
      workspace: true  # iterates every [workspace].members crate
```

More complete examples (pnpm, uv, docker, multi-language, custom) live in [`examples/`](examples/).

## FAQ

<!-- fsrc src="docs/FAQ.md" -->
### Non-conventional commits are silently ignored

sr only understands commits that match the configured commit pattern (derived from type names defined in `commit.types`; follows [Conventional Commits](https://www.conventionalcommits.org/) by default). Commits that don't match (merge commits, JIRA-style messages, freeform text) are silently skipped during release planning. They won't trigger a version bump or appear in the changelog.

This means:
- **Merge commits** (`Merge pull request #123 from...`): ignored, no impact
- **Squash merges with conventional titles** (`feat: add search`): work perfectly
- **JIRA-style commits** (`PROJ-1234: fix login`): ignored
- **Dependabot commits** (`Bump serde from 1.0 to 1.1`): ignored
- **Freeform messages** (`fixed the bug`, `wip`): ignored

If *all* commits since the last tag are non-conventional, sr exits with code 2 (no releasable changes).

### How merge strategies affect sr

sr reads the commit history from HEAD back to the latest tag. It doesn't care *how* commits landed on the branch, only what the commit messages say.

| Strategy | What sr sees | Impact |
|----------|-------------|--------|
| **Merge commit** (default) | The merge commit itself (`Merge pull request...`) + all individual commits from the branch | Merge commit is ignored (non-conventional). Individual commits are parsed normally. |
| **Squash merge** | A single commit with the PR title as the message | Works perfectly if the PR title is conventional (e.g. `feat: add search`). |
| **Rebase merge** | All individual commits replayed onto the branch | Each commit is parsed independently. Same as regular commits. |
| **Fast-forward** | All individual commits | Same as rebase. |

**Recommendation:** Squash merges with conventional PR titles give the cleanest release history: one commit per PR, one changelog entry per feature/fix.

### `sr release` exits with code 2

Exit code 2 means **no releasable commits** were found since the last tag. Not an error: all commits since the last release are non-bumping types (e.g. `chore`, `docs`, `ci`) or non-conventional messages. To force a release, push a `feat:`/`fix:`/`perf:`/`refactor:` commit (an empty commit works: `git commit --allow-empty -m "fix: trigger release"`).

### Changelog is not generated

Set `changelog.file` in `sr.yaml`. Changelog generation is opt-in:

```yaml
changelog:
  file: CHANGELOG.md
```

### Version files not updated

Ensure your manifest files are listed in `packages[].version_files` and match a [supported format](../README.md#supported-version-files). Paths must be literal, no glob expansion.

### Tags are not signed

Set `git.sign_tags: true` in `sr.yaml` or pass `--sign-tags`. You must have a GPG or SSH signing key configured in git (`git config user.signingkey`).

### How do binaries get the correct version embedded?

Most build tools read the version from a manifest at compile time:
- `cargo build` reads `CARGO_PKG_VERSION` from `Cargo.toml`
- `npm pack` reads `package.json`
- `uv build` reads `pyproject.toml`

Run `sr prepare` **before** your build step so the bumped manifest is on disk when the build runs. Then `sr release` commits, tags, uploads, and publishes. See [examples/ci/cargo-multi-platform.yml](../examples/ci/cargo-multi-platform.yml) for the three-job shape (prepare → matrix build → release).

### Why doesn't sr run build commands itself?

`sr` is a release-state reconciler, not a task runner. It writes versions, creates tags + releases, invokes typed registry publishers (`cargo publish`, `npm publish`, `docker buildx build --push`, `uv publish`). Running arbitrary shell commands is a CI concern, not sr's.

The one escape hatch is `publish: custom`, which takes a shell command for registries without a built-in publisher (helm, private Maven, etc.).

### Does updating lock files need a network resolve?

No. After bumping the manifests, sr rewrites the affected entries in `Cargo.lock`, `uv.lock`, `poetry.lock` and `package-lock.json` in place (as TOML and JSON, never by invoking a resolver), then stages the lockfile in the release commit. It never shells out to `cargo update`, `uv lock`, `poetry lock` or `npm install`, so the release job needs no registry access and no repo needs a hook to keep its lockfile in sync.

Only locally-sourced entries are touched:

| Lock | Entries rewritten | Left alone |
|---|---|---|
| `Cargo.lock` | `[[package]]` with no `source` | anything with a `source` (registry deps) |
| `uv.lock` | `[[package]]` whose `source` is `editable`, `virtual`, or `directory` | `registry` sources |
| `poetry.lock` | `[[package]]` with `[package.source] type = "directory"` | registry and git sources; `content-hash` covers declared specs, not resolved versions |
| `package-lock.json` | the root `version`, plus `packages` entries naming a workspace member | registry entries, integrity hashes, `link: true` pointers |

`pnpm-lock.yaml` and `yarn.lock` need no sync at all: pnpm records workspace deps as `link:../core` and yarn omits members entirely, so neither stores a member version that a bump could invalidate.

Intra-workspace pins don't go stale either, but each ecosystem gets there differently. uv drops the version specifier for anything resolved through `[tool.uv.sources]` with `workspace = true`, so nothing needs rewriting. Cargo and npm do enforce the requirement, so sr retargets it: a Cargo path dep carrying `{ path = ..., version = ... }` moves to the new version, and an npm sibling range moves from `^1.0.0` to `^2.0.0`. Ranges that stay satisfied (`>=1.0.0`), wildcards, and protocol specs (`workspace:*`, `file:../core`) are deliberately left alone.

Because this reproduces a derived file, each rewrite is pinned to the format it was verified against: `Cargo.lock` v4, `uv.lock` v1 (revision 2), `poetry.lock` lock-version 2.x, `package-lock.json` v3. A newer structural version is refused outright rather than half-edited, because a partially-updated lock in a release commit is worse than a failed release.

Those pins are held honest by a conformance suite (`just conformance`, also run weekly in CI) that builds a fixture, locks it with the real tool, bumps it with sr, re-locks, and requires the files be byte-identical. All four ecosystems pass, including a uv workspace with registry dependencies and environment markers.

### Does sr support cross-compilation?

Not directly. Run your matrix in CI between `sr prepare` and `sr release`. Every build job downloads the prepared manifests (via `actions/upload-artifact` + `download-artifact`), builds for its target platform with the correct version embedded, and uploads its binary. The release job then downloads everything and runs `sr release` to tag + upload. See [examples/ci/cargo-multi-platform.yml](../examples/ci/cargo-multi-platform.yml).

### What happens if a release fails mid-flight?

Re-run `sr release`. Every stage has a strict `is_complete` check reading external state (tag exists? release object exists? assets uploaded? package on registry?). The pipeline picks up exactly where it left off. There's no state file to corrupt.

If the failure came after the tag was pushed, run it from the tagged commit (re-dispatch the workflow on the release branch). sr sees there are no new commits, checks the tagged release, and finishes whatever is missing: release object, assets, publish. See [Recovery from a broken release](../README.md#recovery-from-a-broken-release) for the limits.

### What if the branch moves while a release is running?

Nothing changes about what gets released. `sr plan` resolves the base commit once (HEAD, or `--base-ref`) and locks the release to it: the commit walk is bounded by that SHA, the tag targets exactly that commit (plus the `chore(release)` commit built on it), and the branch push carries only that SHA. With queued CI runs on a busy trunk, each run releases its own commit. If the remote branch has already advanced, the branch push is skipped with a warning and the release completes; the release commit stays reachable through the tag.

If the *local checkout* moves between plan and execute (something rewrote HEAD under sr, or `--base-ref` doesn't match the checkout), sr refuses with a `base ref moved` error: bumping files on a tree that isn't the planned commit would corrupt the release.

### Monorepo with one release per package?

Not supported. `sr` releases one tag per repo, one version across every package. Per-package tags (`core/v1.2.0`, `cli-v3.0.0`) are deliberately out of scope; that model is what changesets / Lerna are for.

For workspace-aware ecosystems, declare one entry at the workspace root with `publish.workspace: true`; every member publishes at the shared version.

### Migrating from v7.x

Run `sr migrate` or read [migration.md](../crates/cli/docs/migration.md). The v8 jump is breaking: `sr status` → `sr plan`; `packages[].independent` / `tag_prefix` / `hooks` are gone; `publish:` becomes a typed enum; globs in `artifacts`/`stage_files` become literal paths; `sr-manifest.json` is no longer produced.
<!-- /fsrc -->

## Development

```bash
cargo test --workspace  # run tests
cargo clippy --workspace  # lint
cargo build  # build
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for development setup, code style, and PR guidelines.

## Documentation

- [FAQ](docs/FAQ.md)
- [Migration guide](crates/cli/docs/migration.md)
- [Example configs](examples/README.md)
- [CI workflow examples](examples/ci/README.md)
- [Changelog](CHANGELOG.md)
- [Contributing](CONTRIBUTING.md)

## Agent Skill

This repo's conventions are available as portable agent skills in [`skills/`](skills/). Once installed, use `/sr` to plan, dry-run, or execute releases from conventional commits.

## License

[Apache-2.0](LICENSE)
