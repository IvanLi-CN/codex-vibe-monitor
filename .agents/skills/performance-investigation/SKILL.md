---
name: performance-investigation
description: Diagnose codex-vibe-monitor database contention, high CPU, or slow responses through Grafana and fixed hotpath reports.
---

# Performance investigation

Use the repository's `scripts/cvm-observe` with private token files. The owner or
operator supplies `GRAFANA_URL`, `GRAFANA_TOKEN_FILE`, optional `GRAFANA_ORG_ID`,
and for code reports `CVM_URL` / `CVM_READ_TOKEN_FILE`. Grafana uses a dedicated
Viewer service account over public HTTPS. Hotpath has a separate read token.

1. Fix the UTC incident window (default: last 30 minutes), service, environment
   and instance. Run `scripts/cvm-observe health`, then `freshness`. Treat absent
   data, unsupported platforms and stale samples as unknown.
2. Run `latency`, `samples`, `proxy`, `cpu`, `sqlite`, `pool`, `queue`, `errors`
   and `tasks`. Compare the incident window with an equivalent healthy window
   in Grafana. Record rates, sample counts and the actual query window.
3. For database contention, separate coordinator wait, pool acquisition, queue
   wait, SQL execution, batch ACK and enqueue-to-commit. Run `sql` and `server`
   to attribute expensive normalized SQL to functions and route templates.
4. For slow requests, separate response-head and full-body time, upstream
   attempts/retries, TTFB, TTFT and full stream lifetime. One terminal invocation
   is distinct from its upstream attempts. Cancellation is a lifecycle outcome.
5. For high CPU, run `functions`. Timing includes waits and is not CPU cost.
   If CPU attribution is needed, use the operator-installed restricted SSH
   command `cvm-hotpath-cpu capture` (30 seconds, 100 Hz; maximum 60 seconds).
   Retrieve only the returned project's artifacts and inspect with
   `samply load <profile> --symbol-dir <matching-symbol-directory>`.
6. Return evidence, likely cause and the smallest proposed fix. Report tool or
   permission failures as unavailable; stop at a diagnosis until edits are
   authorized. Verify a fix with the same load and window before claiming an
   improvement.

Read [metric semantics](../../../docs/design/performance-observability-metrics.md)
when interpreting a signal; read [operations](../../../ops/observability/README.md)
for missing access, CPU sampling or deployment questions.

App histograms are classic (`_bucket` / `_count`); hotpath uses native histograms
without those suffixes. Quantiles require enough measured samples: disclose low
counts and never convert missing quantiles to zero. Function timing samples 10%
of calls; `sampled_calls` is the duration denominator. CPU percentage is relative
to one core and can exceed 100%. Counters reset on process restart. Resource
attribution and frame sizes are estimates where the metric contract says so.

The official Grafana `gcx` can query the same fixed datasource and dashboards;
use its installed `--help` for the pinned CLI's syntax. The compatible HTTP path
is `GET /api/datasources/proxy/uid/cvm-prometheus/api/v1/query_range`, also usable
with `curl` and `jq`. Keep secrets out of prompts, command-line arguments and
artifacts. Grafana queries use HTTPS; SSH is only for extra hotpath/CPU diagnosis.
