# Example `sr.yaml` configs

Each file is a complete, working `sr.yaml` for one release scenario.
Copy the closest match into your repo and adjust paths.

## Ecosystems

| Example | What it shows |
|---|---|
| [`cargo-single.yaml`](cargo-single.yaml) | Single Rust crate → crates.io |
| [`cargo-workspace.yaml`](cargo-workspace.yaml) | Rust workspace; every member publishes at the shared version |
| [`npm-single.yaml`](npm-single.yaml) | Single npm package → registry.npmjs.org |
| [`npm-workspace.yaml`](npm-workspace.yaml) | npm workspaces (`npm publish --workspaces`) |
| [`pnpm-workspace.yaml`](pnpm-workspace.yaml) | pnpm monorepo (`pnpm publish -r`) |
| [`uv-workspace.yaml`](uv-workspace.yaml) | uv / Python monorepo → PyPI |
| [`go.yaml`](go.yaml) | Go module (tag-only; no registry) |
| [`docker.yaml`](docker.yaml) | Container image → any OCI registry |
| [`multi-language.yaml`](multi-language.yaml) | Rust core + Node CLI, one tag |
| [`custom.yaml`](custom.yaml) | Escape hatch — arbitrary publish command + state check |

## CI workflows

See [`ci/`](ci/) for complete `.github/workflows/release.yml` examples that pair with each `sr.yaml`. These show the three-verb pattern:

- **`sr plan`** (preview; no side effects)
- **`sr prepare`** (bump manifest files + write changelog; no commit/tag)
- **`sr release`** (commit, tag, push, create GH release, upload, publish)

Most users only need `sr release`. Split into `prepare` + `build` + `release` when producing binaries that embed a version at build time (e.g. Rust binaries, Python wheels).

## Rules that apply to all configs

- **One global version for the whole repo.** Every package bumps to the
  same version on each release. `packages[]` declares *where to write
  versions and how to publish*, not *how to version*.
- **Literal paths, not globs.** `version_files`, `stage_files`, and
  `artifacts` are each exact filenames. sr does not expand `*.tar.gz`.
- **Reconciler model.** `sr plan` shows a resource-by-resource diff of
  desired vs. actual state (tags, releases, assets, registry presence).
  `sr release` applies only what's missing; re-running on a converged
  release is a noop.
- **Registry state is the source of truth.** Built-in publishers
  (`cargo`, `npm`, `docker`, `pypi`) query the registry to decide if
  work is needed, not a local state file.
