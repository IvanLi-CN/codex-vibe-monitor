# 统计按浏览器时区自然日（#0004）

## 状态

- Status: 已完成
- Created: 2026-02-20
- Last: 2026-02-20

## 问题陈述

当前统计“按天”的分桶与窗口边界在后端默认使用 UTC 00:00，对东八区等时区会表现为 08:00~次日 08:00，不符合自然日（00:00~次日 00:00）的直觉；同时数据库内 `codex_invocations.occurred_at` 存储为 Asia/Shanghai 的 naive 字符串，而后端在 SQL 下界绑定上使用 naive UTC，会导致窗口下界错位（通常差 8 小时）。

## 目标 / 非目标

### Goals

- 统计口径使用“浏览器实际时区”（IANA TZ，例如 `Asia/Shanghai`、`America/Los_Angeles`）。
- `today/thisWeek/thisMonth` 的窗口边界按该时区计算（本地 00:00 边界）。
- 当 `bucket=1d` 时，按该时区做严格自然日分桶（DST 切换周也正确，允许出现 23/25 小时日桶）。
- 修复所有基于 `occurred_at` 的 SQL 过滤下界绑定，避免因存储为 Shanghai naive 而造成错位。

### Non-goals

- 不改变 `bucket < 1d` 的等长桶语义（仍按秒数对齐；仅窗口起点对齐到 reporting tz 的 named range）。
- 不对历史数据做迁移/回填（保持现有 DB 表结构）。
- 不引入复杂的前端时区库（以浏览器原生时区能力为主）。

## 范围（Scope）

### In scope

- 统计类 API（summary/timeseries/errors）支持 query 参数 `timeZone`（camelCase），用于指定 reporting tz。
- 后端 named ranges（today/thisWeek/thisMonth）按 `timeZone` 计算窗口边界。
- 后端 `bucket=1d` 时按 `timeZone` 计算自然日分桶，并输出每桶的 UTC RFC3339 start/end。
- 前端从浏览器读取 `Intl.DateTimeFormat().resolvedOptions().timeZone` 并默认附带到上述请求。
- Dashboard 的使用活动日历与 Stats 页的相关统计口径保持一致。

### Out of scope

- 新增用户可选时区的设置页（默认使用浏览器时区）。
- 变更 SSE 协议结构（允许前端对 `bucket=1d` 走 resync，而非本地增量合并）。

## 验收标准（Acceptance Criteria）

- Given 浏览器时区为任意 IANA TZ
  When 打开 Dashboard 的“使用活动”日历
  Then 每个格子对应该时区的自然日（00:00~次日 00:00），不再按 UTC 日界偏移。
- Given Stats 页选择 `today/thisWeek/thisMonth`
  When 请求 summary/timeseries/errors
  Then 窗口起点/终点按该时区的自然日/自然周/自然月边界计算（截至当前时刻）。
- Given `bucket=1d`
  When 统计跨越 DST 切换日
  Then 记录仍归属到正确的本地日期桶，且桶边界以本地 00:00 为准（UTC start/end 可能不相差 86400 秒）。
- Given 数据库存储 `occurred_at` 为 Shanghai naive 字符串
  When 后端执行任何 `occurred_at >= ?` 的时间窗过滤
  Then 下界绑定与解析一致，不再出现系统性 8 小时错位。

## 里程碑（Milestones）

- [x] M1: 后端引入 `timeZone` 参数并修复 `occurred_at` 下界绑定。
- [x] M2: 实现 `bucket=1d` 的严格自然日分桶与 DST 覆盖。
- [x] M3: 前端默认附带浏览器时区，并修复日历视图的 off-by-one 与展示口径提示。

## 交付（Delivery）

- PR: #38
