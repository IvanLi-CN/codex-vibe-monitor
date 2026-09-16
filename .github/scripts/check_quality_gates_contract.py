#!/usr/bin/env python3
from pathlib import Path


implementation_path = (Path(__file__).resolve().parent / ".." / "quality_gates_contract_impl.py").resolve()
validator_path = implementation_path.with_name("quality-gates-contract-validators.py")
release_snapshot_validator_path = implementation_path.with_name("quality-gates-contract-release-snapshot-validator.py")
for path in (validator_path, release_snapshot_validator_path, implementation_path):
    source = path.read_text(encoding="utf-8")
    exec(compile(source, str(path), "exec"), globals())
