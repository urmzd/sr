# sr — Semantic Release

A single-binary, zero-dependency semantic release tool for any language.

[![CI](https://github.com/urmzd/semantic-release/actions/workflows/ci.yml/badge.svg)](https://github.com/urmzd/semantic-release/actions/workflows/ci.yml)

## Why?

The npm `semantic-release` ecosystem is battle-tested but comes with friction:

- **Requires Node.js** — even for Go, Rust, Python, and Java projects.
- **Complex plugin config** — wiring together `@semantic-release/*` packages is error-prone.
- **Coupled to CI runtime** — plugins shell out to language-specific toolchains at release time.

**sr** solves this:

- **Single static binary** — no runtime, no package manager, minimal dependencies.
- **Language-agnostic** — works with any project that uses git tags for versioning.
- **Zero-config defaults** — conventional commits + semver + GitHub releases out of the box.
- **Structured JSON output** — pipe `sr release` to `jq` for custom CI pipelines.

## Features

- Conventional Commits parsing (built-in, configurable via `commit_pattern`)
- Semantic versioning bumps (major / minor / patch)
- Automatic version file bumping (Cargo.toml, package.json, pyproject.toml, pom.xml, Gradle, Go)
- Changelog generation (markdown, with configurable sections)
- GitHub Releases (via REST API — no external tools needed)
- Structured JSON output for CI piping (`sr release | jq .version`)
- Trunk-based workflow (tag + release from `main`)

## Installation

### Shell installer (Linux/macOS)

```sh
curl -fsSL https://raw.githubusercontent.com/urmzd/semantic-release/main/install.sh | sh
```

The installer automatically adds `~/.local/bin` to your `PATH` in your shell profile (`.zshrc`, `.bashrc`, or `config.fish`).

### GitHub Action (recommended)

```yaml
- uses: urmzd/semantic-release@v1
  with:
    github-token: ${{ secrets.GITHUB_TOKEN }}
```

### Usage

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
      - uses: urmzd/semantic-release@v1
```

Dry-run on pull requests:

```yaml
      - uses: urmzd/semantic-release@v1
        with:
          command: release
          dry-run: "true"
```

Use outputs in subsequent steps:

```yaml
      - uses: urmzd/semantic-release@v1
        id: sr
      - if: steps.sr.outputs.released == 'true'
        run: echo "Released ${{ steps.sr.outputs.version }}"
```

Upload artifacts to the release:

```yaml
      # Build artifacts are downloaded into release-assets/
      - uses: actions/download-artifact@v4
        with:
          path: release-assets
          merge-multiple: true

      - uses: urmzd/semantic-release@v1
        with:
          artifacts: "release-assets/*"
```

The `artifacts` input accepts glob patterns (newline or comma separated). All matching files are uploaded to the GitHub release. This keeps artifact handling self-contained in the action — no separate upload steps needed.

Run a build step between version bump and commit (useful for lock files, codegen, etc.):

```yaml
      - uses: urmzd/semantic-release@v1
        with:
          build-command: "cargo build --release"
```

The command runs with `SR_VERSION` and `SR_TAG` environment variables set, so you can reference the new version in your build scripts.

Manual re-trigger with `workflow_dispatch` (useful when a previous release partially failed):

```yaml
name: Release
on:
  push:
    branches: [main]
  workflow_dispatch:
    inputs:
      force:
        description: "Re-release the current tag"
        type: boolean
        default: false

jobs:
  release:
    runs-on: ubuntu-latest
    permissions:
      contents: write
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0
      - uses: urmzd/semantic-release@v1
        with:
          force: ${{ github.event.inputs.force || 'false' }}
```

#### Inputs

| Input | Description | Default |
|-------|-------------|---------|
| `command` | The `sr` subcommand to run (`release`, `plan`, `changelog`, `version`, `config`, `completions`) | `release` |
| `dry-run` | Preview changes without executing them | `false` |
| `force` | Re-release the current tag (use when a previous release partially failed) | `false` |
| `config` | Path to the config file | `.urmzd.sr.yml` |
| `github-token` | GitHub token for creating releases | `${{ github.token }}` |
| `git-user-name` | Git user name for tag creation | `semantic-release[bot]` |
| `git-user-email` | Git user email for tag creation | `semantic-release[bot]@urmzd.com` |
| `artifacts` | Glob patterns for artifact files to upload (newline or comma separated) | `""` |
| `build-command` | Shell command to run after version bump, before commit (`SR_VERSION` and `SR_TAG` env vars available) | `""` |

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
[Releases](https://github.com/urmzd/semantic-release/releases):

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

## Quick Start

```bash
# Generate a default config file
sr init

# Preview what the next release would look like (includes changelog)
sr plan

# Dry-run a release (no side effects)
sr release --dry-run

# Execute the release
sr release

# Set up shell completions (bash)
sr completions bash >> ~/.bashrc
```

## Developer Workflow

### Commit message validation

`sr` ships a `commit-msg` git hook that enforces [Conventional Commits](https://www.conventionalcommits.org/) at commit time. It reads allowed types and patterns from `.urmzd.sr.yml`, falling back to built-in defaults.

**Option 1 — Native git hooks:**

```bash
# Copy the hook into your project
curl -o .githooks/commit-msg https://raw.githubusercontent.com/urmzd/semantic-release/main/.githooks/commit-msg
chmod +x .githooks/commit-msg
git config core.hooksPath .githooks/
```

**Option 2 — pre-commit framework:**

```yaml
# .pre-commit-config.yaml
repos:
  - repo: https://github.com/urmzd/semantic-release
    rev: v0.5.0
    hooks:
      - id: conventional-commit-msg
```

The hook validates the first line of each commit message against the pattern `<type>(<scope>): <description>`. Merge commits and rebase-generated commits (`fixup!`, `squash!`, `amend!`) are always allowed through.

### End-to-end release flow

```
commit (hook validates) → push → sr plan (preview) → sr release (execute)
```

1. **Commit** — the commit-msg hook ensures every commit follows the conventional format (`feat:`, `fix:`, `feat!:`, etc.).
2. **Preview** — run `sr plan` to see the next version, included commits, and a changelog preview.
3. **Dry-run** — run `sr release --dry-run` to simulate the full release without side effects (no tags created).
4. **Release** — run `sr release` to execute the full pipeline:
   - Bumps version in configured manifest files
   - Runs `build_command` if configured (with `SR_VERSION` and `SR_TAG` env vars)
   - Generates and commits the changelog (with version files)
   - Creates and pushes the git tag
   - Creates a GitHub release
   - Outputs structured JSON to stdout (pipe to `jq` for custom workflows)

## Post-release hooks

`sr` outputs structured JSON to stdout, making it easy to trigger post-release actions.

### GitHub Actions

Use the action outputs to run steps conditionally:

```yaml
- uses: urmzd/semantic-release@v1
  id: sr
- if: steps.sr.outputs.released == 'true'
  run: ./deploy.sh ${{ steps.sr.outputs.version }}
- if: steps.sr.outputs.released == 'true'
  run: |
    curl -X POST "$SLACK_WEBHOOK" \
      -d "{\"text\": \"Released v${{ steps.sr.outputs.version }}\"}"
```

### CLI

Pipe `sr release` output to downstream scripts:

```bash
# Extract the version
VERSION=$(sr release | jq -r '.version')

# Feed JSON into a custom script
sr release | my-post-release-hook.sh

# Publish to a package registry after release
VERSION=$(sr release | jq -r '.version')
if [ -n "$VERSION" ]; then
  npm publish
fi
```

### JSON output schema

`sr release` prints a JSON object to stdout on success:

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

All diagnostic messages go to stderr, so stdout is always clean JSON (or empty on exit code 2).

## CLI Reference

| Command | Description |
|---------|-------------|
| `sr release` | Execute a release (tag + GitHub release) |
| `sr plan` | Show what the next release would look like |
| `sr changelog` | Generate or preview the changelog |
| `sr version` | Show the next version |
| `sr config` | Validate and display resolved configuration |
| `sr init` | Create a default `.urmzd.sr.yml` config file |
| `sr completions` | Generate shell completions (bash, zsh, fish, powershell, elvish) |

### Common flags

- `sr release --dry-run` — preview without making changes
- `sr release --force` — re-release the current tag (for partial failure recovery)
- `sr release --build-command 'npm run build'` — run a command after version bump, before commit
- `sr plan --format json` — machine-readable output
- `sr changelog --write` — write changelog to disk
- `sr version --short` — print only the version number
- `sr config --resolved` — show config with defaults applied
- `sr init --force` — overwrite existing config file
- `sr completions bash` — generate Bash completions

### Exit codes

| Code | Meaning |
|------|---------|
| `0` | Success — a release was created (or dry-run completed). The released version is printed to stdout. |
| `1` | Real error — configuration issue, git failure, VCS provider error, etc. |
| `2` | No releasable changes — no new commits or no releasable commit types since the last tag. |

### `--force` flag

Use `--force` to re-run a release that partially failed (e.g. the tag was created but artifact upload failed). Force mode only works when HEAD is exactly at the latest tag — it re-executes the release pipeline for that tag without bumping the version.

```bash
# Re-release the current tag after a partial failure
sr release --force
```

Force mode will error if:
- There are no tags yet (nothing to re-release)
- HEAD is not at the latest tag (there are new commits — use a normal release instead)

## Configuration

`sr` looks for `.urmzd.sr.yml` in the repository root. All fields are optional and have sensible defaults.

### Configuration reference

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `branches` | `string[]` | `["main", "master"]` | Branches that trigger releases |
| `tag_prefix` | `string` | `"v"` | Prefix for git tags (e.g. `v1.0.0`) |
| `commit_pattern` | `string` | See below | Regex for parsing commit messages (must use named groups: `type`, `scope`, `breaking`, `description`) |
| `breaking_section` | `string` | `"Breaking Changes"` | Changelog section heading for breaking changes |
| `misc_section` | `string` | `"Miscellaneous"` | Changelog section heading for commit types without an explicit section |
| `types` | `CommitType[]` | See below | Commit type definitions (name, bump level, changelog section) |
| `changelog.file` | `string?` | `null` | Path to the changelog file (e.g. `CHANGELOG.md`). Omit to skip changelog generation |
| `version_files` | `string[]` | `[]` | Manifest files to bump (see supported formats below) |
| `version_files_strict` | `bool` | `false` | When `true`, fail the release if any version file is unsupported. When `false`, skip unsupported files with a warning |
| `artifacts` | `string[]` | `[]` | Glob patterns for files to upload to the GitHub release |
| `floating_tags` | `bool` | `false` | Create floating major version tags (e.g. `v3` always points to the latest `v3.x.x` release) |
| `build_command` | `string?` | `null` | Shell command to run after version bump but before commit. `SR_VERSION` and `SR_TAG` env vars are set |

### Example config

```yaml
branches:
  - main

tag_prefix: "v"

# Regex for parsing commits — must have named groups: type, scope, breaking, description
commit_pattern: '^(?P<type>\w+)(?:\((?P<scope>[^)]+)\))?(?P<breaking>!)?:\s+(?P<description>.+)'

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
    section: Refactoring
  - name: revert
    section: Reverts
  - name: chore          # no bump, no changelog section
  - name: ci
  - name: test
  - name: build
  - name: style

changelog:
  file: CHANGELOG.md

version_files:
  - Cargo.toml
  - package.json

version_files_strict: false

floating_tags: false

build_command: "cargo build --release"

artifacts:
  - "dist/*.tar.gz"
```

### Supported version files

| Filename | Key updated | Method | Notes |
|---|---|---|---|
| `Cargo.toml` | `package.version` or `workspace.package.version` | TOML parser | Preserves formatting/comments. Also updates `[workspace.dependencies]` entries that have both `path` and `version` fields |
| `package.json` | `version` | JSON parser | Pretty-printed output with trailing newline |
| `pyproject.toml` | `project.version` or `tool.poetry.version` | TOML parser | Preserves formatting/comments. Supports both PEP 621 and Poetry layouts |
| `pom.xml` | First `<version>` after `</parent>` (or `</modelVersion>`) | Regex | Skips the `<parent>` block to avoid changing the parent version |
| `build.gradle` | `version = '...'` or `version = "..."` | Regex | Only replaces the first match (avoids changing dependency versions) |
| `build.gradle.kts` | `version = "..."` | Regex | Only replaces the first match |
| `*.go` | `var Version = "..."` or `const Version string = "..."` | Regex | Matches the first `Version` variable/constant declaration |

### Environment variables

| Variable | Context | Description |
|----------|---------|-------------|
| `GH_TOKEN` / `GITHUB_TOKEN` | Release | GitHub API token for creating releases and uploading artifacts. Not needed for `--dry-run` |
| `SR_VERSION` | Build command | The new version string (e.g. `1.2.3`), set automatically when `build_command` runs |
| `SR_TAG` | Build command | The new tag name (e.g. `v1.2.3`), set automatically when `build_command` runs |

### Commit types

Each entry in the `types` list has these fields:

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | `string` | Yes | The commit type prefix (e.g. `feat`, `fix`) |
| `bump` | `string?` | No | Bump level: `major`, `minor`, or `patch`. Omit to not trigger a release for this type |
| `section` | `string?` | No | Changelog section heading (e.g. `"Features"`). Omit to exclude from changelog |

Breaking changes (commits with `!` after the type, e.g. `feat!:`) always trigger a `major` bump regardless of the type's configured bump level.

#### Default commit-type mapping

| Type | Bump | Changelog Section |
|------|------|-------------------|
| `feat` | minor | Features |
| `fix` | patch | Bug Fixes |
| `perf` | patch | Performance |
| `docs` | — | Documentation |
| `refactor` | — | Refactoring |
| `revert` | — | Reverts |
| `chore` | — | — |
| `ci` | — | — |
| `test` | — | — |
| `build` | — | — |
| `style` | — | — |

Types without a bump level do not trigger a release on their own. Types without a section are grouped under the `misc_section` heading if they appear in a release with other releasable commits.

### Commit pattern

The default pattern follows the [Conventional Commits](https://www.conventionalcommits.org/) spec:

```
^(?P<type>\w+)(?:\((?P<scope>[^)]+)\))?(?P<breaking>!)?:\s+(?P<description>.+)
```

If you override `commit_pattern`, your regex **must** include these named capture groups:

| Group | Required | Description |
|-------|----------|-------------|
| `type` | Yes | The commit type (e.g. `feat`, `fix`) |
| `scope` | No | Optional scope in parentheses |
| `breaking` | No | The `!` marker for breaking changes |
| `description` | Yes | The commit description |

### Changelog behavior

When `changelog.file` is set:
- If the file doesn't exist, it's created with a `# Changelog` heading
- If it already exists, new entries are inserted after the first heading (prepended, not appended)
- Each entry has the format: `## <version> (<date>)`
- Sections appear in order: Breaking Changes, then type sections in definition order, then Miscellaneous
- Commits link to their full SHA on GitHub when the repo URL is available

### Release execution order

Understanding the execution order helps when configuring `build_command`:

1. **Bump version files** — all configured `version_files` are updated on disk
2. **Write changelog** — the changelog file is written (if configured)
3. **Run build command** — your `build_command` runs with `SR_VERSION`/`SR_TAG` set. The version files already contain the new version, so tools like `cargo build` will pick it up
4. **Git commit** — version files + changelog are staged and committed as `chore(release): <tag> [skip ci]`
5. **Create and push tag**
6. **Create/update floating tag** (if `floating_tags: true`)
7. **Create GitHub release**
8. **Upload artifacts**

If any step fails, execution stops. Steps 5-8 are idempotent — re-running with `--force` will skip already-completed steps.

### Limitations

- **Single branch only** — no release branches or multi-branch workflows
- **No pre-releases** — no support for alpha/beta/rc tags
- **GitHub only** — the `VcsProvider` trait exists for extensibility, but only GitHub is implemented
- **No custom hooks beyond `build_command`** — use CI pipeline steps for pre/post-release actions

## Architecture

| Crate | Description |
|-------|-------------|
| [`sr-core`](crates/sr-core/) | Pure domain logic — traits, config, versioning, changelog |
| [`sr-git`](crates/sr-git/) | Git implementation (native `git` CLI) |
| [`sr-github`](crates/sr-github/) | GitHub VCS provider (REST API) |
| [`sr-cli`](crates/sr-cli/) | CLI binary (`clap`) — wires everything together |

`action.yml` in the repo root is the GitHub Action composite wrapper.

`sr` uses a pluggable `VcsProvider` trait and currently ships with GitHub support. GitLab, Bitbucket, and other providers can be added as separate crates implementing the same trait.

### Core traits

| Trait | Purpose |
|-------|---------|
| `GitRepository` | Tag discovery, commit listing, tag creation, push |
| `VcsProvider` | Remote release creation (GitHub, GitLab, etc.) |
| `CommitParser` | Raw commit to conventional commit |
| `ChangelogFormatter` | Render changelog entries to text |
| `ReleaseStrategy` | Orchestrate plan + execute |

## Design Philosophy

1. **Trunk-based flow** — releases happen from a single branch; no release branches.
2. **Conventional commits as source of truth** — commit messages drive versioning.
3. **Zero-config** — works out of the box with reasonable defaults.
4. **Focused scope** — sr handles versioning, tagging, changelog, and publishing. Pre-release validation and downstream actions belong in CI pipeline steps.
5. **Language-agnostic** — sr knows about git and semver, not about cargo or npm.

## Development

Requires [just](https://github.com/casey/just) for task running.

```bash
just init          # Install clippy + rustfmt
just check         # Run all checks (format, lint, test)
just build         # Build workspace
just test          # Run tests
just lint          # Run clippy
just fmt           # Format code
just run plan      # Run the CLI
```

See the [Justfile](Justfile) for all available recipes.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for development setup, code style, and PR guidelines.
