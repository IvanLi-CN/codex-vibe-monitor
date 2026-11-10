#!/usr/bin/env python3
"""Check the PG18 analytics ownership runbook and Spec contract."""

from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
RUNBOOK = ROOT / "docs/design/pg18-analytics-ownership-cutover.md"
SPEC = ROOT / "docs/specs/pg18-analytics-ownership-v2/SPEC.md"
IMPLEMENTATION = ROOT / "docs/specs/pg18-analytics-ownership-v2/IMPLEMENTATION.md"


def require(path: Path, needles: tuple[str, ...]) -> None:
    text = path.read_text(encoding="utf-8")
    missing = [needle for needle in needles if needle not in text]
    if missing:
        raise SystemExit(f"{path}: missing required contract text: {', '.join(missing)}")


def main() -> None:
    require(
        RUNBOOK,
        (
            "ANALYTICS_OWNERSHIP_V2",
            "ANALYTICS_BACKEND",
            "观测窗口固定为 1 天",
            "PostgreSQL 故障",
            "backpressure",
            "restore/replay",
            "至少保留完整 1 天",
            "最终完全清退 SQLite 属于后续独立 source-migration 阶段",
        ),
    )
    require(SPEC, ("REQ-PG18-OWN-011", "Operational closeout evidence", "one complete day"))
    require(IMPLEMENTATION, ("Runbook and closeout contract", "ownership-test-matrix", "projection-runtime"))
    print("pg18 analytics ownership document contract: ok")


if __name__ == "__main__":
    main()
