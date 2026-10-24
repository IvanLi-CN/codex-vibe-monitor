#!/usr/bin/env python3
"""Deterministic unit checks for benchmark aggregation and validity rules."""

import importlib.util
import sys
import unittest
from pathlib import Path


MODULE_PATH = Path(__file__).with_name("benchmark-web-tests.py")
SPEC = importlib.util.spec_from_file_location("benchmark_web_tests", MODULE_PATH)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = MODULE
SPEC.loader.exec_module(MODULE)


class BenchmarkAggregationTests(unittest.TestCase):
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
        self.assertEqual(MODULE.retry_count("all tests passed"), 0)


if __name__ == "__main__":
    unittest.main()
