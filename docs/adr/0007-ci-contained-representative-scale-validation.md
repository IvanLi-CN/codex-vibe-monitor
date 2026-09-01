# ADR 0007: CI-contained representative-scale validation

## Status

Accepted

Representative-scale validation uses only the target commit, deterministic fixture, independent oracle, and the CI workflow that executes them. PR and Main remain hard gates for the Summary deadlines and exactness contract; Release consumes the same target SHA's successful CI Main result and performs its runtime-image smoke, but never consumes a host path, testbox result, production-copy, manually supplied receipt, or release-snapshot backend-test digest. This supersedes ADR 0006 so the repository retains a project-owned `backend-test` environment without making publication depend on external local validation state.

## Considered Options

- Keep a data-blind production-copy receipt: rejected because its producer and transfer boundary are outside GitHub-hosted CI, so it cannot form an executable release contract.
- Re-run representative-scale validation in Release: rejected because it duplicates the same deterministic CI evidence without improving provenance.
