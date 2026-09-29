# Public Project Metrics API

> This file is the durable topic requirements contract. Current implementation facts belong in `IMPLEMENTATION.md`; lifecycle and change references belong in `HISTORY.md`.

## Context and Scope

- Context: Consumers need a public snapshot of aggregate Codex Vibe Monitor runtime activity. Read-only means there is no public mutation operation or persistent-state write.
- In scope: One project-owned endpoint, its aggregate response shape, freshness and failure behavior, CORS, and request limiting.
- Out of scope: Changes to existing Codex Vibe Monitor APIs, consumer application code, and any raw invocation or account data interface.

## Terms and Interfaces

- Interface: `GET /api/public/metrics/v1/codex-vibe-monitor`.
- Shared project-metrics route family: `/api/public/metrics/v1/{project}`. The terminal project slug identifies the metrics instance independently of its consumers. The three interfaces are `/api/public/metrics/v1/codex-vibe-monitor`, `/api/public/metrics/v1/tavily-hikari`, and `/api/public/metrics/v1/octo-rill`; OctoRill's existing metrics route is the shared-prefix baseline.
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
- Inputs: A same-hour Dashboard aggregate snapshot, existing hourly token rollups, and complete minute activity coverage for the recent parallel-work window.
- Outputs: `tokensPerMinute` and `parallelCalls` trends contain 12 chronologically ordered hourly points with `range="recent-hours"`; `todayTokens` contains 25 hourly boundary positions from local 00:00 through the next local 00:00 with future values set to `null` and `range="today"`. Its current-hour boundary MUST use the live `todayTokens.value`, while earlier boundaries use completed-hour rollups.
- Outputs: Each trend timestamp is an RFC 3339 timestamp at the `Asia/Shanghai` hourly boundary.
- Outputs: Dashboard capture and all aggregate source reads MUST remain within one Shanghai hour so live values are assigned to that hour's boundary. A refresh that crosses an hourly boundary MUST fail rather than publish a misbucketed snapshot.
- Outputs: Recent completed-hour parallel averages MUST be derived only from complete minute coverage. Missing or incomplete coverage MUST fail the refresh rather than fabricate zero activity.

### REQ-PBRA-003

- The system MUST return exactly 90 daily token activity points for the 90 completed Shanghai days ending yesterday, in ascending date order, inside a `tokenActivity90d` object with a `status` and `points`.
- Inputs: The project's long-term overall usage read model, which may be unready or cover only part of the requested range.
- Outputs: `status` is `available` when all points are numeric, `partial` when some points are numeric and some are missing, and `unavailable` when no points are numeric. Each point has only a `date` (`YYYY-MM-DD`) and a non-negative numeric `value` or `null`. Missing or source-uncovered dates MUST serialize as `null`, never as a fabricated zero. Unavailable long-term history MUST NOT make otherwise available current metrics fail.
- Consumer contract: Consumers MUST preserve `null` as unavailable historical data and MUST NOT coerce it to zero. Consumers can render current fields regardless of the long-term status.

### REQ-PBRA-004

- The system MUST assemble the response from project-owned aggregate runtime read models and MUST NOT expose request records, prompts, account or user identifiers, API keys, IP addresses, URLs, error details, or unrelated operational fields.
- Inputs: Dashboard live summary, hourly token rollups, minute activity coverage, the shared timeseries read model, and long-term daily usage rollups. The endpoint's `today`, `1h`, `Asia/Shanghai` timeseries request MUST use the existing hourly rollups and MUST NOT write persistent state.
- Outputs: Only the aggregate contract in `REQ-PBRA-001` is serialized.

### REQ-PBRA-005

- The system MUST serve a cached aggregate snapshot, coalesce concurrent refreshes, bound refresh work to three seconds, and return the last successful snapshot when a later refresh fails.
- Inputs: Requests arriving before or after the 30-second server snapshot freshness interval.
- Outputs: A failed initial refresh returns a sanitized service-unavailable response; it MUST NOT return partial aggregates or internal failure details. A failed refresh attempt MUST be shared with queued concurrent waiters and retries MUST be suppressed for a one-second cooldown.
- Outputs: All aggregate values in one snapshot MUST belong to the same Shanghai calendar day and hour. The system MUST verify the Shanghai day and hour immediately after dashboard capture and again after the final aggregate source read; a refresh that crosses either boundary MUST fail instead of publishing mixed-period values.

### REQ-PBRA-006

- The system MUST support conditional GET with an ETag and enforce a bounded server-side request rate.
- Inputs: `If-None-Match` and ordinary GET requests.
- Outputs: A matching validator returns `304 Not Modified`; excess requests return `429` with a retry hint.

### REQ-PBRA-007

- The system MUST apply an explicit, deployment-configurable CORS origin allowlist to this endpoint without enabling credentials.
- Inputs: The configured public metrics origins, defaulting to `https://ivanli.cc` and `http://127.0.0.1:12620`, through `PUBLIC_METRICS_CORS_ALLOWED_ORIGINS`.
- Outputs: Allowed origins may make GET requests and use `If-None-Match`; wildcard origins and credentialed requests are not allowed. The response MUST expose `ETag`, `Cache-Control`, and `Retry-After` to allowed browser clients.

## Verification

### VER-PBRA-001

- Method: Route and serialization tests.
- covers: `REQ-PBRA-001`, `REQ-PBRA-002`, `REQ-PBRA-004`
- Pass condition: The endpoint accepts GET only, has the exact approved response shape, uses finite non-negative values, and emits the required ordered trend lengths and Shanghai dates.

### VER-PBRA-002

- Method: Cache, validator, rate-limit, and refresh-failure tests.
- covers: `REQ-PBRA-005`, `REQ-PBRA-006`
- Pass condition: Fresh snapshots avoid a refresh, concurrent requests share one refresh (including a failed attempt), the one-second cooldown suppresses retries and later permits refresh, a refresh crossing a Shanghai hour or day boundary during dashboard/source reads is rejected, timeout/failure returns last-known-good data, ETags produce 304, and request limits produce 429.

### VER-PBRA-003

- Method: CORS middleware tests with allowed and denied origins, GET requests, and GET preflights.
- covers: `REQ-PBRA-007`
- Pass condition: Both configured defaults are allowed, unconfigured origins and non-GET preflights are denied, credentials are not enabled, and a cross-origin 429 response exposes its `Retry-After` header.

### VER-PBRA-004

- Method: Stateful SQLite aggregate fixture, response inspection, and fixed-query dispatch test.
- covers: `REQ-PBRA-003`, `REQ-PBRA-004`
- Pass condition: Exactly 90 ascending completed Shanghai dates are returned; unavailable and late-starting long-term coverage retain current metrics and serialize missing days as `null`; recent parallel-hour values require complete minute coverage and missing or incomplete coverage fails the refresh; the endpoint's fixed timeseries query selects hourly rollups before the minute-projection fallback; and no operational identifiers appear in the body.

## Related ADRs

None

## References

- `./IMPLEMENTATION.md`
- `./HISTORY.md`
- `../5k89c-long-term-usage-analytics/SPEC.md`
- `../z6ysw-dashboard-account-activity-tabs/SPEC.md`
