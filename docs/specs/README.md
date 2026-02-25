# 规格（Spec）总览

本目录用于管理工作项的规格与追踪：记录范围、验收标准、任务清单与状态，作为交付依据；实现与验证应以对应 `SPEC.md` 为准。

> Legacy compatibility: historical plans remain in `docs/plan/**/PLAN.md`. New entries are created in `docs/specs/**/SPEC.md`.

## Index

| ID    | Title                                                               | Status | Spec                                         | Last       | Notes                |
| ----- | ------------------------------------------------------------------- | ------ | -------------------------------------------- | ---------- | -------------------- |
| 26knq | 修复 InvocationTable 异常横向滚动并补 E2E 回归                      | 已完成 | `26knq-invocation-table-overflow/SPEC.md`    | 2026-02-26 | PR #56 / fast-track  |
| 5932d | SSE 驱动的请求记录与统计实时更新                                    | 已完成 | `5932d-sse-proxy-live-sync/SPEC.md`          | 2026-02-25 | PR #52               |
| jpg66 | 设置页切换为 shadcn 风格并优化亮/暗主题可读性                       | 已完成 | `jpg66-settings-shadcn-refresh/SPEC.md`      | 2026-02-25 | 已完成并通过视觉确认 |
| q86c7 | 接入 ui-ux-pro-max（Codex）并修正 .gitignore 追踪策略               | 已完成 | `q86c7-setup-uipro-codex/SPEC.md`            | 2026-02-24 | PR #50               |
| gwpsb | 线上失败请求分类治理与可观测性增强                                  | 已完成 | `gwpsb-proxy-failure-hardening/SPEC.md`      | 2026-02-24 | PR #51               |
| z9h7v | 请求日志可观测性增强（IP / Cache Tokens / 分阶段耗时 / Session ID） | 已完成 | `z9h7v-invocation-log-observability/SPEC.md` | 2026-02-25 | local                |
