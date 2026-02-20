#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

DEVCTL="${HOME}/.codex/bin/devctl"

if [[ ! -x "$DEVCTL" ]]; then
  echo "[dev] devctl not found or not executable: $DEVCTL" >&2
  exit 1
fi

set +e
"$DEVCTL" --root "$ROOT_DIR" status backend
"$DEVCTL" --root "$ROOT_DIR" status frontend
exit 0
