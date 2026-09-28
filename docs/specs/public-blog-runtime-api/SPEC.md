# Public Blog Runtime API

> This file is the durable topic requirements contract. Current implementation facts belong in `IMPLEMENTATION.md`; lifecycle and change references belong in `HISTORY.md`.

## Context and Scope

- Context: The IvanLi blog needs a public snapshot of aggregate Codex Vibe Monitor runtime activity. Read-only means there is no public mutation operation or source invocation-data write; an eligible exact fallback may asynchronously warm the existing derived timeseries cache.
- In scope: One project-owned endpoint, its aggregate response shape, freshness and failure behavior, CORS, and request limiting.
- Out of scope: Changes to existing Codex Vibe Monitor APIs, blog application code, and any raw invocation or account data interface.

## Terms and Interfaces

- Interface: `GET /api/public/blog-runtime/v1/codex-vibe-monitor`.
- Stat: An object with a current numeric `value` and a `trend` containing a range and ordered timestamped points.
- Shanghai day: A calendar day bounded in `Asia/Shanghai`.
- Interface discriminator: `kind` is the literal `codex-vibe-monitor`; the only metric fields are `tokensPerMinute`, `parallelCalls`, `todayTokens`, and `tokenActivity90d`.

## Requirements

### REQ-PBRA-001

- The system MUST serve the dedicated endpoint as GET-only and MUST NOT require browser credentials.
- Inputs: A request to the exact endpoint path.
- Outputs: A JSON object containing the `kind` discriminator and only the four approved aggregate metric fields.

### REQ-PBRA-002

- The system MUST serialize every Stat with a finite, non-negative numeric `value` and an ordered trend.
- Inputs: A same-day Dashboard aggregate snapshot, existing hourly token rollups, and complete minute activity coverage for the recent parallel-work window.
- Outputs: `tokensPerMinute` and `parallelCalls` trends contain 12 chronologically ordered hourly points with `range="recent-hours"`; `todayTokens` contains 25 hourly boundary positions from local 00:00 through the next local 00:00 with future values set to `null` and `range="today"`.
- Outputs: Each trend timestamp is an RFC 3339 timestamp at the `Asia/Shanghai` hourly boundary.
- Outputs: Recent completed-hour parallel averages MUST be derived only from complete minute coverage. Missing or incomplete coverage MUST fail the refresh rather than fabricate zero activity.

### REQ-PBRA-003

- The system MUST return exactly 90 daily token activity points for the 90 completed Shanghai days ending yesterday, in ascending date order.
- Inputs: The project's ready long-term overall usage read model.
- Outputs: Each point has only a `date` (`YYYY-MM-DD`) and non-negative numeric `value`; a source-uncovered range MUST NOT be presented as fabricated activity.

### REQ-PBRA-004

- The system MUST assemble the response from project-owned aggregate runtime read models and MUST NOT expose request records, prompts, account or user identifiers, API keys, IP addresses, URLs, error details, or unrelated operational fields.
- Inputs: Dashboard live summary, hourly token rollups, minute activity coverage, the shared timeseries read model, and long-term daily usage rollups. On an eligible exact fallback, the shared timeseries reader MAY asynchronously warm its existing `timeseries_minute_projection_v2` derived cache; response generation MUST NOT wait for that cache write.
- Outputs: Only the aggregate contract in `REQ-PBRA-001` is serialized.

### REQ-PBRA-005

- The system MUST serve a cached aggregate snapshot, coalesce concurrent refreshes, bound refresh work to three seconds, and return the last successful snapshot when a later refresh fails.
- Inputs: Requests arriving before or after the 30-second server snapshot freshness interval.
- Outputs: A failed initial refresh returns a sanitized service-unavailable response; it MUST NOT return partial aggregates or internal failure details. A failed refresh attempt MUST be shared with queued concurrent waiters and retries MUST be suppressed for a one-second cooldown.
- Outputs: All aggregate values in one snapshot MUST belong to the same Shanghai calendar day. The system MUST compare the Shanghai day at refresh start with the day after all aggregate source reads complete; a refresh that crosses the boundary MUST fail instead of publishing mixed-day values.

### REQ-PBRA-006

- The system MUST support conditional GET with an ETag and enforce a bounded server-side request rate.
- Inputs: `If-None-Match` and ordinary GET requests.
- Outputs: A matching validator returns `304 Not Modified`; excess requests return `429` with a retry hint.

### REQ-PBRA-007

- The system MUST apply an explicit, deployment-configurable CORS origin allowlist to this endpoint without enabling credentials.
- Inputs: The configured public blog runtime origins, defaulting to `https://ivanli.cc` and `http://127.0.0.1:12620`.
- Outputs: Allowed origins may make GET requests and use `If-None-Match`; wildcard origins and credentialed requests are not allowed. The response MUST expose `ETag`, `Cache-Control`, and `Retry-After` to allowed browser clients.

## Verification

### VER-PBRA-001

- Method: Route and serialization tests.
- covers: `REQ-PBRA-001`, `REQ-PBRA-002`, `REQ-PBRA-004`
- Pass condition: The endpoint accepts GET only, has the exact approved response shape, uses finite non-negative values, and emits the required ordered trend lengths and Shanghai dates.

### VER-PBRA-002

- Method: Cache, validator, rate-limit, and refresh-failure tests.
- covers: `REQ-PBRA-005`, `REQ-PBRA-006`
- Pass condition: Fresh snapshots avoid a refresh, concurrent requests share one refresh (including a failed attempt), the one-second cooldown suppresses retries and later permits refresh, a refresh crossing Shanghai midnight during aggregate reads is rejected, timeout/failure returns last-known-good data, ETags produce 304, and request limits produce 429.

### VER-PBRA-003

- Method: CORS middleware tests with allowed and denied origins and methods.
- covers: `REQ-PBRA-007`
- Pass condition: Both configured defaults are allowed, unconfigured origins and non-GET preflights are denied, credentials are not enabled, and a cross-origin 429 response exposes its `Retry-After` header.

### VER-PBRA-004

- Method: Stateful SQLite aggregate fixture and response inspection.
- covers: `REQ-PBRA-003`, `REQ-PBRA-004`
- Pass condition: Exactly 90 ascending completed Shanghai dates are returned from the long-term overall rollup, recent parallel-hour values require complete minute coverage and missing or incomplete coverage fails the refresh, and no operational identifiers appear in the body.

## Related ADRs

None

## References

- `./IMPLEMENTATION.md`
- `./HISTORY.md`
- `../5k89c-long-term-usage-analytics/SPEC.md`
- `../z6ysw-dashboard-account-activity-tabs/SPEC.md`
