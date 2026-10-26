#!/usr/bin/env python3
"""Extract a tar archive without allowing path traversal or archive links."""

from __future__ import annotations

import argparse
import os
import shutil
import tarfile
from pathlib import Path, PurePosixPath


class ArchiveError(ValueError):
    """Raised when an archive cannot be safely extracted."""


def _parts(name: str) -> tuple[str, ...]:
    if not name or "\x00" in name:
        raise ArchiveError("archive member has an invalid name")
    path = PurePosixPath(name)
    if path.is_absolute() or name.startswith("/"):
        raise ArchiveError(f"archive member is absolute: {name!r}")
    parts = tuple(part for part in path.parts if part not in ("", "."))
    if ".." in parts:
        raise ArchiveError(f"archive member escapes destination: {name!r}")
    return parts


def _target(root: Path, name: str) -> tuple[Path, tuple[str, ...]]:
    parts = _parts(name)
    target = root.joinpath(*parts)
    try:
        target.resolve(strict=False).relative_to(root)
    except ValueError as error:
        raise ArchiveError(f"archive member escapes destination: {name!r}") from error
    return target, parts


def _ensure_parent(root: Path, parts: tuple[str, ...]) -> Path:
    current = root
    for part in parts:
        current /= part
        if current.is_symlink():
            raise ArchiveError(f"archive extraction encountered a symlink parent: {current}")
        if current.exists() and not current.is_dir():
            raise ArchiveError(f"archive extraction encountered a non-directory parent: {current}")
        current.mkdir(exist_ok=True)
    return current


def _reject_symlink_components(path: Path) -> Path:
    """Reject a symlink destination before resolving its real path."""
    absolute = Path(os.path.abspath(path))
    if absolute.is_symlink():
        raise ArchiveError(f"destination path contains a symlink: {absolute}")
    return absolute


def extract(archive_path: Path, destination: Path) -> None:
    destination = _reject_symlink_components(destination)
    destination = destination.resolve(strict=False)
    if destination.exists() and not destination.is_dir():
        raise ArchiveError(f"destination is not a real directory: {destination}")
    destination.mkdir(parents=True, exist_ok=True)
    _reject_symlink_components(destination)

    with tarfile.open(archive_path, mode="r:*") as archive:
        entries: list[tuple[tarfile.TarInfo, Path, tuple[str, ...]]] = []
        seen: set[tuple[str, ...]] = set()
        for member in archive.getmembers():
            target, parts = _target(destination, member.name)
            if parts in seen:
                raise ArchiveError(f"archive contains duplicate member: {member.name!r}")
            seen.add(parts)
            if member.issym() or member.islnk():
                raise ArchiveError(f"archive links are not allowed: {member.name!r}")
            if not member.isdir() and not member.isfile():
                raise ArchiveError(f"archive member type is not allowed: {member.name!r}")
            entries.append((member, target, parts))

        for member, target, parts in entries:
            if not parts:
                if not member.isdir():
                    raise ArchiveError("archive root member must be a directory")
                continue
            if member.isdir():
                _ensure_parent(destination, parts)
                continue
            _ensure_parent(destination, parts[:-1])
            if target.exists() or target.is_symlink():
                raise ArchiveError(f"archive would overwrite an existing path: {member.name!r}")
            source = archive.extractfile(member)
            if source is None:
                raise ArchiveError(f"archive file has no readable payload: {member.name!r}")
            with source, target.open("xb") as output:
                shutil.copyfileobj(source, output)
            os.chmod(target, member.mode & 0o7777)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("archive", type=Path)
    parser.add_argument("destination", type=Path)
    args = parser.parse_args()
    extract(args.archive, args.destination)


if __name__ == "__main__":
    main()
