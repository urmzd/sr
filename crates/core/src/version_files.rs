use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;

use crate::error::ReleaseError;

// Newest lock formats sr's offline rewrites have been verified to reproduce.
// Each is covered by a round-trip test in `tests/lock_conformance.rs` that
// diffs sr's output against the real tool's. Bump a constant only together
// with a passing conformance run on the new format.
const CARGO_LOCK_MAX_VERSION: i64 = 4;
const UV_LOCK_MAX_VERSION: i64 = 1;
const UV_LOCK_MAX_REVISION: i64 = 2;
const POETRY_LOCK_MAX_VERSION: i64 = 2;
const NPM_LOCK_MAX_VERSION: i64 = 3;

/// package.json sections holding dependency ranges, mirrored per-package into
/// package-lock.json.
const NPM_DEPENDENCY_SECTIONS: [&str; 4] = [
    "dependencies",
    "devDependencies",
    "peerDependencies",
    "optionalDependencies",
];

/// What bumping a manifest produced: the extra manifests that were discovered
/// and rewritten, and the names of every package whose version was set.
///
/// The names are what lets [`VersionFileHandler::sync_lock`] find the right
/// entries in a lock file, so bumping and lock syncing can't drift apart.
#[derive(Debug, Default)]
pub struct BumpOutcome {
    /// Additional manifests auto-discovered and bumped (workspace members).
    pub extra_files: Vec<PathBuf>,
    /// Names of every package this bump set to the new version.
    pub package_names: Vec<String>,
}

/// Trait encapsulating detection, bumping, workspace discovery, and lock file
/// maintenance for a single ecosystem (Cargo, npm, Python, etc.).
pub trait VersionFileHandler: Send + Sync {
    /// Human-readable name, e.g. "Cargo", "npm".
    fn name(&self) -> &str;

    /// Primary manifest filenames, e.g. `["Cargo.toml"]`.
    fn manifest_names(&self) -> &[&str];

    /// Lock files this handler **maintains**, e.g. `["Cargo.lock"]`.
    ///
    /// These are staged into the release commit, so every name listed here
    /// must be handled by [`Self::sync_lock`].
    fn lock_file_names(&self) -> &[&str];

    /// Does this ecosystem exist in `dir`? Default: any manifest file exists.
    fn detect(&self, dir: &Path) -> bool {
        self.manifest_names()
            .iter()
            .any(|name| dir.join(name).exists())
    }

    /// Bump version in the manifest at `path`.
    fn bump(&self, path: &Path, new_version: &str) -> Result<BumpOutcome, ReleaseError>;

    /// Rewrite `lock_path` so it agrees with `new_version` for the packages in
    /// `bumped`. Returns whether the file was modified.
    ///
    /// This is deliberately **not** defaulted. sr stages every lock named by
    /// [`Self::lock_file_names`], so a handler that declares a lock has to say
    /// how it stays consistent — including "this format records no member
    /// versions, so it cannot go stale", written as an explicit no-op. A
    /// silent default here is what lets sr commit a lock it never updated.
    ///
    /// Implementations edit the lock as data (TOML/JSON), never by invoking
    /// the ecosystem's resolver — release time must not need the network.
    fn sync_lock(
        &self,
        lock_path: &Path,
        new_version: &str,
        bumped: &[String],
    ) -> Result<bool, ReleaseError>;
}

// ---------------------------------------------------------------------------
// Handler implementations
// ---------------------------------------------------------------------------

struct CargoHandler;

impl VersionFileHandler for CargoHandler {
    fn name(&self) -> &str {
        "Cargo"
    }
    fn manifest_names(&self) -> &[&str] {
        &["Cargo.toml"]
    }
    fn lock_file_names(&self) -> &[&str] {
        &["Cargo.lock"]
    }
    fn bump(&self, path: &Path, new_version: &str) -> Result<BumpOutcome, ReleaseError> {
        bump_cargo_toml(path, new_version)
    }
    fn sync_lock(
        &self,
        lock_path: &Path,
        new_version: &str,
        bumped: &[String],
    ) -> Result<bool, ReleaseError> {
        sync_cargo_lock(lock_path, new_version, bumped)
    }
}

struct NpmHandler;

impl VersionFileHandler for NpmHandler {
    fn name(&self) -> &str {
        "npm"
    }
    fn manifest_names(&self) -> &[&str] {
        &["package.json"]
    }
    fn lock_file_names(&self) -> &[&str] {
        &["package-lock.json", "yarn.lock", "pnpm-lock.yaml"]
    }
    fn bump(&self, path: &Path, new_version: &str) -> Result<BumpOutcome, ReleaseError> {
        bump_package_json(path, new_version)
    }
    fn sync_lock(
        &self,
        lock_path: &Path,
        new_version: &str,
        bumped: &[String],
    ) -> Result<bool, ReleaseError> {
        match file_name_of(lock_path) {
            "package-lock.json" => sync_package_lock_json(lock_path, new_version, bumped),
            // pnpm-lock.yaml records workspace deps as `link:../core` and
            // yarn.lock omits workspace members entirely — neither stores a
            // member version, so a bump cannot make them stale.
            _ => Ok(false),
        }
    }
}

struct PyprojectHandler;

impl VersionFileHandler for PyprojectHandler {
    fn name(&self) -> &str {
        "Python"
    }
    fn manifest_names(&self) -> &[&str] {
        &["pyproject.toml"]
    }
    fn lock_file_names(&self) -> &[&str] {
        &["uv.lock", "poetry.lock"]
    }
    fn bump(&self, path: &Path, new_version: &str) -> Result<BumpOutcome, ReleaseError> {
        bump_pyproject_toml(path, new_version)
    }
    fn sync_lock(
        &self,
        lock_path: &Path,
        new_version: &str,
        bumped: &[String],
    ) -> Result<bool, ReleaseError> {
        match file_name_of(lock_path) {
            "uv.lock" => sync_uv_lock(lock_path, new_version, bumped),
            "poetry.lock" => sync_poetry_lock(lock_path, new_version, bumped),
            _ => Ok(false),
        }
    }
}

struct MavenHandler;

impl VersionFileHandler for MavenHandler {
    fn name(&self) -> &str {
        "Maven"
    }
    fn manifest_names(&self) -> &[&str] {
        &["pom.xml"]
    }
    fn lock_file_names(&self) -> &[&str] {
        &[]
    }
    fn bump(&self, path: &Path, new_version: &str) -> Result<BumpOutcome, ReleaseError> {
        bump_pom_xml(path, new_version).map(|()| BumpOutcome::default())
    }
    fn sync_lock(&self, _: &Path, _: &str, _: &[String]) -> Result<bool, ReleaseError> {
        Ok(false) // No lock file — `lock_file_names` is empty.
    }
}

struct GradleHandler;

impl VersionFileHandler for GradleHandler {
    fn name(&self) -> &str {
        "Gradle"
    }
    fn manifest_names(&self) -> &[&str] {
        &["build.gradle", "build.gradle.kts"]
    }
    fn lock_file_names(&self) -> &[&str] {
        &[]
    }
    fn bump(&self, path: &Path, new_version: &str) -> Result<BumpOutcome, ReleaseError> {
        bump_gradle(path, new_version).map(|()| BumpOutcome::default())
    }
    fn sync_lock(&self, _: &Path, _: &str, _: &[String]) -> Result<bool, ReleaseError> {
        Ok(false) // No lock file — `lock_file_names` is empty.
    }
}

struct GoHandler;

impl VersionFileHandler for GoHandler {
    fn name(&self) -> &str {
        "Go"
    }
    fn manifest_names(&self) -> &[&str] {
        &[]
    }
    fn lock_file_names(&self) -> &[&str] {
        &[]
    }
    /// Custom detection: scan for `*.go` files containing a `Version` variable.
    fn detect(&self, dir: &Path) -> bool {
        let Ok(entries) = fs::read_dir(dir) else {
            return false;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "go")
                && let Ok(contents) = fs::read_to_string(&path)
                && go_version_re().is_match(&contents)
            {
                return true;
            }
        }
        false
    }
    fn bump(&self, path: &Path, new_version: &str) -> Result<BumpOutcome, ReleaseError> {
        bump_go_version(path, new_version).map(|()| BumpOutcome::default())
    }
    fn sync_lock(&self, _: &Path, _: &str, _: &[String]) -> Result<bool, ReleaseError> {
        Ok(false) // go.sum records module hashes, not this module's version.
    }
}

// ---------------------------------------------------------------------------
// Registry & public API
// ---------------------------------------------------------------------------

/// Return all known version-file handlers.
pub fn all_handlers() -> Vec<Box<dyn VersionFileHandler>> {
    vec![
        Box::new(CargoHandler),
        Box::new(NpmHandler),
        Box::new(PyprojectHandler),
        Box::new(MavenHandler),
        Box::new(GradleHandler),
        Box::new(GoHandler),
    ]
}

/// Auto-detect version files in a directory. Returns relative paths (relative
/// to `dir`) for every manifest whose ecosystem is detected.
///
/// For the Go handler the detected `.go` file containing the Version variable
/// is returned (not a manifest name).
pub fn detect_version_files(dir: &Path) -> Vec<String> {
    let mut files = Vec::new();
    for handler in all_handlers() {
        if !handler.detect(dir) {
            continue;
        }
        if handler.manifest_names().is_empty() {
            // Go handler: find the actual .go file with a Version var
            if let Ok(entries) = fs::read_dir(dir) {
                let re = go_version_re();
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().is_some_and(|e| e == "go")
                        && let Ok(contents) = fs::read_to_string(&path)
                        && re.is_match(&contents)
                    {
                        files.push(path.file_name().unwrap().to_string_lossy().into_owned());
                    }
                }
            }
        } else {
            for name in handler.manifest_names() {
                if dir.join(name).exists() {
                    files.push((*name).to_string());
                }
            }
        }
    }
    files
}

/// Look up the handler for a given filename.
fn handler_for_file(filename: &str) -> Option<Box<dyn VersionFileHandler>> {
    for handler in all_handlers() {
        if handler.manifest_names().contains(&filename) {
            return Some(handler);
        }
    }
    // Go files: any .go extension
    if filename.ends_with(".go") {
        return Some(Box::new(GoHandler));
    }
    None
}

