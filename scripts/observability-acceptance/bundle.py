#!/usr/bin/env python3
"""Run all acceptance suites sequentially on one GitHub-hosted runner."""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import subprocess
import sys


LANES = ("runtime", "pair-0", "pair-1", "pair-2")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("source", "candidate-source", "run", "agent", "candidate", "image", "samply"):
        parser.add_argument("--" + name, required=True)
    parser.add_argument("--seconds", type=int, default=60)
    parser.add_argument("--rate", type=int, default=5)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    if args.seconds != 60 or args.rate != 5:
        raise SystemExit("serial performance acceptance requires 60-second windows and 5 request/s")
    root = Path(args.run).resolve()
    root.mkdir(parents=True, exist_ok=True)
    run_script = Path(args.source).resolve() / "scripts/observability-acceptance/run.py"
    outcomes = {}
    for lane in LANES:
        part = root / lane
        part.mkdir(parents=True, exist_ok=True)
        environment = os.environ.copy()
        environment["OBSERVABILITY_LANE"] = lane
        environment["OBSERVABILITY_RUNNER_SCOPE"] = "serial-bundle"
        command = [
            sys.executable,
            str(run_script),
            "--source",
            args.source,
            "--candidate-source",
            args.candidate_source,
            "--run",
            str(part),
            "--agent",
            f"{args.agent}-{lane.replace('-', '')}",
            "--candidate",
            args.candidate,
            "--image",
            args.image,
            "--samply",
            args.samply,
            "--environment",
            "github-actions",
            "--suite",
            lane,
            "--seconds",
            str(args.seconds),
            "--rate",
            str(args.rate),
        ]
        result = subprocess.run(command, env=environment, check=False)
        outcome = "success" if result.returncode == 0 else "failure"
        (part / "part-outcome.json").write_text(json.dumps({"outcome": outcome}) + "\n")
        outcomes[lane] = outcome
    (root / "bundle-outcome.json").write_text(json.dumps(outcomes, indent=2) + "\n")
    return 0 if all(outcome == "success" for outcome in outcomes.values()) else 1


if __name__ == "__main__":
    raise SystemExit(main())
