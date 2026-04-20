//! Bundled `sr.yaml` examples, embedded at compile time.
//!
//! `sr init <name>` writes one of these to the working directory so users
//! can scaffold a known-good config for their ecosystem and edit from there.

/// An available example: user-facing name and the yaml body.
pub struct Example {
    pub name: &'static str,
    pub description: &'static str,
    pub body: &'static str,
}

pub const EXAMPLES: &[Example] = &[
    Example {
        name: "cargo-single",
        description: "single Rust crate → crates.io",
        body: include_str!("../templates/cargo-single.yaml"),
    },
    Example {
        name: "cargo-workspace",
        description: "Rust workspace (every member at shared version)",
        body: include_str!("../templates/cargo-workspace.yaml"),
    },
    Example {
        name: "npm-single",
        description: "single npm package → registry.npmjs.org",
        body: include_str!("../templates/npm-single.yaml"),
    },
    Example {
        name: "npm-workspace",
        description: "npm workspaces (`npm publish --workspaces`)",
        body: include_str!("../templates/npm-workspace.yaml"),
    },
    Example {
        name: "pnpm-workspace",
        description: "pnpm monorepo (`pnpm publish -r`)",
        body: include_str!("../templates/pnpm-workspace.yaml"),
    },
    Example {
        name: "uv-workspace",
        description: "uv / Python monorepo → PyPI",
        body: include_str!("../templates/uv-workspace.yaml"),
    },
    Example {
        name: "go",
        description: "Go module (tag-only)",
        body: include_str!("../templates/go.yaml"),
    },
    Example {
        name: "docker",
        description: "container image → OCI registry",
        body: include_str!("../templates/docker.yaml"),
    },
    Example {
        name: "multi-language",
        description: "Rust core + Node CLI, one tag",
        body: include_str!("../templates/multi-language.yaml"),
    },
    Example {
        name: "custom",
        description: "arbitrary publish command + state check",
        body: include_str!("../templates/custom.yaml"),
    },
];

pub fn find(name: &str) -> Option<&'static Example> {
    EXAMPLES.iter().find(|e| e.name == name)
}

pub fn list_formatted() -> String {
    let width = EXAMPLES.iter().map(|e| e.name.len()).max().unwrap_or(0);
    let mut out = String::from("Available examples:\n\n");
    for e in EXAMPLES {
        out.push_str(&format!("  {:<width$}  {}\n", e.name, e.description));
    }
    out
}

/// Pure decision for `sr init`: whether to skip, write, or error on an
/// unknown example name. Extracted so overwrite semantics (`--force`) and
/// the bundled-example whitelist are exercisable without touching disk.
#[derive(Debug, PartialEq, Eq)]
pub enum InitDecision {
    /// Config file already exists and `--force` was not set.
    Skip,
    /// Write this body to the config path.
    Write(String),
    /// `--example <name>` did not match any bundled template.
    UnknownExample(String),
}

pub fn decide_init<F>(
    example: Option<&str>,
    path_exists: bool,
    force: bool,
    default_template: F,
) -> InitDecision
where
    F: FnOnce() -> String,
{
    if path_exists && !force {
        return InitDecision::Skip;
    }
    match example {
        Some(name) => match find(name) {
            Some(e) => InitDecision::Write(e.body.to_string()),
            None => InitDecision::UnknownExample(name.to_string()),
        },
        None => InitDecision::Write(default_template()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_name_returns_none() {
        // Path-traversal safety: example lookup is a compile-time
        // whitelist, so user-supplied names can't escape.
        assert!(find("../../etc/passwd").is_none());
        assert!(find("").is_none());
        assert!(find("nonexistent").is_none());
    }

    #[test]
    fn known_names_resolve() {
        for e in EXAMPLES {
            assert!(find(e.name).is_some(), "missing example: {}", e.name);
        }
    }

    #[test]
    fn skips_when_exists_without_force() {
        let d = decide_init(None, true, false, || "DEFAULT".into());
        assert_eq!(d, InitDecision::Skip);
    }

    #[test]
    fn overwrites_when_force_set() {
        let d = decide_init(None, true, true, || "DEFAULT".into());
        assert_eq!(d, InitDecision::Write("DEFAULT".into()));
    }

    #[test]
    fn writes_default_when_absent() {
        let d = decide_init(None, false, false, || "DEFAULT".into());
        assert_eq!(d, InitDecision::Write("DEFAULT".into()));
    }

    #[test]
    fn writes_example_body_when_known() {
        let d = decide_init(Some("cargo-single"), false, false, || "DEFAULT".into());
        match d {
            InitDecision::Write(body) => assert!(!body.is_empty()),
            other => panic!("expected Write, got {other:?}"),
        }
    }

    #[test]
    fn unknown_example_reports_name() {
        let d = decide_init(Some("no-such-thing"), false, false, || "DEFAULT".into());
        assert_eq!(d, InitDecision::UnknownExample("no-such-thing".into()));
    }

    #[test]
    fn force_applies_to_example_path_too() {
        let d = decide_init(Some("cargo-single"), true, true, || "DEFAULT".into());
        assert!(matches!(d, InitDecision::Write(_)));
    }
}
