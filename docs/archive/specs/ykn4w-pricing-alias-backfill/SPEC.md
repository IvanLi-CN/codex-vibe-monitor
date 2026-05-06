# 日期后缀模型成本回退与历史空成本补算（#ykn4w）

## 状态

- Status: 已完成
- Created: 2026-02-28
- Last: 2026-02-28

## 背景 / 问题陈述

- 代理请求出现日期后缀模型（如 `gpt-5.2-2025-12-11`）时，成本估算仅按精确模型 ID 命中，未命中会写入 `cost = NULL`。
- 默认价目表只包含基础模型与常见别名，未逐个包含所有日期后缀快照模型，导致部署后表格出现成本 `-`。
- 历史数据中已存在 `cost IS NULL` 且 token 字段齐全的成功记录，需要自动补算，避免长期数据缺口。

## 目标 / 非目标

### Goals

- 为成本估算增加“精确命中优先，未命中再尝试日期后缀回退”的模型匹配逻辑。
- 在启动流程增加历史空成本增量补算：仅处理 `source=proxy`、`status=success`、`cost IS NULL` 的记录。
- 保持设置 API 与前端价格配置结构不变，确保兼容既有部署与手工维护流程。

### Non-goals

- 不新增前端设置项或运行期开关。
- 不引入外部在线价格拉取或自动同步官方价格。
- 不重算已存在成本值（`cost IS NOT NULL`）的历史记录。

## 范围（Scope）

### In scope

- `src/main.rs`：模型价格匹配回退逻辑（`*-YYYY-MM-DD -> base model`）。
- `src/main.rs`：新增历史空成本 backfill（分批、按 `id` 增量、幂等）。
- `src/main.rs`：启动流程串联“usage backfill -> cost backfill”。
- `README.md`：补充默认行为说明。
- `docs/specs/README.md`：登记规格索引。

### Out of scope

- `web/` 前端实现改动。
- 数据库 schema 变更。
- `PUT /api/settings/pricing` 请求体/响应体格式变更。

## 验收标准（Acceptance Criteria）

- Given 请求模型为 `gpt-5.2-2025-12-11` 且价目表存在 `gpt-5.2`，When 成本估算执行，Then `cost` 为非空且 `cost_estimated=true`。
- Given 同时存在精确条目和基础条目，When 模型为日期后缀条目，Then 必须优先命中精确条目价格。
- Given 启动时存在 `cost IS NULL` 且 usage 可用的历史成功记录，When 启动 backfill 执行，Then 对应行被补算并写入 `cost/cost_estimated/price_version`。
- Given 历史记录缺少模型、缺少 usage 或模型无定价，When backfill 执行，Then 其 `cost` 保持为空；其中“模型存在但无定价”的记录会写入尝试版号，避免同一价目快照重复扫描。
- Given 已补算完成后再次执行 backfill，When 无新增空成本记录，Then `updated=0`（幂等）。
- Given 执行 `cargo test`，When 测试结束，Then 新增回归测试通过且既有关键用例不回归。

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 新建 Spec 并登记 `docs/specs/README.md`。
- [x] M2: 实现日期后缀模型价格回退，且保持精确命中优先。
- [x] M3: 新增历史空成本 backfill，并接入启动流程。
- [x] M4: 新增单元/集成测试覆盖回退命中、精确优先、补算与跳过分支。
- [x] M5: 更新 README 成本估算说明并完成 `cargo test` 验证。

## 进度备注

- 新增日期后缀模型回退规则仅影响“模型未精确命中”的场景，不改变已有模型精确匹配行为。
- 历史空成本补算按批量扫描与条件更新执行，重复运行不会覆盖已有成本值。
