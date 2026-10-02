# Model Management and Manual Price Synchronization

## Status

Accepted

## Context

Model pricing and the proxy's `/v1/models` preset list are managed separately. The pricing catalog is keyed by model ID, and runtime cost estimation does not use provider identity. A provider-specific quote therefore cannot become an independently effective price when multiple providers expose the same model ID.

The existing catalog is a bounded local estimate rather than an upstream billing record. Manual edits must remain under user control, while maintaining model prices and preset membership should take less effort. ADR 0018 declined online synchronization; this decision adds a user-triggered, reviewed synchronization path and supersedes that restriction only.

## Decision

Add a Models page under System Settings. Its main list is the union of local pricing entries and proxy preset candidates, merged by model ID. Show all currently supported price fields, display `—` when a row has no price, and provide a Switch for preset membership on every row, including dynamically added model IDs outside the built-in preset set. Preserve existing preset values. A newly synchronized model does not become a preset automatically; its Switch starts off. Preset membership controls the proxy-generated `GET /v1/models` response and does not assert that an upstream provider can serve the model.

The page supports deleting a model. Deletion removes its local price and preset membership without changing historical invocation costs. If models.dev still provides the model, a later synchronization may offer it again as a new candidate.

The “Sync All” action opens a review dialog and retrieves candidate data from models.dev. It is the only third-party price directory used; LiteLLM is not queried for comparison. The dialog shows retrieval status, new models, local-versus-candidate price differences, and an official pricing-page link when available. Provider groups limit the current preview and synchronization scope. Search matches provider and model names.

Keep one effective local price per model ID. When selected provider groups contain different quotes for the same model ID, show the provider candidates and require the user to choose one before synchronization. Provider identity identifies the candidate source but is not added to runtime price matching.

Import only token price dimensions supported by the existing estimator: input, output, cache read, cache write, and reasoning. Mark other dimensions, such as modality or processing-tier prices, as unsupported in the preview and do not import them. Models without a compatible price are not selectable for price import.

Within the selected provider groups, leave any candidate without a remembered selection unchecked, including new models and price changes. Restore remembered explicit selections and deselections without bypassing provider conflict resolution or import eligibility. The service owns the durable memory of provider scope, explicit quote-provider choices, model/provider checkbox choices, and model discovery; see ADR 0026. Selecting a locally hand-maintained price explicitly authorizes replacing it with the chosen candidate. No price is written until the user applies “Sync Selected”. Synchronization is user-triggered; there is no scheduled or background catalog import.

`source=custom` is the durable marker for a hand-maintained price. Manual edits from the Models page set this source to `custom`; applying a directory quote sets it to `models.dev`. A source label alone never selects a candidate for synchronization.

The official provider pricing page remains the reference for verifying a quote. The application does not automatically compare that page with models.dev or treat a third-party value as billing truth; the user decides which reviewed candidate to store as the local estimate.

## Consequences

- Users can manage local estimates and proxy model presets from one page while keeping price and preset state independent.
- Selecting provider groups bounds each review to the suppliers the user chose instead of importing the entire models.dev catalog at once.
- A chosen quote for a shared model ID applies to all invocations resolved under that ID, even if those invocations route to providers with different rates.
- Prices for unsupported dimensions are visible during review but are not represented in estimated costs.
- Deleting a model removes local state, but does not prevent a later source sync from rediscovering it.
- Historical non-null invocation costs remain unchanged by price edits, synchronization, or model deletion.

## References

- [ADR 0018: OpenAI GPT-6 pricing and usage semantics](./0018-openai-gpt-6-pricing-and-usage-semantics.md)
- [ADR 0026: Service-owned price sync memory](./0026-service-owned-price-sync-memory.md)
- [models.dev](https://models.dev/)
- `src/app_state.rs`
- `src/pricing.rs`
- `src/proxy/raw_capture.rs`
- `web/src/pages/Settings.tsx`
