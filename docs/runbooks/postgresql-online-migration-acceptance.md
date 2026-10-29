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

CI runs a non-pressure PostgreSQL functional and recovery subset through the evidence runner, and records a candidate-bound blocked/not-run report for unavailable acceptance evidence. It also runs 96 individually audited non-pressure backend tests across the lightweight, stateful SQLite and archive profiles. The full backend profiles and representative-scale job remain skipped; the selected tests do not replace their complete coverage. A skipped check is not acceptance evidence.

The functional suite can be run against an isolated PostgreSQL 18 database:

```bash
: "${BASE_SHA:?set BASE_SHA to the exact parent commit SHA before running acceptance}"
: "${CANDIDATE_SHA:?set CANDIDATE_SHA to the exact candidate commit SHA before running acceptance}"

python3 scripts/postgres-acceptance/run.py \
  --output "$EVIDENCE_DIR/report.json" \
  --evidence-dir "$EVIDENCE_DIR" \
  --base-sha "$BASE_SHA" \
  --candidate-sha "$CANDIDATE_SHA" \
  --fixture-version 4.13.11 \
  -- bash scripts/postgres-acceptance/run-functional-suite.sh
```

Exit `78` means `blocked/not-run`, including a missing URL or unavailable service. A non-zero child command is `failed`; exit `0` only reports that the child command completed, and does not establish target capacity, WAL backlog, backup/restore or natural-traffic evidence.

The functional suite writes `process-kill.json` for the process boundary after a PostgreSQL replay/checkpoint commit and before local acknowledgement. Its runner also preserves redacted stdout/stderr and the exact base/candidate SHAs. Pressure and performance evidence is not run; backup/restore and natural-traffic evidence remain deployment-preparation gates.

Performance workflows must complete in less than three minutes overall. Pressure tests remain prohibited until separately authorized.

## Observations and deployment requirements

Pressure and performance measurements are not current acceptance gates. Any future performance workflow must finish in strictly less than three minutes overall and must not run pressure tests while they are prohibited. The pre-commit switch attempt uses a configurable budget; if it expires before authority publication, reopen SQLite and do not publish PostgreSQL authority.

Before production cutover, independently verify PostgreSQL target capacity, actual WAL backlog and data-volume headroom for the planned run. A post-cutover PostgreSQL outage must keep the module in explicit recovery/error handling; it must not silently fail back to stale SQLite authority. Missing safety evidence blocks cutover.

Every measurement must include workload, duration, candidate SHA, source/target identities, sample count, raw result path and whether maintenance was held. Record migration hold and task-restoration periods separately. PostgreSQL target capacity, actual WAL backlog and the target data volume remain unverified by this fixture; verify those, backup and restore, and a natural-traffic baseline before production cutover. None establishes that production cutover has already occurred.

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

The process test covers the target replay/checkpoint commit → local SQLite acknowledgement boundary using a real PostgreSQL 18 target and application-process termination. The deterministic fault tests exercise source acknowledgement after a committed release, source fencing, authority publication, router generation publication, journal acknowledgement, normal timeout/retry and recovery. These tests do not claim coverage for filesystem/WAL crash durability beyond the committed PostgreSQL transaction, backup/restore or the production natural-traffic baseline; those remain deployment-preparation gates.

## Deployment sequence

1. Validate the exact source fixture, inventory, capacity, backups and restore drill in an isolated environment.
2. Deploy the migration-capable candidate once. Do not require an offline copy or a second preliminary application deployment.
3. Keep SQLite authority while schema preparation, bounded copy, catch-up and verification run in the background.
4. Hold only competing heavy work for the active module; keep proxy forwarding, required authentication, routing controls, journals and recovery available.
5. Cut over only after all evidence is passed, convergence is stable, the final delta fits the switch budget and the authority decision can be durably resolved.
6. Retain the source, checkpoints, authority evidence and archive/Raw files through post-cutover verification and an independent retention decision.
7. If the candidate is faulty, repair forward on the confirmed authority. Do not roll back to a binary that can write stale SQLite tables.
