#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

DEVCTL="${HOME}/.codex/bin/devctl"
SERVICE="backend"

if [[ ! -x "$DEVCTL" ]]; then
  echo "[backend] devctl not found or not executable: $DEVCTL" >&2
  exit 1
fi

"$DEVCTL" --root "$ROOT_DIR" down "$SERVICE"