/// Bump the `version` field in the given manifest file.
///
/// Returns a list of additional files that were auto-discovered and bumped
/// (e.g. workspace member manifests). The caller should stage these files.
///
/// The file format is auto-detected from the filename:
/// - `Cargo.toml`          → TOML (`package.version` or `workspace.package.version`)
/// - `package.json`        → JSON (`.version`)
/// - `pyproject.toml`      → TOML (`project.version` or `tool.poetry.version`)
/// - `build.gradle`        → Gradle Groovy DSL (`version = '...'` or `version = "..."`)
/// - `build.gradle.kts`    → Gradle Kotlin DSL (`version = "..."`)
/// - `pom.xml`             → Maven (`<version>...</version>`, skipping `<parent>` block)
/// - `*.go`                → Go (`var/const Version = "..."`)
///
/// For workspace roots (Cargo, npm, uv), member manifests are auto-discovered
/// and bumped without needing to list them in `version_files`.
///
/// Lock files are **not** touched here — call [`sync_lock_files`] once after
/// every manifest has been bumped. A lock records versions for packages whose
/// manifests live elsewhere in the tree (a poetry sibling path dep is in the
/// dependent's lock, not its own), so syncing per-manifest would miss exactly
/// the entries that go stale.
pub fn bump_version_file(path: &Path, new_version: &str) -> Result<BumpOutcome, ReleaseError> {
    let filename = file_name_of(path);

    match handler_for_file(filename) {
        Some(handler) => handler.bump(path, new_version),
        None => Err(ReleaseError::VersionBump(format!(
            "unsupported version file: {filename}"
        ))),
    }
}

/// Bring every lock file associated with `bumped_files` into agreement with
/// `new_version`, for every package named in `package_names`.
///
/// Runs after all manifests are bumped, with the complete set of names — under
/// sr's one-version model every bumped package moves together, so any locally
/// sourced lock entry naming one of them is stale by definition.
///
/// Returns the locks that were actually modified.
pub fn sync_lock_files(
    bumped_files: &[String],
    new_version: &str,
    package_names: &[String],
) -> Result<Vec<PathBuf>, ReleaseError> {
    let mut synced = Vec::new();
    for lock_path in discover_lock_files(bumped_files) {
        let Some(handler) = handler_for_lock(file_name_of(&lock_path)) else {
            continue;
        };
        if handler.sync_lock(&lock_path, new_version, package_names)? {
            synced.push(lock_path);
        }
    }
    Ok(synced)
}

/// Look up the handler that maintains a given lock file.
fn handler_for_lock(filename: &str) -> Option<Box<dyn VersionFileHandler>> {
    all_handlers()
        .into_iter()
        .find(|h| h.lock_file_names().contains(&filename))
}

/// The file name of `path` as a `&str`, or `""` if it has none.
fn file_name_of(path: &Path) -> &str {
    path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
}

/// Given a list of bumped manifest paths, discover associated lock files that exist on disk.
/// Searches the manifest's directory and ancestors (for monorepo roots).
/// Returns deduplicated paths.
pub fn discover_lock_files(bumped_files: &[String]) -> Vec<PathBuf> {
    let handlers = all_handlers();
    let mut seen = std::collections::BTreeSet::new();
    for file in bumped_files {
        let path = Path::new(file);
        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();

        // Collect lock file names from all handlers that match this manifest
        let mut lock_names: Vec<&str> = Vec::new();
        for handler in &handlers {
            if handler.manifest_names().contains(&filename) {
                lock_names.extend(handler.lock_file_names());
            }
        }

        // Search the manifest's directory and ancestors
        let mut dir = path.parent();
        while let Some(d) = dir {
            for lock_name in &lock_names {
                let lock_path = d.join(lock_name);
                if lock_path.exists() {
                    seen.insert(lock_path);
                }
            }
            dir = d.parent();
            // Stop at repo root (don't traverse beyond .git)
            if d.join(".git").exists() {
                break;
            }
        }
    }
    seen.into_iter().collect()
}

/// Returns `true` if the given filename is a supported version file.
pub fn is_supported_version_file(filename: &str) -> bool {
    handler_for_file(filename).is_some()
}

/// Compile the Go Version variable regex (used in detection).
fn go_version_re() -> Regex {
    Regex::new(r#"(?:var|const)\s+Version\s*(?:string\s*)?=\s*""#).unwrap()
}

// ---------------------------------------------------------------------------
// Private bump implementations (unchanged)
// ---------------------------------------------------------------------------

fn bump_cargo_toml(path: &Path, new_version: &str) -> Result<BumpOutcome, ReleaseError> {
    let contents = read_file(path)?;
    let mut doc: toml_edit::DocumentMut = contents.parse().map_err(|e| {
        ReleaseError::VersionBump(format!("failed to parse {}: {e}", path.display()))
    })?;

    let is_workspace = doc
        .get("workspace")
        .and_then(|w| w.get("package"))
        .and_then(|p| p.get("version"))
        .is_some();

    let mut bumped_names: Vec<String> = Vec::new();
    if let Some(name) = doc
        .get("package")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())
    {
        bumped_names.push(name.to_string());
    }

    if doc.get("package").and_then(|p| p.get("version")).is_some() {
        doc["package"]["version"] = toml_edit::value(new_version);
    } else if is_workspace {
        doc["workspace"]["package"]["version"] = toml_edit::value(new_version);
    } else {
        return Err(ReleaseError::VersionBump(format!(
            "no version field found in {}",
            path.display()
        )));
    }

    bump_cargo_path_deps(&mut doc, new_version);
    write_file(path, &doc.to_string())?;

    // Auto-discover and bump workspace member Cargo.toml files
    let mut extra = Vec::new();
    if is_workspace {
        let members = extract_toml_string_array(&doc, &["workspace", "members"]);
        let root_dir = path.parent().unwrap_or(Path::new("."));
        for member_path in resolve_member_globs(root_dir, &members, "Cargo.toml") {
            if member_path.as_path() == path {
                continue;
            }
            match bump_cargo_member(&member_path, new_version) {
                Ok((modified, name)) => {
                    if modified {
                        extra.push(member_path);
                    }
                    if let Some(n) = name {
                        bumped_names.push(n);
                    }
                }
                Err(e) => eprintln!("warning: {e}"),
            }
        }
    }

    Ok(BumpOutcome {
        extra_files: extra,
        package_names: bumped_names,
    })
}

/// Retarget internal path dependencies (`{ path = "...", version = "..." }`) to
/// the new version, across `[dependencies]`, `[dev-dependencies]`,
/// `[build-dependencies]` and `[workspace.dependencies]`.
///
/// Cargo requires the `version` field on a path dep for `cargo publish`, and
/// resolves the workspace against it. Leaving it stale makes the workspace
/// unresolvable the moment a bump crosses a semver-incompatible boundary:
/// `cargo` reports "failed to select a version for the requirement".
///
/// Only deps carrying **both** `path` and `version` are touched — an external
/// dependency has no `path`, and a path dep without a `version` is
/// intentionally unpublished.
fn bump_cargo_path_deps(doc: &mut toml_edit::DocumentMut, new_version: &str) -> bool {
    fn retarget(deps: &mut dyn toml_edit::TableLike, new_version: &str) -> bool {
        let mut changed = false;
        for (_, dep) in deps.iter_mut() {
            if let Some(tbl) = dep.as_table_like_mut()
                && tbl.get("path").is_some()
                && tbl.get("version").is_some()
            {
                tbl.insert("version", toml_edit::value(new_version));
                changed = true;
            }
        }
        changed
    }

    let mut changed = false;
    for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
        if let Some(deps) = doc.get_mut(section).and_then(|d| d.as_table_like_mut()) {
            changed |= retarget(deps, new_version);
        }
    }
    if let Some(deps) = doc
        .get_mut("workspace")
        .and_then(|w| w.get_mut("dependencies"))
        .and_then(|d| d.as_table_like_mut())
    {
        changed |= retarget(deps, new_version);
    }
    changed
}

/// Bump `package.version` in a workspace member Cargo.toml and retarget its
/// internal path deps.
///
/// Returns `(modified, name)` — `modified` is `true` when the file was
/// rewritten, which includes a member that inherits `version.workspace = true`
/// but declares a path dep on a sibling. `name` is the member's `package.name`,
/// present regardless, so the caller can update Cargo.lock.
fn bump_cargo_member(
    path: &Path,
    new_version: &str,
) -> Result<(bool, Option<String>), ReleaseError> {
    let contents = read_file(path)?;
    let mut doc: toml_edit::DocumentMut = contents.parse().map_err(|e| {
        ReleaseError::VersionBump(format!("failed to parse {}: {e}", path.display()))
    })?;

    let name = doc
        .get("package")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())
        .map(|s| s.to_string());

    // Members inheriting `version.workspace = true` keep their version field
    // untouched, but their path deps still need retargeting.
    let owns_version = doc
        .get("package")
        .and_then(|p| p.get("version"))
        .is_some_and(|item| item.is_value());
    if owns_version {
        doc["package"]["version"] = toml_edit::value(new_version);
    }

    let deps_changed = bump_cargo_path_deps(&mut doc, new_version);

    if owns_version || deps_changed {
        write_file(path, &doc.to_string())?;
        return Ok((true, name));
    }
    Ok((false, name))
}

// ---------------------------------------------------------------------------
// Lock file synchronization
// ---------------------------------------------------------------------------
//
// Every sync below rewrites the lock as data — no resolver is ever invoked, so
// release time needs no network and no registry credentials. That only works
// because sr reproduces exactly what the ecosystem's own tool would write, so
// each sync is pinned to the lock format it was verified against; see
// `lock_conformance.rs` for the round-trip tests that keep those pins honest.

/// Refuse to edit a lock whose format is newer than what sr reproduces.
///
/// A newer format may record versions somewhere sr doesn't know about, and a
/// half-updated lock committed to a release is worse than a loud failure.
/// `found` is the structural format version, `supported` the newest sr has
/// been verified against.
fn guard_lock_format(
    lock_path: &Path,
    found: i64,
    supported: i64,
    upgrade_hint: &str,
) -> Result<(), ReleaseError> {
    if found > supported {
        return Err(ReleaseError::VersionBump(format!(
            "{} is format version {found}, but this sr only reproduces up to version {supported}. \
             Refusing to edit it — a partially-updated lock file would be committed to the release. \
             {upgrade_hint}",
            lock_path.display()
        )));
    }
    Ok(())
}

