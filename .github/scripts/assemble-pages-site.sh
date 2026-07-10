#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: assemble-pages-site.sh <docs_dir> <storybook_dir> <demo_dir> <output_dir>

Copy the built docs site into the output root, Storybook into output_dir/storybook,
and the mock-only Web Demo into output_dir/demo.
USAGE
}

if [[ "$#" -ne 4 ]]; then
  usage >&2
  exit 1
fi

docs_dir="$1"
storybook_dir="$2"
demo_dir="$3"
output_dir="$4"

to_abs_path() {
  python3 -c 'import os, sys; print(os.path.abspath(sys.argv[1]))' "$1"
}

is_same_or_parent() {
  local base="$1"
  local candidate="$2"
  [[ "$candidate" == "$base" || "$candidate" == "$base"/* ]]
}

if [[ ! -d "$docs_dir" ]]; then
  echo "docs_dir does not exist: $docs_dir" >&2
  exit 1
fi

if [[ ! -d "$storybook_dir" ]]; then
  echo "storybook_dir does not exist: $storybook_dir" >&2
  exit 1
fi

if [[ ! -d "$demo_dir" ]]; then
  echo "demo_dir does not exist: $demo_dir" >&2
  exit 1
fi

docs_dir_abs="$(to_abs_path "$docs_dir")"
storybook_dir_abs="$(to_abs_path "$storybook_dir")"
demo_dir_abs="$(to_abs_path "$demo_dir")"
output_dir_abs="$(to_abs_path "$output_dir")"

if [[ "$output_dir_abs" == "/" ]]; then
  echo "refusing to use unsafe output_dir: $output_dir" >&2
  exit 1
fi

if is_same_or_parent "$output_dir_abs" "$docs_dir_abs"; then
  echo "refusing to let output_dir contain docs_dir: $output_dir" >&2
  exit 1
fi

if is_same_or_parent "$output_dir_abs" "$storybook_dir_abs"; then
  echo "refusing to let output_dir contain storybook_dir: $output_dir" >&2
  exit 1
fi

if is_same_or_parent "$output_dir_abs" "$demo_dir_abs"; then
  echo "refusing to let output_dir contain demo_dir: $output_dir" >&2
  exit 1
fi

rm -rf "$output_dir"
mkdir -p "$output_dir/storybook" "$output_dir/demo"

cp -R "$docs_dir"/. "$output_dir"/
cp -R "$storybook_dir"/. "$output_dir/storybook"/
cp -R "$demo_dir"/. "$output_dir/demo"/

if [[ ! -f "$output_dir/index.html" ]]; then
  echo "assembled site is missing root index.html" >&2
  exit 1
fi

if [[ ! -f "$output_dir/storybook/index.html" ]]; then
  echo "assembled site is missing storybook/index.html" >&2
  exit 1
fi

if [[ ! -f "$output_dir/storybook.html" ]]; then
  echo "assembled site is missing storybook.html" >&2
  exit 1
fi

if [[ ! -f "$output_dir/demo/index.html" ]]; then
  echo "assembled site is missing demo/index.html" >&2
  exit 1
fi

if [[ ! -f "$output_dir/demo/mockServiceWorker.js" ]]; then
  echo "assembled site is missing demo/mockServiceWorker.js" >&2
  exit 1
fi

if ! grep -q '正在跳转到 Storybook' "$output_dir/storybook.html"; then
  echo "storybook.html is missing the Storybook redirect copy" >&2
  exit 1
fi
