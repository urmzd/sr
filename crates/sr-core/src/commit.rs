use regex::Regex;
use serde::Serialize;

use crate::error::ReleaseError;
use crate::version::BumpLevel;

/// A raw commit as read from git history.
#[derive(Debug, Clone)]
pub struct Commit {
    pub sha: String,
    pub message: String,
}

/// A commit parsed according to the Conventional Commits specification.
#[derive(Debug, Clone, Serialize)]
pub struct ConventionalCommit {
    pub sha: String,
    pub r#type: String,
    pub scope: Option<String>,
    pub description: String,
    pub body: Option<String>,
    pub breaking: bool,
}

/// Internal commit type representation — name + bump level.
#[derive(Debug, Clone, PartialEq)]
pub struct CommitType {
    pub name: String,
    pub bump: Option<BumpLevel>,
}

/// Build the conventional commit regex from configured type names.
///
/// Produces: `^(?P<type>feat|fix|...)(?:\((?P<scope>[^)]+)\))?(?P<breaking>!)?:\s+(?P<description>.+)`
pub fn build_commit_pattern(type_names: &[&str]) -> String {
    let types_alternation = type_names.join("|");
    format!(
        r"^(?P<type>{types_alternation})(?:\((?P<scope>[^)]+)\))?(?P<breaking>!)?:\s+(?P<description>.+)"
    )
}

/// Single source of truth for commit type classification.
pub trait CommitClassifier: Send + Sync {
    fn types(&self) -> &[CommitType];
    fn pattern(&self) -> &str;

    fn bump_level(&self, type_name: &str, breaking: bool) -> Option<BumpLevel> {
        if breaking {
            return Some(BumpLevel::Major);
        }
        self.types().iter().find(|t| t.name == type_name)?.bump
    }

    fn is_allowed(&self, type_name: &str) -> bool {
        self.types().iter().any(|t| t.name == type_name)
    }
}

pub struct DefaultCommitClassifier {
    types: Vec<CommitType>,
    pattern: String,
}

impl DefaultCommitClassifier {
    pub fn new(types: Vec<CommitType>) -> Self {
        let names: Vec<&str> = types.iter().map(|t| t.name.as_str()).collect();
        let pattern = build_commit_pattern(&names);
        Self { types, pattern }
    }
}

impl Default for DefaultCommitClassifier {
    fn default() -> Self {
        Self::new(default_commit_types())
    }
}

impl CommitClassifier for DefaultCommitClassifier {
    fn types(&self) -> &[CommitType] {
        &self.types
    }
    fn pattern(&self) -> &str {
        &self.pattern
    }
}

pub fn default_commit_types() -> Vec<CommitType> {
    use crate::config::CommitTypesConfig;
    CommitTypesConfig::default().into_commit_types()
}

/// Parses raw commits into conventional commits.
pub trait CommitParser: Send + Sync {
    fn parse(&self, commit: &Commit) -> Result<ConventionalCommit, ReleaseError>;
}

/// Parser that builds its pattern from configured type names.
pub struct TypedCommitParser {
    pattern: String,
}

impl TypedCommitParser {
    pub fn new(type_names: &[&str]) -> Self {
        Self {
            pattern: build_commit_pattern(type_names),
        }
    }

    pub fn from_types(types: &[CommitType]) -> Self {
        let names: Vec<&str> = types.iter().map(|t| t.name.as_str()).collect();
        Self::new(&names)
    }
}

impl Default for TypedCommitParser {
    fn default() -> Self {
        Self::from_types(&default_commit_types())
    }
}

