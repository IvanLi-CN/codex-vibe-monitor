# Observability Acceptance Parallel Workload Lanes

Status: Accepted

Supersedes: [ADR 0047: Observability Acceptance Single-Runner Bundle](0047-observability-acceptance-single-runner-bundle.md)

## Context

The serial acceptance bundle runs the runtime scenarios and three independent paired workloads one after another. The paired workload owns its complete `off`, `metrics`, and `full` sequence, including warmup, quiet admission, measurement, trace checks, CPU attribution, and cleanup. The workloads do not share a database, Compose project, fixture directory, or measured process, so serializing different workloads adds wall time without making the within-workload comparison more valid.

GitHub-hosted `ubuntu-24.04` runners are still heterogeneous. Independent runners may report different CPU models, CPU counts, kernels, runner images, or image versions. Parallel execution is therefore valid only when each workload is isolated and the trusted aggregator compares the complete hardware signature before pooling pair samples.

## Decision

- Build the immutable candidate image once and distribute it to four independent hosted workload jobs: `runtime`, `pair-0`, `pair-1`, and `pair-2`.
- Emit one acceptance start timestamp after the image artifact is published. Every workload consumes that timestamp; checkout, image loading, sampler preparation, execution, cleanup, and evidence handoff remain inside the same absolute 60-minute acceptance phase. No lane may restart or extend the clock.
- Use a matrix with `fail-fast: false` and four concurrent workload slots. Each job owns its `$RUNNER_TEMP` child, Compose project, SQLite database, fixture state, credentials, sampler, and CPU admission. The three modes within each paired workload remain serial in `run.py`.
- Record `runnerScope=parallel-<lane>` and a lane-specific `runnerIdentity`. The result job must receive exactly one part for every lane, bind every part to the same candidate SHA, trusted harness, candidate/trusted contract and harness digests, image ID, run/attempt, evidence locator, and shared start timestamp, and reject missing, stale, mixed, duplicated, or malformed evidence.
- Compare the three paired workloads only when their runner identities are distinct and all five hardware fields (`cpuModel`, `cpuCount`, `kernel`, `runnerImage`, and `runnerImageVersion`) are identical. Any mismatch remains unavailable; no samples are pooled and no threshold is relaxed.
- Keep the existing three 60-second warmups and measurements per pair, 5 request/s rate, request/trace/CPU evidence, CV stability rule, 5% CPU/request and successful-request-p95 budgets, pressure admission rules, and failure classification unchanged.

## Consequences

The critical path becomes image publication plus workload preparation and the longest workload lane, subject to GitHub runner queueing and artifact transfer. The workflow can still take the full 60-minute bound when hosted capacity is delayed or a lane exhausts its admission budget. Hardware heterogeneity remains an expected unavailable outcome, and cross-runner parallelism does not provide physical hardware pinning or a performance pass by itself.

The workflow retains `cancel-in-progress: false` and disables matrix fail-fast so a failed or unavailable lane does not cancel sibling evidence. The final result job always waits for all workload jobs and fails closed when cancellation, timeout, or runner loss leaves any required part absent.

## Verification and Re-entry

Deterministic tests cover the matrix lane set, shared clock propagation, fail-fast/cancellation behavior, candidate and image binding, distinct parallel identities, five-field hardware comparison, missing or duplicate parts, stale contract evidence, and failure propagation. A candidate that changes the trusted acceptance harness or contract must use the `main` `workflow_dispatch` path with its complete SHA after merge; PR label runs may only report unavailable evidence for that candidate. A valid certificate still requires all four workload parts, nine valid paired windows, identical paired hardware fields, and all existing thresholds.
