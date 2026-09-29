# Public Project Metrics API Implementation Status

> `./SPEC.md` is the normative contract. This file records implementation coverage and rollout facts.

## Current Status

- Implementation: Complete
- Lifecycle: active
- Catalog note: Public read-only aggregate endpoint for Codex Vibe Monitor metrics consumers.

## Implementation Coverage

- Requirement coverage: REQ-PBRA-001 through REQ-PBRA-007 are implemented. The endpoint serializes only approved aggregates; recent parallel-hour values require complete minute coverage; 90-day activity reports available, partial, or unavailable coverage with missing values as null; the current-hour daily trend point uses the live daily total; and the Shanghai hour is checked after dashboard capture and all aggregate reads.
- Verification commands: `cargo test public_blog -- --nocapture` (22 passed); `cargo fmt --all -- --check`; `python3 .github/scripts/check_rust_source_quality.py --repo-root . --policy .github/rust-source-quality-policy.json`; `cargo check --locked --all-targets --all-features`; `cargo clippy --locked --all-targets --all-features -- -D warnings`. Spec contract and drift checks, version-impact JSON parsing, and `git diff --check` also passed.
- Rollout facts: The endpoint defaults its independent CORS allowlist to `https://ivanli.cc` and `http://127.0.0.1:12620`; `PUBLIC_METRICS_CORS_ALLOWED_ORIGINS` replaces that list. It reads existing hourly and daily rollups without writing persistent state; no schema migration is planned.

## Coverage / rollout summary

- The dedicated endpoint uses `/api/public/metrics/v1/codex-vibe-monitor`, a 30-second server snapshot cache, a three-second bounded single-flight refresh, one-second suppression after failure that permits later retry, stale last-good fallback, ETag, and a process-wide token bucket. The public route has a GET-only method guard and isolated `PUBLIC_METRICS_CORS_ALLOWED_ORIGINS` configuration, exposing `ETag`, `Cache-Control`, and `Retry-After` to allowed browser clients. Its response is assembled from existing aggregate read models.
- The endpoint requests the shared timeseries reader with a whole-hour-aligned `today`/`1h`/`Asia/Shanghai` query, which uses existing hourly rollups and does not enter its sub-hour minute-projection warm-up path. Its snapshot cache and rate limiter remain process memory.

## Remaining Gaps

- None.

## Related Changes

- Implementation and focused verification are tracked by the current PR; the PR reference is recorded in `HISTORY.md` after publication.

## References

- `./SPEC.md`
- `./HISTORY.md`
