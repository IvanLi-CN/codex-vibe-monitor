# Repository Guidelines

## Project Structure & Module Organization

- `src/` — Rust backend (polling scheduler, Axum HTTP API, SSE fan-out, SQLite persistence). Start with `main.rs`. Configuration lives in `AppConfig` and reads `.env.local`.
- `web/` — Vite + React + TypeScript SPA. `src/components/` hosts UI atoms (Tailwind + shadcn 风格基础组件), `src/hooks/` encapsulates API + SSE integration, and `vite.config.ts` wires the proxy to the Rust server.
- `Dockerfile` — multi-stage build assembling the Rust binary and front-end assets.
- Generated artifacts (`target/`, `web/dist/`, SQLite DBs) stay untracked.

## Build, Test, and Development Commands

- `cargo fmt` — format Rust sources with project defaults.
- `cargo check` — type-check backend without producing binaries.
- `cargo run` — start the backend (reads `.env.local`, listens on `127.0.0.1:8080`).
- `cd web && npm install` — install SPA dependencies once per setup.
- `cd web && npm run dev -- --host 127.0.0.1 --port 60080` — run the front-end dev server with proxy to the backend.
- `cd web && npm run build` — produce production assets for `web/dist`.
- `cd web && npm run test` — execute front-end unit tests (Vitest).

## Development Runtime (Background, Non-blocking)

Use non-blocking runtime management for long-lived services, but do not require any specific process manager.

- Backend start: `cargo run` (default `127.0.0.1:8080`)
- Frontend start: `cd web && npm install && npm run dev -- --host 127.0.0.1 --port 60080`
- Readiness checks:
  - Backend: `curl -sS -m 1 http://127.0.0.1:8080/health | grep -q ok`
  - Frontend: `curl -sS -m 1 http://127.0.0.1:60080/ >/dev/null`
- If any required port is occupied, resolve the conflict before starting services.
- Keep logs and process ownership explicit so services can be stopped reliably.
- Do not use `alarm` for long-running services; it is only suitable for one-off commands.

## Coding Style & Naming Conventions

- Rust: rely on `rustfmt` defaults (4-space indent, snake_case for modules/variables, CamelCase for types). Use expressive error contexts via `anyhow::Context`.
- TypeScript/React: prefer functional components, hooks, and explicit return types. Use PascalCase for components, camelCase for hooks/utilities. Tailwind utility classes stay in JSX; shared styles belong in `index.css`.
- Keep comments purposeful—explain non-obvious logic rather than restating code.

## Testing Guidelines

- Backend tests should target polling, persistence, and aggregation (use `#[tokio::test]` + in-memory SQLite). Place files beside implementation or under `tests/` as integration suites.
- Front-end testing uses Vitest + React Testing Library. Name files `ComponentName.test.tsx` and cover hooks (`useXyz.test.ts`).
- Run relevant suites (`cargo test`, `npm run test`) before opening a PR; aim to cover new branches and SSE flows with mocks.

## Commit & Pull Request Guidelines

- Follow Conventional Commits (`feat:`, `fix:`, `chore:`, etc.). Scope components with meaningful tags (`feat(web): add stats chart`).
- Pull requests should include:
  - Summary of changes and affected areas (backend API, SPA view, Docker, etc.).
  - Linked issue or task reference where applicable.
  - Verification steps or screenshots/gifs for UI updates.
  - Notes about config or schema changes (e.g., migrating SQLite, new env vars).

## CI / CD (Release via PR labels)

- Releases only happen on `push` to `main` (after PR merge). PR builds never publish artifacts.
- Every PR must set exactly **one** release type label and exactly **one** release channel label
  (enforced by `.github/workflows/label-gate.yml`).
- Release type (`type:*`):
  - `type:patch` | `type:minor` | `type:major` - trigger a release (semver bump).
  - `type:docs` | `type:skip` - skip release (no image/tag/GitHub Release).
- Release channel (`channel:*`):
  - `channel:stable` - stable release (`vX.Y.Z`, also updates the Docker image `latest` tag).
  - `channel:rc` - pre-release (`vX.Y.Z-rc.<sha7>`, does not update `latest`).
- For more details, see `README.md` and `.github/scripts/compute-version.sh`.

## Security & Configuration Tips

- Store authentication cookies and secrets in `.env.local`; the file is ignored—never commit credentials.
- SQLite files default to `codex_vibe_monitor.db` in the repo root; add alternate paths via `DATABASE_PATH` if running in containers.
- SSE and HTTP clients depend on stable polling; monitor logs (`RUST_LOG=info cargo run`) when adjusting concurrency or timeouts.
