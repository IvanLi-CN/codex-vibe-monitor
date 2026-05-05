# Archived Spec Sources

This directory stores historical spec directories moved out of the active `docs/specs` taxonomy. Contents are preserved from the original spec sources for traceability; active topic-level specs live in `docs/specs/`.

| Directory                                                     | Title                                                                         |
| ------------------------------------------------------------- | ----------------------------------------------------------------------------- |
| `26knq-invocation-table-overflow/`                            | 修复 InvocationTable 异常横向滚动                                             |
| `2qsev-dashboard-tpm-cost-per-minute-kpi/`                    | Dashboard 今日 KPI 上下文统计卡片                                             |
| `2uaxk-remove-xyai-legacy-ingest/`                            | 移除 XYAI 采集，保留历史读取                                                  |
| `3gvtt-records-request-id-response-details/`                  | `/records` 请求 ID 筛选与异常响应详情抽屉                                     |
| `3n287-oauth-temp-mail-automation/`                           | OAuth 临时邮箱自动化与验证码/邀请态集成                                       |
| `3np57-group-bound-proxy-real-traffic-stats/`                 | 绑定代理节点改为“本组真实流量统计”                                            |
| `3qfhk-pool-send-phase-orphan-recovery/`                      | 修复 pool send-phase 孤儿请求长期挂起                                         |
| `3vfxp-invocation-endpoint-request-badges/`                   | InvocationTable 请求类型 Badge 化                                             |
| `3vm5e-live-prompt-cache-call-record-expansion/`              | Live Prompt Cache 调用记录展开与历史抽屉                                      |
| `3xaa3-stats-trend-mode-switch/`                              | 统计页趋势图按点数切换柱状 / 累积面积模式                                     |
| `43bpp-upstream-account-window-actual-usage/`                 | 号池窗口实际使用量与 Token 悬浮详情                                           |
| `47ran-pool-models-override-gpt55-pricing/`                   | 当前 pool `/v1/models` 路径级覆盖与 GPT-5.5 默认价格刷新                      |
| `4kkpp-live-prompt-cache-conversations/`                      | Live 对话统计（按 Prompt Cache Key）— 无统计表方案                            |
| `4reae-compact-502-traceability-dynamic-pool-timeouts/`       | Compact 502 可追踪性与号池动态超时                                            |
| `5gqdb-invocation-proxy-name-truncation-hotfix/`              | InvocationTable 桌面代理名省略回归热修                                        |
| `5uxj8-pool-live-routing-score-followup/`                     | pool `/v1/*` live 路由显式综合打分 follow-up                                  |
| `5v5yf-relay-fast-billing-recognition/`                       | 通用 API Keys 计费层级修复                                                    |
| `62uhg-upstream-account-roster-compact/`                      | 上游账号列表紧凑化改版                                                        |
| `667ae-pool-no-available-account-bounded-wait/`               | 号池暂时无号时的 10 秒有界等待与 503 终态                                     |
| `67acu-update-banner-readability/`                            | 修复更新提示可读性                                                            |
| `6b9ra-upstream-account-group-node-shunt/`                    | 上游账号分组节点分流策略                                                      |
| `6whgx-records-stable-snapshot-analytics/`                    | 请求记录分析页：稳定快照 + 聚焦分析                                           |
| `7272y-gpt-5-4-pricing/`                                      | 内置 GPT-5.4 系列计费规则与下游模型列表                                       |
| `7gb5w-account-pool-bound-proxy-dialog-freshness/`            | 号池分组设置弹窗“绑定代理节点”目录加载与同步热修                              |
| `7n2ex-invocation-account-latency-drawer/`                    | InvocationTable 账号归因、时延压缩展示与当前页账号抽屉                        |
| `7s4kw-dashboard-usage-activity-overview/`                    | Dashboard：把“历史”并入“活动总览”，并将“今日统计信息”改为单行 KPI             |
| `7y5yf-live-prompt-cache-upstream-summary-columns/`           | Live Prompt Cache 对话表改成“上游账号 / 总计”双列复合展示                     |
| `8dun3-stats-success-failure-ttfb/`                           | 统计页成功/失败图增加首字耗时折线与悬浮统计                                   |
| `8pjnh-records-filter-dropdown-overlap-fix/`                  | 请求记录筛选下拉遮挡修复                                                      |
| `8shg4-dashboard-tpm-whole-number-hotfix/`                    | Dashboard TPM 整数显示热修（#8shg4)                                           |
| `96qgn-oauth-mixed-plan-duplicate-warning/`                   | 不同计划 OAuth 账号共存时取消重复 warning                                     |
| `9anzf-bundle-xray-in-image/`                                 | Bundle Xray-core in Docker image for forward proxy subscriptions              |
| `9mbsz-release-docker-smoke-gate/`                            | Release 前 Docker Smoke Gate                                                  |
| `9r45m-ci-pr-critical-path-optimization/`                     | PR CI 关键路径提速                                                            |
| `9vau7-backend-structure-dual-pr-followup/`                   | 后端结构债双 PR 快车道                                                        |
| `a6pm6-dashboard-activity-trend-chart/`                       | Dashboard 活动总览趋势图增强                                                  |
| `ask3x-proxy-timeout-defaults/`                               | 反向代理默认超时口径统一为 60s / 180s                                         |
| `aucd3-compact-first-chunk-timeout/`                          | 号池 compact 首 chunk 超时口径对齐                                            |
| `ay33j-stats-read-path-lock-elimination/`                     | Dashboard / stats 读链路 SQLite 锁冲突治理                                    |
| `b3n4q-archive-manifest-dedupe-fix/`                          | 修复 archive manifest 重复账号唯一键冲突                                      |
| `ben7x-pool-upstream-413-retry/`                              | 号池上游 413 原账号补试与切号                                                 |
| `bh43j-database-path-raw-dir-anchor/`                         | 数据库环境变量重命名与 raw 路径锚点修复                                       |
| `bk2pt-responses-overload-early-route-retry/`                 | Responses-family `server_is_overloaded` 早期重试与分层换路由收口              |
| `bnjhy-upstream-tag-fast-mode/`                               | 上游账号 Tag Fast 模式服务层改造                                              |
| `br38t-responses-overload-preview-cap-followup/`              | Responses overload early gate preview-cap follow-up                           |
| `c58kc-live-forward-proxy-table/`                             | 实况页新增“代理”统计表与 24h 成败示意图                                       |
| `c5yag-subscription-validation-timeout-60/`                   | Extend subscription validation timeout to 60s (keep single-proxy at 5s)       |
| `ca7v4-batch-oauth-action-popover/`                           | 批量 OAuth 主动作合并与气泡重生成                                             |
| `cg6um-upstream-account-detail-records-sticky-conversations/` | 上游账号详情调用记录与 Sticky 对话对齐 Live 交互                              |
| `cgz7s-oauth-token-json-import/`                              | OAuth 凭据 JSON 批量导入与验活                                                |
| `cng8a-pool-temporary-failure-degraded-admission/`            | 号池临时故障“工作降级”与新对话准入收口                                        |
| `dvwja-proxy-fast-mode-request-rewrite/`                      | 反向代理 Fast 模式请求改写（三态设置，`requestedServiceTier`=上游实际请求值） |
| `dzbnx-dashboard-activity-overview-merge/`                    | Dashboard：合并 24h / 7d 活动总览卡片                                         |
| `e5w9m-batch-oauth-mailbox-popover-edit/`                     | 批量 OAuth 邮箱气泡编辑与邮箱动作解耦                                         |
| `e6082-prompt-cache-chart-window-24h-cap/`                    | Prompt Cache 图表时间轴 24 小时封顶热修                                       |
| `enzf8-upstream-account-roster-pagination-bulk-actions/`      | 上游账号列表分页、跨页选择与批量操作                                          |
| `erv4p-legacy-http200-success-like-retention/`                | 修复 legacy `http_200` success-like retention 漏清理                          |
| `f3dx3-parallel-work-bucket-stats/`                           | 并行工作 bucket 统计                                                          |
| `f6f6e-gh-actions-release-anti-cancel/`                       | GH Actions 防取消发布链路全面对齐                                             |
| `f7nqn-invocation-proxy-display-restore-hotfix/`              | InvocationTable 代理节点展示热修（#f7nqn)                                     |
| `fffqk-oauth-manual-mailbox-domain-autocreate/`               | OAuth 手动邮箱域名识别与缺失即创建                                            |
| `fmfuf-graceful-shutdown-hardening/`                          | 优雅停机补强                                                                  |
| `fq45q-startup-readiness-backfill-gating/`                    | 启动就绪保护与历史回填解耦                                                    |
| `g3amk-codex-remote-compact-observability/`                   | Codex 远程压缩请求记录、展示与计费接入                                        |
| `g4e6a-oauth-mailbox-multilingual-recognition/`               | OAuth 邮件多语言验证码与邀请识别                                              |
| `g4ek6-account-pool-upstream-accounts/`                       | 号池模块第一阶段：上游账号管理                                                |
| `g7n33-backend-archive-structure-convergence/`                | 后端 archive 结构收敛                                                         |
| `g8mfs-giant-source-structure-convergence/`                   | 巨型源码结构收敛                                                              |
| `gkser-oauth-responses-large-body-passthrough/`               | OAuth `/v1/responses` 大包体直通与 distinct-account 记账修复                  |
| `gmycv-overlay-prompt-frosting-consistency/`                  | 浮层提示磨砂隔离一致性修复                                                    |
| `gp92q-upstream-account-required-group-proxy/`                | 上游账号强制分组代理约束                                                      |
| `gwpsb-proxy-failure-hardening/`                              | 线上失败请求分类治理与可观测性增强                                            |
| `h4p2x-pool-upstream-429-immediate-failover/`                 | 号池上游 429 立即切号与终态 429                                               |
| `h5k2r-segmented-control-family/`                             | 全站 segmented control family 统一与 Dashboard 样式修复                       |
| `h9r2m-permanent-online-hourly-rollups/`                      | Permanent online hourly stats retention                                       |
| `hbqe3-invocation-reasoning-effort-badge-colors/`             | InvocationTable 推理强度徽标色阶优化                                          |
| `hrvtt-invocation-proxy-weight-delta/`                        | 请求详情补齐代理信息与本次权重变化                                            |
| `huzqt-frontend-structure-convergence-followup/`              | 前端结构收敛 follow-up                                                        |
| `j5x9m-upstream-account-persisted-group-catalog/`             | 上游账号分组持久化目录                                                        |
| `j86ms-oauth-pending-session-live-metadata/`                  | 修复新增账号页 OAuth 地址被字段编辑重置                                       |
| `j9frr-docs-site-storybook-pages/`                            | 文档站与 Storybook GitHub Pages 同构发布                                      |
| `jg7a5-raw-payload-cold-compression-search/`                  | raw 负载冷压缩与磁盘全文搜索                                                  |
| `jk3hm-live-chart-hover-tooltips/`                            | Live 小图表悬浮详情统一升级                                                   |
| `jm3hb-oauth-installation-id-rewrite/`                        | OAuth 上游 `x-codex-installation-id` 代理侧稳定改写                           |
| `jpg66-settings-shadcn-refresh/`                              | 设置页切换为 shadcn 风格并优化亮/暗主题可读性                                 |
| `jpvwj-account-pool-tiered-maintenance/`                      | 号池分层同步高级设置与前 100 溢出低频更新                                     |
| `js2gr-oauth-quota-exhausted-rate-limit-status/`              | OAuth 配额耗尽账号误标为上游拒绝修复                                          |
| `k2z9h-pool-account-hard-failure-audit/`                      | 号池硬失效账号淘汰与账号动作审计可视化                                        |
| `k52tw-forward-proxy-validation-allow-404/`                   | Forward proxy validation allows 404 as reachable                              |
| `k7kpk-bundle-icons-locally/`                                 | 前端运行时图标内置打包                                                        |
| `k8a4r-pool-group-upstream-429-retry/`                        | 号池分组级上游 429 重试与随机回退                                             |
| `kfgvy-upstream-http-402-upstream-rejected/`                  | 402 `deactivated_workspace` 账号状态改判为上游拒绝                            |
| `krsd4-main-rs-structure-refactor/`                           | `main.rs` 结构化拆分与基线同步重构                                            |
| `kwmjr-dashboard-tab-crash-hardening/`                        | Dashboard 长驻崩溃：working-conversations 泄露与 today 面板重渲染硬化         |
| `m2f8k-pool-upstream-attempt-observability/`                  | 号池逐次上游尝试明细、三账号 failover 上限与 7+30 保留                        |
| `m4c2q-prompt-cache-conversation-filter-window/`              | Prompt Cache Key 对话筛选增强与动态时间轴                                     |
| `m4k2q-upstream-account-unavailable-work-status/`             | 号池工作状态新增“不可用（不可调度）”                                          |
| `m7a9k-oauth-manual-mailbox-attach/`                          | OAuth 手动邮箱附着与增强能力判定                                              |
| `m96jw-subscription-validation-xray-runtime-dir/`             | 修复订阅验证路径下 xray 运行目录缺失导致添加失败                              |
| `mbnns-dashboard-working-conversations-wide-4col/`            | Dashboard 工作中对话卡片：1660 宽屏四栏 follow-up                             |
| `mj5nt-live-running-elapsed-sse/`                             | 请求实况即时展示与“用时”订正                                                  |
| `mpgea-dashboard-yesterday-activity-overview/`                | Dashboard 活动总览增加“昨日”页签                                              |
| `mww8f-pool-bound-forward-proxy-routing/`                     | 移除直连反向代理并为号池接入分组绑定正向代理                                  |
| `n78zb-backend-prompt-cache-conversations-structure/`         | 后端 prompt-cache conversations 结构收敛                                      |
| `n7c2r-proxy-hot-path-no-raw-reread/`                         | 代理热路径停止 response raw 二次回读                                          |
| `nepye-sync-node-shunt-nonexclusive-recovery/`                | node shunt 同步路径共享绑定节点恢复                                           |
| `ngwdu-owner-facing-node-health-real-attempts/`               | Owner-facing 节点健康统一为真实节点尝试口径                                   |
| `nkqe3-release-failure-telegram-alerts/`                      | Release 失败 Telegram 告警接入                                                |
| `nm7ep-daily-timeseries-rollup-continuity/`                   | Daily timeseries archive continuity and subday bucket guard                   |
| `p3u4s-stats-select-shadcn-24h-bucket/`                       | 统计页选择器切换为 shadcn 并补齐最近 7 天的 24 小时粒度                       |
| `p4y7m-upstream-team-shared-org-auto-mother/`                 | 共享 Team 组织账号去重修正                                                    |
| `p6x4r-pool-responses-per-account-retry-budget/`              | 号池 `/v1/responses*` 临时失败改为“每个当前账号先重试再切号”                  |
| `phb37-backend-structure-convergence-followup/`               | 后端结构收敛 follow-up                                                        |
| `ppt8w-pool-usage-limit-hard-stop-recovery-gate/`             | 号池 usage-limit 429 硬失效与恢复门控补洞                                     |
| `pqqpf-selectfield-simple-dropdown-rollout/`                  | 全站简单下拉统一为 `SelectField`                                              |
| `q1m9k-release-pr-comment-permission/`                        | Release PR 评论权限补齐                                                       |
| `q6mys-external-upstream-api-keys/`                           | 第三方上游账号开放 API 与 APIKey 管理                                         |
| `q86c7-setup-uipro-codex/`                                    | 接入 ui-ux-pro-max（Codex）并修正 .gitignore 追踪策略                         |
| `q8vxs-account-pool-groups-tab/`                              | 号池新增“分组”子页页签与分组总览页                                            |
| `qdyfv-account-detail-drawer-tabs/`                           | 账号详情抽屉统一关闭语义与 Tabs 分组                                          |
| `qm8r2-readme-current-truth-refresh/`                         | README 当前真相重写与展示刷新                                                 |
| `qz42n-sqlite-write-backpressure/`                            | SQLite 写入可靠性与后台背压                                                   |
| `r4m6v-dashboard-working-conversations-invocation-drawer/`    | Dashboard 工作中对话调用详情抽屉                                              |
| `r5a8k-oauth-sync-retry-terminal-state/`                      | OAuth 同步 refresh 后 retry 失败残留 `syncing` 修复                           |
| `r7o2q-oauth-api-scope-reauth-hardening/`                     | OAuth 池账号 API Scope 与重授权误判修复                                       |
| `r8m3k-invocation-table-responsive-no-overflow/`              | InvocationTable 响应式修复：lg+ 无横向滚动、sm 及以下列表化                   |
| `r99mz-dashboard-today-activity-overview/`                    | Dashboard：把“今日”并入“活动总览”，并为今日新增分钟级柱状 / 累计面积图        |
| `r9v97-batch-oauth-generate-autocopy/`                        | 批量 OAuth 生成后自动复制                                                     |
| `rkc7k-live-summary-flicker-fix/`                             | 修复 Live 实时统计闪烁与数字滚动被打断                                        |
| `rupn7-invocation-table-reasoning-effort/`                    | InvocationTable 推理强度与详情 reasoningTokens                                |
| `rw32e-invocation-fast-mode-indicator/`                       | 请求列表 Fast 模式标识（service tier 版）                                     |
| `rzxey-dashboard-usage-calendar-skeleton-shift/`              | Dashboard：修复 UsageCalendar 加载骨架右偏 + 首行骨架按真实两卡布局           |
| `s6d1q-immutable-invocation-archive-segments/`                | Immutable invocation archive segments                                         |
| `s8d2w-dashboard-today-stats-bento/`                          | Dashboard：将“配额概览”替换为“今日统计信息                                    |
| `s8zhn-dashboard-working-conversations-header-compact/`       | Dashboard 工作中对话卡片头部压缩                                              |
| `s9k3m-upstream-account-status-filter-multiselect/`           | 上游账号列表三组状态筛选改为多选                                              |
| `sbacc-storybook-accessibility/`                              | Storybook Accessibility Gate                                                  |
| `sq8gw-release-pr-version-comment/`                           | Release 工作流 PR 版本评论                                                    |
| `suuez-dashboard-working-conversations-virtual-scroll/`       | Dashboard 工作中对话无限列表、虚拟滚动与增量同步                              |
| `swze7-account-pool-email-name-plan-badge/`                   | 号池账号邮箱 / 名称联动 / mixed-plan 同名放宽 / OAuth 计划徽章优化            |
| `sy7a9-upstream-account-grouped-roster/`                      | 上游账号列表分组视图与代理徽章                                                |
| `t3rdp-account-pool-tag-routing/`                             | 号池 Tag 路由与管理扩展                                                       |
| `t4v9k-retention-backlog-root-cause-fix/`                     | Retention backlog root-cause fix                                              |
| `t7m4h-live-proxy-weight-trend/`                              | Live 代理运行态：新增 24h 权重趋势列与断点适配                                |
| `t9m3p-pool-responses-timeout-guardrails/`                    | 号池 `/v1/responses*` 超时护栏收口为 `180s / 300s`                            |
| `t9wwm-upstream-account-detail-url-state/`                    | 上游账号详情改为 URL / ID 驱动并跨页面统一                                    |
| `thyxm-upstream-account-group-notes/`                         | 上游账号分组共享备注                                                          |
| `tjgyj-records-remove-proxy-filter/`                          | `/records` 移除代理筛选                                                       |
| `ts4zf-rename-remaining-xy-envs/`                             | 修正剩余 `XY_*` 环境变量命名                                                  |
| `ts6qp-account-pool-typing-debt/`                             | account-pool 前端类型债清偿                                                   |
| `u8j4n-fixed-oauth-bridge-sidecar/`                           | 固定 OAuth Bridge Sidecar 方案                                                |
| `uehbv-gpt55-unsupported-account-tag/`                        | GPT-5.5 Unsupported Account Tag                                               |
| `uhn89-upstream-roster-latency/`                              | 上游账号列表 100ms / 10ms 延迟治理                                            |
| `uwke5-proxy-upstream-429-retry/`                             | 反向代理上游 429 自动重试（设置可配）                                         |
| `v5qtm-live-prompt-cache-sse-sync/`                           | Live Prompt Cache 调用记录同源实时同步                                        |
| `v6epa-api-key-upstream-base-url/`                            | API Key 账号上游地址支持                                                      |
| `v7se4-worktree-bootstrap/`                                   | Worktree bootstrap 同步开发环境配置                                           |
| `v8y2p-prevent-routing-exhausted-accounts-race/`              | 修复额度耗尽账号仍被路由与并发误恢复                                          |
| `vdukd-ghcr-glibc-drift-fix/`                                 | Fix GHCR image GLIBC drift (Debian bookworm runtime)                          |
| `vn2e9-wide-shell-1660/`                                      | 全站 1660 宽屏壳层适配                                                        |
| `vw93e-raw-born-gzip-rollup-followup/`                        | raw 保真降本与历史维护追平 follow-up                                          |
| `w3t3w-dashboard-working-conversations-cards/`                | Dashboard：工作中对话卡片替换                                                 |
| `w5s2x-openai-websocket-proxy/`                               | OpenAI 兼容 WebSocket 代理                                                    |
| `w8seb-oauth-import-paste-minimal-validation/`                | OAuth 导入最小校验修复与单条粘贴入列                                          |
| `wjowd-main-branch-protection/`                               | `main` 主干保护禁止直推与 PR 全检查必过                                       |
| `wt76b-backend-structure-convergence/`                        | 后端优先源码结构收敛                                                          |
| `wtwsn-ghcr-multiarch-release-manifest/`                      | GHCR 发布切换多架构 manifest（amd64 + arm64）                                 |
| `wv3m7-forward-proxy-bootstrap-probe/`                        | Forward Proxy 新增后异步首轮探测补齐                                          |
| `ww6et-requested-fast-intel-neutral-bolt/`                    | 请求侧 Fast 情报与中性闪电标识                                                |
| `x2s4h-stats-first-response-byte-total-p95/`                  | 统计页成功/失败图改为首字总耗时 P95                                           |
| `xvdhm-dashboard-sse-refresh-optimization/`                   | Dashboard SSE 更新链路优化                                                    |
| `y5st2-live-prompt-cache-selection-persistence/`              | Live 页 Prompt Cache 对话筛选本地记忆                                         |
| `yf3s3-running-invocation-durable-persistence/`               | 运行中调用主记录实时落库与中断恢复修复                                        |
| `ykn4w-pricing-alias-backfill/`                               | 日期后缀模型成本回退与历史空成本补算                                          |
| `ynr8z-pool-stream-total-timeout/`                            | 号池流式上游误用整请求超时                                                    |
| `yxdy4-upstream-account-roster-filter-persistence/`           | 上游账号列表筛选前端持久化                                                    |
| `zanzr-release-arm64-native-runner/`                          | Release 构建加速：arm64 迁移到 GitHub-hosted ARM runner                       |
| `zrxcd-sticky-footer-layout/`                                 | Sticky Footer 修复：页脚在短页面贴底                                          |
