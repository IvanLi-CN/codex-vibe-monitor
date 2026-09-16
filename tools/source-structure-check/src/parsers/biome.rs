use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

use crate::model::{Diagnostic, ParseError, SourceFile};

pub fn check(
    repo_root: &Path,
    files: &[SourceFile],
    staged: bool,
) -> Result<(Vec<Diagnostic>, Vec<ParseError>), ParseError> {
    let paths = files
        .iter()
        .filter(|file| {
            matches!(
                file.path.rsplit_once('.').map(|(_, ext)| ext),
                Some("ts" | "tsx" | "js" | "jsx" | "css")
            )
        })
        .map(|file| file.path.as_str())
        .collect::<Vec<_>>();
    if paths.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    let binary = find_biome(repo_root).ok_or_else(|| ParseError {
        path: "<biome>".to_owned(),
        message: "Biome executable is required for tracked TypeScript, JavaScript, JSX, TSX, and CSS sources".to_owned(),
    })?;
    let staged_workspace = if staged {
        Some(StagedBiomeWorkspace::create(repo_root, files)?)
    } else {
        None
    };
    let working_directory = staged_workspace
        .as_ref()
        .map(|workspace| workspace.root.as_path())
        .unwrap_or(repo_root);
    let output = Command::new(&binary)
        .current_dir(working_directory)
        .arg("check")
        .arg("--reporter=json")
        .arg("--colors=off")
        .arg("--max-diagnostics=none")
        .arg("--no-errors-on-unmatched")
        .arg("--vcs-use-ignore-file=false")
        .args(paths.iter().copied())
        .output()
        .map_err(|error| ParseError {
            path: "<biome>".to_owned(),
            message: format!("cannot execute Biome {}: {error}", binary.display()),
        })?;
    let report: Value = serde_json::from_slice(&output.stdout).map_err(|error| ParseError {
        path: "<biome>".to_owned(),
        message: format!("Biome JSON diagnostics could not be decoded: {error}"),
    })?;
    let diagnostics = report
        .get("diagnostics")
        .and_then(Value::as_array)
        .ok_or_else(|| ParseError {
            path: "<biome>".to_owned(),
            message: "Biome JSON report did not contain diagnostics".to_owned(),
        })?;

    let mut structure_diagnostics = Vec::new();
    let mut parse_errors = Vec::new();
    for diagnostic in diagnostics {
        let category = diagnostic
            .get("category")
            .and_then(Value::as_str)
            .unwrap_or("diagnostic");
        let message = diagnostic
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("Biome diagnostic")
            .to_owned();
        let (path, line, column) = location(diagnostic);
        if category.contains("parse")
            || category.contains("syntax")
            || category.contains("internalError")
        {
            parse_errors.push(ParseError {
                path,
                message: format!("Biome {category}: {message}"),
            });
            continue;
        }
        structure_diagnostics.push(Diagnostic::new(
            format!("biome/{category}"),
            path,
            format!("{line}:{column}:{message}"),
            1,
            0,
        ));
    }
    if !output.status.success() && parse_errors.is_empty() && structure_diagnostics.is_empty() {
        return Err(ParseError {
            path: "<biome>".to_owned(),
            message: format!(
                "Biome exited with status {} without a diagnostic report",
                output.status
            ),
        });
    }
    Ok((structure_diagnostics, parse_errors))
}

struct StagedBiomeWorkspace {
    root: PathBuf,
}

impl StagedBiomeWorkspace {
    fn create(repo_root: &Path, files: &[SourceFile]) -> Result<Self, ParseError> {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        let root = std::env::temp_dir().join(format!(
            "source-quality-biome-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&root).map_err(|error| {
            biome_error(format!(
                "cannot create staged Biome workspace {}: {error}",
                root.display()
            ))
        })?;
        let workspace = Self { root };
        workspace.copy_configuration(repo_root)?;
        for file in files {
            let relative = Path::new(&file.path);
            if relative.is_absolute()
                || relative
                    .components()
                    .any(|component| matches!(component, std::path::Component::ParentDir))
            {
                return Err(biome_error(format!(
                    "staged source path escapes the temporary Biome workspace: {}",
                    file.path
                )));
            }
            let destination = workspace.root.join(relative);
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent).map_err(|error| {
                    biome_error(format!(
                        "cannot create staged Biome parent {}: {error}",
                        parent.display()
                    ))
                })?;
            }
            fs::write(&destination, file.content.as_bytes()).map_err(|error| {
                biome_error(format!(
                    "cannot write staged Biome source {}: {error}",
                    destination.display()
                ))
            })?;
        }
        Ok(workspace)
    }

    fn copy_configuration(&self, repo_root: &Path) -> Result<(), ParseError> {
        for name in ["biome.json", "biome.jsonc"] {
            let source = repo_root.join(name);
            if source.is_file() {
                fs::copy(&source, self.root.join(name)).map_err(|error| {
                    biome_error(format!(
                        "cannot copy Biome configuration {}: {error}",
                        source.display()
                    ))
                })?;
                break;
            }
        }
        Ok(())
    }
}

impl Drop for StagedBiomeWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn biome_error(message: impl Into<String>) -> ParseError {
    ParseError {
        path: "<biome>".to_owned(),
        message: message.into(),
    }
}

fn location(diagnostic: &Value) -> (String, usize, usize) {
    let path = diagnostic
        .pointer("/location/path")
        .and_then(Value::as_str)
        .map(normalize_path)
        .unwrap_or_else(|| "<biome>".to_owned());
    let line = diagnostic
        .pointer("/location/start/line")
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize
        + 1;
    let column = diagnostic
        .pointer("/location/start/column")
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize
        + 1;
    (path, line, column)
}

fn normalize_path(path: &str) -> String {
    let path = path.replace('\\', "/");
    if let Some(index) = path.find("/web/") {
        return path[index + 1..].to_owned();
    }
    if let Some(index) = path.find("/docs-site/") {
        return path[index + 1..].to_owned();
    }
    path
}

fn find_biome(repo_root: &Path) -> Option<std::path::PathBuf> {
    if let Some(path) = std::env::var_os("SOURCE_QUALITY_BIOME_BIN") {
        let path = std::path::PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
    }
    for candidate in [
        repo_root.join("node_modules/.bin/biome"),
        repo_root.join("web/node_modules/.bin/biome"),
        repo_root.join("docs-site/node_modules/.bin/biome"),
    ] {
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    let path = std::env::var_os("PATH")?;
    for directory in std::env::split_paths(&path) {
        let candidate = directory.join("biome");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}
