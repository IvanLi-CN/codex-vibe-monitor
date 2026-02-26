# 修复更新提示可读性（#67acu）

## 状态

- Status: 已完成
- Created: 2026-02-26
- Last: 2026-02-26

## 背景 / 问题陈述

- Dashboard 顶部“有新版本可用”提示条在浅色主题下文本对比度不足，用户难以辨识版本号与提示内容。
- `AppLayout` 使用 `bg-info/12` 这类非标准透明度 class，Tailwind 不生成该样式，导致背景退化为透明，进一步放大可读性问题。
- 仓库内还存在同类透明度写法（`/12`、`/14`），存在重复回归风险。

## 目标 / 非目标

### Goals

- 修复更新提示条在浅/深色主题下的可读性并补足基础可访问性语义。
- 将更新提示条抽离为独立组件，降低 `AppLayout` 复杂度并增加回归测试覆盖。
- 修复同类非标准透明度写法：`Alert` warning/error 和今日统计卡片渐变。
- 新增组件级测试，防止回退到不可读样式。

### Non-goals

- 不改动后端 API、SSE 协议、数据库结构或版本检测逻辑。
- 不改动更新提示条出现/消失时机（`useUpdateAvailable` 行为保持不变）。
- 不调整页面其它区域布局与交互。

## 范围（Scope）

### In scope

- `web/src/components/UpdateAvailableBanner.tsx`（新增）
- `web/src/components/AppLayout.tsx`（替换内联更新提示条）
- `web/src/components/ui/alert.tsx`（修正 warning/error 透明度类）
- `web/src/components/TodayStatsOverview.tsx`（修正渐变透明度类）
- `web/src/components/UpdateAvailableBanner.test.tsx`（新增）
- `web/src/components/ui/alert.test.tsx`（新增）
- `docs/specs/README.md`（索引登记）

### Out of scope

- `src/` 下 Rust 后端代码。
- 其它页面配色体系重构。

## 验收标准（Acceptance Criteria）

- Given 浅色主题 + 中文界面，When 出现更新提示，Then “有新版本可用：0.10.2 → 0.10.4”可清晰识别，按钮可点击。
- Given 深色主题 + 中文界面，When 出现更新提示，Then 文本对比度与按钮层次清晰。
- Given 英文界面，When 出现更新提示，Then 文案不溢出且布局正常。
- Given 移动端宽度（375）与桌面宽度（1536），When 出现更新提示，Then 不遮挡关键操作且不横向溢出。
- Given 运行前端测试，When 执行 `npm run test`，Then 覆盖更新提示组件及 alert 透明度回归断言。
- Given SSE 离线提示存在，When 更新提示出现，Then 互不影响，离线提示行为不回归。

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 建立 Spec 并登记到 `docs/specs/README.md`。
- [x] M2: 新增 `UpdateAvailableBanner` 组件并接入 `AppLayout`。
- [x] M3: 修复 `alert.tsx` 与 `TodayStatsOverview.tsx` 的非标准透明度类。
- [x] M4: 新增 `UpdateAvailableBanner.test.tsx` 与 `alert.test.tsx`。
- [x] M5: 完成 lint/test/build + `cargo fmt --check` 验证。
- [x] M6: 完成浅/深色 + 中/英 + 移动/桌面视觉验证并记录结果。

## 进度备注

- 已完成更新横幅抽离与可读性修复：容器改为 `bg-info/15 + text-base-content`，版本号高亮 `text-info`，并增加 `role="status" aria-live="polite"`。
- 已修复同类透明度写法：`alert.tsx` warning/error 分别统一为 `/15`，`TodayStatsOverview` 主卡渐变 `from-primary/15`。
- 已通过 `npm run lint`、`npm run test`、`npm run build`、`cargo fmt --all -- --check`。
- 已完成 Playwright 视觉核验：桌面浅色中文、桌面深色中文、桌面浅色英文、移动端浅色中文，横幅文本与操作按钮均可读可点。
