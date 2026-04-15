use serde::Serialize;

use crate::commit::ConventionalCommit;
use crate::config::ChangelogGroup;
use crate::error::ReleaseError;

/// A single changelog entry representing a release.
#[derive(Debug, Clone, Serialize)]
pub struct ChangelogEntry {
    pub version: String,
    pub date: String,
    pub commits: Vec<ConventionalCommit>,
    pub compare_url: Option<String>,
    pub repo_url: Option<String>,
}

/// A rendered group of commits for template context.
#[derive(Debug, Clone, Serialize)]
pub struct RenderedGroup {
    pub name: String,
    pub commits: Vec<ConventionalCommit>,
}

/// Formats changelog entries into a string representation.
pub trait ChangelogFormatter: Send + Sync {
    fn format(&self, entries: &[ChangelogEntry]) -> Result<String, ReleaseError>;
}

/// Default formatter using changelog groups.
/// When a custom template is provided, renders using minijinja.
pub struct DefaultChangelogFormatter {
    template: Option<String>,
    groups: Vec<ChangelogGroup>,
}

impl DefaultChangelogFormatter {
    pub fn new(template: Option<String>, groups: Vec<ChangelogGroup>) -> Self {
        Self { template, groups }
    }

    /// Resolve template: if it's a file path that exists, read it.
    /// Otherwise treat as inline template string.
    fn resolve_template(&self) -> Option<String> {
        let tmpl = self.template.as_ref()?;
        if std::path::Path::new(tmpl).exists() {
            std::fs::read_to_string(tmpl).ok()
        } else {
            Some(tmpl.clone())
        }
    }

