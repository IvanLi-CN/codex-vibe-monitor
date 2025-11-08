#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

LOG_DIR="$ROOT_DIR/logs"
PID_FILE="$LOG_DIR/web.pid"
PORT=60080

mkdir -p "$LOG_DIR"

if lsof -ti tcp:$PORT -sTCP:LISTEN >/dev/null 2>&1; then
  echo "[frontend] port $PORT already in use; refusing to auto-restart" >&2
  exit 1
fi

nohup bash -lc 'cd web && npm run dev -- --host 127.0.0.1 --port 60080 --strictPort true' </dev/null >> "$LOG_DIR/web.dev.log" 2>&1 &
echo $! > "$PID_FILE"
echo "[frontend] started pid $(cat "$PID_FILE")"
