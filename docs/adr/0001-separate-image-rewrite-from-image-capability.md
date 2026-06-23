# ADR 0001: Separate image rewrite from image capability

## Status

Accepted

## Context

The routing system needs two different image-related concerns:

- request-local rewrite policy for group/account routing
- persisted, observed capability on the upstream account itself

These concerns change at different times and come from different sources of truth. A single enum would couple operator intent, request mutation, and capability discovery into one field and make `keep_original` ambiguous.

## Decision

- Keep `imageToolRewriteMode` on the group/account routing rule path only.
- Persist `imageToolCapability` on the account as read-only discovered state.
- Treat `keep_original` as "follow observed capability", where `unknown` remains eligible and only `unsupported` is excluded.
- Treat `fill_missing` and `force_add` as image-compatible request rewrite modes.
- Treat `force_remove` as an image-incompatible request rewrite mode.
- Keep direct image endpoints on capability-based routing only; rewrite stays in the Responses family.

## Consequences

- Group and account policy can express different image-tool behavior without adding a separate image pool.
- Capability learning can evolve independently from operator policy.
- Tag policy stays unchanged.
- The UI can show a stable capability badge without turning it into an operator toggle.
