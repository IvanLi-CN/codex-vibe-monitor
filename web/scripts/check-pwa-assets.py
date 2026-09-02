#!/usr/bin/env python3
"""Verify the generated install-icon contract without third-party Python packages."""

from __future__ import annotations

import hashlib
from html.parser import HTMLParser
import json
import math
import re
import struct
import sys
import zlib
from pathlib import Path


WEB_DIR = Path(__file__).resolve().parents[1]
PUBLIC_DIR = WEB_DIR / "public"
DIST_DIR = WEB_DIR / "dist"
BACKGROUND = (0xFB, 0xFD, 0xFF)
INSTALL_ICON_SPECS = {
    "favicon": ("favicon", ".svg"),
    "icon_192": ("icon-192", ".png"),
    "icon_512": ("icon-512", ".png"),
    "maskable_192": ("maskable-192", ".png"),
    "maskable_512": ("maskable-512", ".png"),
}
LEGACY_INSTALL_ICON_NAMES = {
    "favicon.svg",
    "icon-192.png",
    "icon-512.png",
    "maskable-192.png",
    "maskable-512.png",
}


class LinkTagParser(HTMLParser):
    def __init__(self) -> None:
        super().__init__(convert_charrefs=True)
        self.links: list[dict[str, str]] = []

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        if tag.lower() == "link":
            self.links.append({key: value or "" for key, value in attrs})


def parse_link_tags(html: str) -> list[dict[str, str]]:
    parser = LinkTagParser()
    parser.feed(html)
    parser.close()
    return parser.links


def link_has_rel(link: dict[str, str], rel: str) -> bool:
    return rel in link.get("rel", "").lower().split()


def href_filename(link: dict[str, str]) -> str:
    return link.get("href", "").split("?", 1)[0].rstrip("/").rsplit("/", 1)[-1]


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


def find_install_icon_assets() -> dict[str, Path]:
    assets = {}
    for key, (prefix, extension) in INSTALL_ICON_SPECS.items():
        pattern = re.compile(rf"{re.escape(prefix)}-[0-9a-f]{{12}}{re.escape(extension)}")
        candidates = sorted(
            path for path in PUBLIC_DIR.iterdir() if path.is_file() and pattern.fullmatch(path.name)
        )
        assert len(candidates) == 1, (
            f"expected one content-hashed {prefix}{extension} asset, found "
            f"{', '.join(path.name for path in candidates)}"
        )
        assets[key] = candidates[0]
    for legacy_name in LEGACY_INSTALL_ICON_NAMES:
        assert not (PUBLIC_DIR / legacy_name).exists(), f"legacy stable icon remains: {legacy_name}"
    return assets


def assert_content_hash(path: Path) -> None:
    match = re.search(r"-([0-9a-f]{12})\.[^.]+$", path.name)
    assert match, f"{path}: filename is not content-hashed"
    actual = hashlib.sha256(path.read_bytes()).hexdigest()[:12]
    assert match.group(1) == actual, f"{path}: filename hash does not match its bytes"


