# ADR 0008: Release the triggering CI Main head

## Status

Accepted

## Context

Merged pull requests freeze immutable release snapshots before `CI Main` completes. The former automatic release queue scanned the oldest unreleased snapshot on the first-parent path. That made a successful current `CI Main` release an older ancestor instead of the commit that had just passed its gate.

The intervening snapshots may be obsolete or may retain a release contract that current policy has removed. A release selected from history can also differ from the default branch in workflow files, which prevents the built-in GitHub Actions token from creating its tag. The resulting failure left smoke-verified registry manifests without a corresponding release tag.

## Decision

- An automatic `Release` consumes exactly `workflow_run.head_sha` from the successful triggering `CI Main` run.
- A manual backfill consumes exactly its explicitly requested mainline SHA after the existing successful-CI-Main validation.
- The release workflow does not scan or dispatch historical pending snapshots, and `release_snapshot.py` no longer exposes a pending-selector command.
- Immutable snapshots remain in git notes for audit, version history, and explicit manual backfill. An untagged ancestor is coalesced into the later published mainline target rather than being published solely to drain history.
- The existing release gates remain ordered around the selected target: successful CI Main, per-platform runtime build and smoke, manifest verification, git tag, and GitHub Release.

## Consequences

The automatic path publishes the state that actually passed the current mainline gate and prevents a later healthy CI result from reviving an obsolete ancestor. Intermediate snapshots can remain untagged, but their code is included in the descendant mainline release and their immutable records remain inspectable.

Manual backfill remains available for a deliberately chosen historical mainline commit. It is no longer an implicit continuation mechanism.

## Considered Options

- Keep FIFO historical continuation: rejected because it can publish a state superseded by current release policy and can fail tag creation when the old target changes workflow files relative to the default branch.
- Delete or rewrite old snapshots: rejected because snapshots are immutable audit facts and a missing tag is sufficient to represent an unissued release.
- Publish every historical snapshot manually: rejected because it treats queue cleanup as a reason to expose obsolete runtime code.
