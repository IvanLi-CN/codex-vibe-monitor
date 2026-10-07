#!/usr/bin/env python3
"""Classify the full performance acceptance result without rewriting evidence."""
from __future__ import annotations

import argparse
import json
from pathlib import Path


EXPECTED_SCENARIOS = frozenset(
    {
        "https-auth-query",
        "tempo-cases-tenant",
        "monitoring-fault-isolation",
        "original-process-cpu",
        "default-observability-ab",
    }
)
RESOURCE_UNAVAILABLE_ERRORS = frozenset(
    {
        "cumulative quiet admission budget exhausted",
        "quiet admission budget exhausted",
        "runner did not reach a quiet window; performance acceptance unavailable",
        "runner pressure evidence is invalid",
    }
)


class ClassificationError(RuntimeError):
    """Raised when an acceptance result cannot be safely classified."""


def _read_json(root: Path, name: str) -> object:
    try:
        return json.loads((root / name).read_text())
    except (OSError, ValueError, UnicodeError) as error:
        raise ClassificationError(f"missing or invalid {name}") from error


def _resource_pressure_error(error: object) -> bool:
    if not isinstance(error, str):
        return False
    if error in RESOURCE_UNAVAILABLE_ERRORS:
        return True
    prefix = "measured resource environment unavailable:"
    if not error.startswith(prefix):
        return False
    codes = {code for code in error[len(prefix) :].strip().split(",") if code}
    return bool(codes) and codes <= {"pressure_exceeded"}


def _scenario_statuses(scenarios: object) -> dict[str, dict[str, object]]:
    if not isinstance(scenarios, dict) or set(scenarios) != EXPECTED_SCENARIOS:
        raise ClassificationError("performance scenario set is incomplete or unexpected")
    if any(not isinstance(value, dict) for value in scenarios.values()):
        raise ClassificationError("performance scenario evidence is malformed")
    return scenarios  # type: ignore[return-value]


def classify_result(root: Path, step_outcome: str) -> str:
    """Return ``passed`` or ``neutral-unavailable`` for a safe job outcome.

    The acceptance script intentionally exits non-zero when the hosted runner
    cannot provide valid resource evidence. This classifier makes that one
    environmental condition neutral at the workflow boundary while preserving
    the unavailable card and all raw evidence.
    """
    card = _read_json(root, "empirical-card.json")
    scenarios = _scenario_statuses(_read_json(root, "scenarios.json"))
    if not isinstance(card, dict) or not isinstance(card.get("empirical_evidence_status"), str):
        raise ClassificationError("empirical card is malformed")
    evidence_status = card["empirical_evidence_status"]

    if step_outcome == "success":
        if evidence_status != "passed" or any(row.get("status") != "passed" for row in scenarios.values()):
            raise ClassificationError("successful acceptance step did not produce a passing certificate")
        return "passed"

    if step_outcome != "failure":
        raise ClassificationError(f"acceptance step ended with unsupported outcome {step_outcome!r}")
    if evidence_status != "unavailable":
        raise ClassificationError("failed acceptance step did not produce an unavailable evidence card")

    default = scenarios["default-observability-ab"]
    if any(scenarios[name].get("status") != "passed" for name in EXPECTED_SCENARIOS - {"default-observability-ab"}):
        raise ClassificationError("a functional or CPU acceptance scenario failed")
    if default.get("status") != "unavailable" or not _resource_pressure_error(default.get("error")):
        raise ClassificationError("unavailable evidence was not caused by runner resource pressure")
    return "neutral-unavailable"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--step-outcome", required=True)
    args = parser.parse_args()
    try:
        disposition = classify_result(args.root, args.step_outcome)
    except ClassificationError as error:
        print(f"::error::{error}")
        return 1
    if disposition == "neutral-unavailable":
        print("Performance evidence unavailable because the hosted runner exceeded the resource-pressure limits; preserving the unavailable card and continuing as an auxiliary check.")
    else:
        print("Performance acceptance produced a passing certificate.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
