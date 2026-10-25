# Manual Observability Performance Acceptance

Status: Accepted

## Context

Observability performance acceptance is an expensive empirical experiment. It consumes multiple GitHub-hosted runners, builds and transfers an immutable image, and performs a bounded multi-lane workload. A label event is therefore an unsafe scheduler: adding, removing, or restoring a label can start another expensive run without a new code decision.

The experiment also needs two distinct sources of truth. The acceptance workflow and its validation scripts define the trusted control plane, while the candidate runtime is the source revision being measured. A pre-merge run must be able to measure arbitrary pull request code, but that code must not be able to redefine the acceptance criteria used to judge it.

## Decision

- Start observability performance acceptance only through `workflow_dispatch` from the trusted `main` workflow. Remove the `pull_request_target` label trigger and all label-based job conditions. `run:observability-performance` is no longer an active scheduler or acceptance prerequisite.
- Replace the `candidate_sha` input with `candidate_ref`. Accept any same-repository branch name or full 40-character commit SHA as the measured runtime source. Resolve a branch to one immutable candidate SHA at the beginning of the run and bind checkout, image, workload evidence, and the final acceptance card to that SHA.
- Keep the trusted harness separate from the candidate runtime. The workflow and acceptance scripts run from the trusted `main` revision; `candidate_ref` supplies the PR, branch, or commit code executed inside the isolated candidate image. Changes to acceptance criteria in the candidate do not affect the pre-merge decision. After such harness changes merge, run the updated `main` harness explicitly against the merged candidate for re-validation.
- Remove the historical CPU diagnostics job and its fixed pull request number conditions. Do not replace it with a new automatic or legacy manual path.
- Allow manual reruns when an owner chooses to investigate a failure or unavailable result. No automatic retry, delayed retry, or label change may schedule another acceptance experiment.

## Consequences

Manual dispatch makes the cost of an acceptance run explicit and prevents repeated label events from consuming hosted runner capacity. Operators must provide a candidate reference, and branch references are intentionally pinned so a moving branch cannot mix code or evidence within one run.

Pull request runtime code can be measured before merge, while the trusted harness remains protected from candidate-controlled acceptance changes. A PR that changes the harness has an additional post-merge re-validation step. Existing label-related history remains historical documentation; deleting the remote label is outside this decision.

## Verification and Re-entry

Deterministic contract tests must assert that the workflow has only the manual trigger, has no label-based conditions, accepts `candidate_ref` branch and full-SHA forms, pins branch resolution before measurement, and keeps trusted harness and candidate runtime identities distinct. They must also assert that the diagnostics job and its historical PR bindings are absent.

The parallel workload, hardware admission, evidence completeness, and threshold rules remain governed by [ADR 0046](0046-observability-acceptance-hardware-admission.md), [ADR 0047](0047-observability-acceptance-single-runner-bundle.md), [ADR 0048](0048-observability-acceptance-parallel-workload-lanes.md), and REQ-016/REQ-016A. This ADR changes scheduling and source-of-truth boundaries, not the empirical budget or measurement contract.