impl CommitParser for TypedCommitParser {
    fn parse(&self, commit: &Commit) -> Result<ConventionalCommit, ReleaseError> {
        let re = Regex::new(&self.pattern).map_err(|e| ReleaseError::Config(e.to_string()))?;

        let first_line = commit.message.lines().next().unwrap_or("");

        let caps = re.captures(first_line).ok_or_else(|| {
            ReleaseError::Config(format!("not a conventional commit: {}", commit.message))
        })?;

        let r#type = caps.name("type").unwrap().as_str().to_string();
        let scope = caps.name("scope").map(|m| m.as_str().to_string());
        let breaking = caps.name("breaking").is_some();
        let description = caps.name("description").unwrap().as_str().to_string();

        let body = commit
            .message
            .split_once("\n\n")
            .map(|x| x.1)
            .map(|b| b.to_string());

        // Detect BREAKING CHANGE: / BREAKING-CHANGE: footers in the body.
        let breaking = breaking
            || body.as_deref().is_some_and(|b| {
                b.lines().any(|line| {
                    line.starts_with("BREAKING CHANGE:") || line.starts_with("BREAKING-CHANGE:")
                })
            });

        Ok(ConventionalCommit {
            sha: commit.sha.clone(),
            r#type,
            scope,
            description,
            body,
            breaking,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(message: &str) -> Commit {
        Commit {
            sha: "abc1234".into(),
            message: message.into(),
        }
    }

    fn parser() -> TypedCommitParser {
        TypedCommitParser::default()
    }

    #[test]
    fn build_pattern_from_types() {
        let pattern = build_commit_pattern(&["feat", "fix", "chore"]);
        assert!(pattern.contains("feat|fix|chore"));
        let re = Regex::new(&pattern).unwrap();
        assert!(re.is_match("feat: add button"));
        assert!(re.is_match("fix(core): null check"));
        assert!(!re.is_match("unknown: something"));
    }

    #[test]
    fn parse_simple_feat() {
        let result = parser().parse(&raw("feat: add button")).unwrap();
        assert_eq!(result.r#type, "feat");
        assert_eq!(result.description, "add button");
        assert_eq!(result.scope, None);
        assert!(!result.breaking);
    }

    #[test]
    fn parse_scoped_fix() {
        let result = parser().parse(&raw("fix(core): null check")).unwrap();
        assert_eq!(result.r#type, "fix");
        assert_eq!(result.scope.as_deref(), Some("core"));
    }

    #[test]
    fn parse_breaking_bang() {
        let result = parser().parse(&raw("feat!: new API")).unwrap();
        assert!(result.breaking);
    }

    #[test]
    fn parse_with_body() {
        let result = parser().parse(&raw("fix: x\n\ndetails")).unwrap();
        assert_eq!(result.body.as_deref(), Some("details"));
    }

    #[test]
    fn parse_breaking_change_footer() {
        let result = parser()
            .parse(&raw(
                "feat: new API\n\nBREAKING CHANGE: removed old endpoint",
            ))
            .unwrap();
        assert!(result.breaking);
    }

    #[test]
    fn parse_breaking_change_hyphenated_footer() {
        let result = parser()
            .parse(&raw("fix: update schema\n\nBREAKING-CHANGE: field renamed"))
            .unwrap();
        assert!(result.breaking);
    }

    #[test]
    fn parse_no_breaking_change_in_body() {
        let result = parser()
            .parse(&raw("fix: tweak\n\nThis is not a BREAKING CHANGE footer"))
            .unwrap();
        assert!(!result.breaking);
    }

    #[test]
    fn parse_no_breaking_change_indented_bullet() {
        let result = parser()
            .parse(&raw(
                "feat(mcp): add breaking flag\n\n- add `breaking` field — sets \"!\" and adds\n  BREAKING CHANGE footer automatically",
            ))
            .unwrap();
        assert!(!result.breaking);
    }

    #[test]
    fn parse_invalid_message() {
        let result = parser().parse(&raw("not conventional"));
        assert!(result.is_err());
    }

    #[test]
    fn parse_unknown_type_rejected() {
        let result = parser().parse(&raw("unknown: something"));
        assert!(result.is_err());
    }

    // --- CommitClassifier tests ---

    #[test]
    fn classifier_bump_level_feat() {
        let c = DefaultCommitClassifier::default();
        assert_eq!(c.bump_level("feat", false), Some(BumpLevel::Minor));
    }

    #[test]
    fn classifier_bump_level_fix() {
        let c = DefaultCommitClassifier::default();
        assert_eq!(c.bump_level("fix", false), Some(BumpLevel::Patch));
    }

    #[test]
    fn classifier_bump_level_breaking_overrides() {
        let c = DefaultCommitClassifier::default();
        assert_eq!(c.bump_level("fix", true), Some(BumpLevel::Major));
        assert_eq!(c.bump_level("chore", true), Some(BumpLevel::Major));
    }

    #[test]
    fn classifier_bump_level_no_bump_type() {
        let c = DefaultCommitClassifier::default();
        assert_eq!(c.bump_level("chore", false), None);
        assert_eq!(c.bump_level("docs", false), None);
    }

    #[test]
    fn classifier_is_allowed() {
        let c = DefaultCommitClassifier::default();
        assert!(c.is_allowed("feat"));
        assert!(c.is_allowed("chore"));
        assert!(!c.is_allowed("unknown"));
    }

    #[test]
    fn classifier_pattern_built_from_types() {
        let c = DefaultCommitClassifier::default();
        assert!(c.pattern().contains("feat"));
        assert!(c.pattern().contains("fix"));
        assert!(c.pattern().contains("chore"));
    }

    #[test]
    fn default_commit_types_count() {
        let types = default_commit_types();
        assert_eq!(types.len(), 11);
    }
}
