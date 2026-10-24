#!/usr/bin/env python3
"""Reproducibly compare frontend test stages between two source trees.

The runner deliberately executes every sample in a fresh process. A failed
sample is retained in JSONL for diagnosis but is excluded from the summary.
Warm-cache paired samples are the acceptance population; cold samples are
descriptive only.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import resource
import shutil
import statistics
import subprocess
import sys
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Iterable


MAX_SAMPLE_SECONDS = 30 * 60


STAGES: dict[str, tuple[str, ...]] = {
    "demo": ("bun", "run", "test", "--", "--project=unit", "src/demo"),
    "unit": ("bun", "run", "test"),
    "functional-e2e": (
        "bun",
        "run",
        "test:e2e",
        "--",
        "--config=playwright.ci.config.ts",
        "--project=fixtures-functional",
        "--project=demo-functional",
        "--retries=0",
    ),
    "frontend": (
        "bash",
        "-lc",
        "set -euo pipefail; "
        "bun run test; "
        "bun run build:pwa-test; "
        "bun run test:pwa-assets; "
        "bun run test-storybook; "
        "bun run storybook:build -- --quiet; "
        "cd ../docs-site; DOCS_BASE=/ bun run build; "
        "cd ../web; VITE_DEPLOY_BASE=/demo/ bun run demo:build; "
        "cd ..; bash ./.github/scripts/assemble-pages-site.sh docs-site/doc_build web/storybook-static web/demo-dist .tmp/pages-site; "
        "test -f .tmp/pages-site/index.html; "
        "test -f .tmp/pages-site/storybook/index.html; "
        "test -f .tmp/pages-site/demo/index.html; "
        "test -f .tmp/pages-site/demo/mockServiceWorker.js; "
        "test -f .tmp/pages-site/storybook.html; "
        "grep -q '/storybook.html' .tmp/pages-site/index.html; "
        "cd web; "
        "bun run dev -- --host 127.0.0.1 --port 60080 >/tmp/cvm-benchmark-vite.log 2>&1 & "
        "fixture_pid=$!; "
        "VITE_APP_RUNTIME=demo VITE_BUILD_OUT_DIR=demo-dist bun run demo:preview -- --host 127.0.0.1 --port 60083 >/tmp/cvm-benchmark-demo.log 2>&1 & "
        "demo_pid=$!; "
        "trap 'kill $fixture_pid $demo_pid 2>/dev/null || true' EXIT; "
        "for endpoint in 60080 60083; do "
        "  ready=0; for attempt in $(seq 1 60); do if curl -fsS http://127.0.0.1:$endpoint/ >/dev/null; then ready=1; break; fi; sleep 1; done; "
        "  if [ \"$ready\" -ne 1 ]; then echo \"server on $endpoint did not become ready\" >&2; exit 1; fi; "
        "done; "
        "CI_PROFILE=${CVM_BENCHMARK_PROFILE} E2E_BASE_URL=http://127.0.0.1:60080 E2E_DEMO_BASE_URL=http://127.0.0.1:60083 "
        "bun run test:e2e -- --config=playwright.ci.config.ts --project=fixtures-functional --project=demo-functional --workers=$([ \"${CVM_BENCHMARK_PROFILE}\" = main ] && echo 2 || echo 1) --retries=0; "
        "CI_PROFILE=${CVM_BENCHMARK_PROFILE} E2E_DEMO_BASE_URL=http://127.0.0.1:60083 E2E_PRODUCTION_BUILD=1 "
        "bun run test:e2e -- --config=playwright.ci.config.ts --project=performance --workers=1 --retries=0; "
        "if [ \"${CVM_BENCHMARK_PROFILE}\" = main ]; then "
        "  E2E_DEMO_BASE_URL=http://127.0.0.1:60083 E2E_PRODUCTION_BUILD=1 E2E_TIMELINE_CAPACITY=1 E2E_TIMELINE_CAPACITY_DURATION_MS=60000 "
        "  bun run test:e2e -- --config=playwright.ci.config.ts --project=timeline-capacity --workers=1 --retries=0; "
        "fi",
    ),
}


@dataclass(frozen=True)
class Sample:
    version: str
    stage: str
    profile: str
    pair: int | None
    phase: str
    order: int
    command: tuple[str, ...]
    source: str
    source_sha: str
    wall_seconds: float
    cpu_seconds: float
    peak_rss_bytes: int
    returncode: int
    retry_count: int
    valid: bool
    log: str

    def as_json(self) -> dict[str, Any]:
        return {
            "version": self.version,
            "stage": self.stage,
            "profile": self.profile,
            "pair": self.pair,
            "phase": self.phase,
            "order": self.order,
            "command": list(self.command),
            "source": self.source,
            "source_sha": self.source_sha,
            "wall_seconds": self.wall_seconds,
            "cpu_seconds": self.cpu_seconds,
            "peak_rss_bytes": self.peak_rss_bytes,
            "returncode": self.returncode,
            "retry_count": self.retry_count,
            "valid": self.valid,
            "log": self.log,
        }


def median_absolute_deviation(values: Iterable[float]) -> float:
    values = list(values)
    if not values:
        return float("nan")
    center = statistics.median(values)
    return statistics.median(abs(value - center) for value in values)


def summarize(values: list[float]) -> dict[str, float | int | None]:
    if not values:
        return {"count": 0, "median_seconds": None, "mad_seconds": None}
    return {
        "count": len(values),
        "median_seconds": statistics.median(values),
        "mad_seconds": median_absolute_deviation(values),
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline-src", type=Path, required=True)
    parser.add_argument("--candidate-src", type=Path, required=True)
    parser.add_argument("--baseline-sha", required=True)
    parser.add_argument("--candidate-sha", required=True)
    parser.add_argument("--stage", choices=sorted(STAGES), required=True)
    parser.add_argument("--profile", choices=("pr", "main"), required=True)
    parser.add_argument("--rounds", type=int, default=7)
    parser.add_argument("--output", type=Path, required=True)
    return parser.parse_args()


def source_web_dir(source: Path) -> Path:
    web = source / "web"
    if not web.is_dir():
        raise ValueError(f"source does not contain web/: {source}")
    return web


def git_head(source: Path) -> str:
    completed = subprocess.run(
        ("git", "-C", str(source), "rev-parse", "HEAD"),
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if completed.returncode != 0:
        raise SystemExit(f"source is not a Git checkout: {source}")
    return completed.stdout.strip()


def lockfile_digest(source: Path) -> str:
    digest = hashlib.sha256()
    for path in (source / "bun.lock", source / "web" / "bun.lock"):
        if not path.is_file():
            raise SystemExit(f"missing lockfile: {path}")
        digest.update(path.name.encode())
        digest.update(path.read_bytes())
    return digest.hexdigest()


def command_version(command: tuple[str, ...], cwd: Path) -> str:
    try:
        completed = subprocess.run(
            command,
            cwd=cwd,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            check=False,
            timeout=30,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        return f"unavailable: {error}"
    output = completed.stdout.strip()
    return output if completed.returncode == 0 and output else f"unavailable: exit {completed.returncode}"


def environment_snapshot(source: Path) -> dict[str, Any]:
    web = source_web_dir(source)
    bun_version = command_version(("bun", "--version"), web)
    expected_bun = (source / ".bun-version").read_text(encoding="utf-8").strip()
    return {
        "bun": bun_version,
        "bun_expected": expected_bun,
        "bun_valid": bun_version == expected_bun,
        "playwright": command_version(("bunx", "playwright", "--version"), web),
        "uname": command_version(("uname", "-a"), source),
        "meminfo": command_version(("sh", "-c", "awk '/MemTotal/ { print $2, $3 }' /proc/meminfo"), source),
    }


def retry_count(output: str) -> int:
    matches = re.findall(r"\b(\d+)\s+(?:retry|retries)\b", output, flags=re.IGNORECASE)
    matches.extend(re.findall(r"\bretry\s*#\s*(\d+)\b", output, flags=re.IGNORECASE))
    return max((int(value) for value in matches), default=0)


def measurement_value(output: str, *keys: str) -> float | None:
    values: list[float] = []
    for key in keys:
        match = re.search(rf"{re.escape(key)}=([0-9]+(?:\.[0-9]+)?)", output)
        if match:
            values.append(float(match.group(1)))
    return sum(values) if values else None


def legacy_playwright_schedule(profile: str) -> str:
    if profile == "pr":
        return (
            "set -euo pipefail; "
            "records_status=0; demo_status=0; dashboard_status=0; performance_status=0; retention_status=0; "
            "PLAYWRIGHT_HTML_OUTPUT_DIR=playwright-report-records bun run test:e2e -- --workers=1 --retries=0 --output=test-results/records-overlay records-filter-overlay.spec.ts || records_status=$?; "
            "E2E_BASE_URL=http://127.0.0.1:60083 PLAYWRIGHT_HTML_OUTPUT_DIR=playwright-report-demo bun run test:e2e -- --workers=1 --retries=0 --output=test-results/demo-runtime demo-runtime.spec.ts || demo_status=$?; "
            "E2E_BASE_URL=http://127.0.0.1:60083 PLAYWRIGHT_HTML_OUTPUT_DIR=playwright-report-retention-ownership bun run test:e2e -- --workers=1 --retries=0 --output=test-results/retention-ownership retention-maintenance-ownership.spec.ts || retention_status=$?; "
            "PLAYWRIGHT_HTML_OUTPUT_DIR=playwright-report-dashboard bun run test:e2e -- --workers=1 --retries=0 --output=test-results/dashboard-working-conversations dashboard-working-conversations-layout.spec.ts || dashboard_status=$?; "
            "E2E_BASE_URL=http://127.0.0.1:60083 E2E_PRODUCTION_BUILD=1 PLAYWRIGHT_HTML_OUTPUT_DIR=playwright-report-dashboard-performance bun run test:e2e -- --workers=1 --retries=0 --output=test-results/dashboard-render-performance dashboard-render-performance.spec.ts || performance_status=$?; "
            "exit $(( records_status || demo_status || dashboard_status || performance_status || retention_status ))"
        )
    return (
        "set -euo pipefail; "
        "records_status=0; demo_status=0; dashboard_status=0; performance_status=0; retention_status=0; timeline_capacity_status=0; "
        "PLAYWRIGHT_HTML_OUTPUT_DIR=playwright-report-records bun run test:e2e -- --workers=1 --retries=0 --output=test-results/records-overlay records-filter-overlay.spec.ts & records_pid=$!; "
        "E2E_BASE_URL=http://127.0.0.1:60083 PLAYWRIGHT_HTML_OUTPUT_DIR=playwright-report-demo bun run test:e2e -- --workers=1 --retries=0 --output=test-results/demo-runtime demo-runtime.spec.ts & demo_pid=$!; "
        "E2E_BASE_URL=http://127.0.0.1:60083 PLAYWRIGHT_HTML_OUTPUT_DIR=playwright-report-retention-ownership bun run test:e2e -- --workers=1 --retries=0 --output=test-results/retention-ownership retention-maintenance-ownership.spec.ts & retention_pid=$!; "
        "PLAYWRIGHT_HTML_OUTPUT_DIR=playwright-report-dashboard bun run test:e2e -- --workers=1 --retries=0 --output=test-results/dashboard-working-conversations dashboard-working-conversations-layout.spec.ts & dashboard_pid=$!; "
        "wait $records_pid || records_status=$?; wait $demo_pid || demo_status=$?; wait $retention_pid || retention_status=$?; wait $dashboard_pid || dashboard_status=$?; "
        "E2E_BASE_URL=http://127.0.0.1:60083 E2E_PRODUCTION_BUILD=1 PLAYWRIGHT_HTML_OUTPUT_DIR=playwright-report-dashboard-performance bun run test:e2e -- --workers=1 --retries=0 --output=test-results/dashboard-render-performance dashboard-render-performance.spec.ts || performance_status=$?; "
        "E2E_BASE_URL=http://127.0.0.1:60083 E2E_PRODUCTION_BUILD=1 E2E_TIMELINE_CAPACITY=1 E2E_TIMELINE_CAPACITY_DURATION_MS=60000 PLAYWRIGHT_HTML_OUTPUT_DIR=playwright-report-dashboard-timeline-capacity bun run test:e2e -- --workers=1 --retries=0 --output=test-results/dashboard-timeline-capacity dashboard-timeline-capacity.spec.ts || timeline_capacity_status=$?; "
        "exit $(( records_status || demo_status || retention_status || dashboard_status || performance_status || timeline_capacity_status ))"
    )


def stage_command(source: Path, stage: str, profile: str) -> tuple[str, ...]:
    if stage not in {"functional-e2e", "frontend"} or (source / "web" / "playwright.ci.config.ts").is_file():
        return STAGES[stage]
    legacy = legacy_playwright_schedule(profile)
    if stage == "functional-e2e":
        return ("bash", "-lc", legacy)
    frontend_prefix = STAGES["frontend"][2].split("CI_PROFILE=${CVM_BENCHMARK_PROFILE}", 1)[0]
    return ("bash", "-lc", frontend_prefix + legacy)


def run_sample(
    *,
    version: str,
    source: Path,
    source_sha: str,
    stage: str,
    profile: str,
    pair: int | None,
    phase: str,
    order: int,
    output_dir: Path,
) -> Sample:
    command = stage_command(source, stage, profile)
    env = os.environ.copy()
    env.update({"CI": "1", "PLAYWRIGHT_RETRIES": "0", "CVM_BENCHMARK_PROFILE": profile})
    log_name = f"{order:03d}-{phase}-{version}-pair-{pair or 0}.log"
    log_path = output_dir / "logs" / log_name
    log_path.parent.mkdir(parents=True, exist_ok=True)

    run_env = env
    if phase == "cold":
        cold_cache = output_dir / "cold-cache" / f"{version}-{order}"
        cold_cache.mkdir(parents=True, exist_ok=True)
        run_env = {**env, "XDG_CACHE_HOME": str(cold_cache)}
    timed_command = command
    if shutil.which("/usr/bin/time") is not None and sys.platform.startswith("linux"):
        timed_command = (
            "/usr/bin/time",
            "-f",
            "__CVM_USER=%U __CVM_SYS=%S __CVM_MAX_RSS_KB=%M",
            "--",
            *command,
        )
    started = time.monotonic()
    before = resource.getrusage(resource.RUSAGE_CHILDREN)
    timed_out = False
    try:
        completed = subprocess.run(
            timed_command,
            cwd=source_web_dir(source),
            env=run_env,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            check=False,
            timeout=MAX_SAMPLE_SECONDS,
        )
        output = completed.stdout
        returncode = completed.returncode
    except subprocess.TimeoutExpired as error:
        timed_out = True
        output = error.stdout or ""
        if isinstance(output, bytes):
            output = output.decode(errors="replace")
        output += f"\n__CVM_TIMEOUT_SECONDS={MAX_SAMPLE_SECONDS}\n"
        returncode = 124
    ended = time.monotonic()
    after = resource.getrusage(resource.RUSAGE_CHILDREN)
    log_path.write_text(output, encoding="utf-8")
    retries = retry_count(output)
    return Sample(
        version=version,
        stage=stage,
        profile=profile,
        pair=pair,
        phase=phase,
        order=order,
        command=command,
        source=str(source.resolve()),
        source_sha=source_sha,
        wall_seconds=ended - started,
        cpu_seconds=measurement_value(output, "__CVM_USER", "__CVM_SYS")
        or (after.ru_utime + after.ru_stime) - (before.ru_utime + before.ru_stime),
        peak_rss_bytes=(measurement_value(output, "__CVM_MAX_RSS_KB") or max(after.ru_maxrss, 0)) * 1024,
        returncode=returncode,
        retry_count=retries,
        valid=not timed_out and returncode == 0 and retries == 0,
        log=str(log_path),
    )


def write_summary(output_dir: Path, metadata: dict[str, Any], samples: list[Sample]) -> None:
    warm = [sample for sample in samples if sample.phase == "warm"]
    warmup = [sample for sample in samples if sample.phase == "warmup"]
    grouped: dict[str, list[float]] = {"baseline": [], "candidate": []}
    for sample in warm:
        if sample.valid:
            grouped[sample.version].append(sample.wall_seconds)
    baseline = summarize(grouped["baseline"])
    candidate = summarize(grouped["candidate"])
    baseline_median = baseline["median_seconds"]
    candidate_median = candidate["median_seconds"]
    if baseline_median is not None and candidate_median is not None:
        delta = baseline_median - candidate_median
        percent = delta / baseline_median * 100
    else:
        delta = None
        percent = None
    faster_pairs = sum(
        1
        for pair in range(1, metadata["rounds"] + 1)
        if next((s for s in warm if s.pair == pair and s.version == "candidate" and s.valid), None)
        and next((s for s in warm if s.pair == pair and s.version == "baseline" and s.valid), None)
        and next(s for s in warm if s.pair == pair and s.version == "candidate" and s.valid).wall_seconds
        < next(s for s in warm if s.pair == pair and s.version == "baseline" and s.valid).wall_seconds
    )
    required_warm_count = metadata["rounds"] * 2
    complete_and_valid = (
        metadata.get("environment_valid", True) and len(warm) == required_warm_count
        and len(warmup) == 2
        and all(sample.valid for sample in warm + warmup)
        and baseline_median is not None
        and candidate_median is not None
        and baseline["mad_seconds"] is not None
    )
    delta_value = float(delta) if delta is not None else None
    mad_value = float(baseline["mad_seconds"]) if baseline["mad_seconds"] is not None else None
    allowed_main_regression = (
        max(float(baseline_median) * 0.05, 5.0, 2 * mad_value)
        if baseline_median is not None and mad_value is not None
        else None
    )
    if metadata["profile"] == "main":
        acceptance_passed = bool(
            complete_and_valid
            and allowed_main_regression is not None
            and float(candidate_median) <= float(baseline_median) + allowed_main_regression
        )
        acceptance_mode = "main-regression-bound"
    elif metadata["stage"] == "frontend":
        required_delta = (
            max(float(baseline_median) * 0.10, 5.0, 2 * mad_value)
            if baseline_median is not None and mad_value is not None
            else None
        )
        acceptance_passed = bool(
            complete_and_valid
            and required_delta is not None
            and delta_value is not None
            and delta_value >= required_delta
            and faster_pairs >= 6
        )
        acceptance_mode = "pr-improvement-threshold"
    else:
        required_delta = None
        acceptance_passed = complete_and_valid
        acceptance_mode = "stage-validity-only"
    metadata["acceptance"] = {
        "mode": acceptance_mode,
        "passed": acceptance_passed,
        "complete_and_valid": complete_and_valid,
        "required_delta_seconds": required_delta if metadata["profile"] != "main" else None,
        "allowed_main_regression_seconds": allowed_main_regression,
        "required_faster_pairs": 6 if metadata["stage"] == "frontend" and metadata["profile"] == "pr" else None,
    }
    summary = {
        **metadata,
        "warm": {"baseline": baseline, "candidate": candidate},
        "delta_seconds": delta,
        "delta_percent": percent,
        "candidate_faster_pairs": faster_pairs,
        "valid_warm_samples": all(sample.valid for sample in warm),
        "samples": [sample.as_json() for sample in samples],
    }
    (output_dir / "metadata.json").write_text(json.dumps(metadata, indent=2) + "\n", encoding="utf-8")
    (output_dir / "summary.json").write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8")

    lines = [
        f"# Web test benchmark: {metadata['stage']} ({metadata['profile']})",
        "",
        f"Baseline `{metadata['baseline_sha']}`: {baseline['median_seconds']} s median, {baseline['mad_seconds']} s MAD",
        f"Candidate `{metadata['candidate_sha']}`: {candidate['median_seconds']} s median, {candidate['mad_seconds']} s MAD",
        f"Delta: {delta} s ({percent}%), candidate faster in {faster_pairs}/{metadata['rounds']} pairs.",
        f"Acceptance: {'passed' if metadata['acceptance']['passed'] else 'failed'} ({metadata['acceptance']['mode']}).",
        "",
        "Failed or retried samples are retained in `raw.jsonl` but excluded from medians; warmup failures invalidate the group.",
        "",
        "| order | phase | version | pair | wall (s) | CPU (s) | RSS (MiB) | exit | valid |",
        "| ---: | --- | --- | ---: | ---: | ---: | ---: | ---: | --- |",
    ]
    for sample in samples:
        lines.append(
            f"| {sample.order} | {sample.phase} | {sample.version} | {sample.pair or '-'} | "
            f"{sample.wall_seconds:.3f} | {sample.cpu_seconds:.3f} | {sample.peak_rss_bytes / 1024 / 1024:.1f} | "
            f"{sample.returncode} | {'yes' if sample.valid else 'no'} |"
        )
    (output_dir / "summary.md").write_text("\n".join(lines) + "\n", encoding="utf-8")


def main() -> int:
    if os.environ.get("GITHUB_ACTIONS") != "true":
        raise SystemExit("AB performance benchmarks must run in GitHub Actions; local and Agent VM execution is forbidden")
    args = parse_args()
    if args.rounds < 1:
        raise SystemExit("--rounds must be positive")
    output_dir = args.output
    output_dir.mkdir(parents=True, exist_ok=True)
    baseline_source = args.baseline_src.resolve()
    candidate_source = args.candidate_src.resolve()
    if baseline_source == candidate_source:
        raise SystemExit("baseline and candidate sources must be independent directories")
    actual_shas = {"baseline": git_head(baseline_source), "candidate": git_head(candidate_source)}
    expected_shas = {"baseline": args.baseline_sha, "candidate": args.candidate_sha}
    if actual_shas != expected_shas:
        raise SystemExit(f"source SHA mismatch: expected {expected_shas}, got {actual_shas}")
    lockfiles = {"baseline": lockfile_digest(baseline_source), "candidate": lockfile_digest(candidate_source)}
    if len(set(lockfiles.values())) != 1:
        raise SystemExit(f"lockfile mismatch: {lockfiles}")
    metadata = {
        "baseline_src": str(baseline_source),
        "candidate_src": str(candidate_source),
        "baseline_sha": args.baseline_sha,
        "candidate_sha": args.candidate_sha,
        "stage": args.stage,
        "profile": args.profile,
        "rounds": args.rounds,
        "python": sys.version,
        "platform": sys.platform,
        "cpu_count": os.cpu_count(),
        "environment": {
            "baseline": environment_snapshot(baseline_source),
            "candidate": environment_snapshot(candidate_source),
        },
        "actual_shas": actual_shas,
        "lockfile_sha256": lockfiles,
        "clock": "time.monotonic",
        "cache_policy": "warm paired samples after one warmup per source; one isolated-process/cache descriptive sample per source",
    }
    metadata["environment_valid"] = all(
        snapshot.get("bun_valid")
        and snapshot.get("bun_expected") == "1.3.14"
        and not str(snapshot.get("playwright", "")).startswith("unavailable:")
        for snapshot in metadata["environment"].values()
    )
    (output_dir / "metadata.json").write_text(json.dumps(metadata, indent=2) + "\n", encoding="utf-8")
    samples: list[Sample] = []
    order = 1
    for version, source, sha in (
        ("baseline", args.baseline_src, args.baseline_sha),
        ("candidate", args.candidate_src, args.candidate_sha),
    ):
        samples.append(
            run_sample(
                version=version,
                source=source,
                source_sha=sha,
                stage=args.stage,
                profile=args.profile,
                pair=None,
                phase="warmup",
                order=order,
                output_dir=output_dir,
            )
        )
        order += 1
    for version, source, sha in (
        ("baseline", args.baseline_src, args.baseline_sha),
        ("candidate", args.candidate_src, args.candidate_sha),
    ):
        samples.append(
            run_sample(
                version=version,
                source=source,
                source_sha=sha,
                stage=args.stage,
                profile=args.profile,
                pair=None,
                phase="cold",
                order=order,
                output_dir=output_dir,
            )
        )
        order += 1
    for pair in range(1, args.rounds + 1):
        versions = (("baseline", args.baseline_src, args.baseline_sha), ("candidate", args.candidate_src, args.candidate_sha))
        if pair % 2 == 0:
            versions = tuple(reversed(versions))
        for version, source, sha in versions:
            samples.append(
                run_sample(
                    version=version,
                    source=source,
                    source_sha=sha,
                    stage=args.stage,
                    profile=args.profile,
                    pair=pair,
                    phase="warm",
                    order=order,
                    output_dir=output_dir,
                )
            )
            order += 1
    raw_path = output_dir / "raw.jsonl"
    raw_path.write_text("".join(json.dumps(sample.as_json()) + "\n" for sample in samples), encoding="utf-8")
    write_summary(output_dir, metadata, samples)
    return 0 if metadata["acceptance"]["passed"] else 2


if __name__ == "__main__":
    raise SystemExit(main())
