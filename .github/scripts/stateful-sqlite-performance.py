#!/usr/bin/env python3
"""Aggregate the opt-in Stateful SQLite performance acceptance logs."""

from __future__ import annotations

import argparse
import json
import re
import statistics
import sys
from pathlib import Path


TARGETS = [
    "tests::stateful_sqlite::startup_rebuild_and_retention_basics::"
    "startup_hot_read_hydration_keeps_health_ready_under_sqlite_pool_pressure",
    "tests::stateful_sqlite::startup_rebuild_and_retention_basics::"
    "summary_startup_hydration_publishes_bounded_fallback_under_sqlite_pressure",
    "tests::stateful_sqlite::startup_rebuild_and_retention_basics::"
    "system_status_startup_hydration_has_a_finite_sqlite_pressure_deadline",
    "tests::stateful_sqlite::parallel_work_stats_and_timeseries::"
    "restart_schema_refreshes_existing_objects_across_primed_pool_connections",
    "tests::stateful_sqlite::proxy_backfill_and_cost_repairs::"
    "run_backfill_with_retry_succeeds_after_lock_release",
    "tests::stateful_sqlite::system_status_and_account_roster::"
    "system_raw_metrics_inventory_revalidates_existing_paths_after_hydration",
]

PHASE_RE = re.compile(
    r"stateful-test-timing phase=(?P<phase>[a-z0-9_]+) "
    r"elapsed_ms=(?P<elapsed>[0-9]+)"
)
WALL_RE = re.compile(
    r"^stateful-performance ref=(?P<ref>baseline|candidate) "
    r"replay=(?P<replay>cold|warm) shard=(?P<shard>[12]) "
    r"wall_ms=(?P<wall>[0-9]+) exit_code=(?P<exit>[0-9]+)$"
)
EXCLUDED_FROM_SETUP = {"production_operation", "production_spawn", "explicit_wait", "cleanup", "whole_test"}


def median(values: list[int]) -> int:
    return int(statistics.median(values))


def target_from_lines(lines: list[str], end_index: int) -> str | None:
    for line in reversed(lines[max(0, end_index - 400) : end_index]):
        for target in TARGETS:
            if target in line:
                return target
    return None


def parse_log(path: Path) -> dict[str, dict[str, list[int]]]:
    lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
    events: dict[str, dict[str, list[int]]] = {}
    current_target: str | None = None
    in_stderr = False
    for index, line in enumerate(lines):
        if "stderr ───" in line:
            current_target = target_from_lines(lines, index)
            in_stderr = True
            continue
        if "stdout ───" in line:
            in_stderr = False
            continue
        if not in_stderr or current_target is None:
            continue
        match = PHASE_RE.search(line)
        if match is None:
            continue
        phase = match.group("phase")
        events.setdefault(current_target, {}).setdefault(phase, []).append(
            int(match.group("elapsed"))
        )
    return events


def collect_replay(root: Path, ref: str, replay: str) -> dict[str, dict[str, list[int]]]:
    events: dict[str, dict[str, list[int]]] = {}
    for shard in ("1", "2"):
        path = root / "logs" / ref / replay / f"shard-{shard}.log"
        if not path.is_file():
            raise ValueError(f"missing replay log: {ref}/{replay}/shard-{shard}")
        for target, phases in parse_log(path).items():
            existing = events.setdefault(target, {})
            for phase, values in phases.items():
                existing.setdefault(phase, []).extend(values)
    missing = [target for target in TARGETS if "whole_test" not in events.get(target, {})]
    if missing:
        raise ValueError(f"missing whole_test timing for {ref}/{replay}: {', '.join(missing)}")
    return events


def sample(events: dict[str, list[int]]) -> dict[str, object]:
    phase_totals = {phase: sum(values) for phase, values in events.items()}
    setup_ms = sum(value for phase, value in phase_totals.items() if phase not in EXCLUDED_FROM_SETUP)
    return {
        "phase_ms": phase_totals,
        "setup_ms": setup_ms,
        "production_ms": phase_totals.get("production_operation", 0),
        "wait_ms": phase_totals.get("explicit_wait", 0),
        "cleanup_ms": phase_totals.get("cleanup", 0),
        "whole_test_ms": phase_totals["whole_test"],
    }


