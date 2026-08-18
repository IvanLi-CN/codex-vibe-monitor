---
title: Scoped Git hooks and fingerprinted worktree bootstrap
module: developer workflow
problem_type: local feedback latency
component: Lefthook and linked worktree setup
tags:
  - git-hooks
  - lefthook
  - worktree
  - bun
  - cargo
status: active
related_specs:
  - docs/specs/v7se4-worktree-bootstrap/SPEC.md
  - docs/specs/tr4ev-bun-first-toolchain/SPEC.md
---

# Scoped Git hooks and fingerprinted worktree bootstrap

## Context

Repository-wide type checking, linting, formatting, and dependency recovery provide valuable coverage, but running all of them on every commit or linked-worktree checkout turns routine Git operations into a slow and unreliable feedback loop. CI is the authoritative full verification boundary; local automatic work should be proportional to the files and dependency surface that changed.

## Root cause

- A pre-commit command without a staged-file boundary runs the same full repository work for a Markdown-only or Rust-only change.
- Linked worktrees do not share `node_modules`, so unconditional checkout setup repeats three Bun installs and Cargo fetch even when their manifests and lockfiles have not changed.
- A shared blocking synchronization lock serializes unrelated linked worktrees and can make Git checkout wait for another checkout's local resource copy.

## Resolution

- Keep only staged-path formatters in `pre-commit`: Biome for web files, Rustfmt with the repository edition for Rust, and dprint for Markdown. Keep Lefthook `stage_fixed`, but add a managed pre-commit wrapper that runs the formatter wrapper's `--check` mode before Lefthook hides partial changes. The check and formatter share their path filter, so it does not depend on Lefthook's shared Git common-directory patch; the normal Lefthook path can then stage the verified formatter outputs. Treat a marker-bearing standard pre-commit template as a safe predecessor to migrate, but preserve every other existing hook.
- Put full checks behind explicit commands (`bun run typecheck:web`, `bun run verify:rust`) and retain their existing CI steps and required job names.
- Store one status and digest per dependency surface in each worktree's Git metadata. Digest root Bun, web Bun, docs Bun, and Cargo from their manifests, lockfiles, and toolchain input. A reusable Bun state needs every manifest-direct package directory; Cargo needs every registry archive represented by `Cargo.lock` plus the cache-relative layout captured after a successful fetch. Restore only a missing or changed surface; `--force` is the intentional escape hatch.
- Record a failed digest. Automatic checkout logs and suppresses repeat attempts for that exact input, while manual setup retries it and input changes reactivate automatic recovery.
- Place resource-copy and setup locks in each worktree's Git metadata. Use an advisory lock held by the process that executes the protected work, so lock ownership ends with that process rather than a stale PID file. Treat an active automatic lock as a nonblocking skip.
- Delete a legacy `prepare-commit-msg` wrapper only after generating its standard Lefthook template from a temporary repository that declares that hook and comparing bytes. Update any managed hook only after the same configured-template comparison; preserve symlinks, configured hooks, and any non-identical local hook.

## Guardrails

- A formatter wrapper must filter deleted, nonexistent, and paths containing any symlink component before invoking a formatter and must receive only Lefthook staged files.
- Test partial staging with a real Lefthook run and a formatter fixture that would rewrite staged content; assert that the guard fails before it changes either the index or the worktree.
- CI tooling jobs install the pinned Lefthook binary globally; smoke invokes that repository-external executable directly, and production hook installation requires the same external `PATH` precondition.
- State files must contain no copied resources, environment values, credentials, or dependency directory paths.
- Do not move full static analysis out of CI or alter required check names to make local Git operations faster.
- A new dependency surface needs an independent digest, presence test, failure status, and smoke-test assertion before it can be added to automatic setup.

## References

- `lefthook.yml`
- `scripts/format-staged-files.sh`
- `scripts/worktree-setup.sh`
- `scripts/sync-worktree-resources.sh`
- `scripts/test-git-hooks-contract.sh`
- `scripts/test-worktree-bootstrap.sh`
