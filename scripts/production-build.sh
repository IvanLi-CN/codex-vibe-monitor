#!/bin/sh
set -eu

mode=${1:-}

case "$mode" in
  rust)
    : "${APP_EFFECTIVE_VERSION:?APP_EFFECTIVE_VERSION is required}"
    : "${APP_GIT_REVISION:?APP_GIT_REVISION is required}"
    export APP_EFFECTIVE_VERSION APP_GIT_REVISION
    if [ "${PRODUCTION_BUILD_FORCE_REBUILD:-0}" = "1" ]; then
      find src -type f -name '*.rs' -exec touch {} +
      rm -f "${CARGO_TARGET_DIR:-target}/release/codex-vibe-monitor"
    fi
    cargo build --release --locked
    ;;
  web)
    : "${VITE_APP_VERSION:?VITE_APP_VERSION is required}"
    export VITE_APP_VERSION
    rm -rf node_modules
    bun install --frozen-lockfile
    bun run build
    ;;
  *)
    printf '%s\n' "usage: production-build.sh rust|web" >&2
    exit 2
    ;;
esac
