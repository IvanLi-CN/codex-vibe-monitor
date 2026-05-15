# 规格（Spec）总览

本目录只保存 active topic-level specs。历史任务规格已原样移至 `docs/archive/specs/`；legacy plans have been removed after migration into active topic specs.

## Index

| ID    | Title                                                                     | Lifecycle | Spec                                               | Implementation                                               | Notes                                             |
| ----- | ------------------------------------------------------------------------- | --------- | -------------------------------------------------- | ------------------------------------------------------------ | ------------------------------------------------- |
| 5932d | SSE 驱动的请求记录与统计实时更新                                          | active    | `5932d-sse-proxy-live-sync/SPEC.md`                | `5932d-sse-proxy-live-sync/IMPLEMENTATION.md`                | topic anchor: stats / dashboard / live sync       |
| 8239m | Release `latest` 仅指向最新已发布 stable                                  | active    | `8239m-release-latest-published-stable/SPEC.md`    | `8239m-release-latest-published-stable/IMPLEMENTATION.md`    | topic anchor: release / ci delivery               |
| 9aucy | 数据分层保留、离线归档与长周期汇总                                        | active    | `9aucy-db-retention-archive/SPEC.md`               | `9aucy-db-retention-archive/IMPLEMENTATION.md`               | topic anchor: storage / retention / archive       |
| pd77h | OAuth 数据面内联合并                                                      | active    | `pd77h-oauth-inline-adapter/SPEC.md`               | `pd77h-oauth-inline-adapter/IMPLEMENTATION.md`               | topic anchor: oauth / account management          |
| prk6j | KaisouMail OAuth 邮箱适配                                                 | active    | `prk6j-kaisoumail-oauth-mailbox-adapter/SPEC.md`   | `prk6j-kaisoumail-oauth-mailbox-adapter/IMPLEMENTATION.md`   | topic anchor: oauth / temp mailbox / integrations |
| q8h3n | 代理热路径并发稳定性与传输背压收口                                        | active    | `q8h3n-proxy-hot-path-streaming-stability/SPEC.md` | `q8h3n-proxy-hot-path-streaming-stability/IMPLEMENTATION.md` | topic anchor: proxy / upstream / pool runtime     |
| quhzx | 建立全局 UI 规范文档体系                                                  | active    | `quhzx-ui-guidelines-system/SPEC.md`               | `quhzx-ui-guidelines-system/IMPLEMENTATION.md`               | topic anchor: ui system / design standards        |
| tr4ev | Bun-first 工具链收敛                                                      | active    | `tr4ev-bun-first-toolchain/SPEC.md`                | `tr4ev-bun-first-toolchain/IMPLEMENTATION.md`                | topic anchor: developer tooling / repo structure  |
| v7se4 | Worktree bootstrap 与显式依赖初始化                                       | active    | `v7se4-worktree-bootstrap/SPEC.md`                 | `v7se4-worktree-bootstrap/IMPLEMENTATION.md`                 | topic anchor: developer tooling / linked worktree |
| z9h7v | 请求日志可观测性增强（IP / Cache Tokens / 分阶段耗时 / Prompt Cache Key） | active    | `z9h7v-invocation-log-observability/SPEC.md`       | `z9h7v-invocation-log-observability/IMPLEMENTATION.md`       | topic anchor: records / invocations observability |

## Archived Sources

- Historical specs: `docs/archive/specs/`
- Legacy plans: removed after migration
