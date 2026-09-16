#!/usr/bin/env python3
from pathlib import Path
import argparse
import os


SNAPSHOT_SCHEMA_VERSION = 1
DEFAULT_NOTES_REF = "refs/notes/release-snapshots"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Manage immutable release snapshots stored in git notes.")
    subparsers = parser.add_subparsers(dest="command", required=True)
    ensure = subparsers.add_parser("ensure", help="Create or reuse the immutable snapshot for a main commit.")
    ensure.add_argument("--target-sha", required=True)
    ensure.add_argument("--github-repository", required=True)
    ensure.add_argument("--github-token", required=True)
    ensure.add_argument("--notes-ref", default=DEFAULT_NOTES_REF)
    ensure.add_argument("--registry", default="ghcr.io")
    ensure.add_argument("--api-root", default=os.environ.get("GITHUB_API_URL", "https://api.github.com"))
    ensure.add_argument("--snapshot-source", default="ci-main")
    ensure.add_argument("--output", required=True)
    ensure.add_argument("--max-attempts", type=int, default=6)
    ensure.add_argument("--target-only", action="store_true")
    ensure.add_argument("--skip-publish", action="store_true")
    override = subparsers.add_parser("manual-override", help="Create a job-local release snapshot from explicit workflow_dispatch override inputs.")
    override.add_argument("--target-sha", required=True)
    override.add_argument("--github-repository", required=True)
    override.add_argument("--notes-ref", default=DEFAULT_NOTES_REF)
    override.add_argument("--registry", default="ghcr.io")
    override.add_argument("--version", default="")
    override.add_argument("--bump", default="")
    override.add_argument("--channel", default="stable")
    override.add_argument("--reason", required=True)
    override.add_argument("--actor", required=True)
    override.add_argument("--triggered-at", required=True)
    override.add_argument("--output", required=True)
    export = subparsers.add_parser("export", help="Export a stored release snapshot into GitHub outputs.")
    export.add_argument("--target-sha", required=True)
    export.add_argument("--notes-ref", default=DEFAULT_NOTES_REF)
    export.add_argument("--main-ref", default="")
    export.add_argument("--snapshot-file", default="")
    export.add_argument("--resolve-publication-tags", action="store_true")
    export.add_argument("--github-output", default=os.environ.get("GITHUB_OUTPUT", ""))
    pending = subparsers.add_parser("next-pending", help="Find the newest unreleased snapshot on the first-parent path up to a given main commit.")
    pending.add_argument("--notes-ref", default=DEFAULT_NOTES_REF)
    pending.add_argument("--main-ref", required=True)
    pending.add_argument("--upper-bound", default="")
    pending.add_argument("--github-repository", default="")
    pending.add_argument("--github-token", default="")
    pending.add_argument("--api-root", default=os.environ.get("GITHUB_API_URL", "https://api.github.com"))
    pending.add_argument("--github-output", default=os.environ.get("GITHUB_OUTPUT", ""))
    return parser.parse_args()


implementation_path = (Path(__file__).resolve().parent / ".." / "release_snapshot_impl.py").resolve()
source = implementation_path.read_text(encoding="utf-8")
exec(compile(source, str(implementation_path), "exec"), globals())
