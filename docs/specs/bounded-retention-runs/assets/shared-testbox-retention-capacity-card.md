# Shared Testbox Retention Capacity Card

## Candidate

- Base: `origin/main@6228352d362e9a836fcdbb6271ff7bdb3bab72aa`
- Measured production source: `1b15332d` (`test(retention): honor recovery retry deadlines`); the only later source change is the benchmark-only partial-run output described below.
- Host: `codex-testbox` (`192.168.31.15`)
- Seed: fixed deterministic fixture, 1,300,000 expired invocation rows
- Skew: 500,000 rows share one Prompt key; 64 sparse orphan raw files; 64 invocation-linked request/response raw rows
- Cohort: `source_max_invocation_id=1,300,000`; batch cap 64 rows; raw compression disabled
- Workload: concurrent online reader and writer inserting 1,024 expired rows outside the fixed cohort, 512 read samples

Three release-build candidate runs cleared the fixed cohort. `observed_archived` and the final SQL count are the authoritative cohort measurements; `summary_archived` is lower because prepared archive recovery can remove source rows without counting them as a new invocation batch in that run summary.

```text
candidate #1: runs=55 elapsed_ms=786679 summary_archived=375750 observed_archived=1300000 budget_exhausted_runs=4 lock_retries=50 read_p95_us=118 read_p99_us=631 raw_links_remaining=0 remaining=0
candidate #2: runs=53 elapsed_ms=766679 summary_archived=350022 observed_archived=1300000 budget_exhausted_runs=3 lock_retries=49 read_p95_us=67 read_p99_us=90 raw_links_remaining=0 remaining=0
candidate #3: runs=51 elapsed_ms=786364 summary_archived=344518 observed_archived=1300000 budget_exhausted_runs=4 lock_retries=46 read_p95_us=60 read_p99_us=90 raw_links_remaining=0 remaining=0
```

Candidate medians are `elapsed_ms=786679`, `runs=53`, `read_p95_us=67`, and `read_p99_us=90`. Every run ended with `remaining=0`, `raw_links_remaining=0`, and no recoverable or fatal failure. The productive runs stopped at the 60,000 ms work budget and resumed in later runs; pressure-only runs reported `sqlite_pressure`.

Logs:

- `/srv/codex/agents/01a0f586-886a-77a0-90a2-ff67ef15b774/candidate-final-run2.log`
- `/srv/codex/agents/01a0f586-886a-77a0-90a2-ff67ef15b774/candidate-final-run3.log`
- `/srv/codex/agents/01a0f586-886a-77a0-90a2-ff67ef15b774/candidate-final-run4.log`

## Development Baseline

The baseline used the same fixture, seed, online reader/writer, and fixed cohort from `origin/main`. The old path was unable to keep up with the online expired-row writer: each controlled run submitted only 192 rows across three 60-second budget passes and left 1,299,808 cohort rows. The benchmark's optional `CVM_RETENTION_TEST_MAX_RUNS=3` mode stops after those three passes and still emits the 512 online read samples, so the baseline result is measurable without waiting for a cohort that cannot clear in the 24-hour window.

```text
baseline #1: elapsed_ms=204462 summary_archived=192 observed_archived=192 budget_exhausted_runs=3 read_p95_us=58 read_p99_us=87 raw_links_remaining=0 remaining=1299808 complete=false
baseline #2: elapsed_ms=198535 summary_archived=192 observed_archived=192 budget_exhausted_runs=3 read_p95_us=52 read_p99_us=99 raw_links_remaining=0 remaining=1299808 complete=false
baseline #3: elapsed_ms=243959 summary_archived=192 observed_archived=192 budget_exhausted_runs=3 read_p95_us=53 read_p99_us=75 raw_links_remaining=0 remaining=1299808 complete=false
```

Baseline medians are `read_p95_us=53` and `read_p99_us=87`. Logs:

- `/srv/codex/agents/01a0f586-886a-77a0-90a2-ff67ef15b774/baseline-final-run1-partial.log`
- `/srv/codex/agents/01a0f586-886a-77a0-90a2-ff67ef15b774/baseline-final-run2-partial.log`
- `/srv/codex/agents/01a0f586-886a-77a0-90a2-ff67ef15b774/baseline-final-run3-partial.log`

## Acceptance Interpretation

- Fixed cohort completion: candidate passed in all three runs, within minutes rather than the 24-hour budget; baseline did not clear and was still at 1,299,808 rows after the controlled window.
- Bounded execution: passed; candidate productive runs stop at approximately 60 seconds and report `retention_work_budget` before the next run.
- Lock/pressure behavior: passed for the candidate fixture; pressure retries were recoverable, raw links reached zero, and no run reported a fatal or recoverable failure.
- Online latency: measured, but not a clean non-regression claim. Candidate medians (`67/90us`) are slightly above the baseline medians (`53/87us`) on this shared host while doing substantially more archive work. The absolute tails remain sub-millisecond, but A7's strict "not worse" latency clause is not marked verified from these samples.
- Three-run capacity evidence: throughput and completion are demonstrated; the latency comparison remains the remaining empirical qualification for a full A7 sign-off.

The benchmark source also retains the invocation-linked raw-file and cohort-external writer fixture. Those linked rows are included in the three candidate runs above, and `raw_links_remaining=0` confirms the verified archive ownership boundary for the fixed cohort. This card does not claim production capacity beyond the tested seed, online mix, or shared-testbox limits.
