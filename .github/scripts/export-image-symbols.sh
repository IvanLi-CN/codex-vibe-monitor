#!/usr/bin/env bash
set -euo pipefail

# Export from the smoke-verified image itself so a rebuild cannot change symbols.
image_ref="${1:?usage: export-image-symbols.sh IMAGE REVISION OUTPUT}"
revision="${2:?revision is required}"
output_dir="${3:?output directory is required}"
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
image_revision="$(docker image inspect --format '{{index .Config.Labels "org.opencontainers.image.revision"}}' "$image_ref")"
if [[ "$image_revision" != "$revision" ]]; then
  echo "symbol source image revision differs from the candidate" >&2
  exit 1
fi
temporary_dir="$(mktemp -d)"
container_id=""
cleanup() {
  if [[ -n "$container_id" ]]; then docker rm "$container_id" >/dev/null; fi
  rm -f "$temporary_dir/codex-vibe-monitor"
  rmdir "$temporary_dir"
}
trap cleanup EXIT
container_id="$(docker create --entrypoint /bin/true "$image_ref")"
docker cp "$container_id:/usr/local/bin/codex-vibe-monitor" "$temporary_dir/codex-vibe-monitor"
python3 "$repo_root/scripts/export-observability-symbols.py" \
  --binary "$temporary_dir/codex-vibe-monitor" --revision "$revision" --output "$output_dir"
docker image inspect --format '{{json .}}' "$image_ref" > "$output_dir/image-identity.json"
