#!/usr/bin/env python3
"""Verify the generated install-icon contract without third-party Python packages."""

from __future__ import annotations

import hashlib
import json
import math
import struct
import sys
import zlib
from pathlib import Path


WEB_DIR = Path(__file__).resolve().parents[1]
PUBLIC_DIR = WEB_DIR / "public"
DIST_DIR = WEB_DIR / "dist"
BACKGROUND = (0xFB, 0xFD, 0xFF)
INSTALL_ICON_ASSETS = (
    "apple-touch-icon.png",
    "favicon.svg",
    "icon-192.png",
    "icon-512.png",
    "maskable-192.png",
    "maskable-512.png",
)


def read_png(path: Path) -> tuple[int, int, list[tuple[int, int, int, int]]]:
    raw = path.read_bytes()
    if raw[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError(f"{path}: not a PNG")

    cursor = 8
    width = height = color_type = None
    compressed = bytearray()
    while cursor < len(raw):
        length = struct.unpack(">I", raw[cursor : cursor + 4])[0]
        chunk_type = raw[cursor + 4 : cursor + 8]
        chunk = raw[cursor + 8 : cursor + 8 + length]
        cursor += length + 12
        if chunk_type == b"IHDR":
            width, height, bit_depth, color_type, compression, filtering, interlace = struct.unpack(
                ">IIBBBBB", chunk
            )
            if bit_depth != 8 or color_type not in (2, 6) or compression or filtering or interlace:
                raise ValueError(f"{path}: unsupported PNG encoding")
        elif chunk_type == b"IDAT":
            compressed.extend(chunk)
        elif chunk_type == b"IEND":
            break

    if width is None or height is None or color_type is None:
        raise ValueError(f"{path}: missing IHDR")
    channels = 4 if color_type == 6 else 3
    scanlines = zlib.decompress(compressed)
    stride = width * channels
    expected = (stride + 1) * height
    if len(scanlines) != expected:
        raise ValueError(f"{path}: invalid scanline length")

    rows: list[bytes] = []
    previous = bytearray(stride)
    offset = 0
    for _ in range(height):
        filter_type = scanlines[offset]
        source = scanlines[offset + 1 : offset + 1 + stride]
        offset += stride + 1
        row = bytearray(stride)
        for index, value in enumerate(source):
            left = row[index - channels] if index >= channels else 0
            up = previous[index]
            up_left = previous[index - channels] if index >= channels else 0
            if filter_type == 0:
                result = value
            elif filter_type == 1:
                result = value + left
            elif filter_type == 2:
                result = value + up
            elif filter_type == 3:
                result = value + ((left + up) // 2)
            elif filter_type == 4:
                prediction = left + up - up_left
                distances = (abs(prediction - left), abs(prediction - up), abs(prediction - up_left))
                result = value + (left if distances[0] <= distances[1] and distances[0] <= distances[2] else up if distances[1] <= distances[2] else up_left)
            else:
                raise ValueError(f"{path}: unknown PNG filter {filter_type}")
            row[index] = result & 0xFF
        rows.append(bytes(row))
        previous = row

    pixels = []
    for row in rows:
        for index in range(0, stride, channels):
            red, green, blue = row[index : index + 3]
            alpha = row[index + 3] if channels == 4 else 255
            pixels.append((red, green, blue, alpha))
    return width, height, pixels


def assert_maskable(path: Path, expected_size: int) -> None:
    width, height, pixels = read_png(path)
    assert (width, height) == (expected_size, expected_size), f"{path}: unexpected dimensions"
    assert all(alpha == 255 for _, _, _, alpha in pixels), f"{path}: maskable surface is not opaque"

    foreground = [
        (index % width, index // width)
        for index, (red, green, blue, _) in enumerate(pixels)
        if max(abs(red - BACKGROUND[0]), abs(green - BACKGROUND[1]), abs(blue - BACKGROUND[2])) >= 16
    ]
    assert foreground, f"{path}: foreground is missing"
    xs, ys = zip(*foreground, strict=True)
    ratio = max(max(xs) - min(xs) + 1, max(ys) - min(ys) + 1) / width
    assert 0.58 <= ratio <= 0.62, f"{path}: foreground ratio {ratio:.3f} is outside 58%-62%"
    center = width / 2
    radius = width * 0.4
    assert all(math.hypot(x + 0.5 - center, y + 0.5 - center) <= radius for x, y in foreground), (
        f"{path}: important foreground escapes the 40% safe circle"
    )


def assert_regular(path: Path, expected_size: int) -> None:
    width, height, pixels = read_png(path)
    assert (width, height) == (expected_size, expected_size), f"{path}: unexpected dimensions"
    assert any(alpha < 16 for _, _, _, alpha in pixels), f"{path}: regular icon lost transparency"


def install_icon_version() -> str:
    digest = hashlib.sha256()
    for asset in INSTALL_ICON_ASSETS:
        digest.update((PUBLIC_DIR / asset).read_bytes())
    return digest.hexdigest()[:12]


def assert_build_contract(version: str) -> None:
    assert DIST_DIR.is_dir(), "build output is missing; run the PWA build before this checker"
    manifest = json.loads((DIST_DIR / "site.webmanifest").read_text())
    expected_icons = {
        f"icon-192.png?v={version}",
        f"icon-512.png?v={version}",
        f"favicon.svg?v={version}",
        f"maskable-192.png?v={version}",
        f"maskable-512.png?v={version}",
    }
    actual_icons = {icon["src"] for icon in manifest["icons"]}
    assert actual_icons == expected_icons, "built manifest icon URLs do not match the generated assets"
    for shortcut in manifest["shortcuts"]:
        assert shortcut["icons"][0]["src"] == f"icon-192.png?v={version}", (
            "built shortcut icon URL is not content-versioned"
        )

    built_html = (DIST_DIR / "index.html").read_text()
    assert f"apple-touch-icon.png?v={version}" in built_html, "built Apple touch URL is stale"
    worker = (DIST_DIR / "sw.js").read_text()
    assert "ignoreURLParametersMatching" in worker, "built service worker does not match versioned icon URLs offline"
    for asset in INSTALL_ICON_ASSETS:
        assert asset in worker, f"Workbox precache omits {asset}"


def main() -> None:
    assert_regular(PUBLIC_DIR / "icon-192.png", 192)
    assert_regular(PUBLIC_DIR / "icon-512.png", 512)
    assert_maskable(PUBLIC_DIR / "maskable-192.png", 192)
    assert_maskable(PUBLIC_DIR / "maskable-512.png", 512)
    assert_maskable(PUBLIC_DIR / "apple-touch-icon.png", 180)

    regular_hash = hashlib.sha256((PUBLIC_DIR / "icon-512.png").read_bytes()).digest()
    maskable_hash = hashlib.sha256((PUBLIC_DIR / "maskable-512.png").read_bytes()).digest()
    assert regular_hash != maskable_hash, "regular and maskable icons share bytes"

    regular_svg = (WEB_DIR.parent / "docs" / "readme-assets" / "brand" / "codex-vibe-monitor-app-icon.svg").read_text()
    assert "feDropShadow" not in regular_svg and "rx=" not in regular_svg, "regular source contains platform chrome"

    config = (WEB_DIR / "vite.config.ts").read_text()
    assert 'purpose: "any"' in config and 'purpose: "maskable"' in config, "manifest purposes are incomplete"
    assert 'maskable-192.png' in config and 'maskable-512.png' in config, "maskable assets are not declared"
    assert 'versionedIcon("icon-192.png")' in config, "shortcut icon is not content-versioned"
    service_worker = (WEB_DIR / "src" / "pwa" / "sw.ts").read_text()
    assert "ignoreURLParametersMatching" in service_worker, "service worker does not match versioned icon URLs offline"
    index = (WEB_DIR / "index.html").read_text()
    assert "%INSTALL_ICON_VERSION%" in index, "HTML install links are not content-versioned"
    assert_build_contract(install_icon_version())


if __name__ == "__main__":
    try:
        main()
    except (AssertionError, ValueError) as error:
        print(f"PWA asset contract failed: {error}", file=sys.stderr)
        raise SystemExit(1) from error
    print("PWA asset contract passed.")
