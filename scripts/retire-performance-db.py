#!/usr/bin/env python3
"""Offline, reentrant retirement of an identified schema-v1 performance database."""
import argparse
from contextlib import closing
import datetime as dt
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import sqlite3
import subprocess

TABLES = {
    "performance_meta": [("key", "TEXT", 0, 1), ("value", "TEXT", 1, 0)],
    "performance_epochs": [("epoch", "TEXT", 0, 1), ("started_at", "TEXT", 1, 0), ("ended_at", "TEXT", 0, 0)],
    "performance_buckets": [
        ("bucket_start", "INTEGER", 1, 1), ("resolution_seconds", "INTEGER", 1, 2),
        ("metric_id", "TEXT", 1, 3), ("dimension_code", "TEXT", 1, 4),
        ("sample_count", "INTEGER", 1, 0), ("expected_count", "INTEGER", 1, 0),
        ("sum_value", "REAL", 1, 0), ("min_value", "REAL", 0, 0),
        ("max_value", "REAL", 0, 0), ("last_value", "REAL", 0, 0),
        ("weighted_sum", "REAL", 1, 0), ("weighted_seconds", "REAL", 1, 0),
        ("histogram_json", "TEXT", 1, 0), ("epoch", "TEXT", 1, 0),
    ],
    "performance_collector_health": [
        ("id", "INTEGER", 0, 1), ("state", "TEXT", 1, 0),
        ("last_successful_flush", "TEXT", 0, 0), ("dropped_samples", "INTEGER", 1, 0),
        ("flush_failure_count", "INTEGER", 1, 0), ("last_error", "TEXT", 0, 0),
    ],
}
ALLOWED = set(TABLES) | {"idx_performance_buckets_range"}

def utc_now():
    return dt.datetime.now(dt.timezone.utc)

def family_identity(target):
    family = []
    for suffix in ["", "-wal", "-shm"]:
        member = Path(str(target) + suffix)
        if member.is_symlink(): raise ValueError("sidecar symlink is not owned by this migration")
        if member.exists():
            family.append({"suffix": suffix, "identity": identity(member), "sha256": digest(member)})
    return family

def failed_manifest(path, manifest, target, failure_class):
    manifest.update(state="failed", failedAt=utc_now().isoformat(), failureClass=failure_class,
                    integrityCheck={"status": "failed"})
    try:
        manifest["family"] = family_identity(target)
    except (OSError, ValueError) as error:
        manifest.update(family=[], familyInspectionFailure=type(error).__name__)
    write_json(path, manifest)

def digest(path):
    hasher = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            hasher.update(chunk)
    return hasher.hexdigest()

def identity(path):
    stat = path.stat()
    return {"device": stat.st_dev, "inode": stat.st_ino, "size": stat.st_size}

def write_json(path, value):
    temporary = path.with_suffix(path.suffix + ".pending")
    with temporary.open("w", encoding="utf-8") as stream:
        json.dump(value, stream, indent=2); stream.write("\n"); stream.flush(); os.fsync(stream.fileno())
    os.replace(temporary, path)
    descriptor = os.open(path.parent, os.O_RDONLY)
    try: os.fsync(descriptor)
    finally: os.close(descriptor)

def stopped(container):
    if not re.fullmatch(r"[a-zA-Z0-9][a-zA-Z0-9_.-]{0,127}", container):
        raise ValueError("invalid container identity")
    state = json.loads(subprocess.check_output(["docker", "inspect", "--format", "{{json .State}}", container], timeout=10))
    if state.get("Running") or state.get("Restarting") or state.get("Pid", 0):
        raise ValueError("old application and its writer must be stopped and drained first")

def compatible_image(image, container=None):
    if not re.fullmatch(r"[^\s]+@sha256:[a-f0-9]{64}", image):
        raise ValueError("previous image must be pinned by digest")
    metadata = json.loads(subprocess.check_output(["docker", "image", "inspect", image], timeout=10))[0]
    version = metadata.get("Config", {}).get("Labels", {}).get("org.opencontainers.image.version", "")
    # This migration is the v2 -> v3 cutover contract, not a generic major-skip tool.
    if not re.fullmatch(r"v?2\.[0-9]+\.[0-9]+(?:-[A-Za-z0-9.-]+)?(?:\+[A-Za-z0-9.-]+)?", version):
        raise ValueError("only the immediately preceding v2 image is supported; direct major skips are refused")
    if container:
        actual = subprocess.check_output(["docker", "inspect", "--format", "{{.Image}}", container], text=True, timeout=10).strip()
        if actual != metadata["Id"]:
            raise ValueError("stopped writer container does not match the pinned old image")
    return version

