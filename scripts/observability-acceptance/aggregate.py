#!/usr/bin/env python3
"""Verify all isolated parts before issuing the only full empirical certificate."""
import argparse
import json
import time
from pathlib import Path

from environment import actions_context, comparison_report, verify_measurement_evidence
from run import certificate

RUNTIME_SCENARIOS = {"https-auth-query", "tempo-cases-tenant", "monitoring-fault-isolation", "original-process-cpu"}
LANES = ("runtime", "pair-0", "pair-1", "pair-2")


def read(root, name):
    return json.loads((root / name).read_text())


def verify_part(root, lane, expected):
    card, config, context = (read(root, name) for name in ("runtime-card.json", "run-config.json", "runner-context.json"))
    image = read(root, "image-identity.json")
    outcome = read(root, "part-outcome.json")["outcome"]
    for key in ("empirical_candidate_sha", "acceptance_contract_digest", "scenario_set_digest", "evidence_locator"):
        if card[key] != expected[key]:
            raise ValueError("part certificate identity mismatch")
    if (context["runnerEnvironment"] != "github-hosted" or context["evidenceLocator"] != expected["evidence_locator"]
            or context["candidate"] != expected["empirical_candidate_sha"] or config["candidate"] != context["candidate"]
            or config["suite"] != lane or image["Config"]["Labels"]["org.opencontainers.image.revision"] != context["candidate"]):
        raise ValueError("part runner/image identity mismatch")
    scenarios = read(root, "scenarios.json")
    names = RUNTIME_SCENARIOS if lane == "runtime" else {"default-observability-ab"}
    if set(scenarios) != names:
        raise ValueError("incomplete or unexpected part scenarios")
    passed = all(row["status"] == "passed" for row in scenarios.values())
    elapsed = config["executionFinishedUTCSeconds"] - config["executionStartedUTCSeconds"]
    if not 0 <= elapsed <= 900 or (passed and outcome != "success"):
        raise ValueError("part exceeded its deadline or failed after writing evidence")
    if (card["empirical_evidence_status"] == "passed") != passed:
        raise ValueError("part disposition mismatch")
    if lane != "runtime" and passed:
        verify_measurement_evidence(root, pair_index=int(lane[-1]))
    return image["Id"], context, scenarios


def aggregate(source, candidate_source, root, parts, candidate):
    root.mkdir(parents=True, exist_ok=True)
    # The result job verifies runner identities from each measurement part; it
    # is not itself a measurement runner and may not expose RUNNER_ID.
    context = actions_context(candidate_source, root, candidate, require_runner_identity=False)
    expected = certificate(source, candidate, context, "unavailable", "Hosted runtime and three isolated same-runner paired experiments; each test job is bounded to 15 minutes.")
    scenarios = {name: {"status": "unavailable", "error": "required part evidence missing or invalid"}
                 for name in RUNTIME_SCENARIOS | {"default-observability-ab"}}
    status = "unavailable"
    try:
        verified = []
        prefix = f"observability-part-{context['runId']}-{context['attempt']}-"
        for lane in LANES:
            part = parts / (prefix + lane)
            verified.append(verify_part(part, lane, expected))
        starts = [read(parts / (prefix + lane), "run-config.json")["executionStartedUTCSeconds"] for lane in LANES]
        if len(set(starts)) != 1:
            raise ValueError("parts reset the shared test clock")
        if not 0 <= time.time() - starts[0] <= 900:
            raise ValueError("shared test phase deadline exceeded")
        if len({item[0] for item in verified}) != 1:
            raise ValueError("parts used different candidate images")
        pairs = [item[1] for item in verified[1:]]
        # Each pair owns its off/metrics/full windows on one isolated runner.
        # GitHub-hosted runners may legitimately differ across pairs, so raw
        # machine-dependent timings are never compared across hosts.
        if len({pair["runnerIdentity"] for pair in pairs}) != 3:
            raise ValueError("paired experiments did not use isolated runners")
        scenarios = dict(verified[0][2])
        pair_scenarios = [item[2]["default-observability-ab"] for item in verified[1:]]
        bad = next((row for row in pair_scenarios if row["status"] == "failed"), None)
        bad = bad or next((row for row in pair_scenarios if row["status"] != "passed"), None)
        if bad:
            scenarios["default-observability-ab"] = bad
        else:
            samples = {mode: [] for mode in ("off", "metrics", "full")}
            for index in range(3):
                values = read(parts / (prefix + f"pair-{index}"), "ab-samples.json")
                for mode in samples:
                    samples[mode].extend(values[mode])
            report = comparison_report(samples)
            (root / "ab-samples.json").write_text(json.dumps(samples, indent=2) + "\n")
            (root / "ab-summary.json").write_text(json.dumps(report, indent=2) + "\n")
            stable = all(metric["comparisonValid"] for metric in report["metrics"].values())
            within = all(metric["withinBudget"] for metric in report["metrics"].values())
            scenarios["default-observability-ab"] = {"status": "passed" if within else "unavailable" if not stable else "failed"}
            if not within:
                scenarios["default-observability-ab"]["error"] = "unstable measurement windows" if not stable else "5% observability budget exceeded"
        status = "failed" if any(row["status"] == "failed" for row in scenarios.values()) else "unavailable" if any(row["status"] != "passed" for row in scenarios.values()) else "passed"
    except (OSError, ValueError, KeyError, TypeError) as error:
        # Missing parts, mismatches, and timeout never produce a passing card.
        scenarios["default-observability-ab"] = {"status": "unavailable", "error": "part evidence invalid: " + type(error).__name__}
    expected["empirical_evidence_status"] = status
    (root / "scenarios.json").write_text(json.dumps(scenarios, indent=2) + "\n")
    (root / "empirical-card.json").write_text(json.dumps(expected, indent=2) + "\n")
    return status == "passed"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("source", "candidate-source", "run", "parts", "candidate"):
        parser.add_argument("--" + name, required=True)
    args = parser.parse_args()
    return 0 if aggregate(Path(args.source).resolve(), Path(args.candidate_source).resolve(), Path(args.run).resolve(), Path(args.parts).resolve(), args.candidate) else 1


if __name__ == "__main__":
    raise SystemExit(main())
