# 全源码结构质量零收敛

> 本文是源码结构质量债务收敛到零的长期交付合同。当前实现覆盖与 rollout 事实记录在 `IMPLEMENTATION.md`，主题生命周期记录在 `HISTORY.md`。

## Context and Scope

- Context: `source-quality-contract` 已建立全源码结构质量合同与不可增长 ratchet；现有集成头仍保留可测量的历史诊断，需要按模块边界收敛到不可逆 zero 状态。
- In scope: 受 `source-quality-contract` 管辖的 tracked Rust、TypeScript、TSX、JavaScript、CSS、Python 和 Shell 源码，及其 Source Structure Check、Biome、Clippy、hook 和 Integration CI 收口证据。
- Out of scope: 公开 API、数据库 schema、运行时业务语义、发布版本、镜像内容和第三方生成物。

## Terms and Interfaces

- `diagnostic frontier`: 当前集成分支经过全量 Source Structure Check 后的稳定诊断身份与度量集合。
- `child convergence`: 一个独立拥有源码边界、可单独验证并可合并的 child PR。
- `zero ratchet`: 在所有 child convergence 完成后，删除 baseline 并启用不可逆的 zero 模式。
- Interface: `prd/source-quality-zero` branch-push Integration CI、Source Structure Check、Biome、Clippy 和最终 aggregate PR。

## Requirements

### REQ-SQZ-001

- Remediation MUST partition the diagnostic frontier into disjoint owned surfaces; every child convergence MUST have an independently verifiable acceptance boundary.
- No child convergence may add a path exclusion, waiver, structural suppression, threshold relaxation, or Biome suppression to make its surface pass.

### REQ-SQZ-002

- Every child PR targeting the integration branch MUST pass the exact branch-push `initiative-source-structure-check` against its current head and the shared source-quality contract.
- The Integration CI MUST be isolated to `prd/source-quality-zero`, use `contents: read`, and contain no release or image publication step.

### REQ-SQZ-003

- The final zero ratchet MUST remain blocked until every predecessor surface reports zero diagnostics, zero parse errors, and no metric growth or structural suppression.
- Zero transition MUST delete the ratchet baseline and record an irreversible zero state; the state MUST reject baseline recreation or rollback.

### REQ-SQZ-004

- Final acceptance MUST prove that the tracked source inventory, Biome diagnostics, Rust structural lint thresholds, hooks, and repository quality gates are all clean at one exact aggregate head.
- The aggregate proof MUST preserve the existing runtime, API, schema, release, and image contracts.

## Verification

### VER-SQZ-001

- Method: inventory the tracked source scope and partition the diagnostic frontier by child-owned surface.
- covers: `REQ-SQZ-001`
- Pass condition: owned surfaces are disjoint, every diagnostic has one owner, and no waiver or exclusion is introduced.

### VER-SQZ-002

- Method: branch-push Integration CI and child PR required-check inspection.
- covers: `REQ-SQZ-002`
- Pass condition: the exact named check passes on the bound head, only the successor integration branch triggers it, and permissions are read-only.

### VER-SQZ-003

- Method: zero-ratchet fixture and final full-repository validation.
- covers: `REQ-SQZ-003`, `REQ-SQZ-004`
- Pass condition: baseline is absent, state is irreversible `zero`, all diagnostics and parse errors are zero, and all required quality gates pass at the aggregate head.

## Related ADRs

None

## References

- `../source-quality-contract/SPEC.md`
- `./IMPLEMENTATION.md`
- `./HISTORY.md`
