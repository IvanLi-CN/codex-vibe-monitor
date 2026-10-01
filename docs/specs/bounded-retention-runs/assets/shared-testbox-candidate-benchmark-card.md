# Shared Testbox Candidate Benchmark

## Run

- Date: 2026-10-01 (Asia/Shanghai)
- Branch: `th/retention-bounded-recovery-design`
- Test: `retention_prompt_cache_million_row_candidate_benchmark`
- Command: `cargo test --locked retention_prompt_cache_million_row_candidate_benchmark -- --ignored --nocapture --test-threads=1`
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
| Measured refresh elapsed | 347,620 ms |
| Online read samples      |        512 |
| Online read p95          | 225,033 us |
| Online read p99          | 250,408 us |
| Exact request count      |    500,000 |
| Remaining queue rows     |          0 |

The raw test output was:

```text
retention-million-row candidate total_rows=1300000 hot_rows=500000 pages=1954 elapsed_ms=347620 read_samples=512 read_p95_us=225033 read_p99_us=250408 exact_request_count=500000 queue_count=0
```

## Development baseline result

- Test: `retention_prompt_cache_million_row_baseline_benchmark` on the same fixture, fixed online load, and testbox at `a02b08f12c84d3c52ce4bae6f2c3e58a31243aab`
- The baseline refresh uses the pre-change single aggregate SQL path; it reports one logical refresh page.

| Measure                  |   Baseline |  Candidate | Comparison                              |
| ------------------------ | ---------: | ---------: | --------------------------------------- |
| Bounded/logical pages    |          1 |      1,954 | Candidate checkpoints each bounded page |
| Measured refresh elapsed | 103,187 ms | 347,620 ms | Candidate is 3.37x slower overall       |
| Online read samples      |        512 |        512 | Same fixed load                         |
| Online read p95          | 247,196 us | 225,033 us | Candidate 9% lower                      |
| Online read p99          | 278,407 us | 250,408 us | Candidate 10% lower                     |
| Exact request count      |    500,000 |    500,000 | Exact match                             |
| Remaining queue rows     |          0 |          0 | Converged                               |

Baseline raw output:

```text
retention-million-row baseline total_rows=1300000 hot_rows=500000 pages=1 elapsed_ms=103187 read_samples=512 read_p95_us=247196 read_p99_us=278407 exact_request_count=500000 queue_count=0
```

## Limits

This card proves candidate convergence, exact aggregation, queue drainage, busy-retry recovery, and concurrent read/write observation at the million-row scale. The candidate meets the acceptance comparison for online read p95/p99, while its total refresh elapsed is 3.37x higher because each page commits a durable cursor. The benchmark is an empirical candidate gate, not a production capacity guarantee; the final visual comparison remains a separate gate.
