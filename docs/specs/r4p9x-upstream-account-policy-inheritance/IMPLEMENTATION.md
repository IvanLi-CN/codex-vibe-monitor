# Implementation

## Backend

SQLite schema adds nullable account and group `policy_*` columns for policy overrides and extends tags with upstream 429 retry fields.

Runtime policy resolution now builds one effective `EffectiveRoutingRule` per account, records field-level sources for the final values, and feeds the effective rule to routing selection, sticky behavior, FAST rewriting, concurrency limiting, and upstream 429 retry.

## Frontend

The API client normalizes the expanded routing policy surface on tags, groups, and effective account rules.

The tag rule dialog edits upstream 429 retry alongside the existing routing controls. Group settings expose a routing policy editor entry, account detail exposes an account policy editor from the routing tab, and the effective routing card displays concurrency, upstream 429 retry state, and a field source breakdown for root, group, tag, and account layers.

## Validation

Validation covers:

- backend policy resolution across group, tag, and account layers
- upstream 429 retry in the final effective policy
- tag-layer override of group policy plus account override source tracking
- frontend payload normalization for routing policy fields
- tag dialog submission with expanded policy payloads
