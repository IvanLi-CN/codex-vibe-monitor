#!/usr/bin/env python3
from __future__ import annotations

import argparse
import importlib.util
import json
import re
import shlex
import shutil
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import Any


class ContractError(RuntimeError):
    pass


@dataclass(frozen=True)
class ContractModel:
    implementation_profile: str
    required_checks: set[str]
    informational_checks: set[str]
    status_check_integrations: dict[str, int]
    review_check_name: str
    review_required_approvals: int
    review_exempt_permissions: set[str]
    review_allowed_permissions: set[str]
    expected_pr_workflows: dict[str, tuple[str, ...]]
    expected_pr_auxiliary_workflows: dict[str, tuple[str, ...]]
    expected_main_workflows: dict[str, tuple[str, ...]]
    expected_main_auxiliary_workflows: dict[str, tuple[str, ...]]
    expected_auxiliary_workflows: dict[str, tuple[str, ...]]
    expected_release_workflows: dict[str, tuple[str, ...]]
    label_check_name: str


CI_PULL_REQUEST_TYPES = {"opened", "reopened", "synchronize", "ready_for_review", "edited"}
LABEL_GATE_PULL_REQUEST_TYPES = {
    "opened",
    "reopened",
    "synchronize",
    "labeled",
    "unlabeled",
    "ready_for_review",
    "edited",
}
REVIEW_POLICY_PULL_REQUEST_TYPES = {"opened", "reopened", "synchronize", "ready_for_review", "edited"}
REVIEW_POLICY_REVIEW_TYPES = {"submitted", "dismissed", "edited"}
RUNNER_X64 = "ubuntu-24.04"
RUNNER_ARM64 = "ubuntu-24.04-arm"
ACTION_CHECKOUT = "actions/checkout@v7"
TEST_CACHE_INPUT_HASH = "${{ hashFiles('Cargo.lock', 'rust-toolchain.toml') }}"
RUST_SOURCE_INPUT_HASH = "${{ hashFiles('Cargo.toml', 'src/**/*.rs', 'vendor/hotpath/**', 'build.rs', '.cargo/config.toml') }}"
CARGO_TEST_CACHE_KEY = "${{ runner.os }}-cargo-test-v4-" + TEST_CACHE_INPUT_HASH + "-" + RUST_SOURCE_INPUT_HASH
CARGO_CLIPPY_CACHE_KEY = "${{ runner.os }}-cargo-clippy-v2-" + TEST_CACHE_INPUT_HASH + "-" + RUST_SOURCE_INPUT_HASH
BUN_DOWNLOAD_CACHE_KEY = "${{ runner.os }}-bun-download-v1-${{ hashFiles('.bun-version', 'bun.lock', 'web/bun.lock', 'docs-site/bun.lock') }}"
PLAYWRIGHT_CACHE_KEY = "${{ runner.os }}-playwright-chromium-v1-${{ hashFiles('.bun-version', 'web/package.json', 'web/bun.lock') }}"
CARGO_NEXTEST_VERSION = "0.9.138"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Validate the codex-vibe-monitor quality-gates contract.")
    parser.add_argument(
        "--repo-root",
        default="",
        help="Repository root containing .github/workflows and the candidate quality-gates files.",
    )
    parser.add_argument(
        "--declaration",
        default="",
        help="Optional trusted quality-gates declaration path. Defaults to <repo-root>/.github/quality-gates.json.",
    )
    parser.add_argument(
        "--metadata-script",
        default="",
        help="Optional trusted metadata_gate.py path. Defaults to <repo-root>/.github/scripts/metadata_gate.py.",
    )
    parser.add_argument(
        "--profile",
        choices=("auto", "bootstrap", "final"),
        default="auto",
        help="Contract profile to validate. Defaults to auto-detect, or final when no repo-root is provided.",
    )
    return parser.parse_args()


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ContractError(message)


def require_backend_partition_command(run: str, partition: str, workflow_name: str) -> None:
    expected_args = [
        "bash",
        ".github/scripts/run-backend-tests.sh",
        "--profile",
        "stateful-sqlite",
        "--partition",
        partition,
        "--archive-dir",
        "$RUNNER_TEMP/backend-test-extract",
    ]
    try:
        actual_args = shlex.split(run)
    except ValueError:
        actual_args = []
    require(
        actual_args == expected_args,
        f"{workflow_name} must run partition {partition}",
    )


def require_playwright_ci_config(workflow_path: Path, workflow_name: str) -> None:
    config_path = workflow_path.parents[2] / "web" / "playwright.ci.config.ts"
    require(config_path.is_file(), f"{workflow_name} must include web/playwright.ci.config.ts")
    config = config_path.read_text(encoding="utf-8")
    required_specs = (
        "records-filter-overlay",
        "dashboard-working-conversations-layout",
        "demo-runtime",
        "retention-maintenance-ownership",
        "dashboard-render-performance",
        "dashboard-timeline-capacity",
    )
    missing_specs = [
        spec
        for spec in required_specs
        if not re.search(rf"(?<![A-Za-z0-9_-]){re.escape(spec)}(?![A-Za-z0-9_-])", config)
    ]
    require(
        not missing_specs,
        f"{workflow_name} web/playwright.ci.config.ts is missing required specs: {', '.join(missing_specs)}",
    )
    require(
        "workers: 1" in config and "fixtureBaseURL" in config and "demoBaseURL" in config,
        f"{workflow_name} web/playwright.ci.config.ts must preserve per-project workers and dedicated base URLs",
    )


def require_observability_performance_contract(workflow: dict[str, Any], workflow_path: str = "ci-observability-performance.yml") -> None:
    require(workflow.get("name") == "Observability Performance", f"{workflow_path}: workflow name drifted")
    require_exact_named_jobs(
        workflow,
        {"Observability Performance Image", "Observability Acceptance (parallel workload lanes)", "Observability Performance Budget"},
        workflow_path,
    )
    on_section = require_mapping(mapping_get(workflow, "on"), f"{workflow_path}.on")
    require(set(on_section) == {"workflow_dispatch"}, f"{workflow_path} must have only the workflow_dispatch trigger")
    dispatch = event_config(workflow, "workflow_dispatch", workflow_path)
    dispatch_inputs = require_mapping(dispatch.get("inputs"), f"{workflow_path}.on.workflow_dispatch.inputs")
    require(set(dispatch_inputs) == {"candidate_ref"}, f"{workflow_path} must expose only the candidate_ref input")
    candidate_input = require_mapping(dispatch_inputs.get("candidate_ref"), f"{workflow_path}.on.workflow_dispatch.inputs.candidate_ref")
    require(candidate_input.get("required") is True and candidate_input.get("type") == "string",
            f"{workflow_path} manual acceptance must require a string candidate_ref")
    permissions = require_mapping(workflow.get("permissions"), f"{workflow_path}.permissions")
    require(permissions.get("contents") == "read", f"{workflow_path}.permissions.contents must stay read")
    performance_job = job_config(workflow, "observability-performance", workflow_path)
    image_job = job_config(workflow, "observability-performance-image", workflow_path)
    resolver = step_config(image_job, "Resolve performance sources", f"{workflow_path}.jobs.observability-performance-image")
    resolver_run = str(resolver.get("run", ""))
    require(
        resolver.get("id") == "refs"
        and 'test "$EVENT_NAME" = "workflow_dispatch"' in resolver_run
        and "WORKFLOW_REF" in resolver_run
        and 'refs/heads/main' in resolver_run
        and 'candidate_repository="$REPOSITORY"' in resolver_run
        and "INPUT_CANDIDATE_REF" in resolver_run
        and 'trusted_ref="$WORKFLOW_SHA"' in resolver_run
        and 'if [ "$WORKFLOW_REF" != "refs/heads/main" ]; then' in resolver_run
        and 'echo "workflow must be dispatched from trusted main"' in resolver_run
        and 'exit 1' in resolver_run
        and "[0-9a-fA-F]{40}" in resolver_run
        and "git check-ref-format --branch" in resolver_run
        and 'candidate_ref="refs/heads/$candidate_input"' in resolver_run
        and "github.event.pull_request" not in resolver_run,
        f"{workflow_path} must resolve and bind the manual trusted/candidate sources",
    )
    image_checkout = checkout_step(image_job, "Checkout performance candidate", f"{workflow_path}.jobs.observability-performance-image")
    require(
        image_checkout.get("repository") == "${{ steps.refs.outputs.candidate_repository }}"
        and image_checkout.get("path") == "candidate"
        and image_checkout.get("ref") == "refs/heads/main"
        and image_checkout.get("fetch-depth") == 0,
        f"{workflow_path} image job must build only the resolved candidate checkout",
    )
    pin_candidate = step_config(image_job, "Pin candidate checkout", f"{workflow_path}.jobs.observability-performance-image")
    require(
        'git -C candidate fetch --no-tags --force origin "$CANDIDATE_REF"' in str(pin_candidate.get("run", ""))
        and "git -C candidate checkout --detach FETCH_HEAD" in str(pin_candidate.get("run", "")),
        f"{workflow_path} image job must explicitly pin the candidate ref",
    )
    identity = step_config(image_job, "Bind candidate identity", f"{workflow_path}.jobs.observability-performance-image")
    identity_run = str(identity.get("run", ""))
    require("git -C candidate rev-parse HEAD" in identity_run and "[0-9a-f]{40}" in identity_run,
            f"{workflow_path} must pin the candidate checkout to its resolved commit")
    for job_id, job_name in [("observability-performance", "performance"), ("observability-performance-result", "aggregation")]:
        job = job_config(workflow, job_id, workflow_path)
        trusted = checkout_step(job, "Checkout trusted acceptance harness", f"{workflow_path}.jobs.{job_id}")
        candidate = checkout_step(job, "Checkout candidate identity", f"{workflow_path}.jobs.{job_id}")
        require(
            trusted.get("repository") == "${{ github.repository }}"
            and trusted.get("ref") == "${{ needs.observability-performance-image.outputs.trusted_ref }}"
            and trusted.get("path") == "trusted",
            f"{workflow_path} {job_name} must run the harness from the resolved trusted checkout",
        )
        require(
            candidate.get("repository") == "${{ needs.observability-performance-image.outputs.candidate_repository }}"
            and candidate.get("ref") == "refs/heads/main"
            and candidate.get("fetch-depth") == 0
            and candidate.get("path") == "candidate",
            f"{workflow_path} {job_name} must use the resolved candidate checkout only for identity",
        )
        pin_candidate = step_config(job, "Pin candidate identity", f"{workflow_path}.jobs.{job_id}")
        require(
            'git -C candidate fetch --no-tags --force origin "$CANDIDATE"' in str(pin_candidate.get("run", ""))
            and "git -C candidate checkout --detach FETCH_HEAD" in str(pin_candidate.get("run", "")),
            f"{workflow_path} {job_name} must explicitly pin the candidate SHA",
        )
    require(performance_job.get("name") == "Observability Acceptance (parallel workload lanes)" and image_job.get("name") == "Observability Performance Image", "observability performance jobs must match their declared names")
    require(performance_job.get("timeout-minutes") == 60 and image_job.get("timeout-minutes") == 15,
            "the image job must finish within 15 minutes and each workload lane within 60 minutes")
    strategy = require_mapping(performance_job.get("strategy"), f"{workflow_path}.jobs.observability-performance.strategy")
    matrix = require_mapping(strategy.get("matrix"), f"{workflow_path}.jobs.observability-performance.strategy.matrix")
    require(
        strategy.get("fail-fast") is False
        and strategy.get("max-parallel") == 4
        and matrix.get("lane") == ["runtime", "pair-0", "pair-1", "pair-2"],
        "performance workload lanes must run concurrently without cancelling sibling evidence",
    )
    concurrency = require_mapping(workflow.get("concurrency"), f"{workflow_path}.concurrency")
    require(
        concurrency.get("group") == "observability-performance-${{ inputs.candidate_ref || github.run_id }}"
        and concurrency.get("cancel-in-progress") is False,
        "performance acceptance runs must not cancel an in-flight candidate measurement",
    )
    clock = step_config(image_job, "Start shared acceptance wall clock", workflow_path)
    require(
        clock.get("id") == "acceptance_clock"
        and str(clock.get("run", "")).strip().splitlines() == ["set -euo pipefail",
            'started="$(date +%s)"',
            'echo "started=$started" >> "$GITHUB_OUTPUT"']
        and clock.get("timeout-minutes") == 1,
        "the image producer must emit one shared acceptance wall-clock start",
    )
    image_outputs = require_mapping(image_job.get("outputs"), f"{workflow_path}.jobs.observability-performance-image.outputs")
    require(
        image_outputs.get("acceptance_started") == "${{ steps.acceptance_clock.outputs.started }}",
        "every workload lane must consume the image producer's shared clock",
    )
    for name in ("Checkout trusted acceptance harness", "Checkout candidate identity", "Download performance candidate image"):
        require(step_config(performance_job, name, workflow_path).get("timeout-minutes") == 1,
                "checkout and artifact actions must fit bounded preparation before the shared deadline")
    for name in ("Verify and load candidate image", "Prepare pinned CPU sampler"):
        require("python3 trusted/scripts/observability-acceptance/deadline.py -- "
                in str(step_config(performance_job, name, workflow_path).get("run", "")),
                "preparation including download retries must share the execution deadline")
    require(performance_job.get("needs") == "observability-performance-image", "performance measurement must consume the separate image producer")
    performance_env = require_mapping(performance_job.get("env"), f"{workflow_path}.jobs.observability-performance.env")
    require(
        performance_env.get("OBSERVABILITY_TEST_STARTED_UTC") == "${{ needs.observability-performance-image.outputs.acceptance_started }}"
        and performance_env.get("OBSERVABILITY_LANE") == "${{ matrix.lane }}"
        and performance_env.get("OBSERVABILITY_RUNNER_SCOPE") == "parallel-${{ matrix.lane }}",
        "each workload lane must bind its shared clock, lane, and isolated runner scope",
    )
    require("if" not in image_job and "if" not in performance_job,
            "observability performance jobs must run only from the workflow dispatch trigger")
    require_job_and_steps_fail_closed(
        performance_job,
        "ci-pr.yml.jobs.observability-performance",
        non_blocking_steps=("Run candidate-bound runtime and performance acceptance",),
    )
    require_job_and_steps_fail_closed(job=image_job, where="ci-pr.yml.jobs.observability-performance-image")
    for name, job in [("observability-performance", performance_job), ("observability-performance-image", image_job)]:
        require(job.get("runs-on") == RUNNER_X64, f"{name} must use the fixed GitHub-hosted runner")
    image_build = uses_step_config(
        image_job,
        "Build exact production image",
        "docker/build-push-action@v7",
        "ci-observability-performance.yml.jobs.observability-performance-image",
    )
    image_build_with = require_mapping(image_build.get("with"), "observability performance image build")
    require(
        "cache-to" not in image_build_with,
        "performance builds must not export to the read-only Actions cache",
    )
    image_upload = uses_step_config(
        image_job,
        "Upload performance candidate image",
        "actions/upload-artifact@v7",
        "ci-pr.yml.jobs.observability-performance-image",
    )
    image_upload_with = require_mapping(
        image_upload.get("with"),
        "ci-pr.yml observability performance image upload",
    )
    require(
        image_upload_with.get("name") == "observability-image-${{ github.run_id }}"
        and image_upload_with.get("overwrite") is True,
        "observability performance image artifact must be run-scoped and replace the prior attempt",
    )
    image_download = uses_step_config(
        performance_job,
        "Download performance candidate image",
        "actions/download-artifact@v7",
        "ci-pr.yml.jobs.observability-performance",
    )
    image_download_with = require_mapping(
        image_download.get("with"),
        "ci-pr.yml observability performance image download",
    )
    require(
        image_download_with.get("name") == "observability-image-${{ github.run_id }}",
        "observability performance must download the run-scoped image artifact",
    )
    acceptance = step_config(performance_job, "Run candidate-bound runtime and performance acceptance", "ci-pr.yml.jobs.observability-performance")
    require(acceptance.get("id") == "acceptance", "performance acceptance step must expose its outcome")
    require(acceptance.get("continue-on-error") is True, "resource-unavailable performance evidence must reach the classifier")
    require("scripts/observability-acceptance/bundle.py" in str(acceptance.get("run", ""))
            and "--source \"$PWD/trusted\"" in str(acceptance.get("run", ""))
            and "--candidate-source \"$PWD/candidate\"" in str(acceptance.get("run", ""))
            and '--lane "$OBSERVABILITY_LANE"' in str(acceptance.get("run", ""))
            and "--seconds 60 --rate 5" in str(acceptance.get("run", "")),
            "performance gate must use the fixed bounded workload lane Actions bundle")
    result_job = job_config(workflow, "observability-performance-result", workflow_path)
    require(result_job.get("name") == "Observability Performance Budget"
            and result_job.get("needs") == ["observability-performance-image", "observability-performance"]
            and result_job.get("if") == "always() && needs.observability-performance-image.result == 'success'"
            and result_job.get("timeout-minutes") == 2 and result_job.get("runs-on") == RUNNER_X64,
            "full certificate must wait for every isolated test part and bounded verification")
    require_job_and_steps_fail_closed(result_job, workflow_path, non_blocking_steps=("Verify and aggregate all parts",))
    aggregate = step_config(result_job, "Verify and aggregate all parts", workflow_path)
    require(aggregate.get("id") == "aggregate" and aggregate.get("continue-on-error") is True
            and 'trusted/scripts/observability-acceptance/aggregate.py' in str(aggregate.get("run", "")),
            "only the trusted all-parts verifier may issue the full certificate")
    classifier = step_config(result_job, "Classify performance acceptance", workflow_path)
    require(classifier.get("if") == "always()", "performance acceptance classifier must run after unavailable evidence")
    classifier_env = require_mapping(classifier.get("env"), "ci-pr.yml performance acceptance classifier environment")
    require(
        classifier_env.get("ACCEPTANCE_OUTCOME") == "${{ steps.aggregate.outcome }}"
        and 'scripts/observability-acceptance/classify.py' in str(classifier.get("run", ""))
        and '--root "$RUNNER_TEMP/observability-acceptance"' in str(classifier.get("run", ""))
        and '--step-outcome "$ACCEPTANCE_OUTCOME"' in str(classifier.get("run", "")),
        "performance acceptance classifier must consume the recorded step outcome and evidence root",
    )
    expected_evidence = {"runtime-card.json", "part-outcome.json", "scenarios.json", "client-failures.json", "ab-samples.json",
                         "run-config.json", "image-identity.json", "runner-context.json", "cpu-attribution.json",
                         "environment-admission.jsonl", "resource-observer.jsonl", "measurement-windows.json",
                         "cpu-verified/*.manifest.json", "cpu-verified/*.json.gz", "cpu-verified/*.syms.json"}
    upload = step_config(performance_job, "Upload workload acceptance evidence", workflow_path)
    require(upload.get("if") == "always()", "failed performance lanes must preserve evidence")
    prefix = "${{ runner.temp }}/observability-acceptance/${{ matrix.lane }}/"
    evidence = str(upload.get("with", {}).get("path", "")).splitlines()
    require({line.strip() for line in evidence if line.strip()} == {prefix + name for name in expected_evidence},
            "performance evidence must include exactly the declared artifact whitelist")
    require(upload.get("with", {}).get("name") == "observability-part-${{ github.run_id }}-${{ github.run_attempt }}-${{ matrix.lane }}",
            "workload evidence must be attempt/lane scoped")
    combined = step_config(result_job, "Upload aggregated acceptance evidence", workflow_path)
    aggregate_prefix = "${{ runner.temp }}/observability-acceptance/"
    require(combined.get("if") == "always()" and set(str(combined.get("with", {}).get("path", "")).splitlines())
            == {aggregate_prefix + name for name in ("empirical-card.json", "scenarios.json", "ab-samples.json", "ab-summary.json")},
            "aggregate evidence must use its exact safe whitelist")