def assert_build_contract(assets: dict[str, Path]) -> None:
    assert DIST_DIR.is_dir(), "build output is missing; run the PWA build before this checker"
    manifest = json.loads((DIST_DIR / "site.webmanifest").read_text())
    assert manifest["id"] == "./", "built manifest identity changed"
    assert manifest["scope"] == "./", "built manifest scope changed"
    assert manifest["start_url"] == "./#/dashboard", "built manifest start_url changed"
    expected_icons = {path.name for path in assets.values()}
    actual_icons = {icon["src"] for icon in manifest["icons"]}
    assert actual_icons == expected_icons, "built manifest icon URLs do not match the generated assets"
    expected_purposes = {
        assets["icon_192"].name: "any",
        assets["icon_512"].name: "any",
        assets["favicon"].name: "any",
        assets["maskable_192"].name: "maskable",
        assets["maskable_512"].name: "maskable",
    }
    assert {icon["src"]: icon["purpose"] for icon in manifest["icons"]} == expected_purposes, (
        "built manifest icon purposes do not match the asset roles"
    )
    for icon in manifest["icons"]:
        assert "?" not in icon["src"], "built manifest icon URL still uses a query-string version"
        assert icon["src"] in expected_icons, "built manifest references an unknown icon"
    assert {shortcut["icons"][0]["src"] for shortcut in manifest["shortcuts"]} == {
        assets["icon_192"].name
    }, "built shortcut icon URL is not content-hashed"

    built_html = (DIST_DIR / "index.html").read_text()
    links = parse_link_tags(built_html)
    manifest_links = [link for link in links if link_has_rel(link, "manifest")]
    assert len(manifest_links) == 1, "built HTML must expose exactly one manifest link"
    assert href_filename(manifest_links[0]) == "site.webmanifest", (
        "built HTML manifest link does not point to site.webmanifest"
    )
    favicon_links = [link for link in links if link_has_rel(link, "icon")]
    assert len(favicon_links) == 1, "built HTML must expose exactly one favicon link"
    assert href_filename(favicon_links[0]) == assets["favicon"].name, "built favicon URL is stale"
    assert not any(link_has_rel(link, "apple-touch-icon") for link in links), (
        "built HTML retains the Apple touch icon link"
    )
    worker = (DIST_DIR / "sw.js").read_text()
    assert not re.search(r'"url":"[^"]*site\.webmanifest', worker), (
        "service worker precaches the manifest"
    )
    assert not re.search(r'"url":"[^"]*version\.json', worker), (
        "service worker precaches version metadata"
    )
    for path in assets.values():
        assert path.name not in worker, f"service worker precaches install icon {path.name}"
    assert "CacheFirst" not in worker, "service worker cache-first routes can pin install metadata"


def main() -> None:
    assets = find_install_icon_assets()
    for path in assets.values():
        assert_content_hash(path)
    assert_regular(assets["icon_192"], 192)
    assert_regular(assets["icon_512"], 512)
    assert_maskable(assets["maskable_192"], 192)
    assert_maskable(assets["maskable_512"], 512)
    regular_hash = hashlib.sha256(assets["icon_512"].read_bytes()).digest()
    maskable_hash = hashlib.sha256(assets["maskable_512"].read_bytes()).digest()
    assert regular_hash != maskable_hash, "regular and maskable icons share bytes"

    regular_svg = (WEB_DIR.parent / "docs" / "readme-assets" / "brand" / "codex-vibe-monitor-app-icon.svg").read_text()
    assert assets["favicon"].read_text() == regular_svg, (
        "favicon artwork no longer matches the approved regular icon source"
    )
    assert "feDropShadow" not in regular_svg and "rx=" not in regular_svg, "regular source contains platform chrome"

    config = (WEB_DIR / "vite.config.ts").read_text()
    assert 'purpose: "any"' in config and 'purpose: "maskable"' in config, "manifest purposes are incomplete"
    assert (
        "findInstallIconAsset" in config
        and "isPwaInstallIconEntry" in config
        and "installIconFiles" not in config
        and "apple-touch-icon" not in config
        and "includeManifestIcons: false" in config
        and "globIgnores" in config
    ), "Vite does not resolve only the content-hashed install assets"
    assert 'id: "./"' in config and 'scope: "./"' in config and 'start_url: "./#/dashboard"' in config, (
        "manifest identity contract is incomplete"
    )
    assert "versionedIcon" not in config and "?v=" not in config, "Vite still uses query-versioned icons"
    service_worker = (WEB_DIR / "src" / "pwa" / "sw.ts").read_text()
    assert "site.webmanifest" in service_worker and "NetworkOnly" in service_worker, (
        "service worker does not revalidate the manifest"
    )
    assert "isInstallIconPath" in service_worker, "service worker does not classify install icons"
    assert "ignoreURLParametersMatching" not in service_worker, "service worker still matches query-versioned icons"
    index = (WEB_DIR / "index.html").read_text()
    assert "%INSTALL_FAVICON%" in index, "HTML favicon link is not content-hashed"
    assert_build_contract(assets)


if __name__ == "__main__":
    try:
        main()
    except (AssertionError, ValueError) as error:
        print(f"PWA asset contract failed: {error}", file=sys.stderr)
        raise SystemExit(1) from error
    print("PWA asset contract passed.")
