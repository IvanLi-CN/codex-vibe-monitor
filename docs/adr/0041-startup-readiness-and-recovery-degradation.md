---
status: accepted
---

# ADR 0041: Startup readiness and bounded recovery

## Context

The service previously treated terminal journal replay and proxy orphan recovery as a startup barrier. A large in-flight set or a locked SQLite writer could therefore delay the listener for minutes or make the process exit before `/health` became ready. The barrier also combined unrelated replay and business recovery work into transactions that were too large to attribute or retry safely.

## Decision

Basic HTTP readiness is published after the database connection, schema, required structure state and listener are ready. Terminal journal replay, proxy invocation recovery and pool attempt recovery run behind a single post-readiness recovery coordinator. Live requests retain their existing fail-closed terminal journal confirmation contract; readiness does not claim that historical recovery is complete.

The coordinator captures a startup fence using the maximum invocation and pool-attempt IDs observed before the listener is published. It processes each fenced table in ascending ID order, with at most 64 IDs per transaction. A committed batch advances its cursor and then refreshes runtime projections and broadcasts. Rows created after the fence are left to normal lifecycle or a later recovery pass.

Recovery uses a dedicated SQLite connection with a 200ms busy timeout and a 250ms write-admission budget. Busy, locked, pool-pressure and I/O failures roll back the current batch, retain the durable obligation, emit a fixed-category warning and retry with 250ms, 500ms, 1s, 2s and 5s capped delays. The startup coordinator and periodic pool orphan sweep share an in-process owner so they cannot run overlapping recovery transactions.

Terminal journal frames and database schemas are unchanged. Journal acknowledgement remains after SQLite commit. Incomplete tails and isolated historical frames continue to follow ADR 0039; an unavailable journal or an unconfirmed live append still rejects new terminal acceptance.

## Consequences

The listener and `/health` no longer wait for historical recovery, while recovery backlog and oldest age remain observable through `cvm_startup_recovery_*` gauges and batch/retry histograms. A healthy process can therefore report HTTP readiness while recovery is still pending; operators must use the recovery and SQLite contention panels to distinguish those states.

The bounded coordinator may take multiple short transactions to finish a large backlog. Shutdown cancels the coordinator after native SQLite work returns, leaving uncommitted obligations in the existing journal or durable source rows for the next start. No schema migration, journal format change, connection-pool expansion or protocol change is required.
