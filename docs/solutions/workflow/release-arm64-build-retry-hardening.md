---
title: "Release arm64 smoke builds should retry transient registry failures"
module: "release automation"
problem_type: "transient registry dependency failure"
component: "GitHub Actions multi-arch release build"
tags:
  - github-actions
  - release
  - docker-buildx
  - arm64
  - retry
status: "active"
related_specs: []
symptoms:
  - "Release fails in `Build + Smoke + Push Candidate (linux/arm64)` while amd64 succeeds."
  - "The failing build dies while resolving a base image or registry metadata, often with `DeadlineExceeded` or `context deadline exceeded`."
  - "The failing build dies inside Cargo dependency resolution with `curl failed` / `Error in the HTTP2 framing layer`, then succeeds on rerun."
root_cause: "The arm64 release smoke build treated transient Docker Hub or registry metadata timeouts as terminal failures, so one flaky upstream fetch aborted the entire publish path."
resolution_type: "retry-hardening"
---

# Release arm64 build retry hardening

## Context

The release workflow publishes both `linux/amd64` and `linux/arm64` images before it creates the final manifest tags. The arm64 lane runs on a native arm runner, but it still depends on external registry metadata fetches for base images and build cache layers.

## Symptoms

- `Release` fails even though the application code and the amd64 release lane are healthy.
- The failing step is `Build smoke image (linux/arm64, load)`.
- The error surface points at registry access rather than Dockerfile logic, for example `failed to authorize`, `DeadlineExceeded`, `context deadline exceeded`, or similar network timeouts.
- Cargo dependency fetches inside the Docker build can show the same transient class via `curl failed` or `Error in the HTTP2 framing layer` while resolving `crates.io`.
- Rerunning the same workflow often succeeds without any code change.

## Root Cause

The old arm64 smoke build used a single `docker/build-push-action` attempt. That is fine for deterministic Dockerfile failures, but it is too brittle for transient upstream registry failures. When Docker Hub or another registry stalls during metadata resolution, the release path should retry the fetch instead of treating the first timeout as a product regression.

## Resolution

Move the arm64 smoke build behind a repo-owned retry helper and validate that contract in CI:

- Run the arm64 smoke build through `.github/scripts/build-smoke-image-with-retry.sh`.
- Retry only known transient registry/network failures such as `DeadlineExceeded`, `context deadline exceeded`, TLS handshake timeouts, connection resets, unexpected EOF, and rate-limit style fetch failures.
- Treat Cargo's transient registry fetch surfaces the same way when the build log shows `curl failed` or `Error in the HTTP2 framing layer` for a dependency download.
- Keep non-transient build failures fail-closed on the first attempt so real Dockerfile or packaging regressions stay loud.
- When the release queue can backfill older pending commits, stage workflow-owned helpers into the target checkout before the arm64 build so historical targets do not fail just because the helper file was introduced later.
- Add a dedicated script regression test that proves both paths:
  - transient registry failures retry and eventually succeed;
  - permanent image-resolution failures stop immediately without looping.
- Add workflow-contract assertions so the release topology cannot silently drift back to a non-retrying arm64 build step.

## Guardrails / Reuse Notes

- Keep retry classification small and explicit. A helper that retries everything will hide real build breakages.
- Put retry policy in a repo-owned script instead of duplicating shell loops inline in workflow YAML; this keeps release, tests, and contract fixtures aligned.
- If the workflow needs to run against older release targets, keep any helper files outside the target checkout or source them from the workflow revision so the queued backfill stays compatible.
- When a release job fails in only one architecture lane, inspect whether the error happens before the real Dockerfile steps begin. If yes, suspect registry flakiness before suspecting app code.
- When the build fails during the Cargo warm-up layer, distinguish dependency-fetch transport errors from compile errors. Transport noise should extend the retry classifier; deterministic compiler failures should still stop immediately.
- For workflow-level resilience changes, update both live workflows and `quality-gates-contract` fixtures in the same patch. Otherwise CI may accept topology drift or reject the intended contract.

## References

- `.github/workflows/release.yml`
- `.github/scripts/build-smoke-image-with-retry.sh`
- `.github/scripts/test-build-smoke-image-with-retry.sh`
- `.github/scripts/check_quality_gates_contract.py`
