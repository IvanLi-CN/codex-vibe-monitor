from __future__ import annotations


def validate_ci_pr(path: Path, contract: ContractModel) -> None:
    workflow = load_yaml(path)
    workflow_name = workflow.get("name")
    require(isinstance(workflow_name, str) and workflow_name, "ci-pr.yml: workflow name must stay non-empty")
    expected_jobs = set(contract.expected_pr_workflows.get(workflow_name, ()))
    require(expected_jobs, f"ci-pr.yml: workflow {workflow_name!r} must be declared in expected_pr_workflows")
    auxiliary_jobs = set(contract.expected_pr_auxiliary_workflows.get(workflow_name, ()))
    require_exact_named_jobs(workflow, expected_jobs | auxiliary_jobs, "ci-pr.yml")

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
    legacy_lint_cache = step_config(lint_job, "Restore legacy Cargo workspace cache", "ci-pr.yml.jobs.lint")
    legacy_lint_cache_paths = str(legacy_lint_cache.get("with", {}).get("path", ""))
    require(
        "target" not in legacy_lint_cache_paths.splitlines(),
        "ci-pr.yml.jobs.lint: legacy cache must not restore Cargo target artifacts",
    )
    _validate_ci_pr_tooling(workflow, expected_jobs)
    _validate_ci_pr_build(workflow, expected_jobs)
    _validate_ci_pr_auxiliary(workflow, auxiliary_jobs)


def _validate_ci_pr_tooling(workflow: dict[str, Any], expected_jobs: set[str]) -> None:
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
        build_step = step_config(representative_job, "Build candidate backend-test image (linux/amd64)", "ci-pr.yml.jobs.backend-tests-representative-scale")
        require(build_step.get("uses") == "docker/build-push-action@v7", "ci-pr.yml representative-scale image build action drifted")
        require(build_step.get("with", {}).get("target") == "backend-test", "ci-pr.yml representative-scale image target drifted")
        run_step = step_config(representative_job, "Run deterministic representative-scale acceptance", "ci-pr.yml.jobs.backend-tests-representative-scale")
        run_text = str(run_step.get("run", ""))
        require("--profile stateful-sqlite" in run_text and "representative_scale_acceptance" in run_text, "ci-pr.yml representative-scale selector drifted")



def _validate_ci_pr_build(workflow: dict[str, Any], expected_jobs: set[str]) -> None:
    build_job = named_job_config(workflow, "build", expected_jobs, "ci-pr.yml")
    require(
        build_job.get("needs") == "build-pr-smoke-artifacts",
        "ci-pr.yml.jobs.build.needs must use the PR smoke artifact producer",
    )
    require_exact_if(
        build_job,
        "${{ always() && github.event_name == 'pull_request' }}",
        "ci-pr.yml.jobs.build",
    )
    producer_result_step = step_config(build_job, "Verify smoke artifact producer", "ci-pr.yml.jobs.build")
    producer_result_env = require_mapping(
        producer_result_step.get("env"),
        "ci-pr.yml.jobs.build.steps['Verify smoke artifact producer'].env",
    )
    require(
        producer_result_env.get("PRODUCER_RESULT") == "${{ needs.build-pr-smoke-artifacts.result }}",
        "ci-pr.yml.jobs.build must fail when the PR smoke artifact producer fails",
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
        e2e_producer_result_env.get("PRODUCER_RESULT") == "${{ needs.records-overlay-e2e-producer.result }}",
        "ci-pr.yml.jobs.records-overlay-e2e must fail when the E2E test producer fails",
    )