/// Rewrite workspace-member `[[package]]` entries in Cargo.lock to the new
/// version. Entries with a `source` field (published deps) are ignored.
///
/// Verified against `Cargo.lock` format version 4.
fn sync_cargo_lock(
    lock_path: &Path,
    new_version: &str,
    member_names: &[String],
) -> Result<bool, ReleaseError> {
    if member_names.is_empty() {
        return Ok(false);
    }

    let mut doc = read_toml(lock_path)?;

    // Absent `version` means the pre-v3 format, which uses the same
    // `[[package]]` shape.
    let format = doc.get("version").and_then(|v| v.as_integer()).unwrap_or(3);
    guard_lock_format(
        lock_path,
        format,
        CARGO_LOCK_MAX_VERSION,
        "Upgrade sr, or drop Cargo.lock from version control.",
    )?;

    let Some(packages) = doc
        .get_mut("package")
        .and_then(|p| p.as_array_of_tables_mut())
    else {
        return Ok(false);
    };

    let mut changed = false;
    for pkg in packages.iter_mut() {
        let Some(name) = pkg.get("name").and_then(|n| n.as_str()) else {
            continue;
        };
        if pkg.contains_key("source") {
            continue; // Published dep, not a workspace member
        }
        if member_names.iter().any(|m| m == name) {
            pkg.insert("version", toml_edit::value(new_version));
            changed = true;
        }
    }

    if changed {
        write_file(lock_path, &doc.to_string())?;
    }
    Ok(changed)
}

/// Rewrite the `version` of locally-sourced `[[package]]` entries in uv.lock.
/// Registry-sourced entries (published deps) are left alone — only packages uv
/// resolves from a path in this repo are workspace members.
///
/// Verified against `uv.lock` format version 1 (revision 2): sr's output is
/// byte-identical to a full `uv lock` re-resolve.
fn sync_uv_lock(
    lock_path: &Path,
    new_version: &str,
    member_names: &[String],
) -> Result<bool, ReleaseError> {
    if member_names.is_empty() {
        return Ok(false);
    }

    let mut doc = read_toml(lock_path)?;

    let format = doc.get("version").and_then(|v| v.as_integer()).unwrap_or(1);
    guard_lock_format(
        lock_path,
        format,
        UV_LOCK_MAX_VERSION,
        "Upgrade sr, or drop uv.lock from version control.",
    )?;
    // `revision` moves within a format version without relocating the
    // `[[package]] name/version/source` shape this rewrite depends on, so a
    // newer revision warns rather than blocks.
    if let Some(rev) = doc.get("revision").and_then(|v| v.as_integer())
        && rev > UV_LOCK_MAX_REVISION
    {
        eprintln!(
            "warning: {} is revision {rev}, newer than the revision {UV_LOCK_MAX_REVISION} sr was verified against — \
             syncing anyway; run `uv lock --check` if the release looks wrong",
            lock_path.display()
        );
    }

    let Some(packages) = doc
        .get_mut("package")
        .and_then(|p| p.as_array_of_tables_mut())
    else {
        return Ok(false);
    };

    let normalized: Vec<String> = member_names
        .iter()
        .map(|n| normalize_dist_name(n))
        .collect();

    let mut changed = false;
    for pkg in packages.iter_mut() {
        let matches_member = pkg
            .get("name")
            .and_then(|n| n.as_str())
            .is_some_and(|name| normalized.contains(&normalize_dist_name(name)));
        if !matches_member || !is_local_uv_source(pkg) {
            continue;
        }
        // Only rewrite an existing key — inserting one here would land after
        // the entry's `[package.metadata]` sub-table and change its meaning.
        if pkg.contains_key("version") {
            pkg.insert("version", toml_edit::value(new_version));
            changed = true;
        }
    }

    if changed {
        write_file(lock_path, &doc.to_string())?;
    }
    Ok(changed)
}

/// `true` when a uv.lock entry resolves from a path in this repo (a workspace
/// member or path dependency) rather than from a registry.
fn is_local_uv_source(pkg: &toml_edit::Table) -> bool {
    pkg.get("source")
        .and_then(|s| s.as_table_like())
        .is_some_and(|src| {
            ["editable", "virtual", "directory"]
                .iter()
                .any(|key| src.contains_key(key))
        })
}

/// Rewrite sibling path-dependency versions in poetry.lock.
///
/// poetry.lock never contains the project itself, only its dependencies — so
/// the entries that go stale are sibling members pulled in as path deps, which
/// carry `[package.source] type = "directory"`. `[metadata].content-hash` is
/// derived from the declared dependency specs, not resolved versions, so it
/// stays valid.
///
/// Verified against `poetry.lock` lock-version 2.1: sr's output is
/// byte-identical to a full `poetry lock` regenerate.
fn sync_poetry_lock(
    lock_path: &Path,
    new_version: &str,
    member_names: &[String],
) -> Result<bool, ReleaseError> {
    if member_names.is_empty() {
        return Ok(false);
    }

    let mut doc = read_toml(lock_path)?;

    // `lock-version` is a "major.minor" string; only the major moves structure.
    let format = doc
        .get("metadata")
        .and_then(|m| m.get("lock-version"))
        .and_then(|v| v.as_str())
        .and_then(|s| s.split('.').next())
        .and_then(|major| major.parse::<i64>().ok())
        .unwrap_or(POETRY_LOCK_MAX_VERSION);
    guard_lock_format(
        lock_path,
        format,
        POETRY_LOCK_MAX_VERSION,
        "Upgrade sr, or drop poetry.lock from version control.",
    )?;

    let Some(packages) = doc
        .get_mut("package")
        .and_then(|p| p.as_array_of_tables_mut())
    else {
        return Ok(false);
    };

    let normalized: Vec<String> = member_names
        .iter()
        .map(|n| normalize_dist_name(n))
        .collect();

    let mut changed = false;
    for pkg in packages.iter_mut() {
        let matches_member = pkg
            .get("name")
            .and_then(|n| n.as_str())
            .is_some_and(|name| normalized.contains(&normalize_dist_name(name)));
        let is_directory_dep = pkg
            .get("source")
            .and_then(|s| s.as_table_like())
            .and_then(|src| src.get("type"))
            .and_then(|t| t.as_str())
            == Some("directory");
        if matches_member && is_directory_dep && pkg.contains_key("version") {
            pkg.insert("version", toml_edit::value(new_version));
            changed = true;
        }
    }

    if changed {
        write_file(lock_path, &doc.to_string())?;
    }
    Ok(changed)
}

/// Rewrite workspace-member versions in package-lock.json.
///
/// npm records a member's version in two places: the `packages` entry keyed by
/// its directory, and — for the root package — the top-level `version` plus the
/// `packages[""]` entry. Entries under `node_modules/` that are `link: true`
/// point at a workspace member and carry no version of their own.
///
/// Verified against `lockfileVersion` 3: sr's output is byte-identical to a
/// full `npm install --package-lock-only`.
fn sync_package_lock_json(
    lock_path: &Path,
    new_version: &str,
    member_names: &[String],
) -> Result<bool, ReleaseError> {
    if member_names.is_empty() {
        return Ok(false);
    }

    let contents = read_file(lock_path)?;
    let mut value: serde_json::Value = serde_json::from_str(&contents).map_err(|e| {
        ReleaseError::VersionBump(format!("failed to parse {}: {e}", lock_path.display()))
    })?;

    let format = value
        .get("lockfileVersion")
        .and_then(|v| v.as_i64())
        .unwrap_or(NPM_LOCK_MAX_VERSION);
    guard_lock_format(
        lock_path,
        format,
        NPM_LOCK_MAX_VERSION,
        "Upgrade sr, or drop package-lock.json from version control.",
    )?;

    let Some(obj) = value.as_object_mut() else {
        return Ok(false);
    };

    let mut changed = false;

    // Top-level mirror of the root package's version.
    if obj
        .get("name")
        .and_then(|n| n.as_str())
        .is_some_and(|n| member_names.iter().any(|m| m == n))
        && obj.contains_key("version")
    {
        obj.insert("version".into(), new_version.into());
        changed = true;
    }

    // Every `packages` entry whose `name` is one this bump touched. The root
    // is keyed by "" and carries its name explicitly.
    if let Some(packages) = obj.get_mut("packages").and_then(|p| p.as_object_mut()) {
        for (_, entry) in packages.iter_mut() {
            let Some(entry_obj) = entry.as_object_mut() else {
                continue;
            };
            let is_member = entry_obj
                .get("name")
                .and_then(|n| n.as_str())
                .is_some_and(|n| member_names.iter().any(|m| m == n));
            // `link: true` entries are pointers to a member directory and
            // hold no version to update.
            if is_member && entry_obj.contains_key("version") {
                entry_obj.insert("version".into(), new_version.into());
                changed = true;
            }
            // npm mirrors each package's declared dependency ranges into the
            // lock, so a sibling range retargeted in package.json has to be
            // retargeted here too or the two disagree.
            for section in NPM_DEPENDENCY_SECTIONS {
                let Some(deps) = entry_obj.get_mut(section).and_then(|d| d.as_object_mut()) else {
                    continue;
                };
                for (name, spec) in deps.iter_mut() {
                    if !member_names.iter().any(|m| m == name) {
                        continue;
                    }
                    let Some(current) = spec.as_str() else {
                        continue;
                    };
                    if let Some(updated) = retarget_range(current, new_version) {
                        *spec = serde_json::Value::String(updated);
                        changed = true;
                    }
                }
            }
        }
    }

    if changed {
        let output = serde_json::to_string_pretty(&value).map_err(|e| {
            ReleaseError::VersionBump(format!("failed to serialize {}: {e}", lock_path.display()))
        })?;
        write_file(lock_path, &format!("{output}\n"))?;
    }
    Ok(changed)
}

/// Read and parse a TOML lock file.
fn read_toml(path: &Path) -> Result<toml_edit::DocumentMut, ReleaseError> {
    read_file(path)?
        .parse()
        .map_err(|e| ReleaseError::VersionBump(format!("failed to parse {}: {e}", path.display())))
}

