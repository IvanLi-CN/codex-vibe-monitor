# LiteLLM Price Catalog and Multidimensional Quotes

## Status

Accepted

## Context

The reviewed model-price flow previously normalized a directory response into five legacy token fields. That shape cannot represent the image, audio, tier, threshold, and duration dimensions needed by current OpenAI quotes, and its source coverage omitted models present in the maintained LiteLLM catalog.

## Decision

This ADR supersedes the price-source and single-dimension limitations in ADR 0023. ADR 0023 remains the governing workflow for user-triggered review, compatibility, and manual maintenance where this ADR does not replace it.

Use the LiteLLM maintained raw JSON at `https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json` as the only online price source for the synchronization flow. Fetch it on explicit preview, retain the response SHA-256 and retrieval time, and do not advance the durable discovery baseline when retrieval or parsing fails.

Normalize source entries by model ID and provider marker, while keeping source metadata, lifecycle fields, and every numeric rate. A rate stores its original key/value/unit plus a readable normalized value/unit and optional modality, processing tier, and threshold. A model may have a complete legacy token projection, a quote-only record, or both. Only a valid generic token input/output pair updates `pricing_settings_models`; missing dimensions remain missing and are never filled with placeholders. A future deprecation date remains metadata and does not hide or disable a quote.

Persist quotes in an additive model-keyed SQLite table containing serialized quote records. The settings and synchronization APIs add an optional `quotes` collection while retaining the existing `entries`, five legacy fields, and `importable` semantics. Omitting quotes from an old request preserves stored quote rows. Explicit apply and delete operations update quote rows, the legacy projection, and managed-model membership in one transaction. Manual edits mark the legacy projection `custom` and preserve unrelated quote dimensions; reviewed source applies mark it `litellm`.

The review UI exposes complete source rates, expands standard token rates by default, and lets users inspect other dimensions. The existing provider-selection, model-selection, search, retry, preset, and memory contracts remain governed by ADR 0026 and the model-management Spec. Runtime cost estimation continues to use the legacy projection and historical costs are unchanged.

## Consequences

- OpenAI model coverage follows LiteLLM's maintained catalog, including the GPT Image 2.5 entries and their text/image token rates.
- The project maintains only adapter logic and user-confirmed local snapshots; it does not curate an upstream model or price list and does not add a fallback source.
- Existing clients remain compatible because legacy fields and request forms continue to work; clients that understand `quotes` can inspect and preserve complete dimensions.
- The additive quote table requires a minor release because persistent state and settings responses gain new capabilities.

## References

- [ADR 0023: Model management and manual price synchronization](./0023-model-management-and-manual-price-synchronization.md)
- [ADR 0018: OpenAI GPT-6 pricing and usage semantics](./0018-openai-gpt-6-pricing-and-usage-semantics.md)
- [ADR 0026: Service-owned price sync memory](./0026-service-owned-price-sync-memory.md)
- [Model management price sync Spec](../specs/model-management-price-sync/SPEC.md)
- `src/app_state.rs`
- `src/pricing.rs`
- `src/api/slices/model_management.rs`
