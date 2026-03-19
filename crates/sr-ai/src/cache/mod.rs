pub mod fingerprint;
pub mod store;

use crate::commands::commit::CommitPlan;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use fingerprint::{compute_fingerprints, sha256_hex};
use store::{CacheEntry, cache_dir, list_entries, read_entry, write_entry};

/// Result of a cache lookup.
pub enum CacheLookup {
    /// Exact fingerprint match — use cached plan directly.
    ExactHit(CommitPlan),
    /// Partial match — provides previous plan + delta summary as hints.
    IncrementalHit {
        previous_plan: CommitPlan,
        delta_summary: String,
    },
    /// No useful cached data.
    Miss,
}

pub struct CacheManager {
    repo_root: PathBuf,
    dir: PathBuf,
    fingerprints: BTreeMap<String, String>,
    state_key: String,
}

impl CacheManager {
    /// Build a new CacheManager, computing fingerprints and state key.
    /// Returns None if cache dir can't be resolved (graceful degradation).
    pub fn new(
        repo_root: &Path,
        staged_only: bool,
        user_message: Option<&str>,
        backend: &str,
        model: &str,
    ) -> Option<Self> {
        let dir = cache_dir(repo_root)?;
        let fingerprints = compute_fingerprints(repo_root, staged_only);

        let state_key = compute_state_key(&fingerprints, staged_only, user_message, backend, model);

        Some(Self {
            repo_root: repo_root.to_path_buf(),
            dir,
            fingerprints,
            state_key,
        })
    }

    /// Look up the cache. Returns ExactHit, IncrementalHit, or Miss.
    pub fn lookup(&self) -> CacheLookup {
        // Tier 1: exact match
        let exact_path = store::entry_path(&self.dir, &self.state_key);
        if let Ok(entry) = read_entry(&exact_path) {
            return CacheLookup::ExactHit(entry.plan);
        }

        // Tier 2: find best incremental candidate
        let entries = match list_entries(&self.dir) {
            Ok(e) => e,
            Err(_) => return CacheLookup::Miss,
        };

        if entries.is_empty() {
            return CacheLookup::Miss;
        }

        // Pick the most recent entry as the incremental candidate
        let candidate = &entries[0];
        let delta = compute_delta(&candidate.fingerprints, &self.fingerprints);

        // Only use incremental if ≤50% of files changed
        let total = self.fingerprints.len().max(candidate.fingerprints.len());
        let changed = delta.changed.len() + delta.added.len() + delta.removed.len();

        if total == 0 || changed * 2 > total {
            return CacheLookup::Miss;
        }

        let summary = format_delta_summary(&delta);
        CacheLookup::IncrementalHit {
            previous_plan: candidate.plan.clone(),
            delta_summary: summary,
        }
    }

    /// Store a plan in the cache.
    pub fn store(&self, plan: &CommitPlan, backend: &str, model: &str) {
        let entry = CacheEntry {
            state_key: self.state_key.clone(),
            fingerprints: self.fingerprints.clone(),
            plan: plan.clone(),
            created_at: store::now_secs(),
            backend: backend.to_string(),
            model: model.to_string(),
        };

        if let Err(e) = write_entry(&self.dir, &entry) {
            eprintln!("Warning: failed to write cache: {e}");
        }
    }

    /// Clear cache for this repo.
    pub fn clear(&self) -> anyhow::Result<usize> {
        store::clear(&self.dir)
    }

    #[allow(dead_code)]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    #[allow(dead_code)]
    pub fn repo_root(&self) -> &Path {
        &self.repo_root
    }
}

/// Compute the full state key from fingerprints + parameters.
fn compute_state_key(
    fingerprints: &BTreeMap<String, String>,
    staged_only: bool,
    user_message: Option<&str>,
    backend: &str,
    model: &str,
) -> String {
    let mut data = String::new();
    for (file, hash) in fingerprints {
        data.push_str(file);
        data.push(':');
        data.push_str(hash);
        data.push('\n');
    }
    data.push_str(&format!("staged:{staged_only}\n"));
    if let Some(msg) = user_message {
        data.push_str(&format!("message:{msg}\n"));
    }
    data.push_str(&format!("backend:{backend}\n"));
    data.push_str(&format!("model:{model}\n"));

    sha256_hex(data.as_bytes())
}

struct FileDelta {
    unchanged: Vec<String>,
    changed: Vec<String>,
    added: Vec<String>,
    removed: Vec<String>,
}

fn compute_delta(old: &BTreeMap<String, String>, new: &BTreeMap<String, String>) -> FileDelta {
    let mut unchanged = Vec::new();
    let mut changed = Vec::new();
    let mut added = Vec::new();
    let mut removed = Vec::new();

    for (file, new_hash) in new {
        match old.get(file) {
            Some(old_hash) if old_hash == new_hash => unchanged.push(file.clone()),
            Some(_) => changed.push(file.clone()),
            None => added.push(file.clone()),
        }
    }

    for file in old.keys() {
        if !new.contains_key(file) {
            removed.push(file.clone());
        }
    }

    FileDelta {
        unchanged,
        changed,
        added,
        removed,
    }
}

fn format_delta_summary(delta: &FileDelta) -> String {
    let mut parts = Vec::new();

    if !delta.unchanged.is_empty() {
        parts.push(format!(
            "Unchanged files (keep previous groupings): {}",
            delta.unchanged.join(", ")
        ));
    }
    if !delta.changed.is_empty() {
        parts.push(format!(
            "Modified files (re-analyze): {}",
            delta.changed.join(", ")
        ));
    }
    if !delta.added.is_empty() {
        parts.push(format!("New files: {}", delta.added.join(", ")));
    }
    if !delta.removed.is_empty() {
        parts.push(format!(
            "Removed files (drop from plan): {}",
            delta.removed.join(", ")
        ));
    }

    parts.join("\n")
}
