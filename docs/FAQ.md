### Non-conventional commits are silently ignored

sr only understands commits that match the configured commit pattern (derived from type names defined in `commit.types`; follows [Conventional Commits](https://www.conventionalcommits.org/) by default). Commits that don't match — merge commits, JIRA-style messages, freeform text — are silently skipped during release planning. They won't trigger a version bump or appear in the changelog.

This means:
- **Merge commits** (`Merge pull request #123 from...`) — ignored, no impact
- **Squash merges with conventional titles** (`feat: add search`) — work perfectly
- **JIRA-style commits** (`PROJ-1234: fix login`) — ignored
- **Dependabot commits** (`Bump serde from 1.0 to 1.1`) — ignored
- **Freeform messages** (`fixed the bug`, `wip`) — ignored

If *all* commits since the last tag are non-conventional, sr exits with code 2 (no releasable changes).

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

Exit code 2 means **no releasable commits** were found since the last tag. Not an error — all commits since the last release are non-bumping types (e.g. `chore`, `docs`, `ci`) or non-conventional messages. To force a release, push a `feat:`/`fix:`/`perf:`/`refactor:` commit (an empty commit works: `git commit --allow-empty -m "fix: trigger release"`).

### Changelog is not generated

Set `changelog.file` in `sr.yaml` — changelog generation is opt-in:

```yaml
changelog:
  file: CHANGELOG.md
```

### Version files not updated

Ensure your manifest files are listed in `packages[].version_files` and match a [supported format](../README.md#supported-version-files). Paths must be literal — no glob expansion.

### Tags are not signed

Set `git.sign_tags: true` in `sr.yaml` or pass `--sign-tags`. You must have a GPG or SSH signing key configured in git (`git config user.signingkey`).

### How do binaries get the correct version embedded?

Most build tools read the version from a manifest at compile time:
- `cargo build` reads `CARGO_PKG_VERSION` from `Cargo.toml`
- `npm pack` reads `package.json`
- `uv build` reads `pyproject.toml`

Run `sr prepare` **before** your build step so the bumped manifest is on disk when the build runs. Then `sr release` commits, tags, uploads, and publishes. See [examples/ci/cargo-multi-platform.yml](../examples/ci/cargo-multi-platform.yml) for the three-job shape (prepare → matrix build → release).

### Why doesn't sr run build commands itself?

`sr` is a release-state reconciler, not a task runner. It writes versions, creates tags + releases, invokes typed registry publishers (`cargo publish`, `npm publish`, `docker buildx build --push`, `uv publish`). Running arbitrary shell commands is a CI concern — not sr's.

The one escape hatch is `publish: custom`, which takes a shell command for registries without a built-in publisher (helm, private Maven, etc.).

### Does sr support cross-compilation?

Not directly. Run your matrix in CI between `sr prepare` and `sr release`. Every build job downloads the prepared manifests (via `actions/upload-artifact` + `download-artifact`), builds for its target platform with the correct version embedded, and uploads its binary. The release job then downloads everything and runs `sr release` to tag + upload. See [examples/ci/cargo-multi-platform.yml](../examples/ci/cargo-multi-platform.yml).

### What happens if a release fails mid-flight?

Re-run `sr release`. Every stage has a strict `is_complete` check reading external state (tag exists? release object exists? assets uploaded? package on registry?). The pipeline picks up exactly where it left off. There's no state file to corrupt.

### Monorepo with one release per package?

Not supported. `sr` releases one tag per repo, one version across every package. Per-package tags (`core/v1.2.0`, `cli-v3.0.0`) are deliberately out of scope — that model is what changesets / Lerna are for.

For workspace-aware ecosystems, declare one entry at the workspace root with `publish.workspace: true`; every member publishes at the shared version.

### Migrating from v7.x

Run `sr migrate` or read [migration.md](../crates/cli/docs/migration.md). The v8 jump is breaking: `sr status` → `sr plan`; `packages[].independent` / `tag_prefix` / `hooks` are gone; `publish:` becomes a typed enum; globs in `artifacts`/`stage_files` become literal paths; `sr-manifest.json` is no longer produced.
