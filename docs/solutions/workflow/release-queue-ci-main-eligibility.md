---
title: "Release queue should skip snapshots without a releasable CI Main result"
module: "release automation"
problem_type: "invalid pending release target"
component: "GitHub Actions release queue"
tags:
  - github-actions
  - release
  - ci-main
  - release-snapshot
status: "active"
related_specs: []
symptoms:
  - "A later successful `CI Main` run triggers `Release`, but `Release Meta` selects an older pending snapshot instead of the triggering SHA."
  - "The selected pending target belongs to a commit whose `CI Main` failed in a non-snapshot job."
  - "Release can publish or attempt to publish a commit that never met the mainline quality gate."
root_cause: "Merged-PR release snapshots are frozen before `CI Main` finishes, but pending-target selection only looked for unreleased snapshots on the first-parent path. It did not re-check whether each pending snapshot still had a releasable `CI Main` outcome."
resolution_type: "pending-target eligibility gate"
---

# Release queue CI Main eligibility

## Context

The release queue is intentionally event-sourced: merged PRs freeze release intent into `refs/notes/release-snapshots`, and later `Release` runs choose the oldest pending snapshot on `main`. That preserves release ordering, but snapshot existence alone is not enough to make a commit releasable.

## Symptoms

- A merge commit gets a valid immutable snapshot from the merged PR event, then later fails `CI Main`.
- A newer merge commit passes `CI Main` and triggers `Release`.
- `Release Meta` still picks the older failed commit because it is the oldest unreleased snapshot in the queue.

## Root Cause

The original pending selector only filtered on two facts:

- the commit has a `release_enabled` snapshot;
- the snapshot's release tag does not already point at that commit.

That logic ignored whether the corresponding `CI Main` run ever produced a releasable result. Because merged-PR snapshots are created before `CI Main`, a failing mainline run left behind a valid-but-ineligible pending snapshot.

## Resolution

Keep the immutable snapshot queue, but add a second eligibility gate when selecting pending targets:

- Accept commits with a successful `CI Main` run on `main`.
- Also accept the existing compatibility carve-out where `CI Main` failed only in the `Release Snapshot` job and all other jobs succeeded.
- Skip pending snapshots that fail both checks, and continue scanning newer pending snapshots on the first-parent path.
- Apply the same eligibility filter both when choosing the initial pending target and when continuing the release queue after a publish.

## Guardrails / Reuse Notes

- Do not delete or mutate immutable snapshots just because `CI Main` failed; snapshot history is still useful for audit and catch-up logic.
- The eligibility filter belongs at selection time, not snapshot creation time. Merge-event snapshot freezing must remain independent from later CI outcomes.
- Reuse the exact same success-or-snapshot-only-failure rule for automatic queue selection and manual backfill validation so the release contract stays coherent.
- Add regression coverage for both cases:
  - an older pending snapshot is skipped because `Backend Tests` failed;
  - a snapshot-only `CI Main` failure remains releasable.

## References

- `.github/workflows/release.yml`
- `.github/scripts/release_snapshot.py`
- `.github/scripts/test-release-snapshot.sh`
