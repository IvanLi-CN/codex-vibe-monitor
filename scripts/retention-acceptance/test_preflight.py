from __future__ import annotations

import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from contract import BASELINE_SHA


WORKFLOW = Path(__file__).parents[2] / ".github/workflows/ci-retention-acceptance.yml"
CANDIDATE_SHA = "a" * 40
PHASE_STARTED_AT = "2026-10-24T12:00:00.000Z"
STAGES = (
    "checkout_candidate",
    "download_baseline",
    "download_candidate",
    "verify_images",
    "measure_acceptance",
)
OUTCOME_ENV = (
    "ACCEPTANCE_OUTCOME_CHECKOUT",
    "ACCEPTANCE_OUTCOME_DOWNLOAD_BASELINE",
    "ACCEPTANCE_OUTCOME_DOWNLOAD_CANDIDATE",
    "ACCEPTANCE_OUTCOME_VERIFY",
    "ACCEPTANCE_OUTCOME_MEASURE",
)
ALLOWED_OUTCOMES = {"success", "failure", "cancelled", "skipped"}


def workflow_python_block(step_name: str) -> str:
    lines = WORKFLOW.read_text().splitlines()
    step_start = lines.index(f"      - name: {step_name}")
    python_start = lines.index("          python3 - <<'PY'", step_start) + 1
    python_end = lines.index("          PY", python_start)
    return "\n".join(line[10:] for line in lines[python_start:python_end])


def execute_workflow_python(step_name: str, environment: dict[str, str]) -> None:
    source = workflow_python_block(step_name)
    with patch.dict(os.environ, environment, clear=True):
        exec(compile(source, f"{WORKFLOW}:{step_name}", "exec"), {})


def initialize_record(directory: Path) -> tuple[Path, dict]:
    step_name = "Start complete test-phase clock and initialize preflight evidence"
    execute_workflow_python(
        step_name,
        {
            "ACCEPTANCE_EVIDENCE_DIR": str(directory),
            "ACCEPTANCE_RUN_ID": "40098586687",
            "ACCEPTANCE_RUN_ATTEMPT": "1",
            "ACCEPTANCE_PR_NUMBER": "1184",
            "ACCEPTANCE_BASELINE_SHA": BASELINE_SHA,
            "ACCEPTANCE_CANDIDATE_SHA": CANDIDATE_SHA,
            "PHASE_STARTED_AT": PHASE_STARTED_AT,
        },
    )
    path = directory / "preflight.json"
    return path, json.loads(path.read_text())


def run_finalizer(directory: Path, outcomes: dict[str, str]) -> None:
    execute_workflow_python(
        "Record preflight stage outcomes",
        {
            "ACCEPTANCE_EVIDENCE_DIR": str(directory),
            **dict(zip(OUTCOME_ENV, (outcomes[name] for name in STAGES), strict=True)),
        },
    )


