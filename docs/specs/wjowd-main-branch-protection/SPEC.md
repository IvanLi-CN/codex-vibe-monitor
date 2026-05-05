# `main` 主干保护禁止直推与 PR 全检查必过（#wjowd）

## 背景 / 问题陈述

- 仓库已经声明 `main` 需要 PR 与 required checks，但实际 GitHub ruleset 仍允许高权限身份直接 push。
- `.github/quality-gates.json` 只把 5 个 job 作为 required checks，`Front-end Tests` 与 `Records Overlay E2E` 仍是 informational，PR 并不是真正意义上的“全检查通过才可合并”。
- `check_live_quality_gates.py` 会把缺失的 `bypass_actors` 当成空数组，导致 live 输出可能错误地宣称“已验证 bypass actors”。

## 目标 / 非目标

### Goals

- 禁止任何身份直接提交到 `main`，包括 owner / admin。
- 让 PR 合并门槛收敛为“7 个治理检查全部通过”。
- 修复 quality-gates live 输出，避免对 bypass 状态产生假阳性结论。
- 将治理要求沉淀到 spec、solution 与 README，保持仓库内外真相一致。

### Non-goals

- 不新增 GitHub native approval 强制策略。
- 不改 merge queue、release label 或 merge method 语义。
- 不修改产品运行时 API、前端功能或部署拓扑。

## 接口契约（Interfaces & Contracts）

### `required_checks`

- 固定为以下 7 项：
  - `Validate PR labels`
  - `Lint & Format Check`
  - `Front-end Tests`
  - `Records Overlay E2E`
  - `Backend Tests`
  - `Build Artifacts`
  - `Review Policy Gate`

### `informational_checks`

- 对 PR gate 固定为空；任何会阻止合并的 PR 检查都必须进入 `required_checks`。

### live quality-gates 输出

- 输出必须带 `bypass_actor_status`，至少支持：
  - `verified`
  - `unverified`
- 当 ruleset payload 未显式返回 `bypass_actors` 时，只能报告 `unverified`，不能宣称“已验证 bypass actors”。

## 功能规格

### 1. 仓库契约收紧

- `.github/quality-gates.json`、contract fixtures 与 live fixtures 全部对齐到 7 个 required checks。
- `check_quality_gates_contract.py` 必须拒绝非空 `informational_checks`，并强制 `expected_pr_workflows` 的 job 集合与 `required_checks` 完全一致。

### 2. live bypass 状态输出

- `check_live_quality_gates.py` 不再把缺失的 `bypass_actors` 自动补成空数组。
- 若 ruleset payload 中 `bypass_actors` 缺失或不可验证，脚本输出标记 `unverified` 并给出说明 note。
- 只有显式拿到空数组时，才可输出 `verified`。

### 3. GitHub `main` ruleset

- 浏览器实配后，`main` 必须满足：
  - 仅允许 PR 合并
  - 禁止 bypass
  - 7 个 required checks 全绿
  - require signed commits
  - 禁止 force push
  - 禁止删除分支

## 验收标准（Acceptance Criteria）

- Given 当前可写身份，When 尝试 `git push origin main`，Then GitHub 服务端拒绝直推。
- Given PR 上任一 required check 未通过，When 查看合并状态，Then PR 不可合并。
- Given PR 上 7 个 required checks 全绿，When freshness / review proof 收敛完成，Then PR 达到 merge-ready。
- Given live quality-gates 无法从 ruleset payload 证明 bypass actors，When 脚本输出结果，Then 结果显示 `bypass_actor_status=unverified`，且不再出现“Validated effective branch rules and bypass actors”式假阳性说明。

## 质量门槛（Quality Gates）

- `python3 .github/scripts/check_quality_gates_contract.py --repo-root "$PWD" --declaration .github/quality-gates.json --metadata-script .github/scripts/metadata_gate.py --profile final`
- `bash .github/scripts/test-quality-gates-contract.sh`
- `bash .github/scripts/test-live-quality-gates.sh`
- `python3 .github/scripts/check_live_quality_gates.py --repo IvanLi-CN/codex-vibe-monitor --branch main --mode require`

## 风险 / 假设

- 假设：浏览器会话已具备修改仓库 ruleset / branch protection 的权限。
- 风险：GitHub ruleset API 对 `bypass_actors` 的可见性与认证上下文相关，因此 live script 只能保证“不误报已验证”，不能代替浏览器实配本身。
