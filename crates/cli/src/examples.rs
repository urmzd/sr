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
        body: include_str!("../../../examples/cargo-single.yaml"),
    },
    Example {
        name: "cargo-workspace",
        description: "Rust workspace (every member at shared version)",
        body: include_str!("../../../examples/cargo-workspace.yaml"),
    },
    Example {
        name: "npm-single",
        description: "single npm package → registry.npmjs.org",
        body: include_str!("../../../examples/npm-single.yaml"),
    },
    Example {
        name: "npm-workspace",
        description: "npm workspaces (`npm publish --workspaces`)",
        body: include_str!("../../../examples/npm-workspace.yaml"),
    },
    Example {
        name: "pnpm-workspace",
        description: "pnpm monorepo (`pnpm publish -r`)",
        body: include_str!("../../../examples/pnpm-workspace.yaml"),
    },
    Example {
        name: "uv-workspace",
        description: "uv / Python monorepo → PyPI",
        body: include_str!("../../../examples/uv-workspace.yaml"),
    },
    Example {
        name: "go",
        description: "Go module (tag-only)",
        body: include_str!("../../../examples/go.yaml"),
    },
    Example {
        name: "docker",
        description: "container image → OCI registry",
        body: include_str!("../../../examples/docker.yaml"),
    },
    Example {
        name: "multi-language",
        description: "Rust core + Node CLI, one tag",
        body: include_str!("../../../examples/multi-language.yaml"),
    },
    Example {
        name: "custom",
        description: "arbitrary publish command + state check",
        body: include_str!("../../../examples/custom.yaml"),
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
