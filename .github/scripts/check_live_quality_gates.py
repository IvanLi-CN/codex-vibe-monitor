#!/usr/bin/env python3
from pathlib import Path
import sys


script_dir = Path(__file__).resolve().parent
sys.path.insert(0, str(script_dir))
for implementation in ("live_quality_rules.py", "../check_live_quality_gates_impl.py"):
    implementation_path = (script_dir / implementation).resolve()
    source = implementation_path.read_text(encoding="utf-8")
    exec(compile(source, str(implementation_path), "exec"), globals())
