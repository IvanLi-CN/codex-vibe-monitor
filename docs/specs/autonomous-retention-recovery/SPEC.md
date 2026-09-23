# Autonomous Retention Recovery and Raw Capture Circuit Breaker

> This file is the durable topic requirements contract. Current implementation facts belong in `IMPLEMENTATION.md`; lifecycle and change references belong in `HISTORY.md`.

## Context and Scope

- Context: The retention pipeline can leave expired live details and raw payloads behind when archive preparation succeeds but source publication cannot complete. Repeated failures consume storage and increase SQLite write pressure while manual intervention is unavailable.
- In scope: Autonomous, pressure-aware recovery of expired invocation retention; verified archive publication; raw payload storage protection; and sanitized owner-facing recovery diagnostics.
- Out of scope: Upstream network latency, upstream provider capacity, proxy routing policy, manual maintenance commands, process restarts, and SQLite `VACUUM`.

## Terms and Interfaces

- **Verified Archive**: An immutable archive whose artifact, manifest, Summary proof, and source mutation have committed as one publication. Only it authorizes removal of its corresponding live detail and raw payload files.
- **Prepared Archive**: An immutable artifact with recorded source identity and digest that is eligible for recovery verification but does not authorize source deletion.
- **Physical Raw Inventory**: The incrementally reconciled byte count for raw files physically owned by the configured raw store, including bounded in-flight reservations.
- **Raw Capture Circuit Breaker**: A hysteretic state that suppresses new raw request and response files while preserving proxy delivery and structured invocation facts.
- Interface: Existing retention maintenance lifecycle, raw-capture write path, runtime configuration, and additive `/api/system/status` health fields.

## Requirements

### REQ-ARR-001

- The system MUST release a live invocation detail or its raw-owner link only in the same finalized source transaction that publishes its corresponding Verified Archive.
- Inputs: An expired invocation candidate, archive artifact, manifest, Summary proof, and source rows.
- Outputs: Either an atomically finalized archive plus source transition, or unchanged live source ownership.
- A `live_mirror` detail-prune archive is a prepared, source-identity-verified recovery artifact whose live canonical row remains online; it MUST NOT become a Summary or rollup authority. Its manifest, prepared-ledger removal, structured-field transition, and raw-owner release still commit together.

### REQ-ARR-002

- The system MUST persist bounded recovery state for every Prepared Archive and resume it automatically after process interruption, archive I/O failure, or SQLite admission deferral.
- A recovery pass MAY reuse an artifact only after exact source-identity and digest verification. Invocation identity covers every archive-column value, including its SQLite storage type; both the live source rows and the rows in the artifact MUST match. An unverified artifact MUST NOT authorize source or raw deletion.
- A legacy or unmatched artifact MUST be handled by a resumable, bounded reconciliation cursor that wraps after exhausting the current tail so files arriving behind the saved cursor are eventually revisited. It is retained as quarantined evidence for 24 hours and may then be removed only when it matches neither a Prepared Archive nor a committed manifest.
- Legacy reconciliation MUST obtain background write admission before traversing the archive directory or performing archive-file I/O. It MUST stream directory entries through a selection structure whose retained allocation, comparison work per entry, and candidate set are bounded by the scan batch rather than materializing and sorting a complete directory. Directory cursors MUST preserve the lexical stream position between same-prefix files and descendants; a truncated directory MUST pause traversal at its selected boundary, and cursor persistence MUST be monotonic with a conditional tail wrap so overlapping passes cannot skip or regress legacy artifacts.

### REQ-ARR-003

- The system MUST make archive finalization, raw-owner release, filesystem-safe inventory reset, and Prepared Archive reconciliation independently resumable stages of one Retention Recovery lifecycle. PR2 MUST NOT enumerate or delete unlinked raw residuals; filesystem availability remains the safety signal for those files.
- A failure in one stage MUST report that stage and schedule bounded retry/backoff without preventing a separately safe stage from making progress.
- All database mutations in this lifecycle MUST retain maintenance write admission and MUST yield to P1 terminal and interactive proxy writes.

### REQ-ARR-004

- The system MUST suppress new raw request and response files when either physical raw storage reaches `16 GiB` or filesystem free space falls to `20 GiB` or lower.
- It MUST resume raw capture only after physical raw storage is below `12 GiB` and filesystem free space is at least `30 GiB`.
- While suppressed, the system MUST continue proxy delivery and durable structured invocation facts, and MUST record a storage-suppression reason rather than representing the event as a per-payload truncation.
- The circuit-breaker decision MUST use Physical Raw Inventory and bounded write reservations; it MUST NOT perform a full directory scan in the proxy request path.
- A new-format overflow-spool capture MUST be replayed only when its completion marker proves the recorded final segment; a missing or mismatched marker MUST retain the validated segments for a later bounded recovery pass. A bounded spool inventory overflow MUST remain observable as inventory preparation rather than inventory readiness.

### REQ-ARR-005

- The system MUST autonomously work through all expired historical raw/detail backlog under the existing retention policy until each candidate has either finalized a Verified Archive or remains in an explicit, observable recoverable state.
- Returning to normal raw capture MUST require both the recovery watermarks in REQ-ARR-004 and a non-growing expired backlog. Sustained P1 load may keep recovery and raw capture suppressed; it MUST NOT force a larger or higher-priority retention transaction.

