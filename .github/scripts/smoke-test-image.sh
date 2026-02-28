#!/usr/bin/env bash
set -euo pipefail

tag="${1:-}"
if [[ -z "$tag" ]]; then
  echo "usage: $0 <image-tag>" >&2
  exit 2
fi

host="${SMOKE_HOST:-127.0.0.1}"
port="${SMOKE_PORT:-18080}"
timeout_secs="${SMOKE_TIMEOUT_SECS:-60}"
name="${SMOKE_CONTAINER_NAME:-smoke-codex-vibe-monitor}"

docker rm -f "$name" >/dev/null 2>&1 || true

echo "[smoke] starting container: ${tag}"
docker run -d --name "$name" -p "${host}:${port}:8080" "$tag" >/dev/null

deadline=$((SECONDS + timeout_secs))
while (( SECONDS < deadline )); do
  if curl -fsS "http://${host}:${port}/health" | grep -q "ok"; then
    echo "[smoke] /health ok"
    docker rm -f "$name" >/dev/null
    exit 0
  fi
  sleep 1
done

echo "[smoke] timed out waiting for /health (timeout=${timeout_secs}s)" >&2
docker ps -a >&2 || true
docker logs "$name" >&2 || true
docker rm -f "$name" >/dev/null 2>&1 || true
exit 1