def parse_wall_file(path: Path) -> tuple[dict[tuple[str, str, str], int], list[str]]:
    values: dict[tuple[str, str, str], int] = {}
    failures: list[str] = []
    for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
        match = WALL_RE.fullmatch(line.strip())
        if match is None:
            continue
        key = (match.group("ref"), match.group("replay"), match.group("shard"))
        if key in values:
            raise ValueError(f"duplicate wall-time record: {'/'.join(key)}")
        exit_code = int(match.group("exit"))
        if exit_code != 0:
            failures.append(f"{'/'.join(key)} exit_code={exit_code}")
        values[key] = int(match.group("wall"))
    expected = {
        (ref, replay, shard)
        for ref in ("baseline", "candidate")
        for replay in ("cold", "warm")
        for shard in ("1", "2")
    }
    missing = sorted(expected - values.keys())
    if missing:
        raise ValueError("missing wall-time records: " + ", ".join("/".join(key) for key in missing))
    return values, failures


def build_card(
    root: Path,
    baseline_sha: str,
    candidate_sha: str,
    wall: dict[tuple[str, str, str], int],
    replay_failures: list[str],
) -> dict[str, object]:
    replay_samples: dict[str, dict[str, dict[str, object]]] = {"baseline": {}, "candidate": {}}
    for ref in replay_samples:
        for replay in ("cold", "warm"):
            events = collect_replay(root, ref, replay)
            replay_samples[ref][replay] = {target: sample(events[target]) for target in TARGETS}

    tests: list[dict[str, object]] = []
    measured_setup_improvements = 0
    measured_setup_tests = 0
    for target in TARGETS:
        baseline_samples = [replay_samples["baseline"][replay][target] for replay in ("cold", "warm")]
        candidate_samples = [replay_samples["candidate"][replay][target] for replay in ("cold", "warm")]
        baseline_setup = median([int(item["setup_ms"]) for item in baseline_samples])
        candidate_setup = median([int(item["setup_ms"]) for item in candidate_samples])
        if baseline_setup > 0 or candidate_setup > 0:
            measured_setup_tests += 1
            reduction_pct = (baseline_setup - candidate_setup) * 100.0 / baseline_setup if baseline_setup else -100.0
            improved = reduction_pct >= 20.0
            if improved:
                measured_setup_improvements += 1
            setup_status = "pass" if improved else "fail"
        else:
            reduction_pct = None
            setup_status = "control"
        tests.append(
            {
                "test": target,
                "baseline_setup_median_ms": baseline_setup,
                "candidate_setup_median_ms": candidate_setup,
                "setup_reduction_pct": reduction_pct,
                "setup_status": setup_status,
                "baseline_whole_test_median_ms": median(
                    [int(item["whole_test_ms"]) for item in baseline_samples]
                ),
                "candidate_whole_test_median_ms": median(
                    [int(item["whole_test_ms"]) for item in candidate_samples]
                ),
                "baseline_phase_medians_ms": {
                    phase: median([int(item["phase_ms"].get(phase, 0)) for item in baseline_samples])
                    for phase in sorted({phase for item in baseline_samples for phase in item["phase_ms"]})
                },
                "candidate_phase_medians_ms": {
                    phase: median([int(item["phase_ms"].get(phase, 0)) for item in candidate_samples])
                    for phase in sorted({phase for item in candidate_samples for phase in item["phase_ms"]})
                },
            }
        )

    shard_wall: list[dict[str, object]] = []
    for shard in ("1", "2"):
        baseline_values = [wall[("baseline", replay, shard)] for replay in ("cold", "warm")]
        candidate_values = [wall[("candidate", replay, shard)] for replay in ("cold", "warm")]
        baseline_median = median(baseline_values)
        candidate_median = median(candidate_values)
        shard_wall.append(
            {
                "shard": shard,
                "baseline_replays_ms": baseline_values,
                "candidate_replays_ms": candidate_values,
                "baseline_median_ms": baseline_median,
                "candidate_median_ms": candidate_median,
                "regression_pct": (candidate_median - baseline_median) * 100.0 / baseline_median
                if baseline_median
                else None,
            }
        )

    baseline_totals = [sum(wall[("baseline", replay, shard)] for shard in ("1", "2")) for replay in ("cold", "warm")]
    candidate_totals = [sum(wall[("candidate", replay, shard)] for shard in ("1", "2")) for replay in ("cold", "warm")]
    baseline_total_median = median(baseline_totals)
    candidate_total_median = median(candidate_totals)
    total_regression_pct = (
        (candidate_total_median - baseline_total_median) * 100.0 / baseline_total_median
        if baseline_total_median
        else None
    )
    setup_pass = measured_setup_tests >= 4 and measured_setup_improvements >= 4
    wall_pass = candidate_total_median <= baseline_total_median * 1.05
    accepted = not replay_failures and setup_pass and wall_pass
    return {
        "acceptance": "pass" if accepted else "fail",
        "baseline_sha": baseline_sha,
        "candidate_sha": candidate_sha,
        "runner": "GitHub-hosted ubuntu-24.04; one job; 6 nextest threads",
        "replay_order": ["cold", "warm"],
        "thresholds": {
            "minimum_setup_improvements": 4,
            "setup_reduction_pct": 20.0,
            "maximum_full_profile_regression_pct": 5.0,
        },
        "setup_gate": {
            "measured_tests": measured_setup_tests,
            "improved_tests": measured_setup_improvements,
            "passed": setup_pass,
        },
        "full_profile_gate": {
            "baseline_replays_ms": baseline_totals,
            "candidate_replays_ms": candidate_totals,
            "baseline_median_ms": baseline_total_median,
            "candidate_median_ms": candidate_total_median,
            "regression_pct": total_regression_pct,
            "passed": wall_pass,
        },
        "replay_failures": replay_failures,
        "shard_wall": shard_wall,
        "tests": tests,
    }


