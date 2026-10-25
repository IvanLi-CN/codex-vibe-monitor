#!/usr/bin/env python3
"""Deterministic contract tests for the reusable performance build path."""

from __future__ import annotations

import copy
import hashlib
import importlib.util
import json
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
CONTRACT_PATH = ROOT / ".github/scripts/check_quality_gates_contract.py"
WORKFLOW_PATH = ROOT / ".github/workflows/ci-observability-performance.yml"
REPORT_PATH = ROOT / "scripts/observability-build-report.py"


def load_module(path: Path, name: str):
    import sys

    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load {path}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


contract = load_module(CONTRACT_PATH, "performance_build_contract")
report = load_module(REPORT_PATH, "performance_build_report")


class PerformanceBuildContractTests(unittest.TestCase):
    def setUp(self) -> None:
        self.workflow = contract.load_yaml(WORKFLOW_PATH)

    def test_shared_recipe_preserves_production_commands(self) -> None:
        recipe = (ROOT / "scripts/production-build.sh").read_text()
        self.assertIn("cargo build --release --locked", recipe)
        self.assertIn("bun install --frozen-lockfile", recipe)
        self.assertIn("bun run build", recipe)
        dockerfile = (ROOT / "Dockerfile").read_text()
        self.assertIn("production-build.sh rust", dockerfile)
        self.assertIn("production-build.sh web", dockerfile)

    def test_auto_dag_has_three_required_producers_and_complete_image_join(self) -> None:
        producer = self.workflow["jobs"]["observability-performance-producer"]
        self.assertEqual(producer["strategy"]["matrix"]["component"], ["rust", "web", "runtime-base"])
        self.assertIn("complete_image_cache_hit != 'true'", producer["if"])
        image = self.workflow["jobs"]["observability-performance-image"]
        self.assertIn("complete_image_cache_hit == 'true'", image["if"])
        auto = next(step for step in image["steps"] if step["name"] == "Assemble production image from verified contexts")
        contexts = auto["with"]["build-contexts"]
        self.assertIn("rust-builder=type=local,src=", contexts)
        self.assertIn("web-builder=type=local,src=", contexts)
        self.assertIn("runtime-base=oci-layout://", contexts)

    def test_cache_write_failure_is_non_blocking_but_transfer_is_required(self) -> None:
        broken = copy.deepcopy(self.workflow)
        auto = next(step for step in broken["jobs"]["observability-performance-image"]["steps"] if step["name"] == "Assemble production image from verified contexts")
        auto["with"]["cache-to"] = auto["with"]["cache-to"].replace(",ignore-error=true", "")
        with self.assertRaises(contract.ContractError):
            contract.require_observability_performance_contract(broken)
        broken = copy.deepcopy(self.workflow)
        upload = next(step for step in broken["jobs"]["observability-performance-image"]["steps"] if step["name"] == "Upload performance candidate image")
        upload["with"]["if-no-files-found"] = "warn"
        with self.assertRaises(contract.ContractError):
            contract.require_observability_performance_contract(broken)

    def test_build_report_schema_contains_measured_cache_and_timing_fields(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            component_dir = root / "rust"
            component_dir.mkdir()
            archive = component_dir / "component.tar.gz"
            archive.write_bytes(b"component")
            (component_dir / "component.json").write_text(json.dumps({
                "schema": "cvm-performance-build-component-v1",
                "component": "rust",
                "candidate": "a" * 40,
                "buildIdentity": "b" * 64,
                "status": "ready",
                "startedAt": "2026-10-25T00:00:10Z",
                "finishedAt": "2026-10-25T00:01:10Z",
                "cache": {"restore": "miss", "save": "best-effort"},
                "archiveSha256": hashlib.sha256(b"component").hexdigest(),
            }))
            image_dir = root / "image"
            image_dir.mkdir()
            (image_dir / "image.json").write_text(json.dumps({
                "schema": "cvm-performance-build-image-v1",
                "component": "image",
                "candidate": "a" * 40,
                "buildIdentity": "b" * 64,
                "status": "ready",
                "startedAt": "2026-10-25T00:01:15Z",
                "finishedAt": "2026-10-25T00:01:45Z",
                "cache": {"status": "assembled"},
                "transfer": {"archiveBytes": 12},
                "imageId": "sha256:image",
                "archiveSha256": "c" * 64,
            }))
            output = root / "build-report.json"
            import sys
            old_argv = sys.argv
            try:
                sys.argv = ["observability-build-report.py", "--input", str(root), "--output", str(output), "--candidate", "a" * 40, "--build-identity", "b" * 64, "--build-path", "auto", "--run-started", "2026-10-25T00:00:00Z", "--repository", "IvanLi-CN/codex-vibe-monitor", "--run-id", "123"]
                report.main()
            finally:
                sys.argv = old_argv
            value = json.loads(output.read_text())
            self.assertEqual(value["schema"], "cvm-performance-build-report-v1")
            self.assertEqual(value["buildIdentity"], "b" * 64)
            self.assertEqual(value["cache"]["namespace"], "observability-performance-v2")
            self.assertEqual(value["status"], "passed")
            self.assertGreater(value["timing"]["imageReadySeconds"], 0)


if __name__ == "__main__":
    unittest.main()