# Forward Proxy Backtest

This project ships a local-only backtest tool for forward proxy weight algorithms.

## Purpose

- Compare baseline `v1` and candidate `v2` on the same SQLite snapshot.
- Use backtest output as the acceptance gate for algorithm changes.
- Keep snapshot data local and avoid accidental disclosure.

## Script

- Path: `scripts/forward_proxy_backtest.py`
- Input: a local SQLite snapshot containing `forward_proxy_attempts` and `forward_proxy_runtime`.
- Output: one JSON report and one Markdown report.

## Security Defaults

The script enforces the following checks:

- Opens SQLite with read-only URI mode.
- Refuses database paths located inside the repository tree.
- Writes reports to `/tmp` by default.
- Redacts proxy identifiers by default.

If any required security check fails, acceptance will fail.

## Usage

Run with the same acceptance command used in planning:

```bash
python3 scripts/forward_proxy_backtest.py \
  --db /tmp/cvm-prod-db-Jd0UiY/codex_vibe_monitor.db \
  --algos v1,v2 \
  --seeds 7,11,19,23,29 \
  --requests 50000
```

Expected output lines:

- `json_report=/tmp/forward-proxy-backtest-<timestamp>.json`
- `markdown_report=/tmp/forward-proxy-backtest-<timestamp>.md`
- `acceptance_pass=<true|false>`

## Acceptance Rules

`v2` must satisfy all checks below against `v1` on the same snapshot:

1. `trace_replay.positive_nodes_p50 >= 2`
2. `trace_replay.single_node_collapse_ratio <= 0.35`
3. `simulation.top1_share_mean <= 0.55`
4. `simulation.success_rate_mean >= baseline_success_rate_mean - 0.2%`
5. `simulation.p95_latency_mean <= baseline_p95_latency_mean * 1.10`
6. Required security checks all pass

Script exit code behavior:

- `0`: acceptance pass
- `1`: acceptance fail
