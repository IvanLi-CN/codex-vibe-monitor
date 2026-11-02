from __future__ import annotations

import unittest
from unittest.mock import patch

from foreground import INVOCATION_RATE, WRITE_PATH, write_sample


class ForegroundTimingTests(unittest.TestCase):
    def test_write_sample_measures_from_planned_arrival_through_completion(self) -> None:
        with (
            patch("foreground.time.monotonic", side_effect=[100.010, 100.030]),
            patch("foreground.response_request", return_value="invocation-0") as request,
            patch("foreground.utc_now", return_value="2026-10-27T00:00:00.030Z"),
        ):
            sample = write_sample(0, 100.0)
        request.assert_called_once_with(0)
        self.assertEqual(sample["sequence"], 0)
        self.assertEqual(sample["invoke_id"], "invocation-0")
        self.assertEqual(sample["path"], WRITE_PATH)
        self.assertEqual(sample["observed_at"], "2026-10-27T00:00:00.030Z")
        self.assertEqual(sample["scheduled_offset_ms"], 0.0)
        self.assertAlmostEqual(sample["dispatch_lag_ms"], 10.0)
        self.assertAlmostEqual(sample["request_elapsed_ms"], 20.0)
        self.assertAlmostEqual(sample["elapsed_ms"], 30.0)

    def test_slow_previous_request_leaves_queueing_in_the_next_sample(self) -> None:
        clock = [100.0]

        def response(sequence: int) -> str:
            clock[0] += 2.0 if sequence == 0 else 0.1
            return f"invocation-{sequence}"

        with (
            patch("foreground.time.monotonic", side_effect=lambda: clock[0]),
            patch("foreground.response_request", side_effect=response),
            patch("foreground.utc_now", return_value="2026-10-27T00:00:02.100Z"),
        ):
            first = write_sample(0, 100.0)
            second = write_sample(1, 100.0)
        self.assertAlmostEqual(first["elapsed_ms"], 2000.0)
        self.assertAlmostEqual(second["scheduled_offset_ms"], 1000 / INVOCATION_RATE)
        self.assertAlmostEqual(second["dispatched_offset_ms"], 2000.0)
        self.assertAlmostEqual(second["completed_offset_ms"], 2100.0)
        self.assertAlmostEqual(second["dispatch_lag_ms"], 2000.0 - 1000 / INVOCATION_RATE)
        self.assertAlmostEqual(second["request_elapsed_ms"], 100.0)
        self.assertAlmostEqual(second["elapsed_ms"], 2100.0 - 1000 / INVOCATION_RATE)
        self.assertGreater(second["elapsed_ms"], second["request_elapsed_ms"])


if __name__ == "__main__":
    unittest.main()