fn bump_package_json(path: &Path, new_version: &str) -> Result<BumpOutcome, ReleaseError> {
    let contents = read_file(path)?;
    let mut value: serde_json::Value = serde_json::from_str(&contents).map_err(|e| {
        ReleaseError::VersionBump(format!("failed to parse {}: {e}", path.display()))
    })?;

    let obj = value
        .as_object_mut()
        .ok_or_else(|| ReleaseError::VersionBump("package.json is not an object".into()))?;

    // Extract workspace patterns before mutating
    let workspace_patterns: Vec<String> = obj
        .get("workspaces")
        .and_then(|w| w.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let mut bumped_names: Vec<String> = Vec::new();
    if let Some(name) = obj.get("name").and_then(|n| n.as_str()) {
        bumped_names.push(name.to_string());
    }

    obj.insert(
        "version".into(),
        serde_json::Value::String(new_version.into()),
    );

    let output = serde_json::to_string_pretty(&value).map_err(|e| {
        ReleaseError::VersionBump(format!("failed to serialize {}: {e}", path.display()))
    })?;

    write_file(path, &format!("{output}\n"))?;

    // Auto-discover and bump workspace member package.json files
    let mut extra = Vec::new();
    let mut all_manifests = vec![path.to_path_buf()];
    if !workspace_patterns.is_empty() {
        let root_dir = path.parent().unwrap_or(Path::new("."));
        for member_path in resolve_member_globs(root_dir, &workspace_patterns, "package.json") {
            if member_path == path {
                continue;
            }
            all_manifests.push(member_path.clone());
            match bump_json_version(&member_path, new_version) {
                Ok((modified, name)) => {
                    if modified {
                        extra.push(member_path);
                    }
                    // A member without its own `version` still counts as a
                    // sibling other members may depend on.
                    if let Some(n) = name {
                        bumped_names.push(n);
                    }
                }
                Err(e) => eprintln!("warning: {e}"),
            }
        }
    }

    // Second pass: every member name is known now, so sibling ranges can be
    // retargeted. npm resolves a workspace dep locally only while the declared
    // range still admits the member's version — after a major bump `^1.0.0`
    // stops matching and npm goes to the registry for a package that was never
    // published there.
    for manifest in &all_manifests {
        match retarget_npm_sibling_ranges(manifest, new_version, &bumped_names) {
            Ok(true) => {
                if manifest != path && !extra.contains(manifest) {
                    extra.push(manifest.clone());
                }
            }
            Ok(false) => {}
            Err(e) => eprintln!("warning: {e}"),
        }
    }

    Ok(BumpOutcome {
        extra_files: extra,
        package_names: bumped_names,
    })
}

/// Retarget dependency ranges that point at a workspace sibling.
///
/// Only caret, tilde, and bare-exact specs are rewritten. A comparator range
/// (`>=1.0.0`), a wildcard (`*`), and the protocol forms (`workspace:*`,
/// `file:../core`, `link:`) are left alone: the first two stay satisfied by a
/// higher version, and the protocol forms resolve by path and carry no version
/// to update.
fn retarget_npm_sibling_ranges(
    path: &Path,
    new_version: &str,
    member_names: &[String],
) -> Result<bool, ReleaseError> {
    let contents = read_file(path)?;
    let mut value: serde_json::Value = serde_json::from_str(&contents).map_err(|e| {
        ReleaseError::VersionBump(format!("failed to parse {}: {e}", path.display()))
    })?;
    let Some(obj) = value.as_object_mut() else {
        return Ok(false);
    };

    let mut changed = false;
    for section in NPM_DEPENDENCY_SECTIONS {
        let Some(deps) = obj.get_mut(section).and_then(|d| d.as_object_mut()) else {
            continue;
        };
        for (name, spec) in deps.iter_mut() {
            if !member_names.iter().any(|m| m == name) {
                continue;
            }
            let Some(current) = spec.as_str() else {
                continue;
            };
            if let Some(updated) = retarget_range(current, new_version) {
                *spec = serde_json::Value::String(updated);
                changed = true;
            }
        }
    }

    if changed {
        let output = serde_json::to_string_pretty(&value).map_err(|e| {
            ReleaseError::VersionBump(format!("failed to serialize {}: {e}", path.display()))
        })?;
        write_file(path, &format!("{output}\n"))?;
    }
    Ok(changed)
}

/// Rewrite `^1.2.3` / `~1.2.3` / `1.2.3` to the new version, preserving the
/// operator. Returns `None` for anything else, including a spec that is already
/// the new version.
fn retarget_range(spec: &str, new_version: &str) -> Option<String> {
    let (prefix, bare) = match spec.as_bytes().first()? {
        b'^' => ("^", &spec[1..]),
        b'~' => ("~", &spec[1..]),
        b'0'..=b'9' => ("", spec),
        _ => return None,
    };
    // A bare version only — not a range like `1.x` or `1.0.0 - 2.0.0`.
    semver::Version::parse(bare).ok()?;
    let updated = format!("{prefix}{new_version}");
    (updated != spec).then_some(updated)
}

/// Bump `version` in a member package.json (skip if no version field).
/// Returns `(modified, name)` — `name` is the member's declared package name,
/// used to locate its entry in package-lock.json.
fn bump_json_version(
    path: &Path,
    new_version: &str,
) -> Result<(bool, Option<String>), ReleaseError> {
    let contents = read_file(path)?;
    let mut value: serde_json::Value = serde_json::from_str(&contents).map_err(|e| {
        ReleaseError::VersionBump(format!("failed to parse {}: {e}", path.display()))
    })?;

    let obj = match value.as_object_mut() {
        Some(o) => o,
        None => return Ok((false, None)),
    };

    let name = obj
        .get("name")
        .and_then(|n| n.as_str())
        .map(|s| s.to_string());

    if obj.get("version").is_none() {
        return Ok((false, name));
    }

    obj.insert(
        "version".into(),
        serde_json::Value::String(new_version.into()),
    );

    let output = serde_json::to_string_pretty(&value).map_err(|e| {
        ReleaseError::VersionBump(format!("failed to serialize {}: {e}", path.display()))
    })?;

    write_file(path, &format!("{output}\n"))?;
    Ok((true, name))
}

fn bump_pyproject_toml(path: &Path, new_version: &str) -> Result<BumpOutcome, ReleaseError> {
    let contents = read_file(path)?;
    let mut doc: toml_edit::DocumentMut = contents.parse().map_err(|e| {
        ReleaseError::VersionBump(format!("failed to parse {}: {e}", path.display()))
    })?;

    if doc.get("project").and_then(|p| p.get("version")).is_some() {
        doc["project"]["version"] = toml_edit::value(new_version);
    } else if doc
        .get("tool")
        .and_then(|t| t.get("poetry"))
        .and_then(|p| p.get("version"))
        .is_some()
    {
        doc["tool"]["poetry"]["version"] = toml_edit::value(new_version);
    } else {
        return Err(ReleaseError::VersionBump(format!(
            "no version field found in {}",
            path.display()
        )));
    }

    write_file(path, &doc.to_string())?;

    let mut bumped_names: Vec<String> = Vec::new();
    if let Some(name) = pyproject_package_name(&doc) {
        bumped_names.push(name);
    }

    // Auto-discover uv workspace members
    let members = extract_toml_string_array(&doc, &["tool", "uv", "workspace", "members"]);
    let mut extra = Vec::new();
    if !members.is_empty() {
        let root_dir = path.parent().unwrap_or(Path::new("."));
        for member_path in resolve_member_globs(root_dir, &members, "pyproject.toml") {
            if member_path.as_path() == path {
                continue;
            }
            match bump_pyproject_member(&member_path, new_version) {
                Ok((modified, name)) => {
                    if modified {
                        extra.push(member_path);
                        if let Some(n) = name {
                            bumped_names.push(n);
                        }
                    }
                }
                Err(e) => eprintln!("warning: {e}"),
            }
        }
    }

    Ok(BumpOutcome {
        extra_files: extra,
        package_names: bumped_names,
    })
}

/// Bump version in a uv workspace member pyproject.toml (skip if no version field).
/// Returns `(modified, name)` — `modified` is `true` when the file was
/// rewritten; `name` is the member's declared package name, used to locate its
/// entry in uv.lock.
fn bump_pyproject_member(
    path: &Path,
    new_version: &str,
) -> Result<(bool, Option<String>), ReleaseError> {
    let contents = read_file(path)?;
    let mut doc: toml_edit::DocumentMut = contents.parse().map_err(|e| {
        ReleaseError::VersionBump(format!("failed to parse {}: {e}", path.display()))
    })?;

    let name = pyproject_package_name(&doc);

    if doc.get("project").and_then(|p| p.get("version")).is_some() {
        doc["project"]["version"] = toml_edit::value(new_version);
    } else if doc
        .get("tool")
        .and_then(|t| t.get("poetry"))
        .and_then(|p| p.get("version"))
        .is_some()
    {
        doc["tool"]["poetry"]["version"] = toml_edit::value(new_version);
    } else {
        return Ok((false, name)); // No version field — skip
    }

    write_file(path, &doc.to_string())?;
    Ok((true, name))
}

/// Read the declared package name from a pyproject.toml (PEP 621 or Poetry).
fn pyproject_package_name(doc: &toml_edit::DocumentMut) -> Option<String> {
    doc.get("project")
        .and_then(|p| p.get("name"))
        .or_else(|| {
            doc.get("tool")
                .and_then(|t| t.get("poetry"))
                .and_then(|p| p.get("name"))
        })
        .and_then(|n| n.as_str())
        .map(|s| s.to_string())
}

/// Normalize a distribution name per PEP 503 — uv.lock stores normalized names
/// (`My_Pkg.core` → `my-pkg-core`) while pyproject.toml keeps them as written.
fn normalize_dist_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut last_was_sep = false;
    for ch in name.chars() {
        if matches!(ch, '-' | '_' | '.') {
            if !last_was_sep {
                out.push('-');
            }
            last_was_sep = true;
        } else {
            out.extend(ch.to_lowercase());
            last_was_sep = false;
        }
    }
    out
}

