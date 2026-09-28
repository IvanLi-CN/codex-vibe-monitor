# Public Blog Runtime API Implementation Status

> `./SPEC.md` is the normative contract. This file records implementation coverage and rollout facts.

## Current Status

- Implementation: Complete
- Lifecycle: active
- Catalog note: Public read-only aggregate endpoint for the IvanLi blog runtime panel.

## Implementation Coverage

- Requirement coverage: REQ-PBRA-001 through REQ-PBRA-007 are implemented. The endpoint serializes only approved aggregates; recent parallel-hour values require complete minute coverage, 90-day activity requires ready long-term coverage, and the Shanghai-day boundary is checked after all aggregate reads.
- Verification commands: `cargo test public_blog -- --nocapture` (17 passed); `cargo test spawn_http_server_leaves_health_unready_until_runtime_declares_readiness -- --nocapture` (1 passed); `cargo fmt --all -- --check`; `python3 .github/scripts/check_rust_source_quality.py --repo-root . --policy .github/rust-source-quality-policy.json`; `cargo check --locked --all-targets --all-features`; `cargo clippy --locked --all-targets --all-features -- -D warnings`.
- Rollout facts: The endpoint defaults its independent CORS allowlist to `https://ivanli.cc` and `http://127.0.0.1:12620`; `PUBLIC_BLOG_RUNTIME_CORS_ALLOWED_ORIGINS` replaces that list. No persistent schema migration is planned.

## Coverage / rollout summary

- The dedicated endpoint has a 30-second server snapshot cache, a three-second bounded single-flight refresh, one-second suppression after failure that permits later retry, stale last-good fallback, ETag, and a process-wide token bucket. The public route has a GET-only method guard and isolated CORS configuration, exposing `ETag`, `Cache-Control`, and `Retry-After` to allowed browser clients. Its response is assembled from existing aggregate read models.

## Remaining Gaps

- None.

## Related Changes

- Implementation and focused verification are tracked by the current PR; the PR reference is recorded in `HISTORY.md` after publication.

## References

- `./SPEC.md`
- `./HISTORY.md`
