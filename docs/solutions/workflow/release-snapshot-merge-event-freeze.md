---
title: "Release snapshots should freeze on merge events"
module: "release automation"
problem_type: "unstable release metadata source"
component: "GitHub Actions release snapshot"
tags:
  - github-actions
  - release
  - git-notes
  - squash-merge
status: "active"
related_specs: []
symptoms:
  - "Release cannot find the PR labels or version intent for a main commit."
  - "Squash-merged commits intermittently return no PRs from commit-to-PR lookup."
root_cause: "Release metadata was reconstructed after the fact from commit association instead of frozen when GitHub still had the merged PR event context."
resolution_type: "event-sourced immutable snapshot"
---

# Release snapshot merge-event freeze

## Context

The release pipeline derives version intent from PR labels such as `type:*` and `channel:*`, then stores an immutable release snapshot in `refs/notes/release-snapshots`. That snapshot is the contract consumed by `Release`; publishing should not depend on live PR lookup behavior after a commit lands on `main`.

## Symptoms

- `Release Snapshot` or `Release Meta (snapshot + tags)` fails after a PR is merged.
- The target commit is on `main`, but PR metadata cannot be recovered reliably.
- Failures are more likely after squash merges, where GitHub commit association is not a durable source for the merged PR.
- Rerunning release jobs does not fix the root issue if the snapshot was never frozen.

## Root Cause

The unstable boundary is using `commits/{sha}/pulls` as the primary fact source for release metadata. That API answers an association query after the merge, not an immutable release intent contract.

For release automation, the durable fact is the `pull_request.closed` event with `merged == true`. At that point GitHub provides the PR number, labels, head SHA, title, and `merge_commit_sha` together. Waiting until `push main`, `CI Main`, or `Release` to reconstruct those facts couples publishing to GitHub's later commit association behavior.

## Resolution

Use the merged PR event as the write path and the release snapshot note as the read path:

- Add a `pull_request_target` workflow for `closed` PRs and guard it with `github.event.pull_request.merged == true`.
- Key the snapshot by `github.event.pull_request.merge_commit_sha`.
- Checkout the merge commit and run `release_snapshot.py ensure --target-only --snapshot-source merged-pr`.
- Persist the snapshot to `refs/notes/release-snapshots` before release selection consumes it.
- Let `CI Main` and `Release` consume the immutable snapshot rather than requiring a fresh commit-to-PR reverse lookup.
- Keep manual and historical catch-up paths, but treat them as compatibility layers, not the main source of truth.

## Guardrails / Reuse Notes

- Snapshot creation must be idempotent: rerunning the merged PR workflow for the same `merge_commit_sha` should reuse or overwrite the same immutable fact without changing release intent.
- Contract tests should cover squash merge behavior where commit-to-PR lookup returns no result.
- Contract tests should cover merge commits, reruns, and historical catch-up so the compatibility layer remains bounded and intentional.
- Quality-gates fixtures must include the merged-PR snapshot workflow; otherwise the contract may pass while release topology drifts.
- Release jobs should fail when an expected snapshot is missing. Silent fallback to fresh PR inference hides the exact class of bug this pattern is meant to remove.
- This pattern generalizes to any automation where later jobs need PR labels, approvals, author intent, or other event-time metadata: freeze the event-time fact under the eventual commit key, then consume the frozen record.

## References

- `.github/workflows/release-snapshot-pr.yml`
- `.github/scripts/release_snapshot.py`
- `.github/scripts/test-release-snapshot.sh`
- `.github/scripts/check_quality_gates_contract.py`
