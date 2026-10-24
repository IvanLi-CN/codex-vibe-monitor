#!/usr/bin/env python3
"""Focused unit checks for the Summary production exactness oracle."""

from __future__ import annotations

import datetime as dt
import importlib.util
import unittest
from pathlib import Path


MODULE_PATH = Path(__file__).with_name("summary-production-exact-oracle.py")
SPEC = importlib.util.spec_from_file_location("summary_exact_oracle", MODULE_PATH)
assert SPEC is not None and SPEC.loader is not None
ORACLE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(ORACLE)


class ExactOracleTimeTests(unittest.TestCase):
    def test_naive_source_timestamp_uses_shanghai(self) -> None:
        parsed = ORACLE.parse_time("2026-01-02 00:01:00")
        self.assertEqual(parsed, dt.datetime(2026, 1, 1, 16, 1, tzinfo=dt.timezone.utc))

    def test_calendar_windows_use_canonical_local_midnight(self) -> None:
        rows = [
            {"id": 1, "occurred_at": "2026-01-02 00:01:00", "status": "success"},
            {"id": 2, "occurred_at": "2026-01-01 23:59:00", "status": "success"},
        ]
        now = dt.datetime(2026, 1, 2, 12, tzinfo=dt.timezone.utc)
        self.assertEqual(ORACLE.expected(rows, "today", now)["totalCount"], 1)
        self.assertEqual(ORACLE.expected(rows, "yesterday", now)["totalCount"], 1)


if __name__ == "__main__":
    unittest.main()
