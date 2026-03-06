# 内置 GPT-5.4 系列计费规则与下游模型列表（#7272y）

## 状态

- Status: 已完成（4/4）
- Created: 2026-03-06
- Last: 2026-03-06

## 背景 / 问题陈述

- 代理侧需要为下游提供可选模型列表（`/v1/models` hijack preset），以及在未配置外部 pricing catalog 时的默认计费目录。
- 新增 `gpt-5.4` / `gpt-5.4-pro` 后，需要内置默认单价，并在 prompt 超过 272K tokens 时按官方规则加价，避免成本估算偏低。
- 现有 SQLite 中的 pricing / enabled models 需要做到“缺啥补啥”，且不覆盖用户自定义价格/条目。

## 目标 / 非目标

### Goals

- 默认 pricing catalog 内置 `gpt-5.4` / `gpt-5.4-pro` 的单价。
- `estimate_proxy_cost` 对 `gpt-5.4*` 在 `usage.input_tokens > 272_000` 时应用加价：
  - input cost x2（包含 cached input 部分）
  - output cost x1.5
  - reasoning cost x1.5
- `/v1/models` hijack preset 列表可向下游暴露 `gpt-5.4` / `gpt-5.4-pro`（仅扩展 id 集合，不改变 payload 结构）。
- SQLite 启动/加载阶段自动补齐：
  - pricing 缺少新模型条目则 `INSERT OR IGNORE` 插入
  - proxy enabled list 仅在保持 legacy 默认列表时才追加，避免覆盖用户自定义

### Non-goals

- 不引入在线自动同步 pricing 的能力。
- 不调整已有模型的价格（除 source 字段归一化为 `official`）。
- 不更改任何 API schema，仅调整默认数据与估算逻辑。

## 范围（Scope）

### In scope

- Rust backend: `src/main.rs` pricing catalog / preset list / surcharge / seed。
- Unit tests covering surcharge edge and seeding behavior.

### Out of scope

- Web UI changes.
- 自动更新外部 pricing 配置文件。

## 需求（Requirements）

### MUST

- 不覆盖 `pricing_settings_models` 中已有的同名 model 行（保留用户值）。
- `gpt-5.4*` 超阈值加价按 whole session 计费（对 input/output/reasoning 部分分别乘倍率）。
- `/v1/models` hijack 仍返回 `{object:\"list\", data:[{id, object, owned_by, created}, ...]}`。

### SHOULD

- 为新逻辑补齐回归测试（in-memory sqlite + pure unit tests）。

### COULD

- 将阈值/倍率配置化（未来）。

## 功能与行为规格（Functional/Behavior Spec）

### Core flows

- 当 pricing catalog 为默认目录（或 legacy 默认目录）且 SQLite 缺少 `gpt-5.4` / `gpt-5.4-pro` 行时：启动时自动插入两行（`INSERT OR IGNORE`）。
- 当 `/v1/models` hijack 开启：
  - preset 模型列表集合包含 `gpt-5.4` / `gpt-5.4-pro`
  - enabled list 等于 legacy 默认列表时，自动追加新模型
  - enabled list 为用户自定义时不自动改动
- 成本估算：
  - `model` 以 `\"gpt-5.4\"` 前缀匹配
  - 若 `usage.input_tokens > 272_000`：input x2、output x1.5、reasoning x1.5
  - 阈值等于 272_000 时不触发（strictly greater）

### Edge cases / errors

- pricing catalog 为自定义 version 时，不自动插入新模型（避免干预用户自定义目录）。
- `gpt-5.4-pro` 不提供 cache input 单价时，cached tokens 不作为 cache 计费（保持既有行为）。

## 接口契约（Interfaces & Contracts）

None

## 验收标准（Acceptance Criteria）

- Given SQLite 中缺少 `gpt-5.4` / `gpt-5.4-pro` pricing 行
  When 启动并加载 pricing catalog（默认/legacy 默认 version）
  Then 两模型条目会被插入；若行已存在则不会被覆盖

- Given `/v1/models` hijack enabled 且 enabled_models 包含 `gpt-5.4` / `gpt-5.4-pro`
  When `GET /v1/models`
  Then 返回列表包含两个 id，payload 结构保持不变

- Given `model=gpt-5.4` 且 `input_tokens=272000`
  When 估算成本
  Then 不触发加价

- Given `model=gpt-5.4` 且 `input_tokens=272001`
  When 估算成本
  Then input 部分 x2，output 部分 x1.5（reasoning 同 output）

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Unit tests: `cargo test`

### Quality checks

- `cargo fmt` / `cargo clippy` via lefthook (commit hooks).

## 文档更新（Docs to Update）

- `docs/specs/README.md`: add index row for #7272y
- `docs/specs/7272y-gpt-5-4-pricing/SPEC.md`: this spec

## Visual Evidence (PR)

None

## 资产晋升（Asset promotion）

None

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: Add `gpt-5.4` / `gpt-5.4-pro` to preset model ids for downstream `/v1/models`.
- [x] M2: Add default pricing catalog entries and catalog version refresh.
- [x] M3: Add long-context surcharge rule for `gpt-5.4*`.
- [x] M4: Add SQLite ensure/seed logic and regression tests.

## 方案概述（Approach, high-level）

- Keep pricing defaults embedded; seed missing rows in SQLite via `INSERT OR IGNORE` to preserve user overrides.
- Apply long-context surcharge only for `gpt-5.4*` and only when prompt tokens strictly exceed threshold.

## 风险 / 开放问题 / 假设（Risks, Open Questions, Assumptions）

- 风险：若 upstream pricing/规则变更，需要再次手动刷新默认目录。
- 假设：`usage.input_tokens` 代表 prompt token 数量（用于 272K 阈值判定）。

## 变更记录（Change log）

- 2026-03-06: Create spec and record current implementation (PR #89).

## 参考（References）

- PR #89
