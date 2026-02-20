#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

DEVCTL="${HOME}/.codex/bin/devctl"
SERVICE="frontend"
PORT=60080

if [[ ! -x "$DEVCTL" ]]; then
  echo "[frontend] devctl not found or not executable: $DEVCTL" >&2
  echo "[frontend] this repo uses devctl+zellij for long-lived dev services (no fallback)" >&2
  exit 1
fi

if ! command -v zellij >/dev/null 2>&1; then
  echo "[frontend] zellij not found in PATH" >&2
  exit 1
fi

if "$DEVCTL" --root "$ROOT_DIR" status "$SERVICE" >/dev/null 2>&1; then
  echo "[frontend] already running (devctl session exists)"
  exit 0
fi

if lsof -ti tcp:$PORT -sTCP:LISTEN >/dev/null 2>&1; then
  echo "[frontend] port $PORT already in use; stop the existing process first" >&2
  exit 1
fi

"$DEVCTL" --root "$ROOT_DIR" up "$SERVICE" -- bash -lc 'cd web && npm run dev -- --host 127.0.0.1 --port 60080 --strictPort true'
echo "[frontend] started via devctl (logs: $ROOT_DIR/.codex/logs/$SERVICE.log)"
