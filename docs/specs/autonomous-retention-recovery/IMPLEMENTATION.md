# Autonomous Retention Recovery and Raw Capture Circuit Breaker Implementation

> The durable requirements remain in `./SPEC.md`; this file records implementation facts and delivery boundaries.

## PR1: Autonomous Retention Recovery

### Durable Archive Recovery

- SQLite stores prepared archive identity, source IDs and identity digest, state, artifact digest/size, retry count/deadline, and sanitized failure fingerprint in `retention_prepared_archives`.
- The existing deterministic archive path and same-SHA reuse / different-SHA conflict rules remain authoritative. Invocation archive preparation records source identity before artifact creation and marks the artifact `published` only after its digest is recorded.
- Final publication verifies the prepared journal, artifact digest, and live source identity in the source transaction. The manifest/proof transition, source-row deletion, and journal removal commit together; a failed transaction retains live rows, raw files, and the journal for retry.
- `retention_recovery_cursors` advances legacy-segment reconciliation. Each maintenance pass considers at most 32 archive segment files, verifies their contents and source identity before adopting them, and quarantines unmatched artifacts. Quarantined files are eligible for removal only after 24 hours and only when no manifest or other active prepared journal references them.
- Recovery and orphan cleanup use maintenance-class writes. A failed invocation archive stage no longer prevents the independent raw-file orphan sweep from running.

### Health Contract and Interface

- `/api/system/status` adds optional `runtimePressureHealth.retentionRecovery` diagnostics for health state, current stage, prepared/quarantined/expired backlog counts and age, last progress, next retry, and a sanitized failure fingerprint.
- Older or incomplete responses normalize the recovery state to `unknown`. System Status exposes healthy, recovering, and degraded scenarios alongside Runtime Pressure details.
- Status fields, UI copy, and recovery logs omit raw content, SQL/bindings, account identifiers, and complete paths.

## PR2 Boundary

PR1 does not implement the aggregate physical raw inventory or the raw-capture circuit breaker. The 16 GiB / 12 GiB raw-store thresholds and 20 GiB / 30 GiB filesystem watermarks, their capture suppression reason, and their System Status fields remain in `SPEC.md` and are reserved for the serial PR2 based on PR1's merge commit.

## Operational Boundaries

- Recovery requires no operator command, manual deletion, process restart, or `VACUUM`.
- The archive file format and deterministic path contract are unchanged. SQLite row deletion may make pages reusable but does not guarantee a smaller database file.
- This work reduces retention failure amplification; it does not establish a cause for observed upstream throughput changes or the previously observed approximately 50 GiB project footprint.

## Verification

- `cargo fmt --all -- --check`, Linux `cargo check --locked --all-targets --all-features`, and Linux Clippy with `-D warnings` pass.
- The `stateful-sqlite` profile passes 1303 tests, including transaction rollback/retry, prepared-schema re-entry, serialized status diagnostics, and the 32-file legacy reconciliation bound.
- The `archive-file-io` profile passes 246 tests.
- Web unit tests pass 1536 tests across 156 files, with 6 skipped. Type checking, Storybook recovery interactions (13 tests), lint, and production build pass. Repo-wide lint reports 86 existing warnings; the build reports the existing large-bundle warning.

## References

- `./SPEC.md`
- `./HISTORY.md`
- `../../adr/0016-autonomous-raw-capture-circuit-breaker.md`
