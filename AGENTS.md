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

- Always start local dev servers via the wrapper scripts under `scripts/`:
  - `./scripts/start-backend.sh` for the Rust API (port 8080).
  - `./scripts/start-frontend.sh` for the Vite SPA (port 60080).
- Do **not** launch these services by hand with ad-hoc `nohup`/`cargo run`/`npm run dev` commands; the wrappers enforce PID tracking and strict port usage. If a port is in use the script exits and you must resolve the conflict manually instead of allowing automatic port changes.

- NEVER wrap long‑running dev services (backend, Vite) with `alarm` or any hard kill timeout.
- Run services in background with `nohup` + PID files; detach stdin to prevent TTY hangs.
- Use bounded readiness probes (curl loops with a hard max wait) instead of waiting on processes.
- If readiness fails, fail fast and print the last log lines; do not keep waiting.

Service management (recommended patterns)

- Backend (Rust, port `8080`):
  - Start (detached):
    - `mkdir -p logs && nohup env RUST_LOG=info cargo run </dev/null >> logs/backend.dev.log 2>&1 & echo $! > logs/backend.pid`
  - Readiness (max 60s):
    - `SECS=0; until curl -sS -m 1 http://127.0.0.1:8080/health | grep -q ok; do sleep 1; SECS=$((SECS+1)); if [ $SECS -ge 60 ]; then echo 'backend not ready'; tail -n 200 logs/backend.dev.log; exit 1; fi; done`
  - Stop:
    - `kill $(lsof -ti tcp:8080 -sTCP:LISTEN) 2>/dev/null || kill $(cat logs/backend.pid) 2>/dev/null || true`

- Front‑end (Vite, port `60080`):
  - Start (detached):
    - `mkdir -p logs && nohup bash -lc 'cd web && npm run dev -- --host 127.0.0.1 --port 60080' </dev/null >> logs/web.dev.log 2>&1 & echo $! > logs/web.pid`
  - Readiness (max 90s):
    - `SECS=0; until curl -sS -m 1 http://127.0.0.1:60080/ >/dev/null; do sleep 1; SECS=$((SECS+1)); if [ $SECS -ge 90 ]; then echo 'frontend not ready'; tail -n 200 logs/web.dev.log; exit 1; fi; done`
  - Stop:
    - `kill $(lsof -ti tcp:60080 -sTCP:LISTEN) 2>/dev/null || kill $(cat logs/web.pid) 2>/dev/null || true`

- Avoid overlapping instances:
  - Prefer kill‑by‑port before restart (see Stop commands above).
  - Always write `logs/*.pid`; do not commit log/PID files.

Operational notes

- `alarm` must NOT be used for any background service. It is permitted only for ad‑hoc commands that could hang (e.g., migrations) and must never be combined with `nohup`.
- Always run readiness checks with finite timeouts and exit non‑zero on failure to avoid indefinite blocking.
- Keep services’ stdout/err in `logs/*.log` and rely on `tail -n` on failures instead of `tail -f` to avoid blocking the shell.
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

## Security & Configuration Tips

- Store authentication cookies and secrets in `.env.local`; the file is ignored—never commit credentials.
- SQLite files default to `codex_vibe_monitor.db` in the repo root; add alternate paths via `XY_DATABASE_PATH` if running in containers.
- SSE and HTTP clients depend on stable polling; monitor logs (`RUST_LOG=info cargo run`) when adjusting concurrency or timeouts.
