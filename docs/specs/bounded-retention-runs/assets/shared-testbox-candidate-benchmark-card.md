# Shared Testbox Candidate Benchmark

## Run

- Date: 2026-10-01 (Asia/Shanghai)
- Branch: `th/retention-bounded-recovery-design`
- Candidate source commit: `7ee46e116df24df89b8af8b1da32be24a5a5d861`
- Tests: `retention_prompt_cache_million_row_candidate_benchmark` and `retention_prompt_cache_million_row_baseline_benchmark`
- Commands: `cargo test --locked retention_prompt_cache_million_row_candidate_benchmark -- --ignored --nocapture --test-threads=1`; `cargo test --locked retention_prompt_cache_million_row_baseline_benchmark -- --ignored --nocapture --test-threads=1`
- Database: temporary SQLite file with WAL mode and an 8-connection writer pool plus a 2-connection online-read pool
- Online load: both candidates run the same fixed 512 read samples and 512 write operations; write load is independent of the number of refresh pages.

## Fixture and method

- 1,300,000 invocation rows with one prompt-cache key covering 500,000 rows.
- The remaining 800,000 rows have no prompt-cache key and exercise the surrounding invocation indexes.
- The prompt-cache refresh uses the production bounded paging entry point and persists its cursor between calls.
- A concurrent reader repeatedly executes `SELECT COUNT(*) FROM codex_invocations WHERE id > 0`; a separate writer performs the same fixed 512-row write load for both candidates. Lock conflicts on the candidate page are retried with a bounded backoff.
- The test asserts exact `request_count = 500000` and a drained refresh queue before it reports success.

## Candidate result

| Measure                  |     Result |
| ------------------------ | ---------: |
| Bounded pages            |      1,954 |
| Measured refresh elapsed | 370,793 ms |
| Online read samples      |        512 |
| Online read p95          | 238,421 us |
| Online read p99          | 290,324 us |
| Exact request count      |    500,000 |
| Remaining queue rows     |          0 |

The raw test output was:

```text
retention-million-row candidate total_rows=1300000 hot_rows=500000 pages=1954 elapsed_ms=370793 read_samples=512 read_p95_us=238421 read_p99_us=290324 exact_request_count=500000 queue_count=0
```

## Development baseline result

- Test: `retention_prompt_cache_million_row_baseline_benchmark` in the current candidate tree, using the same fixture and fixed online load. The test contains the pre-change single aggregate SQL path, so the comparison is reproducible without modifying the base checkout.
- The baseline refresh reports one logical refresh page.

| Measure                  |   Baseline |  Candidate | Comparison                              |
| ------------------------ | ---------: | ---------: | --------------------------------------- |
| Bounded/logical pages    |          1 |      1,954 | Candidate checkpoints each bounded page |
| Measured refresh elapsed | 101,673 ms | 370,793 ms | Candidate is 3.65x slower overall       |
| Online read samples      |        512 |        512 | Same fixed load                         |
| Online read p95          | 230,190 us | 238,421 us | Candidate 4% higher                     |
| Online read p99          | 253,214 us | 290,324 us | Candidate 15% higher                    |
| Exact request count      |    500,000 |    500,000 | Exact match                             |
| Remaining queue rows     |          0 |          0 | Converged                               |

Baseline raw output:

```text
retention-million-row baseline total_rows=1300000 hot_rows=500000 pages=1 elapsed_ms=101673 read_samples=512 read_p95_us=230190 read_p99_us=253214 exact_request_count=500000 queue_count=0
```

## Limits

This card proves candidate convergence, exact aggregation, queue drainage, busy-retry recovery, and concurrent read/write observation at the million-row scale. The current run shows the durable-page candidate taking 3.65x longer overall and 4%/15% higher online read p95/p99 than the baseline under this run's fixed load; it is retained as an empirical cost and recovery bound rather than a throughput claim. The benchmark is not a production capacity guarantee.
