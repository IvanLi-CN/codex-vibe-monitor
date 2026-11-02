# ADR 0051: OAuth Luna Reserve fallback routing

## Status

Accepted

## Context

OAuth upstream usage currently persists and routes only the ordinary primary and secondary usage windows. The upstream usage response may also expose an additional `gpt-reserve` limit for subscription accounts whose ordinary allowance is unavailable. That limit is a separate upstream quota capability commonly presented as Luna Reserve.

The existing model-mapping path is not sufficient for this behavior. A model mapping changes the upstream model string, while Reserve selection must also select a different quota bucket, use different availability state, and obey an operator-controlled fallback policy. Treating a normal Luna model name as a Reserve alias could silently consume Reserve quota or route requests for accounts that do not have it.

The project also uses “Reserve” for local invocation and capacity reservations. The upstream quota capability therefore needs distinct domain terminology and state semantics.

## Decision

- Call the upstream capability **Luna Reserve** and call its usage allowance **Reserve Quota**.
- Treat `gpt-reserve` as the Reserve wire model, not as a normal public model alias. A normal model whose name contains `luna` remains on Normal Quota unless fallback is explicitly selected by the routing policy.
- Parse the upstream usage response's additional limits and retain the Reserve-specific allowance signal, usage windows, normal model slug, and snapshot freshness. A Reserve upsell indicator is supporting evidence only; it cannot establish routability by itself.
- Represent Normal Quota and Luna Reserve as separate quota routes. Persist additional limits as typed records related to a usage snapshot rather than deriving availability from prices, token usage, or an unstructured model alias.
- Add a global operator switch for Automatic Reserve Fallback. It is enabled by default for OAuth upstream accounts and controls only automatic fallback. An explicitly requested `gpt-reserve` request is not disabled by this switch.
- Keep Reserve usage synchronization and account-pool visibility independent from the Automatic Reserve Fallback switch. Disabling fallback stops automatic Reserve use but does not hide or stop reading Reserve state.
- Route normal requests to Normal Quota first. If the request is classified as ordinary quota exhaustion, refresh usage at most once when necessary, and try Luna Reserve once when the account is explicitly Reserve-available. Subsequent failures may select another Reserve-available account, subject to the existing failover budget.
- Do not trigger Automatic Reserve Fallback for generic 429 responses, upstream 5xx responses, authentication failures, unsupported models, or invalid requests.
- Treat missing, stale, or failed Reserve observations as `unknown`, not as `unavailable`. Unknown Reserve state cannot authorize automatic fallback; Normal Quota routing remains unaffected.
- Accept an explicit `gpt-reserve` request as Reserve-only routing. It must not silently downgrade to Normal Quota. The default model catalog does not advertise `gpt-reserve` as an ordinary user-facing model.
- Preserve the client's requested model identity when a Reserve fallback succeeds, while recording the actual upstream wire model and selected quota route for diagnostics and usage accounting.
- Treat an individual Reserve failure as a routing failure, not proof that Reserve is exhausted. Persist Reserve exhaustion or unavailability only after an explicit upstream signal or a confirming usage snapshot; transient transport, server, and request errors must not poison the quota state.
- Expose the global fallback switch through the existing pool-routing settings API and UI. Expose Reserve availability and the latest usage percentage and reset time in account summaries and account details. Do not label a percentage as an absolute remaining request or token balance.

## Consequences

- Existing normal model routing remains unchanged until ordinary quota exhaustion; when the Automatic Reserve Fallback switch is disabled, this behavior remains unchanged without affecting explicit Reserve requests.
- Reserve quota can be consumed by normal requests only after a classified ordinary-quota failure, making the policy observable and operator-controllable.
- The schema and usage API gain an additive representation for additional limits and Reserve state. Existing accounts without Reserve data remain compatible and report `unknown` rather than false availability.
- Routing, request rewriting, usage persistence, API responses, and the account-pool UI must carry both the client-requested model and the selected quota route. The upstream wire model may be `gpt-reserve` while the client-facing model identity remains unchanged.
- Operators can disable automatic Reserve consumption without losing visibility into which OAuth accounts expose Reserve and how much of its reported window is used.
- The upstream usage endpoint exposes percentages and reset windows rather than a guaranteed absolute balance. The UI and routing logic must not infer a token balance from pricing or local usage estimates.
- Historical Reserve snapshots can support later trend views, but the first UI surface is the current list/detail status rather than a new historical chart.

## Rejected alternatives

- Mapping every `gpt-*-luna` request to `gpt-reserve`: this conflates a normal model identity with a quota route and can consume Reserve unexpectedly.
- Using `rate_limit_upsell` alone to decide availability: an upsell is not a routability guarantee.
- Storing only the raw upstream JSON: this would make candidate selection and stale-state handling dependent on response-shape parsing in multiple layers.
- Inferring Reserve quota from price or token usage: local pricing is not upstream billing or allowance truth.

## References

- [OpenAI Codex rate-limit client](https://raw.githubusercontent.com/openai/codex/main/codex-rs/backend-client/src/client/rate_limit_resets.rs)
- [CLIProxyAPI Codex quota parser](https://raw.githubusercontent.com/router-for-me/CLIProxyAPI/main/internal/runtime/executor/helps/codex_quota.go)
- [CLIProxyAPI Reserve behavior report](https://github.com/router-for-me/CLIProxyAPI/issues/5568)
- [sub2api OpenAI quota service](https://raw.githubusercontent.com/Wei-Shaw/sub2api/main/backend/internal/service/openai_quota_service.go)
- `src/upstream_accounts/sync_oauth_crypto_utils.rs`
- `src/upstream_accounts/routing/selection.rs`
- `src/proxy/failover/retry_orchestration.rs`
- `web/src/lib/api/core-upstream.ts`
