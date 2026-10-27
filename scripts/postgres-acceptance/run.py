#!/usr/bin/env python3
"""Run PostgreSQL acceptance tests without allowing an unavailable service to pass."""

from __future__ import annotations

import argparse
import datetime as dt
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import time
from urllib.parse import urlsplit


BLOCKED_EXIT_CODE = 78
DEFAULT_FIXTURE_VERSION = "4.13.11"


def utc_now() -> str:
    return dt.datetime.now(dt.timezone.utc).isoformat().replace("+00:00", "Z")


def resolve_sha(repo_root: Path, requested: str | None) -> str | None:
    if requested:
        return requested
    try:
        result = subprocess.run(
            ["git", "rev-parse", "--verify", "HEAD"],
            cwd=repo_root,
            check=True,
            capture_output=True,
            text=True,
        )
    except (OSError, subprocess.CalledProcessError):
        return None
    return result.stdout.strip() or None


def redact_text(value: str, database_url: str | None) -> str:
    if database_url:
        value = value.replace(database_url, "<redacted-postgres-url>")
    return value


def redacted_database_url(database_url: str) -> str:
    try:
        parsed = urlsplit(database_url)
        if parsed.scheme not in {"postgres", "postgresql"}:
            return "<configured-database-url>"
        host = parsed.hostname or "<local-socket>"
        port = parsed.port
        port_text = f":{port}" if port else ""
        return f"{parsed.scheme}://{host}{port_text}{parsed.path}"
    except ValueError:
        return "<configured-database-url>"


def probe_postgres(database_url: str) -> dict[str, object]:
    try:
        parsed = urlsplit(database_url)
        if parsed.scheme not in {"postgres", "postgresql"}:
            return {"reachable": False, "reason": "postgres_database_url_invalid_scheme"}
        host = parsed.hostname
        if not host:
            return {
                "reachable": True,
                "method": "command-only",
                "reason": "database URL does not expose a TCP host",
            }
        port = parsed.port or 5432
    except ValueError:
        return {"reachable": False, "reason": "postgres_database_url_invalid"}

    try:
        with socket.create_connection((host, port), timeout=2.0):
            return {"reachable": True, "host": host, "port": port}
    except OSError as error:
        return {
            "reachable": False,
            "host": host,
            "port": port,
            "reason": "postgres_service_unavailable",
            "error_type": type(error).__name__,
        }


