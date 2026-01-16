# 接入 Claude Relay 日统计源（#0001）

## 状态

- Status: 待实现
- Created: 2026-01-16
- Last: 2026-01-16

## 问题陈述

目前系统只抓取既有数据源的调用记录与配额快照，无法纳入新的外部日统计源；需要在后端接入该统计源并在 UI 中合并展示，同时在数据库内可区分来源，以便后续分析与扩展。

## 目标 / 非目标

### Goals

- 接入外部日统计源，按约 10 秒节奏拉取并持久化。
- 数据库内可区分数据来源，统计层做合并展示。
- 现有统计 API 与 SSE 的结构保持兼容，语义更新为“合并口径”。

### Non-goals

- 不实现该外部源的“最近请求日志/逐条调用明细”。
- 不新增前端“按来源筛选”功能（除非后续明确需求）。
- 不做历史回填或跨日补录（除非后续明确需求）。
- 不接入 `period=monthly` 的统计（后续需求再评估）。

## 用户与场景

- 运营/维护者在统计页查看当日与近几日的总体使用量，希望合并展示多来源数据。
- 后续可能需要按来源审计或扩展更多数据源，因此需要在库中保留来源维度。

## 范围（Scope）

### In scope

- 外部日统计源的拉取、解析、持久化与合并口径。
- 数据库结构扩展以区分来源并支持“日统计快照/增量”。
- 统计 API 与 SSE 事件的合并口径调整。
- 配置与文档更新（新增必要的配置项说明）。

### Out of scope

- 新的 UI 交互或来源筛选控件。
- 该外部源的逐条调用明细或错误详情。
- 历史数据回填或跨日回溯聚合。

## 需求（Requirements）

### MUST

- 支持外部日统计源拉取，轮询周期约 10 秒。
- 持久化时标记数据来源；默认统计口径为“所有来源合并”。
- 当外部源仅提供日累计时，采用可重复执行且不重复计数的累积策略。
- 日切换或数值回退时具备可预期的重置/对账行为。
- 不依赖该外部源提供“最近请求日志”。
- 外部统计无失败细分时，按 `failure=0`、`success=total` 口径处理。

### SHOULD

- 外部源的 base URL / apiId / period 通过配置项提供，避免硬编码。
- 统计 API 在缺失外部数据时仍可正常返回（退化但不报错）。
- 增加必要的日志/指标，便于观察外部拉取失败、日切换与增量异常。

### COULD

- 增加内部调试用的“按来源过滤统计”能力（不影响默认合并口径）。
- 在 `/api/invocations` 中可选地暴露来源字段（保持向后兼容）。

## 接口契约（Interfaces & Contracts）

### 接口清单（Inventory）

| 接口（Name）                                        | 类型（Kind） | 范围（Scope） | 变更（Change） | 契约文档（Contract Doc）    | 负责人（Owner） | 使用方（Consumers） | 备注（Notes）                |
| --------------------------------------------------- | ------------ | ------------- | -------------- | --------------------------- | --------------- | ------------------- | ---------------------------- |
| 外部日统计源（POST /apiStats/api/user-model-stats） | HTTP API     | external      | New            | ./contracts/http-apis.md    | backend         | poller              | period=daily（无鉴权）       |
| GET /api/stats                                      | HTTP API     | internal      | Modify         | ./contracts/http-apis.md    | backend         | web                 | 合并口径                     |
| GET /api/stats/summary                              | HTTP API     | internal      | Modify         | ./contracts/http-apis.md    | backend         | web/SSE             | 合并口径                     |
| GET /api/stats/timeseries                           | HTTP API     | internal      | Modify         | ./contracts/http-apis.md    | backend         | web                 | 合并口径（受数据可用性影响） |
| SSE: summary event                                  | Event        | internal      | Modify         | ./contracts/events.md       | backend         | web                 | summary 合并口径             |
| SQLite schema                                       | DB           | internal      | Modify         | ./contracts/db.md           | backend         | backend             | 来源列 + 日统计快照          |
| .env.local                                          | File format  | internal      | Modify         | ./contracts/file-formats.md | backend         | ops                 | 新增外部源配置               |

