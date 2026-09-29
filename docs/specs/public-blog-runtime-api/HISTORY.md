# Public Project Metrics API Topic History

> This file records lifecycle, compatibility, and related changes. The normative contract remains in `./SPEC.md`.

## Lifecycle / Compatibility

- The topic introduces one additive public read-only API and leaves existing API contracts unchanged.

## Replacements / Background

- Consumers use aggregate values and do not receive invocation-level details.

## Related Changes

- The implementation and its cross-origin `Retry-After` follow-up are delivered by [PR #1049](https://github.com/IvanLi-CN/codex-vibe-monitor/pull/1049).
- The endpoint's fixed `today`/`1h`/`Asia/Shanghai` timeseries query uses existing hourly rollups and does not enter the shared reader's sub-hour minute-projection warm-up path; this change adds no persistent-state writes or schema changes.
- The refresh uses one Shanghai-hour observation for live trend boundaries and rejects reads that cross that hour.

## References

- `./SPEC.md`
- `./IMPLEMENTATION.md`
