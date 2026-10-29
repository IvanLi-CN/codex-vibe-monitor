# Module-owned PostgreSQL migration implementation

## Current Status

- Implementation: migration routing, capture, replay, authority and acceptance fixtures are under development; this record does not establish production readiness.
- Lifecycle: active

## Implementation Coverage

- `src/storage/` contains module routing, migration state, SQLite capture and PostgreSQL target paths for the authority and replay requirements.
- `scripts/postgres-acceptance/` and the PostgreSQL CI workflow separate a non-pressure functional/recovery subset from unavailable pressure and performance evidence.
- The acceptance runner validates the base and candidate commit relationship for CI and manual runs, and requires an explicit report scope. CI records the functional subset's scoped report and redacted output. A separate explicit list selects audited non-pressure backend tests; it does not replace the full backend profiles.
- The source compatibility fixture is `4.13.11` at `85b46550da5fb04b8a4c39d310a9b04c80824047`; other source versions require their own evidence.
- The non-pressure functional subset selects real PostgreSQL source/target operations, archive/Raw ownership and transaction checks, source acknowledgement, deterministic fence and publication faults, journal recovery, and a process SIGKILL recovery boundary. Selection alone does not establish candidate-bound results for every failure boundary.

## Remaining Gaps

- Pressure tests are not run under the owner restriction; no load, convergence or capacity result is claimed.
- Actual PostgreSQL target capacity, WAL backlog, target data-volume headroom, backup/restore and a natural-traffic baseline remain deployment-preparation checks, not established results.
- Archive/Raw atomicity and fence, authority, router, journal and post-cutover outage boundaries require candidate-bound evidence before their coverage can be claimed.
- Repository and live branch-protection required checks must agree before CI gate compliance can be established.

## References

- [Requirements](./SPEC.md)
- [Acceptance contract](./ACCEPTANCE.md)
- [Topic history](./HISTORY.md)