    /// Build rendered groups from commits using configured groups.
    fn build_groups(&self, commits: &[ConventionalCommit]) -> Vec<RenderedGroup> {
        self.groups
            .iter()
            .map(|group| {
                let group_commits: Vec<_> = commits
                    .iter()
                    .filter(|c| {
                        if group.content.contains(&"breaking".to_string()) {
                            c.breaking
                        } else {
                            !c.breaking && group.content.contains(&c.r#type)
                        }
                    })
                    .cloned()
                    .collect();
                RenderedGroup {
                    name: group.name.clone(),
                    commits: group_commits,
                }
            })
            .collect()
    }
}

impl ChangelogFormatter for DefaultChangelogFormatter {
    fn format(&self, entries: &[ChangelogEntry]) -> Result<String, ReleaseError> {
        if let Some(template_str) = self.resolve_template() {
            // Build groups for each entry and pass to template.
            #[derive(Serialize)]
            struct TemplateEntry {
                version: String,
                date: String,
                groups: Vec<RenderedGroup>,
                compare_url: Option<String>,
                repo_url: Option<String>,
            }

            let template_entries: Vec<_> = entries
                .iter()
                .map(|e| TemplateEntry {
                    version: e.version.clone(),
                    date: e.date.clone(),
                    groups: self.build_groups(&e.commits),
                    compare_url: e.compare_url.clone(),
                    repo_url: e.repo_url.clone(),
                })
                .collect();

            let mut env = minijinja::Environment::new();
            env.add_template("changelog", &template_str)
                .map_err(|e| ReleaseError::Changelog(format!("invalid template: {e}")))?;
            let tmpl = env
                .get_template("changelog")
                .map_err(|e| ReleaseError::Changelog(format!("template error: {e}")))?;
            let output = tmpl
                .render(minijinja::context! { entries => template_entries })
                .map_err(|e| ReleaseError::Changelog(format!("template render error: {e}")))?;
            return Ok(output.trim_end().to_string());
        }

        // Built-in default format using groups.
        let mut output = String::new();

        for entry in entries {
            output.push_str(&format!("## {} ({})\n", entry.version, entry.date));

            let groups = self.build_groups(&entry.commits);
            for group in &groups {
                if group.commits.is_empty() {
                    continue;
                }
                // Convert group name to title case for heading.
                let title = group
                    .name
                    .split('-')
                    .map(|w| {
                        let mut c = w.chars();
                        match c.next() {
                            None => String::new(),
                            Some(f) => {
                                f.to_uppercase().to_string() + c.as_str()
                            }
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" ");

                output.push_str(&format!("\n### {title}\n\n"));
                for commit in &group.commits {
                    format_commit_line(&mut output, commit, entry.repo_url.as_deref());
                }
            }

            if let Some(url) = &entry.compare_url {
                output.push_str(&format!("\n[Full Changelog]({url})\n"));
            }

            output.push('\n');
        }

        Ok(output.trim_end().to_string())
    }
}

fn format_commit_line(output: &mut String, commit: &ConventionalCommit, repo_url: Option<&str>) {
    let short_sha = &commit.sha[..7.min(commit.sha.len())];
    let sha_display = match repo_url {
        Some(url) => format!("[{short_sha}]({url}/commit/{})", commit.sha),
        None => short_sha.to_string(),
    };
    if let Some(scope) = &commit.scope {
        output.push_str(&format!(
            "- **{scope}**: {} ({sha_display})\n",
            commit.description
        ));
    } else {
        output.push_str(&format!("- {} ({sha_display})\n", commit.description));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::default_changelog_groups;

    fn make_commit(
        type_: &str,
        desc: &str,
        scope: Option<&str>,
        breaking: bool,
    ) -> ConventionalCommit {
        ConventionalCommit {
            sha: "abc1234def5678".into(),
            r#type: type_.into(),
            scope: scope.map(Into::into),
            description: desc.into(),
            body: None,
            breaking,
        }
    }

    fn entry(commits: Vec<ConventionalCommit>, compare_url: Option<&str>) -> ChangelogEntry {
        ChangelogEntry {
            version: "1.0.0".into(),
            date: "2025-01-01".into(),
            commits,
            compare_url: compare_url.map(Into::into),
            repo_url: None,
        }
    }

    fn format(entries: &[ChangelogEntry]) -> String {
        DefaultChangelogFormatter::new(None, default_changelog_groups())
            .format(entries)
            .unwrap()
    }

    #[test]
    fn format_features_only() {
        let out = format(&[entry(
            vec![make_commit("feat", "add button", None, false)],
            None,
        )]);
        assert!(out.contains("## 1.0.0"));
        assert!(out.contains("### Features"));
        assert!(out.contains("add button"));
    }

    #[test]
    fn format_fixes_only() {
        let out = format(&[entry(
            vec![make_commit("fix", "null check", None, false)],
            None,
        )]);
        assert!(out.contains("### Bug Fixes"));
        assert!(out.contains("null check"));
    }

    #[test]
    fn format_breaking_changes() {
        let out = format(&[entry(
            vec![make_commit("feat", "new API", None, true)],
            None,
        )]);
        assert!(out.contains("### Breaking"));
    }

    #[test]
    fn format_mixed_commits() {
        let commits = vec![
            make_commit("feat", "add button", None, false),
            make_commit("fix", "null check", None, false),
            make_commit("feat", "breaking thing", None, true),
        ];
        let out = format(&[entry(commits, None)]);
        assert!(out.contains("### Features"));
        assert!(out.contains("### Bug Fixes"));
        assert!(out.contains("### Breaking"));
    }

    #[test]
    fn format_with_scope() {
        let out = format(&[entry(
            vec![make_commit("feat", "add flag", Some("cli"), false)],
            None,
        )]);
        assert!(out.contains("**cli**:"));
    }

    #[test]
    fn format_with_compare_url() {
        let out = format(&[entry(
            vec![make_commit("feat", "add button", None, false)],
            Some("https://github.com/o/r/compare/v0.1.0...v1.0.0"),
        )]);
        assert!(out.contains("[Full Changelog]"));
    }

    #[test]
    fn format_empty_entries() {
        let out = format(&[entry(vec![], None)]);
        assert!(!out.contains("### Features"));
        assert!(!out.contains("### Bug Fixes"));
    }

    #[test]
    fn format_with_commit_links() {
        let mut e = entry(vec![make_commit("feat", "add button", None, false)], None);
        e.repo_url = Some("https://github.com/o/r".into());
        let out = format(&[e]);
        assert!(out.contains("[abc1234](https://github.com/o/r/commit/abc1234def5678)"));
    }

    #[test]
    fn format_breaking_at_top() {
        let commits = vec![
            make_commit("feat", "add button", None, false),
            make_commit("feat", "breaking thing", None, true),
        ];
        let out = format(&[entry(commits, None)]);
        let breaking_pos = out.find("### Breaking").unwrap();
        let features_pos = out.find("### Features").unwrap();
        assert!(breaking_pos < features_pos);
    }

    #[test]
    fn format_misc_catch_all() {
        let commits = vec![
            make_commit("feat", "add button", None, false),
            make_commit("chore", "tidy up", None, false),
            make_commit("ci", "fix pipeline", None, false),
        ];
        let out = format(&[entry(commits, None)]);
        assert!(out.contains("### Misc"));
        assert!(out.contains("tidy up"));
        assert!(out.contains("fix pipeline"));
    }

    #[test]
    fn format_breaking_excluded_from_type_sections() {
        let commits = vec![
            make_commit("feat", "normal feature", None, false),
            make_commit("feat", "breaking feature", None, true),
        ];
        let out = format(&[entry(commits, None)]);
        let features_section_start = out.find("### Features").unwrap();
        let features_section_end = out[features_section_start..]
            .find("\n### ")
            .map(|p| features_section_start + p)
            .unwrap_or(out.len());
        let features_section = &out[features_section_start..features_section_end];
        assert!(features_section.contains("normal feature"));
        assert!(!features_section.contains("breaking feature"));
    }

    #[test]
    fn custom_template_renders() {
        let template = r#"{% for entry in entries %}Release {{ entry.version }}
{% for group in entry.groups %}{% if group.commits %}{{ group.name }}:
{% for c in group.commits %}- {{ c.description }}
{% endfor %}{% endif %}{% endfor %}{% endfor %}"#;
        let formatter =
            DefaultChangelogFormatter::new(Some(template.into()), default_changelog_groups());
        let out = formatter
            .format(&[entry(
                vec![
                    make_commit("feat", "add button", None, false),
                    make_commit("fix", "null check", None, false),
                ],
                None,
            )])
            .unwrap();
        assert!(out.contains("Release 1.0.0"));
        assert!(out.contains("- add button"));
        assert!(out.contains("- null check"));
        assert!(!out.contains("### Features"));
    }

    #[test]
    fn invalid_template_returns_error() {
        let template = "{% invalid %}";
        let formatter =
            DefaultChangelogFormatter::new(Some(template.into()), default_changelog_groups());
        let result = formatter.format(&[entry(vec![], None)]);
        assert!(result.is_err());
    }
}
