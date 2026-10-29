# PostgreSQL migration acceptance contract

This contract defines the evidence needed before an online module migration can be treated as accepted. It does not authorize a production cutover.

## Fixture

The initial compatibility fixture is the deployed `4.13.11` tag at commit `85b46550da5fb04b8a4c39d310a9b04c80824047`. The supported source range is exact-tag-only until another source version has its own fixture and report. An older binary must fail closed against a migrated authority.

The canonical fixture record is [acceptance-fixtures.json](../../adr/assets/module-owned-postgresql-migration/acceptance-fixtures.json). It records the source commit, representative archive rows, PostgreSQL service contract, runner exit semantics and candidate-bound evidence requirements without credentials or business payloads. The live archive mapping test loads and consumes those representative `4.13.11` rows.

## Runner contract

Use `scripts/postgres-acceptance/run.py` around the acceptance command with explicit `--base-sha` and `--candidate-sha` values. The runner requires `POSTGRES_TEST_DATABASE_URL`, sets `POSTGRES_ACCEPTANCE_REQUIRED=1`, `BASE_SHA`, `CANDIDATE_SHA`, `POSTGRES_ACCEPTANCE_FIXTURE_VERSION` and `POSTGRES_ACCEPTANCE_EVIDENCE_DIR` for the child process, and never infers either SHA from the checked-out `HEAD`.

| Runner result                   | Meaning                                                                       | Gate result               |
| ------------------------------- | ----------------------------------------------------------------------------- | ------------------------- |
| `passed`, exit `0`              | PostgreSQL was reachable and the child command completed successfully.        | Evidence may be reviewed. |
| `blocked`, `not-run`, exit `78` | The database URL or service was unavailable; no child acceptance command ran. | Not accepted.             |
| `failed`, non-zero exit         | The child command ran and produced a failure.                                 | Not accepted.             |

Existing live PostgreSQL tests retain optional behavior for ordinary local suites, but panic with `status=blocked result=not-run` when the required mode is set. This prevents CI from silently converting missing service setup into a green test.

## Required evidence

The report must bind every result to the candidate SHA and contain raw stdout/stderr paths.

1. Run the `4.13.11` source fixture through real PostgreSQL copy, catch-up, coverage proof and transaction routing tests.
2. Exercise concurrent insert, update, delete, key change and replacement with retry and restart; prove no stale overwrite, resurrection, duplicate business effect or missing fact.
3. Exercise archive ownership and Raw-link atomicity, including release/prune recovery.
4. Resolve crash and fault outcomes around target commit, source acknowledgement, source fence, authority publish, router publication and journal acknowledgement. The live suite must exercise a real process `SIGKILL` after the PostgreSQL replay/checkpoint commit and before local acknowledgement, then restart and reconcile from the target checkpoint. Cover each boundary with deterministic fault injection and document the coverage and any missing evidence.
5. Measure startup, convergence, switch duration, ongoing traffic, source capture/WAL growth and disk headroom against the thresholds in the operations runbook. The live suite must emit `startup-performance.json` and `switch-performance.json`; the SIGKILL test must emit `process-kill.json`.
6. Before production cutover, complete the backup/restore drill for source SQLite, PostgreSQL, authority state and archive/Raw files, and collect a representative natural-traffic baseline.

SQLite-only tests are contract coverage, not a substitute for the live PostgreSQL result. A missing or unavailable PostgreSQL service remains `blocked/not-run`.
