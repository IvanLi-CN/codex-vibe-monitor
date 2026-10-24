#!/usr/bin/env python3
"""Deterministic unit checks for benchmark aggregation and validity rules."""

import importlib.util
import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch


MODULE_PATH = Path(__file__).with_name("benchmark-web-tests.py")
SPEC = importlib.util.spec_from_file_location("benchmark_web_tests", MODULE_PATH)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = MODULE
SPEC.loader.exec_module(MODULE)


class BenchmarkAggregationTests(unittest.TestCase):
    def test_main_rejects_non_github_actions_execution(self):
        with patch.dict(MODULE.os.environ, {"GITHUB_ACTIONS": "false"}, clear=False):
            with self.assertRaisesRegex(SystemExit, "must run in GitHub Actions"):
                MODULE.main()

    def test_median_and_mad_are_stable(self):
        self.assertEqual(MODULE.summarize([10.0, 12.0, 11.0]), {
            "count": 3,
            "median_seconds": 11.0,
            "mad_seconds": 1.0,
        })

    def test_empty_summary_has_no_successful_measurement(self):
        self.assertEqual(MODULE.summarize([]), {
            "count": 0,
            "median_seconds": None,
            "mad_seconds": None,
        })

    def test_retry_count_is_read_from_output(self):
        self.assertEqual(MODULE.retry_count("1 retry, 2 retries"), 2)
        self.assertEqual(MODULE.retry_count("1 retry"), 1)
        self.assertEqual(MODULE.retry_count("git fetch origin retry timeout after 429 seconds"), 0)
        self.assertEqual(MODULE.retry_count("Retry #2"), 2)
        self.assertEqual(MODULE.retry_count("all tests passed"), 0)

    def test_frontend_acceptance_requires_the_full_improvement_gate(self):
        metadata = {
            "baseline_sha": "baseline",
            "candidate_sha": "candidate",
            "stage": "frontend",
            "profile": "pr",
            "rounds": 7,
        }
        samples = []
        order = 1
        for version in ("baseline", "candidate"):
            samples.append(
                MODULE.Sample(
                    version, "frontend", "pr", None, "warmup", order, (), "source", version,
                    100.0 if version == "baseline" else 90.0, 1.0, 1, 0, 0, True, "log",
                )
            )
            order += 1
        for pair in range(1, 8):
            for version, wall in (("baseline", 100.0), ("candidate", 90.0)):
                samples.append(
                    MODULE.Sample(
                        version, "frontend", "pr", pair, "warm", order, (), "source", version,
                        wall, 1.0, 1, 0, 0, True, "log",
                    )
                )
                order += 1
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            MODULE.write_summary(output, metadata, samples)
            summary = json.loads((output / "summary.json").read_text())
        self.assertTrue(summary["acceptance"]["passed"])
        self.assertEqual(summary["candidate_faster_pairs"], 7)

    def test_invalid_warm_sample_fails_acceptance(self):
        metadata = {
            "baseline_sha": "baseline",
            "candidate_sha": "candidate",
            "stage": "frontend",
            "profile": "pr",
            "rounds": 7,
        }
        samples = [
            MODULE.Sample(
                version, "frontend", "pr", None, "warmup", index, (), "source", version,
                100.0, 1.0, 1, 0, 0, True, "log",
            )
            for index, version in enumerate(("baseline", "candidate"), start=1)
        ]
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            MODULE.write_summary(output, metadata, samples)
            summary = json.loads((output / "summary.json").read_text())
        self.assertFalse(summary["acceptance"]["passed"])


if __name__ == "__main__":
    unittest.main()