def require_observability_performance_contract(workflow: dict[str, Any], workflow_path: str = "ci-observability-performance.yml") -> None:
    """Validate the performance build DAG and the unchanged measurement contract."""
    require(workflow.get("name") == "Observability Performance", f"{workflow_path}: workflow name drifted")
    expected_jobs = {
        "Observability Performance Resolve",
        "Observability Performance Producer (${{ matrix.component }})",
        "Observability Performance Image",
        "Observability Acceptance (parallel workload lanes)",
        "Observability Performance Budget",
        "Observability Performance Build Report",
    }
    require_exact_named_jobs(workflow, expected_jobs, workflow_path)

    on_section = require_mapping(mapping_get(workflow, "on"), f"{workflow_path}.on")
    require(set(on_section) == {"workflow_dispatch"}, f"{workflow_path} must have only the workflow_dispatch trigger")
    dispatch = event_config(workflow, "workflow_dispatch", workflow_path)
    dispatch_inputs = require_mapping(dispatch.get("inputs"), f"{workflow_path}.on.workflow_dispatch.inputs")
    require(set(dispatch_inputs) == {"candidate_ref", "build_only", "build_path"}, f"{workflow_path} dispatch inputs drifted")
    candidate_input = require_mapping(dispatch_inputs["candidate_ref"], f"{workflow_path}.candidate_ref")
    require(candidate_input.get("required") is True and candidate_input.get("type") == "string", f"{workflow_path} candidate_ref must be required string")
    build_only_input = require_mapping(dispatch_inputs["build_only"], f"{workflow_path}.build_only")
    require(build_only_input.get("required") is True and build_only_input.get("type") == "boolean" and build_only_input.get("default") is False, f"{workflow_path} build_only must default false")
    build_path_input = require_mapping(dispatch_inputs["build_path"], f"{workflow_path}.build_path")
    require(build_path_input.get("required") is True and build_path_input.get("type") == "choice" and build_path_input.get("default") == "auto" and build_path_input.get("options") == ["auto", "canonical"], f"{workflow_path} build_path choices drifted")
    permissions = require_mapping(workflow.get("permissions"), f"{workflow_path}.permissions")
    require(permissions == {"contents": "read", "actions": "read", "checks": "read"}, f"{workflow_path} permissions must allow read-only Actions and check-run timestamps")

    resolve = job_config(workflow, "observability-performance-resolve", workflow_path)
    producer = job_config(workflow, "observability-performance-producer", workflow_path)
    image = job_config(workflow, "observability-performance-image", workflow_path)
    performance = job_config(workflow, "observability-performance", workflow_path)
    result = job_config(workflow, "observability-performance-result", workflow_path)
    report = job_config(workflow, "observability-performance-build-report", workflow_path)

    resolver = step_config(resolve, "Resolve performance sources", f"{workflow_path}.jobs.observability-performance-resolve")
    resolver_run = str(resolver.get("run", ""))
    require(
        resolver.get("id") == "refs"
        and 'test "$EVENT_NAME" = "workflow_dispatch"' in resolver_run
        and "WORKFLOW_REF" in resolver_run
        and 'refs/heads/main' in resolver_run
        and 'candidate_repository="$REPOSITORY"' in resolver_run
        and "INPUT_CANDIDATE_REF" in resolver_run
        and 'trusted_ref="$WORKFLOW_SHA"' in resolver_run
        and 'if [ "$WORKFLOW_REF" != "refs/heads/main" ]; then' in resolver_run
        and 'echo "workflow must be dispatched from trusted main"' in resolver_run
        and 'exit 1' in resolver_run
        and "[0-9a-fA-F]{40}" in resolver_run
        and "git check-ref-format --branch" in resolver_run
        and 'candidate_ref="refs/heads/$candidate_input"' in resolver_run
        and "INPUT_BUILD_ONLY" in resolver_run
        and "INPUT_BUILD_PATH" in resolver_run
        and 'test "$INPUT_BUILD_PATH" = "auto"' in resolver_run
        and 'if [ "$INPUT_BUILD_ONLY" != "true" ]; then' in resolver_run
        and "github.event.pull_request" not in resolver_run,
        f"{workflow_path} must resolve trusted main and same-repository candidate sources",
    )
    resolve_checkout = checkout_step(resolve, "Checkout performance candidate", f"{workflow_path}.jobs.observability-performance-resolve")
    require(
        resolve_checkout.get("repository") == "${{ steps.refs.outputs.candidate_repository }}"
        and resolve_checkout.get("ref") == "refs/heads/main"
        and resolve_checkout.get("fetch-depth") == 0
        and resolve_checkout.get("path") == "candidate",
        f"{workflow_path} resolver checkout drifted",
    )
    pin_candidate = step_config(resolve, "Pin candidate checkout", f"{workflow_path}.jobs.observability-performance-resolve")
    require(
        'git -C candidate fetch --no-tags --force origin "$CANDIDATE_REF"' in str(pin_candidate.get("run", ""))
        and "git -C candidate checkout --detach FETCH_HEAD" in str(pin_candidate.get("run", "")),
        f"{workflow_path} resolver must explicitly pin branch and SHA candidates",
    )
    identity = step_config(resolve, "Bind candidate and build identity", f"{workflow_path}.jobs.observability-performance-resolve")
    identity_run = str(identity.get("run", ""))
    require("git -C candidate rev-parse HEAD" in identity_run and "[0-9a-f]{40}" in identity_run and "cvm-performance-build-v1" in identity_run and all(token in identity_run for token in ("recipe_digest", "build_identity", "rust_cache_key", "rust_deps_cache_key", "rust_target_cache_key", "web_cache_key", "web_deps_cache_key", "runtime_cache_key", "base_image_digests", "trusted_recipe_digest", "rust_image", "web_image", "runtime_image", "def pinned", "cache_digest", 'build_only == "true"')), f"{workflow_path} must bind candidate, pinned base images and build identity")
    require('candidate_recipe_contract_digest == trusted_recipe_digest or build_only == "true"' not in identity_run, f"{workflow_path} build_only must not bypass the trusted recipe fallback")
    outputs = require_mapping(resolve.get("outputs"), f"{workflow_path}.jobs.observability-performance-resolve.outputs")
    require(outputs.get("candidate") == "${{ steps.identity.outputs.candidate }}" and outputs.get("trusted_ref") == "${{ steps.refs.outputs.trusted_ref }}" and outputs.get("effective_build_path") == "${{ steps.identity.outputs.effective_build_path }}" and outputs.get("complete_image_cache_hit") == "${{ steps.complete-image-cache-check.outputs.valid }}", f"{workflow_path} resolver outputs drifted")
    complete_cache = uses_step_config(resolve, "Restore complete candidate image cache", "actions/cache/restore@v4", workflow_path)
    require(
        complete_cache.get("with", {}).get("path") == "${{ runner.temp }}/observability-image"
        and "observability-performance-v2-image-" in str(complete_cache.get("with", {}).get("key", ""))
        and complete_cache.get("continue-on-error") is True,
        f"{workflow_path} complete image cache restore must be isolated and recoverable",
    )
    cache_check = step_config(resolve, "Verify complete candidate image cache identity", workflow_path)
    require(cache_check.get("id") == "complete-image-cache-check" and "archiveSha256" in str(cache_check.get("run", "")) and "buildIdentity" in str(cache_check.get("run", "")), f"{workflow_path} complete image cache must be identity checked")

    require(producer.get("name") == "Observability Performance Producer (${{ matrix.component }})" and producer.get("needs") == "observability-performance-resolve" and "outputs.effective_build_path == 'auto'" in str(producer.get("if", "")) and "complete_image_cache_hit != 'true'" in str(producer.get("if", "")) and producer.get("runs-on") == RUNNER_X64 and producer.get("timeout-minutes") == 15, f"{workflow_path} producer job must be independent and auto-only")
    producer_strategy = require_mapping(producer.get("strategy"), f"{workflow_path}.producer.strategy")
    producer_matrix = require_mapping(producer_strategy.get("matrix"), f"{workflow_path}.producer.strategy.matrix")
    require(producer_strategy.get("fail-fast") is False and producer_matrix.get("component") == ["rust", "web", "runtime-base"], f"{workflow_path} producer matrix drifted")
    for name in ("Restore Rust dependency cache", "Restore Rust target cache", "Restore Bun dependency cache", "Save Rust dependency and target cache", "Save Rust target cache", "Save Bun dependency cache"):
        require(step_config(producer, name, workflow_path).get("continue-on-error") is True, f"{workflow_path} {name} must remain best effort")
    web_restore = step_config(producer, "Restore Bun dependency cache", workflow_path)
    web_save = step_config(producer, "Save Bun dependency cache", workflow_path)
    require(web_restore.get("with", {}).get("path") == "${{ runner.temp }}/cvm-bun" and web_save.get("with", {}).get("path") == "${{ runner.temp }}/cvm-bun", f"{workflow_path} Bun cache must contain downloads only, never candidate node_modules")
    rust_save = step_config(producer, "Save Rust dependency and target cache", workflow_path)
    require(
        rust_save.get("id") == "rust-cache-save"
        and "rust-deps-cache.outputs.cache-hit != 'true'" in str(rust_save.get("if", ""))
        and step_config(producer, "Save Rust target cache", workflow_path).get("id") == "rust-target-cache-save"
        and "rust-target-cache.outputs.cache-hit != 'true'" in str(step_config(producer, "Save Rust target cache", workflow_path).get("if", ""))
        and web_save.get("id") == "web-cache-save"
        and "cache-hit != 'true'" in str(web_save.get("if", "")),
        f"{workflow_path} dependency cache saves must be skipped only after an exact cache hit",
    )
    for name in ("Finalize Rust cache report", "Finalize Web cache report"):
        finalize = step_config(producer, name, workflow_path)
        finalize_env = require_mapping(finalize.get("env"), f"{workflow_path}.{name}.env")
        finalize_run = str(finalize.get("run", ""))
        require(
            "always()" in str(finalize.get("if", ""))
            and "CACHE_SAVE_OUTCOME" in finalize_env
            and '"cache"]["save"]' in finalize_run
            and all(status in finalize_run for status in ("saved", "failed", "skipped", "unknown")),
            f"{workflow_path} {name} must record the actual cache save outcome",
        )
    runtime_producer = step_config(producer, "Build runtime-base production artifact", workflow_path)
    runtime_run = str(runtime_producer.get("run", ""))
    require(
        "--progress=rawjson" in runtime_run
        and "cache-observation.json" in runtime_run
        and "restoreSeconds" in runtime_run
        and "saveSeconds" in runtime_run
        and "failed to export cache" in runtime_run
        and 'save = "saved"' in runtime_run
        and 'save = "unknown"' in runtime_run,
        f"{workflow_path} runtime-base producer must report observed cache export status",
    )
    require(checkout_step(resolve, "Checkout trusted production recipe", workflow_path).get("ref") == "${{ steps.refs.outputs.trusted_ref }}", f"{workflow_path} must compare candidate recipes with the trusted workflow revision")
    require("Resolve production base image digests" in {str(step.get("name")) for step in producer.get("steps", [])} | {str(step.get("name")) for step in resolve.get("steps", [])}, f"{workflow_path} must resolve production base image digests before cache identity")
    for name in ("Build Rust production artifact", "Build Web production artifact", "Build runtime-base production artifact"):
        step = step_config(producer, name, workflow_path)
        require("matrix.component" in str(step.get("if", "")) and "production" in str(step.get("run", "")), f"{workflow_path} {name} must remain required")
    require("$RUST_IMAGE" in str(step_config(producer, "Build Rust production artifact", workflow_path).get("run", "")), f"{workflow_path} Rust producer must use the resolved immutable base image")
    require("$WEB_IMAGE" in str(step_config(producer, "Build Web production artifact", workflow_path).get("run", "")), f"{workflow_path} Web producer must use the resolved immutable base image")
    require("$RUNTIME_IMAGE" in runtime_run, f"{workflow_path} runtime-base producer must use the resolved immutable base image")
    cached_upload = uses_step_config(resolve, "Upload cached candidate image for assembly", "actions/upload-artifact@v7", workflow_path)
    require(cached_upload.get("with", {}).get("name") == "observability-cached-image-${{ github.run_id }}" and cached_upload.get("with", {}).get("overwrite") is True, f"{workflow_path} cached image artifacts must replace the prior attempt")
    producer_upload = uses_step_config(producer, "Upload verified producer artifact", "actions/upload-artifact@v7", workflow_path)
    require(producer_upload.get("with", {}).get("name") == "observability-build-${{ github.run_id }}-${{ matrix.component }}" and producer_upload.get("with", {}).get("overwrite") is True, f"{workflow_path} producer artifacts must be run/component scoped and replace the prior attempt")

    require(image.get("needs") == ["observability-performance-resolve", "observability-performance-producer"] and "always()" in str(image.get("if", "")) and "effective_build_path == 'canonical'" in str(image.get("if", "")) and "complete_image_cache_hit == 'true'" in str(image.get("if", "")) and image.get("runs-on") == RUNNER_X64 and image.get("timeout-minutes") == 15, f"{workflow_path} image join must distinguish canonical skip, complete-image hit and producer failure")
    image_checkout = checkout_step(image, "Checkout candidate assembly sources", f"{workflow_path}.jobs.observability-performance-image")
    require(
        image_checkout.get("repository") == "${{ needs.observability-performance-resolve.outputs.candidate_repository }}"
        and image_checkout.get("ref") == "refs/heads/main"
        and image_checkout.get("fetch-depth") == 0,
        f"{workflow_path} image assembly must use the pinned candidate",
    )
    pin_candidate = step_config(image, "Pin candidate assembly sources", f"{workflow_path}.jobs.observability-performance-image")
    require(
        'git -C candidate fetch --no-tags --force origin "$CANDIDATE"' in str(pin_candidate.get("run", ""))
        and "git -C candidate checkout --detach FETCH_HEAD" in str(pin_candidate.get("run", "")),
        f"{workflow_path} image assembly must explicitly pin the candidate SHA",
    )
    trusted_validator = checkout_step(image, "Checkout trusted archive validator", f"{workflow_path}.jobs.observability-performance-image")
    require(
        trusted_validator.get("repository") == "${{ github.repository }}"
        and trusted_validator.get("ref") == "${{ needs.observability-performance-resolve.outputs.trusted_ref }}"
        and trusted_validator.get("path") == "trusted",
        f"{workflow_path} image assembly must use the trusted archive validator",
    )
    archive_load = step_config(image, "Load and verify producer artifacts", workflow_path)
    require(
        "trusted/scripts/safe-extract-tar.py" in str(archive_load.get("run", ""))
        and "tar -xzf" not in str(archive_load.get("run", "")),
        f"{workflow_path} producer archives must use trusted safe extraction",
    )
    for name in ("Assemble production image from verified contexts", "Build canonical production image"):
        build = uses_step_config(image, name, "docker/build-push-action@v7", workflow_path)
        build_with = require_mapping(build.get("with"), f"{workflow_path}.{name}")
        cache_to = str(build_with.get("cache-to", ""))
        require("mode=max" in cache_to and "ignore-error=true" in cache_to and "timeout=60s" in cache_to, f"{workflow_path} cache writes must be bounded and best effort")
        require("APP_GIT_REVISION" in str(build_with.get("build-args", "")), f"{workflow_path} image must bind APP_GIT_REVISION")
    auto_build_with = require_mapping(uses_step_config(image, "Assemble production image from verified contexts", "docker/build-push-action@v7", workflow_path).get("with"), f"{workflow_path}.auto-build.with")
    contexts = str(auto_build_with.get("build-contexts", ""))
    require(
        "rust-builder=${{ runner.temp }}/" in contexts
        and "web-builder=${{ runner.temp }}/" in contexts
        and "runtime-base=oci-layout://" in contexts,
        f"{workflow_path} auto image must consume verified local and OCI producer contexts",
    )
    auto_args = str(auto_build_with.get("build-args", ""))
    require(
        "RUST_CONTEXT=rust-builder" in auto_args
        and "RUST_SYMBOLS_CONTEXT=rust-symbols" in auto_args
        and "WEB_CONTEXT=web-builder" in auto_args
        and "RUNTIME_CONTEXT=runtime-base" in auto_args,
        f"{workflow_path} auto image must select the verified named contexts",
    )
    canonical_with = require_mapping(uses_step_config(image, "Build canonical production image", "docker/build-push-action@v7", workflow_path).get("with"), f"{workflow_path}.canonical-build.with")
    require("build-contexts" not in canonical_with, f"{workflow_path} canonical path must not consume producer contexts")
    image_upload = uses_step_config(image, "Upload performance candidate image", "actions/upload-artifact@v7", workflow_path)
    require(image_upload.get("with", {}).get("name") == "observability-image-${{ github.run_id }}" and image_upload.get("with", {}).get("overwrite") is True and image_upload.get("with", {}).get("if-no-files-found") == "error", f"{workflow_path} image artifact must be run scoped and required")
    cached_download = uses_step_config(image, "Download verified complete image cache", "actions/download-artifact@v7", workflow_path)
    require(cached_download.get("with", {}).get("name") == "observability-cached-image-${{ github.run_id }}", f"{workflow_path} complete image hit must be transferred to the assembly runner")
    cached_load = step_config(image, "Load and verify complete image cache", workflow_path)
    require(("docker load" in str(cached_load.get("run", "")) or "'docker','load'" in str(cached_load.get("run", ""))) and "buildIdentity" in str(cached_load.get("run", "")), f"{workflow_path} complete image hit must be verified before measurement")
    cache_save = uses_step_config(image, "Save complete candidate image cache", "actions/cache/save@v4", workflow_path)
    require(
        cache_save.get("continue-on-error") is True
        and cache_save.get("with", {}).get("path") == "${{ runner.temp }}/observability-image"
        and "observability-performance-v2-image-" in str(cache_save.get("with", {}).get("key", "")),
        f"{workflow_path} complete image cache save must use the exact restore path and remain isolated/best effort",
    )
    image_cache_report = step_config(image, "Finalize image cache report", workflow_path)
    image_cache_report_run = str(image_cache_report.get("run", ""))
    require(
        "always()" in str(image_cache_report.get("if", ""))
        and "IMAGE_CACHE_SAVE_OUTCOME" in require_mapping(image_cache_report.get("env"), "image cache report env")
        and "saveSeconds" in image_cache_report_run
        and all(status in image_cache_report_run for status in ("saved", "failed", "skipped", "unknown")),
        f"{workflow_path} complete image cache save outcome must be recorded",
    )
    clock = step_config(image, "Start shared acceptance wall clock", workflow_path)
    require(clock.get("id") == "acceptance_clock" and "date +%s" in str(clock.get("run", "")) and "GITHUB_OUTPUT" in str(clock.get("run", "")) and clock.get("timeout-minutes") == 1, f"{workflow_path} image job must emit one shared clock")
    image_outputs = require_mapping(image.get("outputs"), f"{workflow_path}.image.outputs")
    require(image_outputs.get("acceptance_started") == "${{ steps.acceptance_clock.outputs.started }}", f"{workflow_path} workload lanes must consume image clock")

    require(performance.get("name") == "Observability Acceptance (parallel workload lanes)" and performance.get("needs") == ["observability-performance-resolve", "observability-performance-image"] and performance.get("timeout-minutes") == 60 and performance.get("runs-on") == RUNNER_X64, f"{workflow_path} workload job metadata drifted")
    strategy = require_mapping(performance.get("strategy"), f"{workflow_path}.performance.strategy")
    matrix = require_mapping(strategy.get("matrix"), f"{workflow_path}.performance.strategy.matrix")
    require(strategy.get("fail-fast") is False and strategy.get("max-parallel") == 4 and matrix.get("lane") == ["runtime", "pair-0", "pair-1", "pair-2"], f"{workflow_path} workload lanes must remain four parallel units")
    concurrency = require_mapping(workflow.get("concurrency"), f"{workflow_path}.concurrency")
    require(concurrency.get("group") == "observability-performance-${{ inputs.candidate_ref || github.run_id }}" and concurrency.get("cancel-in-progress") is False, f"{workflow_path} concurrency contract drifted")
    performance_env = require_mapping(performance.get("env"), f"{workflow_path}.performance.env")
    require(performance_env.get("OBSERVABILITY_TEST_STARTED_UTC") == "${{ needs.observability-performance-image.outputs.acceptance_started }}" and performance_env.get("OBSERVABILITY_LANE") == "${{ matrix.lane }}" and performance_env.get("OBSERVABILITY_RUNNER_SCOPE") == "parallel-${{ matrix.lane }}", f"{workflow_path} workload lanes must bind shared clock and runner scope")
    for job_id in ("observability-performance", "observability-performance-result"):
        candidate = checkout_step(workflow["jobs"][job_id], "Checkout candidate identity", f"{workflow_path}.jobs.{job_id}")
        trusted = checkout_step(workflow["jobs"][job_id], "Checkout trusted acceptance harness", f"{workflow_path}.jobs.{job_id}")
        require(
            candidate.get("ref") == "refs/heads/main"
            and candidate.get("fetch-depth") == 0
            and trusted.get("ref") == "${{ needs.observability-performance-image.outputs.trusted_ref }}",
            f"{workflow_path} downstream jobs must consume resolver-pinned identities",
        )
        pin_candidate = step_config(workflow["jobs"][job_id], "Pin candidate identity", f"{workflow_path}.jobs.{job_id}")
        require(
            'git -C candidate fetch --no-tags --force origin "$CANDIDATE"' in str(pin_candidate.get("run", ""))
            and "git -C candidate checkout --detach FETCH_HEAD" in str(pin_candidate.get("run", "")),
            f"{workflow_path} downstream jobs must explicitly pin the candidate SHA",
        )
    for name in ("Checkout trusted acceptance harness", "Checkout candidate identity", "Download performance candidate image"):
        require(step_config(performance, name, workflow_path).get("timeout-minutes") == 1, f"{workflow_path} {name} must have a bounded preparation timeout")
    for name in ("Verify and load candidate image", "Prepare pinned CPU sampler"):
        require("python3 trusted/scripts/observability-acceptance/deadline.py -- " in str(step_config(performance, name, workflow_path).get("run", "")), f"{workflow_path} {name} must use the shared deadline")
    require_job_and_steps_fail_closed(performance, f"{workflow_path}.jobs.observability-performance", non_blocking_steps=("Run candidate-bound runtime and performance acceptance",))
    require_job_and_steps_fail_closed(image, f"{workflow_path}.jobs.observability-performance-image", best_effort_steps=("Save complete candidate image cache",))
    acceptance = step_config(performance, "Run candidate-bound runtime and performance acceptance", workflow_path)
    require(acceptance.get("id") == "acceptance" and acceptance.get("continue-on-error") is True and "scripts/observability-acceptance/bundle.py" in str(acceptance.get("run", "")) and "--seconds 60 --rate 5" in str(acceptance.get("run", "")), f"{workflow_path} acceptance must preserve the bounded non-blocking evidence step")
    image_download = uses_step_config(performance, "Download performance candidate image", "actions/download-artifact@v7", workflow_path)
    require(image_download.get("with", {}).get("name") == "observability-image-${{ github.run_id }}", f"{workflow_path} workloads must download the run-scoped image")
    expected_evidence = {"runtime-card.json", "part-outcome.json", "scenarios.json", "client-failures.json", "ab-samples.json", "run-config.json", "image-identity.json", "runner-context.json", "cpu-attribution.json", "environment-admission.jsonl", "resource-observer.jsonl", "measurement-windows.json", "cpu-verified/*.manifest.json", "cpu-verified/*.json.gz", "cpu-verified/*.syms.json"}
    upload = step_config(performance, "Upload workload acceptance evidence", workflow_path)
    prefix = "${{ runner.temp }}/observability-acceptance/${{ matrix.lane }}/"
    require(upload.get("if") == "always()" and {line.strip() for line in str(upload.get("with", {}).get("path", "")).splitlines() if line.strip()} == {prefix + item for item in expected_evidence} and upload.get("with", {}).get("name") == "observability-part-${{ github.run_id }}-${{ github.run_attempt }}-${{ matrix.lane }}", f"{workflow_path} evidence whitelist drifted")

    require(result.get("name") == "Observability Performance Budget" and result.get("needs") == ["observability-performance-resolve", "observability-performance-image", "observability-performance"] and "build_only != 'true'" in str(result.get("if", "")) and result.get("timeout-minutes") == 2 and result.get("runs-on") == RUNNER_X64, f"{workflow_path} budget job metadata drifted")
    require_job_and_steps_fail_closed(result, f"{workflow_path}.jobs.observability-performance-result", non_blocking_steps=("Verify and aggregate all parts",))
    aggregate = step_config(result, "Verify and aggregate all parts", workflow_path)
    require(aggregate.get("id") == "aggregate" and aggregate.get("continue-on-error") is True and "trusted/scripts/observability-acceptance/aggregate.py" in str(aggregate.get("run", "")), f"{workflow_path} aggregate must stay trusted and non-blocking")
    classifier = step_config(result, "Classify performance acceptance", workflow_path)
    require(classifier.get("if") == "always()" and "scripts/observability-acceptance/classify.py" in str(classifier.get("run", "")) and '${{ steps.aggregate.outcome }}' == require_mapping(classifier.get("env"), "classifier.env").get("ACCEPTANCE_OUTCOME"), f"{workflow_path} classifier must consume aggregate outcome")
    combined = step_config(result, "Upload aggregated acceptance evidence", workflow_path)
    aggregate_prefix = "${{ runner.temp }}/observability-acceptance/"
    require(combined.get("if") == "always()" and set(str(combined.get("with", {}).get("path", "")).splitlines()) == {aggregate_prefix + item for item in ("empirical-card.json", "scenarios.json", "ab-samples.json", "ab-summary.json")}, f"{workflow_path} aggregate evidence whitelist drifted")

    require(report.get("name") == "Observability Performance Build Report" and report.get("needs") == ["observability-performance-resolve", "observability-performance-producer", "observability-performance-image"] and "always()" in str(report.get("if", "")) and "needs.observability-performance-resolve.result == 'success'" in str(report.get("if", "")) and "needs.observability-performance-image.result == 'success'" not in str(report.get("if", "")) and report.get("timeout-minutes") == 5, f"{workflow_path} build report job metadata drifted")
    timestamps = step_config(report, "Collect Actions job timestamps", workflow_path)
    timestamp_run = str(timestamps.get("run", ""))
    require(
        "gh run view" in timestamp_run
        and "--json jobs" in timestamp_run
        and "check-runs?per_page=100" in timestamp_run
        and "WORKFLOW_SHA" in timestamp_run
        and "commits/$WORKFLOW_SHA/check-runs" in timestamp_run
        and "created_at" in timestamp_run
        and "started_at" in timestamp_run
        and "completed_at" in timestamp_run
        and "tracked_names" in timestamp_run
        and "Observability Performance Image" in timestamp_run
        and require_mapping(timestamps.get("env"), "Actions timestamp collection env").get("GH_TOKEN") == "${{ github.token }}",
        f"{workflow_path} build report must collect read-only Actions job and check-run timestamps",
    )
    report_step = step_config(report, "Generate build report", workflow_path)
    report_checkout = step_config(report, "Checkout report generator from trusted workflow revision", workflow_path)
    require(
        report_checkout.get("with", {}).get("repository") == "${{ github.repository }}"
        and report_checkout.get("with", {}).get("ref") == "${{ needs.observability-performance-resolve.outputs.trusted_ref }}"
        and report_checkout.get("with", {}).get("path") == "trusted",
        f"{workflow_path} formal build reports must use the trusted generator",
    )
    candidate_report_checkout = uses_step_config(report, "Checkout candidate report generator (build-only)", "actions/checkout@v7", workflow_path)
    require(
        candidate_report_checkout.get("if") == "env.BUILD_ONLY == 'true'"
        and candidate_report_checkout.get("with", {}).get("repository") == "${{ github.repository }}"
        and candidate_report_checkout.get("with", {}).get("ref") == "refs/heads/main"
        and candidate_report_checkout.get("with", {}).get("fetch-depth") == 0
        and candidate_report_checkout.get("with", {}).get("path") == "candidate-report",
        f"{workflow_path} build-only reports must use the candidate-compatible generator only in qualification mode",
    )
    pin_report = step_config(report, "Pin candidate report generator (build-only)", workflow_path)
    require(
        'git -C candidate-report fetch --no-tags --force origin "$CANDIDATE"' in str(pin_report.get("run", ""))
        and "git -C candidate-report checkout --detach FETCH_HEAD" in str(pin_report.get("run", "")),
        f"{workflow_path} build-only report generator must be pinned to the candidate SHA",
    )
    report_run = str(report_step.get("run", ""))
    require(
        "observability-build-report.py" in report_run
        and "report_root" in report_run
        and 'report_root="$PWD/trusted"' in report_run
        and 'report_root="$PWD/candidate-report"' in report_run
        and 'if [ "$BUILD_ONLY" = "true" ]' in report_run
        and "--build-identity" in report_run
        and "--run-started" in report_run
        and "--jobs-json" in report_run
        and "--workflow-failure" in report_run
        and "COMPLETE_IMAGE_CACHE_HIT" in report_run
        and 'BUILD_PATH" = "auto"' in report_run
        and "PRODUCER_RESULT" in report_run,
        f"{workflow_path} build report must bind identity, measured Actions timings and distinguish expected producer skips from failures",
    )
    report_upload = uses_step_config(report, "Upload build report", "actions/upload-artifact@v7", workflow_path)
    require(report_upload.get("with", {}).get("name") == "observability-build-report-${{ github.run_id }}" and report_upload.get("with", {}).get("overwrite") is True and report_upload.get("with", {}).get("retention-days") == 14, f"{workflow_path} build report retention or rerun policy drifted")


