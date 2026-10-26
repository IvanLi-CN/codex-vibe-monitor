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
        self.assertIn("FROM ${RUNTIME_CONTEXT} AS production-runtime", dockerfile)
        self.assertIn("FROM ${RUST_CONTEXT} AS production-rust-context", dockerfile)
        self.assertIn("FROM ${WEB_CONTEXT} AS production-web-context", dockerfile)
        self.assertIn("COPY --from=production-rust-context", dockerfile)
        self.assertIn("COPY --from=production-web-context", dockerfile)
        self.assertIn("AS rust-builder-canonical", dockerfile)
        self.assertIn("AS web-builder-canonical", dockerfile)
        self.assertIn("ARG RUST_BASE=rust:1.96.0-bookworm", dockerfile.split("FROM", 1)[0])
        self.assertIn("ARG RUNTIME_BASE=debian:bookworm-slim", dockerfile.split("FROM", 1)[0])

    def test_build_identity_includes_web_compiler_inputs(self) -> None:
        identity = next(
            step for step in self.workflow["jobs"]["observability-performance-resolve"]["steps"]
            if step["name"] == "Bind candidate and build identity"
        )
        run = identity["run"]
        for path in ("web/tsconfig.json", "web/tsconfig.app.json", "web/tsconfig.node.json"):
            self.assertIn(path, run)

    def test_cache_save_results_are_reported(self) -> None:
        workflow = WORKFLOW_PATH.read_text()
        self.assertIn("id: rust-cache-save", workflow)
        self.assertIn("id: rust-target-cache-save", workflow)
        self.assertIn("id: web-cache-save", workflow)
        self.assertIn("rust_deps_cache_key", workflow)
        self.assertIn("rust_target_cache_key", workflow)
        self.assertIn("web_deps_cache_key", workflow)
        self.assertIn("Finalize Rust cache report", workflow)
        self.assertIn("Finalize Web cache report", workflow)
        self.assertNotIn("'save':'best-effort'", workflow)

    def test_auto_dag_has_three_required_producers_and_complete_image_join(self) -> None:
        producer = self.workflow["jobs"]["observability-performance-producer"]
        self.assertEqual(producer["strategy"]["matrix"]["component"], ["rust", "web", "runtime-base"])
        self.assertIn("complete_image_cache_hit != 'true'", producer["if"])
        image = self.workflow["jobs"]["observability-performance-image"]
        self.assertIn("complete_image_cache_hit == 'true'", image["if"])
        auto = next(step for step in image["steps"] if step["name"] == "Assemble production image from verified contexts")
        contexts = auto["with"]["build-contexts"]
        self.assertIn("rust-builder=${{ runner.temp }}/", contexts)
        self.assertIn("web-builder=${{ runner.temp }}/", contexts)
        self.assertIn("runtime-base=oci-layout://", contexts)

    def test_cache_write_failure_is_non_blocking_but_transfer_is_required(self) -> None:
        broken = copy.deepcopy(self.workflow)
        auto = next(step for step in broken["jobs"]["observability-performance-image"]["steps"] if step["name"] == "Assemble production image from verified contexts")
        auto["with"]["cache-to"] = auto["with"]["cache-to"].replace(",ignore-error=true", "")
        with self.assertRaises(contract.ContractError):
            contract.require_observability_performance_contract(broken)

    def test_complete_image_cache_uses_the_same_path_for_restore_and_save(self) -> None:
        resolver = self.workflow["jobs"]["observability-performance-resolve"]
        restore = next(step for step in resolver["steps"] if step["name"] == "Restore complete candidate image cache")
        verify = next(step for step in resolver["steps"] if step["name"] == "Verify complete candidate image cache identity")
        upload = next(step for step in resolver["steps"] if step["name"] == "Upload cached candidate image for assembly")
        image = self.workflow["jobs"]["observability-performance-image"]
        save = next(step for step in image["steps"] if step["name"] == "Save complete candidate image cache")
        self.assertEqual(restore["with"]["path"], "${{ runner.temp }}/observability-image")
        self.assertIn("$RUNNER_TEMP/observability-image", verify["run"])
        self.assertEqual(upload["with"]["path"], "${{ runner.temp }}/observability-image/")
        self.assertEqual(save["with"]["path"], "${{ runner.temp }}/observability-image")
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
                "cache": {"restore": "miss", "save": "saved", "restoreSeconds": 1.5, "saveSeconds": 2.5},
                "artifact": "component.tar.gz",
                "archiveSha256": hashlib.sha256(b"component").hexdigest(),
            }))
            image_dir = root / "image"
            image_dir.mkdir()
            image_archive = image_dir / "candidate.tar.gz"
            image_archive.write_bytes(b"image")
            (image_dir / "image.json").write_text(json.dumps({
                "schema": "cvm-performance-build-image-v1",
                "component": "image",
                "candidate": "a" * 40,
                "buildIdentity": "b" * 64,
                "status": "ready",
                "startedAt": "2026-10-25T00:01:15Z",
                "finishedAt": "2026-10-25T00:01:45Z",
                "cache": {"status": "hit"},
                "transfer": {"archiveBytes": 12},
                "imageId": "sha256:image",
                "artifact": "candidate.tar.gz",
                "archiveSha256": hashlib.sha256(b"image").hexdigest(),
            }))
            output = root / "build-report.json"
            jobs_json = root / "actions-jobs.json"
            jobs_json.write_text(json.dumps({
                "jobs": [
                    {
                        "databaseId": 101,
                        "name": "Observability Performance Producer (rust)",
                        "status": "completed",
                        "conclusion": "success",
                        "createdAt": "2026-10-25T00:00:11Z",
                        "startedAt": "2026-10-25T00:00:12Z",
                        "completedAt": "2026-10-25T00:01:12Z",
                    },
                    {
                        "databaseId": 102,
                        "name": "Observability Performance Image",
                        "status": "completed",
                        "conclusion": "success",
                        "createdAt": "2026-10-25T00:01:13Z",
                        "startedAt": "2026-10-25T00:01:14Z",
                        "completedAt": "2026-10-25T00:01:46Z",
                    },
                ]
            }))
            import sys
            old_argv = sys.argv
            try:
                sys.argv = ["observability-build-report.py", "--input", str(root), "--output", str(output), "--candidate", "a" * 40, "--build-identity", "b" * 64, "--build-path", "auto", "--run-started", "2026-10-25T00:00:00Z", "--jobs-json", str(jobs_json), "--repository", "IvanLi-CN/codex-vibe-monitor", "--run-id", "123"]
                report.main()
            finally:
                sys.argv = old_argv
            value = json.loads(output.read_text())
            self.assertEqual(value["schema"], "cvm-performance-build-report-v1")
            self.assertEqual(value["buildIdentity"], "b" * 64)
            self.assertEqual(value["cache"]["namespace"], "observability-performance-v2")
            self.assertEqual(value["status"], "passed")
            self.assertGreater(value["timing"]["imageReadySeconds"], 0)
            self.assertEqual(value["timing"]["queueSeconds"], 2.0)
            self.assertEqual(value["timing"]["runnerUsageSeconds"], 92.0)
            self.assertEqual(value["timing"]["cacheRestoreSeconds"], 1.5)
            self.assertEqual(value["timing"]["cacheSaveSeconds"], 2.5)
            self.assertEqual(value["timing"]["transferBytes"], 12.0)
            self.assertEqual(value["jobs"][0]["actions"]["databaseId"], 101)
            self.assertEqual(value["jobs"][0]["executionSeconds"], 60.0)

            jobs = json.loads(jobs_json.read_text())
            for job in jobs["jobs"]:
                job["createdAt"] = None
            jobs_json.write_text(json.dumps(jobs))
            try:
                sys.argv = ["observability-build-report.py", "--input", str(root), "--output", str(output), "--candidate", "a" * 40, "--build-identity", "b" * 64, "--build-path", "auto", "--run-started", "2026-10-25T00:00:00Z", "--jobs-json", str(jobs_json), "--repository", "IvanLi-CN/codex-vibe-monitor", "--run-id", "123"]
                report.main()
            finally:
                sys.argv = old_argv
            unavailable_queue = json.loads(output.read_text())
            self.assertEqual(unavailable_queue["status"], "passed")
            self.assertIsNone(unavailable_queue["timing"]["queueSeconds"])
            self.assertFalse(unavailable_queue["timing"]["queueTimingComplete"])

    def test_build_report_rejects_missing_actions_timestamps(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            component_dir = root / "rust"
            component_dir.mkdir()
            (component_dir / "component.json").write_text(json.dumps({
                "schema": "cvm-performance-build-component-v1",
                "component": "rust",
                "candidate": "a" * 40,
                "buildIdentity": "b" * 64,
                "status": "ready",
                "startedAt": "2026-10-25T00:00:10Z",
                "finishedAt": "2026-10-25T00:01:10Z",
                "elapsedSeconds": 60,
                "cache": {"restore": "miss", "save": "saved"},
                "artifact": "component.tar.gz",
                "archiveSha256": hashlib.sha256(b"component").hexdigest(),
            }))
            (component_dir / "component.tar.gz").write_bytes(b"component")
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
                "imageId": "sha256:image",
                "artifact": "candidate.tar.gz",
                "archiveSha256": hashlib.sha256(b"image").hexdigest(),
            }))
            (image_dir / "candidate.tar.gz").write_bytes(b"image")
            jobs_json = root / "actions-jobs.json"
            jobs_json.write_text(json.dumps({
                "jobs": [
                    {
                        "databaseId": 101,
                        "name": "Observability Performance Producer (rust)",
                        "status": "completed",
                        "conclusion": "success",
                        "createdAt": "2026-10-25T00:00:11Z",
                        "startedAt": "2026-10-25T00:00:12Z",
                        "completedAt": None,
                    },
                    {
                        "databaseId": 102,
                        "name": "Observability Performance Image",
                        "status": "completed",
                        "conclusion": "success",
                        "createdAt": "2026-10-25T00:01:13Z",
                        "startedAt": "2026-10-25T00:01:14Z",
                        "completedAt": "2026-10-25T00:01:46Z",
                    },
                ]
            }))
            import sys
            old_argv = sys.argv
            try:
                sys.argv = [
                    "observability-build-report.py", "--input", str(root), "--output", str(root / "report.json"),
                    "--candidate", "a" * 40, "--build-identity", "b" * 64, "--build-path", "auto",
                    "--run-started", "2026-10-25T00:00:00Z", "--jobs-json", str(jobs_json),
                    "--repository", "IvanLi-CN/codex-vibe-monitor", "--run-id", "123",
                ]
                with self.assertRaisesRegex(ValueError, "Actions job timestamps incomplete"):
                    report.main()
            finally:
                sys.argv = old_argv

    def test_build_report_rejects_failed_actions_job(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            image_dir = root / "image"
            image_dir.mkdir()
            (image_dir / "image.json").write_text(json.dumps({
                "schema": "cvm-performance-build-image-v1",
                "component": "image",
                "candidate": "a" * 40,
                "buildIdentity": "b" * 64,
                "status": "ready",
                "startedAt": "2026-10-25T00:00:10Z",
                "finishedAt": "2026-10-25T00:00:20Z",
                "cache": {"status": "hit"},
                "artifact": "candidate.tar.gz",
                "archiveSha256": hashlib.sha256(b"image").hexdigest(),
            }))
            (image_dir / "candidate.tar.gz").write_bytes(b"image")
            jobs_json = root / "actions-jobs.json"
            jobs_json.write_text(json.dumps({"jobs": [{
                "databaseId": 101,
                "name": "Observability Performance Image",
                "status": "completed",
                "conclusion": "failure",
                "createdAt": "2026-10-25T00:00:11Z",
                "startedAt": "2026-10-25T00:00:12Z",
                "completedAt": "2026-10-25T00:00:20Z",
            }]}))
            import sys
            old_argv = sys.argv
            try:
                sys.argv = [
                    "observability-build-report.py", "--input", str(root), "--output", str(root / "report.json"),
                    "--candidate", "a" * 40, "--build-identity", "b" * 64, "--build-path", "auto",
                    "--run-started", "2026-10-25T00:00:00Z", "--jobs-json", str(jobs_json),
                    "--repository", "IvanLi-CN/codex-vibe-monitor", "--run-id", "123",
                ]
                with self.assertRaisesRegex(ValueError, "did not complete successfully"):
                    report.main()
            finally:
                sys.argv = old_argv

    def test_build_report_accepts_complete_image_cache_hit_without_producer_cards(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            image_dir = root / "image"
            image_dir.mkdir()
            (image_dir / "image.json").write_text(json.dumps({
                "schema": "cvm-performance-build-image-v1",
                "component": "image",
                "candidate": "a" * 40,
                "buildIdentity": "b" * 64,
                "status": "ready",
                "startedAt": "2026-10-25T00:00:10Z",
                "finishedAt": "2026-10-25T00:00:20Z",
                "cache": {"status": "hit"},
                "artifact": "candidate.tar.gz",
                "archiveSha256": hashlib.sha256(b"image").hexdigest(),
            }))
            (image_dir / "candidate.tar.gz").write_bytes(b"image")
            jobs_json = root / "actions-jobs.json"
            jobs_json.write_text(json.dumps({"jobs": [{
                "databaseId": 101,
                "name": "Observability Performance Image",
                "status": "completed",
                "conclusion": "success",
                "createdAt": None,
                "startedAt": "2026-10-25T00:00:12Z",
                "completedAt": "2026-10-25T00:00:20Z",
            }]}))
            output = root / "report.json"
            import sys
            old_argv = sys.argv
            try:
                sys.argv = [
                    "observability-build-report.py", "--input", str(root), "--output", str(output),
                    "--candidate", "a" * 40, "--build-identity", "b" * 64, "--build-path", "auto",
                    "--run-started", "2026-10-25T00:00:00Z", "--jobs-json", str(jobs_json),
                    "--repository", "IvanLi-CN/codex-vibe-monitor", "--run-id", "123",
                ]
                report.main()
            finally:
                sys.argv = old_argv
            value = json.loads(output.read_text())
            self.assertEqual(value["status"], "passed")
            self.assertEqual(value["cache"]["image"]["status"], "hit")
            self.assertEqual([job["component"] for job in value["jobs"]], ["image"])
            self.assertIsNone(value["timing"]["cacheRestoreSeconds"])
            self.assertIsNone(value["timing"]["cacheSaveSeconds"])
            self.assertFalse(value["timing"]["cacheTimingComplete"])

    def test_build_report_preserves_workflow_failure_as_failed_report(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            jobs_json = root / "actions-jobs.json"
            jobs_json.write_text(json.dumps({"jobs": []}))
            output = root / "report.json"
            import sys
            old_argv = sys.argv
            try:
                sys.argv = [
                    "observability-build-report.py", "--input", str(root), "--output", str(output),
                    "--candidate", "a" * 40, "--build-identity", "b" * 64, "--build-path", "auto",
                    "--run-started", "2026-10-25T00:00:00Z", "--jobs-json", str(jobs_json),
                    "--repository", "IvanLi-CN/codex-vibe-monitor", "--run-id", "123",
                    "--workflow-failure",
                ]
                report.main()
            finally:
                sys.argv = old_argv
            value = json.loads(output.read_text())
            self.assertEqual(value["status"], "failed")
            self.assertIn("workflow dependency result was not successful", value["validationErrors"])

    def test_build_report_writes_failure_when_input_directory_is_missing(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            input_dir = root / "missing-input"
            jobs_json = root / "actions-jobs.json"
            jobs_json.write_text(json.dumps({"jobs": []}))
            output = root / "report.json"
            import sys
            old_argv = sys.argv
            try:
                sys.argv = [
                    "observability-build-report.py", "--input", str(input_dir), "--output", str(output),
                    "--candidate", "a" * 40, "--build-identity", "b" * 64, "--build-path", "auto",
                    "--run-started", "2026-10-25T00:00:00Z", "--jobs-json", str(jobs_json),
                    "--repository", "IvanLi-CN/codex-vibe-monitor", "--run-id", "123",
                    "--workflow-failure",
                ]
                report.main()
            finally:
                sys.argv = old_argv
            value = json.loads(output.read_text())
            self.assertEqual(value["status"], "failed")
            self.assertIn("workflow dependency result was not successful", value["validationErrors"])

    def test_canonical_build_only_treats_skipped_producers_as_expected(self) -> None:
        report_step = next(
            step for step in self.workflow["jobs"]["observability-performance-build-report"]["steps"]
            if step["name"] == "Generate build report"
        )
        run = report_step["run"]
        self.assertIn(
            'elif [ "$BUILD_PATH" = "auto" ] && [ "$PRODUCER_RESULT" != "success" ]',
            run,
        )

    def test_candidate_branch_and_sha_pinning_uses_fetch_head(self) -> None:
        workflow = WORKFLOW_PATH.read_text()
        self.assertGreaterEqual(workflow.count("git -C candidate fetch --no-tags --force origin"), 5)
        self.assertGreaterEqual(workflow.count("git -C candidate checkout --detach FETCH_HEAD"), 5)
        self.assertIn("git -C candidate-report checkout --detach FETCH_HEAD", workflow)
        self.assertNotIn('checkout --detach "$CANDIDATE"', workflow)
        self.assertNotIn('checkout --detach "$CANDIDATE_REF"', workflow)

    def test_build_only_workflow_definition_is_trusted_main_only(self) -> None:
        resolver = next(
            step for step in self.workflow["jobs"]["observability-performance-resolve"]["steps"]
            if step["name"] == "Resolve performance sources"
        )
        run = resolver["run"]
        self.assertIn('if [ "$WORKFLOW_REF" != "refs/heads/main" ]; then', run)
        self.assertIn('echo "workflow must be dispatched from trusted main"', run)
        self.assertIn("exit 1", run)
        mutated = run.replace(
            'if [ "$WORKFLOW_REF" != "refs/heads/main" ]; then',
            'if [ "$WORKFLOW_REF" != "refs/heads/not-main" ]; then',
        )
        workflow = copy.deepcopy(self.workflow)
        resolver = next(
            step for step in workflow["jobs"]["observability-performance-resolve"]["steps"]
            if step["name"] == "Resolve performance sources"
        )
        resolver["run"] = mutated
        with self.assertRaises(contract.ContractError):
            contract.require_observability_performance_contract(workflow)

    def test_build_report_rejects_corrupt_component_archive(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            component_dir = root / "rust"
            component_dir.mkdir()
            archive = component_dir / "component.tar.gz"
            archive.write_bytes(b"corrupt")
            (component_dir / "component.json").write_text(json.dumps({
                "schema": "cvm-performance-build-component-v1",
                "component": "rust",
                "candidate": "a" * 40,
                "buildIdentity": "b" * 64,
                "status": "ready",
                "artifact": "component.tar.gz",
                "archiveSha256": hashlib.sha256(b"expected").hexdigest(),
            }))
            jobs_json = root / "actions-jobs.json"
            jobs_json.write_text(json.dumps({"jobs": []}))
            import sys
            old_argv = sys.argv
            try:
                sys.argv = [
                    "observability-build-report.py", "--input", str(root), "--output", str(root / "report.json"),
                    "--candidate", "a" * 40, "--build-identity", "b" * 64, "--build-path", "auto",
                    "--run-started", "2026-10-25T00:00:00Z", "--jobs-json", str(jobs_json),
                    "--repository", "IvanLi-CN/codex-vibe-monitor", "--run-id", "123",
                ]
                with self.assertRaisesRegex(ValueError, "archive hash mismatch"):
                    report.main()
            finally:
                sys.argv = old_argv


if __name__ == "__main__":
    unittest.main()