fn bump_gradle(path: &Path, new_version: &str) -> Result<(), ReleaseError> {
    let contents = read_file(path)?;
    let re = Regex::new(r#"(version\s*=\s*["'])([^"']*)(["'])"#).unwrap();
    if !re.is_match(&contents) {
        return Err(ReleaseError::VersionBump(format!(
            "no version assignment found in {}",
            path.display()
        )));
    }
    let result = re.replacen(&contents, 1, format!("${{1}}{new_version}${{3}}"));
    write_file(path, &result)
}

fn bump_pom_xml(path: &Path, new_version: &str) -> Result<(), ReleaseError> {
    let contents = read_file(path)?;

    // Determine search start: skip past </parent> if present, else after </modelVersion>
    let search_start = if let Some(pos) = contents.find("</parent>") {
        pos + "</parent>".len()
    } else if let Some(pos) = contents.find("</modelVersion>") {
        pos + "</modelVersion>".len()
    } else {
        0
    };

    let rest = &contents[search_start..];
    let re = Regex::new(r"<version>[^<]*</version>").unwrap();
    if let Some(m) = re.find(rest) {
        let replacement = format!("<version>{new_version}</version>");
        let mut result = String::with_capacity(contents.len());
        result.push_str(&contents[..search_start + m.start()]);
        result.push_str(&replacement);
        result.push_str(&contents[search_start + m.end()..]);
        write_file(path, &result)
    } else {
        Err(ReleaseError::VersionBump(format!(
            "no <version> element found in {}",
            path.display()
        )))
    }
}

fn bump_go_version(path: &Path, new_version: &str) -> Result<(), ReleaseError> {
    let contents = read_file(path)?;
    let re = Regex::new(r#"((?:var|const)\s+Version\s*(?:string\s*)?=\s*")([^"]*)(")"#).unwrap();
    if !re.is_match(&contents) {
        return Err(ReleaseError::VersionBump(format!(
            "no Version variable found in {}",
            path.display()
        )));
    }
    let result = re.replacen(&contents, 1, format!("${{1}}{new_version}${{3}}"));
    write_file(path, &result)
}

/// Extract a string array from a nested TOML path (e.g. `["workspace", "members"]`).
fn extract_toml_string_array(doc: &toml_edit::DocumentMut, keys: &[&str]) -> Vec<String> {
    let mut item: Option<&toml_edit::Item> = None;
    for key in keys {
        item = match item {
            None => doc.get(key),
            Some(parent) => parent.get(key),
        };
        if item.is_none() {
            return vec![];
        }
    }
    item.and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

/// Resolve workspace member glob patterns into manifest file paths.
/// Each glob is resolved relative to `root_dir`, and `manifest_name` is appended
/// to each matched directory (e.g. "Cargo.toml", "package.json", "pyproject.toml").
fn resolve_member_globs(root_dir: &Path, patterns: &[String], manifest_name: &str) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for pattern in patterns {
        let full_pattern = root_dir.join(pattern).to_string_lossy().into_owned();
        let Ok(entries) = glob::glob(&full_pattern) else {
            continue;
        };
        for entry in entries.flatten() {
            let manifest = if entry.is_dir() {
                entry.join(manifest_name)
            } else {
                continue;
            };
            if manifest.exists() {
                paths.push(manifest);
            }
        }
    }
    paths
}

fn read_file(path: &Path) -> Result<String, ReleaseError> {
    fs::read_to_string(path)
        .map_err(|e| ReleaseError::VersionBump(format!("failed to read {}: {e}", path.display())))
}

fn write_file(path: &Path, contents: &str) -> Result<(), ReleaseError> {
    fs::write(path, contents)
        .map_err(|e| ReleaseError::VersionBump(format!("failed to write {}: {e}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bump a manifest and then sync its lock files, mirroring what the bump
    /// stage does: all manifests first, then one lock pass with the complete
    /// set of bumped package names.
    fn bump_and_sync(path: &Path, new_version: &str) -> Result<BumpOutcome, ReleaseError> {
        let outcome = bump_version_file(path, new_version)?;
        let mut bumped: Vec<String> = vec![path.to_string_lossy().into_owned()];
        bumped.extend(
            outcome
                .extra_files
                .iter()
                .map(|p| p.to_string_lossy().into_owned()),
        );
        sync_lock_files(&bumped, new_version, &outcome.package_names)?;
        Ok(outcome)
    }

    #[test]
    fn bump_cargo_toml_package_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Cargo.toml");
        fs::write(
            &path,
            r#"[package]
name = "my-crate"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = "1"
"#,
        )
        .unwrap();

        bump_version_file(&path, "1.2.3").unwrap();

        let contents = fs::read_to_string(&path).unwrap();
        assert!(contents.contains("version = \"1.2.3\""));
        assert!(contents.contains("name = \"my-crate\""));
        assert!(contents.contains("serde = \"1\""));
    }

    #[test]
    fn bump_cargo_toml_workspace_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Cargo.toml");
        fs::write(
            &path,
            r#"[workspace]
members = ["crates/*"]

[workspace.package]
version = "0.0.1"
edition = "2021"
"#,
        )
        .unwrap();

        bump_version_file(&path, "2.0.0").unwrap();

        let contents = fs::read_to_string(&path).unwrap();
        assert!(contents.contains("version = \"2.0.0\""));
        assert!(contents.contains("members = [\"crates/*\"]"));
    }

    #[test]
    fn bump_package_json_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("package.json");
        fs::write(
            &path,
            r#"{
  "name": "my-pkg",
  "version": "0.0.0",
  "description": "test"
}"#,
        )
        .unwrap();

        bump_version_file(&path, "3.1.0").unwrap();

        let contents = fs::read_to_string(&path).unwrap();
        let value: serde_json::Value = serde_json::from_str(&contents).unwrap();
        assert_eq!(value["version"], "3.1.0");
        assert_eq!(value["name"], "my-pkg");
        assert_eq!(value["description"], "test");
        assert!(contents.ends_with('\n'));
    }

    #[test]
    fn bump_pyproject_toml_project_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pyproject.toml");
        fs::write(
            &path,
            r#"[project]
name = "my-project"
version = "0.1.0"
description = "A test project"
"#,
        )
        .unwrap();

        bump_version_file(&path, "1.0.0").unwrap();

        let contents = fs::read_to_string(&path).unwrap();
        assert!(contents.contains("version = \"1.0.0\""));
        assert!(contents.contains("name = \"my-project\""));
    }

    #[test]
    fn bump_pyproject_toml_poetry_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pyproject.toml");
        fs::write(
            &path,
            r#"[tool.poetry]
name = "my-poetry-project"
version = "0.2.0"
description = "A poetry project"
"#,
        )
        .unwrap();

        bump_version_file(&path, "0.3.0").unwrap();

        let contents = fs::read_to_string(&path).unwrap();
        assert!(contents.contains("version = \"0.3.0\""));
        assert!(contents.contains("name = \"my-poetry-project\""));
    }

    #[test]
    fn bump_unknown_file_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("unknown.txt");
        fs::write(&path, "version = 1").unwrap();

        let err = bump_version_file(&path, "1.0.0").unwrap_err();
        assert!(matches!(err, ReleaseError::VersionBump(_)));
        assert!(err.to_string().contains("unsupported"));
    }

    #[test]
    fn bump_build_gradle_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("build.gradle");
        fs::write(
            &path,
            r#"plugins {
    id 'java'
}

group = 'com.example'
version = '1.0.0'

dependencies {
    implementation 'org.slf4j:slf4j-api:2.0.0'
}
"#,
        )
        .unwrap();

        bump_version_file(&path, "2.0.0").unwrap();

        let contents = fs::read_to_string(&path).unwrap();
        assert!(contents.contains("version = '2.0.0'"));
        assert!(contents.contains("group = 'com.example'"));
        // dependency version must not change
        assert!(contents.contains("slf4j-api:2.0.0"));
    }

    #[test]
    fn bump_build_gradle_kts_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("build.gradle.kts");
        fs::write(
            &path,
            r#"plugins {
    kotlin("jvm") version "1.9.0"
}

group = "com.example"
version = "1.0.0"

