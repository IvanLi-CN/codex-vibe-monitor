use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::model::{Config, Limits};

const APPROVED_EXTENSIONS: [&str; 8] = [".rs", ".ts", ".tsx", ".js", ".jsx", ".css", ".py", ".sh"];
const GENERATED_WORKER: &str = "web/public/mockServiceWorker.js";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileClass {
    Production,
    TestStoryDemo,
    Tool,
    Entry,
}

pub fn load(repo_root: &Path) -> Result<Config, Box<dyn std::error::Error>> {
    let config_path = repo_root.join(".source-quality/config.json");
    let bytes = fs::read(&config_path).map_err(|error| {
        format!(
            "cannot read source-quality configuration {}: {error}",
            config_path.display()
        )
    })?;
    let config: Config = serde_json::from_slice(&bytes).map_err(|error| {
        format!(
            "invalid source-quality configuration {}: {error}",
            config_path.display()
        )
    })?;
    validate(&config)?;
    Ok(config)
}

fn validate(config: &Config) -> Result<(), Box<dyn std::error::Error>> {
    if config.schema_version != 1 {
        return Err(format!(
            "unsupported source-quality schema version {}",
            config.schema_version
        )
        .into());
    }
    if config.extensions != APPROVED_EXTENSIONS {
        return Err(format!(
            "source-quality extensions must be exactly {:?}",
            APPROVED_EXTENSIONS
        )
        .into());
    }
    if config.exclusions != [GENERATED_WORKER.to_owned()] {
        return Err(
            format!("source-quality exclusions must be exactly [{GENERATED_WORKER}]").into(),
        );
    }
    for path in config
        .exclusions
        .iter()
        .chain(config.entry_paths.iter())
        .chain(config.tool_roots.iter())
    {
        validate_relative_path(path)?;
    }
    if config.baseline != ".source-quality/baseline.json" {
        return Err("baseline path must be .source-quality/baseline.json".into());
    }
    if config.state != ".source-quality/state.json" {
        return Err("state path must be .source-quality/state.json".into());
    }
    validate_limits(&config.limits)?;
    if config.entry_paths.is_empty() || config.tool_roots.is_empty() {
        return Err("entry_paths and tool_roots must not be empty".into());
    }
    Ok(())
}

fn validate_limits(limits: &Limits) -> Result<(), Box<dyn std::error::Error>> {
    let expected = [
        ("function_lines", limits.function_lines, 100),
        ("parameters", limits.parameters, 7),
        ("nesting", limits.nesting, 4),
        ("production_file_lines", limits.production_file_lines, 1000),
        ("test_file_lines", limits.test_file_lines, 2000),
        ("tool_file_lines", limits.tool_file_lines, 500),
        ("entry_file_lines", limits.entry_file_lines, 500),
    ];
    for (name, actual, required) in expected {
        if actual != required {
            return Err(
                format!("source-quality limit {name} must be {required}, got {actual}").into(),
            );
        }
    }
    Ok(())
}

fn validate_relative_path(path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let path = Path::new(path);
    if path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(format!("source-quality path must stay relative: {path:?}").into());
    }
    Ok(())
}

pub fn extension(path: &str) -> Option<&'static str> {
    APPROVED_EXTENSIONS
        .iter()
        .copied()
        .find(|suffix| path.ends_with(suffix))
}

pub fn is_excluded(config: &Config, path: &str) -> bool {
    config.exclusions.iter().any(|excluded| excluded == path)
}

pub fn classify(config: &Config, path: &str) -> FileClass {
    if config
        .entry_paths
        .iter()
        .any(|entry| matches_glob(entry, path))
    {
        return FileClass::Entry;
    }
    if config
        .tool_roots
        .iter()
        .any(|root| path == root || path.starts_with(&format!("{root}/")))
    {
        return FileClass::Tool;
    }
    if is_test_story_demo(path) {
        return FileClass::TestStoryDemo;
    }
    FileClass::Production
}

fn matches_glob(pattern: &str, path: &str) -> bool {
    match pattern.strip_suffix("/**") {
        Some(prefix) => path == prefix || path.starts_with(&format!("{prefix}/")),
        None => pattern == path,
    }
}

fn is_test_story_demo(path: &str) -> bool {
    let file_name = Path::new(path)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    let components = path.split('/').collect::<BTreeSet<_>>();
    components.contains("tests")
        || components.contains("test")
        || components.contains("demo")
        || file_name.contains(".test.")
        || file_name.contains(".spec.")
        || file_name.contains(".stories.")
        || file_name.ends_with("_test.rs")
}

pub fn limit_for(config: &Config, path: &str) -> (FileClass, u64) {
    let class = classify(config, path);
    let limit = match class {
        FileClass::Entry => config.limits.entry_file_lines,
        FileClass::Tool => config.limits.tool_file_lines,
        FileClass::TestStoryDemo => config.limits.test_file_lines,
        FileClass::Production => config.limits.production_file_lines,
    };
    (class, limit)
}

pub fn path_from_repo(repo_root: &Path, path: &str) -> PathBuf {
    repo_root.join(path)
}
