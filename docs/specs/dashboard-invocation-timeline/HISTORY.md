# Dashboard 对外调用时间线 主题历史

> 这里记录主题局部生命周期、替换、兼容性与必要背景；完整 ADR 取舍保留在 `docs/adr/`。单次任务流水账不放这里，规范正文仍以 `./SPEC.md` 为准。

## Lifecycle / Compatibility

- The timeline is the replacement read model for the screenshot's natural-day “今日/昨日 + 次数” activity chart. Existing aggregate metrics and PWA snapshot formats remain compatible outside this replacement surface.

## Replacements / Background

- Replaces the in-scope natural-day “次数” chart presentation with invocation bars. The old aggregate chart is not a fallback for this surface; it remains only for metrics and ranges outside this topic. The timeline reads existing Invocation records directly, so no conversion exception is part of this topic.

## Related Changes

- The live refresh path now retains a completed self-consistent older-window snapshot while coalescing one follow-up for the latest viewport; persisted materialization uses a timeline-specific source projection and batched account fallback without changing the public endpoint or snapshot contract. Runtime overlay records, the live revision, and source watermarks are captured once, while subsequent runtime revisions are coalesced into the next scheduled traversal instead of invalidating the current response. Source pages use short read transactions, and client-side incomplete traversals retain their `asOf` cursor until TTL rather than releasing it before recovery.
- The live timeline now remains mounted across a local-midnight window advance, while account and natural-day context changes retain their explicit loading reset. Runtime account hydration shares the bounded source transaction with persisted rows, account filtering selects the latest valid attempt with indexed per-key lookups, and materialization keeps the ADR 0044 page-level consistency boundary without a global live-change abort gate. Materialization summaries retain cancellation-safe phase timings for capture, source connection/transaction, lock, SQL, in-memory work, and publication diagnosis.
- Current-head performance evidence for candidate `942bc0dd1f8060733d8c4924cb57468d70ebda52` passed on an Agent VM: 20 global snapshots over 7,000 rows measured p95 9,441 ms, and 20 account snapshots over 3,500 rows measured p95 5,976 ms; EXPLAIN confirmed indexed bounded source, deduplication, and account-fallback paths.

## References

- `./SPEC.md`
- `./IMPLEMENTATION.md`
