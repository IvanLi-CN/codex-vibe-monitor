# 受控手动发版覆盖 - Implementation

- Canonical spec: `docs/specs/m7q2r-manual-release-override/SPEC.md`

## Current Status

- [x] M1: `release_snapshot.py` 支持 job-local `manual-release-override` snapshot。
- [x] M2: `release.yml` 支持手动覆盖 dispatch 输入，并保持内部 queue dispatch 的 immutable snapshot backfill 兼容。
- [x] M3: GitHub Release body 输出手动覆盖审计字段。
- [x] M4: release snapshot 与 quality-gates contract 回归测试覆盖新路径。

## Verification

- `bash .github/scripts/test-release-snapshot.sh`
- `bash .github/scripts/test-quality-gates-contract.sh`
- `python3 -m py_compile .github/scripts/release_snapshot.py .github/scripts/check_quality_gates_contract.py`
