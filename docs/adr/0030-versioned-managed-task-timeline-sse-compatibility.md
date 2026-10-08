# Versioned Managed Task Timeline SSE Compatibility

Status: Accepted

## Context

The timeline chart needs marker-only SSE notifications and fixed-watermark HTTP pagination so large histories do not exceed the bounded SSE snapshot capacity. Replacing the existing interval-row topic in place breaks consumers of `system.managed-tasks.timeline/v1` and would require a major release. This release must remain within the current major.

## Decision

Keep the existing `system.managed-tasks.timeline` descriptor without `schemaVersion` on schema epoch `/v1`, preserving its interval snapshot and delta payloads and its existing 10,000-segment capacity behavior. Add `schemaVersion: "2"` as an opt-in descriptor parameter for schema epoch `/v2`; this version sends only `watermark` and `observedAt`, with interval data read through the fixed-watermark HTTP page API. The built-in task page selects v2. Both versions refresh from the same persisted timeline changes.

## Consequences

The public API change is additive within the current major and has minor impact. Existing v1 consumers keep their contract; consumers that need histories beyond the legacy capacity opt into v2 and page through HTTP. Removing v1 requires a future major-release decision. Cache identity and resume cursors remain separated by descriptor parameters and schema epoch.
