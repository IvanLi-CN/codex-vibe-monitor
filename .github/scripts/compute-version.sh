#!/usr/bin/env bash
set -euo pipefail

# Compute effective semver based on release intent from PR labels.
#
# Inputs (env):
#   RELEASE_BUMP: patch|minor|major
#   RELEASE_CHANNEL: stable|rc
#
# Uses:
#   GITHUB_SHA: commit SHA (required for rc, used as -rc.<sha7> suffix)
#
# Outputs (to $GITHUB_ENV when set, otherwise prints key=val lines):
#   APP_EFFECTIVE_VERSION: X.Y.Z or X.Y.Z-rc.<sha7>
#   RELEASE_TAG: v${APP_EFFECTIVE_VERSION}
#   RELEASE_PRERELEASE: true|false

root_dir=$(git rev-parse --show-toplevel)
cd "$root_dir"

# Ensure we have full history and tags when running in CI (defensive if caller forgot fetch-depth: 0)
git fetch --tags --force || true

release_bump="${RELEASE_BUMP:-}"
release_channel="${RELEASE_CHANNEL:-}"

if [[ -z "$release_bump" ]]; then
  echo "Missing RELEASE_BUMP (patch|minor|major)" >&2
  exit 1
fi
if [[ -z "$release_channel" ]]; then
  echo "Missing RELEASE_CHANNEL (stable|rc)" >&2
  exit 1
fi

# Base version: the max existing *stable* semver tag vX.Y.Z (ignore prereleases).
latest_stable_tag=$(
  git tag -l 'v*' \
    | grep -E '^v[0-9]+\.[0-9]+\.[0-9]+$' \
    | sort -V \
    | tail -n 1
)

if [[ -z "$latest_stable_tag" ]]; then
  cargo_ver=$(grep -m1 '^version\s*=\s*"' "$root_dir/Cargo.toml" | sed -E 's/.*"([0-9]+\.[0-9]+\.[0-9]+)".*/\1/')
  if [[ -z "$cargo_ver" ]]; then
    echo "Failed to detect version from Cargo.toml" >&2
    exit 1
  fi
  latest_stable_tag="v${cargo_ver}"
fi

base="${latest_stable_tag#v}"
IFS='.' read -r base_major base_minor base_patch <<< "$base"
if [[ -z "$base_major" || -z "$base_minor" || -z "$base_patch" ]]; then
  echo "Failed to parse stable tag: ${latest_stable_tag}" >&2
  exit 1
fi

major="$base_major"
minor="$base_minor"
patch="$base_patch"

case "$release_bump" in
  patch)
    patch=$((patch + 1))
    ;;
  minor)
    minor=$((minor + 1))
    patch=0
    ;;
  major)
    major=$((major + 1))
    minor=0
    patch=0
    ;;
  *)
    echo "Unknown RELEASE_BUMP: ${release_bump} (expected patch|minor|major)" >&2
    exit 1
    ;;
esac

next_stable="${major}.${minor}.${patch}"

prerelease="false"
effective="$next_stable"
case "$release_channel" in
  stable)
    prerelease="false"
    effective="$next_stable"
    ;;
  rc)
    sha="${GITHUB_SHA:-}"
    sha7="$(printf '%s' "$sha" | cut -c1-7)"
    if [[ -z "$sha7" ]]; then
      echo "Missing GITHUB_SHA (required for rc version suffix)" >&2
      exit 1
    fi
    prerelease="true"
    effective="${next_stable}-rc.${sha7}"
    ;;
  *)
    echo "Unknown RELEASE_CHANNEL: ${release_channel} (expected stable|rc)" >&2
    exit 1
    ;;
esac

release_tag="v${effective}"

if [[ -n "${GITHUB_ENV:-}" ]]; then
  {
    echo "APP_EFFECTIVE_VERSION=${effective}"
    echo "RELEASE_TAG=${release_tag}"
    echo "RELEASE_PRERELEASE=${prerelease}"
    echo "RELEASE_BUMP=${release_bump}"
    echo "RELEASE_CHANNEL=${release_channel}"
  } >> "$GITHUB_ENV"
else
  echo "APP_EFFECTIVE_VERSION=${effective}"
  echo "RELEASE_TAG=${release_tag}"
  echo "RELEASE_PRERELEASE=${prerelease}"
  echo "RELEASE_BUMP=${release_bump}"
  echo "RELEASE_CHANNEL=${release_channel}"
fi

echo "Computed APP_EFFECTIVE_VERSION=${effective} (base ${latest_stable_tag}, bump ${release_bump}, channel ${release_channel})"
