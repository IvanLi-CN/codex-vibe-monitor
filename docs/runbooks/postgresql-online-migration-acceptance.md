# PostgreSQL online migration acceptance and operations

This runbook covers acceptance and deployment preparation for the module-owned PostgreSQL migration. It does not perform a production migration and does not authorize a release.

## Preflight

Confirm all of the following before a live run:

- The source compatibility fixture is exactly `4.13.11` at `85b46550da5fb04b8a4c39d310a9b04c80824047`.
- The candidate SHA and the parent base SHA are recorded in the evidence report.
- `POSTGRES_TEST_DATABASE_URL` points to an isolated PostgreSQL database. Credentials stay in the environment and never enter reports.
- The source SQLite database, PostgreSQL target, authority manifest, archive files, Raw files and WAL/archive retention have independent backup and restore owners.
- The service account can create the migration schema, constraints, indexes and bounded capture state, but the acceptance database contains no production data.

## Acceptance status

The live acceptance harness is disabled in CI: it includes pressure tests and its complete runtime has not been bounded below three minutes. CI runs a separate non-pressure PostgreSQL functional and recovery subset and records a candidate-bound blocked/not-run report for the unavailable acceptance evidence. The backend PR profiles and representative-scale job are also not run until their contents are established to exclude pressure tests. A skipped check is not acceptance evidence. The command below describes the retained harness; it must not be run while pressure tests are prohibited.

The retained harness command is:

```bash
: "${BASE_SHA:?set BASE_SHA to the exact parent commit SHA before running acceptance}"
: "${CANDIDATE_SHA:?set CANDIDATE_SHA to the exact candidate commit SHA before running acceptance}"

python3 scripts/postgres-acceptance/run.py \
  --output "$EVIDENCE_DIR/report.json" \
  --evidence-dir "$EVIDENCE_DIR" \
  --base-sha "$BASE_SHA" \
  --candidate-sha "$CANDIDATE_SHA" \
  --fixture-version 4.13.11 \
  -- bash scripts/postgres-acceptance/run-live-suite.sh
```

Exit `78` means `blocked/not-run`, including a missing URL or unavailable service. A non-zero child command is `failed`; exit `0` only reports that the child command completed, and does not establish target capacity, WAL backlog, backup/restore or natural-traffic evidence.

The live suite requires PostgreSQL 18.x and writes candidate-bound evidence files into `EVIDENCE_DIR`: `process-kill.json` proves the real-process boundary after target replay/checkpoint commit and before local acknowledgement; `startup-performance.json` compares migration-disabled and PostgreSQL-configured startup initialization over small and large SQLite histories; `switch-performance.json` measures the real PostgreSQL authority switch for small and large copied tables; `continuous-load.json` runs three 20-second real-capture windows with a non-empty tail and checks a 150-second fixture elapsed-time budget; `runtime-pressure.json` and `runtime-maintenance-pressure.json` exercise the 32-request bounded runtime load with and without a held maintenance task; `fault-boundaries.json` records the deterministic target/source/fence/authority/router/journal boundary tests that passed; and `system-headroom.json` records CI disk, CPU and load observations. The runner also preserves redacted stdout/stderr and the exact base/candidate SHAs. Backup/restore and natural-traffic evidence remain deployment-preparation gates.

The fixture elapsed-time check does not bound setup, cleanup, compilation, or the complete CI workflow. Performance workflows must complete in less than three minutes overall; the existing live suite has not established that limit. Pressure tests remain prohibited until separately authorized, so their evidence is not run under that restriction.

## Observations and deployment requirements

The following performance and pressure measurements are not current acceptance gates while their workflows are disabled. They describe previously proposed observations and do not establish production behavior. Any future performance workflow must finish in strictly less than three minutes overall and must not run pressure tests while they are prohibited.

- Startup migration initialization adds no historical table scan. Compare startup with small and large historical datasets and against the same candidate with migration work disabled. Target at most 500 ms of added p95 startup work; repeat and investigate a noisy or materially slower measurement using the same workload and environment.
- The pre-commit switch attempt stays within the configured three-second budget. If the budget expires before authority publication, reopen SQLite and do not publish PostgreSQL authority.
- Catch-up remains convergent for three consecutive 20-second observation windows within the fixture's 150-second elapsed-time check: applied change rate exceeds incoming change rate and the oldest pending change age decreases. This bounded gate does not claim one-minute production behavior.
- The representative 32-request pressure run keeps request p95 strictly below two seconds, reaches exactly 32 durable terminal facts within five seconds of request start, records zero `InteractiveProxy` admission deltas and zero `AttemptProgress` SQL writes, and drains the terminal materialization backlog within five seconds after the lock is released.
- The same representative traffic run shows no more than a 10% p95 regression for unchanged proxy/admin endpoints. Time saved by pausing optional maintenance is reported separately.

