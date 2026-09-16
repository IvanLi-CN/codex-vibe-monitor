from __future__ import annotations


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
