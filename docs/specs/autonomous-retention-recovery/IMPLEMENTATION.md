# Autonomous Retention Recovery and Raw Capture Circuit Breaker Implementation

> The durable requirements remain in `./SPEC.md`; this file records implementation facts and delivery boundaries.

## PR1: Autonomous Retention Recovery

### Durable Archive Recovery

- SQLite stores prepared archive identity, source IDs and identity digest, state, artifact digest/size, retry count/deadline, and sanitized failure fingerprint in `retention_prepared_archives`.
- The existing deterministic archive path and same-SHA reuse / different-SHA conflict rules remain authoritative. Invocation archive preparation hashes every archive-covered column together with its SQLite storage type and value. The writer hashes the actual rows copied into the artifact, and a mismatch is quarantined before publication.
- Final publication verifies the prepared journal, artifact digest, and full live-source identity inside the source transaction. The manifest/proof transition, source-row deletion, and journal removal commit together; a failed transaction retains live rows, raw files, and the journal for retry.
- Detail-prune `live_mirror` archives now use a deterministic mirror-only path, the same prepared ledger and source-identity verification, and a single transaction for the mirror manifest, structured-only transition, raw-owner release, and ledger removal. They remain excluded from Summary/rollup authority and do not emit an authoritative Summary proof; a defensive manifest upsert refuses to let a live mirror alter an existing authoritative manifest.
- Expired `live_mirror` manifests use an independent TTL cleanup path that does not wait for Summary proof or advance an authoritative source boundary. The finalizer rechecks the role and file digest under the writer transaction before removing the file and manifest.
- Archive publishers and cleanup finalizers share a directory-level filesystem lock in addition to SQLite admission; mirror cleanup also requires an archive-root ownership proof. File hashing happens while the directory fence is held but before SQLite admission, so large artifacts do not occupy the maintenance writer slot. Legacy reconciliation first probes pressure and write admission before archive discovery, then releases the short-lived permit before verification so large legacy artifacts do not occupy the maintenance writer slot. Legacy replacement records its restore path in the prepared ledger or `archive_batches` before rename, updates the manifest digest in the same replacement transaction, and recovers by either removing a committed rollback copy or restoring it before retry; an ambiguous commit is left for this recovery pass instead of guessing. Missing archive parents are treated as an idempotent absent-file state for metadata cleanup, while historical replay re-locks and rechecks the current artifact SHA before materialization.
- Legacy archive recovery upgrades older `codex_invocations` archive schemas before identity reads, preserves prepared layout metadata for exact segment checks, verifies artifact rows against the recorded source IDs and identity digest (including verified supersets for appendable legacy-month files), and rejects out-of-root, parent-traversal, or symlink journal paths.
- A verified orphaned legacy segment is adopted as a completed `live_mirror` manifest and its prepared journal is retired in the same maintenance transaction; unverified files remain quarantined.
- Legacy-month append reuses an existing artifact only when every recorded manifest or prepared-ledger digest still matches the file. A bounded legacy detail reconciliation also checks older authoritative month manifests: a fully live identity match is downgraded to `live_mirror`, while a partial match is fail-closed as `unknown` for proof recovery.
- `retention_recovery_cursors` advances legacy-segment reconciliation. Each maintenance pass considers at most 32 archive segment files and streams directory entries through a fixed-size selection heap; only the bounded selected entries are retained and sorted, and a truncated directory pauses traversal at its selected boundary so later entries are not skipped. Candidate contents and source identity are verified before adoption and unmatched artifacts are quarantined. Cursor writes are monotonic and tail wrap is conditional on the observed cursor, so overlapping passes cannot regress progress and newly arrived earlier paths are not starved. Quarantined files are eligible for removal only after 24 hours and only when no manifest or other active prepared journal references them. Prepared reconciliation selects due active journals and eligible expired quarantines, prioritizing published artifacts.
- Recovery and orphan cleanup use maintenance-class writes. A failed invocation archive stage no longer prevents the independent raw-file orphan sweep from running.

### Health Contract and Interface

- `/api/system/status` adds optional `runtimePressureHealth.retentionRecovery` diagnostics for health state, current stage, prepared/quarantined/expired backlog counts and age, last progress, next retry, and a sanitized failure fingerprint.
- Older or incomplete responses normalize the recovery state to `unknown`. Counts remain null until a successful database refresh measures them; System Status normalizes null or missing counts to unknown and exposes healthy, recovering, and degraded scenarios alongside Runtime Pressure details.
- Missing or partial backlog/prepared/quarantined counters remain unknown in the UI rather than being presented as zero.
- Status fields, UI copy, and structured recovery logs expose the recovery snapshot fields without raw content, SQL/bindings, account identifiers, or complete paths.

## PR2 Boundary

PR1 does not implement the aggregate physical raw inventory or the raw-capture circuit breaker. The 16 GiB / 12 GiB raw-store thresholds and 20 GiB / 30 GiB filesystem watermarks, their capture suppression reason, and their System Status fields remain in `SPEC.md` and are reserved for the serial PR2 based on PR1's merge commit.

## Operational Boundaries

- Recovery requires no operator command, manual deletion, process restart, or `VACUUM`.
- The archive file format and deterministic path contract are unchanged. SQLite row deletion may make pages reusable but does not guarantee a smaller database file.
- This work reduces retention failure amplification; it does not establish a cause for observed upstream throughput changes or the previously observed approximately 50 GiB project footprint.

## Verification

- `cargo fmt --all -- --check`, Linux `cargo check --locked --all-targets --all-features`, and Linux Clippy with `-D warnings` pass.
- The `lightweight` profile passes 1113 tests, including the rule that maintenance fairness cannot bypass an interactive SQLite writer; archive-specific recovery tests run in the disjoint archive-file-I/O profile.
- The `stateful-sqlite` profile passes 1305 tests, including P1 admission for failure persistence, prepared-schema re-entry, unmeasured status counts, due published work past 32 unexpired quarantines, and successful live-mirror ledger retirement.
- The `archive-file-io` profile passes 268 tests, including full archive-column identity mismatch rollback, retry after publication failure, source/raw ownership retention, independent orphan cleanup, sanitized failure fingerprints, prepared-key failure attribution, the authoritative-manifest preservation guard, the republish-safe quarantine guard, admission-before-I/O legacy recovery, bounded directory-entry discovery, monotonic cursor advancement, the referenced-window and truncated-sibling starvation guards, the 32-file bound, cursor wrap for late earlier paths, and expired live-mirror cleanup without Summary proof.
- The archive-file-I/O coverage also verifies expired live-mirror cleanup without Summary proof and bounded legacy identity reconciliation.
- Web unit tests pass 1538 tests across 156 files, with 6 skipped. Type checking and a focused Biome check pass. Storybook (91 tests) and production build passed for the System Status recovery surface and its translations; the change adds the recovery panel and status-region accessibility semantics without changing unrelated pages. Repo-wide lint from the preceding candidate reported 86 existing warnings; the build reported the existing large-bundle warning.

## References

- `./SPEC.md`
- `./HISTORY.md`
- `../../adr/0016-autonomous-raw-capture-circuit-breaker.md`
