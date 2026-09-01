---
title: "Release should target the triggering successful CI Main head"
module: "release automation"
problem_type: "invalid pending release target"
component: "GitHub Actions release target selection"
tags:
  - github-actions
  - release
  - ci-main
  - release-snapshot
status: "active"
related_specs: []
symptoms:
  - "A successful `CI Main` run triggers `Release`, but Release Meta selects an older pending snapshot instead of the triggering SHA."
  - "The selected historical target retains a release contract that current policy has removed."
  - "A historical target changes workflow files relative to the default branch, so the built-in GitHub Actions token cannot create its tag."
root_cause: "The automatic release queue treated immutable snapshot history as a FIFO publication backlog instead of treating the successful CI Main head as the release candidate."
resolution_type: "current-head release coalescing"
---

# Release current CI Main head

## Context

Merged PRs freeze release intent into `refs/notes/release-snapshots`. Those notes are audit facts, not a mandate to expose every historical tree as a public runtime release. A successful `CI Main` has already identified the current mainline tree that passed the full gate.

## Symptoms

- A newer mainline commit passes `CI Main` and triggers `Release`.
- `Release Meta` selects an older ancestor because it is first in the pending queue.
- The ancestor is obsolete or differs from the default branch in workflow files, and tag creation fails after runtime manifests are already pushed.

## Root Cause

The original selector searched the first-parent history for unreleased snapshots. That made the actual release candidate a function of old repository history rather than the CI run that supplied its evidence. Even an eligibility filter cannot prove that an older release is still the intended runtime state.

## Resolution

Forward the triggering successful `CI Main` head SHA directly through the automatic Release workflow:

- Automatic Release loads and publishes only `workflow_run.head_sha`.
- Manual backfill loads and publishes only its explicit SHA after successful-CI-Main validation.
- Do not scan, dispatch, or mutate historical pending snapshots.

## Guardrails / Reuse Notes

- Do not delete or mutate immutable snapshots simply because they are coalesced into a later release.
- The automatic path must not make a GitHub Actions API query to reorder historical targets.
- The direct target selector and the absence of `next-pending` are contract-tested, so FIFO continuation cannot silently return.
- Manual backfill remains the only path that deliberately selects a historical mainline SHA.

## References

- `.github/workflows/release.yml`
- `.github/scripts/release_snapshot.py`
- `.github/scripts/test-release-snapshot.sh`
- `docs/adr/0008-release-current-ci-head-coalescing.md`
