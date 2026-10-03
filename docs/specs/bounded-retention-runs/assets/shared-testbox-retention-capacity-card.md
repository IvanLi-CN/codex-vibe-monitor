# Shared Testbox Retention Capacity Card

## Candidate

- Base: `origin/main@6228352d362e9a836fcdbb6271ff7bdb3bab72aa`
- Measured production source: `1b15332d` (`test(retention): honor recovery retry deadlines`); these three runs precede the final scheduler/observer review repair; their production-source identity is not relabeled as the final delivery head.
- Host: `codex-testbox` (`192.168.31.15`)
- Seed: fixed deterministic fixture, 1,300,000 expired invocation rows
- Skew: 500,000 rows share one Prompt key; 64 sparse orphan raw files; 64 invocation-linked request/response raw rows
- Cohort: `source_max_invocation_id=1,300,000`; batch cap 64 rows; raw compression disabled
- Workload: a writer attempts at most 1,024 expired rows outside the fixed cohort, one per 500 ms; a reader collects 512 indexed reads near run start. This is a synthetic fixture, not a calibrated replay of production traffic.

Three release-build candidate runs cleared the fixed cohort. `observed_archived` is the difference in source-row counts and the final SQL count checks zero remaining source rows; `summary_archived` is lower because prepared archive recovery can remove source rows without counting them as a new invocation batch in that run summary.

```text
candidate #1: runs=55 elapsed_ms=786679 summary_archived=375750 observed_archived=1300000 budget_exhausted_runs=4 lock_retries=50 read_p95_us=118 read_p99_us=631 raw_links_remaining=0 remaining=0
candidate #2: runs=53 elapsed_ms=766679 summary_archived=350022 observed_archived=1300000 budget_exhausted_runs=3 lock_retries=49 read_p95_us=67 read_p99_us=90 raw_links_remaining=0 remaining=0
candidate #3: runs=51 elapsed_ms=786364 summary_archived=344518 observed_archived=1300000 budget_exhausted_runs=4 lock_retries=46 read_p95_us=60 read_p99_us=90 raw_links_remaining=0 remaining=0
```

Candidate medians are `elapsed_ms=786364`, `runs=53`, `read_p95_us=67`, and `read_p99_us=90`. Every run ended with `remaining=0`, `raw_links_remaining=0`, and no recoverable or fatal failure. The productive runs stopped at the 60,000 ms work budget and resumed in later runs; pressure-only runs reported `sqlite_pressure`.

Logs:

- `/srv/codex/agents/01a0f586-886a-77a0-90a2-ff67ef15b774/candidate-final-run2.log`
- `/srv/codex/agents/01a0f586-886a-77a0-90a2-ff67ef15b774/candidate-final-run3.log`
- `/srv/codex/agents/01a0f586-886a-77a0-90a2-ff67ef15b774/candidate-final-run4.log`

## Development Baseline

The baseline was compiled independently from the development base and used the same seed, reader/writer fixture and fixed cohort. Each controlled run reported 192 archived rows across three budget passes and left 1,299,808 cohort rows. The optional `CVM_RETENTION_TEST_MAX_RUNS=3` mode bounds the baseline observation window. This short observation demonstrates slow measured progress; it does not prove what a full 24-hour baseline run would achieve.

```text
baseline #1: elapsed_ms=204462 summary_archived=192 observed_archived=192 budget_exhausted_runs=3 read_p95_us=58 read_p99_us=87 raw_links_remaining=0 remaining=1299808 complete=false
baseline #2: elapsed_ms=198535 summary_archived=192 observed_archived=192 budget_exhausted_runs=3 read_p95_us=52 read_p99_us=99 raw_links_remaining=0 remaining=1299808 complete=false
baseline #3: elapsed_ms=243959 summary_archived=192 observed_archived=192 budget_exhausted_runs=3 read_p95_us=53 read_p99_us=75 raw_links_remaining=0 remaining=1299808 complete=false
```

Baseline medians are `read_p95_us=53` and `read_p99_us=87`. Logs:

- `/srv/codex/agents/01a0f586-886a-77a0-90a2-ff67ef15b774/baseline-final-run1-partial.log`
- `/srv/codex/agents/01a0f586-886a-77a0-90a2-ff67ef15b774/baseline-final-run2-partial.log`
- `/srv/codex/agents/01a0f586-886a-77a0-90a2-ff67ef15b774/baseline-final-run3-partial.log`

## Staged Delivery Interpretation

The owner explicitly authorized shipping a demonstrated positive improvement in this PR, with further optimization in a separate PR. The staged delivery criterion is observed improvement in source-cohort drainage under the same synthetic fixture, plus ordinary correctness, migration, API/UI and CI checks. The long-term `VER-BRR-009` / original A7 target is retained and is not marked passed.

- Observed cohort drainage: all three candidate runs reached zero source rows in 766–787 seconds (median 786.364 seconds); the three bounded baseline windows each removed 192 rows.
- Execution boundaries: logs include productive runs stopping at the 60,000 ms work budget and pressure retries. These logs alone do not measure SQLite cancellation completion or prove absence of leaked locks.
- Read latency: candidate median p95/p99 is 67/90 us, baseline 53/87 us. This is a small indexed read probe near startup, not a full-run request distribution or a strict non-regression pass.
- Raw links: candidate runs ended at zero fixed-cohort invocation raw links. This is an ownership outcome check; it is not independent verification of every archive manifest, file checksum or crash boundary. The archive-file-io regression profile provides separate safety evidence.

## Limits and Follow-up

- Retention is invoked directly in the harness loop; managed catch-up scheduling is covered separately by scheduler regressions, not this capacity experiment.
- The 500,000-row hot key is present in invocation payloads without a materialized conversation identity; this experiment does not exercise the full hot-conversation refresh/orphan path.
- The writer is finite and the reader ends after 512 samples. Actual production request rates/read-write proportions, sustained peak competition, full-runtime p95/p99, verified per-file publication and independent lock-release measurement remain follow-up work.
- Source-row drainage cannot by itself prove the complete ordinary-load 24-hour capacity contract. No production capacity guarantee or production repair is claimed.

The final review repair additionally preserves catch-up when backlog measurement is unknown, restores the default inspection after clearing an override, and puts the independent observer behind background admission. Those changes do not modify archive publication/raw ownership or the benchmark fixture; delivery evidence includes a refreshed run instead of relabeling the earlier measurements.
