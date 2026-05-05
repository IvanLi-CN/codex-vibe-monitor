# Prompt Cache 图表时间轴 24 小时封顶热修（#e6082）

## 状态

- Status: 已完成（4/4）
- Created: 2026-03-20
- Last: 2026-03-20

## 背景 / 问题陈述

- Prompt Cache Key 对话表新增“近 1/3/6/12/24 小时活动”筛选后，图表列当前按“表内最早 `createdAt` 到当前时间”直接计算共享时间轴。
- 当某个对话创建于 24 小时之前、但最近仍有活动时，图表列标题会被拉成 `50 小时 Token 累计` 之类的错误跨度。
- 该行为违背了“图表上限仍是最近 24 小时”的产品要求，也让活动时间筛选与图表累计口径出现错位。

## 目标 / 非目标

### Goals

- 将 Prompt Cache Key 对话图表的共享时间轴强制封顶为最近 24 小时。
- 保持“所有行共享同一开始/结束时间”的表格内一致性。
- 修正后端图表点位查询窗口，使累计值仅基于最近 24 小时内的请求点重新计算。
- 为前后端补回归测试，防止再次出现 `>24h` 的标题与累计口径漂移。

### Non-goals

- 不改动 Prompt Cache Key 对话筛选选项、排序规则或隐含过滤提示文案。
- 不修改 Sticky Key 对话表的 24 小时 sparkline 语义。
- 不新增 Storybook 视觉资产或重新设计 Live 页面布局。

## 范围（Scope）

### In scope

- `src/api/mod.rs` 与 `src/tests/mod.rs` 中 Prompt Cache 对话图表窗口起点与事件查询范围。
- `web/src/components/PromptCacheConversationTable.tsx` 与对应 Vitest 回归。
- `docs/specs/README.md` 与当前热修 spec 的状态同步。

### Out of scope

- Prompt Cache 对话接口的筛选参数契约。
- Live 页其它表格或卡片的时间窗口逻辑。
- Storybook mock 数据重做。

## 功能与行为规格（Functional/Behavior Spec）

### Core flows

- Prompt Cache Key 对话表图表范围改为：
  - `rangeEnd = now`
  - `rangeStart = max(表内最早 createdAt, now - 24h)`
- 后端 Prompt Cache 图表点位查询同样只取 `rangeStart..rangeEnd` 范围内的事件，并以该范围内的第一条点为累计起点。
- 当某个对话创建于 24 小时之前但最近仍有活动时：
  - 表格行依然允许展示
  - 图表列标题必须保持 `24 小时 Token 累计`
  - sparkline 只显示最近 24 小时的累计变化

### Edge cases / errors

- 若表内所有对话都创建于最近 24 小时内，则图表标题仍按真实小时数向上取整显示，不做额外抬高。
- 若图表窗口内没有请求点，保留空图表外观，但标题仍不得超过 `24 小时`。
- 若后端历史数据包含 24 小时外的旧请求点，不得继续混入 Prompt Cache 图表累计序列。

## 验收标准（Acceptance Criteria）

- Given 选择 `近 1 小时活动`，When 表内包含创建于 24 小时之前但最近仍活跃的对话，Then 图表列标题最多显示 `24 小时 Token 累计`。
- Given 某个对话既有 50 小时前的历史点，也有最近 24 小时内的新点，When 渲染 Prompt Cache 图表，Then 累计值只基于最近 24 小时内的点重新计算。
- Given 运行 Prompt Cache 相关 Rust 与 Vitest 回归，When 执行本次热修验证命令，Then 新增 24 小时封顶断言通过，且原有筛选/提示逻辑不回归。

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Rust: `cargo test prompt_cache_conversations -- --nocapture`
- Web: `cd web && bunx vitest run src/components/PromptCacheConversationTable.test.tsx`

### Quality checks

- Rust format: `cargo fmt`
- Rust type-check: `cargo check`

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 收敛问题根因，确认当前图表窗口实现缺少 24 小时封顶。
- [x] M2: 后端图表事件查询窗口与前端共享时间轴同时加上 `24h` 封顶。
- [x] M3: 补齐前后端回归测试并完成本地验证。
- [x] M4: fast-flow 推送、PR、checks 与 review-loop 收敛到 merge-ready。

## 风险 / 假设

- 风险：Prompt Cache 对话行允许展示 24 小时前创建的老对话，因此图表封顶后会出现“创建时间早于图表起点”的正常现象；这是预期行为，不应再被解释成数据缺失。
- 假设：主人要求的“上限 24 小时”适用于图表展示与累计口径，而不是禁止展示 24 小时前创建但仍在筛选窗口内活跃的对话。

## 变更记录（Change log）

- 2026-03-20: 新建热修 spec，冻结 Prompt Cache 图表时间轴必须封顶 24 小时的修复边界与验证要求。
- 2026-03-20: 实现前后端 24 小时封顶修复，并同步收齐 fast-flow 的 PR / spec 门禁记录。
