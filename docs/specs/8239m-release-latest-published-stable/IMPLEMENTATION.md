# Release `latest` 与当前 CI Main stable - Implementation

## Current State

- Canonical spec: `docs/specs/8239m-release-latest-published-stable/SPEC.md`
- Automatic Release forwards the successful `CI Main` head SHA directly to snapshot loading, runtime smoke, tag creation, and GitHub Release creation.
- Manual backfill retains its explicit mainline SHA and successful-CI-Main validation.
- Historical release snapshots remain immutable audit facts; they are not automatic release-queue work items.

## Validation

- `bash .github/scripts/test-release-snapshot.sh`
- `bash .github/scripts/test-quality-gates-contract.sh`
- `bash .github/scripts/test-live-quality-gates.sh`