def outside_application_mounts(archive_root, container):
    archive_root = Path(archive_root).resolve(strict=False)
    mounts = json.loads(subprocess.check_output(["docker", "inspect", "--format", "{{json .Mounts}}", container], text=True, timeout=10))
    for mount in mounts:
        if mount.get("Type") not in ("bind", "volume"):
            continue
        source = Path(mount.get("Source", ""))
        if not source.is_absolute():
            raise ValueError("application mount identity is unavailable")
        source = source.resolve(strict=True)
        if archive_root == source or archive_root.is_relative_to(source):
            raise ValueError("archive must be outside every application mount")

def schema(connection):
    if connection.execute("PRAGMA user_version").fetchone()[0] != 1:
        raise ValueError("unknown performance schema marker")
    names = {row[0] for row in connection.execute("SELECT name FROM sqlite_master WHERE name NOT LIKE 'sqlite_%'")}
    if names != ALLOWED:
        raise ValueError("unknown or partial schema; source is untouched")
    for table, columns in TABLES.items():
        actual = [(row[1], row[2].upper(), row[3], row[5]) for row in connection.execute(f'PRAGMA table_info("{table}")')]
        if actual != columns:
            raise ValueError("unexpected performance table contract")
    index = [(row[0], row[2]) for row in connection.execute('PRAGMA index_info("idx_performance_buckets_range")')]
    if index != [(0, "resolution_seconds"), (1, "bucket_start")]:
        raise ValueError("unexpected performance range index")
    health_sql = connection.execute("SELECT sql FROM sqlite_master WHERE type='table' AND name='performance_collector_health'").fetchone()[0]
    if "CHECK(ID=1)" not in re.sub(r"\s", "", health_sql).upper():
        raise ValueError("unexpected performance health constraint")
    if connection.execute("SELECT value FROM performance_meta WHERE key='schema_version'").fetchone() != ("1",):
        raise ValueError("missing final performance schema marker")

def valid_backup(path, expected):
    if path.is_symlink() or not path.is_file() or digest(path) != expected:
        raise ValueError("archive hash mismatch")
    with closing(sqlite3.connect(path.as_uri() + "?mode=ro", uri=True)) as connection:
        schema(connection)
        if connection.execute("PRAGMA integrity_check").fetchone() != ("ok",):
            raise ValueError("archive integrity check failed")

