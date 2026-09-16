use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config;
use crate::model::{Config, Scope, SourceFile};

pub fn resolve_repo_root(requested: Option<&Path>) -> Result<PathBuf, Box<dyn std::error::Error>> {
    if let Some(path) = requested {
        let root = path
            .canonicalize()
            .map_err(|error| format!("cannot resolve repo root {}: {error}", path.display()))?;
        if !root.join(".git").exists() && !root.join(".git").is_file() {
            return Err(format!("repo root is not a Git worktree: {}", root.display()).into());
        }
        return Ok(root);
    }
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .map_err(|error| format!("cannot invoke git to find repo root: {error}"))?;
    if !output.status.success() {
        return Err("current directory is not inside a Git worktree".into());
    }
    let root = String::from_utf8(output.stdout)?.trim().to_owned();
    if root.is_empty() {
        return Err("git returned an empty repo root".into());
    }
    Ok(PathBuf::from(root).canonicalize()?)
}

pub fn collect(
    repo_root: &Path,
    config: &Config,
    staged: bool,
) -> Result<Scope, Box<dyn std::error::Error>> {
    let tracked = git_paths(repo_root, &["ls-files", "-z"])?;
    let tracked_source = tracked
        .iter()
        .filter(|path| config::extension(path).is_some())
        .filter(|path| !config::is_excluded(config, path))
        .cloned()
        .collect::<Vec<_>>();
    let excluded_tracked = tracked
        .iter()
        .filter(|path| config::is_excluded(config, path))
        .cloned()
        .collect::<Vec<_>>();

    let paths = if staged {
        let staged_paths = git_paths(
            repo_root,
            &[
                "diff",
                "--cached",
                "--name-only",
                "--diff-filter=ACMRTUXB",
                "-z",
            ],
        )?;
        staged_paths
            .into_iter()
            .filter(|path| config::extension(path).is_some())
            .filter(|path| !config::is_excluded(config, path))
            .collect::<Vec<_>>()
    } else {
        tracked_source.clone()
    };

    let mut files = Vec::with_capacity(paths.len());
    for path in paths {
        let bytes = if staged {
            git_show_index(repo_root, &path)?
        } else {
            let full_path = config::path_from_repo(repo_root, &path);
            let canonical_root = repo_root.canonicalize()?;
            let canonical_path = full_path.canonicalize().map_err(|error| {
                format!(
                    "tracked source path is unavailable {}: {error}",
                    full_path.display()
                )
            })?;
            if !canonical_path.starts_with(&canonical_root) {
                return Err(format!("tracked source path escapes repo root: {path}").into());
            }
            fs::read(&full_path)
                .map_err(|error| format!("cannot read tracked source {path}: {error}"))?
        };
        let content = String::from_utf8(bytes)
            .map_err(|error| format!("tracked source is not UTF-8 {path}: {error}"))?;
        files.push(SourceFile { path, content });
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(Scope {
        files,
        declared_exclusions: config.exclusions.clone(),
        excluded_tracked,
        tracked_source_count: tracked_source.len(),
    })
}

fn git_paths(repo_root: &Path, args: &[&str]) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(args)
        .output()
        .map_err(|error| format!("cannot invoke git {}: {error}", args.join(" ")))?;
    if !output.status.success() {
        return Err(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        )
        .into());
    }
    output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| {
            let path = std::str::from_utf8(path)?.to_owned();
            validate_git_path(&path)?;
            Ok(path)
        })
        .collect()
}

fn git_show_index(repo_root: &Path, path: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let index_path = format!(":{path}");
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(["show", &index_path])
        .output()
        .map_err(|error| format!("cannot read staged source {path}: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "cannot read staged source {path}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )
        .into());
    }
    Ok(output.stdout)
}

fn validate_git_path(path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let path = Path::new(path);
    if path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(format!("Git returned an unsafe tracked path: {path:?}").into());
    }
    Ok(())
}
