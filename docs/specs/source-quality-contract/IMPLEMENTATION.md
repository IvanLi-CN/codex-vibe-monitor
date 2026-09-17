# 全源码结构质量合同实现状态

> 当前有效规范仍以 `./SPEC.md` 为准；本文记录实现覆盖和 rollout 事实，不作为任务看板。

## Current Status

- Implementation: Source Structure CLI、tracked-source baseline、统一脚本入口、hooks 和 PR/main quality job 已落地；各业务域 remediation 与最终 zero transition 仍待后续 child tickets。
- Lifecycle: active
- Catalog note: source scope、AST limits、ratchet/zero 和 CI/hook enforcement 已冻结。

## Implementation Coverage

- Requirement coverage: `REQ-SQC-001` 至 `REQ-SQC-007` 由 `tools/source-structure-check`、Biome JSON ratchet、`scripts/check-source-quality.sh`、`scripts/check-staged-source-quality.sh`、hooks、quality-gates workflows 和连续 child PR 覆盖。
- Verification commands: `python3 /Users/ivan/.codex/bin/spec_contract_check.py --path docs/specs/source-quality-contract/SPEC.md`；后续实现必须提供 checker fixture、hook contract、workflow contract 和全量 source-quality 命令。
- Rollout facts: Bootstrap 从开发基线 `8520d9ca33a680385541cfd94b0c09a847bef50f` 开始；首次 baseline 只能由 tracked source snapshot 生成，最终必须切换 `zero` 并删除 baseline。

## Coverage / rollout summary

- 过渡阶段由 AST checker 与 Biome diagnostics ratchet 防止债务增长。
- 零阶段由无 baseline 的全量检查、严格 Clippy 阈值和 required `Source Structure Check` 共同守门。

## Remaining Gaps

- Source Structure CLI 与多语言 fixture 尚未落地。
- 当前既有超长文件、Biome diagnostics 和 Rust 结构性 suppression 仍由 ratchet baseline 管理，等待各 child ticket 收敛。
- 最终 zero transition、baseline 删除和 Clippy 固定阈值升格由收尾 child ticket 完成。

## Related Changes

- None

## References

- `./SPEC.md`
- `./HISTORY.md`
