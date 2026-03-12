#!/usr/bin/env bash
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
cd "$repo_root"

files=(
  "README.md"
  "AGENTS.md"
  "Dockerfile"
  "lefthook.yml"
  "package.json"
  "web/package.json"
  ".github/workflows/ci.yml"
)

patterns=(
  'npm([[:space:]]|$)'
  'npx([[:space:]]|$)'
  'actions/setup-node@'
  'package-lock\.json'
  'node[[:space:]]+[^[:space:]]'
  'FROM[[:space:]]+node:'
)

failures=()

for file in "${files[@]}"; do
  for pattern in "${patterns[@]}"; do
    if match="$(rg -n -e "$pattern" "$file" || true)"; then
      if [[ -n "$match" ]]; then
        failures+=("$match")
      fi
    fi
  done
done

if ((${#failures[@]} > 0)); then
  echo "[bun-first] forbidden operational references detected:" >&2
  printf '%s\n' "${failures[@]}" >&2
  exit 1
fi

echo "[bun-first] operational surface is Bun-first"
