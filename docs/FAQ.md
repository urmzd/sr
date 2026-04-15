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

Exit code 2 means **no releasable commits** were found since the last tag. This is not an error — it means all commits since the last release are either non-bumping types (e.g. `chore`, `docs`, `ci`) or non-conventional messages that were skipped. To force a release anyway, use `sr release --force`.

### Changelog is not generated

Set `changelog.file` in `sr.yaml` — changelog generation is opt-in:

```yaml
changelog:
  file: CHANGELOG.md
```

### Version files not updated

Ensure your manifest files are listed in `packages[].version_files` and match a [supported format](../README.md#supported-version-files).

### Tags are not signed

Set `git.sign_tags: true` in `sr.yaml` or pass `--sign-tags`. You must have a GPG or SSH signing key configured in git (`git config user.signingkey`).

### Migrating from v6.x

Run `sr migrate` to see the full migration guide, or read [migration.md](migration.md).
