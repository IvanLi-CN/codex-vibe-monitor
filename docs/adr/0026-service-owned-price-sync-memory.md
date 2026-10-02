# Service-Owned Price Sync Memory

## Status

Accepted

## Context and Decision

Price synchronization is a system management capability. The owner's model and provider choices, and the record of models previously encountered during review, therefore belong to the service instance. Persist this memory on the server and share it across browsers and devices connected to that instance; preserve it across service restarts.

Browser-local persistence was considered because it keeps review preferences independent between clients. It was rejected because switching browsers or devices would otherwise lose the system's established choices and discovery history. Persisted sync memory remains separate from the effective pricing catalog and proxy preset membership; changing memory does not itself import a quote or enable a model preset.

Save model and provider selection changes immediately. Canceling a price review leaves successfully saved memory intact; it cancels candidate-price application. Saving memory only after successful price application was rejected because dismissing a review would otherwise lose intentional deselections.

Remember model checkbox choices by exact model ID and provider ID, and remember an explicit quote-provider choice separately. Changing quote providers must not carry the old provider's selection into a different quote. Restoring a missing or filtered-out chosen provider must not silently authorize an alternative provider's price. Discovery history remains keyed by model ID, so another provider's quote does not make an existing model new.

## Consequences

- Sync memory is shared system state; it is not a private preference of the current browser or device.
- A local browser cache may reflect server state but cannot be its authority.
- Requirements for selection restoration and discovery behavior belong to the existing model-management price-sync topic.

## References

- [Model management and price sync](../specs/model-management-price-sync/SPEC.md)
- [ADR 0023: Model management and manual price synchronization](./0023-model-management-and-manual-price-synchronization.md)
