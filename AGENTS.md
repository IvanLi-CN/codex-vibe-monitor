# Repository Guidelines

## Project Structure & Module Organization

- `src/` — Rust backend (polling scheduler, Axum HTTP API, SSE fan-out, SQLite persistence). Start with `main.rs`. Configuration lives in `AppConfig` and reads `.env.local`.
- `web/` — Vite + React + TypeScript SPA. `src/components/` hosts UI atoms (DaisyUI/Tailwind), `src/hooks/` encapsulates API + SSE integration, and `vite.config.ts` wires the proxy to the Rust server.
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

This workflow avoids blocking the shell and strictly prohibits using `alarm` for long‑running services. The only acceptable use of short timeouts is for one‑off, non‑service commands (e.g., a build), never for dev servers.

- Always run long-lived dev services via `devctl` (Zellij background sessions), either directly or through `scripts/` wrappers:
  - Backend start: `./scripts/start-backend.sh`
  - Frontend start: `./scripts/start-frontend.sh`
  - Status: `./scripts/dev-status.sh`
  - Stop: `./scripts/stop-backend.sh` / `./scripts/stop-frontend.sh`

- Do **not** launch these services by hand with ad-hoc `nohup`/`cargo run`/`npm run dev` commands.
- Do **not** use `alarm` for any long-running service (backend, Vite); it is only allowed for one-off commands that might hang.

Service management (recommended patterns)

- Backend (Rust, port `8080`):
  - Start (detached via zellij):
    - `~/.codex/bin/devctl --root $(pwd) up backend -- env RUST_LOG=info cargo run`
  - Readiness (max 60s):
    - `SECS=0; until curl -sS -m 1 http://127.0.0.1:8080/health | grep -q ok; do sleep 1; SECS=$((SECS+1)); if [ $SECS -ge 60 ]; then echo 'backend not ready'; ~/.codex/bin/devctl --root $(pwd) logs backend -n 200; exit 1; fi; done`
  - Logs:
    - `~/.codex/bin/devctl --root $(pwd) logs backend -n 200`
  - Stop:
    - `~/.codex/bin/devctl --root $(pwd) down backend`

- Front-end (Vite, port `60080`):
  - Start (detached via zellij):
    - `~/.codex/bin/devctl --root $(pwd) up frontend -- bash -lc 'cd web && npm run dev -- --host 127.0.0.1 --port 60080 --strictPort true'`
  - Readiness (max 90s):
    - `SECS=0; until curl -sS -m 1 http://127.0.0.1:60080/ >/dev/null; do sleep 1; SECS=$((SECS+1)); if [ $SECS -ge 90 ]; then echo 'frontend not ready'; ~/.codex/bin/devctl --root $(pwd) logs frontend -n 200; exit 1; fi; done`
  - Logs:
    - `~/.codex/bin/devctl --root $(pwd) logs frontend -n 200`
  - Stop:
    - `~/.codex/bin/devctl --root $(pwd) down frontend`

Operational notes

- One service per zellij session; stopping is always `devctl down <service>` (focus-independent).
- Logs are written to `.codex/logs/<service>.log` so viewing does not depend on focused panes.
- If a port is in use, the wrapper scripts will refuse to start and you must resolve the conflict manually.
- Vite dev server proxies to the backend as configured in `web/vite.config.ts`.

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
- SQLite files default to `codex_vibe_monitor.db` in the repo root; add alternate paths via `XY_DATABASE_PATH` if running in containers.
- SSE and HTTP clients depend on stable polling; monitor logs (`RUST_LOG=info cargo run`) when adjusting concurrency or timeouts.
