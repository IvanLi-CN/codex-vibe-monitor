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

cleanup() {
  docker rm -f -v "$name" >/dev/null 2>&1 || true
}
trap cleanup EXIT

cleanup

if ! docker image inspect "$tag" >/dev/null 2>&1; then
  echo "[smoke] image not present locally: ${tag}" >&2
  docker image ls >&2 || true
  exit 1
fi

echo "[smoke] starting container: ${tag}"
docker run -d --name "$name" --pull=never -p "${host}:${port}:8080" "$tag" >/dev/null

deadline=$((SECONDS + timeout_secs))
while (( SECONDS < deadline )); do
  status="$(docker inspect -f '{{.State.Status}}' "$name" 2>/dev/null || true)"
  if [[ "$status" == "exited" || "$status" == "dead" ]]; then
    echo "[smoke] container exited before health became ready" >&2
    docker logs "$name" >&2 || true
    exit 1
  fi

  if curl -m 1 -fsS "http://${host}:${port}/health" | grep -qx "ok"; then
    echo "[smoke] /health ok"
    exit 0
  fi
  sleep 1
done

echo "[smoke] timed out waiting for /health (timeout=${timeout_secs}s)" >&2
docker ps -a >&2 || true
docker logs "$name" >&2 || true
exit 1