### 契约文档（按 Kind 拆分）

- [contracts/README.md](./contracts/README.md)
- [contracts/http-apis.md](./contracts/http-apis.md)
- [contracts/events.md](./contracts/events.md)
- [contracts/db.md](./contracts/db.md)
- [contracts/file-formats.md](./contracts/file-formats.md)

## 约束与风险

- 外部源仅提供“当日累计”，无法还原逐条调用细节。
- 外部接口为 JSON POST；当前无需鉴权，但需明确稳定性与限流策略。
- 日切换或服务端回滚可能导致累计值下降，需要明确定义处理策略。
- 时区与“日”的边界固定为东八区（Asia/Shanghai）。
- 合并口径后，错误分布/明细与总数可能出现不一致，需要说明范围。

## 验收标准（Acceptance Criteria）

- Given 配置了外部日统计源信息
  When 后端轮询运行
  Then 数据会被持久化且标记来源，不会重复计数。
- Given 统计页请求 `/api/stats` 或 `/api/stats/summary`
  When 外部源有数据
  Then 返回的 totals 为多来源合并口径。
- Given 外部源不提供 failure/success 细分
  When 统计聚合发生
  Then `success=total` 且 `failure=0`。
- Given 外部源数据回退或跨日清零
  When 系统检测到回退/切日
  Then 按约定策略重置当日累计且不产生负增量。
- Given 外部源暂时不可用
  When 拉取失败
  Then 统计 API 仍返回既有来源数据并记录错误日志。
- Given 北京时间到达 00:00
  When 进入新的一天
  Then 外部源日统计以东八区日界重置并从 0 开始累积。
- Given 北京时间非 0 点出现累计回到 0
  When 系统检测到该异常
  Then 记录错误日志并忽略该次回退（保持当日最大值）。

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Unit tests: 外部统计增量计算、回退/日切换处理。
- Integration tests: 统计聚合（含多来源合并）与 API 返回一致性。
- E2E tests (if applicable): 暂无。

### UI / Storybook (if applicable)

- Stories to add/update: 暂无。
- Visual regression baseline changes (if any): 无。

### Quality checks

- `cargo fmt`
- `cargo check`
- `cargo test`

## 文档更新（Docs to Update）

- `README.md`: 新增外部统计源配置项与合并口径说明。
- `DESGIN.md`: 更新“数据来源定位/入库设计/统计口径”章节。

## 里程碑（Milestones）

- [ ] M1: 明确外部接口契约与口径（含样例数据/时区/回退策略）。
- [ ] M2: 数据库结构与增量计算方案确定（来源区分 + 日统计快照）。
- [ ] M3: 统计 API/SSE 合并口径方案与测试清单确认。

## 方案概述（Approach, high-level）

- 基于 `POST /apiStats/api/user-model-stats`（period=daily）拉取模型级日统计，汇总为当日总量。
- 存储原始快照并计算“可累积的增量”，用于与现有来源做合并口径。
- 失败/成功口径：外部源无细分时按 `failure=0`、`success=total` 处理。
- 时区口径：以东八区（Asia/Shanghai）对齐“日”边界。
- 回退处理：默认取当日**最大累计值**；若检测到非北京时间 0 点出现回到 0，记录错误日志并**忽略该次回退**（保持当日最大值）。
- 统计 API/SSE 在聚合时合并多来源数据；对无明细来源仅参与汇总。
- 日切换/回退通过明确规则处理，保证累计数单调且可解释。

## 风险与开放问题（Risks & Open Questions）

- 风险：合并后错误分布与总量不一致可能引发理解偏差。
- 风险：若外部源在非 0 点回到 0，将按“取当日最大值”处理，可能低估实际当日总量（但保证口径稳定）。

## 开放问题（需要主人回答）

- 无。

## 假设（Assumptions，待确认）

- 无。

## 参考（References）

- 项目现有统计 API 与数据表定义。
- Chrome DevTools 抓包（2026-01-16，外部统计接口）。