def _validate_ci_pr_auxiliary(workflow: dict[str, Any], auxiliary_jobs: set[str]) -> None:
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
            "records-filter-overlay.spec.ts" in e2e_run
            and "demo-runtime.spec.ts" in e2e_run
            and "dashboard-working-conversations-layout.spec.ts" in e2e_run,
            "ci-pr.yml.jobs.records-overlay-e2e-producer must run all required Playwright regression specs",
        )
        require(
            "--output=test-results/records-overlay" in e2e_run
            and "--output=test-results/demo-runtime" in e2e_run
            and "--output=test-results/dashboard-working-conversations" in e2e_run,
            "ci-pr.yml.jobs.records-overlay-e2e-producer must isolate all Playwright result directories",
        )
        require(
            "dashboard_status=0" in e2e_run
            and "dashboard_pid=$!" in e2e_run
            and 'wait "$dashboard_pid" || dashboard_status=$?' in e2e_run
            and "exit $(( records_status || demo_status || dashboard_status ))" in e2e_run,
            "ci-pr.yml.jobs.records-overlay-e2e-producer must propagate Dashboard Playwright status",
        )
        require(
            "E2E_BASE_URL=http://127.0.0.1:60083" in e2e_run,
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
            "github.event_name == 'pull_request'",
            "ci-pr.yml.jobs.build-pr-smoke-artifacts",
        )
        require_fail_closed(smoke_artifact_job, "ci-pr.yml.jobs.build-pr-smoke-artifacts")
        step_config(smoke_artifact_job, "Upload PR smoke artifacts", "ci-pr.yml.jobs.build-pr-smoke-artifacts")
        archive_job = job_config(workflow, "backend-test-archive", "ci-pr.yml")
        require(
            archive_job.get("name") == "Backend Test Archive Producer" and archive_job.get("name") in auxiliary_jobs,
            "ci-pr.yml.jobs.backend-test-archive must be the declared archive producer",
        )
        require_no_if(archive_job, "ci-pr.yml.jobs.backend-test-archive")
        require_fail_closed(archive_job, "ci-pr.yml.jobs.backend-test-archive")
        archive_build_step = step_config(archive_job, "Build backend test archive", "ci-pr.yml.jobs.backend-test-archive")
        require(
            archive_build_step.get("id") == "build-backend-test-archive",
            "ci-pr.yml.jobs.backend-test-archive: archive build step id drifted",
        )
        target_cache_save_step = step_config(archive_job, "Save Cargo test artifacts", "ci-pr.yml.jobs.backend-test-archive")
        require(
            target_cache_save_step.get("if")
            == "${{ steps.build-backend-test-archive.outcome == 'success' && steps.cargo-test-cache.outputs.cache-hit != 'true' }}",
            "ci-pr.yml.jobs.backend-test-archive: target cache must save only after a successful archive build",
        )
        for backend_job_id in ("backend-tests-lightweight", "backend-tests-stateful-sqlite", "backend-tests-archive-file-io"):
            require(
                job_config(workflow, backend_job_id, "ci-pr.yml").get("needs") == "backend-test-archive",
                f"ci-pr.yml.jobs.{backend_job_id}.needs must use the archive producer",
            )
            require_exact_if(
                job_config(workflow, backend_job_id, "ci-pr.yml"),
                "always()",
                f"ci-pr.yml.jobs.{backend_job_id}",
            )


def validate_ci_main(path: Path, contract: ContractModel) -> None:
    workflow = load_yaml(path)
    workflow_name = workflow.get("name")
    require(isinstance(workflow_name, str) and workflow_name, "ci-main.yml: workflow name must stay non-empty")
    expected_jobs = set(contract.expected_main_workflows.get(workflow_name, ()))
    require(expected_jobs, f"ci-main.yml: workflow {workflow_name!r} must be declared in expected_main_workflows")
    auxiliary_jobs = set(contract.expected_main_auxiliary_workflows.get(workflow_name, ()))
    require_exact_named_jobs(workflow, expected_jobs | auxiliary_jobs, "ci-main.yml")

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
    legacy_lint_cache = step_config(lint_job, "Restore legacy Cargo workspace cache", "ci-main.yml.jobs.lint")
    legacy_lint_cache_paths = str(legacy_lint_cache.get("with", {}).get("path", ""))
    require(
        "target" not in legacy_lint_cache_paths.splitlines(),
        "ci-main.yml.jobs.lint: legacy cache must not restore Cargo target artifacts",
    )
    _validate_ci_main_tooling(workflow, expected_jobs)
    release_snapshot = _validate_ci_main_release_jobs(workflow, expected_jobs, auxiliary_jobs)
    _validate_ci_main_release_snapshot(release_snapshot)


