#!/usr/bin/env python3
"""Export the exact unstripped production binary under its ELF build ID."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True)
    parser.add_argument("--output", required=True)
    parser.add_argument("--revision", required=True)
    args = parser.parse_args()
    if not re.fullmatch(r"[a-f0-9]{40}", args.revision):
        parser.error("revision must be a full Git SHA")
    binary = Path(args.binary).resolve(strict=True)
    notes = subprocess.check_output(["readelf", "-n", str(binary)], text=True)
    sections = subprocess.check_output(["readelf", "-S", str(binary)], text=True)
    match = re.search(r"Build ID: ([a-fA-F0-9]+)", notes)
    if not match or ".debug_line" not in sections:
        raise ValueError("binary must have a build ID and line-table debug information")
    build_id = match[1].lower()
    destination = Path(args.output).resolve() / build_id
    destination.mkdir(parents=True, exist_ok=False)
    shutil.copy2(binary, destination / "codex-vibe-monitor")
    with binary.open("rb") as stream:
        checksum = hashlib.file_digest(stream, "sha256").hexdigest()
    (destination / "manifest.json").write_text(json.dumps({"kind": "cvm-symbols-v1", "revision": args.revision, "buildId": build_id, "sha256": checksum}, indent=2) + "\n")
    print(destination)

if __name__ == "__main__":
    main()