### REQ-ARR-006

- `/api/system/status` and structured logs MUST expose low-cardinality Retention Recovery state: circuit-breaker state and reason, physical raw bytes and watermarks, filesystem free bytes, backlog counts/age, current stage, last successful progress, retry time, and a sanitized failure fingerprint.
- Prepared, quarantined, and expired-backlog counts MUST be null or omitted until a successful status refresh measures them; an unmeasured count MUST NOT appear as zero.
- These diagnostics MUST NOT expose raw request/response content, SQL text or bindings, account identifiers, or full payload/archive paths.

### REQ-ARR-007

- Autonomous Retention Recovery MUST NOT invoke maintenance CLI commands, restart the process, run `VACUUM`, or delete database files.
- It MAY release raw-file and orphan/prepared-artifact storage only when the proof requirements in REQ-ARR-001 and REQ-ARR-002 hold. SQLite pages made reusable by row deletion are not a promise of immediate database-file shrinkage.

## Verification

### VER-ARR-001

- Method: Stateful SQLite and archive-file I/O fault-injection tests at each publication boundary.
- covers: `REQ-ARR-001`, `REQ-ARR-002`
- Pass condition: Interruptions before final publication leave live rows and raw-owner links intact; a matching Prepared Archive is reused after exact verification; an unmatched artifact cannot finalize or delete a source row.

### VER-ARR-002

- Method: Scheduler tests that fail archive finalization while allowing orphan sweep and recovery-journal work.
- covers: `REQ-ARR-003`
- Pass condition: Each safe stage remains independently resumable, retry/backoff is bounded, and P1 or interactive writes retain coordinator priority.

### VER-ARR-003

- Method: Raw-capture integration tests using a bounded Physical Raw Inventory and mocked filesystem free-space values.
- covers: `REQ-ARR-004`, `REQ-ARR-005`
- Pass condition: Capture stops at either high condition, resumes only after both low conditions, preserves proxy and structured records, and does not scan the raw directory on a request.

### VER-ARR-004

- Method: System-status serialization and structured-log inspection tests.
- covers: `REQ-ARR-006`
- Pass condition: Required recovery fields are present with stable low-cardinality values and failure output contains no payload, SQL, account, or full-path data.

### VER-ARR-005

- Method: Static behavior tests and maintenance-lifecycle fixtures.
- covers: `REQ-ARR-007`
- Pass condition: Automatic paths never invoke a CLI, restart, `VACUUM`, or database-file deletion; only proven raw/artifact candidates are removed.

## Related ADRs

- [ADR 0004: Summary archive publication proof](../../adr/0004-summary-archive-publication-proof.md)
- [ADR 0015: Coordinated runtime SQLite write admission](../../adr/0015-coordinated-runtime-sqlite-write-admission.md)
- [ADR 0016: Autonomous raw capture circuit breaker](../../adr/0016-autonomous-raw-capture-circuit-breaker.md)

## Visual Evidence

- source_type: ui_demo
  target_program: mock-only
  capture_scope: browser-viewport
  requested_viewport: 1280x577
  viewport_strategy: devtools-emulate
  margin_policy: trim_only
  evidence_surface: page
  state: degraded recovery with `status_refresh` stage and failure
  evidence_note: shows the recovery backlog and sanitized failure fingerprint in System Status
  image:
  ![Desktop System Status retention recovery](./assets/retention-recovery-desktop.png)
- source_type: ui_demo
  target_program: mock-only
  capture_scope: browser-viewport
  requested_viewport: 393x852
  viewport_strategy: devtools-emulate
  margin_policy: trim_only
  evidence_surface: page
  state: degraded recovery with `status_refresh` stage and failure
  evidence_note: shows the recovery diagnostics within the mobile System Status viewport
  image:
  ![Mobile System Status retention recovery](./assets/retention-recovery-mobile.png)
- Healthy, recovering, degraded, and missing-field `unknown` states are covered by System Workspace Storybook interactions.
- source_type: storybook_canvas
  target_program: mock-only
  capture_scope: element
  requested_viewport: 1660x900
  viewport_strategy: storybook-viewport
  margin_policy: trim_only
  evidence_surface: page
  state: storage-suppressed raw capture with filesystem-low reason
  evidence_note: shows the raw capture circuit state, inventory, physical raw usage, available space, reservation, and backlog trend in Runtime Pressure.
  image:
  ![Desktop System Status raw capture suppressed](./assets/raw-capture-suppressed-desktop.png)
- source_type: storybook_canvas
  target_program: mock-only
  capture_scope: element
  requested_viewport: 1660x900
  viewport_strategy: storybook-viewport
  margin_policy: trim_only
  evidence_surface: page
  state: missing raw capture diagnostic fields normalized to unknown
  evidence_note: verifies additive compatibility when older backends do not publish the raw capture contract.
  image:
  ![Desktop System Status raw capture unknown](./assets/raw-capture-unknown-desktop.png)

## References

- `./IMPLEMENTATION.md`
- `./HISTORY.md`
- `../9aucy-db-retention-archive/SPEC.md`
- `../../archive/specs/t4v9k-retention-backlog-root-cause-fix/SPEC.md`
