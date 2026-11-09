"""Integrity-checked snapshots of released-image fixtures, never candidate results."""
import hashlib
import json
from pathlib import Path
import shutil
import stat

ROOT = Path(__file__).resolve().parents[1]
INPUTS = (
    "scripts/generate-retention-maintenance-release-fixtures.py",
    "scripts/retention_fixture_cache.py",
    "scripts/validate-retention-maintenance-upgrade.py",
    "docs/adr/assets/retention-maintenance-task-ownership/released-state-sources.json",
)
MANIFEST = "fixture-cache.json"
MAX_BYTES = 512 * 1024 * 1024
MAX_ENTRIES = 4096


def input_fingerprint(root=ROOT):
    digest = hashlib.sha256()
    for name in INPUTS:
        digest.update(name.encode() + b"\0" + hashlib.sha256((root / name).read_bytes()).digest())
    return digest.hexdigest()


def inventory(root):
    if root.is_symlink() or not root.is_dir():
        raise ValueError("Fixture snapshot must be a real directory")
    entries = {}
    total = 0
    for path in sorted(root.rglob("*")):
        relative = path.relative_to(root).as_posix()
        if relative == MANIFEST:
            continue
        mode = path.lstat().st_mode
        if stat.S_ISDIR(mode):
            entries[relative] = {"kind": "directory"}
        elif stat.S_ISREG(mode):
            size = path.stat().st_size
            total += size
            if total > MAX_BYTES:
                raise ValueError("Fixture snapshot exceeds size limit")
            with path.open("rb") as stream:
                digest = hashlib.file_digest(stream, "sha256").hexdigest()
            entries[relative] = {"kind": "file", "size": size, "sha256": digest}
        else:
            raise ValueError("Fixture snapshot contains a link or special file")
        if len(entries) > MAX_ENTRIES:
            raise ValueError("Fixture snapshot exceeds entry limit")
    return entries


def validate_sources(root, sources):
    expected = set(sources) | {MANIFEST}
    if {path.name for path in root.iterdir()} - expected:
        raise ValueError("Fixture snapshot has unexpected versions or root entries")
    for version, expected_source in sources.items():
        source = json.loads((root / version / "source.json").read_text())
        for key in ("tag", "image", "digest", "generator", "exitCode"):
            if source.get(key) != expected_source[key]:
                raise ValueError("Fixture source does not match pinned release manifest")
        if source.get("imageRef") != expected_source["digest"]:
            raise ValueError("Fixture source image reference is not digest pinned")
        for name in ("business.sqlite", "maintenance.sqlite"):
            if not (root / version / name).is_file():
                raise ValueError("Fixture source database is missing")


def validate_snapshot(root, fingerprint, sources):
    # Inspect entry types before opening metadata or copying anything from cache.
    files = inventory(root)
    metadata_path = root / MANIFEST
    if metadata_path.is_symlink() or not metadata_path.is_file() or metadata_path.stat().st_size > 1024 * 1024:
        raise ValueError("Fixture snapshot metadata is missing or invalid")
    metadata = json.loads(metadata_path.read_text())
    if metadata != {"schemaVersion": 1, "inputFingerprint": fingerprint, "entries": files}:
        raise ValueError("Fixture snapshot identity or contents changed")
    validate_sources(root, sources)
    return sum(entry.get("size", 0) for entry in files.values())


def prepare_fixtures(output, cache, sources, generate):
    if output.exists() or output.is_symlink():
        raise ValueError("Cached fixture preparation requires a fresh output directory")
    if cache.is_symlink():
        raise ValueError("Fixture cache must not be a symbolic link")
    output = output.resolve()
    cache = cache.resolve()
    if output == cache or output in cache.parents or cache in output.parents:
        raise ValueError("Fixture cache and working copy must be separate directories")
    fingerprint = input_fingerprint()
    if cache.exists():
        size = validate_snapshot(cache, fingerprint, sources)
        shutil.copytree(cache, output)
        # Recheck the copy so mutations during transfer cannot silently enter tests.
        validate_snapshot(output, fingerprint, sources)
        print(f"Released fixture cache: hit; bytes={size}; input={fingerprint}", flush=True)
        return
    generate(output)
    files = inventory(output)
    validate_sources(output, sources)
    (output / MANIFEST).write_text(json.dumps({"schemaVersion": 1, "inputFingerprint": fingerprint, "entries": files}, sort_keys=True) + "\n")
    validate_snapshot(output, fingerprint, sources)
    shutil.copytree(output, cache)
    size = validate_snapshot(cache, fingerprint, sources)
    print(f"Released fixture cache: miss; bytes={size}; input={fingerprint}", flush=True)
