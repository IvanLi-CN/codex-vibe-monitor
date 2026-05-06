# Sticky Footer 修复：页脚在短页面贴底（#zrxcd）

## 状态

- Status: 已完成
- Created: 2026-03-01
- Last: 2026-03-01

## 背景 / 问题陈述

- 当页面内容高度不足一屏时，页脚会跟随内容结束位置停在页面中部，导致页脚下方出现大块空白区域，视觉上不稳定。
- 期望行为：短页面时页脚贴在视口底部；长页面时页脚位于内容之后（需要滚动到底部才出现），不覆盖内容。

## 目标 / 非目标

### Goals

- 将 `AppLayout` 调整为标准的 sticky footer 布局：
  - 内容不足一屏：footer 贴底。
  - 内容超出一屏：footer 在视口外，滚动到底部才出现。
- 不影响现有 `header` 的 `sticky` 行为与更新横幅的 `sticky` 行为。

### Non-goals

- 不将 footer 改为 `fixed`（避免覆盖内容）。
- 不做 UI 视觉风格重构，不调整页面信息架构。
- 不改动 Rust 后端、SSE 协议、数据库或 API。

## 范围（Scope）

### In scope

- `web/src/components/AppLayout.tsx`
  - 外层容器改为纵向 flex，`main` 使用 `flex-1` 撑满剩余高度。
- （可选）新增轻量 Playwright E2E 回归：`web/tests/e2e/sticky-footer.spec.ts`

### Out of scope

- 其它页面布局重构或组件重排。
- 任何后端接口变更。

## 验收标准（Acceptance Criteria）

- Given 内容高度不足一屏（拉高浏览器窗口即可复现）
  When 打开任意主页面（`#/dashboard`、`#/stats`、`#/live`、`#/settings`）
  Then footer 应贴到视口底部（footer 下方不应再出现明显空白）。
- Given 内容高度超过视口（缩小窗口高度即可复现）
  When 页面可滚动
  Then footer 位于内容之后，初始不在视口内；滚动到底部才出现（不覆盖内容）。
- Given header 为 `sticky` 且更新横幅为 `sticky`
  When 页面滚动/窗口 resize
  Then 二者行为不回归。

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 新增本 Spec 并登记到 `docs/specs/README.md`。
- [x] M2: 完成 `AppLayout` sticky footer 布局修复。
- [x] M3: 通过 `web` 的 lint + build（必要时补充 E2E 回归）。
- [x] M4: PR ready（checks 状态明确），并在 Index Notes 记录 PR 号。

## 进度备注

- 2026-03-01: 完成 sticky footer 布局修复（`web/src/components/AppLayout.tsx`）：`app-shell` 增加 `flex flex-col`，`main` 增加 `flex-1 min-h-0`。
- 2026-03-01: 新增 Playwright E2E 回归：`web/tests/e2e/sticky-footer.spec.ts`（短页贴底 + 长页滚动到底部才出现）。
- 2026-03-01: 验证通过：`cd web && npm run lint -- --max-warnings=0`、`cd web && npx tsc -b`、`cd web && npm run build`。
- 2026-03-01: PR #65，CI Pipeline 与 Label Gate 均为 success。
