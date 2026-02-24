# 接入 ui-ux-pro-max（Codex）并修正 .gitignore 追踪策略（#q86c7）

## 状态

- Status: 已完成
- Created: 2026-02-24
- Last: 2026-02-24

## 背景 / 问题陈述

- 当前仓库未接入 `ui-ux-pro-max` Codex skill，无法直接使用 styles/design-system 搜索能力。
- 当前 `.gitignore` 直接忽略 `.codex/`，导致 skill 资产无法被仓库追踪与复现。
- 需要在不改业务功能的前提下完成技能接入与可提交追踪。

## 目标 / 非目标

### Goals

- 在当前仓库安装 `ui-ux-pro-max`（Codex 模板）。
- 调整 `.gitignore`，允许追踪 `.codex/skills/**`，继续忽略运行噪音（logs/evidence）。
- 完成最小可用验证并走完 fast-track（push + PR + checks + review-loop）。

### Non-goals

- 不修改 `src/**`、`web/src/**` 业务逻辑。
- 不引入新的运行时依赖或 API 变更。

## 范围（Scope）

### In scope

- `docs/specs/**` 新增与状态维护。
- `.codex/skills/ui-ux-pro-max/**` 安装产物入仓。
- `.gitignore` 规则细化。
- PR 创建、标签设置与收敛汇报。

### Out of scope

- UI 页面改造、视觉重构、组件重写。
- 旧 `docs/plan/**` 批量迁移。

## 需求（Requirements）

### MUST

- 使用 `uipro-cli@latest` 生成 Codex skill 目录。
- `.codex/skills/**` 可追踪，`.codex/logs/**` 与 `.codex/evidence/**` 继续忽略。
- 验证 `search.py --design-system` 可执行成功。
- 使用 conventional commit + `--signoff`。

### SHOULD

- PR body 明确冻结范围与验收标准。
- PR 标签满足 `type:docs + channel:stable`。

### COULD

- 增补一次 `--domain style` 检索作为 styles 可用性辅助证明。

## 功能与行为规格（Functional/Behavior Spec）

### Core flows

- 在仓库根目录执行安装命令后，生成 `.codex/skills/ui-ux-pro-max/`，包含 `SKILL.md`、`data/`、`scripts/`。
- 执行 `python3 .codex/skills/ui-ux-pro-max/scripts/search.py ... --design-system` 返回成功并输出设计系统。
- Git ignore 规则允许 skill 文件出现在 `git status`，但日志/证据文件仍被 ignore。

### Edge cases / errors

- 若安装失败或脚本不可执行：阻断提交并修复环境问题后重试。
- 若 `.gitignore` 规则覆盖错误导致运行噪音入仓：必须修正后再提交。

## 接口契约（Interfaces & Contracts）

None

## 验收标准（Acceptance Criteria）

- Given 仓库在目标分支，When 执行 skill 安装命令，Then `.codex/skills/ui-ux-pro-max/SKILL.md`、`data/*.csv`、`scripts/search.py` 存在。
- Given 安装完成，When 执行 `search.py --design-system`，Then 命令退出码为 0 且输出包含设计系统字段。
- Given 更新后的 `.gitignore`，When 检查 ignore 语义，Then `.codex/skills/**` 不被 ignore 且 `.codex/logs/**`、`.codex/evidence/**` 被 ignore。
- Given PR 创建，When 标签校验运行，Then 通过 `type:*` + `channel:*` 门禁。

## 实现前置条件（Definition of Ready / Preconditions）

- [x] 目标/范围/验收标准已冻结。
- [x] 变更边界明确为工程配置与技能资产。
- [x] 分支策略与 PR 标签已确定。

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- 命令级验证：`search.py --design-system` 成功。
- 命令级验证：`search.py --domain style` 成功。

### Quality checks

- `git check-ignore` 语义检查。
- 无业务代码改动（`src/**`、`web/src/**` 不变）。

## 文档更新（Docs to Update）

- `docs/specs/README.md`
- `docs/specs/q86c7-setup-uipro-codex/SPEC.md`

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 完成 `docs/specs` 初始化与规格建档。
- [x] M2: 完成 Codex skill 安装与 `.gitignore` 调整。
- [x] M3: 完成验证、提交、PR、checks 与 review-loop 收敛。

## 方案概述（Approach, high-level）

- 先补齐 specs-first 门禁，再在独立分支执行最小改动。
- 通过 CLI 安装官方 skill 资产，避免手工拼装文件。
- 用 `git check-ignore` 与脚本执行结果作为验收证据。

## 风险 / 开放问题 / 假设（Risks, Open Questions, Assumptions）

- 风险：latest 版本变化可能造成资产差异。
- 开放问题：无。
- 假设：GitHub MCP 与本地 `codex review` 命令可用。

## 变更记录（Change log）

- 2026-02-24: 初始化规格并冻结实现口径。
- 2026-02-24: 完成 skill 接入、PR #50、CI 通过与 review 轮次收敛记录。