def archive(source, business_db, data_root, archive_root, operation, previous_image, previous_config, previous_version):
    source = Path(source).absolute()
    target = source.resolve(strict=False)
    business = Path(business_db).resolve(strict=False)
    data_root = Path(data_root).resolve(strict=True)
    archive_root = Path(archive_root).resolve(strict=False)
    if not re.fullmatch(r"[^\s]+@sha256:[a-f0-9]{64}", previous_image):
        raise ValueError("previous image must be pinned by digest")
    if not re.fullmatch(r"v?2\.[0-9]+\.[0-9]+(?:-[A-Za-z0-9.-]+)?(?:\+[A-Za-z0-9.-]+)?", previous_version):
        raise ValueError("previous program version must be the verified v2 version")
    if target == business or (target.exists() and business.exists() and os.path.samefile(target, business)):
        raise ValueError("refusing business database identity")
    if archive_root == data_root or archive_root.is_relative_to(data_root):
        raise ValueError("archive must be outside the application's data mount")
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_-]{0,63}", operation):
        raise ValueError("invalid operation id")
    run = archive_root / operation
    if run.is_symlink(): raise ValueError("archive operation is a symlink")
    run.mkdir(parents=True, exist_ok=True, mode=0o700)
    manifest_path = run / "manifest.json"
    if manifest_path.exists():
        manifest = json.loads(manifest_path.read_text())
        if manifest["source"] != str(source) or manifest["businessDb"] != str(business):
            raise ValueError("operation belongs to different file identities")
        if manifest["state"] != "absent" and (manifest["previousImage"] != previous_image or manifest["previousProgramVersion"] != previous_version or manifest["previousConfigSha256"] != digest(Path(previous_config))):
            raise ValueError("operation belongs to different previous program/configuration")
        recorded_target = Path(manifest["resolvedSource"])
        if source.exists() and target != recorded_target: raise ValueError("source alias changed")
        target = recorded_target
        if manifest["state"] == "failed":
            raise ValueError("recorded retirement failure: " + manifest["failureClass"])
        if manifest["state"] == "absent":
            if target.exists(): raise ValueError("previously absent source now exists")
            return manifest
        if manifest["state"] == "archived" and target.exists(): raise ValueError("retired source path is occupied")
    else:
        if not target.exists():
            manifest = {"version": 1, "state": "absent", "source": str(source), "resolvedSource": str(target), "businessDb": str(business), "operation": operation}
            write_json(manifest_path, manifest); return manifest
        if not target.is_file(): raise ValueError("source is not a regular file")
        manifest = {"version": 1, "state": "identified", "source": str(source), "resolvedSource": str(target), "sourceParent": str(source.parent.resolve(strict=True)), "sourceIsSymlink": source.is_symlink(), "businessDb": str(business), "identity": identity(target), "operation": operation, "previousImage": previous_image, "previousProgramVersion": previous_version, "previousConfigSha256": digest(Path(previous_config)), "createdAt": utc_now().isoformat(), "retainDays": 90, "sourceOwnership": "unverified"}
        # Identity is proven before any source is moved. A corrupt/unknown DB is preserved in place.
        try:
            with closing(sqlite3.connect(target.as_uri() + "?mode=ro", uri=True)) as connection:
                schema(connection)
                manifest["schema"] = {"userVersion": 1, "metaVersion": "1", "objects": [list(row) for row in connection.execute("SELECT name, sql FROM sqlite_master WHERE name NOT LIKE 'sqlite_%' ORDER BY name")]}
        except (sqlite3.DatabaseError, ValueError) as error:
            failed_manifest(manifest_path, manifest, target, "sqlite_integrity" if isinstance(error, sqlite3.DatabaseError) else "unrecognized_schema")
            raise
        manifest["sourceOwnership"] = "performance_schema_v1"
        write_json(manifest_path, manifest)
    backup = run / "performance.sqlite"
    if manifest["state"] == "identified":
        if identity(target) != manifest["identity"]: raise ValueError("source identity changed")
        temporary = run / "performance.sqlite.pending"
        if temporary.exists(): temporary.unlink()
        with closing(sqlite3.connect(target.as_uri() + "?mode=ro", uri=True)) as original, closing(sqlite3.connect(temporary)) as destination:
            schema(original); original.backup(destination)
            destination.execute("PRAGMA journal_mode=DELETE")
        checksum = digest(temporary)
        try:
            valid_backup(temporary, checksum)
        except (sqlite3.DatabaseError, ValueError):
            failed_manifest(manifest_path, manifest, target, "backup_integrity")
            raise
        os.replace(temporary, backup)
        manifest.update(state="verified", backupSha256=checksum, verifiedAt=utc_now().isoformat(), integrityCheck={"status": "passed", "result": "ok"})
        # Preserve exact WAL/SHM bytes for incident recovery, separately from the consistent backup.
        family = []
        for suffix in ["", "-wal", "-shm"]:
            member = Path(str(target) + suffix)
            if member.is_symlink(): raise ValueError("sidecar symlink is not owned by this migration")
            if member.exists():
                record = {"suffix": suffix, "identity": identity(member), "sha256": digest(member)}
                destination = run / ("source" + suffix)
                shutil.copyfile(member, destination); os.chmod(destination, 0o600)
                if digest(destination) != record["sha256"]: raise ValueError("copy verification failed")
                family.append(record)
        manifest["family"] = family; write_json(manifest_path, manifest)
    valid_backup(backup, manifest["backupSha256"])
    if manifest["state"] == "verified":
        # Stage is immutable after verification. Reentry removes only exact, unchanged owned members.
        for member in manifest["family"]:
            path = Path(str(target) + member["suffix"])
            if path.exists():
                if path.is_symlink() or identity(path) != member["identity"] or digest(path) != member["sha256"]:
                    raise ValueError("source family changed; preserving remaining files")
        for member in manifest["family"]:
            path = Path(str(target) + member["suffix"])
            if path.exists(): path.unlink()
        if source.is_symlink():
            if source.resolve(strict=False) != target: raise ValueError("source alias changed")
            source.unlink()
        archived_at = utc_now()
        manifest.update(state="archived", archivedAt=archived_at.isoformat(), cutoverAt=archived_at.isoformat(), cutoverScope="performance_file_family", retainUntil=(archived_at + dt.timedelta(days=90)).isoformat())
        write_json(manifest_path, manifest)
    return manifest

