# PostgreSQL online migration acceptance and operations

This runbook covers acceptance and deployment preparation for the module-owned PostgreSQL migration. It does not perform a production migration and does not authorize a release.

## Preflight

Confirm all of the following before a live run:

- The source compatibility fixture is exactly `4.13.11` at `85b46550da5fb04b8a4c39d310a9b04c80824047`.
- The candidate SHA and the parent base SHA are recorded in the evidence report.
- `POSTGRES_TEST_DATABASE_URL` points to an isolated PostgreSQL database. Credentials stay in the environment and never enter reports.
- The source SQLite database, PostgreSQL target, authority manifest, archive files, Raw files and WAL/archive retention have independent backup and restore owners.
- The service account can create the migration schema, constraints, indexes and bounded capture state, but the acceptance database contains no production data.

## Acceptance command

The CI workflow starts PostgreSQL 18.6 with a pinned image digest and runs both the lightweight PostgreSQL migration tests and the archive/file-I/O PostgreSQL tests:

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

Exit `78` means `blocked/not-run`, including a missing URL or unavailable service. It is an acceptance failure, not a skip. A non-zero child command is `failed`; only exit `0` is `passed`.

The live suite requires PostgreSQL 18.x and writes three candidate-bound evidence files into `EVIDENCE_DIR`: `process-kill.json` proves the real-process boundary after target replay/checkpoint commit and before local acknowledgement; `startup-performance.json` compares migration-disabled and PostgreSQL-configured startup initialization over small and large SQLite histories; and `switch-performance.json` measures the real PostgreSQL authority switch for small and large copied tables. The runner also preserves redacted stdout/stderr and the exact base/candidate SHAs. These files do not close the other fault boundaries unless their own evidence is present.

## Initial operating thresholds

These are acceptance gates, not promises that a production system has already met them:

- Startup migration initialization adds no historical table scan. Compare startup with small and large historical datasets and against the same candidate with migration work disabled. Target at most 500 ms of added p95 startup work; repeat and investigate a noisy or materially slower measurement using the same workload and environment.
- The pre-commit switch attempt stays within the configured three-second budget. If the budget expires before authority publication, reopen SQLite and do not publish PostgreSQL authority.
- Catch-up remains convergent for three consecutive one-minute observation windows: applied change rate exceeds incoming change rate and the oldest pending change age decreases.
- Capture/WAL backlog stays below 50% of its configured capacity, and free disk headroom covers at least twice the measured peak target-plus-capture growth for the planned run.
- The same representative traffic run shows no more than a 10% p95 regression for unchanged proxy/admin endpoints. Time saved by pausing optional maintenance is reported separately.
- A post-cutover PostgreSQL outage keeps the module in explicit recovery/error handling. It must not silently fail back to stale SQLite authority.

Every measurement must include workload, duration, candidate SHA, source/target identities, sample count, raw result path and whether maintenance was held. Record migration hold and task-restoration periods separately. Validate backup and restore and collect a natural-traffic baseline before production cutover; neither is evidence that production cutover has already occurred.

The startup measurement times `StorageRuntimeRouter::initialize` after fixture schema/history preparation and before worker start. The PostgreSQL-configured case uses the lazy real target pool and a separately probed PostgreSQL 18.x service; it must not scan historical archive rows. The switch measurement prepares a real PostgreSQL target and times the complete pre-fence verification, bounded tail catch-up, fence and authority publication path against the three-second budget.

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

The current live process test covers only the target replay/checkpoint commit → local SQLite acknowledgement boundary. It does not claim coverage for source acknowledgement, source fencing, authority manifest publication, router generation publication, journal acknowledgement, filesystem/WAL durability or backup/restore. Those remain explicit acceptance gaps until dedicated evidence is attached.

## Deployment sequence

1. Validate the exact source fixture, inventory, capacity, backups and restore drill in an isolated environment.
2. Deploy the migration-capable candidate once. Do not require an offline copy or a second preliminary application deployment.
3. Keep SQLite authority while schema preparation, bounded copy, catch-up and verification run in the background.
4. Hold only competing heavy work for the active module; keep proxy forwarding, required authentication, routing controls, journals and recovery available.
5. Cut over only after all evidence is passed, convergence is stable, the final delta fits the switch budget and the authority decision can be durably resolved.
6. Retain the source, checkpoints, authority evidence and archive/Raw files through post-cutover verification and an independent retention decision.
7. If the candidate is faulty, repair forward on the confirmed authority. Do not roll back to a binary that can write stale SQLite tables.
