#!/usr/bin/env python3
from pathlib import Path


implementation_path = (Path(__file__).resolve().parent.parent / ".github" / "forward_proxy_backtest_impl.py").resolve()
source = implementation_path.read_text(encoding="utf-8")
exec(compile(source, str(implementation_path), "exec"), globals())
