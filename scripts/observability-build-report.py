#!/usr/bin/env python3
"""Combine build cards and read-only Actions timestamps into the build report."""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import pathlib
from typing import Any


SCHEMA = "cvm-performance-build-report-v1"


def parse_time(value: str | None) -> dt.datetime | None:
    if not value:
        return None
    return dt.datetime.fromisoformat(value.replace("Z", "+00:00"))


def seconds_between(start: str | None, end: str | None) -> float | None:
    first = parse_time(start)
    last = parse_time(end)
    if first is None or last is None:
        return None
    return round((last - first).total_seconds(), 3)


def load_cards(root: pathlib.Path) -> list[dict[str, Any]]:
    cards: list[dict[str, Any]] = []
    for path in sorted(root.rglob("*.json")):
        try:
            value = json.loads(path.read_text())
        except (OSError, json.JSONDecodeError):
            continue
        if isinstance(value, dict) and value.get("schema", "").startswith("cvm-performance-build-"):
            value["_path"] = str(path.relative_to(root))
            cards.append(value)
    return cards


def load_actions_jobs(path: pathlib.Path) -> dict[str, dict[str, Any]]:
    payload = json.loads(path.read_text())
    jobs = payload.get("jobs")
    if not isinstance(jobs, list):
        raise ValueError("Actions jobs payload must contain a jobs list")
    indexed: dict[str, dict[str, Any]] = {}
    for job in jobs:
        if not isinstance(job, dict) or not isinstance(job.get("name"), str):
            continue
        indexed[job["name"]] = job
    return indexed


def actions_job_name(card: dict[str, Any]) -> str:
    component = card.get("component")
    if component in {"rust", "web", "runtime-base"}:
        return f"Observability Performance Producer ({component})"
    return "Observability Performance Image"


def actions_timing(job: dict[str, Any], run_started: str) -> dict[str, Any]:
    started = job.get("startedAt")
    finished = job.get("completedAt")
    return {
        "databaseId": job.get("databaseId", job.get("id")),
        "name": job.get("name"),
        "status": job.get("status"),
        "conclusion": job.get("conclusion"),
        "startedAt": started,
        "completedAt": finished,
        "queueSeconds": seconds_between(run_started, started),
        "executionSeconds": seconds_between(started, finished),
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", required=True, type=pathlib.Path)
    parser.add_argument("--output", required=True, type=pathlib.Path)
    parser.add_argument("--candidate", required=True)
    parser.add_argument("--build-identity", required=True)
    parser.add_argument("--build-path", required=True, choices=("auto", "canonical"))
    parser.add_argument("--run-started", required=True)
    parser.add_argument("--jobs-json", required=True, type=pathlib.Path)
    parser.add_argument("--build-only", action="store_true")
    parser.add_argument("--repository", required=True)
    parser.add_argument("--run-id", required=True)
    args = parser.parse_args()

    cards = load_cards(args.input)
    actions_jobs = load_actions_jobs(args.jobs_json)
    components = {
        card["component"]: card
        for card in cards
        if card.get("schema") == "cvm-performance-build-component-v1"
    }
    image = next(
        (card for card in cards if card.get("schema") == "cvm-performance-build-image-v1"),
        None,
    )
    jobs = []
    for card in [*components.values(), image] if image else list(components.values()):
        action_name = actions_job_name(card)
        action_job = actions_jobs.get(action_name)
        if action_job is None:
            raise ValueError(f"Actions job timestamp missing for {action_name}")
        action = actions_timing(action_job, args.run_started)
        if action["queueSeconds"] is None or action["executionSeconds"] is None:
            raise ValueError(f"Actions job timestamps incomplete for {action_name}")
        card_execution = card.get("elapsedSeconds") or seconds_between(card.get("startedAt"), card.get("finishedAt"))
        jobs.append(
            {
                "component": card.get("component", "image"),
                "runner": card.get("runner"),
                "startedAt": card.get("startedAt"),
                "finishedAt": card.get("finishedAt"),
                "queueSeconds": action["queueSeconds"],
                "executionSeconds": action["executionSeconds"],
                "cardExecutionSeconds": card_execution,
                "actions": action,
                "cache": card.get("cache", {}),
                "transfer": card.get("transfer", {}),
                "status": card.get("status", "unknown"),
            }
        )

    ready_at = image.get("finishedAt") if image else None
    report = {
        "schema": SCHEMA,
        "candidate": args.candidate,
        "controller": {
            "repository": args.repository,
            "runId": args.run_id,
            "runStartedAt": args.run_started,
            "buildOnly": args.build_only,
        },
        "buildIdentity": args.build_identity,
        "buildPath": args.build_path,
        "cache": {
            "namespace": "observability-performance-v2",
            "components": {name: card.get("cache", {}) for name, card in components.items()},
            "image": image.get("cache", {}) if image else {"status": "missing"},
        },
        "jobs": jobs,
        "timing": {
            "runStartedAt": args.run_started,
            "imageReadyAt": ready_at,
            "imageReadySeconds": seconds_between(args.run_started, ready_at),
            "executionSeconds": sum(
                float(job["executionSeconds"])
                for job in jobs
                if job["executionSeconds"] is not None
            ),
            "actionsObserved": True,
            "transfer": image.get("transfer", {}) if image else {"status": "missing"},
        },
        "artifacts": {
            "components": {
                name: {
                    "archiveSha256": card.get("archiveSha256"),
                    "artifact": card.get("artifact"),
                }
                for name, card in components.items()
            },
            "image": {
                "imageId": image.get("imageId") if image else None,
                "archiveSha256": image.get("archiveSha256") if image else None,
            },
        },
        "status": "passed" if image and image.get("status") == "ready" else "incomplete",
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    digest = hashlib.sha256(args.output.read_bytes()).hexdigest()
    print(json.dumps({"report": str(args.output), "sha256": digest, "status": report["status"]}))


if __name__ == "__main__":
    main()