def require_pr_smoke_artifact_contract(smoke_job: dict[str, Any], build_job: dict[str, Any]) -> None:
    upload = uses_step_config(
        smoke_job,
        "Upload PR smoke artifacts",
        "actions/upload-artifact@v7",
        "ci-pr.yml.jobs.build-pr-smoke-artifacts",
    )
    upload_with = require_mapping(upload.get("with"), "ci-pr.yml PR smoke artifact upload")
    require(
        upload_with.get("name") == "pr-smoke-artifacts-${{ github.run_id }}"
        and upload_with.get("overwrite") is True,
        "PR smoke artifact must be run-scoped and replace the prior attempt",
    )
    download = uses_step_config(
        build_job,
        "Download PR smoke artifacts",
        "actions/download-artifact@v7",
        "ci-pr.yml.jobs.build",
    )
    download_with = require_mapping(download.get("with"), "ci-pr.yml PR smoke artifact download")
    require(
        download_with.get("name") == "pr-smoke-artifacts-${{ github.run_id }}",
        "Build Artifacts must download the run-scoped PR smoke artifact",
    )


def validate_observability_performance(path: Path) -> None:
    workflow_path = "ci-observability-performance.yml"
    workflow = load_yaml(path)
    require_observability_performance_contract(workflow, workflow_path)


def require_lint_cache_contract(lint_job: dict[str, Any], workflow_name: str) -> None:
    cache = step_config(lint_job, "Restore Cargo Clippy artifacts", f"{workflow_name}.jobs.lint")
    cache_with = require_mapping(cache.get("with"), f"{workflow_name}.jobs.lint Clippy cache")
    expected_key = CARGO_CLIPPY_CACHE_KEY
    require(
        cache.get("uses") == "actions/cache/restore@v5"
        and cache_with.get("path") == "target-clippy"
        and cache_with.get("key") == expected_key
        and str(cache_with.get("restore-keys", "")).strip()
        == "${{ runner.os }}-cargo-clippy-v2-${{ hashFiles('Cargo.lock', 'rust-toolchain.toml') }}-",
        f"{workflow_name}.jobs.lint Clippy cache must be isolated and source-keyed",
    )

    require_exact_cargo_cache_reuse(
        lint_job,
        "Prepare exact Cargo Clippy cache reuse",
        "cargo-clippy-cache",
        f"{workflow_name}.jobs.lint",
    )

    quality = step_config(lint_job, "Rust source quality", f"{workflow_name}.jobs.lint")
    quality_env = require_mapping(quality.get("env"), f"{workflow_name}.jobs.lint Rust source quality environment")
    require(
        quality.get("id") == "rust-source-quality"
        and quality.get("run") == "bash .github/scripts/run-rust-source-quality.sh"
        and quality_env.get("CARGO_BUILD_JOBS") == 8
        and quality_env.get("CARGO_TARGET_DIR") == "target-clippy",
        f"{workflow_name}.jobs.lint must preserve full source checks in the isolated Clippy cache",
    )

    save = step_config(lint_job, "Save Cargo Clippy artifacts", f"{workflow_name}.jobs.lint")
    save_with = require_mapping(save.get("with"), f"{workflow_name}.jobs.lint Clippy cache save")
    require(
        save.get("uses") == "actions/cache/save@v5"
        and save.get("continue-on-error") is True
        and save.get("if")
        == "${{ steps.rust-source-quality.outcome == 'success' && steps.cargo-clippy-cache.outputs.cache-hit != 'true' }}"
        and save_with.get("path") == "target-clippy"
        and save_with.get("key") == expected_key,
        f"{workflow_name}.jobs.lint must save Clippy artifacts best-effort after successful source checks",
    )
    registry_save = step_config(lint_job, "Save Cargo registry", f"{workflow_name}.jobs.lint")
    require(
        registry_save.get("uses") == "actions/cache/save@v5"
        and registry_save.get("continue-on-error") is True,
        f"{workflow_name}.jobs.lint registry cache writes must be best-effort",
    )


def require_fail_closed_stateful_aggregate(run: str, workflow_name: str) -> None:
    expected_lines = [
        "set -euo pipefail",
        'echo "Stateful SQLite shard 1/2: ${SHARD_ONE_RESULT}"',
        'echo "Stateful SQLite shard 2/2: ${SHARD_TWO_RESULT}"',
        'test "${SHARD_ONE_RESULT}" = success',
        'test "${SHARD_TWO_RESULT}" = success',
    ]
    actual_lines = [line.strip() for line in run.splitlines() if line.strip()]
    require(
        actual_lines == expected_lines,
        f"{workflow_name} Stateful SQLite aggregate must preserve the exact fail-closed script",
    )


