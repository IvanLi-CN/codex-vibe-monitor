#!/usr/bin/env python3
"""Verify all serial-bundle parts before issuing the only full empirical certificate."""
import argparse
import json
import time
from pathlib import Path

from cpu_attribution import cpu_attribution_diagnostic
from deadline import TEST_JOB_SECONDS
from environment import actions_context, comparison_report, verify_measurement_evidence
from run import acceptance_contract_digest, acceptance_harness_digest, certificate, source_revision

RUNTIME_SCENARIOS = {"https-auth-query", "tempo-cases-tenant", "monitoring-fault-isolation", "original-process-cpu"}
LANES = ("runtime", "pair-0", "pair-1", "pair-2")
HARDWARE_KEYS = ("cpuModel", "cpuCount", "kernel", "runnerImage", "runnerImageVersion")


class HardwareCompatibilityError(ValueError):
    """A stable reason for evidence that cannot support the paired comparison."""


class AcceptanceHarnessIdentityError(ValueError):
    """Evidence was produced with a different trusted or candidate harness."""


def read(root, name):
    return json.loads((root / name).read_text())


def verify_part(
    root,
    lane,
    expected,
    trusted_harness_sha,
    trusted_contract_digest,
    candidate_contract_digest,
    trusted_harness_digest,
    candidate_harness_digest,
):
    card, config, context = (read(root, name) for name in ("runtime-card.json", "run-config.json", "runner-context.json"))
    image = read(root, "image-identity.json")
    outcome = read(root, "part-outcome.json")["outcome"]
    for key in ("empirical_candidate_sha", "acceptance_contract_digest", "scenario_set_digest", "evidence_locator"):
        if card[key] != expected[key]:
            raise ValueError("part certificate identity mismatch")
    if (config.get("harnessSha") != trusted_harness_sha
            or config.get("acceptanceContractDigest") != trusted_contract_digest
            or config.get("acceptanceHarnessDigest") != trusted_harness_digest):
        raise AcceptanceHarnessIdentityError("trusted acceptance harness identity mismatch")
    if (candidate_contract_digest != trusted_contract_digest
            or candidate_harness_digest != trusted_harness_digest
            or config.get("candidateAcceptanceContractDigest") != candidate_contract_digest
            or config.get("candidateAcceptanceHarnessDigest") != candidate_harness_digest):
        raise AcceptanceHarnessIdentityError("candidate acceptance contract or harness differs from trusted harness; run post-merge verification")
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
    if not 0 <= elapsed <= TEST_JOB_SECONDS or (passed and outcome != "success"):
        raise ValueError("part exceeded its deadline or failed after writing evidence")
    if (card["empirical_evidence_status"] == "passed") != passed:
        raise ValueError("part disposition mismatch")
    if lane != "runtime" and passed:
        verify_measurement_evidence(root, pair_index=int(lane[-1]))
    return image["Id"], context, scenarios


def hardware_signature(context):
    for key in HARDWARE_KEYS:
        if key not in context or context[key] in (None, "", []):
            raise HardwareCompatibilityError(f"hardware metadata unavailable: missing {key}")
    models = context["cpuModel"]
    if not isinstance(models, list) or any(not isinstance(model, str) or not model for model in models):
        raise HardwareCompatibilityError("hardware metadata invalid: cpuModel")
    if len(set(models)) != len(models):
        raise HardwareCompatibilityError("hardware metadata invalid: duplicate cpuModel")
    if len(models) != 1:
        raise HardwareCompatibilityError("hardware metadata incompatible: cpuModel")
    if isinstance(context["cpuCount"], bool) or not isinstance(context["cpuCount"], int) or context["cpuCount"] <= 0:
        raise HardwareCompatibilityError("hardware metadata invalid: cpuCount")
    if any(not isinstance(context[key], str) or not context[key] for key in ("kernel", "runnerImage", "runnerImageVersion")):
        raise HardwareCompatibilityError("hardware metadata invalid: runner identity")
    return (
        tuple(models),
        context["cpuCount"],
        context["kernel"],
        context["runnerImage"],
        context["runnerImageVersion"],
    )


def verify_hardware_compatibility(contexts):
    if len(contexts) != 3:
        raise HardwareCompatibilityError("hardware metadata unavailable: expected three paired suites")
    if any(context.get("runnerScope") != "serial-bundle" for context in contexts):
        raise HardwareCompatibilityError("hardware metadata unavailable: missing serial runner scope")
    if len({context["runnerIdentity"] for context in contexts}) != 1:
        raise HardwareCompatibilityError("hardware metadata incompatible: runner identity")
    reference = hardware_signature(contexts[0])
    for context in contexts[1:]:
        signature = hardware_signature(context)
        for index, key in enumerate(HARDWARE_KEYS):
            if signature[index] != reference[index]:
                raise HardwareCompatibilityError(f"hardware metadata incompatible: {key}")


