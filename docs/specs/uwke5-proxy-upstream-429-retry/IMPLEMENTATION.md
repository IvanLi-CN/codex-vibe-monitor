# 反向代理上游 429 自动重试（设置可配） - Implementation

## Current State

- Canonical spec: `docs/specs/uwke5-proxy-upstream-429-retry/SPEC.md`
- Implementation summary: 已完成（5/5）

## Migrated Implementation Notes

## 状态

- Status: 已完成（5/5）
- Created: 2026-03-10
- Last: 2026-03-10

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Rust tests：settings roundtrip、legacy schema migration default、capture-target retry success/exhaustion、generic proxy body replay、`/v1/models` merge retry、`Retry-After` 解析/backoff。
- Front-end tests：API normalize/update payload、Storybook/mock 设置 roundtrip。
- Browser smoke：真实浏览器打开 settings 页面，确认新控件可保存且页面会话保持打开供复查。
