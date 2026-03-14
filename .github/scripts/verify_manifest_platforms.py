#!/usr/bin/env python3
from __future__ import annotations

import json
import os
import sys


def main() -> int:
    if len(sys.argv) != 2:
        raise SystemExit("usage: verify_manifest_platforms.py <tag>")

    tag = sys.argv[1]
    raw_json = os.environ.get("RAW_JSON", "")
    if not raw_json:
        raise SystemExit("RAW_JSON is required")

    manifest = json.loads(raw_json)
    required = {part.strip() for part in os.environ.get("REQUIRED_PLATFORMS", "").split(",") if part.strip()}
    platforms = {
        f"{platform.get('os')}/{platform.get('architecture')}"
        for platform in ((item.get("platform") or {}) for item in manifest.get("manifests", []))
        if platform.get("os") and platform.get("architecture")
    }
    missing = sorted(required - platforms)
    if missing:
        raise SystemExit(
            f"missing required platforms for {tag}: {', '.join(missing)} (detected: {sorted(platforms)})"
        )

    print(f"[verify] {tag}: detected {sorted(platforms)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
