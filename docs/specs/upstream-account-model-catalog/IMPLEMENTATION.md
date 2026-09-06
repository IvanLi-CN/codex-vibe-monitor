# Implementation

## Current Shape

- The catalog is stored separately from account routing policy and model mappings, with one persisted snapshot per upstream account.
- Provider adapters own credential extraction, endpoint construction, proxy scope, bounded network execution, response parsing, and safe error classification.
- Account detail returns the catalog snapshot; an explicit refresh endpoint performs discovery and returns the refreshed account detail.
- The selector treats project presets and account-discovered IDs as candidate metadata while persisting only the existing string ID list.

## Compatibility

- Existing accounts receive an empty catalog state after the schema migration.
- Existing routing rules, model mappings, and `/v1/models` behavior are unchanged.
- Provider kinds without an adapter remain readable but cannot be refreshed.

## Invariants

- The last successful snapshot remains available after a failed refresh.
- Catalog IDs are trimmed, non-empty, and exactly deduplicated.
- Refresh state is serialized per account; concurrent refreshes for different accounts remain independent.
- Safe errors contain status/classification only and never credentials or raw response bodies.
