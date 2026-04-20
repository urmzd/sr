# CI workflow examples

Each file is a complete `.github/workflows/release.yml` for one shape.
Pair them with the matching `sr.yaml` from the parent `examples/` directory.

## Shapes

| Workflow | Pair with | What it shows |
|---|---|---|
| [`cargo-single.yml`](cargo-single.yml) | `cargo-single.yaml` | Single-job release. cargo publish builds + uploads internally. |
| [`cargo-multi-platform.yml`](cargo-multi-platform.yml) | `cargo-workspace.yaml` or `cargo-single.yaml` | Three-job shape: `prepare` → matrix `build` → `release`. Binaries embed the correct version via bumped Cargo.toml. |
| [`npm.yml`](npm.yml) | `npm-single.yaml` | Single-job npm publish. |
| [`pnpm-workspace.yml`](pnpm-workspace.yml) | `pnpm-workspace.yaml` | pnpm monorepo with optional `prepare → build → release` split. |
| [`uv-workspace.yml`](uv-workspace.yml) | `uv-workspace.yaml` | Python: `prepare` runs, then `uv build --all`, then `release`. |
| [`docker.yml`](docker.yml) | `docker.yaml` | Docker image release. |

## The three-verb model

Every workflow uses some combination of:

1. **`sr plan`** (or `mode: plan`) — preview. Outputs `version`, `tag`, `bump`. No side effects.
2. **`sr prepare`** (or `mode: prepare`) — writes bumped `version_files` + changelog. No commit, no tag, no push.
3. **`sr release`** (or `mode: release`, the default) — full reconcile: commit, tag, push, GH release, upload, publish.

For a single-job flow where you don't need artifacts, use `release` alone. For multi-platform or matrix-build flows, split into `prepare` → `build jobs` → `release` so each build sees the bumped manifest.

## State contract

`sr release` is idempotent. If `sr prepare` already bumped the manifests, `sr release`'s internal Bump stage is a noop. Re-running on a converged release (tag exists, assets uploaded, packages published) is also a noop. There's no hidden local state — actual state lives in the VCS + registries.