def _validate_ci_main_tooling(workflow: dict[str, Any], expected_jobs: set[str]) -> None:
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



def _validate_ci_main_release_jobs(workflow: dict[str, Any], expected_jobs: set[str], auxiliary_jobs: set[str]) -> dict[str, Any]:
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
        require_fail_closed(archive_job, "ci-main.yml.jobs.backend-test-archive")
        archive_build_step = step_config(archive_job, "Build backend test archive", "ci-main.yml.jobs.backend-test-archive")
        require(
            archive_build_step.get("id") == "build-backend-test-archive",
            "ci-main.yml.jobs.backend-test-archive: archive build step id drifted",
        )
        target_cache_save_step = step_config(archive_job, "Save Cargo test artifacts", "ci-main.yml.jobs.backend-test-archive")
        require(
            target_cache_save_step.get("if")
            == "${{ steps.build-backend-test-archive.outcome == 'success' && steps.cargo-test-cache.outputs.cache-hit != 'true' }}",
            "ci-main.yml.jobs.backend-test-archive: target cache must save only after a successful archive build",
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
        for shard_id, partition in (
            ("backend-tests-stateful-sqlite-shard-1", "hash:1/2"),
            ("backend-tests-stateful-sqlite-shard-2", "hash:2/2"),
        ):
            shard_job = job_config(workflow, shard_id, "ci-main.yml")
            require(shard_job.get("needs") == "backend-test-archive", f"ci-main.yml.jobs.{shard_id}.needs must use the archive producer")
            require_exact_if(shard_job, "always()", f"ci-main.yml.jobs.{shard_id}")
            shard_run = str(step_config(shard_job, "Run stateful SQLite backend profile", f"ci-main.yml.jobs.{shard_id}").get("run", ""))
            require(f"--partition {partition}" in shard_run, f"ci-main.yml.jobs.{shard_id} must run partition {partition}")
        aggregate_job = job_config(workflow, "backend-tests-stateful-sqlite", "ci-main.yml")
        require(
            aggregate_job.get("needs") == ["backend-tests-stateful-sqlite-shard-1", "backend-tests-stateful-sqlite-shard-2"],
            "ci-main.yml.jobs.backend-tests-stateful-sqlite.needs must aggregate both shards",
        )
        require_exact_if(aggregate_job, "always()", "ci-main.yml.jobs.backend-tests-stateful-sqlite")
        aggregate_run = str(step_config(aggregate_job, "Require both Stateful SQLite shards", "ci-main.yml.jobs.backend-tests-stateful-sqlite").get("run", ""))
        require("SHARD_ONE_RESULT" in aggregate_run and "SHARD_TWO_RESULT" in aggregate_run, "ci-main.yml Stateful SQLite aggregate must inspect both shard results")

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

    return release_snapshot


def _validate_ci_main_release_snapshot(release_snapshot: dict[str, Any]) -> None:
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
    _validate_release_workflow(workflow, expected_jobs)
    release_meta = _validate_release_meta_setup(workflow, expected_jobs)
    _validate_release_meta_steps(release_meta)
    _validate_release_candidates(workflow, expected_jobs)
    _validate_release_docker_arm(workflow, expected_jobs)
    _validate_release_publish(workflow, expected_jobs)


def _validate_release_workflow(workflow: dict[str, Any], expected_jobs: set[str]) -> None:
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



def _validate_release_meta_setup(workflow: dict[str, Any], expected_jobs: set[str]) -> dict[str, Any]:
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

    return release_meta


def _validate_release_meta_steps(release_meta: dict[str, Any]) -> None:
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



def _validate_release_candidates(workflow: dict[str, Any], expected_jobs: set[str]) -> None:
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



def _validate_release_docker_arm(workflow: dict[str, Any], expected_jobs: set[str]) -> None:
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



def _validate_release_publish(workflow: dict[str, Any], expected_jobs: set[str]) -> None:
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
