# Observability Acceptance Single-Runner Bundle

Status: Superseded by ADR 0048

## Context

The hosted `ubuntu-24.04` label does not guarantee that independent matrix jobs receive the same CPU model, kernel, or runner image version. The required five-field hardware check correctly rejected the post-merge candidate evidence after pair-0 and pair-2 used AMD EPYC 7763 while pair-1 used AMD EPYC 9V74, and pair-2 also used a different kernel and image version. There is no repository self-hosted runner group available to pin three compatible machines.

## Decision

- Keep the exact five-field hardware equality check, the nine measured windows, the 5% CPU/request and successful-request-p95 budgets, the CV stability limit, request completion checks, trace checks, and all existing resource-admission rules.
- Keep production image construction in a separate GitHub-hosted job. Run the runtime suite and all three paired suites sequentially in one explicit GitHub-hosted acceptance job so the evidence shares one Actions job identity and one runner for the complete experiment.
- Require every paired part to record `runnerScope=serial-bundle` and the same `runnerIdentity`; a different identity or any hardware mismatch remains unavailable and cannot be pooled.
- Start the shared acceptance clock at the beginning of the serial measurement job and bound the complete checkout, image load, sampler preparation, runtime suite, three pairs, cleanup, and evidence handoff to 60 minutes. The paired window shape remains 60 seconds of warmup plus 60 seconds of measurement at 5 req/s.
- Keep the performance workflow explicit-only. No automatic retry, delay, window selection, local VM result, or self-hosted resource is allowed to turn unavailable evidence into a passing certificate.

## Consequences

The workflow no longer relies on a random cross-job hardware match, and the same-runner identity is independently visible in every part artifact. The serial bundle costs more wall time and gives the host more opportunity to drift, so the existing per-window admission and CV checks remain mandatory; a 60-minute bound is a timing change only, not a relaxed acceptance threshold.

## Verification and Re-entry

Deterministic tests cover serial runner scope, shared runner identity, hardware mismatch, missing evidence, stale candidate identity, deadline enforcement, and the workflow artifact whitelist. A merged candidate that changes the trusted harness or acceptance contract must use the `main` `workflow_dispatch` path with its complete `candidate_sha`. Only a complete bundle with all four parts, nine valid windows, identical runner identity and five hardware fields, and passing thresholds can issue the empirical card.
