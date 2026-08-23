# Repository Agent Guide

Use this file as the repository-specific decision map. Read the referenced script or workflow when a task enters that branch; keep this file focused on choices that are easy to get wrong.

## Repository Map

- `src/` — Rust backend: Axum HTTP/SSE, polling, proxying, account routing, SQLite persistence, and maintenance.
- `web/` — React/Vite/TypeScript application, unit tests, Storybook, and Playwright suites.
- `docs-site/` — public documentation site. `docs/` — internal deployment notes, UI guidance, Specs, and reusable Solutions.
- `.github/workflows/` and `.github/scripts/` — CI, quality-gate, release, and backend test contracts.
- `Dockerfile` — production image assembly. Keep `target/`, `web/dist/`, and SQLite database files untracked.

## Verification Contract

Choose the smallest complete validation path for the changed surface.

- Targeted Rust regression: run `cargo test <selector> -- --nocapture` for named tests. Keep tests in the matching resource bucket under `src/tests/` or `src/upstream_accounts/tests/`.
- Backend resource profiles: use the shared runner, not a hand-built full-suite Cargo command:
  - `bash .github/scripts/run-backend-tests.sh --profile lightweight`
  - `bash .github/scripts/run-backend-tests.sh --profile stateful-sqlite`
  - `bash .github/scripts/run-backend-tests.sh --profile archive-file-io`
- Full local backend regression: run `bash .github/scripts/run-backend-tests.sh`. Without `--profile`, it runs the three profiles sequentially. `cargo-nextest` must be installed; a missing binary is an environment blocker.
- CI full backend regression: the archive producer runs `cargo nextest archive --locked --all-features`, then the three profile jobs replay that archive independently. Treat those jobs as controlled parallel units; do not invent local background-process concurrency for them.
- A task, Spec, or compatibility gate may explicitly request a Cargo compatibility command; run that command exactly. Do not infer a full `cargo test --all-features` entry point by copying `--all-features` from Clippy or nextest.
- Rust checks: `cargo fmt --all -- --check`, `cargo check --locked --all-targets --all-features`, and `cargo clippy --locked --all-targets --all-features -- -D warnings`.
- Web unit tests: `cd web && bun run test`. Use `cd web && bun run test-storybook`, `cd web && bun run test:e2e`, or `cd web && bun run test:e2e:pwa` only when the changed surface requires them. Use `bun run typecheck:web`, `bun run lint:web`, and `cd web && bun run build` for the corresponding checks.
- Changes to docs or repository tooling should use the relevant `bun` script and the focused contract script under `.github/scripts/` or `scripts/`.

Backend test placement follows the resource contract: database-only behavior belongs in `lightweight` or `stateful_sqlite`; real archive, file-path, gzip, corruption, and write-lock behavior stays in `archive_file_io`. Preserve production timing and thresholds unless the test-only seam is part of the task.

## Runtime and Ports

- Long-lived local services run in a non-blocking session with explicit logs and process ownership. Read `$global-port-manager` before starting any service that listens on a port.
- Inspect a known/default port first, lease it to the current repository scope, and start the process only after the lease exists. If another scope owns it or an unknown process is listening, allocate a different port; never take over or kill it.
- Backend default: `127.0.0.1:8080`. Override with `HTTP_BIND=127.0.0.1:<port> cargo run` or the CLI `--http-bind` option.
- Frontend default: `127.0.0.1:60080`. Start with `cd web && VITE_APP_PORT=<port> VITE_BACKEND_PROXY=http://127.0.0.1:<backend-port> bun run dev -- --host 127.0.0.1 --port <port>` when either service uses a leased non-default port.
- Readiness checks must target the leased ports: `curl -sS -m 1 http://127.0.0.1:<backend-port>/health | grep -q ok` and `curl -sS -m 1 http://127.0.0.1:<frontend-port>/ >/dev/null`.
- Keep the lease alive for long-running services and share a localhost URL only while the current scope owns the active service lease and background session.

## Worktrees and Configuration

- `lefthook` version `2.1.7` or newer must be available outside repo-local `node_modules/.bin` before `bun run hooks:install` or linked-worktree setup.
- Use `bun run worktree:bootstrap` for manual recovery and `bun run worktree:setup` for locked dependency-surface restoration. The post-checkout hook copies only missing declared resources, never overwrites an existing `.env.local`, and never copies dependency or runtime directories. Automatic recovery may warn without blocking checkout; manual bootstrap returns non-zero when recovery fails.
- Keep credentials and authentication cookies in ignored `.env.local`. Use `DATABASE_PATH` to select a non-default SQLite file in local or container environments; never commit database files or secrets.
- Use `$shared-testbox-runner` for Docker/Compose integration tests. Keep remote writes under `/srv/codex/**`, use a unique run/project, and clean only resources created by that run.

## Code and UI Style

- Rust follows `rustfmt`, snake_case names, CamelCase types, and `anyhow::Context` for useful error context. Comments explain non-obvious reasoning, not syntax.
- React/TypeScript uses functional components, hooks, explicit return types, PascalCase components, and camelCase hooks/utilities. Keep Tailwind utilities in JSX and shared styles in `index.css`.
- UI or rendered-surface changes require the applicable `$ui-visual-evidence` workflow before claiming verification or entering PR delivery. Do not create visual evidence for non-UI work.

## Git, PR, and Release

- Work from a `th/` topic branch. Protect `main`: use a PR, do not push directly, and do not force-push or rewrite history without explicit authorization.
- Commits use English Conventional Commits, cryptographic signing, and `--signoff`; these are separate requirements. Example: `git commit -S --signoff -m "fix(web): clarify runtime contract"`.
- A PR targeting `main` must carry exactly one release-type label (`type:patch`, `type:minor`, `type:major`, `type:docs`, or `type:skip`) and exactly one channel label (`channel:stable` or `channel:rc`). The label gate does not apply to PRs targeting another base branch.
- `type:patch|minor|major` enables a release; `type:docs|skip` skips publication. `channel:stable` publishes the stable tag and `latest`; `channel:rc` publishes a pre-release without updating `latest`.
- Release queue, snapshot, manual backfill, and override behavior are defined by `.github/workflows/release.yml`, `.github/scripts/release_snapshot.py`, `.github/scripts/compute-version.sh`, and the release section of `README.md`. Follow those sources instead of duplicating their state machine here.
- Before PR delivery, report the changed surfaces, relevant validation, config/schema changes, and UI evidence when applicable. Keep the current PR head and required checks fresh after every commit.

## Safety Boundaries

- Preserve unrelated user changes. Do not reset, clean, stash, overwrite, or delete broad paths to make a task convenient.
- Treat credentials, cookies, database files, raw payloads, and external service data as sensitive. Redact them from logs and responses.
- External writes, uploads, permission changes, real-program screenshots, remote GitHub operations, and PR publication require the active flow's authorization. Login, CAPTCHA, and 2FA always pause for the owner.
