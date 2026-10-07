#!/usr/bin/env python3
"""Generate isolated retention upgrade fixtures from individual released Linux images."""
import argparse
from importlib.util import module_from_spec, spec_from_file_location
import json
from pathlib import Path
import subprocess

SPEC = spec_from_file_location("ownership_upgrade", Path(__file__).with_name("validate-retention-maintenance-upgrade.py"))
UPGRADE = module_from_spec(SPEC)
SPEC.loader.exec_module(UPGRADE)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-dir", type=Path, required=True)
    args = parser.parse_args()
    args.output_dir.mkdir(parents=True, exist_ok=True)
    for version in UPGRADE.VERSIONS:
        image = f"ghcr.io/ivanli-cn/codex-vibe-monitor:{version}"
        target = args.output_dir.resolve() / version
        if (target / "source.json").exists():
            source = json.loads((target / "source.json").read_text())
            assert source["tag"] == version and source["image"] == image and source["exitCode"] == 0
            assert (target / "business.sqlite").is_file() and (target / "maintenance.sqlite").is_file()
            print(version, "already generated", flush=True)
            continue
        assert not target.exists(), f"Incomplete fixture must be preserved; choose a fresh output directory: {target}"
        subprocess.run(["docker", "pull", image], check=True, timeout=600)
        target.mkdir()
        command = ["docker", "run", "--rm", "--network", "none", "-v", f"{target}:/fixture", "-e", "DATABASE_PATH=/fixture/business.sqlite", "-e", "MAINTENANCE_DATABASE_PATH=/fixture/maintenance.sqlite", "-e", "ARCHIVE_DIR=/fixture/archives", "-e", "PROXY_RAW_DIR=/fixture/raw", "-e", "RETENTION_ENABLED=false", image, "codex-vibe-monitor", "maintenance", "verify-archive-storage", "--dry-run"]
        result = subprocess.run(command, capture_output=True, text=True, timeout=180)
        (target / "generation.log").write_text(result.stdout + result.stderr)
        assert result.returncode == 0, result.stderr[-2000:]
        digest = subprocess.check_output(["docker", "image", "inspect", image, "--format", "{{index .RepoDigests 0}}"], text=True).strip()
        source = {"tag": version, "image": image, "digest": digest, "generator": "released maintenance verify-archive-storage --dry-run", "command": command, "exitCode": result.returncode}
        (target / "source.json").write_text(json.dumps(source, indent=2) + "\n")
        print(version, digest, flush=True)


if __name__ == "__main__":
    main()