class PreflightEvidenceTests(unittest.TestCase):
    def test_workflow_inline_scripts_retain_checkout_download_and_verification_failures(self) -> None:
        cases = (
            {
                "checkout_candidate": "failure",
                "download_baseline": "skipped",
                "download_candidate": "skipped",
                "verify_images": "skipped",
                "measure_acceptance": "skipped",
            },
            {
                "checkout_candidate": "success",
                "download_baseline": "failure",
                "download_candidate": "skipped",
                "verify_images": "skipped",
                "measure_acceptance": "skipped",
            },
            {
                "checkout_candidate": "success",
                "download_baseline": "success",
                "download_candidate": "failure",
                "verify_images": "skipped",
                "measure_acceptance": "skipped",
            },
            {
                "checkout_candidate": "success",
                "download_baseline": "success",
                "download_candidate": "success",
                "verify_images": "failure",
                "measure_acceptance": "skipped",
            },
        )
        for outcomes in cases:
            failed_stage = next(name for name, value in outcomes.items() if value == "failure")
            with self.subTest(failed_stage=failed_stage), tempfile.TemporaryDirectory() as temporary:
                directory = Path(temporary)
                preflight_path, initial = initialize_record(directory)
                self.assertEqual(initial["schema_version"], 1)
                self.assertEqual(initial["status"], "incomplete")
                self.assertEqual(
                    initial["identity"],
                    {
                        "run_id": "40098586687",
                        "run_attempt": "1",
                        "pull_request_number": "1184",
                        "baseline_sha": BASELINE_SHA,
                        "candidate_sha": CANDIDATE_SHA,
                        "phase_started_at": PHASE_STARTED_AT,
                    },
                )
                self.assertEqual(
                    initial["stages"],
                    [{"name": name, "outcome": "not_started"} for name in STAGES],
                )
                self.assertEqual(set(initial), {"schema_version", "status", "identity", "stages"})

                run_finalizer(directory, outcomes)

                record = json.loads(preflight_path.read_text())
                self.assertEqual(record["status"], "failed")
                self.assertEqual(
                    record["stages"],
                    [{"name": name, "outcome": outcomes[name]} for name in STAGES],
                )
                self.assertTrue(all(stage["outcome"] in ALLOWED_OUTCOMES for stage in record["stages"]))

    def test_workflow_inline_scripts_mark_a_complete_preflight(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            path, _ = initialize_record(directory)
            run_finalizer(directory, {name: "success" for name in STAGES})
            record = json.loads(path.read_text())
        self.assertEqual(record["status"], "completed")
        self.assertTrue(all(stage["outcome"] == "success" for stage in record["stages"]))

    def test_workflow_finalizer_rejects_unallowlisted_and_out_of_order_outcomes(self) -> None:
        cases = (
            {
                "checkout_candidate": "failure",
                "download_baseline": "success",
                "download_candidate": "skipped",
                "verify_images": "skipped",
                "measure_acceptance": "skipped",
            },
            {
                "checkout_candidate": "success",
                "download_baseline": "success",
                "download_candidate": "success",
                "verify_images": "timed_out",
                "measure_acceptance": "skipped",
            },
        )
        for outcomes in cases:
            with self.subTest(outcomes=outcomes), tempfile.TemporaryDirectory() as temporary:
                directory = Path(temporary)
                path, initial = initialize_record(directory)
                with self.assertRaises(SystemExit):
                    run_finalizer(directory, outcomes)
                self.assertEqual(json.loads(path.read_text()), initial)

    def test_workflow_initializes_before_checkout_and_requires_the_artifact(self) -> None:
        workflow = WORKFLOW.read_text()
        initialize = workflow.index("Start complete test-phase clock and initialize preflight evidence")
        checkout = workflow.index("Checkout exact candidate harness")
        self.assertLess(initialize, checkout)
        for name in (
            "ACCEPTANCE_RUN_ID: ${{ github.run_id }}",
            "ACCEPTANCE_RUN_ATTEMPT: ${{ github.run_attempt }}",
            "ACCEPTANCE_CANDIDATE_SHA: ${{ github.event.pull_request.head.sha }}",
            "RETENTION_ACCEPTANCE_MEASUREMENT_START_UTC=$measurement_started_at",
            "steps.checkout_candidate.outcome",
            "steps.download_baseline.outcome",
            "steps.download_candidate.outcome",
            "steps.verify_images.outcome",
            "steps.measure_acceptance.outcome",
        ):
            self.assertIn(name, workflow)
        self.assertIn("if-no-files-found: error", workflow)
        self.assertNotIn("if-no-files-found: warn", workflow)

    def test_bounded_measurement_window_starts_after_immutable_image_verification(self) -> None:
        workflow = WORKFLOW.read_text()
        verify = workflow.index("Verify and load exact image artifacts")
        start = workflow.index("Start bounded measurement window")
        measure = workflow.index("Run complete 900-second-bounded comparison")
        self.assertLess(verify, start)
        self.assertLess(start, measure)


if __name__ == "__main__":
    unittest.main()
