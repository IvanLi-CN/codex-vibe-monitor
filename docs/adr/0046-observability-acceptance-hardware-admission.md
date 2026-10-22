# Observability Acceptance Hardware Admission

Status: Accepted

## Context

The observability acceptance contract compares three independent pair runners across CPU model, CPU count, kernel, runner image, and runner image version. The `ubuntu-24.04` GitHub-hosted label identifies a runner image family, but it does not establish physical hardware equality. The latest investigated run, `39717762452` attempt 1, observed AMD EPYC 7763 on pair-0 and pair-2 versus AMD EPYC 9V74 on pair-1, with all three pair preparation paths also reporting `cpu_read_failed`; no comparable A/B sample existed. A read-only GitHub Actions inventory currently reports zero self-hosted runners, and the repository workflow has no dedicated runner group or pinned hardware admission.

## Decision

- Keep the exact five-field hardware equality check. Any mismatch makes the empirical result unavailable and must never pool incompatible samples or replace the mismatch with a random hosted-runner assumption.
- Preserve independently verified runtime scenarios and pair-level preparation diagnostics when hardware compatibility fails. Functional failures, preparation failures, evidence failures, and hardware errors remain visible and cannot be converted into a pass.
- Treat the current hosted workflow as capable of testing the tool contract only. A successful workflow boundary or deterministic regression does not prove a 5% CPU/request or successful-request-p95 budget.
- The future low-cost preflight design is a three-pair admission job that collects the five required hardware fields before image load and measurement, compares them in the trusted base checkout, and records a bounded unavailable result before expensive windows when they differ. It is an admission and cost gate, not a contract relaxation.
- A real 5% proof requires an owner-authorized dedicated runner group with pinned CPU/image/kernel characteristics or an explicitly approved external resource. A different within-runner or statistical experiment requires a successor ADR and acceptance-contract approval.
- This decision does not configure paid or external runners, change thresholds or statistics, or rerun the historical label event.

## Consequences

The acceptance card remains honest when hosted infrastructure is heterogeneous, and runtime functional evidence is not erased by a later hardware decision. A future preflight can avoid spending the full window budget on an obviously incomparable set, but it cannot certify a result by itself. Until comparable resources are available, the release decision must state that empirical acceptance is unavailable and include the exact rerun entry and required evidence.

## Verification and Re-entry

Deterministic tests cover future `cpu.stat` fields, bounded preparation diagnostics, runtime/A-B/hardware aggregation, and mismatch combinations with both successful and failing runtime scenarios. The exact empirical re-entry is a fresh `run:observability-performance` label on a candidate whose trusted base checkout contains the acceptance fixes, followed by one complete build, runtime, pair-0, pair-1, and pair-2 result set. All five hardware fields must match, all nine windows must be valid, and the shared timing, request, trace, CPU, p95, and stability thresholds must pass; otherwise the result remains unavailable or failed according to the existing contract.
