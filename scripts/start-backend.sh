#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

DEVCTL="${HOME}/.codex/bin/devctl"
SERVICE="backend"
PORT=8080

if [[ ! -x "$DEVCTL" ]]; then
  echo "[backend] devctl not found or not executable: $DEVCTL" >&2
  echo "[backend] this repo uses devctl+zellij for long-lived dev services (no fallback)" >&2
  exit 1
fi

if ! command -v zellij >/dev/null 2>&1; then
  echo "[backend] zellij not found in PATH" >&2
  exit 1
fi

# If already running via devctl, do not fail on the port check (port is expected to be in use).
if "$DEVCTL" --root "$ROOT_DIR" status "$SERVICE" >/dev/null 2>&1; then
  echo "[backend] already running (devctl session exists)"
  exit 0
fi

if lsof -ti tcp:$PORT -sTCP:LISTEN >/dev/null 2>&1; then
  echo "[backend] port $PORT already in use; stop the existing process first" >&2
  exit 1
fi

"$DEVCTL" --root "$ROOT_DIR" up "$SERVICE" -- env RUST_LOG=info cargo run
echo "[backend] started via devctl (logs: $ROOT_DIR/.codex/logs/$SERVICE.log)"