def restore(manifest_path, previous_image, previous_config):
    manifest_path = Path(manifest_path).resolve(strict=True)
    manifest = json.loads(manifest_path.read_text())
    if manifest["state"] != "archived": raise ValueError("only a completed archive can be restored")
    if previous_image != manifest["previousImage"] or digest(Path(previous_config)) != manifest["previousConfigSha256"]:
        raise ValueError("restore requires the recorded compatible old image and configuration")
    backup = manifest_path.parent / "performance.sqlite"; valid_backup(backup, manifest["backupSha256"])
    target = Path(manifest["resolvedSource"])
    source = Path(manifest["source"])
    if not target.is_absolute() or target.parent.resolve(strict=True) != target.parent or source.parent.resolve(strict=True) != Path(manifest["sourceParent"]):
        raise ValueError("restore parent identity changed")
    if target == Path(manifest["businessDb"]):
        raise ValueError("refusing business database identity")
    leaf_alias = manifest["sourceIsSymlink"]
    if leaf_alias and (source.exists() or source.is_symlink()):
        if not source.is_symlink() or source.resolve(strict=False) != target:
            raise ValueError("restore alias occupied")
    if target.exists() and not target.is_symlink() and not any(Path(str(target) + suffix).exists() for suffix in ["-wal", "-shm"]):
        valid_backup(target, manifest["backupSha256"])
        if not leaf_alias or source.is_symlink():
            return {"state": "restored", "source": str(source), "backupSha256": manifest["backupSha256"]}
    if any(Path(str(target) + suffix).exists() or Path(str(target) + suffix).is_symlink() for suffix in ["", "-wal", "-shm"]):
        raise ValueError("restore destination is occupied; no overwrite allowed")
    with target.open("xb") as destination, backup.open("rb") as original:
        shutil.copyfileobj(original, destination); destination.flush(); os.fsync(destination.fileno())
    os.chmod(target, 0o600)
    valid_backup(target, manifest["backupSha256"])
    if leaf_alias:
        source.symlink_to(target)
    return {"state": "restored", "source": str(source), "backupSha256": manifest["backupSha256"]}

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["archive", "restore"])
    parser.add_argument("--container", required=True)
    parser.add_argument("--previous-image", required=True)
    parser.add_argument("--previous-config", required=True)
    parser.add_argument("--source"); parser.add_argument("--business-db"); parser.add_argument("--data-root")
    parser.add_argument("--archive-root"); parser.add_argument("--operation-id"); parser.add_argument("--manifest")
    args = parser.parse_args(); stopped(args.container)
    previous_version = compatible_image(args.previous_image, args.container if args.action == "archive" else None)
    if args.action == "archive":
        if not all([args.source, args.business_db, args.data_root, args.archive_root, args.operation_id]): parser.error("archive requires exact source, business-db, data-root, archive-root and operation-id")
        outside_application_mounts(args.archive_root, args.container)
        result = archive(args.source, args.business_db, args.data_root, args.archive_root, args.operation_id, args.previous_image, args.previous_config, previous_version)
    else:
        if not args.manifest: parser.error("restore requires manifest")
        result = restore(args.manifest, args.previous_image, args.previous_config)
    print(json.dumps(result, indent=2))
if __name__ == "__main__": main()
