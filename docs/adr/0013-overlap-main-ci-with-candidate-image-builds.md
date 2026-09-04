# ADR 0013: Overlap main CI with candidate image builds

## Status

Accepted

Automatic publication currently waits for all of `CI Main`, then repeats the two native image builds, smoke checks, and cache export in `Release`. Historical successful runs therefore have a median merge-to-publication time of 20 minutes 35 seconds, with no observed run meeting the 10-minute objective. The workflow will instead build an immutable, smoke-verified candidate image for each releasable main SHA while CI runs, then make `Release` promote those exact candidates only after the matching CI Main run succeeds. This preserves the release boundary while turning the two long stages from serial work into overlapping work.

## Decision

- A **candidate image** is an architecture-specific, SHA-derived GHCR tag that has completed local smoke. It is not a version tag, `latest`, a multi-architecture manifest, a Git tag, a GitHub Release, or a deployment source. `CI Main` may push `candidate-<sha12>-amd64` and `candidate-<sha12>-arm64` only for a release-enabled snapshot.
- A new early CI metadata job reads the immutable merged-PR snapshot for the exact main SHA. If the merge-event snapshot has not arrived yet, it idempotently creates the same target-only snapshot before exporting the version and candidate suffix. Both candidate builders check out that target SHA, build, smoke, and push their architecture tag. A candidate build or smoke failure fails CI Main and cannot be promoted.
- `Release` continues to serialize with `release-main`, select the existing queued target, validate both candidate tags, create the version manifests, verify platforms, and create the Git tag and GitHub Release. It must not rebuild a missing candidate for an automatic main release. This keeps ordered release semantics while leaving only promotion on the post-CI critical path.
- `workflow_dispatch` remains compatible with historical main commits. When both exact candidate tags exist it promotes them; when either is absent it may use the existing synchronized build-and-smoke path as a manual-backfill fallback. That fallback is intentionally outside the automatic merge-to-publication SLO.
- The Dockerfile declares `APP_EFFECTIVE_VERSION` only immediately before the version-dependent web and Rust application builds, not before dependency installation or the dummy Rust dependency build. A release version changes every publication; declaring it earlier invalidates otherwise reusable dependency layers.
- Stateful SQLite validation is split into two deterministic `cargo nextest` hash partitions (`hash:1/2` and `hash:2/2`) after the shared archive producer. Each keeps the established six test threads and a private schema template. Their aggregate remains the sole owner-facing `Backend Tests (Stateful SQLite)` required check; it fails unless both shards succeed. Thus every test remains mandatory without adding a branch-protection check or increasing per-test concurrency.
- The `backend-test` image moves to a separate success-only `workflow_run` workflow for CI Main. It checks out and tags the CI run's exact head SHA, remains auxiliary to branch protection and Release, and never supplies a release image digest.

## Consequences

- The automatic critical path becomes `max(CI quality gates, candidate build and smoke) + manifest/tag/Release promotion`, rather than `CI Main + candidate build and smoke + promotion`.
- The acceptance metric is the elapsed time from automatic CI Main creation for an eligible main SHA to completion of that SHA's GitHub Release creation. Across the most recent ten eligible automatic releases, P95 must be at most 10 minutes and the operating target for P50 is at most 7 minutes. Candidate cache-hit evidence and each Stateful shard duration are retained with that measurement so a regression identifies its owner.
- Workflow contracts, fixtures, and branch-protection declarations must retain the three existing backend check names. The new shard jobs and candidate jobs are implementation detail, not new required checks.
- Candidate retention and deletion are deliberately not automated by this decision. Any GHCR cleanup requires a separately approved retention policy and an exact, recoverable target definition.

## Considered Options

- Keep image builds in Release: rejected because it guarantees the observed serial critical path and cannot meet the objective through minor cache tuning alone.
- Publish version tags before CI completes: rejected because a failing quality gate could leave a user-visible release that was never fully validated.
- Add two Stateful shard checks directly to branch protection: rejected because it changes the external required-check contract; an aggregate check preserves that interface while retaining complete coverage.
- Rebuild missing automatic candidates in Release: rejected because it silently restores the slow path and weakens the guarantee that promotion uses CI-smoked artifacts.
