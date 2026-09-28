# Public Blog Runtime API Topic History

> This file records lifecycle, compatibility, and related changes. The normative contract remains in `./SPEC.md`.

## Lifecycle / Compatibility

- The topic introduces one additive public read-only API and leaves existing API contracts unchanged.

## Replacements / Background

- The blog runtime panel consumes aggregate values and does not use invocation-level details.

## Related Changes

- The implementation and its cross-origin `Retry-After` follow-up are delivered by [PR #1049](https://github.com/IvanLi-CN/codex-vibe-monitor/pull/1049).
- The shared timeseries reader may asynchronously warm its existing derived minute projection on eligible exact fallback; this PR adds no projection schema or migration.

## References

- `./SPEC.md`
- `./IMPLEMENTATION.md`
