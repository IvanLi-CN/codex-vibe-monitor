---
title: Coherent observability mock data
module: frontend-delivery
problem_type: realistic-deterministic-observability-demo
component: React/Vite routing observability demo
tags:
  - frontend
  - mock-data
  - msw
  - storybook
  - observability
status: active
related_specs:
  - docs/specs/zr9jd-api-key-model-routing-health/SPEC.md
---

# Coherent observability mock data

## Research question

How should the Web Demo represent request attempts, retries, routing decisions,
state transitions and time-series data without producing obviously fabricated or
internally contradictory values?

## Findings from primary sources

- MSW's official guidance recommends declaring named runtime scenarios as handler
  overrides and selecting them from a runtime criterion such as a query parameter.
  Handler order is significant because the left-most override wins. This keeps a
  demo scenario explicit and switchable without changing the application request
  path. See [Dynamic mock scenarios](https://mswjs.io/docs/best-practices/dynamic-mock-scenarios).
- MSW's project documentation describes network-level interception that preserves
  the production request path and allows the same handlers to be reused in the
  browser and Node. This is preferable to replacing the application's fetch layer
  with a second, demo-only API shape. See [MSW project README](https://github.com/mswjs/msw).
- Faker's official guide says seeded output is reproducible, but relative-date
  helpers also need a fixed reference date because they otherwise depend on the
  current day. It also warns that values can change when the Faker version changes,
  so the version and seed are part of the evidence contract. See [Faker reproducible results](https://fakerjs.dev/guide/usage#reproducible-results).
- Faker's complex-object guidance explicitly recommends a typed factory and
  generating dependent fields in order. Its example derives the first name from a
  previously selected sex and derives the email from the generated names, because
  independently random fields can produce undesirable combinations. It also
  recommends overwrite/options parameters when a scenario needs precise control.
  See [Faker complex objects](https://fakerjs.dev/guide/usage#create-complex-objects).
- Storybook documents that stories should render isolated component examples with
  data defined as story args or alongside the story; loaders are an advanced escape
  hatch for data that genuinely must be loaded externally. See [Storybook loaders](https://storybook.js.org/docs/writing-stories/loaders)
  and [Building pages with Storybook](https://storybook.js.org/docs/writing-stories/build-pages-with-storybook).

## Recommendations for this demo

1. Use a small, named fixture factory for the domain entities: account, model,
   invocation, attempt, decision, and event. Keep the API types as the source of
   truth. Do not generate display-only strings such as `示例 API Key（冷却候选）`
   or synthetic statuses that have no corresponding persisted event.
2. Generate one immutable request timeline first, then derive every projection from
   it. A retry must share the invocation correlation but have a new attempt id and
   later timestamp. A `cooling` or `degraded` row must be backed by the failure
   response that caused that transition; a recovery row must have a preceding real
   failure and a subsequent successful attempt.
3. Use a fixed scenario seed and fixed reference time. Pin the generator version if
   a generator is introduced. Avoid current-time helpers in fixtures; otherwise the
   same screenshot and filtering assertions will drift between runs.
4. Model distributions and constraints explicitly instead of using uniform random
   values. For example, make successful responses the majority, make retries less
   frequent than first attempts, keep latency ranges plausible for the same upstream
   class, and ensure status transitions obey the routing state machine. Randomness
   may fill identifiers and minor variation only after these constraints are applied.
5. Make the dataset large enough for the supported windows and filters, but preserve
   a readable, real-looking history. Every visible record must map to a real fixture
   invocation and exact `(upstream_account_id, model)` pair. Do not inflate counts by
   duplicating one row with different labels or by inventing accounts/models merely
   to fill the screen.
6. Keep scenario selection explicit in the URL or story args. A default operational
   scenario should be coherent and boring; exceptional states such as retry recovery,
   cooldown and empty/error should be separate named scenarios that can be tested and
   captured independently.

## Acceptance checks

- Each route decision attempt resolves to exactly one fixture invocation, account and
  model, and each retry has a unique attempt id.
- Every state transition has a causally ordered source event and a valid preceding
  state; no row claims recovery without a successful terminal attempt.
- 15m/1h/6h/24h filters are projections of the same timestamped dataset rather than
  separate hand-written arrays. Counts decrease monotonically as the window narrows.
- Re-running the demo with the same scenario and seed produces the same records,
  timestamps, statuses and screenshot; the seed and fixture version are documented.
- The demo contains no raw request/response payloads, credentials, or unredacted
  upstream errors, and it cannot fall back to a real backend.

## References

- `docs/solutions/workflow/mock-only-web-demo-runtime.md`
- `docs/specs/zr9jd-api-key-model-routing-health/SPEC.md`
