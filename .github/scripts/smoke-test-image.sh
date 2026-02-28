#!/usr/bin/env bash
set -euo pipefail

tag="${1:-}"
if [[ -z "$tag" ]]; then
  echo "usage: $0 <image-tag>" >&2
  exit 2
fi

host="${SMOKE_HOST:-127.0.0.1}"
port="${SMOKE_PORT-}"
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

echo "[smoke] ${tag} -> codex-vibe-monitor --help"
docker run --rm --pull=never "$tag" codex-vibe-monitor --help >/dev/null

echo "[smoke] ${tag} -> xray version"
docker run --rm --pull=never "$tag" xray version >/dev/null

port_args=("-p" "${host}::8080")
host_port=""
if [[ -n "$port" ]]; then
  port_args=("-p" "${host}:${port}:8080")
  host_port="$port"
fi

echo "[smoke] starting container: ${tag}"
docker run -d --name "$name" --pull=never "${port_args[@]}" "$tag" >/dev/null

if [[ -z "$host_port" ]]; then
  host_port="$(docker inspect -f '{{ (index (index .NetworkSettings.Ports "8080/tcp") 0).HostPort }}' "$name" 2>/dev/null || true)"
  if [[ -z "$host_port" ]]; then
    echo "[smoke] failed to resolve published host port for container ${name}" >&2
    docker ps -a >&2 || true
    docker logs "$name" >&2 || true
    exit 1
  fi
fi

deadline=$((SECONDS + timeout_secs))
while (( SECONDS < deadline )); do
  status="$(docker inspect -f '{{.State.Status}}' "$name" 2>/dev/null || true)"
  if [[ "$status" == "exited" || "$status" == "dead" ]]; then
    echo "[smoke] container exited before health became ready" >&2
    docker logs "$name" >&2 || true
    exit 1
  fi

  if curl -m 1 -fsS "http://${host}:${host_port}/health" | grep -qx "ok"; then
    echo "[smoke] /health ok"
    exit 0
  fi
  sleep 1
done

echo "[smoke] timed out waiting for /health (timeout=${timeout_secs}s)" >&2
docker ps -a >&2 || true
docker logs "$name" >&2 || true
exit 1