dependencies {
    implementation("org.slf4j:slf4j-api:2.0.0")
}
"#,
        )
        .unwrap();

        bump_version_file(&path, "3.0.0").unwrap();

        let contents = fs::read_to_string(&path).unwrap();
        assert!(contents.contains("version = \"3.0.0\""));
        assert!(contents.contains("group = \"com.example\""));
    }

    #[test]
    fn bump_pom_xml_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pom.xml");
        fs::write(
            &path,
            r#"<?xml version="1.0" encoding="UTF-8"?>
<project>
    <modelVersion>4.0.0</modelVersion>
    <groupId>com.example</groupId>
    <artifactId>my-app</artifactId>
    <version>1.0.0</version>
</project>
"#,
        )
        .unwrap();

        bump_version_file(&path, "2.0.0").unwrap();

        let contents = fs::read_to_string(&path).unwrap();
        assert!(contents.contains("<version>2.0.0</version>"));
        assert!(contents.contains("<groupId>com.example</groupId>"));
    }

    #[test]
    fn bump_pom_xml_with_parent_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pom.xml");
        fs::write(
            &path,
            r#"<?xml version="1.0" encoding="UTF-8"?>
<project>
    <modelVersion>4.0.0</modelVersion>
    <parent>
        <groupId>com.example</groupId>
        <artifactId>parent</artifactId>
        <version>5.0.0</version>
    </parent>
    <artifactId>my-app</artifactId>
    <version>1.0.0</version>
</project>
"#,
        )
        .unwrap();

        bump_version_file(&path, "2.0.0").unwrap();

        let contents = fs::read_to_string(&path).unwrap();
        // Parent version must NOT be changed
        assert!(contents.contains("<version>5.0.0</version>"));
        // Project version must be changed
        assert!(contents.contains("<version>2.0.0</version>"));
        // Verify there are exactly two <version> tags with expected values
        let version_count: Vec<&str> = contents.matches("<version>").collect();
        assert_eq!(version_count.len(), 2);
    }

    #[test]
    fn bump_cargo_toml_workspace_dependencies_with_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Cargo.toml");
        fs::write(
            &path,
            r#"[workspace]
members = ["crates/*"]

[workspace.package]
version = "0.1.0"
edition = "2021"

[workspace.dependencies]
# Internal crates
sr-core = { path = "crates/sr-core", version = "0.1.0" }
sr-git = { path = "crates/sr-git", version = "0.1.0" }
# External dep should not change
serde = { version = "1", features = ["derive"] }
"#,
        )
        .unwrap();

        bump_version_file(&path, "2.0.0").unwrap();

        let contents = fs::read_to_string(&path).unwrap();
        let doc: toml_edit::DocumentMut = contents.parse().unwrap();

        // workspace.package.version should be bumped
        assert_eq!(
            doc["workspace"]["package"]["version"].as_str().unwrap(),
            "2.0.0"
        );
        // Internal path deps should have their version bumped
        assert_eq!(
            doc["workspace"]["dependencies"]["sr-core"]["version"]
                .as_str()
                .unwrap(),
            "2.0.0"
        );
        assert_eq!(
            doc["workspace"]["dependencies"]["sr-git"]["version"]
                .as_str()
                .unwrap(),
            "2.0.0"
        );
        // External dep version must NOT change
        assert_eq!(
            doc["workspace"]["dependencies"]["serde"]["version"]
                .as_str()
                .unwrap(),
            "1"
        );
    }

    #[test]
    fn bump_go_version_var() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("version.go");
        fs::write(
            &path,
            r#"package main

var Version = "1.0.0"

func main() {}
"#,
        )
        .unwrap();

        bump_version_file(&path, "2.0.0").unwrap();

        let contents = fs::read_to_string(&path).unwrap();
        assert!(contents.contains(r#"var Version = "2.0.0""#));
    }

    #[test]
    fn bump_go_version_const() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("version.go");
        fs::write(
            &path,
            r#"package main

const Version string = "0.5.0"

func main() {}
"#,
        )
        .unwrap();

        bump_version_file(&path, "0.6.0").unwrap();

        let contents = fs::read_to_string(&path).unwrap();
        assert!(contents.contains(r#"const Version string = "0.6.0""#));
    }

    // --- workspace auto-discovery tests ---

    #[test]
    fn bump_cargo_workspace_discovers_members() {
        let dir = tempfile::tempdir().unwrap();

        // Create workspace root
        let root = dir.path().join("Cargo.toml");
        fs::write(
            &root,
            r#"[workspace]
members = ["crates/*"]

[workspace.package]
version = "1.0.0"
edition = "2021"

[workspace.dependencies]
my-core = { path = "crates/core", version = "1.0.0" }
"#,
        )
        .unwrap();

        // Create member with hardcoded version
        fs::create_dir_all(dir.path().join("crates/core")).unwrap();
        let member = dir.path().join("crates/core/Cargo.toml");
        fs::write(
            &member,
            r#"[package]
name = "my-core"
version = "1.0.0"
edition = "2021"
"#,
        )
        .unwrap();

        // Create member that uses workspace inheritance (should be skipped)
        fs::create_dir_all(dir.path().join("crates/cli")).unwrap();
        let inherited_member = dir.path().join("crates/cli/Cargo.toml");
        fs::write(
            &inherited_member,
            r#"[package]
name = "my-cli"
version.workspace = true
edition.workspace = true
"#,
        )
        .unwrap();

        let extra = bump_version_file(&root, "2.0.0").unwrap();

        // Root should be bumped
        let root_contents = fs::read_to_string(&root).unwrap();
        assert!(root_contents.contains("version = \"2.0.0\""));

        // Workspace dep should be bumped
        let doc: toml_edit::DocumentMut = root_contents.parse().unwrap();
        assert_eq!(
            doc["workspace"]["dependencies"]["my-core"]["version"]
                .as_str()
                .unwrap(),
            "2.0.0"
        );

        // Member with hardcoded version should be bumped
        let member_contents = fs::read_to_string(&member).unwrap();
        assert!(member_contents.contains("version = \"2.0.0\""));

        // Member with workspace inheritance should NOT be modified
        let inherited_contents = fs::read_to_string(&inherited_member).unwrap();
        assert!(inherited_contents.contains("version.workspace = true"));

        // Only the hardcoded member should be in extra
        assert_eq!(extra.extra_files.len(), 1);
        assert_eq!(extra.extra_files[0], member);
    }

    #[test]
    fn bump_npm_workspace_discovers_members() {
        let dir = tempfile::tempdir().unwrap();

        // Create root package.json with workspaces
        let root = dir.path().join("package.json");
        fs::write(
            &root,
            r#"{
  "name": "my-monorepo",
  "version": "1.0.0",
  "workspaces": ["packages/*"]
}"#,
        )
        .unwrap();

        // Create member
        fs::create_dir_all(dir.path().join("packages/core")).unwrap();
        let member = dir.path().join("packages/core/package.json");
        fs::write(
            &member,
            r#"{
  "name": "@my/core",
  "version": "1.0.0"
}"#,
        )
        .unwrap();

        // Create member without version (should be skipped)
        fs::create_dir_all(dir.path().join("packages/utils")).unwrap();
        let no_version_member = dir.path().join("packages/utils/package.json");
        fs::write(
            &no_version_member,
            r#"{
  "name": "@my/utils",
  "private": true
}"#,
        )
        .unwrap();

        let extra = bump_version_file(&root, "2.0.0").unwrap();

        // Root bumped
        let root_contents: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&root).unwrap()).unwrap();
        assert_eq!(root_contents["version"], "2.0.0");

        // Member with version bumped
        let member_contents: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&member).unwrap()).unwrap();
        assert_eq!(member_contents["version"], "2.0.0");

        // Member without version untouched
        let utils_contents: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&no_version_member).unwrap()).unwrap();
        assert!(utils_contents.get("version").is_none());

        assert_eq!(extra.extra_files.len(), 1);
        assert_eq!(extra.extra_files[0], member);
    }

    #[test]
    fn bump_uv_workspace_discovers_members() {
        let dir = tempfile::tempdir().unwrap();

        // Create root pyproject.toml with uv workspace
        let root = dir.path().join("pyproject.toml");
        fs::write(
            &root,
            r#"[project]
name = "my-monorepo"
version = "1.0.0"

[tool.uv.workspace]
members = ["packages/*"]
"#,
        )
        .unwrap();

        // Create member
        fs::create_dir_all(dir.path().join("packages/core")).unwrap();
        let member = dir.path().join("packages/core/pyproject.toml");
        fs::write(
            &member,
            r#"[project]
name = "my-core"
version = "1.0.0"
"#,
        )
        .unwrap();

        let extra = bump_version_file(&root, "2.0.0").unwrap();

        // Root bumped
        let root_contents = fs::read_to_string(&root).unwrap();
        assert!(root_contents.contains("version = \"2.0.0\""));

        // Member bumped
        let member_contents = fs::read_to_string(&member).unwrap();
        assert!(member_contents.contains("version = \"2.0.0\""));

        assert_eq!(extra.extra_files.len(), 1);
        assert_eq!(extra.extra_files[0], member);
    }

    #[test]
    fn bump_cargo_workspace_refreshes_lockfile() {
        let dir = tempfile::tempdir().unwrap();

        // Workspace root
        let root = dir.path().join("Cargo.toml");
        fs::write(
            &root,
            r#"[workspace]
members = ["crates/*"]

[workspace.package]
version = "1.0.0"
"#,
        )
        .unwrap();

        // Member with hardcoded version
        fs::create_dir_all(dir.path().join("crates/my-core")).unwrap();
        fs::write(
            dir.path().join("crates/my-core/Cargo.toml"),
            r#"[package]
name = "my-core"
version = "1.0.0"
"#,
        )
        .unwrap();

        // Cargo.lock with both a workspace member and a published dep
        let lock = dir.path().join("Cargo.lock");
        fs::write(
            &lock,
            r#"version = 3

[[package]]
name = "my-core"
version = "1.0.0"

[[package]]
name = "serde"
version = "1.0.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "abc123"
"#,
        )
        .unwrap();

        bump_and_sync(&root, "2.0.0").unwrap();

        let lock_contents = fs::read_to_string(&lock).unwrap();
        let doc: toml_edit::DocumentMut = lock_contents.parse().unwrap();
        let packages = doc["package"].as_array_of_tables().unwrap();

        let my_core = packages
            .iter()
            .find(|p| p.get("name").and_then(|n| n.as_str()) == Some("my-core"))
            .unwrap();
        assert_eq!(my_core["version"].as_str().unwrap(), "2.0.0");

        // Published dep must NOT be touched
        let serde = packages
            .iter()
            .find(|p| p.get("name").and_then(|n| n.as_str()) == Some("serde"))
            .unwrap();
        assert_eq!(serde["version"].as_str().unwrap(), "1.0.0");
        assert!(serde.contains_key("source"));
        assert_eq!(serde["checksum"].as_str().unwrap(), "abc123");
    }

    #[test]
    fn bump_uv_workspace_refreshes_lockfile() {
        let dir = tempfile::tempdir().unwrap();

        let root = dir.path().join("pyproject.toml");
        fs::write(
            &root,
            r#"[project]
name = "my-monorepo"
version = "1.0.0"

[tool.uv.workspace]
members = ["packages/*"]
"#,
        )
        .unwrap();

        fs::create_dir_all(dir.path().join("packages/core")).unwrap();
        fs::write(
            dir.path().join("packages/core/pyproject.toml"),
            r#"[project]
name = "My_Core"
version = "1.0.0"
"#,
        )
        .unwrap();

        // Lock holds the workspace root (virtual), a member (editable), a
        // registry dep, and a registry dep that shares a member's name.
        let lock = dir.path().join("uv.lock");
        fs::write(
            &lock,
            r#"version = 1
requires-python = ">=3.12"

[[package]]
name = "my-monorepo"
version = "1.0.0"
source = { virtual = "." }

[[package]]
name = "my-core"
version = "1.0.0"
source = { editable = "packages/core" }
dependencies = [
    { name = "requests" },
]

[package.metadata]
requires-dist = [{ name = "requests", specifier = ">=2" }]

[[package]]
name = "requests"
version = "2.31.0"
source = { registry = "https://pypi.org/simple" }

[[package]]
name = "my-monorepo"
version = "0.9.0"
source = { registry = "https://pypi.org/simple" }
"#,
        )
        .unwrap();

        bump_and_sync(&root, "2.0.0").unwrap();

        let doc: toml_edit::DocumentMut = fs::read_to_string(&lock).unwrap().parse().unwrap();
        let packages = doc["package"].as_array_of_tables().unwrap();

        // Virtual root and editable member follow the bump. The member's
        // pyproject name (`My_Core`) is matched against the lock's PEP 503
        // normalized `my-core`.
        assert_eq!(packages.get(0).unwrap()["version"].as_str(), Some("2.0.0"));
        assert_eq!(packages.get(1).unwrap()["version"].as_str(), Some("2.0.0"));

        // The member's `[package.metadata]` sub-table stays attached.
        assert!(packages.get(1).unwrap()["metadata"]["requires-dist"].is_value());

        // Registry entries are untouched, including the name collision.
        assert_eq!(packages.get(2).unwrap()["version"].as_str(), Some("2.31.0"));
        assert_eq!(packages.get(3).unwrap()["version"].as_str(), Some("0.9.0"));
    }

    #[test]
    fn bump_uv_single_package_refreshes_lockfile() {
        let dir = tempfile::tempdir().unwrap();

        let root = dir.path().join("pyproject.toml");
        fs::write(
            &root,
            r#"[project]
name = "solo"
version = "1.0.0"
"#,
        )
        .unwrap();

        let lock = dir.path().join("uv.lock");
        fs::write(
            &lock,
            r#"version = 1

[[package]]
name = "solo"
version = "1.0.0"
source = { editable = "." }
"#,
        )
        .unwrap();

        bump_and_sync(&root, "1.1.0").unwrap();

        let doc: toml_edit::DocumentMut = fs::read_to_string(&lock).unwrap().parse().unwrap();
        let packages = doc["package"].as_array_of_tables().unwrap();
        assert_eq!(packages.get(0).unwrap()["version"].as_str(), Some("1.1.0"));
    }

    #[test]
    fn bump_uv_without_lockfile_is_a_noop() {
        let dir = tempfile::tempdir().unwrap();
        // `.git` stops the upward walk, so a stray uv.lock above the temp dir
        // can never be picked up.
        fs::create_dir_all(dir.path().join(".git")).unwrap();

        let root = dir.path().join("pyproject.toml");
        fs::write(
            &root,
            r#"[project]
name = "solo"
version = "1.0.0"
"#,
        )
        .unwrap();

        bump_and_sync(&root, "1.1.0").unwrap();
        assert!(!dir.path().join("uv.lock").exists());
    }

    #[test]
    fn bump_npm_workspace_refreshes_package_lock() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("packages/core")).unwrap();

        let root = dir.path().join("package.json");
        fs::write(
            &root,
            r#"{"name": "root", "version": "1.0.0", "workspaces": ["packages/*"]}"#,
        )
        .unwrap();
        fs::write(
            dir.path().join("packages/core/package.json"),
            r#"{"name": "@my/core", "version": "1.0.0"}"#,
        )
        .unwrap();

        // Shape produced by `npm install --package-lock-only` (lockfileVersion 3).
        let lock = dir.path().join("package-lock.json");
        fs::write(
            &lock,
            r#"{
  "name": "root",
  "version": "1.0.0",
  "lockfileVersion": 3,
  "requires": true,
  "packages": {
    "": {
      "name": "root",
      "version": "1.0.0",
      "workspaces": ["packages/*"]
    },
    "node_modules/@my/core": {
      "resolved": "packages/core",
      "link": true
    },
    "node_modules/left-pad": {
      "name": "left-pad",
      "version": "1.3.0",
      "resolved": "https://registry.npmjs.org/left-pad/-/left-pad-1.3.0.tgz"
    },
    "packages/core": {
      "name": "@my/core",
      "version": "1.0.0"
    }
  }
}
"#,
        )
        .unwrap();

        bump_and_sync(&root, "2.0.0").unwrap();

        let v: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&lock).unwrap()).unwrap();
        assert_eq!(v["version"], "2.0.0");
        assert_eq!(v["packages"][""]["version"], "2.0.0");
        assert_eq!(v["packages"]["packages/core"]["version"], "2.0.0");
        // The `link: true` pointer has no version to gain.
        assert!(
            v["packages"]["node_modules/@my/core"]
                .get("version")
                .is_none()
        );
        // Registry dep untouched.
        assert_eq!(v["packages"]["node_modules/left-pad"]["version"], "1.3.0");
    }

    #[test]
    fn bump_poetry_refreshes_sibling_path_dep_in_lock() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("packages/core")).unwrap();

        // Poetry members are bumped via uv-style workspace discovery; here the
        // root declares the member so both names reach the lock sync.
        let root = dir.path().join("pyproject.toml");
        fs::write(
            &root,
            r#"[project]
name = "my-api"
version = "1.0.0"

[tool.uv.workspace]
members = ["packages/*"]
"#,
        )
        .unwrap();
        fs::write(
            dir.path().join("packages/core/pyproject.toml"),
            r#"[project]
name = "my-core"
version = "1.0.0"
"#,
        )
        .unwrap();

        // Shape produced by `poetry lock` (lock-version 2.1).
        let lock = dir.path().join("poetry.lock");
        fs::write(
            &lock,
            r#"# This file is automatically @generated by Poetry 2.4.1 and should not be changed by hand.

[[package]]
name = "my-core"
version = "1.0.0"
description = ""
optional = false
python-versions = ">=3.9"
groups = ["main"]
files = []
develop = true

[package.source]
type = "directory"
url = "packages/core"

[[package]]
name = "certifi"
version = "2024.2.2"
description = "certs"
optional = false
python-versions = ">=3.6"
groups = ["main"]

[metadata]
lock-version = "2.1"
python-versions = "^3.9"
content-hash = "abc123"
"#,
        )
        .unwrap();

        bump_and_sync(&root, "2.0.0").unwrap();

        let doc: toml_edit::DocumentMut = fs::read_to_string(&lock).unwrap().parse().unwrap();
        let packages = doc["package"].as_array_of_tables().unwrap();
        // Sibling path dep follows the bump.
        assert_eq!(packages.get(0).unwrap()["version"].as_str(), Some("2.0.0"));
        // Registry dep untouched.
        assert_eq!(
            packages.get(1).unwrap()["version"].as_str(),
            Some("2024.2.2")
        );
        // content-hash covers declared specs, not resolved versions.
        assert_eq!(doc["metadata"]["content-hash"].as_str(), Some("abc123"));
    }

    #[test]
    fn cargo_member_path_deps_follow_the_bump() {
        let dir = tempfile::tempdir().unwrap();

        let root = dir.path().join("Cargo.toml");
        fs::write(
            &root,
            "[workspace]\nmembers = [\"crates/*\"]\n\n[workspace.package]\nversion = \"1.0.0\"\n",
        )
        .unwrap();

        fs::create_dir_all(dir.path().join("crates/core")).unwrap();
        fs::write(
            dir.path().join("crates/core/Cargo.toml"),
            "[package]\nname = \"my-core\"\nversion = \"1.0.0\"\n",
        )
        .unwrap();

        // Inherits its version but declares a path dep on a sibling — the case
        // that made the workspace unresolvable after a major bump.
        fs::create_dir_all(dir.path().join("crates/cli")).unwrap();
        let cli = dir.path().join("crates/cli/Cargo.toml");
        fs::write(
            &cli,
            r#"[package]
name = "my-cli"
version.workspace = true

[dependencies]
my-core = { path = "../core", version = "1.0.0" }
serde = { version = "1", features = ["derive"] }

[dev-dependencies]
my-testkit = { path = "../testkit", version = "1.0.0" }

[build-dependencies]
no-version-path-dep = { path = "../gen" }
"#,
        )
        .unwrap();

        let outcome = bump_version_file(&root, "2.0.0").unwrap();

        let doc: toml_edit::DocumentMut = fs::read_to_string(&cli).unwrap().parse().unwrap();
        assert_eq!(
            doc["dependencies"]["my-core"]["version"].as_str(),
            Some("2.0.0")
        );
        assert_eq!(
            doc["dev-dependencies"]["my-testkit"]["version"].as_str(),
            Some("2.0.0")
        );
        // External dep untouched.
        assert_eq!(doc["dependencies"]["serde"]["version"].as_str(), Some("1"));
        // A path dep with no `version` is deliberately unpublished — leave it.
        assert!(
            doc["build-dependencies"]["no-version-path-dep"]
                .get("version")
                .is_none()
        );
        // Version inheritance is preserved, not overwritten.
        assert!(
            fs::read_to_string(&cli)
                .unwrap()
                .contains("version.workspace = true")
        );
        // Modified only via its deps, so it still has to be staged.
        assert!(outcome.extra_files.contains(&cli));
    }

    #[test]
    fn npm_sibling_ranges_follow_the_bump() {
        let dir = tempfile::tempdir().unwrap();

        let root = dir.path().join("package.json");
        fs::write(
            &root,
            r#"{"name": "root", "version": "1.0.0", "workspaces": ["packages/*"]}"#,
        )
        .unwrap();

        fs::create_dir_all(dir.path().join("packages/core")).unwrap();
        fs::write(
            dir.path().join("packages/core/package.json"),
            r#"{"name": "@my/core", "version": "1.0.0"}"#,
        )
        .unwrap();

        fs::create_dir_all(dir.path().join("packages/api")).unwrap();
        let api = dir.path().join("packages/api/package.json");
        fs::write(
            &api,
            r#"{
  "name": "@my/api",
  "version": "1.0.0",
  "dependencies": {
    "@my/core": "^1.0.0",
    "left-pad": "^1.3.0"
  },
  "devDependencies": { "@my/core": "1.0.0" },
  "peerDependencies": { "@my/core": ">=1.0.0" },
  "optionalDependencies": { "@my/core": "workspace:*" }
}"#,
        )
        .unwrap();

        bump_version_file(&root, "2.0.0").unwrap();

        let v: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&api).unwrap()).unwrap();
        // Caret and exact specs are retargeted.
        assert_eq!(v["dependencies"]["@my/core"], "^2.0.0");
        assert_eq!(v["devDependencies"]["@my/core"], "2.0.0");
        // A comparator range is still satisfied by 2.0.0 — leave it alone.
        assert_eq!(v["peerDependencies"]["@my/core"], ">=1.0.0");
        // The workspace protocol resolves by path and carries no version.
        assert_eq!(v["optionalDependencies"]["@my/core"], "workspace:*");
        // External dep untouched.
        assert_eq!(v["dependencies"]["left-pad"], "^1.3.0");
    }

    #[test]
    fn retarget_range_only_touches_caret_tilde_and_exact() {
        assert_eq!(retarget_range("^1.0.0", "2.0.0").as_deref(), Some("^2.0.0"));
        assert_eq!(retarget_range("~1.0.0", "2.0.0").as_deref(), Some("~2.0.0"));
        assert_eq!(retarget_range("1.0.0", "2.0.0").as_deref(), Some("2.0.0"));
        // Already current — no spurious rewrite.
        assert_eq!(retarget_range("^2.0.0", "2.0.0"), None);
        // Ranges, wildcards and protocols are left alone.
        assert_eq!(retarget_range(">=1.0.0", "2.0.0"), None);
        assert_eq!(retarget_range("1.x", "2.0.0"), None);
        assert_eq!(retarget_range("*", "2.0.0"), None);
        assert_eq!(retarget_range("workspace:*", "2.0.0"), None);
        assert_eq!(retarget_range("file:../core", "2.0.0"), None);
        assert_eq!(retarget_range("1.0.0 - 2.0.0", "3.0.0"), None);
    }

    #[test]
    fn package_json_key_order_survives_a_bump() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("package.json");
        fs::write(
            &path,
            "{\n  \"name\": \"z-pkg\",\n  \"version\": \"1.0.0\",\n  \"private\": true,\n  \"author\": \"a\"\n}",
        )
        .unwrap();

        bump_version_file(&path, "2.0.0").unwrap();

        let contents = fs::read_to_string(&path).unwrap();
        let order: Vec<&str> = ["name", "version", "private", "author"]
            .into_iter()
            .filter(|k| contents.contains(&format!("\"{k}\"")))
            .collect();
        let positions: Vec<usize> = order
            .iter()
            .map(|k| contents.find(&format!("\"{k}\"")).unwrap())
            .collect();
        let mut sorted = positions.clone();
        sorted.sort_unstable();
        assert_eq!(
            positions, sorted,
            "key order must be preserved, not alphabetized:\n{contents}"
        );
    }

    /// A poetry monorepo lists each member in `version_files`. `my-core`'s
    /// stale entry lives in `packages/api/poetry.lock` — not reachable from
    /// `packages/core/pyproject.toml`, and not nameable from `my-api`'s own
    /// bump. Only a sync that runs after every manifest, with every name,
    /// catches it.
    #[test]
    fn poetry_monorepo_syncs_a_siblings_lock() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("packages/api")).unwrap();
        fs::create_dir_all(dir.path().join("packages/core")).unwrap();
        fs::create_dir_all(dir.path().join(".git")).unwrap();

        let api = dir.path().join("packages/api/pyproject.toml");
        fs::write(&api, "[project]\nname = \"my-api\"\nversion = \"1.0.0\"\n").unwrap();
        let core = dir.path().join("packages/core/pyproject.toml");
        fs::write(
            &core,
            "[project]\nname = \"my-core\"\nversion = \"1.0.0\"\n",
        )
        .unwrap();

        let lock = dir.path().join("packages/api/poetry.lock");
        fs::write(
            &lock,
            r#"[[package]]
name = "my-core"
version = "1.0.0"
description = ""
optional = false
python-versions = ">=3.9"
groups = ["main"]
files = []
develop = true

[package.source]
type = "directory"
url = "../core"

[metadata]
lock-version = "2.1"
python-versions = "^3.9"
content-hash = "abc123"
"#,
        )
        .unwrap();

        // Exactly what the bump stage does across multiple version_files.
        let mut staged: Vec<String> = Vec::new();
        let mut names: Vec<String> = Vec::new();
        for manifest in [&api, &core] {
            let outcome = bump_version_file(manifest, "2.0.0").unwrap();
            staged.push(manifest.to_string_lossy().into_owned());
            names.extend(outcome.package_names);
        }
        let synced = sync_lock_files(&staged, "2.0.0", &names).unwrap();

        assert_eq!(synced, vec![lock.clone()]);
        let doc: toml_edit::DocumentMut = fs::read_to_string(&lock).unwrap().parse().unwrap();
        let packages = doc["package"].as_array_of_tables().unwrap();
        assert_eq!(packages.get(0).unwrap()["version"].as_str(), Some("2.0.0"));
    }

    #[test]
    fn newer_lock_format_is_refused_rather_than_half_edited() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("pyproject.toml");
        fs::write(&root, "[project]\nname = \"solo\"\nversion = \"1.0.0\"\n").unwrap();

        let lock = dir.path().join("uv.lock");
        let original = format!(
            "version = {}\n\n[[package]]\nname = \"solo\"\nversion = \"1.0.0\"\nsource = {{ editable = \".\" }}\n",
            UV_LOCK_MAX_VERSION + 1
        );
        fs::write(&lock, &original).unwrap();

        let err = bump_and_sync(&root, "1.1.0").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("format version"), "got: {msg}");
        assert!(msg.contains("Upgrade sr"), "got: {msg}");
        // The lock must be left exactly as it was, not partially rewritten.
        assert_eq!(fs::read_to_string(&lock).unwrap(), original);
    }

    #[test]
    fn newer_uv_revision_warns_but_still_syncs() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("pyproject.toml");
        fs::write(&root, "[project]\nname = \"solo\"\nversion = \"1.0.0\"\n").unwrap();

        let lock = dir.path().join("uv.lock");
        fs::write(
            &lock,
            format!(
                "version = {UV_LOCK_MAX_VERSION}\nrevision = {}\n\n[[package]]\nname = \"solo\"\nversion = \"1.0.0\"\nsource = {{ editable = \".\" }}\n",
                UV_LOCK_MAX_REVISION + 1
            ),
        )
        .unwrap();

        bump_and_sync(&root, "1.1.0").unwrap();

        let doc: toml_edit::DocumentMut = fs::read_to_string(&lock).unwrap().parse().unwrap();
        let packages = doc["package"].as_array_of_tables().unwrap();
        assert_eq!(packages.get(0).unwrap()["version"].as_str(), Some("1.1.0"));
    }

    /// Every lock file sr stages must be one sr keeps consistent. If a handler
    /// declares a lock name, `sync_lock` has to recognise it — otherwise sr
    /// commits a lock it never updated, which is what this invariant prevents.
    #[test]
    fn every_declared_lock_is_handled_by_sync_lock() {
        for handler in all_handlers() {
            for lock_name in handler.lock_file_names() {
                let dir = tempfile::tempdir().unwrap();
                let lock_path = dir.path().join(lock_name);
                // A syntactically valid but empty lock of each kind: sync must
                // return cleanly (no panic, no parse error, no "unknown file").
                let empty = if lock_name.ends_with(".json") {
                    "{}"
                } else {
                    ""
                };
                fs::write(&lock_path, empty).unwrap();
                let result = handler.sync_lock(&lock_path, "1.0.0", &["x".to_string()]);
                assert!(
                    result.is_ok(),
                    "{} declares {lock_name} but sync_lock failed: {:?}",
                    handler.name(),
                    result.err()
                );
            }
        }
    }

    #[test]
    fn discover_lock_files_finds_uv_lock_from_member_manifest() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".git")).unwrap();
        fs::create_dir_all(dir.path().join("packages/core")).unwrap();
        fs::write(dir.path().join("uv.lock"), "version = 1\n").unwrap();

        let bumped = vec![
            dir.path().join("pyproject.toml").display().to_string(),
            dir.path()
                .join("packages/core/pyproject.toml")
                .display()
                .to_string(),
        ];

        // One entry, found once from the root and once by walking up from the
        // member — the release commit stages it alongside the manifests.
        assert_eq!(
            discover_lock_files(&bumped),
            vec![dir.path().join("uv.lock")]
        );
    }

    #[test]
    fn normalize_dist_name_follows_pep503() {
        assert_eq!(normalize_dist_name("My_Core"), "my-core");
        assert_eq!(normalize_dist_name("my.core"), "my-core");
        assert_eq!(normalize_dist_name("my__-.core"), "my-core");
        assert_eq!(normalize_dist_name("my-core"), "my-core");
    }

    #[test]
    fn bump_non_workspace_returns_empty_extra() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Cargo.toml");
        fs::write(
            &path,
            r#"[package]
name = "solo-crate"
version = "1.0.0"
"#,
        )
        .unwrap();

        let extra = bump_version_file(&path, "2.0.0").unwrap();
        assert!(extra.extra_files.is_empty());
    }

    // --- auto-detection tests ---

    #[test]
    fn detect_cargo_toml() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"x\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();

        let detected = detect_version_files(dir.path());
        assert_eq!(detected, vec!["Cargo.toml"]);
    }

    #[test]
    fn detect_package_json() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("package.json"),
            r#"{"name": "x", "version": "1.0.0"}"#,
        )
        .unwrap();

        let detected = detect_version_files(dir.path());
        assert_eq!(detected, vec!["package.json"]);
    }

    #[test]
    fn detect_pyproject_toml() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("pyproject.toml"),
            "[project]\nname = \"x\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();

        let detected = detect_version_files(dir.path());
        assert_eq!(detected, vec!["pyproject.toml"]);
    }

    #[test]
    fn detect_multiple_ecosystems() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"x\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("package.json"),
            r#"{"name": "x", "version": "1.0.0"}"#,
        )
        .unwrap();

        let detected = detect_version_files(dir.path());
        assert!(detected.contains(&"Cargo.toml".to_string()));
        assert!(detected.contains(&"package.json".to_string()));
    }

    #[test]
    fn detect_empty_directory() {
        let dir = tempfile::tempdir().unwrap();
        let detected = detect_version_files(dir.path());
        assert!(detected.is_empty());
    }

    #[test]
    fn detect_go_version_file() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("version.go"),
            "package main\n\nvar Version = \"1.0.0\"\n",
        )
        .unwrap();

        let detected = detect_version_files(dir.path());
        assert_eq!(detected, vec!["version.go"]);
    }

    #[test]
    fn is_supported_recognizes_all_types() {
        assert!(is_supported_version_file("Cargo.toml"));
        assert!(is_supported_version_file("package.json"));
        assert!(is_supported_version_file("pyproject.toml"));
        assert!(is_supported_version_file("pom.xml"));
        assert!(is_supported_version_file("build.gradle"));
        assert!(is_supported_version_file("build.gradle.kts"));
        assert!(is_supported_version_file("version.go"));
        assert!(!is_supported_version_file("unknown.txt"));
    }
}