def write_json(path: Path, payload: dict[str, object]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def base_report(args: argparse.Namespace, database_url: str | None) -> dict[str, object]:
    repo_root = args.repo_root.resolve()
    return {
        "schema_version": 1,
        "acceptance": "A9",
        "started_at": utc_now(),
        "fixture": {
            "source_version": args.fixture_version,
            "record": "docs/adr/assets/module-owned-postgresql-migration/acceptance-fixtures.json",
        },
        "repository": {
            "base_sha": args.base_sha,
            "candidate_sha": resolve_sha(repo_root, args.candidate_sha),
            "repo_root": str(repo_root),
        },
        "database": {
            "configured": bool(database_url),
            "endpoint": redacted_database_url(database_url) if database_url else None,
        },
        "command": {
            "argv": [redact_text(value, database_url) for value in args.command],
            "cwd": str(repo_root),
            "acceptance_required": True,
        },
    }


def finish_report(
    report: dict[str, object],
    output: Path,
    status: str,
    result: str,
    reason: dict[str, object] | None = None,
    raw_result: dict[str, object] | None = None,
) -> int:
    report["status"] = status
    report["result"] = result
    report["finished_at"] = utc_now()
    if reason is not None:
        report["reason"] = reason
    if raw_result is not None:
        report["raw_result"] = raw_result
    write_json(output, report)
    print(f"postgres_acceptance_status={status}")
    print(f"postgres_acceptance_result={result}")
    print(f"postgres_acceptance_evidence={output}")
    if status == "blocked":
        return BLOCKED_EXIT_CODE
    if raw_result and isinstance(raw_result.get("exit_code"), int):
        return int(raw_result["exit_code"])
    return 1


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--evidence-dir", type=Path, required=True)
    parser.add_argument("--repo-root", type=Path, default=Path.cwd())
    parser.add_argument("--base-sha")
    parser.add_argument("--candidate-sha")
    parser.add_argument("--fixture-version", default=DEFAULT_FIXTURE_VERSION)
    parser.add_argument("--timeout-seconds", type=int, default=1800)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if args.command and args.command[0] == "--":
        args.command = args.command[1:]
    if not args.command:
        parser.error("an acceptance command is required after --")
    return args


def main() -> int:
    args = parse_args()
    args.evidence_dir.mkdir(parents=True, exist_ok=True)
    database_url = os.environ.get("POSTGRES_TEST_DATABASE_URL", "").strip() or None
    report = base_report(args, database_url)

    if not database_url:
        return finish_report(
            report,
            args.output,
            "blocked",
            "not-run",
            {
                "code": "postgres_database_url_missing",
                "message": "POSTGRES_TEST_DATABASE_URL is unset or empty; acceptance was not run",
            },
            {"executed": False},
        )

    probe = probe_postgres(database_url)
    report["database"]["probe"] = probe
    if not probe.get("reachable"):
        return finish_report(
            report,
            args.output,
            "blocked",
            "not-run",
            {
                "code": probe.get("reason", "postgres_service_unavailable"),
                "message": "PostgreSQL service is unavailable; acceptance was not run",
            },
            {"executed": False},
        )

    command_file = args.evidence_dir / "command.txt"
    stdout_file = args.evidence_dir / "stdout.txt"
    stderr_file = args.evidence_dir / "stderr.txt"
    command_file.write_text(
        " ".join(redact_text(value, database_url) for value in args.command) + "\n",
        encoding="utf-8",
    )
    command_env = os.environ.copy()
    command_env["POSTGRES_ACCEPTANCE_REQUIRED"] = "1"
    started = time.monotonic()
    try:
        completed = subprocess.run(
            args.command,
            cwd=args.repo_root,
            env=command_env,
            capture_output=True,
            text=True,
            timeout=args.timeout_seconds,
            check=False,
        )
        stdout = completed.stdout or ""
        stderr = completed.stderr or ""
        exit_code = completed.returncode
    except subprocess.TimeoutExpired as error:
        stdout = error.stdout or ""
        stderr = error.stderr or ""
        exit_code = 124
        report["duration_seconds"] = round(time.monotonic() - started, 3)
        stdout_file.write_text(redact_text(stdout, database_url), encoding="utf-8")
        stderr_file.write_text(redact_text(stderr, database_url), encoding="utf-8")
        return finish_report(
            report,
            args.output,
            "failed",
            "failed",
            {"code": "acceptance_command_timeout", "message": "acceptance command exceeded timeout"},
            {
                "executed": True,
                "exit_code": exit_code,
                "duration_seconds": report["duration_seconds"],
                "command_file": str(command_file),
                "stdout_file": str(stdout_file),
                "stderr_file": str(stderr_file),
            },
        )

    report["duration_seconds"] = round(time.monotonic() - started, 3)
    stdout_file.write_text(redact_text(stdout, database_url), encoding="utf-8")
    stderr_file.write_text(redact_text(stderr, database_url), encoding="utf-8")
    raw_result = {
        "executed": True,
        "exit_code": exit_code,
        "duration_seconds": report["duration_seconds"],
        "command_file": str(command_file),
        "stdout_file": str(stdout_file),
        "stderr_file": str(stderr_file),
    }
    if exit_code == 0:
        return finish_report(report, args.output, "passed", "passed", raw_result=raw_result)
    return finish_report(
        report,
        args.output,
        "failed",
        "failed",
        {"code": "acceptance_command_failed", "message": "acceptance command returned a non-zero exit code"},
        raw_result,
    )


if __name__ == "__main__":
    sys.exit(main())
