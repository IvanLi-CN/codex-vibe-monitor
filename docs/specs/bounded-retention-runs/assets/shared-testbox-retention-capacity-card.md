# Shared Testbox Retention Capacity Card

## Candidate

- Base: `origin/main@9b7967f26fbeb51bcdb586e73b0db7013a308292`
- Measured source: `9f6ab720` (`fix(retention): schedule catch-up from managed runs`); the later dispatcher-gate and catch-up-clear fixes are not re-benchmarked here.
- Host: `codex-testbox` (`192.168.31.15`)
- Seed: fixed deterministic fixture, 1,300,000 expired invocation rows
- Skew: 500,000 rows share one Prompt key; 64 sparse orphan raw files
- Cohort: `source_max_invocation_id=1,300,000`; batch cap 64 rows; raw compression disabled
- Workload: concurrent online reader and writer, 512 read samples

The current-head harness cleared the fixed cohort in 38 bounded runs. The first pressure-only runs recorded `sqlite_pressure`; the productive runs stopped at the 60,000 ms work budget until the final partial run. The final run settled in 39 ms and observed zero remaining rows. This is a single candidate harness observation, not the three-run A7 capacity gate.

```text
retention-capacity-candidate total_rows=1300000 hot_rows=500000 orphan_files=64 cohort_source_max=1300000 runs=38 adaptive=true elapsed_ms=988199 summary_archived=1208070 observed_archived=1300000 budget_exhausted_runs=15 lock_retries=22 recoverable_retries=0 no_progress_retries=0 online_samples=512 read_p95_us=63 read_p99_us=97 remaining=0
```

`observed_archived` is the authoritative fixed-cohort measurement. `summary_archived` is the run-summary counter and is lower because recovery and already-prepared archive work can remove source rows without being counted as a new invocation batch in that summary. The final SQL count and bounded source upper bound reached the cohort terminal state. The fixture did not attach raw files to invocation rows, so raw publication/ownership is outside this card.

## Development Baseline

The baseline used the same fixture, seed, online reader/writer, and fixed cohort. An initial unpatched run failed at the old archive write path with SQLite extended error 517 (`database is locked`) and then made no progress. A bounded-retry rerun made only 64 rows per run: run 1 took 107,144 ms and run 2 took 75,192 ms, both stopped at `retention_work_budget`; it was stopped after those two identical low-throughput observations because completing the cohort at that rate would exceed the 24-hour acceptance window. The raw logs are retained at:

`/srv/codex/agents/01a0f586-886a-77a0-90a2-ff67ef15b774/baseline-capacity-1.log`

`/srv/codex/agents/01a0f586-886a-77a0-90a2-ff67ef15b774/baseline-capacity-final.log`

This is a useful harness regression result: the old implementation could not submit the cohort within the declared budget under the online lock load, so it cannot produce a complete baseline p95/p99 distribution before the capacity window expires. The candidate's p95/p99 must therefore be read as an absolute online-probe observation, not as a claim of a numeric baseline comparison. The harness also uses the current head with adaptive mode toggled rather than compiling the merge-base implementation.

## Acceptance Interpretation

- Fixed cohort completion: observed for the candidate harness (`remaining=0`); not a full A7 pass.
- Bounded execution: pass; productive runs stop at approximately 60 seconds and report `retention_work_budget`.
- Lock/pressure behavior: observed for the candidate harness; 22 lock-pressure retries were recoverable and no run leaked a lock.
- Online latency: candidate measured 63 microseconds p95 and 97 microseconds p99 over 512 reads; numeric baseline comparison remains unavailable while the old path fails before producing a comparable sample set.
- Three-run statistical median: not claimed. This card records one candidate run and two controlled baseline rounds showing the same bounded-throughput failure; it does not turn that limitation into a pass.

The baseline failure and harness scope mean the full A7 “candidate versus three-run baseline p95/p99 median” comparison is not satisfied by this card. The candidate shows bounded database-row progress under the same online probe, ending with a zero fixed cohort; this card does not establish 24-hour production capacity, continuous new-expiry handling, or complete raw-file Verified Archive behavior.

The benchmark source now includes a follow-up fixture with invocation-linked request/response raw files and a writer that inserts new expired invocations outside the fixed source cohort. That follow-up harness passed a small 2,000-row smoke run locally, but has not yet produced a shared-testbox release-build result and is not included in the capacity numbers above.
