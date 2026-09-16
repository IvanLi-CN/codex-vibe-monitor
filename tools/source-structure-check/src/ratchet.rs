use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::checks;
use crate::model::{Baseline, CheckResult, Config, Diagnostic, RatchetReport, Report, State};

pub fn baseline(
    repo_root: &Path,
    config: &Config,
    staged: bool,
) -> Result<u8, Box<dyn std::error::Error>> {
    let state_path = repo_root.join(&config.state);
    if let Some(state) = read_state(&state_path)? {
        if state.mode == "zero" {
            return emit_error("zero mode is irreversible; baseline cannot be recreated");
        }
        return emit_error(format!(
            "unsupported source-quality state mode: {}",
            state.mode
        ));
    }
    let baseline_path = repo_root.join(&config.baseline);
    let result = checks::run(repo_root, config, staged)?;
    if !result.parse_errors.is_empty() {
        return emit_result(Report::new("parse-error", mode(staged), &result), 2);
    }

    let existing = if baseline_path.exists() {
        Some(read_baseline(&baseline_path, config)?)
    } else {
        None
    };
    let (violations, ratchet) = existing
        .as_ref()
        .map(|baseline| compare(&result, baseline))
        .unwrap_or_else(|| {
            (
                Vec::new(),
                RatchetReport {
                    new_diagnostics: 0,
                    metric_growth: 0,
                    removed_diagnostics: 0,
                },
            )
        });
    let mut report = Report::new("pass", mode(staged), &result);
    report.ratchet = Some(ratchet);
    if !violations.is_empty() {
        report.status = "fail".to_owned();
        report.add_ratchet_diagnostics(violations);
        return emit_result(report, 1);
    }

    let next = Baseline {
        schema_version: 1,
        mode: "ratchet".to_owned(),
        extensions: config.extensions.clone(),
        exclusions: config.exclusions.clone(),
        diagnostics: result.diagnostics.clone(),
    };
    write_json_atomic(&baseline_path, &next)?;
    report.baseline_written = true;
    emit_result(report, 0)
}

pub fn check(
    repo_root: &Path,
    config: &Config,
    staged: bool,
) -> Result<u8, Box<dyn std::error::Error>> {
    let result = checks::run(repo_root, config, staged)?;
    if !result.parse_errors.is_empty() {
        return emit_result(Report::new("parse-error", mode(staged), &result), 2);
    }
    let state_path = repo_root.join(&config.state);
    if let Some(state) = read_state(&state_path)? {
        if state.mode != "zero" {
            return emit_error(format!(
                "unsupported source-quality state mode: {}",
                state.mode
            ));
        }
        let baseline_path = repo_root.join(&config.baseline);
        if baseline_path.exists() {
            return emit_error("zero mode cannot have a baseline file");
        }
        let mut report = Report::new("pass", mode(staged), &result);
        if !result.diagnostics.is_empty() {
            report.status = "fail".to_owned();
            return emit_result(report, 1);
        }
        return emit_result(report, 0);
    }

    let baseline_path = repo_root.join(&config.baseline);
    let baseline = if baseline_path.exists() {
        read_baseline(&baseline_path, config)?
    } else {
        return emit_error(format!(
            "baseline is missing: run {} baseline first",
            config.baseline
        ));
    };
    let (violations, ratchet) = compare(&result, &baseline);
    let mut report = Report::new(
        if violations.is_empty() {
            "pass"
        } else {
            "fail"
        },
        mode(staged),
        &result,
    );
    report.ratchet = Some(ratchet);
    if !violations.is_empty() {
        report.add_ratchet_diagnostics(violations);
        emit_result(report, 1)
    } else {
        emit_result(report, 0)
    }
}

pub fn require_zero(
    repo_root: &Path,
    config: &Config,
    staged: bool,
) -> Result<u8, Box<dyn std::error::Error>> {
    let result = checks::run(repo_root, config, staged)?;
    if !result.parse_errors.is_empty() {
        return emit_result(Report::new("parse-error", mode(staged), &result), 2);
    }
    let state_path = repo_root.join(&config.state);
    if let Some(state) = read_state(&state_path)? {
        if state.mode != "zero" {
            return emit_error(format!(
                "unsupported source-quality state mode: {}",
                state.mode
            ));
        }
        if repo_root.join(&config.baseline).exists() {
            return emit_error("zero mode cannot have a baseline file");
        }
        let mut report = Report::new("pass", mode(staged), &result);
        if !result.diagnostics.is_empty() {
            report.status = "fail".to_owned();
            return emit_result(report, 1);
        }
        return emit_result(report, 0);
    }
    let mut report = Report::new("pass", mode(staged), &result);
    if !result.diagnostics.is_empty() {
        report.status = "fail".to_owned();
        return emit_result(report, 1);
    }
    let baseline_path = repo_root.join(&config.baseline);
    if baseline_path.exists() {
        let _ = read_baseline(&baseline_path, config)?;
        fs::remove_file(&baseline_path).map_err(|error| {
            format!(
                "cannot remove baseline during zero transition {}: {error}",
                baseline_path.display()
            )
        })?;
    }
    write_json_atomic(
        &state_path,
        &State {
            schema_version: 1,
            mode: "zero".to_owned(),
        },
    )?;
    report.zero_transitioned = true;
    emit_result(report, 0)
}

