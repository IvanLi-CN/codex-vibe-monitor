# Autonomous Retention Recovery and Raw Capture Circuit Breaker History

> This file records topic lifecycle, compatibility, and necessary background. The current requirements remain in `./SPEC.md`.

## Lifecycle / Compatibility

- The additive System Status storage view distinguishes tracked linked-file bytes from unknown physical raw-store coverage; older clients may ignore `physicalCoverage`, while newer clients fail closed for missing raw bytes.
- This active topic extends, rather than replaces, `9aucy-db-retention-archive`: that topic continues to own the baseline retention tiers, archive/rollup contract, and normal maintenance admission.
- Existing deployments retain per-payload `PROXY_RAW_MAX_BYTES` behavior. The new breaker is an aggregate-store policy and adds an explicit structured omission reason when it suppresses raw capture.
- System-status additions are additive. Existing clients may ignore them without changing proxy or structured invocation behavior.

## Replacements / Background

- The archived retention-backlog work established raw-codec and bounded-catchup behavior, but it did not establish a durable recovery journal, global physical storage budget, filesystem reserve, or autonomous reconciliation of pre-publication artifacts.
- This topic adopts the archive-publication proof and coordinated write-admission boundaries while making recovery progress independently observable and resumable.

## Related Changes

- PR1 delivers autonomous archive recovery and System Status diagnostics on `th/autonomous-retention-recovery`.
- The raw-reference finalization follow-up moves candidate ownership confirmation into the source transaction and keeps physical unlink after commit, preserving the accepted write-admission boundary.
- Raw-reference finalization normalizes relative database roots before checking legacy relative/absolute ledger aliases, including both `.bin` and `.bin.gz` variants.
- The ownership proof also checks cwd-relative raw-path representations before any post-commit unlink.
- The ownership proof resolves absolute fallback roots and prefixed relative paths without double-prefixing the database root.
- PR2 adds the bounded physical raw inventory and capture circuit breaker as an additive extension to
  the retention metrics ledger; it preserves the existing proxy, archive, and structured-record
  contracts while exposing fail-closed storage health through System Status.
- PR3 adds a bounded, restart-safe raw residual reconciliation stage. It preserves original raw
  paths during quarantine and releases only candidates with durable identity, owner-reference, and
  quarantine proof.
- The retention-recovery-liveness follow-up makes prepared publication kind explicit, resumes valid
  `published` rows through their original final transaction, and retires expired quarantined archive
  artifacts without requiring a digest when archive-root ownership and manifest/reference absence
  are proven. Its dedicated scheduler cursor persists pressure defers, bounded retry backoff, and
  progress independently of raw inventory reset work.
- The raw-orphan-sweep throughput follow-up replaces full-directory selection and per-file owner-table
  scans with a process-local bounded iterator, indexed batched blob-link checks, a separately
  scheduled worker, and observable retry/progress state. Bounded filesystem inspections retain the
  background pressure slot but release the SQLite write-coordinator permit; final unlink locking is
  nonblocking while maintenance admission is held, and an item-level lock or unlink failure does
  not prevent later candidates in the same slice from running.
- The raw-orphan-sweep admission-liveness follow-up adds FIFO background-slot and bounded
  maintenance-write admission, fixed sanitized stage/cause evidence, pending-candidate retention
  across mid-pass deferral, and a two-second candidate budget with one-second progress cadence. It
  adds nullable admission, settled-pass, and nonzero-removal evidence columns to the existing
  `raw_payload_files` cursor; old values remain unknown, and defer no longer erases settled or
  removal evidence. Raw writers hold the shared directory fence through owner-link persistence;
  release records a durable pending marker and timestamp, rechecks identity and size immediately
  before unlink, synchronizes the raw parent, and atomically marks removal evidence as persisted
  before ledger cleanup. Pending recovery has an independent bounded share and recovers older
  evidence before a replaced identity resets the ledger row. The System
  Status API, Web normalizer, Runtime Pressure UI, translations, and Storybook states expose the
  new diagnostics additively.

## References

- `./SPEC.md`
- `./IMPLEMENTATION.md`
- `../9aucy-db-retention-archive/SPEC.md`