def markdown_card(card: dict[str, object]) -> str:
    setup_gate = card["setup_gate"]
    full_gate = card["full_profile_gate"]
    lines = [
        "# Stateful SQLite empirical acceptance",
        "",
        f"Decision: **{str(card['acceptance']).upper()}**",
        "",
        f"- Baseline: `{card['baseline_sha']}`",
        f"- Candidate: `{card['candidate_sha']}`",
        f"- Runner: {card['runner']}",
        f"- Replay order: `{', '.join(card['replay_order'])}`",
        "",
    ]
    if card["replay_failures"]:
        lines.extend(
            [
                "## Replay failures",
                "",
                "The card is diagnostic only because one or more replay commands failed:",
                "",
            ]
            + [f"- `{failure}`" for failure in card["replay_failures"]]
            + [""],
        )
    lines.extend(
        [
            "## Setup gate",
            "",
            f"{setup_gate['improved_tests']}/{setup_gate['measured_tests']} setup-bearing tests improved by at least 20%.",
            "",
            "| Test | Baseline setup median (ms) | Candidate setup median (ms) | Reduction | Status |",
            "| --- | ---: | ---: | ---: | --- |",
        ]
    )
    for result in card["tests"]:
        reduction = result["setup_reduction_pct"]
        reduction_text = "control" if reduction is None else f"{reduction:.1f}%"
        short_name = str(result["test"]).rsplit("::", 1)[-1]
        lines.append(
            f"| `{short_name}` | {result['baseline_setup_median_ms']} | "
            f"{result['candidate_setup_median_ms']} | {reduction_text} | {result['setup_status']} |"
        )
    lines.extend(
        [
            "",
            "## Full profile wall time",
            "",
            f"Baseline median: `{full_gate['baseline_median_ms']} ms`; "
            f"candidate median: `{full_gate['candidate_median_ms']} ms`; "
            f"regression: `{full_gate['regression_pct']:.1f}%`; "
            f"limit: `5.0%`; gate: **{'pass' if full_gate['passed'] else 'fail'}**.",
            "",
            "| Shard | Baseline replays (ms) | Candidate replays (ms) | Baseline median (ms) | Candidate median (ms) |",
            "| --- | --- | --- | ---: | ---: |",
        ]
    )
    for shard in card["shard_wall"]:
        lines.append(
            f"| {shard['shard']} | {', '.join(str(value) for value in shard['baseline_replays_ms'])} | "
            f"{', '.join(str(value) for value in shard['candidate_replays_ms'])} | "
            f"{shard['baseline_median_ms']} | {shard['candidate_median_ms']} |"
        )
    lines.append("")
    return "\n".join(lines)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--wall-file", type=Path, required=True)
    parser.add_argument("--baseline-sha", required=True)
    parser.add_argument("--candidate-sha", required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    args = parser.parse_args()

    try:
        wall, replay_failures = parse_wall_file(args.wall_file)
        card = build_card(
            args.root,
            args.baseline_sha,
            args.candidate_sha,
            wall,
            replay_failures,
        )
    except (OSError, ValueError, KeyError) as error:
        print(f"::error::stateful SQLite performance evidence is incomplete: {error}", file=sys.stderr)
        return 1

    args.output_dir.mkdir(parents=True, exist_ok=True)
    (args.output_dir / "acceptance-card.json").write_text(
        json.dumps(card, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    (args.output_dir / "acceptance-card.md").write_text(markdown_card(card), encoding="utf-8")
    print(markdown_card(card))
    return 0 if card["acceptance"] == "pass" else 1


if __name__ == "__main__":
    raise SystemExit(main())
