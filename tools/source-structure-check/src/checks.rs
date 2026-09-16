use std::path::Path;

use crate::config::{self, FileClass};
use crate::model::{CheckResult, Config, Diagnostic};
use crate::parsers::{self, biome};
use crate::repository;

pub fn run(
    repo_root: &Path,
    config: &Config,
    staged: bool,
) -> Result<CheckResult, Box<dyn std::error::Error>> {
    let scope = repository::collect(repo_root, config, staged)?;
    let mut diagnostics = Vec::new();
    let mut parse_errors = Vec::new();

    let biome_files = scope.files.clone();
    match biome::check(repo_root, &biome_files, staged) {
        Ok((biome_diagnostics, biome_errors)) => {
            diagnostics.extend(biome_diagnostics);
            parse_errors.extend(biome_errors);
        }
        Err(error) => parse_errors.push(error),
    }

    for file in &scope.files {
        let (class, limit) = config::limit_for(config, &file.path);
        let lines = physical_lines(&file.content);
        if lines > limit {
            diagnostics.push(Diagnostic::new(
                file_rule(class),
                &file.path,
                "file",
                lines,
                limit,
            ));
        }
        match parsers::analyze(file, config, class) {
            Ok(analysis) => diagnostics.extend(analysis.diagnostics),
            Err(error) => parse_errors.push(error),
        }
    }
    diagnostics.sort_by(|left, right| {
        (&left.path, &left.rule, &left.subject).cmp(&(&right.path, &right.rule, &right.subject))
    });
    Ok(CheckResult {
        scope,
        diagnostics: crate::model::sorted_diagnostics(diagnostics),
        parse_errors,
    })
}

fn physical_lines(content: &str) -> u64 {
    if content.is_empty() {
        0
    } else {
        content.lines().count() as u64
    }
}

fn file_rule(class: FileClass) -> &'static str {
    match class {
        FileClass::Entry => "entry-file-lines",
        FileClass::Tool => "tool-file-lines",
        FileClass::TestStoryDemo => "test-file-lines",
        FileClass::Production => "production-file-lines",
    }
}
