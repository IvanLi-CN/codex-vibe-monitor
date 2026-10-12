---
title: Dashboard browser performance baseline
module: web-dashboard
problem_type: performance
component: Dashboard realtime browser workload
tags:
  - dashboard
  - browser
  - performance
  - sse
  - baseline
status: active
related_specs:
  - docs/specs/performance-telemetry/SPEC.md
  - docs/specs/dashboard-hot-topic-projection/SPEC.md
---

# Dashboard browser performance baseline

## Measurement contract

The baseline uses `https://vibe-code.ivanli.cc/#/dashboard` in a fresh Ego Lite tab. Each run waits 10 seconds for warm-up, then samples for 60 seconds without user interaction. The same page is reloaded between the two runs. The probe records `PerformanceObserver` long tasks, 100 ms timer drift, `requestAnimationFrame` delivery, Chrome Performance metrics, heap usage, resource timings, and CDP EventSource messages.

SSE byte counts are UTF-8 bytes of decoded event payloads, before transport compression and TLS overhead. The probe itself adds a small, fixed observation cost, so future comparisons should use the same method. These numbers describe the observed production workload; they are not a claim about every device or network.

## Baseline results

| Metric                                          |      Run 1 |      Run 2 | Baseline interpretation                         |
| ----------------------------------------------- | ---------: | ---------: | ----------------------------------------------- |
| SSE messages / 60 s                             |        226 |        266 | 226–266 messages                                |
| SSE payload bytes / 60 s                        | 73,473,129 | 88,029,512 | 73–88 MB per minute                             |
| SSE payload rate                                |  1.22 MB/s |  1.47 MB/s | sustained high-rate stream                      |
| Long tasks                                      |         61 |         99 | repeated main-thread stalls                     |
| Long-task total                                 |     5.92 s |    11.86 s | 9.3–19.8% of the window                         |
| Long-task maximum                               |     176 ms |     314 ms | exceeds the 200 ms responsiveness guardrail     |
| Event-loop lag P95                              |    93.7 ms |   141.2 ms | above a 100 ms initial budget in run 2          |
| Event-loop lag maximum                          |   159.2 ms |   333.9 ms | visible input and update delay is possible      |
| Effective `requestAnimationFrame` rate          |   27.2 FPS |   24.0 FPS | below a 30 FPS dashboard target                 |
| Timeline resource calls, including cursor pages |          4 |          8 | repeated pagination during the window           |
| Slowest timeline request                        |     1.53 s |    20.64 s | backend latency can amplify UI refresh pressure |

Heap usage was 78.6 MB → 67.8 MB in run 1 and 65.7 MB → 81.6 MB in run 2. This variation is not sufficient to establish a leak; it should be followed by 10-minute heap snapshots with a forced-GC comparison when available.

## Payload attribution

A separate 10-second topic attribution sample on the same dashboard observed 38 SSE messages and 12,239,208 bytes. The largest repeated full snapshots were:

- `stats.timeseries.open-window`: 10 messages, 5.17 MB total, about 517 KB per message, 734 points per payload.
- `dashboard.working-conversations.current`: 7 messages, 5.32 MB total, about 760 KB per message, 14 conversations per payload.
- `dashboard.activity.current`: 21 messages, 1.75 MB total, about 84 KB per message.

The topic mix explains why the browser spends time parsing and committing data even when the visible dashboard changes only slightly.

## Accepted performance contract

The owner confirmed that the fix must preserve Dashboard data semantics, interaction, historical precision, and the existing full snapshot model. An additive delta envelope is allowed for live and replay delivery only when the client can validate and merge it, with a complete snapshot fallback.

The implementation boundary follows `docs/specs/dashboard-hot-topic-projection/SPEC.md`: complete the typed hot projections, compact/delta delivery and shared serialized frames; replace repeated full timeline refreshes with incremental/cursor reconciliation; add per-topic browser and backend attribution; and validate the deployed result with the same fixed browser experiment. The owner-facing Dashboard data model remains unchanged; the additive `delivery: "delta"` envelope is protocol opt-in and must fall back to a full frame when continuity or merge safety is uncertain.

The acceptance gate requires both independent 60-second production runs to satisfy every hard budget. A generic builder, live database read, or sustained cadence miss during either run is a failure; the median is retained for trend reporting only and cannot hide a failing run.

## Initial budgets for regression comparisons

Use these as working budgets until product-specific targets are agreed:

- SSE payload rate: below 100 KB/s while idle and below 250 KB/s during active work.
- Long-task P95: below 50 ms; no individual task above 200 ms.
- Event-loop lag P95: below 100 ms.
- Dashboard rendering: at least 30 FPS during normal live updates, with 60 FPS as the upper target on capable hardware.
- Timeline refresh: incremental or cursor-based updates; avoid repeatedly rebuilding the same 500-row snapshot.
- Memory: after a 10-minute stable run and GC, retained heap growth below 10%.

## Next measurements

The next A/B run should add per-topic frame count, payload bytes, message interval, JSON parse time, React commit time, and render duration. Backend telemetry already has `projection.snapshot_bytes` and `sse.frame_bytes`; those values should be grouped by topic and matched to the browser sample.

The first investigation target is why `working-conversations` and `timeseries.open-window` publish complete snapshots at the observed frequency. The second is the 15-second invocation timeline refresh and its 20-second request. Rendering optimizations should follow only after the data rate and refresh triggers are measured again.

## Implemented experiment

The branch now emits an additive `delivery: "delta"` envelope for `stats.timeseries.open-window` and `dashboard.working-conversations.current` when the server can prove a contiguous, safe merge. The server retains one canonical full snapshot and performs one bounded extra compact-delta serialization only when the delta is strictly smaller; this trades a small backend serialization cost for avoiding repeated large browser parses and state commits. Timeseries deltas update the live window metadata or one bucket; working-conversations deltas update one conversation or changed top-level fields. The client validates topic, schema, base cursor, and payload shape before merging. Any mismatch, gap, list-size change, or multi-item change requests a fresh complete snapshot.

This is an experiment until the same deployed two-run browser measurement is repeated. The expected gain is largest for timeseries metadata-only ticks and single-conversation updates, but the hard budgets remain unproven until production A/B data records lower bytes, JSON parse time, React commit time, long tasks, and event-loop lag.

## Reproduction

Repeat the contract above from a fresh tab, keeping the same viewport, browser profile, time zone, and network conditions. Report the two 60-second runs separately and retain median, P95, maximum, and raw byte counts; do not report a single average that hides bursty topic behavior.