def load_module(path: Path):
    spec = importlib.util.spec_from_file_location("metadata_gate", path)
    if spec is None or spec.loader is None:
        raise ContractError(f"Unable to load module from {path}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def load_yaml(path: Path) -> dict[str, Any]:
    ruby = (
        "require 'json'; "
        "require 'psych'; "
        "path = ARGV.fetch(0); "
        "data = Psych.safe_load("
        "File.read(path), "
        "permitted_classes: [], "
        "permitted_symbols: [], "
        "aliases: false, "
        "filename: path"
        "); "
        "print JSON.generate(data)"
    )
    result = subprocess.run(
        ["ruby", "-e", ruby, str(path)],
        check=False,
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        raise ContractError(f"{path.name}: unable to parse YAML via ruby: {result.stderr.strip()}")
    payload = json.loads(result.stdout)
    if not isinstance(payload, dict):
        raise ContractError(f"{path.name}: workflow YAML must decode to an object")
    return payload


def mapping_get(mapping: dict[str, Any], key: str, default: Any = None) -> Any:
    if key in mapping:
        return mapping[key]
    if key == "on" and True in mapping:
        return mapping[True]
    if key == "on" and "true" in mapping:
        return mapping["true"]
    return default


def require_mapping(value: Any, where: str) -> dict[str, Any]:
    require(isinstance(value, dict), f"{where} must be an object")
    return value


def require_string_set(value: Any, where: str) -> set[str]:
    require(isinstance(value, list), f"{where} must be a list")
    normalized: set[str] = set()
    for index, item in enumerate(value):
        require(isinstance(item, str) and item, f"{where}[{index}] must be a non-empty string")
        normalized.add(item)
    return normalized


def require_string_collection(value: Any, where: str) -> set[str]:
    require(isinstance(value, (list, tuple, set, frozenset)), f"{where} must be a string collection")
    normalized: set[str] = set()
    for index, item in enumerate(value):
        require(isinstance(item, str) and item, f"{where}[{index}] must be a non-empty string")
        normalized.add(item)
    return normalized


def event_config(workflow: dict[str, Any], event_name: str, where: str) -> dict[str, Any]:
    on_section = require_mapping(mapping_get(workflow, "on"), f"{where}.on")
    config = mapping_get(on_section, event_name)
    require(config is not None, f"{where}.on.{event_name} must be configured")
    if isinstance(config, dict):
        return config
    raise ContractError(f"{where}.on.{event_name} must be an object")


def assert_event_branches(config: dict[str, Any], expected: set[str], where: str) -> None:
    branches = require_string_set(config.get("branches"), f"{where}.branches")
    require(branches == expected, f"{where}.branches drifted: {sorted(branches)}")


def assert_event_types(config: dict[str, Any], expected: set[str], where: str) -> None:
    types = require_string_set(config.get("types"), f"{where}.types")
    require(types == expected, f"{where}.types drifted: {sorted(types)}")


def workflow_jobs(workflow: dict[str, Any], where: str) -> dict[str, Any]:
    return require_mapping(workflow.get("jobs"), f"{where}.jobs")


def job_config(workflow: dict[str, Any], job_id: str, where: str) -> dict[str, Any]:
    jobs = workflow_jobs(workflow, where)
    return require_mapping(jobs.get(job_id), f"{where}.jobs.{job_id}")


def named_job_config(workflow: dict[str, Any], job_id: str, expected_jobs: set[str], where: str) -> dict[str, Any]:
    job = job_config(workflow, job_id, where)
    name = job.get("name")
    require(isinstance(name, str) and name, f"{where}.jobs.{job_id}.name must be a non-empty string")
    require(name in expected_jobs, f"{where}.jobs.{job_id}.name={name!r} must be declared in the contract")
    return job


def workflow_named_job_names(workflow: dict[str, Any], where: str) -> set[str]:
    names: set[str] = set()
    for job_id, raw_job in workflow_jobs(workflow, where).items():
        job = require_mapping(raw_job, f"{where}.jobs.{job_id}")
        name = job.get("name")
        require(isinstance(name, str) and name, f"{where}.jobs.{job_id}.name must be a non-empty string")
        names.add(name)
    return names


def require_exact_named_jobs(workflow: dict[str, Any], expected_jobs: set[str], where: str) -> None:
    jobs = workflow_jobs(workflow, where)
    actual_jobs = workflow_named_job_names(workflow, where)
    require(
        len(jobs) == len(expected_jobs)
        and len(actual_jobs) == len(jobs)
        and actual_jobs == expected_jobs,
        f"{where}: declared jobs drifted count={len(jobs)} expected_count={len(expected_jobs)} "
        f"missing={sorted(expected_jobs - actual_jobs)} unexpected={sorted(actual_jobs - expected_jobs)}",
    )


def step_config(job: dict[str, Any], step_name: str, where: str) -> dict[str, Any]:
    steps = job.get("steps")
    require(isinstance(steps, list), f"{where}.steps must be a list")
    for step in steps:
        if isinstance(step, dict) and step.get("name") == step_name:
            return step
    raise ContractError(f"{where}: missing step {step_name!r}")


def require_step_absent(job: dict[str, Any], step_name: str, where: str) -> None:
    steps = job.get("steps")
    require(isinstance(steps, list), f"{where}.steps must be a list")
    require(
        not any(isinstance(step, dict) and step.get("name") == step_name for step in steps),
        f"{where}: step {step_name!r} must be removed",
    )


def uses_step_config(job: dict[str, Any], step_name: str, expected_uses: str, where: str) -> dict[str, Any]:
    step = step_config(job, step_name, where)
    require(step.get("uses") == expected_uses, f"{where}.steps[{step_name!r}].uses must stay {expected_uses!r}")
    return step


def step_run(step: dict[str, Any], where: str) -> str:
    run = step.get("run")
    require(isinstance(run, str) and run.strip(), f"{where}.run must be a non-empty string")
    return run


def shell_commands(step: dict[str, Any], where: str) -> list[list[str]]:
    commands: list[list[str]] = []
    current = ""
    for raw_line in step_run(step, where).splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#"):
            continue
        if current:
            current = f"{current} {line}"
        else:
            current = line
        if current.endswith("\\"):
            current = current[:-1].rstrip()
            continue
        try:
            tokens = shlex.split(current, posix=True)
        except ValueError as exc:
            raise ContractError(f"{where}.run contains invalid shell syntax: {exc}") from exc
        if tokens:
            commands.append(tokens)
        current = ""
    require(not current, f"{where}.run ends with an unterminated shell continuation")
    require(commands, f"{where}.run must contain at least one shell command")
    return commands


def require_command(step: dict[str, Any], prefix: list[str], where: str, message: str) -> list[str]:
    for command in shell_commands(step, where):
        if command[: len(prefix)] == prefix:
            return command
    raise ContractError(message)


def command_option_map(command: list[str], where: str) -> dict[str, str]:
    options: dict[str, str] = {}
    index = 0
    while index < len(command):
        token = command[index]
        if not token.startswith("--"):
            index += 1
            continue
        require(index + 1 < len(command), f"{where}: option {token} is missing a value")
        options[token] = command[index + 1]
        index += 2
    return options


def checkout_step(job: dict[str, Any], step_name: str, where: str) -> dict[str, Any]:
    step = uses_step_config(job, step_name, ACTION_CHECKOUT, where)
    return require_mapping(step.get("with"), f"{where}.steps[{step_name!r}].with")


def require_no_if(mapping: dict[str, Any], where: str) -> None:
    require("if" not in mapping, f"{where}.if must stay unset")


def require_exact_if(mapping: dict[str, Any], expected: str, where: str) -> None:
    require(mapping.get("if") == expected, f"{where}.if must stay {expected!r}")


def require_bun_download_cache(job: dict[str, Any], where: str) -> None:
    cache = uses_step_config(job, "Cache Bun downloads", "actions/cache@v5", where)
    cache_with = require_mapping(cache.get("with"), f"{where}.steps['Cache Bun downloads'].with")
    require(
        cache_with.get("path") == "~/.bun/install/cache" and cache_with.get("key") == BUN_DOWNLOAD_CACHE_KEY,
        f"{where} must cache Bun downloads with all lockfile and Bun-version inputs",
    )


def require_playwright_browser_cache(job: dict[str, Any], where: str) -> None:
    cache = uses_step_config(job, "Cache Playwright Chromium", "actions/cache@v5", where)
    cache_with = require_mapping(cache.get("with"), f"{where}.steps['Cache Playwright Chromium'].with")
    require(
        cache_with.get("path") == "~/.cache/ms-playwright" and cache_with.get("key") == PLAYWRIGHT_CACHE_KEY,
        f"{where} must cache the version-matched Playwright browser",
    )


def require_bun_caches(workflow: dict[str, Any], workflow_path: str) -> None:
    jobs = workflow_jobs(workflow, workflow_path)
    for job_id, raw_job in jobs.items():
        job = require_mapping(raw_job, f"{workflow_path}.jobs.{job_id}")
        step_names = {step.get("name") for step in job.get("steps", []) if isinstance(step, dict)}
        if "Set up Bun" in step_names:
            require_bun_download_cache(job, f"{workflow_path}.jobs.{job_id}")
        if "Install Playwright Chromium" in step_names or "Install Playwright Chromium for Storybook" in step_names:
            require_playwright_browser_cache(job, f"{workflow_path}.jobs.{job_id}")


def require_fixed_cargo_nextest_install(job: dict[str, Any], where: str) -> None:
    install = uses_step_config(job, "Install cargo-nextest", "taiki-e/install-action@v2", where)
    install_with = require_mapping(install.get("with"), f"{where}.steps['Install cargo-nextest'].with")
    require(
        install_with.get("tool") == f"cargo-nextest@{CARGO_NEXTEST_VERSION}",
        f"{where} must install the fixed cargo-nextest runtime",
    )


def require_backend_archive_prepare(job: dict[str, Any], where: str) -> None:
    prepare = step_config(job, "Prepare backend test archive", where)
    run = str(prepare.get("run", ""))
    required_fragments = (
        'archive_dir="$RUNNER_TEMP/backend-test-extract"',
        'test ! -e "$archive_dir"',
        'mkdir "$archive_dir"',
        'cargo-nextest nextest run \\',
        '--archive-file "$RUNNER_TEMP/backend-tests.tar.zst" \\',
        '--extract-to "$archive_dir" \\',
        '--workspace-remap "$GITHUB_WORKSPACE" \\',
        '--no-run',
    )
    require(
        all(fragment in run for fragment in required_fragments),
        f"{where} must extract the backend archive exactly once before replay",
    )


def require_fail_closed(mapping: dict[str, Any], where: str) -> None:
    require(mapping.get("continue-on-error") in (None, False), f"{where}.continue-on-error must not ignore failures")


def require_job_and_steps_fail_closed(
    job: dict[str, Any],
    where: str,
    best_effort_steps: tuple[str, ...] = (),
    non_blocking_steps: tuple[str, ...] = (),
) -> None:
    require_fail_closed(job, where)
    steps = job.get("steps")
    require(isinstance(steps, list), f"{where}.steps must be a list")
    for index, value in enumerate(steps):
        step = require_mapping(value, f"{where}.steps[{index}]")
        step_where = f"{where}.steps[{step.get('name', index)}]"
        if step.get("name") in best_effort_steps:
            require(step.get("continue-on-error") is True, f"{step_where}.continue-on-error must keep cache writes best-effort")
        elif step.get("name") in non_blocking_steps:
            require(step.get("continue-on-error") is True, f"{step_where}.continue-on-error must preserve unavailable performance evidence for classification")
        else:
            require_fail_closed(step, step_where)


def require_backend_archive_consumer(job: dict[str, Any], workflow_name: str, job_id: str) -> None:
    where = f"{workflow_name}.jobs.{job_id}"
    require_job_and_steps_fail_closed(job, where)
    require_step_absent(job, "Install system dependencies", where)
    require_step_absent(job, "Set up Rust toolchain", where)
    require_fixed_cargo_nextest_install(job, where)
    runtime = step_config(job, "Verify cargo-nextest runtime", where)
    require(
        runtime.get("run") == "cargo-nextest --version | grep -qE '^cargo-nextest 0\\.9\\.138( |$)'",
        f"{where} must verify the fixed cargo-nextest runtime",
    )
    producer_check = step_config(job, "Verify backend test archive producer", where)
    producer_env = require_mapping(producer_check.get("env"), f"{where} producer result environment")
    require(
        producer_env.get("PRODUCER_RESULT") == "${{ needs.backend-test-archive.result }}"
        and producer_check.get("run") == 'test "$PRODUCER_RESULT" = success',
        f"{where} must fail when its backend archive producer fails",
    )
    download = uses_step_config(job, "Download backend test archive", "actions/download-artifact@v7", where)
    download_with = require_mapping(download.get("with"), f"{where} archive download")
    require(
        download_with.get("name") == "backend-test-archive-${{ github.run_id }}",
        f"{where} must download the backend test archive for its workflow run",
    )
    require(
        download_with.get("path") == "${{ runner.temp }}",
        f"{where} must download the backend test archive to runner.temp",
    )
    require_backend_archive_prepare(job, where)

    expected_profiles = {
        "backend-tests-lightweight": (
            "Run lightweight backend profile",
            [
                "bash",
                ".github/scripts/run-backend-tests.sh",
                "--profile",
                "lightweight",
                "--archive-dir",
                "$RUNNER_TEMP/backend-test-extract",
            ],
        ),
        "backend-tests-archive-file-io": (
            "Run archive / file I/O backend profile",
            [
                "bash",
                ".github/scripts/run-backend-tests.sh",
                "--profile",
                "archive-file-io",
                "--archive-dir",
                "$RUNNER_TEMP/backend-test-extract",
            ],
        ),
    }
    if job_id == "backend-tests-representative-scale":
        replay = step_config(job, "Run deterministic representative-scale acceptance", where)
        actual_commands = shell_commands(replay, where)
        expected_commands = [
            [
                "bash", ".github/scripts/run-backend-tests.sh", "--profile", profile,
                "--archive-dir", "$RUNNER_TEMP/backend-test-extract", "--test-filter", selector,
            ]
            for profile, selector in (
                (
                    "stateful-sqlite",
                    "test(=tests::stateful_sqlite::representative_scale_acceptance::summary_representative_scale_acceptance)",
                ),
                (
                    "archive-file-io",
                    "test(=tests::archive_file_io::proxy_runtime_pressure::proxy_runtime_attempt_lifecycle_avoids_sqlite_admission_under_write_pressure) | test(=tests::archive_file_io::proxy_runtime_pressure::proxy_runtime_terminal_commits_survive_retention_and_backfill_contention)",
                ),
            )
        ]
        require(
            actual_commands == expected_commands,
            f"{where} must replay the exact representative-scale profiles from the extracted archive",
        )
        return
    expected_profile = expected_profiles.get(job_id)
    if expected_profile is not None:
        step_name, expected_args = expected_profile
        replay = step_config(job, step_name, where)
        try:
            actual_args = shlex.split(str(replay.get("run", "")))
        except ValueError:
            actual_args = []
        require(
            actual_args == expected_args,
            f"{where} must replay the expected backend profile from the workflow archive",
        )


def require_exact_cargo_cache_reuse(job: dict[str, Any], step_name: str, cache_id: str, where: str) -> None:
    reuse = step_config(job, step_name, where)
    run = str(reuse.get("run", ""))
    require(
        reuse.get("if") == f"${{{{ steps.{cache_id}.outputs.cache-hit == 'true' }}}}"
        and "find src vendor/hotpath -type f -name '*.rs' -exec touch -d '@1' {} +" in run
        and "touch -d '@1' Cargo.toml Cargo.lock rust-toolchain.toml" in run
        and "for input in build.rs .cargo/config.toml; do" in run
        and 'if [ -f "$input" ]; then touch -d \'@1\' "$input"; fi' in run,
        f"{where}: only an exact Rust source/toolchain cache may normalize checkout mtimes",
    )


def require_cargo_test_cache_restore(job: dict[str, Any], where: str) -> None:
    restore = step_config(job, "Restore Cargo test artifacts", where)
    cache_with = require_mapping(restore.get("with"), f"{where}. Cargo test cache")
    require(
        restore.get("id") == "cargo-test-cache"
        and restore.get("uses") == "actions/cache/restore@v5"
        and cache_with.get("path") == "target"
        and cache_with.get("key") == CARGO_TEST_CACHE_KEY
        and str(cache_with.get("restore-keys", "")).strip()
        == "${{ runner.os }}-cargo-test-v4-${{ hashFiles('Cargo.lock', 'rust-toolchain.toml') }}-",
        f"{where}: Cargo test cache must be isolated and keyed by source and toolchain inputs",
    )


def parse_expected_workflows(payload: dict[str, Any], key: str) -> tuple[dict[str, tuple[str, ...]], dict[str, tuple[str, ...]]]:
    raw_expected = payload.get(key)
    require(isinstance(raw_expected, list) and raw_expected, f"quality-gates.json: {key} must be a non-empty array")
    expected: dict[str, tuple[str, ...]] = {}
    auxiliary: dict[str, tuple[str, ...]] = {}
    for index, raw_entry in enumerate(raw_expected):
        entry = require_mapping(raw_entry, f"quality-gates.json.{key}[{index}]")
        workflow_name = entry.get("workflow")
        require(
            isinstance(workflow_name, str) and workflow_name,
            f"quality-gates.json.{key}[{index}].workflow must be a non-empty string",
        )
        jobs = require_string_set(entry.get("jobs"), f"quality-gates.json.{key}[{index}].jobs")
        auxiliary_jobs = require_string_set(
            entry.get("auxiliary_jobs", []), f"quality-gates.json.{key}[{index}].auxiliary_jobs"
        )
        require(
            jobs.isdisjoint(auxiliary_jobs),
            f"quality-gates.json.{key}[{index}]: jobs and auxiliary_jobs must be disjoint",
        )
        require(workflow_name not in expected, f"quality-gates.json: duplicate {key} entry {workflow_name!r}")
        expected[workflow_name] = tuple(sorted(jobs))
        auxiliary[workflow_name] = tuple(sorted(auxiliary_jobs))
    return expected, auxiliary


def validate_quality_gates(payload: dict[str, Any]) -> ContractModel:
    policy = require_mapping(payload.get("policy"), "quality-gates.json.policy")
    branch_policy = require_mapping(policy.get("branch_protection"), "quality-gates.json.policy.branch_protection")
    review_policy = require_mapping(policy.get("review_policy"), "quality-gates.json.policy.review_policy")
    review_enforcement = require_mapping(review_policy.get("enforcement"), "quality-gates.json.policy.review_policy.enforcement")

    require(payload.get("schema_version") == 1, "quality-gates.json: schema_version must be 1")
    implementation_profile = payload.get("implementation_profile")
    require(implementation_profile in {"final", "bootstrap"}, "quality-gates.json: implementation_profile must be 'bootstrap' or 'final'")
    require(policy.get("baseline_policy") == "explicit-waiver-required", "quality-gates.json: baseline_policy drifted")
    require(policy.get("require_signed_commits") is True, "quality-gates.json: require_signed_commits must be true")
    require(branch_policy.get("protected_branches") == ["main"], "quality-gates.json: protected_branches drifted")
    require(branch_policy.get("require_pull_request") is True, "quality-gates.json: require_pull_request must be true")
    require(branch_policy.get("disallow_direct_pushes") is True, "quality-gates.json: disallow_direct_pushes must be true")
    require(branch_policy.get("disallow_branch_deletions") is True, "quality-gates.json: disallow_branch_deletions must be true")
    require(branch_policy.get("disallow_force_pushes") is True, "quality-gates.json: disallow_force_pushes must be true")
    require(branch_policy.get("allow_merge_commits") is True, "quality-gates.json: allow_merge_commits must be true")
    require(branch_policy.get("require_merge_queue") is False, "quality-gates.json: require_merge_queue must be false")
    require(branch_policy.get("required_reviewers") == [], "quality-gates.json: required_reviewers must stay empty")

    status_check_policy = require_mapping(
        branch_policy.get("required_status_checks"),
        "quality-gates.json.policy.branch_protection.required_status_checks",
    )
    require(status_check_policy.get("strict") is True, "quality-gates.json: required_status_checks.strict must be true")
    integrations = require_mapping(
        status_check_policy.get("integrations"),
        "quality-gates.json.policy.branch_protection.required_status_checks.integrations",
    )
    required_checks = require_string_set(payload.get("required_checks"), "quality-gates.json.required_checks")
    informational_checks = require_string_set(payload.get("informational_checks"), "quality-gates.json.informational_checks")
    require(required_checks.isdisjoint(informational_checks), "quality-gates.json: required_checks and informational_checks must be disjoint")
    require(not informational_checks, "quality-gates.json: informational_checks must stay empty")
    require(set(integrations) == required_checks, f"quality-gates.json: required_status_checks.integrations drifted: {sorted(integrations)}")
    normalized_integrations: dict[str, int] = {}
    for context, integration_id in integrations.items():
        require(isinstance(integration_id, int), f"quality-gates.json: required_status_checks.integrations[{context!r}] must be an integer")
        normalized_integrations[context] = integration_id

    require(review_policy.get("mode") == "conditional-required", "quality-gates.json: review_policy.mode drifted")
    review_required_approvals = review_policy.get("required_approvals")
    require(
        isinstance(review_required_approvals, int) and not isinstance(review_required_approvals, bool) and review_required_approvals >= 1,
        "quality-gates.json: review_policy.required_approvals must be a positive integer",
    )
    require(review_policy.get("exempt_repository_owner") is True, "quality-gates.json: exempt_repository_owner must be true")
    review_exempt_permissions = require_string_set(
        review_policy.get("exempt_author_permissions"),
        "quality-gates.json.policy.review_policy.exempt_author_permissions",
    )
    review_allowed_permissions = require_string_set(
        review_policy.get("allowed_reviewer_permissions"),
        "quality-gates.json.policy.review_policy.allowed_reviewer_permissions",
    )
    require(review_enforcement.get("mode") == "required-check", "quality-gates.json: enforcement.mode drifted")
    review_check_name = review_enforcement.get("check_name")
    require(isinstance(review_check_name, str) and review_check_name, "quality-gates.json: enforcement.check_name must be a non-empty string")
    require(review_check_name in required_checks, "quality-gates.json: enforcement.check_name must be required")

    expected_pr_workflows, expected_pr_auxiliary_workflows = parse_expected_workflows(payload, "expected_pr_workflows")
    expected_main_workflows, expected_main_auxiliary_workflows = parse_expected_workflows(payload, "expected_main_workflows")
    expected_auxiliary_workflows, expected_auxiliary_auxiliary_workflows = parse_expected_workflows(payload, "expected_auxiliary_workflows")
    require(
        not any(expected_auxiliary_auxiliary_workflows.values()),
        "quality-gates.json: auxiliary workflows must not declare auxiliary jobs",
    )
    expected_release_workflows, expected_release_auxiliary_workflows = parse_expected_workflows(payload, "expected_release_workflows")
    require(
        not any(expected_release_auxiliary_workflows.values()),
        "quality-gates.json: release workflows must not declare auxiliary jobs",
    )

    declared_pr_jobs = {job for jobs in expected_pr_workflows.values() for job in jobs}
    require(
        declared_pr_jobs == required_checks,
        "quality-gates.json: expected_pr_workflows jobs must exactly cover required_checks",
    )

    label_jobs = set(expected_pr_workflows.get("Label Gate", ()))
    require(label_jobs, "quality-gates.json: expected_pr_workflows must declare Label Gate jobs")
    label_required = sorted(required_checks & label_jobs)
    require(len(label_required) == 1, "quality-gates.json: Label Gate must expose exactly one required check")

    waivers = payload.get("waivers", [])
    require(isinstance(waivers, list), "quality-gates.json: waivers must be an array")
    for index, waiver in enumerate(waivers):
        entry = require_mapping(waiver, f"quality-gates.json.waivers[{index}]")
        require(entry.get("kind") == "bypass-actors-unverified", "quality-gates.json: only bypass-actors-unverified waivers are allowed")
        require(entry.get("branch") == "main", "quality-gates.json: waivers must target main")
        require(isinstance(entry.get("reason"), str) and entry["reason"], "quality-gates.json: waivers must include a non-empty reason")

    return ContractModel(
        implementation_profile=implementation_profile,
        required_checks=required_checks,
        informational_checks=informational_checks,
        status_check_integrations=normalized_integrations,
        review_check_name=review_check_name,
        review_required_approvals=review_required_approvals,
        review_exempt_permissions=review_exempt_permissions,
        review_allowed_permissions=review_allowed_permissions,
        expected_pr_workflows=expected_pr_workflows,
        expected_pr_auxiliary_workflows=expected_pr_auxiliary_workflows,
        expected_main_workflows=expected_main_workflows,
        expected_main_auxiliary_workflows=expected_main_auxiliary_workflows,
        expected_auxiliary_workflows=expected_auxiliary_workflows,
        expected_release_workflows=expected_release_workflows,
        label_check_name=label_required[0],
    )


def validate_metadata_policy(module: Any, contract: ContractModel) -> None:
    require(getattr(module, "REVIEW_REQUIRED_APPROVALS", None) == contract.review_required_approvals, "metadata_gate.REVIEW_REQUIRED_APPROVALS drifted from quality-gates.json")
    require(
        require_string_collection(getattr(module, "REVIEW_EXEMPT_PERMISSIONS", None), "metadata_gate.REVIEW_EXEMPT_PERMISSIONS")
        == contract.review_exempt_permissions,
        "metadata_gate.REVIEW_EXEMPT_PERMISSIONS drifted from quality-gates.json",
    )
    require(
        require_string_collection(getattr(module, "REVIEW_ALLOWED_PERMISSIONS", None), "metadata_gate.REVIEW_ALLOWED_PERMISSIONS")
        == contract.review_allowed_permissions,
        "metadata_gate.REVIEW_ALLOWED_PERMISSIONS drifted from quality-gates.json",
    )


def require_storybook_browser_install(workflow, workflow_path: str) -> None:
    storybook_job = job_config(workflow, "storybook-accessibility-tests", workflow_path)
    playwright_install = step_config(
        storybook_job,
        "Install Playwright Chromium for Storybook",
        f"{workflow_path}.jobs.storybook-accessibility-tests",
    )
    require(
        playwright_install.get("run") == "bunx playwright install chromium",
        f"{workflow_path}.jobs.storybook-accessibility-tests must install Chromium without system dependencies",
    )


def require_retention_migration_validation_contract(
    workflow: dict[str, Any], expected_jobs: set[str], auxiliary_jobs: set[str]
) -> None:
    where = "ci-pr.yml.jobs.retention-migration-validation"
    job = named_job_config(
        workflow,
        "retention-migration-validation",
        expected_jobs | auxiliary_jobs,
        "ci-pr.yml",
    )
    require(
        job.get("name") in auxiliary_jobs,
        f"{where} must remain auxiliary and propagate through Build Artifacts",
    )
    require(
        job.get("needs") == "build-pr-smoke-artifacts",
        f"{where} must depend on build-pr-smoke-artifacts",
    )
    require_exact_if(
        job,
        "${{ always() && (github.event_name == 'pull_request' || github.event_name == 'merge_group') }}",
        where,
    )
    require(job.get("timeout-minutes") == 60, f"{where}.timeout-minutes must stay 60")
    require_job_and_steps_fail_closed(job, where)

    scope = step_config(job, "Detect retention migration scope", where)
    require(scope.get("id") == "retention_scope", f"{where} scope detection must expose retention_scope")
    scope_command = step_run(scope, f"{where}.steps['Detect retention migration scope']")
    for fragment in (
        "git diff --name-only",
        "BASE_SHA",
        "src/*",
        "build.rs",
        ".cargo/config.toml",
        "Cargo.lock",
        "Cargo.toml",
        "Dockerfile",
        "rust-toolchain.toml",
        "vendor/hotpath/*",
        "scripts/bind-retention-maintenance-candidate.py",
        "scripts/generate-retention-maintenance-release-fixtures.py",
        "scripts/validate-retention-maintenance-runtime.py",
        "scripts/validate-retention-maintenance-upgrade.py",
        "docs/adr/assets/retention-maintenance-task-ownership/*",
        "docs/specs/retention-maintenance-task-ownership/*",
        "required=",
        "$GITHUB_OUTPUT",
    ):
        require(fragment in scope_command, f"{where} scope detection must contain {fragment!r}")

    gated_if = "${{ steps.retention_scope.outputs.required == 'true' }}"
    for step_name in (
        "Verify smoke artifact producer",
        "Download PR smoke artifacts",
        "Extract candidate binary",
        "Generate released-source fixtures",
        "Run released-source migration validation",
        "Run runtime ownership validation",
        "Verify candidate-bound migration evidence",
    ):
        require_exact_if(step_config(job, step_name, where), gated_if, f"{where}.steps['{step_name}']")

    producer = step_config(job, "Verify smoke artifact producer", where)
    producer_env = require_mapping(producer.get("env"), f"{where}.steps['Verify smoke artifact producer'].env")
    require(
        producer_env.get("PRODUCER_RESULT") == "${{ needs.build-pr-smoke-artifacts.result }}"
        and producer.get("run") == 'test "$PRODUCER_RESULT" = success',
        f"{where} must fail when the smoke artifact producer fails",
    )
    checkout = checkout_step(job, "Checkout", where)
    require(checkout.get("fetch-depth") == 0, f"{where} Checkout must fetch the carrier parent")
    download = uses_step_config(job, "Download PR smoke artifacts", "actions/download-artifact@v7", where)
    require(
        download.get("with", {}).get("name") == "pr-smoke-artifacts-${{ github.run_id }}"
        and download.get("with", {}).get("path") == "${{ runner.temp }}/pr-smoke-artifacts",
        f"{where} must download the current run's smoke artifact",
    )

    extract = step_run(step_config(job, "Extract candidate binary", where), f"{where}.steps['Extract candidate binary']")
    for fragment in (
        "set -euo pipefail",
        "mkdir -p .ci-smoke",
        "tar -C .ci-smoke -xf",
        "test -x .ci-smoke/codex-vibe-monitor",
        "binary_sha256",
        "scripts/bind-retention-maintenance-candidate.py",
        "--record",
        "--github-sha",
        "--binary-sha256",
        "--output",
        "candidate-identity.json",
    ):
        require(fragment in extract, f"{where} candidate extraction must bind {fragment!r}")

    fixture_where = f"{where}.steps['Generate released-source fixtures']"
    fixture_command = require_command(
        step_config(job, "Generate released-source fixtures", where),
        ["python3", "scripts/generate-retention-maintenance-release-fixtures.py"],
        fixture_where,
        f"{where} must execute released-source fixture generation",
    )
    require(
        "$RUNNER_TEMP/retention-migration-fixtures" in fixture_command,
        f"{where} must generate manifest-pinned released-source fixtures",
    )
    upgrade_where = f"{where}.steps['Run released-source migration validation']"
    upgrade_command = require_command(
        step_config(job, "Run released-source migration validation", where),
        ["python3", "scripts/validate-retention-maintenance-upgrade.py"],
        upgrade_where,
        f"{where} must execute released-source migration validation",
    )
    require(
        "--candidate" in upgrade_command
        and any(".ci-smoke/codex-vibe-monitor" in token for token in upgrade_command)
        and "$RUNNER_TEMP/retention-migration-results" in upgrade_command,
        f"{where} must run released-source migration validation against the candidate binary",
    )
    runtime_where = f"{where}.steps['Run runtime ownership validation']"
    runtime_command = require_command(
        step_config(job, "Run runtime ownership validation", where),
        ["python3", "scripts/validate-retention-maintenance-runtime.py"],
        runtime_where,
        f"{where} must execute online ownership validation",
    )
    require(
        "$RUNNER_TEMP/retention-migration-fixtures/v4.2.2" in runtime_command
        and "--candidate" in runtime_command
        and "$PWD/.ci-smoke/codex-vibe-monitor" in runtime_command
        and "--http-port" in runtime_command
        and "60081" in runtime_command,
        f"{where} must run online ownership validation against the candidate binary",
    )
    verify_step = step_run(step_config(job, "Verify candidate-bound migration evidence", where), f"{where}.steps['Verify candidate-bound migration evidence']")
    for fragment in (
        "candidate-identity.json",
        "retention-migration-results/candidate.json",
        "retention-runtime-results/results.json",
        "import json",
        "import sys",
        "identity = json.loads",
        "upgrade = json.loads",
        "runtime = json.loads",
        "binary_sha256 = sys.argv[6]",
        'identity["githubSha"] == github_sha == identity["checkoutSha"] == checkout_sha',
        'identity["candidateSha256"] == binary_sha256',
        'upgrade["candidateSha256"] == binary_sha256',
        'runtime["candidateSha256"] == binary_sha256',
        "print(json.dumps(identity, sort_keys=True))",
    ):
        require(fragment in verify_step, f"{where} evidence verification must contain {fragment!r}")

    upload = uses_step_config(job, "Upload retention migration evidence", "actions/upload-artifact@v7", where)
    require_exact_if(
        upload,
        "${{ always() && steps.retention_scope.outputs.required == 'true' }}",
        f"{where}.steps['Upload retention migration evidence']",
    )
    upload_with = require_mapping(upload.get("with"), f"{where}.steps['Upload retention migration evidence'].with")
    expected_paths = {
        "${{ runner.temp }}/candidate-identity.json",
        "${{ runner.temp }}/retention-migration-results/candidate.json",
        "${{ runner.temp }}/retention-migration-results/results.json",
        "${{ runner.temp }}/retention-migration-results/*/source.json",
        "${{ runner.temp }}/retention-migration-results/*/candidate.log",
        "${{ runner.temp }}/retention-migration-results/*/candidate-diagnostics.jsonl",
        "${{ runner.temp }}/retention-runtime-results/results.json",
        "${{ runner.temp }}/retention-runtime-results/*/candidate.log",
        "${{ runner.temp }}/retention-runtime-results/*/daemon.log",
    }
    actual_paths = {line.strip() for line in str(upload_with.get("path", "")).splitlines() if line.strip()}
    require(
        upload_with.get("name") == "retention-migration-validation-${{ github.run_id }}"
        and actual_paths == expected_paths
        and upload_with.get("if-no-files-found") == "error",
        f"{where} must upload the strict candidate-bound evidence allowlist fail-closed",
    )
    require(
        upload_with.get("overwrite") is True,
        f"{where} must overwrite evidence artifacts on reruns",
    )


def validate_ci_pr(path: Path, contract: ContractModel) -> None:
    workflow = load_yaml(path)
    workflow_name = workflow.get("name")
    require(isinstance(workflow_name, str) and workflow_name, "ci-pr.yml: workflow name must stay non-empty")
    expected_jobs = set(contract.expected_pr_workflows.get(workflow_name, ()))
    require(expected_jobs, f"ci-pr.yml: workflow {workflow_name!r} must be declared in expected_pr_workflows")
    auxiliary_jobs = set(contract.expected_pr_auxiliary_workflows.get(workflow_name, ()))
    require_exact_named_jobs(workflow, expected_jobs | auxiliary_jobs, "ci-pr.yml")
    require_storybook_browser_install(workflow, "ci-pr.yml")
    require_retention_migration_validation_contract(workflow, expected_jobs, auxiliary_jobs)
    require_bun_caches(workflow, "ci-pr.yml")
    require_playwright_ci_config(path, "ci-pr.yml")

    on_section = require_mapping(mapping_get(workflow, "on"), "ci-pr.yml.on")
    require("push" not in on_section, "ci-pr.yml: push must stay disabled")
    require("workflow_dispatch" not in on_section, "ci-pr.yml: workflow_dispatch must stay disabled")
    pull_request_config = event_config(workflow, "pull_request", "ci-pr.yml")
    require("branches" not in pull_request_config, "ci-pr.yml.on.pull_request.branches must stay unset")
    require("branches-ignore" not in pull_request_config, "ci-pr.yml.on.pull_request.branches-ignore must stay unset")
    assert_event_types(pull_request_config, CI_PULL_REQUEST_TYPES, "ci-pr.yml.on.pull_request")
    merge_group_config = event_config(workflow, "merge_group", "ci-pr.yml")
    assert_event_types(merge_group_config, {"checks_requested"}, "ci-pr.yml.on.merge_group")

    concurrency = require_mapping(workflow.get("concurrency"), "ci-pr.yml.concurrency")
    require(concurrency.get("group") == "ci-pr-${{ github.event_name == 'pull_request' && github.event.action == 'edited' && format('metadata-{0}-{1}', github.event.pull_request.number, github.run_id) || github.event.pull_request.number || github.ref }}", "ci-pr.yml.concurrency.group drifted")
    require(concurrency.get("cancel-in-progress") is True, "ci-pr.yml.concurrency.cancel-in-progress must stay true")

    permissions = require_mapping(workflow.get("permissions"), "ci-pr.yml.permissions")
    require(permissions.get("contents") == "read", "ci-pr.yml.permissions.contents must stay read")
    require("statuses" not in permissions, "ci-pr.yml.permissions.statuses must stay unset")

    lint_job = named_job_config(workflow, "lint", expected_jobs, "ci-pr.yml")
    require_no_if(lint_job, "ci-pr.yml.jobs.lint")
    require_fail_closed(lint_job, "ci-pr.yml.jobs.lint")
    require_step_absent(lint_job, "Restore legacy Cargo workspace cache", "ci-pr.yml.jobs.lint")
    require_lint_cache_contract(lint_job, "ci-pr.yml")
    tooling_job = named_job_config(workflow, "repository-tooling-checks", expected_jobs, "ci-pr.yml")
    require_no_if(tooling_job, "ci-pr.yml.jobs.repository-tooling-checks")
    require_fail_closed(tooling_job, "ci-pr.yml.jobs.repository-tooling-checks")
    lefthook_install = step_config(tooling_job, "Install Lefthook for hook smoke", "ci-pr.yml.jobs.repository-tooling-checks")
    require(
        lefthook_install.get("run") == "bun install --global lefthook@2.1.10",
        "ci-pr.yml.jobs.repository-tooling-checks: Lefthook smoke must use the pinned external binary",
    )
    checkout = checkout_step(tooling_job, "Checkout", "ci-pr.yml.jobs.repository-tooling-checks")
    require(checkout.get("fetch-depth") == 0, "ci-pr.yml.jobs.repository-tooling-checks Checkout must fetch full history for trusted source resolution")
    trusted_step = step_config(tooling_job, "Resolve trusted quality-gates sources", "ci-pr.yml.jobs.repository-tooling-checks")
    trusted_run = str(trusted_step.get("run", ""))
    require('elif [ "${{ github.event_name }}" = "merge_group" ]; then' in trusted_run, "ci-pr.yml.jobs.repository-tooling-checks: merge_group trusted-source branch handling drifted")
    require('queue_prefix="refs/heads/gh-readonly-queue/"' in trusted_run, "ci-pr.yml.jobs.repository-tooling-checks: merge_group queue ref parsing drifted")
    require('supports_final_topology="true"' in trusted_run, "ci-pr.yml.jobs.repository-tooling-checks: rollout support flag drifted")
    require('source_kind="merge-group-base-branch"' in trusted_run, "ci-pr.yml.jobs.repository-tooling-checks: merge_group trusted source kind drifted")
    require(
        'github.event.pull_request.head.repo.full_name' in trusted_run,
        "ci-pr.yml.jobs.repository-tooling-checks: same-repository quality-gates source detection drifted",
    )
    require(
        'changed_quality_gate_paths="$(git diff --name-only "${source_ref}...HEAD" -- "${paths[@]}" "${final_topology_paths[@]}")"' in trusted_run,
        "ci-pr.yml.jobs.repository-tooling-checks: quality-gates change detection drifted",
    )
    require(
        'source_kind="current-branch-quality-gates-change"' in trusted_run,
        "ci-pr.yml.jobs.repository-tooling-checks: quality-gates change source kind drifted",
    )
    require(
        "keeping trusted scripts pinned to base and skipping trusted final-topology checks during rollout" in trusted_run,
        "ci-pr.yml.jobs.repository-tooling-checks: rollout warning drifted",
    )

    contract_step = step_config(tooling_job, "Quality-gates contract check", "ci-pr.yml.jobs.repository-tooling-checks")
    require(
        contract_step.get("if") == "steps.trusted-quality-gates.outputs.supports_final_topology == 'true'",
        "ci-pr.yml.jobs.repository-tooling-checks: contract check rollout gate drifted",
    )
    contract_run = str(contract_step.get("run", ""))
    require('steps.trusted-quality-gates.outputs.contract_script' in contract_run, "ci-pr.yml.jobs.repository-tooling-checks: contract check must use trusted sources")

    live_step = step_config(tooling_job, "Quality-gates live rules check", "ci-pr.yml.jobs.repository-tooling-checks")
    require(
        live_step.get("if")
        == "steps.trusted-quality-gates.outputs.supports_final_topology == 'true' && (github.event_name != 'pull_request' || github.base_ref == 'main')",
        "ci-pr.yml.jobs.repository-tooling-checks: live rules rollout gate drifted",
    )
    live_env = require_mapping(live_step.get("env"), "ci-pr.yml.jobs.repository-tooling-checks.steps['Quality-gates live rules check'].env")
    require(live_env.get("QUALITY_GATES_LIVE_RULES_MODE") == "require", "ci-pr.yml.jobs.repository-tooling-checks: live rules mode must stay require")

    self_tests = step_config(tooling_job, "Quality gates self-tests", "ci-pr.yml.jobs.repository-tooling-checks")
    self_tests_run = str(self_tests.get("run", ""))
    require(
        "test-quality-gates-contract.sh" in self_tests_run
        and "test-build-smoke-image-with-retry.sh" in self_tests_run
        and "test-live-quality-gates.sh" in self_tests_run,
        "ci-pr.yml.jobs.repository-tooling-checks: self-tests step drifted",
    )
    if "Backend Tests (Representative Scale)" in expected_jobs:
        require(
            "test-backend-test-contract.sh" in self_tests_run
            and "test-representative-scale-contract.sh" in self_tests_run,
            "ci-pr.yml.jobs.repository-tooling-checks: representative-scale self-tests drifted",
        )

    scripts_step = step_config(tooling_job, "Check quality-gates scripts", "ci-pr.yml.jobs.repository-tooling-checks")
    scripts_run = str(scripts_step.get("run", ""))
    require(
        "bash -n .github/scripts/build-smoke-image-with-retry.sh" in scripts_run,
        "ci-pr.yml.jobs.repository-tooling-checks: build smoke retry helper syntax check drifted",
    )

    if "Backend Tests (Representative Scale)" in expected_jobs:
        representative_job = named_job_config(workflow, "backend-tests-representative-scale", expected_jobs, "ci-pr.yml")
        require(representative_job.get("name") == "Backend Tests (Representative Scale)", "ci-pr.yml representative-scale job name drifted")
        require_job_and_steps_fail_closed(representative_job, "ci-pr.yml.jobs.backend-tests-representative-scale")
        require(
            representative_job.get("needs") == "backend-test-archive",
            "ci-pr.yml representative-scale job must replay the current run's backend archive",
        )
        require_exact_if(representative_job, "always()", "ci-pr.yml.jobs.backend-tests-representative-scale")
        require_backend_archive_consumer(representative_job, "ci-pr.yml", "backend-tests-representative-scale")
        producer_check = step_config(
            representative_job,
            "Verify backend test archive producer",
            "ci-pr.yml.jobs.backend-tests-representative-scale",
        )
        require(
            producer_check.get("run") == 'test "$PRODUCER_RESULT" = success',
            "ci-pr.yml representative-scale job must fail when its archive producer fails",
        )
        archive_download = step_config(
            representative_job,
            "Download backend test archive",
            "ci-pr.yml.jobs.backend-tests-representative-scale",
        )
        require(
            archive_download.get("uses") == "actions/download-artifact@v7"
            and archive_download.get("with", {}).get("name")
            == "backend-test-archive-${{ github.run_id }}"
            and archive_download.get("with", {}).get("path") == "${{ runner.temp }}",
            "ci-pr.yml representative-scale job must download the current run's archive",
        )
        run_step = step_config(representative_job, "Run deterministic representative-scale acceptance", "ci-pr.yml.jobs.backend-tests-representative-scale")
        commands = shell_commands(run_step, "ci-pr.yml.jobs.backend-tests-representative-scale")
        expected_commands = [
            [
                "bash", ".github/scripts/run-backend-tests.sh", "--profile", profile,
                "--archive-dir", "$RUNNER_TEMP/backend-test-extract", "--test-filter", selector,
            ]
            for profile, selector in [
                ("stateful-sqlite", "test(=tests::stateful_sqlite::representative_scale_acceptance::summary_representative_scale_acceptance)"),
                ("archive-file-io", "test(=tests::archive_file_io::proxy_runtime_pressure::proxy_runtime_attempt_lifecycle_avoids_sqlite_admission_under_write_pressure) | test(=tests::archive_file_io::proxy_runtime_pressure::proxy_runtime_terminal_commits_survive_retention_and_backfill_contention)"),
            ]
        ]
        require(
            commands == expected_commands,
            "ci-pr.yml representative-scale selectors must replay Summary, proxy and maintenance contention acceptance in their resource profiles",
        )

    build_job = named_job_config(workflow, "build", expected_jobs, "ci-pr.yml")
    performance_declared = "Observability Performance Budget" in contract.expected_pr_auxiliary_workflows.get(workflow_name, ())
    require(
        build_job.get("needs") == ["build-pr-smoke-artifacts", "retention-migration-validation"],
        "ci-pr.yml.jobs.build must depend on the PR smoke producer and retention validation",
    )
    if performance_declared:
        require_observability_performance_contract(workflow)
    require_exact_if(
        build_job,
        "${{ always() && (github.event_name == 'pull_request' || github.event_name == 'merge_group') }}",
        "ci-pr.yml.jobs.build",
    )
    producer_result_step = step_config(build_job, "Verify smoke artifact producer", "ci-pr.yml.jobs.build")
    producer_result_env = require_mapping(
        producer_result_step.get("env"),
        "ci-pr.yml.jobs.build.steps['Verify smoke artifact producer'].env",
    )
    require(
        producer_result_env.get("PRODUCER_RESULT") == "${{ needs.build-pr-smoke-artifacts.result }}"
        and producer_result_step.get("run") == 'test "$PRODUCER_RESULT" = success',
        "ci-pr.yml.jobs.build must fail when the PR smoke artifact producer fails",
    )
    migration_result_step = step_config(
        build_job,
        "Verify retention migration validation",
        "ci-pr.yml.jobs.build",
    )
    migration_result_env = require_mapping(
        migration_result_step.get("env"),
        "ci-pr.yml.jobs.build.steps['Verify retention migration validation'].env",
    )
    require(
        migration_result_env.get("RETENTION_MIGRATION_RESULT")
        == "${{ needs.retention-migration-validation.result }}"
        and migration_result_step.get("run") == 'test "$RETENTION_MIGRATION_RESULT" = success',
        "ci-pr.yml.jobs.build must fail when retention migration validation fails, cancels, or skips",
    )
    step_config(build_job, "Download PR smoke artifacts", "ci-pr.yml.jobs.build")
    step_config(build_job, "Extract PR smoke artifacts", "ci-pr.yml.jobs.build")

    e2e_job = named_job_config(workflow, "records-overlay-e2e", expected_jobs, "ci-pr.yml")
    require(
        e2e_job.get("needs") == "records-overlay-e2e-producer",
        "ci-pr.yml.jobs.records-overlay-e2e.needs must use the E2E test producer",
    )
    require_exact_if(e2e_job, "always()", "ci-pr.yml.jobs.records-overlay-e2e")
    e2e_producer_result_step = step_config(e2e_job, "Verify E2E test producer", "ci-pr.yml.jobs.records-overlay-e2e")
    e2e_producer_result_env = require_mapping(
        e2e_producer_result_step.get("env"),
        "ci-pr.yml.jobs.records-overlay-e2e.steps['Verify E2E test producer'].env",
    )
    require(
        e2e_producer_result_env.get("PRODUCER_RESULT") == "${{ needs.records-overlay-e2e-producer.result }}"
        and e2e_producer_result_step.get("run") == 'test "$PRODUCER_RESULT" = success',
        "ci-pr.yml.jobs.records-overlay-e2e must fail when the E2E test producer fails",
    )

    if auxiliary_jobs:
        e2e_producer_job = job_config(workflow, "records-overlay-e2e-producer", "ci-pr.yml")
        require(
            e2e_producer_job.get("name") == "PR E2E Test Producer" and e2e_producer_job.get("name") in auxiliary_jobs,
            "ci-pr.yml.jobs.records-overlay-e2e-producer must be the declared E2E test producer",
        )
        require_no_if(e2e_producer_job, "ci-pr.yml.jobs.records-overlay-e2e-producer")
        require_fail_closed(e2e_producer_job, "ci-pr.yml.jobs.records-overlay-e2e-producer")
        e2e_run_step = step_config(
            e2e_producer_job,
            "Run records overlay and Web Demo Playwright regression",
            "ci-pr.yml.jobs.records-overlay-e2e-producer",
        )
        e2e_run = str(e2e_run_step.get("run", ""))
        require(
            "playwright.ci.config.ts" in e2e_run
            and "--project=fixtures-functional" in e2e_run
            and "--project=demo-functional" in e2e_run
            and "--project=performance" in e2e_run,
            "ci-pr.yml.jobs.records-overlay-e2e-producer must run all required Playwright regression projects",
        )
        require(
            "--output=test-results/functional" in e2e_run
            and "--output=test-results/performance" in e2e_run,
            "ci-pr.yml.jobs.records-overlay-e2e-producer must isolate all Playwright result directories",
        )
        expected_e2e_run = "\n".join(
            [
                "set -euo pipefail",
                "functional_status=0",
                "performance_status=0",
                "CI_PROFILE=pr E2E_BASE_URL=http://127.0.0.1:60080 E2E_DEMO_BASE_URL=http://127.0.0.1:60083 \\",
                "  PLAYWRIGHT_HTML_OUTPUT_DIR=playwright-report-functional \\",
                "  bun run test:e2e -- --config=playwright.ci.config.ts --project=fixtures-functional --project=demo-functional --workers=1 --output=test-results/functional || functional_status=$?",
                "CI_PROFILE=pr E2E_DEMO_BASE_URL=http://127.0.0.1:60084 E2E_PRODUCTION_BUILD=1 \\",
                "  PLAYWRIGHT_HTML_OUTPUT_DIR=playwright-report-performance \\",
                "  bun run test:e2e -- --config=playwright.ci.config.ts --project=performance --workers=1 --output=test-results/performance || performance_status=$?",
                "exit $(( functional_status || performance_status ))",
            ]
        )
        require(
            e2e_run.strip() == expected_e2e_run,
            "ci-pr.yml.jobs.records-overlay-e2e-producer must run the functional and performance batches serially and propagate each status",
        )
        require(
            "E2E_DEMO_BASE_URL=http://127.0.0.1:60083" in e2e_run,
            "ci-pr.yml.jobs.records-overlay-e2e-producer must run Web Demo against its mock-only server",
        )
        smoke_artifact_job = job_config(workflow, "build-pr-smoke-artifacts", "ci-pr.yml")
        require(
            smoke_artifact_job.get("name") == "PR Smoke Artifact Producer"
            and smoke_artifact_job.get("name") in auxiliary_jobs,
            "ci-pr.yml.jobs.build-pr-smoke-artifacts must be the declared PR smoke artifact producer",
        )
        require_exact_if(
            smoke_artifact_job,
            "${{ github.event_name == 'pull_request' || github.event_name == 'merge_group' }}",
            "ci-pr.yml.jobs.build-pr-smoke-artifacts",
        )
        require_job_and_steps_fail_closed(smoke_artifact_job, "ci-pr.yml.jobs.build-pr-smoke-artifacts")
        require_cargo_test_cache_restore(smoke_artifact_job, "ci-pr.yml.jobs.build-pr-smoke-artifacts")
        require_exact_cargo_cache_reuse(
            smoke_artifact_job,
            "Prepare exact Cargo test cache reuse",
            "cargo-test-cache",
            "ci-pr.yml.jobs.build-pr-smoke-artifacts",
        )
        require_pr_smoke_artifact_contract(smoke_artifact_job, build_job)
        archive_job = job_config(workflow, "backend-test-archive", "ci-pr.yml")
        require(
            archive_job.get("name") == "Backend Test Archive Producer" and archive_job.get("name") in auxiliary_jobs,
            "ci-pr.yml.jobs.backend-test-archive must be the declared archive producer",
        )
        require_no_if(archive_job, "ci-pr.yml.jobs.backend-test-archive")
        require_job_and_steps_fail_closed(
            archive_job,
            "ci-pr.yml.jobs.backend-test-archive",
            best_effort_steps=("Save Cargo registry", "Save Cargo test artifacts"),
        )
        require_fixed_cargo_nextest_install(archive_job, "ci-pr.yml.jobs.backend-test-archive")
        require_cargo_test_cache_restore(archive_job, "ci-pr.yml.jobs.backend-test-archive")
        archive_build_step = step_config(archive_job, "Build backend test archive", "ci-pr.yml.jobs.backend-test-archive")
        require(
            archive_build_step.get("id") == "build-backend-test-archive"
            and "--zstd-level 0" in str(archive_build_step.get("run", "")),
            "ci-pr.yml.jobs.backend-test-archive: archive build step id drifted",
        )
        target_cache_save_step = step_config(archive_job, "Save Cargo test artifacts", "ci-pr.yml.jobs.backend-test-archive")
        require(
            target_cache_save_step.get("if")
            == "${{ steps.build-backend-test-archive.outcome == 'success' && steps.cargo-test-cache.outputs.cache-hit != 'true' }}",
            "ci-pr.yml.jobs.backend-test-archive: target cache must save only after a successful archive build",
        )
        target_cache_save_with = require_mapping(
            target_cache_save_step.get("with"),
            "ci-pr.yml.jobs.backend-test-archive target cache save",
        )
        require(
            target_cache_save_step.get("uses") == "actions/cache/save@v5"
            and target_cache_save_with.get("path") == "target"
            and target_cache_save_with.get("key") == CARGO_TEST_CACHE_KEY,
            "ci-pr.yml.jobs.backend-test-archive: save the source/toolchain-keyed Cargo test artifacts",
        )
        registry_cache_save_step = step_config(archive_job, "Save Cargo registry", "ci-pr.yml.jobs.backend-test-archive")
        require(
            registry_cache_save_step.get("uses") == "actions/cache/save@v5"
            and registry_cache_save_step.get("continue-on-error") is True,
            "ci-pr.yml.jobs.backend-test-archive: registry cache writes must be best-effort",
        )
        archive_upload = uses_step_config(
            archive_job,
            "Upload backend test archive",
            "actions/upload-artifact@v7",
            "ci-pr.yml.jobs.backend-test-archive",
        )
        archive_upload_with = require_mapping(archive_upload.get("with"), "ci-pr.yml backend archive upload")
        require(
            archive_upload_with.get("name") == "backend-test-archive-${{ github.run_id }}"
            and archive_upload_with.get("overwrite") is True
            and archive_upload_with.get("compression-level") == 0,
            "ci-pr.yml backend archive artifact must be run-scoped, uncompressed by upload, and replace the prior attempt",
        )
        require_exact_cargo_cache_reuse(
            archive_job,
            "Prepare exact Cargo test cache reuse",
            "cargo-test-cache",
            "ci-pr.yml.jobs.backend-test-archive",
        )
        for backend_job_id in ("backend-tests-lightweight", "backend-tests-archive-file-io"):
            require(
                job_config(workflow, backend_job_id, "ci-pr.yml").get("needs") == "backend-test-archive",
                f"ci-pr.yml.jobs.{backend_job_id}.needs must use the archive producer",
            )
            require_exact_if(
                job_config(workflow, backend_job_id, "ci-pr.yml"),
                "always()",
                f"ci-pr.yml.jobs.{backend_job_id}",
            )
            require_backend_archive_consumer(job_config(workflow, backend_job_id, "ci-pr.yml"), "ci-pr.yml", backend_job_id)
        for shard_id, partition, expected_name in (
            (
                "backend-tests-stateful-sqlite-shard-1",
                "hash:1/2",
                "Backend Tests (Stateful SQLite shard 1/2)",
            ),
            (
                "backend-tests-stateful-sqlite-shard-2",
                "hash:2/2",
                "Backend Tests (Stateful SQLite shard 2/2)",
            ),
        ):
            shard_job = job_config(workflow, shard_id, "ci-pr.yml")
            require(
                shard_job.get("name") == expected_name and expected_name in auxiliary_jobs,
                f"ci-pr.yml.jobs.{shard_id} must be a declared Stateful SQLite auxiliary job",
            )
            require_job_and_steps_fail_closed(shard_job, f"ci-pr.yml.jobs.{shard_id}")
            require(shard_job.get("needs") == "backend-test-archive", f"ci-pr.yml.jobs.{shard_id}.needs must use the archive producer")
            require_exact_if(shard_job, "always()", f"ci-pr.yml.jobs.{shard_id}")
            require_backend_archive_consumer(shard_job, "ci-pr.yml", shard_id)
            shard_run = str(step_config(shard_job, "Run stateful SQLite backend profile", f"ci-pr.yml.jobs.{shard_id}").get("run", ""))
            require_backend_partition_command(shard_run, partition, f"ci-pr.yml.jobs.{shard_id}")
        aggregate_job = job_config(workflow, "backend-tests-stateful-sqlite", "ci-pr.yml")
        require_job_and_steps_fail_closed(aggregate_job, "ci-pr.yml.jobs.backend-tests-stateful-sqlite")
        require(
            aggregate_job.get("name") == "Backend Tests (Stateful SQLite)"
            and aggregate_job.get("needs") == [
                "backend-tests-stateful-sqlite-shard-1",
                "backend-tests-stateful-sqlite-shard-2",
            ],
            "ci-pr.yml.jobs.backend-tests-stateful-sqlite must aggregate both Stateful SQLite shards under the required check name",
        )
        require_exact_if(aggregate_job, "always()", "ci-pr.yml.jobs.backend-tests-stateful-sqlite")
        aggregate_step = step_config(
            aggregate_job,
            "Require both Stateful SQLite shards",
            "ci-pr.yml.jobs.backend-tests-stateful-sqlite",
        )
        aggregate_env = require_mapping(
            aggregate_step.get("env"),
            "ci-pr.yml.jobs.backend-tests-stateful-sqlite.steps['Require both Stateful SQLite shards'].env",
        )
        require(
            aggregate_env.get("SHARD_ONE_RESULT") == "${{ needs.backend-tests-stateful-sqlite-shard-1.result }}"
            and aggregate_env.get("SHARD_TWO_RESULT") == "${{ needs.backend-tests-stateful-sqlite-shard-2.result }}",
            "ci-pr.yml Stateful SQLite aggregate must read both shard results",
        )
        aggregate_run = str(aggregate_step.get("run", ""))
        require_fail_closed_stateful_aggregate(aggregate_run, "ci-pr.yml")


def validate_ci_main(path: Path, contract: ContractModel) -> None:
    workflow = load_yaml(path)
    workflow_name = workflow.get("name")
    require(isinstance(workflow_name, str) and workflow_name, "ci-main.yml: workflow name must stay non-empty")
    expected_jobs = set(contract.expected_main_workflows.get(workflow_name, ()))
    require(expected_jobs, f"ci-main.yml: workflow {workflow_name!r} must be declared in expected_main_workflows")
    auxiliary_jobs = set(contract.expected_main_auxiliary_workflows.get(workflow_name, ()))
    require_exact_named_jobs(workflow, expected_jobs | auxiliary_jobs, "ci-main.yml")
    require_storybook_browser_install(workflow, "ci-main.yml")
    require_bun_caches(workflow, "ci-main.yml")
    require_playwright_ci_config(path, "ci-main.yml")

    e2e_job = named_job_config(workflow, "records-overlay-e2e", expected_jobs, "ci-main.yml")
    require_no_if(e2e_job, "ci-main.yml.jobs.records-overlay-e2e")
    require_fail_closed(e2e_job, "ci-main.yml.jobs.records-overlay-e2e")
    e2e_run_step = step_config(
        e2e_job,
        "Run records overlay and Web Demo Playwright regression",
        "ci-main.yml.jobs.records-overlay-e2e",
    )
    e2e_run = str(e2e_run_step.get("run", ""))
    expected_e2e_run = "\n".join(
        [
            "set -euo pipefail",
            "functional_status=0",
            "performance_status=0",
            "timeline_capacity_status=0",
            "CI_PROFILE=main E2E_BASE_URL=http://127.0.0.1:60080 E2E_DEMO_BASE_URL=http://127.0.0.1:60083 \\",
            "  PLAYWRIGHT_HTML_OUTPUT_DIR=playwright-report-functional \\",
            "  bun run test:e2e -- --config=playwright.ci.config.ts --project=fixtures-functional --project=demo-functional --workers=2 --output=test-results/functional || functional_status=$?",
            "CI_PROFILE=main E2E_DEMO_BASE_URL=http://127.0.0.1:60084 E2E_PRODUCTION_BUILD=1 \\",
            "  PLAYWRIGHT_HTML_OUTPUT_DIR=playwright-report-performance \\",
            "  bun run test:e2e -- --config=playwright.ci.config.ts --project=performance --workers=1 --output=test-results/performance || performance_status=$?",
            "CI_PROFILE=main E2E_DEMO_BASE_URL=http://127.0.0.1:60085 E2E_PRODUCTION_BUILD=1 E2E_TIMELINE_CAPACITY=1 E2E_TIMELINE_CAPACITY_DURATION_MS=30000 \\",
            "  PLAYWRIGHT_HTML_OUTPUT_DIR=playwright-report-timeline-capacity \\",
            "  bun run test:e2e -- --config=playwright.ci.config.ts --project=timeline-capacity --workers=1 --output=test-results/timeline-capacity || timeline_capacity_status=$?",
            "exit $(( functional_status || performance_status || timeline_capacity_status ))",
        ]
    )
    require(
        e2e_run.strip() == expected_e2e_run,
        "ci-main.yml.jobs.records-overlay-e2e must run functional, performance, and timeline batches serially and propagate each status",
    )
    require(
        all(project in e2e_run for project in ("fixtures-functional", "demo-functional", "performance", "timeline-capacity")),
        "ci-main.yml.jobs.records-overlay-e2e must run all required Playwright projects",
    )

    on_section = require_mapping(mapping_get(workflow, "on"), "ci-main.yml.on")
    require("pull_request" not in on_section, "ci-main.yml: pull_request must stay disabled")
    require("merge_group" not in on_section, "ci-main.yml: merge_group must stay disabled")
    require("workflow_dispatch" not in on_section, "ci-main.yml: workflow_dispatch must stay disabled")
    push_config = event_config(workflow, "push", "ci-main.yml")
    assert_event_branches(push_config, {"main"}, "ci-main.yml.on.push")

    concurrency = require_mapping(workflow.get("concurrency"), "ci-main.yml.concurrency")
    require(concurrency.get("group") == "ci-main-main", "ci-main.yml.concurrency.group drifted")
    require(concurrency.get("cancel-in-progress") is False, "ci-main.yml.concurrency.cancel-in-progress must stay false")

    permissions = require_mapping(workflow.get("permissions"), "ci-main.yml.permissions")
    require(permissions.get("contents") == "read", "ci-main.yml.permissions.contents must stay read")

    lint_job = named_job_config(workflow, "lint", expected_jobs, "ci-main.yml")
    require_no_if(lint_job, "ci-main.yml.jobs.lint")
    require_fail_closed(lint_job, "ci-main.yml.jobs.lint")
    require_step_absent(lint_job, "Restore legacy Cargo workspace cache", "ci-main.yml.jobs.lint")
    require_lint_cache_contract(lint_job, "ci-main.yml")
    tooling_job = named_job_config(workflow, "repository-tooling-checks", expected_jobs, "ci-main.yml")
    require_no_if(tooling_job, "ci-main.yml.jobs.repository-tooling-checks")
    require_fail_closed(tooling_job, "ci-main.yml.jobs.repository-tooling-checks")
    lefthook_install = step_config(tooling_job, "Install Lefthook for hook smoke", "ci-main.yml.jobs.repository-tooling-checks")
    require(
        lefthook_install.get("run") == "bun install --global lefthook@2.1.10",
        "ci-main.yml.jobs.repository-tooling-checks: Lefthook smoke must use the pinned external binary",
    )
    scripts_step = step_config(tooling_job, "Check quality-gates scripts", "ci-main.yml.jobs.repository-tooling-checks")
    scripts_run = str(scripts_step.get("run", ""))
    require(
        "bash -n .github/scripts/build-smoke-image-with-retry.sh" in scripts_run,
        "ci-main.yml.jobs.repository-tooling-checks: build smoke retry helper syntax check drifted",
    )
    trusted_step = step_config(tooling_job, "Resolve trusted quality-gates sources", "ci-main.yml.jobs.repository-tooling-checks")
    trusted_run = str(trusted_step.get("run", ""))
    require('source_ref="HEAD"' in trusted_run, "ci-main.yml.jobs.repository-tooling-checks: trusted-source ref drifted")
    require('source_kind="current-branch"' in trusted_run, "ci-main.yml.jobs.repository-tooling-checks: trusted-source kind drifted")
    require("cp \"$path\" \"$trusted_root/$path\"" in trusted_run, "ci-main.yml.jobs.repository-tooling-checks: trusted-source copy drifted")

    self_tests = step_config(tooling_job, "Quality gates self-tests", "ci-main.yml.jobs.repository-tooling-checks")
    self_tests_run = str(self_tests.get("run", ""))
    require(
        "test-quality-gates-contract.sh" in self_tests_run
        and "test-build-smoke-image-with-retry.sh" in self_tests_run
        and "test-live-quality-gates.sh" in self_tests_run,
        "ci-main.yml.jobs.repository-tooling-checks: self-tests step drifted",
    )

    release_snapshot = named_job_config(workflow, "release-snapshot", expected_jobs, "ci-main.yml")
    expected_release_needs = [
        "lint",
        "repository-tooling-checks",
        "frontend-tests",
        "storybook-accessibility-tests",
        "docs-demo-build",
        "records-overlay-e2e",
        "backend-tests-lightweight",
        "backend-tests-stateful-sqlite",
        "backend-tests-archive-file-io",
    ]
    require(
        release_snapshot.get("needs") == expected_release_needs,
        "ci-main.yml.jobs.release-snapshot.needs drifted",
    )
    if auxiliary_jobs:
        archive_job = job_config(workflow, "backend-test-archive", "ci-main.yml")
        require(
            archive_job.get("name") == "Backend Test Archive Producer" and archive_job.get("name") in auxiliary_jobs,
            "ci-main.yml.jobs.backend-test-archive must be the declared archive producer",
        )
        require_no_if(archive_job, "ci-main.yml.jobs.backend-test-archive")
        require_job_and_steps_fail_closed(
            archive_job,
            "ci-main.yml.jobs.backend-test-archive",
            best_effort_steps=("Save Cargo registry", "Save Cargo test artifacts"),
        )
        require_fixed_cargo_nextest_install(archive_job, "ci-main.yml.jobs.backend-test-archive")
        require_cargo_test_cache_restore(archive_job, "ci-main.yml.jobs.backend-test-archive")
        archive_build_step = step_config(archive_job, "Build backend test archive", "ci-main.yml.jobs.backend-test-archive")
        require(
            archive_build_step.get("id") == "build-backend-test-archive"
            and "--zstd-level 0" in str(archive_build_step.get("run", "")),
            "ci-main.yml.jobs.backend-test-archive: archive build step id drifted",
        )
        target_cache_save_step = step_config(archive_job, "Save Cargo test artifacts", "ci-main.yml.jobs.backend-test-archive")
        require(
            target_cache_save_step.get("if")
            == "${{ steps.build-backend-test-archive.outcome == 'success' && steps.cargo-test-cache.outputs.cache-hit != 'true' }}",
            "ci-main.yml.jobs.backend-test-archive: target cache must save only after a successful archive build",
        )
        target_cache_save_with = require_mapping(
            target_cache_save_step.get("with"),
            "ci-main.yml.jobs.backend-test-archive target cache save",
        )
        require(
            target_cache_save_step.get("uses") == "actions/cache/save@v5"
            and target_cache_save_with.get("path") == "target"
            and target_cache_save_with.get("key") == CARGO_TEST_CACHE_KEY,
            "ci-main.yml.jobs.backend-test-archive: save the source/toolchain-keyed Cargo test artifacts",
        )
        registry_cache_save_step = step_config(archive_job, "Save Cargo registry", "ci-main.yml.jobs.backend-test-archive")
        require(
            registry_cache_save_step.get("uses") == "actions/cache/save@v5"
            and registry_cache_save_step.get("continue-on-error") is True,
            "ci-main.yml.jobs.backend-test-archive: registry cache writes must be best-effort",
        )
        archive_upload = uses_step_config(
            archive_job,
            "Upload backend test archive",
            "actions/upload-artifact@v7",
            "ci-main.yml.jobs.backend-test-archive",
        )
        archive_upload_with = require_mapping(archive_upload.get("with"), "ci-main.yml backend archive upload")
        require(
            archive_upload_with.get("name") == "backend-test-archive-${{ github.run_id }}"
            and archive_upload_with.get("overwrite") is True
            and archive_upload_with.get("compression-level") == 0,
            "ci-main.yml backend archive artifact must be run-scoped, uncompressed by upload, and replace the prior attempt",
        )
        require_exact_cargo_cache_reuse(
            archive_job,
            "Prepare exact Cargo test cache reuse",
            "cargo-test-cache",
            "ci-main.yml.jobs.backend-test-archive",
        )
        for backend_job_id in ("backend-tests-lightweight", "backend-tests-archive-file-io"):
            require(
                job_config(workflow, backend_job_id, "ci-main.yml").get("needs") == "backend-test-archive",
                f"ci-main.yml.jobs.{backend_job_id}.needs must use the archive producer",
            )
            require_exact_if(
                job_config(workflow, backend_job_id, "ci-main.yml"),
                "always()",
                f"ci-main.yml.jobs.{backend_job_id}",
            )
            require_backend_archive_consumer(job_config(workflow, backend_job_id, "ci-main.yml"), "ci-main.yml", backend_job_id)
        for shard_id, partition in (
            ("backend-tests-stateful-sqlite-shard-1", "hash:1/2"),
            ("backend-tests-stateful-sqlite-shard-2", "hash:2/2"),
        ):
            shard_job = job_config(workflow, shard_id, "ci-main.yml")
            require_job_and_steps_fail_closed(shard_job, f"ci-main.yml.jobs.{shard_id}")
            require(shard_job.get("needs") == "backend-test-archive", f"ci-main.yml.jobs.{shard_id}.needs must use the archive producer")
            require_exact_if(shard_job, "always()", f"ci-main.yml.jobs.{shard_id}")
            require_backend_archive_consumer(shard_job, "ci-main.yml", shard_id)
            shard_run = str(step_config(shard_job, "Run stateful SQLite backend profile", f"ci-main.yml.jobs.{shard_id}").get("run", ""))
            require_backend_partition_command(shard_run, partition, f"ci-main.yml.jobs.{shard_id}")
        aggregate_job = job_config(workflow, "backend-tests-stateful-sqlite", "ci-main.yml")
        require_job_and_steps_fail_closed(aggregate_job, "ci-main.yml.jobs.backend-tests-stateful-sqlite")
        require(
            aggregate_job.get("needs") == ["backend-tests-stateful-sqlite-shard-1", "backend-tests-stateful-sqlite-shard-2"],
            "ci-main.yml.jobs.backend-tests-stateful-sqlite.needs must aggregate both shards",
        )
        require_exact_if(aggregate_job, "always()", "ci-main.yml.jobs.backend-tests-stateful-sqlite")
        aggregate_step = step_config(
            aggregate_job,
            "Require both Stateful SQLite shards",
            "ci-main.yml.jobs.backend-tests-stateful-sqlite",
        )
        aggregate_env = require_mapping(
            aggregate_step.get("env"),
            "ci-main.yml.jobs.backend-tests-stateful-sqlite.steps['Require both Stateful SQLite shards'].env",
        )
        require(
            aggregate_env.get("SHARD_ONE_RESULT") == "${{ needs.backend-tests-stateful-sqlite-shard-1.result }}"
            and aggregate_env.get("SHARD_TWO_RESULT") == "${{ needs.backend-tests-stateful-sqlite-shard-2.result }}",
            "ci-main.yml Stateful SQLite aggregate must read both matching shard results",
        )
        aggregate_run = str(aggregate_step.get("run", ""))
        require_fail_closed_stateful_aggregate(aggregate_run, "ci-main.yml")

        candidate_meta = job_config(workflow, "candidate-meta", "ci-main.yml")
        require(candidate_meta.get("name") == "Candidate Image Metadata", "ci-main.yml candidate metadata job name drifted")
        require_no_if(candidate_meta, "ci-main.yml.jobs.candidate-meta")
        candidate_ensure = step_config(candidate_meta, "Ensure immutable release snapshot", "ci-main.yml.jobs.candidate-meta")
        candidate_ensure_run = str(candidate_ensure.get("run", ""))
        require("--target-only" in candidate_ensure_run, "ci-main.yml candidate metadata must freeze only the current SHA")
        candidate_exports = step_config(candidate_meta, "Export candidate metadata", "ci-main.yml.jobs.candidate-meta")
        candidate_exports_run = str(candidate_exports.get("run", ""))
        require("target_sha_short" in candidate_exports_run and "app_effective_version" in candidate_exports_run, "ci-main.yml candidate metadata outputs drifted")
        for candidate_id, platform in (("candidate-amd64", "amd64"), ("candidate-arm64", "arm64")):
            candidate_job = job_config(workflow, candidate_id, "ci-main.yml")
            require(candidate_job.get("if") == "${{ needs.candidate-meta.outputs.release_enabled == 'true' }}", f"ci-main.yml.jobs.{candidate_id}.if drifted")
            require(candidate_job.get("needs") == "candidate-meta", f"ci-main.yml.jobs.{candidate_id}.needs drifted")
            checkout = checkout_step(candidate_job, "Checkout target SHA", f"ci-main.yml.jobs.{candidate_id}")
            require(checkout.get("ref") == "${{ needs.candidate-meta.outputs.target_sha }}", f"ci-main.yml.jobs.{candidate_id} checkout ref drifted")
            build_name = f"Build candidate image (linux/{platform})"
            build = step_config(candidate_job, build_name, f"ci-main.yml.jobs.{candidate_id}")
            if build.get("uses") == "docker/build-push-action@v7":
                build_with = require_mapping(build.get("with"), f"ci-main.yml.jobs.{candidate_id}.steps[{build_name!r}].with")
                require(build_with.get("target") == "runtime", f"ci-main.yml.jobs.{candidate_id} must build runtime target")
                build_args = str(build_with.get("build-args", ""))
                require("APP_EFFECTIVE_VERSION=" in build_args and "APP_GIT_REVISION=" in build_args, f"ci-main.yml.jobs.{candidate_id} OCI build args drifted")
            else:
                build_env = require_mapping(build.get("env"), f"ci-main.yml.jobs.{candidate_id}.steps[{build_name!r}].env")
                require(build_env.get("APP_EFFECTIVE_VERSION") == "${{ needs.candidate-meta.outputs.app_effective_version }}", f"ci-main.yml.jobs.{candidate_id} version plumbing drifted")
                require(build_env.get("APP_GIT_REVISION") == "${{ needs.candidate-meta.outputs.target_sha }}", f"ci-main.yml.jobs.{candidate_id} revision plumbing drifted")
                require("build-smoke-image-with-retry.sh" in str(build.get("run", "")), f"ci-main.yml.jobs.{candidate_id} must use the retry helper")
            smoke = step_config(candidate_job, f"Smoke test image (linux/{platform})", f"ci-main.yml.jobs.{candidate_id}")
            require("SMOKE_TAG" in require_mapping(smoke.get("env"), f"ci-main.yml.jobs.{candidate_id}.smoke.env"), f"ci-main.yml.jobs.{candidate_id} smoke tag missing")

    release_snapshot_permissions = require_mapping(
        release_snapshot.get("permissions"), "ci-main.yml.jobs.release-snapshot.permissions"
    )
    require(
        release_snapshot_permissions.get("actions") == "read",
        "ci-main.yml.jobs.release-snapshot.permissions.actions must stay read",
    )
    require(
        release_snapshot_permissions.get("contents") == "write",
        "ci-main.yml.jobs.release-snapshot.permissions.contents must stay write",
    )
    require(
        release_snapshot_permissions.get("issues") == "read",
        "ci-main.yml.jobs.release-snapshot.permissions.issues must stay read",
    )
    require(
        release_snapshot_permissions.get("pull-requests") == "read",
        "ci-main.yml.jobs.release-snapshot.permissions.pull-requests must stay read",
    )
    ensure_step = step_config(
        release_snapshot, "Ensure immutable release snapshot", "ci-main.yml.jobs.release-snapshot"
    )
    ensure_run = str(ensure_step.get("run", ""))
    require(
        "release_snapshot.py ensure" in ensure_run,
        "ci-main.yml.jobs.release-snapshot: snapshot writer must use release_snapshot.py ensure",
    )
    require(
        "RELEASE_SNAPSHOT_NOTES_REF" in ensure_run,
        "ci-main.yml.jobs.release-snapshot: notes-ref plumbing drifted",
    )
    require(
        "--target-only" not in ensure_run,
        "ci-main.yml.jobs.release-snapshot: automatic snapshot ensure must materialize missing commits on the mainline path",
    )


def validate_backend_test_image(path: Path, contract: ContractModel) -> None:
    workflow = load_yaml(path)
    workflow_name = workflow.get("name")
    require(isinstance(workflow_name, str) and workflow_name, "backend-test-image.yml: workflow name must stay non-empty")
    expected_jobs = set(contract.expected_auxiliary_workflows.get(workflow_name, ()))
    require(expected_jobs, f"backend-test-image.yml: workflow {workflow_name!r} must be declared in expected_auxiliary_workflows")
    require_exact_named_jobs(workflow, expected_jobs, "backend-test-image.yml")

    workflow_run = event_config(workflow, "workflow_run", "backend-test-image.yml")
    assert_event_types(workflow_run, {"completed"}, "backend-test-image.yml.on.workflow_run")
    assert_event_branches(workflow_run, {"main"}, "backend-test-image.yml.on.workflow_run")
    require(workflow_run.get("workflows") == ["CI Main"], "backend-test-image.yml.on.workflow_run.workflows drifted")
    permissions = require_mapping(workflow.get("permissions"), "backend-test-image.yml.permissions")
    require(permissions.get("contents") == "read", "backend-test-image.yml.permissions.contents must stay read")

    job = named_job_config(workflow, "backend-test-image", expected_jobs, "backend-test-image.yml")
    require_exact_if(job, "${{ github.event.workflow_run.conclusion == 'success' }}", "backend-test-image.yml.jobs.backend-test-image")
    job_permissions = require_mapping(job.get("permissions"), "backend-test-image.yml.jobs.backend-test-image.permissions")
    require(job_permissions.get("packages") == "write", "backend-test-image.yml backend-test-image must publish with packages: write")
    checkout = checkout_step(job, "Checkout CI Main head", "backend-test-image.yml.jobs.backend-test-image")
    require(checkout.get("ref") == "${{ github.event.workflow_run.head_sha }}", "backend-test-image.yml checkout must use workflow_run.head_sha")
    build = step_config(job, "Build and publish immutable backend-test image (linux/amd64)", "backend-test-image.yml.jobs.backend-test-image")
    build_with = require_mapping(build.get("with"), "backend-test-image.yml backend-test build")
    require(build_with.get("target") == "backend-test" and build_with.get("push") is True, "backend-test-image.yml must publish the backend-test target")
    tags = str(build_with.get("tags", ""))
    require("backend-test-${{ github.event.workflow_run.head_sha }}" in tags, "backend-test-image.yml tag must use workflow_run.head_sha")
    require("backend-test-${{ github.sha }}" not in tags, "backend-test-image.yml must not tag from its own workflow SHA")
def validate_label_gate(path: Path, contract: ContractModel) -> None:
    workflow = load_yaml(path)
    workflow_name = workflow.get("name")
    require(isinstance(workflow_name, str) and workflow_name, "label-gate.yml: workflow name must stay non-empty")
    expected_jobs = set(contract.expected_pr_workflows.get(workflow_name, ()))
    require(expected_jobs, f"label-gate.yml: workflow {workflow_name!r} must be declared in expected_pr_workflows")
    require_exact_named_jobs(workflow, expected_jobs, "label-gate.yml")

    on_section = require_mapping(mapping_get(workflow, "on"), "label-gate.yml.on")
    require("merge_group" not in on_section, "label-gate.yml: merge_group must stay disabled")
    require("workflow_dispatch" not in on_section, "label-gate.yml: workflow_dispatch must stay disabled")
    require("pull_request_target" not in on_section, "label-gate.yml: pull_request_target must stay disabled")
    pull_request_config = event_config(workflow, "pull_request", "label-gate.yml")
    assert_event_branches(pull_request_config, {"main"}, "label-gate.yml.on.pull_request")
    assert_event_types(pull_request_config, LABEL_GATE_PULL_REQUEST_TYPES, "label-gate.yml.on.pull_request")

    permissions = require_mapping(workflow.get("permissions"), "label-gate.yml.permissions")
    require(permissions.get("contents") == "read", "label-gate.yml.permissions.contents must stay read")
    require(permissions.get("pull-requests") == "read", "label-gate.yml.permissions.pull-requests must stay read")
    require(permissions.get("issues") == "read", "label-gate.yml.permissions.issues must stay read")

    concurrency = require_mapping(workflow.get("concurrency"), "label-gate.yml.concurrency")
    require(concurrency.get("group") == "label-gate-${{ github.event_name == 'pull_request' && github.event.action == 'edited' && format('metadata-{0}-{1}', github.event.pull_request.number, github.run_id) || github.event.pull_request.number || github.run_id }}", "label-gate.yml.concurrency.group drifted")
    require(concurrency.get("cancel-in-progress") is False, "label-gate.yml.concurrency.cancel-in-progress must stay false")

    job = named_job_config(workflow, "validate-pr-labels", expected_jobs, "label-gate.yml")
    require(job.get("name") == contract.label_check_name, "label-gate.yml: required label check name drifted")
    require_exact_if(job, "${{ github.event.pull_request.base.ref == 'main' }}", "label-gate.yml.jobs.validate-pr-labels")
    require_fail_closed(job, "label-gate.yml.jobs.validate-pr-labels")

    trusted_checkout = checkout_step(job, "Checkout trusted base", "label-gate.yml.jobs.validate-pr-labels")
    require(trusted_checkout.get("ref") == "${{ github.event.pull_request.base.ref }}", "label-gate.yml: trusted checkout ref drifted")
    require(trusted_checkout.get("path") == "trusted", "label-gate.yml: trusted checkout path drifted")
    require(trusted_checkout.get("persist-credentials") is False, "label-gate.yml: trusted checkout must disable persisted credentials")

    candidate_checkout = checkout_step(job, "Checkout candidate pull request", "label-gate.yml.jobs.validate-pr-labels")
    require(candidate_checkout.get("repository") == "${{ github.event.pull_request.head.repo.full_name }}", "label-gate.yml: candidate checkout repository drifted")
    require(candidate_checkout.get("ref") == "${{ github.event.pull_request.head.sha }}", "label-gate.yml: candidate checkout ref drifted")
    require(candidate_checkout.get("fetch-depth") == 0, "label-gate.yml: candidate checkout must fetch full history for trusted source resolution")
    require(candidate_checkout.get("path") == "candidate", "label-gate.yml: candidate checkout path drifted")
    require(candidate_checkout.get("persist-credentials") is False, "label-gate.yml: candidate checkout must disable persisted credentials")

    trusted_step = step_config(job, "Resolve trusted label-gate sources", "label-gate.yml.jobs.validate-pr-labels")
    trusted_run = str(trusted_step.get("run", ""))
    require(
        'github.event.pull_request.head.repo.full_name' in trusted_run,
        "label-gate.yml: same-repository quality-gates source detection drifted",
    )
    require(
        "git -C candidate fetch" not in trusted_run,
        "label-gate.yml: same-repository source detection must not require an extra candidate fetch",
    )
    require(
        'cmp -s "$PWD/trusted/$path" "$PWD/candidate/$path"' in trusted_run,
        "label-gate.yml: quality-gates change detection drifted",
    )
    require(
        'source_kind="current-branch-quality-gates-change"' in trusted_run,
        "label-gate.yml: quality-gates change source kind drifted",
    )
    require(
        'contract_script="$PWD/candidate/.github/scripts/check_quality_gates_contract.py"' in trusted_run,
        "label-gate.yml: current-branch contract script selection drifted",
    )
    require(
        'metadata_script="$PWD/candidate/.github/scripts/metadata_gate.py"' not in trusted_run,
        "label-gate.yml: token-bearing label evaluation must not switch to candidate metadata script",
    )

    contract_step = step_config(job, "Validate trusted label-gate contract", "label-gate.yml.jobs.validate-pr-labels")
    contract_run = str(contract_step.get("run", ""))
    require('steps.trusted-label-gate.outputs.contract_script' in contract_run, "label-gate.yml: contract check must use resolved trusted source")
    require("--repo-root \"$PWD/candidate\"" in contract_run, "label-gate.yml: trusted label gate must validate the candidate checkout")
    require("--declaration \"$PWD/candidate/.github/quality-gates.json\"" in contract_run, "label-gate.yml: trusted label gate declaration drifted")
    require('steps.trusted-label-gate.outputs.metadata_script' in contract_run, "label-gate.yml: trusted label gate metadata script drifted")

    label_step = step_config(job, "Evaluate PR labels", "label-gate.yml.jobs.validate-pr-labels")
    label_env = require_mapping(label_step.get("env"), "label-gate.yml.jobs.validate-pr-labels.steps['Evaluate PR labels'].env")
    require(label_env.get("GITHUB_TOKEN") == "${{ secrets.GITHUB_TOKEN }}", "label-gate.yml: Evaluate PR labels must pass GITHUB_TOKEN via env")
    label_run = str(label_step.get("run", ""))
    require("python3 trusted/.github/scripts/metadata_gate.py label" in label_run, "label-gate.yml: label gate must execute trusted metadata script")
    require("steps.trusted-label-gate.outputs.metadata_script" not in label_run, "label-gate.yml: token-bearing label evaluation must not use candidate-resolved metadata script")
    require(" label" in label_run, "label-gate.yml: label gate command drifted")
    require(
        "--write-intent" not in label_run,
        "label-gate.yml: Evaluate PR labels must validate labels before any release-intent write step",
    )



def validate_review_policy(path: Path, contract: ContractModel) -> None:
    workflow = load_yaml(path)
    workflow_name = workflow.get("name")
    require(isinstance(workflow_name, str) and workflow_name, "review-policy.yml: workflow name must stay non-empty")
    expected_jobs = set(contract.expected_pr_workflows.get(workflow_name, ()))
    require(expected_jobs, f"review-policy.yml: workflow {workflow_name!r} must be declared in expected_pr_workflows")
    require_exact_named_jobs(workflow, expected_jobs, "review-policy.yml")

    on_section = require_mapping(mapping_get(workflow, "on"), "review-policy.yml.on")
    require("merge_group" not in on_section, "review-policy.yml: merge_group must stay disabled")
    require("workflow_dispatch" not in on_section, "review-policy.yml: workflow_dispatch must stay disabled")
    require("pull_request_target" not in on_section, "review-policy.yml: pull_request_target must stay disabled")
    pull_request_config = event_config(workflow, "pull_request", "review-policy.yml")
    assert_event_branches(pull_request_config, {"main"}, "review-policy.yml.on.pull_request")
    assert_event_types(pull_request_config, REVIEW_POLICY_PULL_REQUEST_TYPES, "review-policy.yml.on.pull_request")
    review_config = event_config(workflow, "pull_request_review", "review-policy.yml")
    assert_event_types(review_config, REVIEW_POLICY_REVIEW_TYPES, "review-policy.yml.on.pull_request_review")

    permissions = require_mapping(workflow.get("permissions"), "review-policy.yml.permissions")
    require(permissions.get("contents") == "read", "review-policy.yml.permissions.contents must stay read")
    require(permissions.get("pull-requests") == "read", "review-policy.yml.permissions.pull-requests must stay read")

    concurrency = require_mapping(workflow.get("concurrency"), "review-policy.yml.concurrency")
    require(concurrency.get("group") == "review-policy-${{ github.event_name == 'pull_request' && github.event.action == 'edited' && format('metadata-{0}-{1}', github.event.pull_request.number, github.run_id) || github.event.pull_request.number || github.run_id }}", "review-policy.yml.concurrency.group drifted")
    require(concurrency.get("cancel-in-progress") is True, "review-policy.yml.concurrency.cancel-in-progress must stay true")

    job = named_job_config(workflow, "review-policy", expected_jobs, "review-policy.yml")
    require(job.get("name") == contract.review_check_name, "review-policy.yml: review check name drifted")
    require_exact_if(job, "${{ github.event.pull_request.base.ref == 'main' }}", "review-policy.yml.jobs.review-policy")
    require_fail_closed(job, "review-policy.yml.jobs.review-policy")

    trusted_step = step_config(job, "Resolve trusted quality-gates sources", "review-policy.yml.jobs.review-policy")
    trusted_run = str(trusted_step.get("run", ""))
    require("git fetch --no-tags --depth=1 origin" in trusted_run, "review-policy.yml: trusted-source fetch drifted")
    require("metadata_script=$trusted_root/.github/scripts/metadata_gate.py" in trusted_run, "review-policy.yml: metadata trusted-source output drifted")

    step = step_config(job, "Evaluate review policy", "review-policy.yml.jobs.review-policy")
    env = require_mapping(step.get("env"), "review-policy.yml.jobs.review-policy.steps['Evaluate review policy'].env")
    require(env.get("GITHUB_TOKEN") == "${{ secrets.GITHUB_TOKEN }}", "review-policy.yml: review gate must pass GITHUB_TOKEN via env")
    require_command(
        step,
        ["python3", "${{ steps.trusted-quality-gates.outputs.metadata_script }}", "review"],
        "review-policy.yml.jobs.review-policy.steps['Evaluate review policy']",
        "review-policy.yml: Evaluate review policy must execute the trusted metadata gate in review mode",
    )


def validate_release(path: Path, contract: ContractModel) -> None:
    workflow = load_yaml(path)
    workflow_name = workflow.get("name")
    require(isinstance(workflow_name, str) and workflow_name, "release.yml: workflow name must stay non-empty")
    expected_jobs = set(contract.expected_release_workflows.get(workflow_name, ()))
    require(expected_jobs, f"release.yml: workflow {workflow_name!r} must be declared in expected_release_workflows")
    require_exact_named_jobs(workflow, expected_jobs, "release.yml")

    on_section = require_mapping(mapping_get(workflow, "on"), "release.yml.on")
    workflow_run_config = event_config(workflow, "workflow_run", "release.yml")
    assert_event_types(workflow_run_config, {"completed"}, "release.yml.on.workflow_run")
    assert_event_branches(workflow_run_config, {"main"}, "release.yml.on.workflow_run")
    require(workflow_run_config.get("workflows") == ["CI Main"], "release.yml.on.workflow_run.workflows drifted")
    workflow_dispatch_config = event_config(workflow, "workflow_dispatch", "release.yml")
    inputs = require_mapping(workflow_dispatch_config.get("inputs"), "release.yml.on.workflow_dispatch.inputs")
    commit_sha = require_mapping(inputs.get("commit_sha"), "release.yml.on.workflow_dispatch.inputs.commit_sha")
    require(commit_sha.get("required") is True, "release.yml: workflow_dispatch.commit_sha must stay required")
    require(commit_sha.get("type") == "string", "release.yml: workflow_dispatch.commit_sha must stay string")
    version_input = require_mapping(inputs.get("version"), "release.yml.on.workflow_dispatch.inputs.version")
    require(version_input.get("required") is False, "release.yml: workflow_dispatch.version must stay optional")
    require(version_input.get("type") == "string", "release.yml: workflow_dispatch.version must stay string")
    bump_input = require_mapping(inputs.get("bump"), "release.yml.on.workflow_dispatch.inputs.bump")
    require(bump_input.get("required") is False, "release.yml: workflow_dispatch.bump must stay optional")
    require(bump_input.get("type") == "string", "release.yml: workflow_dispatch.bump must stay string")
    channel_input = require_mapping(inputs.get("channel"), "release.yml.on.workflow_dispatch.inputs.channel")
    require(channel_input.get("default") == "stable", "release.yml: workflow_dispatch.channel must default stable")
    require(channel_input.get("type") == "choice", "release.yml: workflow_dispatch.channel must stay choice")
    require(channel_input.get("options") == ["stable", "rc"], "release.yml: workflow_dispatch.channel options drifted")
    reason_input = require_mapping(inputs.get("reason"), "release.yml.on.workflow_dispatch.inputs.reason")
    require(reason_input.get("required") is False, "release.yml: workflow_dispatch.reason must stay optional for internal queue dispatch")
    require(reason_input.get("type") == "string", "release.yml: workflow_dispatch.reason must stay string")

    concurrency = require_mapping(workflow.get("concurrency"), "release.yml.concurrency")
    require(concurrency.get("group") == "release-main", "release.yml.concurrency.group drifted")
    require(concurrency.get("cancel-in-progress") is False, "release.yml.concurrency.cancel-in-progress must stay false")

    permissions = require_mapping(workflow.get("permissions"), "release.yml.permissions")
    require(permissions.get("contents") == "read", "release.yml.permissions.contents must stay read")

    ci_main_gate = named_job_config(workflow, "ci-main-gate", expected_jobs, "release.yml")
    require(ci_main_gate.get("runs-on") == RUNNER_X64, "release.yml.jobs.ci-main-gate.runs-on drifted")
    require_exact_if(
        ci_main_gate,
        "${{ github.event_name == 'workflow_run' && github.event.workflow_run.conclusion != 'success' }}",
        "release.yml.jobs.ci-main-gate",
    )
    require_fail_closed(ci_main_gate, "release.yml.jobs.ci-main-gate")
    gate_step = step_config(ci_main_gate, "Fail on unsuccessful CI Main", "release.yml.jobs.ci-main-gate")
    gate_run = str(gate_step.get("run", ""))
    require("github.event.workflow_run.conclusion" in gate_run, "release.yml.jobs.ci-main-gate: failure output must include upstream conclusion")
    require("github.event.workflow_run.head_sha" in gate_run, "release.yml.jobs.ci-main-gate: failure output must include upstream head sha")
    require("Release is blocked until CI Main succeeds." in gate_run, "release.yml.jobs.ci-main-gate: failure output drifted")
    require("exit 1" in gate_run, "release.yml.jobs.ci-main-gate: unsuccessful CI Main must fail release")

    release_meta = named_job_config(workflow, "release-meta", expected_jobs, "release.yml")
    require_exact_if(
        release_meta,
        "${{ github.event_name == 'workflow_dispatch' || (github.event_name == 'workflow_run' && github.event.workflow_run.conclusion == 'success') }}",
        "release.yml.jobs.release-meta",
    )
    release_meta_permissions = require_mapping(release_meta.get("permissions"), "release.yml.jobs.release-meta.permissions")
    require(
        release_meta_permissions.get("actions") == "read",
        "release.yml.jobs.release-meta.permissions.actions must stay read",
    )
    require(
        release_meta_permissions.get("contents") == "read",
        "release.yml.jobs.release-meta.permissions.contents must stay read",
    )
    outputs = require_mapping(release_meta.get("outputs"), "release.yml.jobs.release-meta.outputs")
    require("target_sha" in outputs, "release.yml.jobs.release-meta.outputs.target_sha must be exported")
    for output_name in (
        "snapshot_source",
        "manual_version",
        "manual_bump",
        "manual_reason",
        "manual_actor",
        "manual_triggered_at",
    ):
        require(
            output_name not in outputs,
            f"release.yml.jobs.release-meta.outputs.{output_name} must not be exported for Release body rendering",
        )

    target_step = step_config(release_meta, "Resolve requested commit", "release.yml.jobs.release-meta")
    target_run = str(target_step.get("run", ""))
    require("inputs.commit_sha" in target_run, "release.yml.jobs.release-meta: manual commit_sha resolution drifted")
    require("git merge-base --is-ancestor" in target_run, "release.yml.jobs.release-meta: main ancestry gate drifted")

    backfill_step = step_config(release_meta, "Validate manual backfill target passed CI Main", "release.yml.jobs.release-meta")
    require(backfill_step.get("if") == "github.event_name == 'workflow_dispatch'", "release.yml.jobs.release-meta: manual backfill validation gate drifted")
    backfill_env = require_mapping(backfill_step.get("env"), "release.yml.jobs.release-meta.steps['Validate manual backfill target passed CI Main'].env")
    require(backfill_env.get("TARGET_SHA") == "${{ steps.requested-target.outputs.target_sha }}", "release.yml.jobs.release-meta: manual backfill validation must consume target_sha")
    backfill_script = str(backfill_step.get("with", {}).get("script", ""))
    require("requires a successful CI Main run" in backfill_script, "release.yml.jobs.release-meta: manual backfill must require successful CI Main")
    require("snapshot-only CI Main failure" not in backfill_script, "release.yml.jobs.release-meta: snapshot-only backfill exception must stay removed")
    override_step = step_config(release_meta, "Generate manual release override snapshot", "release.yml.jobs.release-meta")
    require(
        override_step.get("if") == "github.event_name == 'workflow_dispatch' && (inputs.version != '' || inputs.bump != '' || inputs.reason != '')",
        "release.yml.jobs.release-meta: manual override snapshot gate drifted",
    )
    override_env = require_mapping(override_step.get("env"), "release.yml.jobs.release-meta.steps['Generate manual release override snapshot'].env")
    require(override_env.get("TARGET_SHA") == "${{ steps.requested-target.outputs.target_sha }}", "release.yml.jobs.release-meta: manual override must consume target_sha")
    require(override_env.get("MANUAL_VERSION") == "${{ inputs.version }}", "release.yml.jobs.release-meta: manual override must consume version input")
    require(override_env.get("MANUAL_BUMP") == "${{ inputs.bump }}", "release.yml.jobs.release-meta: manual override must consume bump input")
    require(override_env.get("MANUAL_REASON") == "${{ inputs.reason }}", "release.yml.jobs.release-meta: manual override must consume reason input")
    override_run = str(override_step.get("run", ""))
    require("release_snapshot.py manual-override" in override_run, "release.yml.jobs.release-meta: manual override must use release_snapshot.py manual-override")
    require("--reason" in override_run, "release.yml.jobs.release-meta: manual override must pass audit reason")
    require("--actor" in override_run, "release.yml.jobs.release-meta: manual override must pass actor")
    ensure_step = step_config(release_meta, "Ensure immutable release snapshot for manual backfill", "release.yml.jobs.release-meta")
    require(
        ensure_step.get("if") == "github.event_name == 'workflow_dispatch' && inputs.version == '' && inputs.bump == '' && inputs.reason == ''",
        "release.yml.jobs.release-meta: manual snapshot ensure gate drifted",
    )
    ensure_env = require_mapping(ensure_step.get("env"), "release.yml.jobs.release-meta.steps['Ensure immutable release snapshot for manual backfill'].env")
    require(ensure_env.get("TARGET_SHA") == "${{ steps.requested-target.outputs.target_sha }}", "release.yml.jobs.release-meta: manual snapshot ensure must consume target_sha")
    require(ensure_env.get("GITHUB_TOKEN") == "${{ secrets.GITHUB_TOKEN }}", "release.yml.jobs.release-meta: manual snapshot ensure must use GITHUB_TOKEN")
    ensure_run = str(ensure_step.get("run", ""))
    require("release_snapshot.py ensure" in ensure_run, "release.yml.jobs.release-meta: manual snapshot ensure must use release_snapshot.py ensure")
    require("--snapshot-source manual-backfill" in ensure_run, "release.yml.jobs.release-meta: manual snapshot ensure must mark manual-backfill source")
    require("--skip-publish" in ensure_run, "release.yml.jobs.release-meta: manual snapshot ensure must avoid notes ref writes")
    pending_step = step_config(release_meta, "Select pending release target", "release.yml.jobs.release-meta")
    pending_env = require_mapping(pending_step.get("env"), "release.yml.jobs.release-meta.steps['Select pending release target'].env")
    require(pending_env.get("REQUESTED_SHA") == "${{ steps.requested-target.outputs.target_sha }}", "release.yml.jobs.release-meta: pending target selector must consume requested target")
    require(pending_env.get("GITHUB_REPOSITORY") == "${{ github.repository }}", "release.yml.jobs.release-meta: pending target selector must receive repository context")
    require(pending_env.get("GITHUB_TOKEN") == "${{ secrets.GITHUB_TOKEN }}", "release.yml.jobs.release-meta: pending target selector must receive GITHUB_TOKEN")
    pending_run = str(pending_step.get("run", ""))
    require("release_snapshot.py next-pending" in pending_run, "release.yml.jobs.release-meta: pending target selector must use release_snapshot.py next-pending")
    require("--github-repository" in pending_run, "release.yml.jobs.release-meta: pending target selector must pass repository to next-pending")
    require("--github-token" in pending_run, "release.yml.jobs.release-meta: pending target selector must pass token to next-pending")
    snapshot_step = step_config(release_meta, "Load immutable release snapshot", "release.yml.jobs.release-meta")
    snapshot_env = require_mapping(snapshot_step.get("env"), "release.yml.jobs.release-meta.steps['Load immutable release snapshot'].env")
    require(
        snapshot_env.get("TARGET_SHA") == "${{ steps.pending-target.outputs.target_sha }}",
        "release.yml.jobs.release-meta: snapshot loader must consume target_sha",
    )
    require(snapshot_step.get("if") == "steps.pending-target.outputs.target_sha != ''", "release.yml.jobs.release-meta: snapshot loader gate drifted")
    snapshot_run = str(snapshot_step.get("run", ""))
    require(
        "release_snapshot.py export" in snapshot_run,
        "release.yml.jobs.release-meta: snapshot loader must use release_snapshot.py export",
    )
    require(
        "--snapshot-file" in snapshot_run,
        "release.yml.jobs.release-meta: manual snapshot loader must support job-local snapshots",
    )
    require(
        "RELEASE_SNAPSHOT_NOTES_REF" in snapshot_run,
        "release.yml.jobs.release-meta: snapshot notes-ref plumbing drifted",
    )
    candidate_step = step_config(release_meta, "Compute candidate suffix", "release.yml.jobs.release-meta")
    require(
        candidate_step.get("if") == "steps.pending-target.outputs.target_sha != '' && steps.snapshot.outputs.release_enabled == 'true'",
        "release.yml.jobs.release-meta: candidate suffix gate drifted",
    )
    candidate_run = str(candidate_step.get("run", ""))
    require("candidate_suffix=${TARGET_SHA:0:12}" in candidate_run, "release.yml.jobs.release-meta: candidate suffix drifted")

    candidate_availability = named_job_config(workflow, "candidate-availability", expected_jobs, "release.yml")
    require(
        candidate_availability.get("needs") == ["release-meta"],
        "release.yml.jobs.candidate-availability.needs drifted",
    )
    require(
        candidate_availability.get("if")
        == "${{ always() && needs.release-meta.result == 'success' && needs.release-meta.outputs.release_enabled == 'true' }}",
        "release.yml.jobs.candidate-availability.if drifted",
    )
    availability_run = str(step_config(candidate_availability, "Verify candidate SHA, version, and platforms", "release.yml.jobs.candidate-availability").get("run", ""))
    require("docker pull --platform" in availability_run, "release.yml candidate availability must pull both candidate platforms")
    require("org.opencontainers.image.revision" in availability_run and "org.opencontainers.image.version" in availability_run, "release.yml candidate availability must verify OCI metadata")
    require("available=${available}" in availability_run, "release.yml candidate availability output drifted")

    candidate_policy = named_job_config(workflow, "candidate-policy", expected_jobs, "release.yml")
    require(
        candidate_policy.get("needs") == ["release-meta", "candidate-availability"],
        "release.yml.jobs.candidate-policy.needs drifted",
    )
    require(
        candidate_policy.get("if")
        == "${{ always() && needs.release-meta.result == 'success' && needs.release-meta.outputs.release_enabled == 'true' }}",
        "release.yml.jobs.candidate-policy.if drifted",
    )
    policy_run = str(step_config(candidate_policy, "Enforce automatic candidate requirement", "release.yml.jobs.candidate-policy").get("run", ""))
    require("github.event_name" in policy_run and "Automatic Release requires complete" in policy_run, "release.yml candidate policy must fail closed for automatic runs")
    require("Manual Release will use the fallback build" in policy_run, "release.yml candidate policy must document manual fallback")

    docker_amd = named_job_config(workflow, "docker-amd64", expected_jobs, "release.yml")
    require(docker_amd.get("needs") == ["release-meta", "candidate-availability"], "release.yml.jobs.docker-amd64.needs drifted")
    require(
        docker_amd.get("if")
        == "${{ always() && needs.release-meta.outputs.release_enabled == 'true' && github.event_name == 'workflow_dispatch' && needs.candidate-availability.outputs.available != 'true' }}",
        "release.yml.jobs.docker-amd64.if drifted",
    )
    amd_checkout = checkout_step(docker_amd, "Checkout code", "release.yml.jobs.docker-amd64")
    require(amd_checkout.get("ref") == "${{ needs.release-meta.outputs.target_sha }}", "release.yml.jobs.docker-amd64 checkout ref drifted")
    amd_smoke_step = step_config(docker_amd, "Smoke test image (linux/amd64)", "release.yml.jobs.docker-amd64")
    amd_smoke_env = require_mapping(amd_smoke_step.get("env"), "release.yml.jobs.docker-amd64.steps['Smoke test image (linux/amd64)'].env")
    require(
        amd_smoke_env.get("SMOKE_TAG") == "${{ env.REGISTRY }}/${{ needs.release-meta.outputs.image_name_lower }}:smoke-${{ needs.release-meta.outputs.candidate_suffix }}-amd64",
        "release.yml.jobs.docker-amd64 smoke tag drifted",
    )

    docker_arm = named_job_config(workflow, "docker-arm64", expected_jobs, "release.yml")
    require(
        docker_arm.get("needs") == ["release-meta", "candidate-availability"],
        "release.yml.jobs.docker-arm64.needs drifted",
    )
    require(
        docker_arm.get("if")
        == "${{ always() && needs.release-meta.outputs.release_enabled == 'true' && github.event_name == 'workflow_dispatch' && needs.candidate-availability.outputs.available != 'true' }}",
        "release.yml.jobs.docker-arm64.if drifted",
    )
    arm_checkout = checkout_step(docker_arm, "Checkout code", "release.yml.jobs.docker-arm64")
    require(arm_checkout.get("ref") == "${{ needs.release-meta.outputs.target_sha }}", "release.yml.jobs.docker-arm64 checkout ref drifted")
    require(arm_checkout.get("path") == "target", "release.yml.jobs.docker-arm64 checkout path drifted")
    arm_helper_checkout = checkout_step(docker_arm, "Checkout workflow helpers", "release.yml.jobs.docker-arm64")
    require(arm_helper_checkout.get("ref") == "${{ github.sha }}", "release.yml.jobs.docker-arm64 helper checkout ref drifted")
    require(arm_helper_checkout.get("path") == "workflow-helpers", "release.yml.jobs.docker-arm64 helper checkout path drifted")
    helper_install_step = step_config(docker_arm, "Install workflow helper scripts", "release.yml.jobs.docker-arm64")
    require(helper_install_step.get("shell") == "bash", "release.yml.jobs.docker-arm64: helper install must run in bash")
    require(helper_install_step.get("working-directory") == "target", "release.yml.jobs.docker-arm64: helper install must run from target checkout")
    helper_install_run = str(helper_install_step.get("run", ""))
    require("install -m 755 ../workflow-helpers/.github/scripts/build-smoke-image-with-retry.sh" in helper_install_run, "release.yml.jobs.docker-arm64: helper install step must copy retry helper into target checkout")
    arm_build_step = step_config(docker_arm, "Build smoke image (linux/arm64, load)", "release.yml.jobs.docker-arm64")
    require(arm_build_step.get("shell") == "bash", "release.yml.jobs.docker-arm64: arm64 smoke build must run in bash")
    require(arm_build_step.get("working-directory") == "target", "release.yml.jobs.docker-arm64: arm64 smoke build must run from target checkout")
    arm_build_env = require_mapping(arm_build_step.get("env"), "release.yml.jobs.docker-arm64.steps['Build smoke image (linux/arm64, load)'].env")
    require(
        arm_build_env.get("BUILD_PLATFORM") == "${{ env.PLATFORM_ARM64 }}",
        "release.yml.jobs.docker-arm64: arm64 smoke build must consume PLATFORM_ARM64",
    )
    require(
        arm_build_env.get("SMOKE_TAG") == "${{ env.REGISTRY }}/${{ needs.release-meta.outputs.image_name_lower }}:smoke-${{ needs.release-meta.outputs.candidate_suffix }}-arm64",
        "release.yml.jobs.docker-arm64: arm64 smoke build smoke tag drifted",
    )
    require(
        arm_build_env.get("CANDIDATE_TAG") == "${{ env.REGISTRY }}/${{ needs.release-meta.outputs.image_name_lower }}:candidate-${{ needs.release-meta.outputs.candidate_suffix }}-arm64",
        "release.yml.jobs.docker-arm64: arm64 smoke build candidate tag drifted",
    )
    require(
        arm_build_env.get("CACHE_REF") == "${{ env.REGISTRY }}/${{ needs.release-meta.outputs.image_name_lower }}:buildcache-arm64",
        "release.yml.jobs.docker-arm64: arm64 smoke build cache ref drifted",
    )
    require(
        arm_build_env.get("APP_EFFECTIVE_VERSION") == "${{ needs.release-meta.outputs.app_effective_version }}",
        "release.yml.jobs.docker-arm64: arm64 smoke build version plumbing drifted",
    )
    require(
        arm_build_env.get("APP_GIT_REVISION") == "${{ needs.release-meta.outputs.target_sha }}",
        "release.yml.jobs.docker-arm64: arm64 smoke build revision plumbing drifted",
    )
    arm_build_run = str(arm_build_step.get("run", ""))
    require(
        "./.github/scripts/build-smoke-image-with-retry.sh" in arm_build_run,
        "release.yml.jobs.docker-arm64: arm64 smoke build must use the retry helper",
    )
    arm_smoke_step = step_config(docker_arm, "Smoke test image (linux/arm64)", "release.yml.jobs.docker-arm64")
    require(arm_smoke_step.get("working-directory") == "target", "release.yml.jobs.docker-arm64: arm64 smoke test must run from target checkout")
    arm_smoke_env = require_mapping(arm_smoke_step.get("env"), "release.yml.jobs.docker-arm64.steps['Smoke test image (linux/arm64)'].env")
    require(
        arm_smoke_env.get("SMOKE_TAG") == "${{ env.REGISTRY }}/${{ needs.release-meta.outputs.image_name_lower }}:smoke-${{ needs.release-meta.outputs.candidate_suffix }}-arm64",
        "release.yml.jobs.docker-arm64 smoke tag drifted",
    )

    publish = named_job_config(workflow, "release-publish", expected_jobs, "release.yml")
    require(
        publish.get("needs")
        == ["release-meta", "candidate-availability", "candidate-policy", "docker-amd64", "docker-arm64"],
        "release.yml.jobs.release-publish.needs drifted",
    )
    require(
        publish.get("if")
        == "${{ always() && needs.release-meta.outputs.release_enabled == 'true' && needs.candidate-policy.result == 'success' && (needs.candidate-availability.outputs.available == 'true' || (github.event_name == 'workflow_dispatch' && needs.docker-amd64.result == 'success' && needs.docker-arm64.result == 'success')) }}",
        "release.yml.jobs.release-publish.if drifted",
    )
    publish_permissions = require_mapping(publish.get("permissions"), "release.yml.jobs.release-publish.permissions")
    require(
        set(publish_permissions) == {"actions", "contents", "packages"},
        "release.yml.jobs.release-publish.permissions must exclude source-PR comment permissions",
    )
    require(publish_permissions.get("actions") == "write", "release.yml.jobs.release-publish.permissions.actions must stay write")
    require(publish_permissions.get("contents") == "write", "release.yml.jobs.release-publish.permissions.contents must stay write")
    require(publish_permissions.get("packages") == "write", "release.yml.jobs.release-publish.permissions.packages must stay write")
    publish_checkout = checkout_step(publish, "Checkout code", "release.yml.jobs.release-publish")
    require(publish_checkout.get("ref") == "${{ needs.release-meta.outputs.target_sha }}", "release.yml.jobs.release-publish checkout ref drifted")
    tag_step = step_config(publish, "Create and push git tag", "release.yml.jobs.release-publish")
    tag_run = str(tag_step.get("run", ""))
    require('sha="${TARGET_SHA}"' in tag_run, "release.yml.jobs.release-publish tag step must use target_sha")
    require("git fetch --tags origin" in tag_run, "release.yml.jobs.release-publish tag step must fetch existing tags")
    release_step = step_config(publish, "Create GitHub Release", "release.yml.jobs.release-publish")
    release_env = require_mapping(release_step.get("env"), "release.yml.jobs.release-publish.steps['Create GitHub Release'].env")
    require(
        set(release_env) == {"RELEASE_TAG", "RELEASE_PRERELEASE"},
        "release.yml.jobs.release-publish: Create GitHub Release env must only carry tag and prerelease state",
    )
    release_script = str(release_step.get("with", {}).get("script", ""))
    create_release_marker = "await github.rest.repos.createRelease({"
    require(create_release_marker in release_script, "release.yml.jobs.release-publish: createRelease call is required")
    create_release_body = release_script.split(create_release_marker, 1)[1]
    create_release_end = create_release_body.find("\n})")
    require(create_release_end >= 0, "release.yml.jobs.release-publish: createRelease call must close its options object")
    create_release_body = create_release_body[:create_release_end]
    require(
        "generate_release_notes: true" in create_release_body,
        "release.yml.jobs.release-publish: createRelease must set generate_release_notes: true",
    )
    require(
        not any(line.strip().startswith("body:") for line in create_release_body.splitlines()),
        "release.yml.jobs.release-publish: createRelease must not set body",
    )
    for forbidden_marker in (
        "PR:",
        "Channel:",
        "Bump:",
        "Commit:",
        "Manual release override:",
        "manualReason",
        "manualActor",
        "manualTriggeredAt",
    ):
        require(
            forbidden_marker not in create_release_body,
            f"release.yml.jobs.release-publish: createRelease must not render {forbidden_marker!r}",
        )
    publish_steps = require_mapping(publish, "release.yml.jobs.release-publish").get("steps")
    require(isinstance(publish_steps, list), "release.yml.jobs.release-publish.steps must be a list")
    require(
        not any(
            isinstance(step, dict) and step.get("name") == "Upsert PR release version comment"
            for step in publish_steps
        ),
        "release.yml.jobs.release-publish: source-PR release comment step must stay removed",
    )
    next_step = step_config(publish, "Resolve next pending release target", "release.yml.jobs.release-publish")
    require(next_step.get("if") == "github.event_name != 'workflow_dispatch'", "release.yml.jobs.release-publish: next pending target gate drifted")
    next_env = require_mapping(next_step.get("env"), "release.yml.jobs.release-publish.steps['Resolve next pending release target'].env")
    require(next_env.get("GITHUB_REPOSITORY") == "${{ github.repository }}", "release.yml.jobs.release-publish: next pending target must receive repository context")
    require(next_env.get("GITHUB_TOKEN") == "${{ secrets.GITHUB_TOKEN }}", "release.yml.jobs.release-publish: next pending target must receive GITHUB_TOKEN")
    next_run = str(next_step.get("run", ""))
    require("release_snapshot.py next-pending" in next_run, "release.yml.jobs.release-publish: next pending target must use release_snapshot.py next-pending")
    require("--github-repository" in next_run, "release.yml.jobs.release-publish: next pending target must pass repository to next-pending")
    require("--github-token" in next_run, "release.yml.jobs.release-publish: next pending target must pass token to next-pending")
    continue_step = step_config(publish, "Continue release queue", "release.yml.jobs.release-publish")
    require(continue_step.get("if") == "github.event_name != 'workflow_dispatch' && steps.next-pending.outputs.target_sha != ''", "release.yml.jobs.release-publish: release queue continuation gate drifted")


def validate_release_snapshot_pr(path: Path) -> None:
    workflow = load_yaml(path)
    workflow_name = workflow.get("name")
    require(isinstance(workflow_name, str) and workflow_name, "release-snapshot-pr.yml: workflow name must stay non-empty")
    require(workflow_name == "Release Snapshot PR", "release-snapshot-pr.yml: workflow name drifted")

    on_section = require_mapping(mapping_get(workflow, "on"), "release-snapshot-pr.yml.on")
    pull_request_target_config = event_config(workflow, "pull_request_target", "release-snapshot-pr.yml")
    assert_event_types(pull_request_target_config, {"closed"}, "release-snapshot-pr.yml.on.pull_request_target")
    assert_event_branches(pull_request_target_config, {"main"}, "release-snapshot-pr.yml.on.pull_request_target")
    require("pull_request" not in on_section, "release-snapshot-pr.yml: pull_request must stay disabled")
    require("push" not in on_section, "release-snapshot-pr.yml: push must stay disabled")
    require("workflow_dispatch" not in on_section, "release-snapshot-pr.yml: workflow_dispatch must stay disabled")

    concurrency = require_mapping(workflow.get("concurrency"), "release-snapshot-pr.yml.concurrency")
    require(
        concurrency.get("group") == "release-snapshot-pr-${{ github.event.pull_request.merge_commit_sha }}",
        "release-snapshot-pr.yml.concurrency.group drifted",
    )
    require(concurrency.get("cancel-in-progress") is False, "release-snapshot-pr.yml.concurrency.cancel-in-progress must stay false")

    permissions = require_mapping(workflow.get("permissions"), "release-snapshot-pr.yml.permissions")
    require(permissions.get("contents") == "write", "release-snapshot-pr.yml.permissions.contents must stay write")
    require(permissions.get("pull-requests") == "read", "release-snapshot-pr.yml.permissions.pull-requests must stay read")

    jobs = require_mapping(workflow.get("jobs"), "release-snapshot-pr.yml.jobs")
    require(set(jobs.keys()) == {"freeze-release-snapshot"}, "release-snapshot-pr.yml: job set drifted")
    job = require_mapping(jobs.get("freeze-release-snapshot"), "release-snapshot-pr.yml.jobs.freeze-release-snapshot")
    require(job.get("name") == "Freeze merged PR release snapshot", "release-snapshot-pr.yml.jobs.freeze-release-snapshot name drifted")
    require_exact_if(
        job,
        "${{ github.event.pull_request.merged == true }}",
        "release-snapshot-pr.yml.jobs.freeze-release-snapshot",
    )
    checkout = checkout_step(job, "Checkout code", "release-snapshot-pr.yml.jobs.freeze-release-snapshot")
    require(
        checkout.get("ref") == "${{ github.event.pull_request.merge_commit_sha }}",
        "release-snapshot-pr.yml.jobs.freeze-release-snapshot checkout ref drifted",
    )
    require(
        checkout.get("fetch-depth") == 0,
        "release-snapshot-pr.yml.jobs.freeze-release-snapshot checkout must fetch full history",
    )
    ensure_step = step_config(job, "Freeze immutable release snapshot", "release-snapshot-pr.yml.jobs.freeze-release-snapshot")
    ensure_env = require_mapping(ensure_step.get("env"), "release-snapshot-pr.yml.jobs.freeze-release-snapshot.steps['Freeze immutable release snapshot'].env")
    require(ensure_env.get("GITHUB_TOKEN") == "${{ secrets.GITHUB_TOKEN }}", "release-snapshot-pr.yml: merged PR snapshot must use GITHUB_TOKEN")
    ensure_run = str(ensure_step.get("run", ""))
    require("release_snapshot.py ensure" in ensure_run, "release-snapshot-pr.yml: merged PR snapshot must use release_snapshot.py ensure")
    require("--target-only" in ensure_run, "release-snapshot-pr.yml: merged PR snapshot must stay target-only")
    require("--snapshot-source merged-pr" in ensure_run, "release-snapshot-pr.yml: merged PR snapshot source drifted")
    require(
        "github.event.pull_request.merge_commit_sha" in ensure_run,
        "release-snapshot-pr.yml: merged PR snapshot must key off merge_commit_sha",
    )


def validate_merge_group_helpers(module: Any) -> None:
    try:
        module.resolve_pull_numbers(
            module.GateContext(
                gate="label",
                owner="IvanLi-CN",
                repo="codex-vibe-monitor",
                api_root="https://api.github.com",
                token="",
                event_name="merge_group",
                event_payload={},
                manual_pull_number=None,
            ),
            module.GitHubClient("IvanLi-CN", "codex-vibe-monitor", "https://api.github.com", ""),
        )
    except module.GateError as exc:
        require("unsupported" in str(exc), f"metadata_gate: unexpected merge_group error {exc}")
    else:
        raise ContractError("metadata_gate: merge_group must fail closed")


def materialize_default_repo_root(script_repo_root: Path) -> Path:
    fixtures_root = script_repo_root / ".github" / "scripts" / "fixtures" / "quality-gates-contract"
    if not fixtures_root.is_dir():
        return script_repo_root

    tempdir = Path(tempfile.mkdtemp(prefix="quality-gates-contract-"))
    shutil.copytree(script_repo_root / ".github", tempdir / ".github", dirs_exist_ok=True)
    playwright_config = script_repo_root / "web" / "playwright.ci.config.ts"
    require(playwright_config.is_file(), "repository must include web/playwright.ci.config.ts")
    (tempdir / "web").mkdir(parents=True, exist_ok=True)
    shutil.copyfile(playwright_config, tempdir / "web" / "playwright.ci.config.ts")
    shutil.copyfile(fixtures_root / "quality-gates.json", tempdir / ".github" / "quality-gates.json")
    for filename in ("ci-pr.yml", "ci-main.yml", "release.yml", "label-gate.yml", "review-policy.yml", "release-snapshot-pr.yml"):
        shutil.copyfile(fixtures_root / filename, tempdir / ".github" / "workflows" / filename)
    return tempdir


def detect_profile(repo_root: Path) -> str:
    if (repo_root / ".github" / "workflows" / "ci-pr.yml").is_file():
        return "final"
    return "bootstrap"


def main() -> int:
    args = parse_args()
    script_repo_root = Path(__file__).resolve().parents[2]
    temp_repo_root: Path | None = None
    if args.repo_root:
        repo_root = Path(args.repo_root).resolve()
    else:
        temp_repo_root = materialize_default_repo_root(script_repo_root)
        repo_root = temp_repo_root
    declaration_path = Path(args.declaration).resolve() if args.declaration else repo_root / ".github" / "quality-gates.json"
    metadata_script_path = Path(args.metadata_script).resolve() if args.metadata_script else repo_root / ".github" / "scripts" / "metadata_gate.py"

    try:
        declaration = json.loads(declaration_path.read_text())
        require(isinstance(declaration, dict), "quality-gates.json must decode to an object")
        module = load_module(metadata_script_path)
        contract = validate_quality_gates(declaration)
        validate_metadata_policy(module, contract)
        profile = "final" if not args.repo_root and args.profile == "auto" else args.profile
        if profile == "auto":
            profile = detect_profile(repo_root)
        require(profile == contract.implementation_profile, f"quality-gates.json: implementation_profile={contract.implementation_profile!r} does not match workflow profile {profile!r}")
        require(profile == "final", "quality-gates-contract: bootstrap profile is no longer supported by this repository")
        validate_ci_pr(repo_root / ".github" / "workflows" / "ci-pr.yml", contract)
        validate_observability_performance(repo_root / ".github" / "workflows" / "ci-observability-performance.yml")
        validate_ci_main(repo_root / ".github" / "workflows" / "ci-main.yml", contract)
        validate_backend_test_image(repo_root / ".github" / "workflows" / "backend-test-image.yml", contract)
        validate_release(repo_root / ".github" / "workflows" / "release.yml", contract)
        validate_release_snapshot_pr(repo_root / ".github" / "workflows" / "release-snapshot-pr.yml")
        validate_label_gate(repo_root / ".github" / "workflows" / "label-gate.yml", contract)
        validate_review_policy(repo_root / ".github" / "workflows" / "review-policy.yml", contract)
        validate_merge_group_helpers(module)
    except ContractError as exc:
        print(f"[quality-gates-contract] {exc}", file=sys.stderr)
        return 1
    finally:
        if temp_repo_root is not None:
            shutil.rmtree(temp_repo_root, ignore_errors=True)

    print("[quality-gates-contract] metadata workflow contract checks passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