fn compare(result: &CheckResult, baseline: &Baseline) -> (Vec<Diagnostic>, RatchetReport) {
    let old = baseline
        .diagnostics
        .iter()
        .map(|diagnostic| (diagnostic.key(), diagnostic))
        .collect::<BTreeMap<_, _>>();
    let current = result
        .diagnostics
        .iter()
        .map(|diagnostic| (diagnostic.key(), diagnostic))
        .collect::<BTreeMap<_, _>>();
    let mut violations = Vec::new();
    let mut new_diagnostics = 0;
    let mut metric_growth = 0;
    for diagnostic in &result.diagnostics {
        match old.get(&diagnostic.key()) {
            None => {
                new_diagnostics += 1;
                violations.push(Diagnostic::new(
                    "ratchet/new-diagnostic",
                    &diagnostic.path,
                    diagnostic.key(),
                    diagnostic.actual,
                    0,
                ));
            }
            Some(previous) => {
                if diagnostic.limit != previous.limit {
                    metric_growth += 1;
                    violations.push(Diagnostic::new(
                        "ratchet/limit-change",
                        &diagnostic.path,
                        diagnostic.key(),
                        diagnostic.limit,
                        previous.limit,
                    ));
                } else if diagnostic.actual > previous.actual {
                    metric_growth += 1;
                    violations.push(Diagnostic::new(
                        "ratchet/metric-growth",
                        &diagnostic.path,
                        diagnostic.key(),
                        diagnostic.actual,
                        previous.actual,
                    ));
                }
            }
        }
    }
    let removed_diagnostics = old.keys().filter(|key| !current.contains_key(*key)).count();
    violations.sort_by(|left, right| {
        (&left.path, &left.rule, &left.subject).cmp(&(&right.path, &right.rule, &right.subject))
    });
    (
        violations,
        RatchetReport {
            new_diagnostics,
            metric_growth,
            removed_diagnostics,
        },
    )
}

fn read_baseline(path: &Path, config: &Config) -> Result<Baseline, Box<dyn std::error::Error>> {
    let bytes = fs::read(path)
        .map_err(|error| format!("cannot read baseline {}: {error}", path.display()))?;
    let baseline: Baseline = serde_json::from_slice(&bytes)
        .map_err(|error| format!("invalid baseline {}: {error}", path.display()))?;
    if baseline.schema_version != 1 || baseline.mode != "ratchet" {
        return Err(format!("baseline {} is not a ratchet baseline", path.display()).into());
    }
    if baseline.extensions != config.extensions || baseline.exclusions != config.exclusions {
        return Err(format!(
            "baseline {} has a source scope different from config",
            path.display()
        )
        .into());
    }
    let mut previous = None;
    for diagnostic in &baseline.diagnostics {
        if diagnostic.actual <= diagnostic.limit {
            return Err(format!(
                "baseline diagnostic is not a violation: {}",
                diagnostic.key()
            )
            .into());
        }
        if previous
            .as_ref()
            .is_some_and(|key: &String| key >= &diagnostic.key())
        {
            return Err("baseline diagnostics must be unique and sorted by identity".into());
        }
        previous = Some(diagnostic.key());
    }
    Ok(baseline)
}

fn read_state(path: &Path) -> Result<Option<State>, Box<dyn std::error::Error>> {
    if !path.exists() {
        return Ok(None);
    }
    let bytes =
        fs::read(path).map_err(|error| format!("cannot read state {}: {error}", path.display()))?;
    let state: State = serde_json::from_slice(&bytes)
        .map_err(|error| format!("invalid state {}: {error}", path.display()))?;
    if state.schema_version != 1 {
        return Err(format!(
            "unsupported source-quality state version {}",
            state.schema_version
        )
        .into());
    }
    Ok(Some(state))
}

fn write_json_atomic<T: serde::Serialize>(
    path: &Path,
    value: &T,
) -> Result<(), Box<dyn std::error::Error>> {
    let parent = path.parent().ok_or("state path has no parent")?;
    fs::create_dir_all(parent)?;
    let temporary = temporary_path(path);
    let contents = serde_json::to_vec_pretty(value)?;
    fs::write(&temporary, contents)?;
    fs::rename(&temporary, path).map_err(|error| {
        let _ = fs::remove_file(&temporary);
        format!("cannot atomically write {}: {error}", path.display())
    })?;
    Ok(())
}

fn temporary_path(path: &Path) -> PathBuf {
    let suffix = std::process::id();
    path.with_extension(format!("tmp-{suffix}.json"))
}

fn mode(staged: bool) -> &'static str {
    if staged { "staged" } else { "all" }
}

fn emit_error(message: impl Into<String>) -> Result<u8, Box<dyn std::error::Error>> {
    emit_result(Report::error(message), 2)
}

fn emit_result(report: Report, code: u8) -> Result<u8, Box<dyn std::error::Error>> {
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(code)
}