def _pair_issue(pair_index, scenario):
    status = scenario.get("status") if isinstance(scenario, dict) else None
    issue = {"pairIndex": pair_index, "status": status if status in {"failed", "unavailable"} else "unavailable"}
    diagnostic = scenario.get("diagnostic") if isinstance(scenario, dict) else None
    if isinstance(diagnostic, dict):
        issue["diagnostic"] = cpu_attribution_diagnostic(
            None,
            diagnostic.get("stage", "unknown"),
            code=diagnostic.get("reason"),
            source=diagnostic.get("source"),
        )
    elif issue["status"] == "failed":
        issue["reason"] = "part_failed"
    else:
        issue["reason"] = "part_unavailable"
    return issue


def _scenario_status(scenarios):
    if any(row.get("status") == "failed" for row in scenarios.values()):
        return "failed"
    return "passed" if all(row.get("status") == "passed" for row in scenarios.values()) else "unavailable"


def aggregate(source, candidate_source, root, parts, candidate):
    root.mkdir(parents=True, exist_ok=True)
    # Derived comparison files are valid only for the current complete aggregation.
    for name in ("ab-samples.json", "ab-summary.json"):
        try:
            (root / name).unlink()
        except FileNotFoundError:
            pass
    # The result job verifies identities from each measurement part; it is not
    # itself a measurement runner and does not need a job-scoped identity.
    context = actions_context(candidate_source, root, candidate, require_runner_identity=False)
    expected = certificate(source, candidate, context, "unavailable", "Hosted runtime and three sequential same-runner paired experiments in one job; the bundle is bounded to 60 minutes.")
    trusted_harness_sha = source_revision(source)
    trusted_contract_digest = acceptance_contract_digest(source)
    candidate_contract_digest = acceptance_contract_digest(candidate_source)
    trusted_harness_digest = acceptance_harness_digest(source)
    candidate_harness_digest = acceptance_harness_digest(candidate_source)
    scenarios = {name: {"status": "unavailable", "error": "required part evidence missing or invalid"}
                 for name in RUNTIME_SCENARIOS | {"default-observability-ab"}}
    status = "unavailable"
    pair_scenarios = []
    try:
        verified = []
        prefix = f"observability-part-{context['runId']}-{context['attempt']}-"
        for lane in LANES:
            part = parts / (prefix + lane)
            verified_part = verify_part(
                part,
                lane,
                expected,
                trusted_harness_sha,
                trusted_contract_digest,
                candidate_contract_digest,
                trusted_harness_digest,
                candidate_harness_digest,
            )
            verified.append(verified_part)
            if lane == "runtime":
                scenarios = dict(verified_part[2])
        starts = [read(parts / (prefix + lane), "run-config.json")["executionStartedUTCSeconds"] for lane in LANES]
        if len(set(starts)) != 1:
            raise ValueError("parts reset the shared test clock")
        if not 0 <= time.time() - starts[0] <= TEST_JOB_SECONDS:
            raise ValueError("shared test phase deadline exceeded")
        if len({item[0] for item in verified}) != 1:
            raise ValueError("parts used different candidate images")
        pairs = [item[1] for item in verified[1:]]
        pair_scenarios = [item[2]["default-observability-ab"] for item in verified[1:]]
        # The bundle owns all off/metrics/full windows on one Actions runner.
        # Exact hardware/runtime metadata remains mandatory even though the job
        # identity already proves that the three suites share one runner.
        verify_hardware_compatibility(pairs)
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
    except HardwareCompatibilityError as error:
        scenarios["default-observability-ab"] = {"status": "unavailable", "error": str(error)}
        issues = [_pair_issue(index, scenario) for index, scenario in enumerate(pair_scenarios) if scenario.get("status") != "passed"]
        if issues:
            scenarios["default-observability-ab"]["pairIssues"] = issues
    except AcceptanceHarnessIdentityError as error:
        scenarios["default-observability-ab"] = {"status": "unavailable", "error": str(error)}
    except FileNotFoundError:
        scenarios["default-observability-ab"] = {
            "status": "unavailable",
            "error": "required part evidence missing or invalid",
        }
    except (OSError, ValueError, KeyError, TypeError):
        # Missing parts, mismatches, and timeout never produce a passing card.
        scenarios["default-observability-ab"] = {"status": "unavailable", "error": "part evidence invalid"}
    status = _scenario_status(scenarios)
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
