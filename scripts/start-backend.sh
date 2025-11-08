#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

LOG_DIR="$ROOT_DIR/logs"
PID_FILE="$LOG_DIR/backend.pid"
PORT=8080

mkdir -p "$LOG_DIR"

# stop existing process if requested
if lsof -ti tcp:$PORT -sTCP:LISTEN >/dev/null 2>&1; then
  echo "[backend] port $PORT already in use; refusing to auto-restart" >&2
  exit 1
fi

nohup env RUST_LOG=info cargo run </dev/null >> "$LOG_DIR/backend.dev.log" 2>&1 &
echo $! > "$PID_FILE"
echo "[backend] started pid $(cat "$PID_FILE")"
