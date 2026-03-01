# 修复 Live 实时统计闪烁与数字滚动被打断（#rkc7k）

## 状态

- Status: 部分完成（5/6）
- Created: 2026-03-02
- Last: 2026-03-02

## 背景 / 问题陈述

- Live 页面“实时统计”卡片在高频 records 推送时出现明显闪烁。
- 当前同一条 records 事件会触发两条 summary 刷新链路：`Live.tsx` 的 `onNewRecords` 手动刷新，以及 `useSummary` 内部监听 `records` 自动刷新。
- `window=current` 的刷新默认会切换 loading，导致卡片短暂显示 `…`，让数字滚动动画反复中断。
- `AnimatedDigits` 中的 `requestAnimationFrame` 缺少取消机制，旧帧可能覆盖新帧，放大“跳字”体感。

## 目标 / 非目标

### Goals

- 合并 Live summary 刷新触发源，避免重复回源。
- 对 `current` 窗口使用静默节流刷新（600ms）与 in-flight 合并，降低请求抖动与视觉闪烁。
- 修复 `AnimatedDigits` 的 rAF 竞态，保证高频更新时动画状态一致。
- 保持现有后端接口与 SSE payload schema 不变。

### Non-goals

- 不调整后端 summary 聚合逻辑与 SSE 事件结构。
- 不改动 Live 以外页面布局或视觉风格。
- 不引入新的数据库迁移或运行时配置。

## 范围（Scope）

### In scope

- `web/src/hooks/useStats.ts`
- `web/src/pages/Live.tsx`
- `web/src/components/AnimatedDigits.tsx`
- `web/src/hooks/useStats.test.ts`
- `docs/specs/README.md`

### Out of scope

- `src/main.rs` 及其他后端逻辑。
- 非实时统计模块的动画风格升级。

## 需求（Requirements）

### MUST

- `window=current` 的 records 驱动刷新必须使用 `silent` 模式，不得反复显示 loading 占位符。
- 高频 records 事件必须通过 600ms 节流窗口收敛为有限次请求。
- 刷新请求在 in-flight 时必须合并 pending 请求，避免并发风暴。
- `AnimatedDigits` 必须在 effect cleanup 时取消未执行的 rAF。

### SHOULD

- 导出可测试 helper，覆盖节流窗口和 in-flight 合并行为。
- 非 `current` 窗口（30m/1h/1d 等）现有 SSE 行为保持不变。

## 功能与行为规格（Functional/Behavior Spec）

### Core flows

- Live 页面接收连续 `records` SSE 事件时：
  - `useSummary('current')` 在节流窗口内最多触发一次静默回源。
  - 卡片数字平滑更新，不显示 `…` 占位符闪烁。
- `AnimatedDigits` 在值连续变化时：
  - 旧动画帧不会覆盖新动画路径。
  - 每次 effect 重跑都清理上一轮 rAF。

### Edge cases / errors

- 若静默刷新失败：保留现有 error 语义，不强制切换 loading。
- 若节流窗口内累计多次触发：在 in-flight 结束后仅补一次 pending 刷新。

## 接口契约（Interfaces & Contracts）

None

## 验收标准（Acceptance Criteria）

- Given Live 页面处于 `current` 窗口，When 高频 records 连续到达，Then summary 请求频率受 600ms 节流控制，且不出现每次都切回 loading 的闪烁。
- Given `AnimatedDigits` 在短时间内多次接收新值，When 动画 effect 反复触发，Then 不出现旧帧覆盖新帧导致的跳变。
- Given 30m/1h/1d 窗口，When 接收 summary SSE，Then 行为与改动前一致。

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Unit tests: `web/src/hooks/useStats.test.ts` 补充节流与 pending 合并用例。
- Unit tests: 现有 `useStats` 兼容用例继续通过。

### Quality checks

- `cd web && npm run test`
- `cd web && npm run build`

## 文档更新（Docs to Update）

- `docs/specs/README.md`: 新增 #rkc7k 规格索引，并在实现完成后回填状态。

## 计划资产（Plan assets）

- Directory: `docs/specs/rkc7k-live-summary-flicker-fix/assets/`

## 资产晋升（Asset promotion）

None

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 新建 spec 并登记 index。
- [x] M2: `useSummary` 增加 current 窗口静默节流刷新与 in-flight 合并。
- [x] M3: 移除 Live 页重复 summary 刷新触发。
- [x] M4: 修复 `AnimatedDigits` rAF cleanup 竞态。
- [x] M5: 完成前端测试与构建验证。
- [ ] M6: 完成 fast-track PR、checks 跟踪与 review-loop 收敛。

## 风险 / 开放问题 / 假设（Risks, Open Questions, Assumptions）

- 风险：节流窗口可能引入轻微数据延迟（<= 600ms）。
- 假设：该延迟可接受，优先换取视觉稳定性。

## 变更记录（Change log）

- 2026-03-02: 创建规格并冻结实现边界。
- 2026-03-02: 完成 M1-M5，实现前端节流刷新与动画竞态修复并通过 web test/build。
