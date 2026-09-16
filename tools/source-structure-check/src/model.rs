use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub schema_version: u32,
    pub extensions: Vec<String>,
    pub exclusions: Vec<String>,
    pub baseline: String,
    pub state: String,
    pub limits: Limits,
    pub entry_paths: Vec<String>,
    pub tool_roots: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    pub function_lines: u64,
    pub parameters: u64,
    pub nesting: u64,
    pub production_file_lines: u64,
    pub test_file_lines: u64,
    pub tool_file_lines: u64,
    pub entry_file_lines: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Diagnostic {
    pub rule: String,
    pub path: String,
    pub subject: String,
    pub actual: u64,
    pub limit: u64,
}

impl Diagnostic {
    pub fn new(
        rule: impl Into<String>,
        path: impl Into<String>,
        subject: impl Into<String>,
        actual: u64,
        limit: u64,
    ) -> Self {
        Self {
            rule: rule.into(),
            path: path.into(),
            subject: subject.into(),
            actual,
            limit,
        }
    }

    pub fn key(&self) -> String {
        format!("{}\u{1f}{}\u{1f}{}", self.rule, self.path, self.subject)
    }
}

#[derive(Clone, Debug)]
pub struct FunctionMetric {
    pub name: String,
    pub line: usize,
    pub lines: u64,
    pub parameters: u64,
    pub nesting: u64,
}

#[derive(Clone, Debug)]
pub struct SourceFile {
    pub path: String,
    pub content: String,
}

#[derive(Clone, Debug)]
pub struct ParseError {
    pub path: String,
    pub message: String,
}

#[derive(Clone, Debug)]
pub struct Scope {
    pub files: Vec<SourceFile>,
    pub declared_exclusions: Vec<String>,
    pub excluded_tracked: Vec<String>,
    pub tracked_source_count: usize,
}

#[derive(Clone, Debug)]
pub struct CheckResult {
    pub scope: Scope,
    pub diagnostics: Vec<Diagnostic>,
    pub parse_errors: Vec<ParseError>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Baseline {
    pub schema_version: u32,
    pub mode: String,
    pub extensions: Vec<String>,
    pub exclusions: Vec<String>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    pub schema_version: u32,
    pub mode: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct ScopeReport {
    pub tracked_source_count: usize,
    pub checked_source_count: usize,
    pub checked_paths: Vec<String>,
    pub declared_exclusions: Vec<String>,
    pub excluded_tracked_paths: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct RatchetReport {
    pub new_diagnostics: usize,
    pub metric_growth: usize,
    pub removed_diagnostics: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct Report {
    pub schema_version: u32,
    pub status: String,
    pub mode: String,
    pub scope: Option<ScopeReport>,
    pub diagnostics: Vec<Diagnostic>,
    pub parse_errors: Vec<ParseErrorReport>,
    pub errors: Vec<String>,
    pub ratchet: Option<RatchetReport>,
    pub baseline_written: bool,
    pub zero_transitioned: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct ParseErrorReport {
    pub path: String,
    pub message: String,
}

impl From<&ParseError> for ParseErrorReport {
    fn from(error: &ParseError) -> Self {
        Self {
            path: error.path.clone(),
            message: error.message.clone(),
        }
    }
}

impl Report {
    pub fn new(status: impl Into<String>, mode: impl Into<String>, result: &CheckResult) -> Self {
        let mut checked_paths = result
            .scope
            .files
            .iter()
            .map(|file| file.path.clone())
            .collect::<Vec<_>>();
        checked_paths.sort();
        Self {
            schema_version: 1,
            status: status.into(),
            mode: mode.into(),
            scope: Some(ScopeReport {
                tracked_source_count: result.scope.tracked_source_count,
                checked_source_count: result.scope.files.len(),
                checked_paths,
                declared_exclusions: result.scope.declared_exclusions.clone(),
                excluded_tracked_paths: result.scope.excluded_tracked.clone(),
            }),
            diagnostics: result.diagnostics.clone(),
            parse_errors: result
                .parse_errors
                .iter()
                .map(ParseErrorReport::from)
                .collect(),
            errors: Vec::new(),
            ratchet: None,
            baseline_written: false,
            zero_transitioned: false,
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            schema_version: 1,
            status: "error".to_owned(),
            mode: "unknown".to_owned(),
            scope: None,
            diagnostics: Vec::new(),
            parse_errors: Vec::new(),
            errors: vec![message.into()],
            ratchet: None,
            baseline_written: false,
            zero_transitioned: false,
        }
    }

    pub fn add_ratchet_diagnostics(&mut self, violations: Vec<Diagnostic>) {
        self.diagnostics.extend(violations);
        self.diagnostics.sort_by(|left, right| {
            (&left.path, &left.rule, &left.subject).cmp(&(&right.path, &right.rule, &right.subject))
        });
    }
}

pub fn sorted_diagnostics(diagnostics: Vec<Diagnostic>) -> Vec<Diagnostic> {
    let mut by_key = BTreeMap::<String, Diagnostic>::new();
    for diagnostic in diagnostics {
        let key = diagnostic.key();
        match by_key.get_mut(&key) {
            Some(existing) if diagnostic.actual > existing.actual => *existing = diagnostic,
            Some(_) => {}
            None => {
                by_key.insert(key, diagnostic);
            }
        }
    }
    by_key.into_values().collect()
}