Before production cutover, independently verify PostgreSQL target capacity, actual WAL backlog and data-volume headroom for the planned run. A post-cutover PostgreSQL outage must keep the module in explicit recovery/error handling; it must not silently fail back to stale SQLite authority. Missing safety evidence blocks cutover.

Every measurement must include workload, duration, candidate SHA, source/target identities, sample count, raw result path and whether maintenance was held. Record migration hold and task-restoration periods separately. PostgreSQL target capacity, actual WAL backlog and the target data volume remain unverified by this fixture; verify those, backup and restore, and a natural-traffic baseline before production cutover. None establishes that production cutover has already occurred.

The startup measurement times `StorageRuntimeRouter::initialize` after fixture schema/history preparation and before worker start. Each sample resets the SQLite statement cache and installs a SQLite authorizer that records `SQLITE_READ` actions for `archive_batches` only during that initialization call; the evidence records the per-sample counts and requires every count to be zero. The PostgreSQL-configured case uses the lazy real target pool and a separately probed PostgreSQL 18.x service; it must not scan historical archive rows. The switch measurement prepares a real PostgreSQL target and times the complete pre-fence verification, bounded tail catch-up, fence and authority publication path against the three-second budget.

## Backup and restore drill

Before switching a module, capture and retain:

1. A consistent SQLite source backup and its schema/inventory digest.
2. A PostgreSQL base backup or equivalent target snapshot plus the required WAL archive range.
3. The durable authority manifest, migration checkpoints, source fence marker and journal state.
4. Archive manifests, proof rows, Raw-link metadata and the referenced archive/Raw file trees.

Restore each owner into an isolated environment. Verify schema/inventory, stable IDs, generation, watermarks, archive proofs, Raw links and normalized row/chunk digests. Resolve the authority manifest before opening writes. Restore is disaster recovery; it is not an automatic post-cutover failback mechanism.

## Fault and crash matrix

| Failure point                           | Required recovery result                                                                                              |
| --------------------------------------- | --------------------------------------------------------------------------------------------------------------------- |
| Target transaction commit uncertain     | Re-read the durable target checkpoint and retry idempotently; do not acknowledge the source from an uncertain result. |
| Source acknowledgement uncertain        | Keep the source obligation pending until the target checkpoint proves the applied prefix.                             |
| Source fence uncertain                  | Keep admission closed and resolve the durable fence before reopening either backend.                                  |
| Authority publication uncertain         | Read the manifest and choose the durable authority; never choose based on reachability.                               |
| Router generation publication uncertain | Reconcile the router with the durable authority generation before admitting new work.                                 |
| Journal acknowledgement uncertain       | Replay the stable journal identity and retain it until destination commit is acknowledged.                            |
| Filesystem or WAL durability uncertain  | Mark the migration blocked/recovering, preserve source authority, and require an explicit repair or new generation.   |

The live process test covers the target replay/checkpoint commit → local SQLite acknowledgement boundary and real PostgreSQL 18 process termination, while the deterministic fault suite exercises source acknowledgement after a committed release, source fencing, authority publication, router generation publication, journal acknowledgement, normal timeout/retry and recovery. The live continuous-load test exercises a non-empty source tail at the switch boundary. It does not claim coverage for filesystem/WAL crash durability beyond the committed PostgreSQL transaction, backup/restore or the production natural-traffic baseline; those remain deployment-preparation gates.

## Deployment sequence

1. Validate the exact source fixture, inventory, capacity, backups and restore drill in an isolated environment.
2. Deploy the migration-capable candidate once. Do not require an offline copy or a second preliminary application deployment.
3. Keep SQLite authority while schema preparation, bounded copy, catch-up and verification run in the background.
4. Hold only competing heavy work for the active module; keep proxy forwarding, required authentication, routing controls, journals and recovery available.
5. Cut over only after all evidence is passed, convergence is stable, the final delta fits the switch budget and the authority decision can be durably resolved.
6. Retain the source, checkpoints, authority evidence and archive/Raw files through post-cutover verification and an independent retention decision.
7. If the candidate is faulty, repair forward on the confirmed authority. Do not roll back to a binary that can write stale SQLite tables.
