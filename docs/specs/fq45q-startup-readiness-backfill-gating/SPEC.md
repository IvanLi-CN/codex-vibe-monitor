# 启动就绪保护与历史回填解耦（#fq45q）

## 状态

- Status: 实现中（本地验证完成，待 shared-testbox / 101 rollout）

## 背景 / 问题陈述

- 101 上的 `ai-codex-vibe-monitor` 虽然容器进程处于 `running`，但本次只读排查确认其在 `2026-03-09 02:21:20 +08:00` 启动后，要到 `2026-03-09 02:27:07 +08:00` 才真正监听 `0.0.0.0:8080`，冷启动恢复耗时约 `346.5s`。
- 当前启动链路会在监听前同步执行多类历史 backfill，其中 `proxy reasoning effort` 与 `invocation failure classification` 在 101 量级数据上分别扫描 `174877` 与 `314111` 行；前者还会对大量已丢失的 raw 文件逐条告警，造成 `request raw file is unavailable` 约 `158161` 条日志噪音。
- 101 当前 Compose 未配置容器级 `healthcheck`，因此外层代理可能在应用尚未 ready 时就把流量转发进去；同时 retention cleanup 还未在 101 正式执行，数据体积与慢 SQL 会持续放大启动和查询成本。

## 目标 / 非目标

### Goals

- 让 HTTP 服务在关键初始化完成后尽快监听，并使 `GET /health` 在 101 等价数据集上于 `30s` 内返回成功。
- 将所有“仅补历史、不影响当前请求处理”的 startup backfill 改为后台有界执行，失败不阻断服务对外可用。
- 为历史 missing raw file / invalid JSON / 无收益 backfill 建立持久化跳过与降频机制，避免每次重启重复重扫。
- 为 Compose / Traefik 补齐 readiness 保护，并在 shared-testbox 与 101 rollout 中留下可复核证据。
- 固化 101 的 retention dry-run / live cleanup / `VACUUM` 执行顺序与验收口径，确保 cleanup 前后 totals 一致且性能指标可对比。

### Non-goals

- 不切换到非 SQLite 存储。
- 不增加 archived 明细在线查询 UI。
- 不改变现有业务域名、Traefik 拓扑与认证方式。
- 不在未通过 shared-testbox 验证前直接执行 101 live cleanup。

## 范围（Scope）

### In scope

- `src/main.rs` 的启动顺序、readiness 语义、startup phase timing、background backfill supervisor、backfill 持久化状态表与日志聚合。
- `Dockerfile`、`README.md`、`docs/deployment.md` 与 shared-testbox / 101 的部署卡片更新，用于接入容器 `healthcheck` 与 Traefik readiness 保护。
- shared-testbox 上的 101 等价数据验证，包含 DB 体积、`proxy_raw_payloads/` 状态、`time-to-health`、`summary?window=all` 一致性与重启窗口流量行为。
- 101 rollout 方案与远端维护记录：`dry-run -> live cleanup -> metrics diff -> manual VACUUM`。

### Out of scope

- archived 数据回灌在线查询。
- 归档文件格式调整。
- 慢 SQL 的全面索引治理（本次只覆盖与启动/cleanup 直接相关的收益验证）。

## 启动与回填设计

### Readiness / startup split

- 关键同步路径只保留：SQLite connect、schema 校验、价格目录加载、forward proxy runtime 加载、raw 目录准备、`AppState` 初始化与 HTTP listener bind。
- `GET /health` 改为 readiness probe：在关键同步路径完成前返回 `503`，完成后返回 `200 ok`；仅“进程活着”不再视为 ready。
- 启动日志必须至少输出 `db_connect`、`schema`、`http_ready` 的阶段耗时，并记录总 `time_to_health_ms`。

### Background backfill supervisor

- 以下任务从 startup 主路径移出，改为后台 supervisor 执行：`usage tokens`、`cost`、`promptCacheKey`、`requestedServiceTier`、`serviceTier`、`reasoningEffort`、`failureClassification`。
- supervisor 为每类任务维护持久化进度：`task_name`、`cursor_id`、`next_run_after`、`zero_update_streak`、`last_scanned`、`last_updated`、`last_status`。
- 单次 task 执行必须受边界约束：最多处理 `2000` 行或最多运行 `3s`，先达到者即结束本轮；backlog 未清空时短间隔续跑，连续零收益后自动降频到小时级。
- 回填失败只影响该 task 的下一轮调度与告警，不影响对外服务。

### 历史脏数据跳过策略

- 对 missing raw file、invalid JSON、missing target field 这三类“已判定不会补成”的记录，必须把 cursor 前推并记为 terminal skip，保证同一行后续重启不再被重复选中。
- 对暂时性 DB / I/O 错误，仅在当前 batch 内告警并保留重试资格，不把它错误标成 terminal skip。
- `failureClassification` 不再全表重扫，只针对“缺失/遗留默认值/不一致”的旧行增量处理；当长期 `updated=0` 时与其他 task 一样降频。

## 日志、部署与 rollout

### 日志与告警

- 启动期与后台回填期间，同类问题只允许输出聚合摘要日志，字段至少包括：`scanned`、`updated`、`skipped_*`、`elapsed_ms`、`next_run_after`。
- 每类摘要最多附带 `5` 个 `id/path` 样本，禁止逐条输出十万级 missing-file / invalid-json 告警。

### 部署保护

- 运行镜像需提供容器内 readiness probe 依赖（`curl`），Compose 为 `ai-codex-vibe-monitor` 增加 `healthcheck`，固定目标 `http://127.0.0.1:8080/health`。
- 101 侧默认以 Docker health 状态作为 Traefik 路由准入前提；若现场确认 Docker provider 启用了 `allowEmptyServices=true`，则额外配置 Traefik service-level active healthcheck 兜底。
- shared-testbox 与 101 都必须验证“容器重启后的外部访问不会提前命中未 ready 实例”。

### 101 rollout gate

- 共享测试环境必须先复制 101 等价 SQLite 与 `proxy_raw_payloads/` 文件状态，再验证 startup/readiness 与 cleanup；若只复制 DB 不复制 raw 文件，则 readiness 验收不通过。
- 101 首次执行顺序固定为：`dry-run`、核对 archive / totals / 体积、维护窗口 `live cleanup`、对比启动时长与慢 SQL、空间允许时人工执行一次 `VACUUM`。
- cleanup 后必须保留 before/after 证据：数据库体积、`summary?window=all` totals、`time_to_health`、慢 SQL 计数/耗时、archive batch 文件清单。

## 验收标准（Acceptance Criteria）

- 在 101 等价数据集上，服务从进程启动到 `GET /health` 返回 `200 ok` 不超过 `30s`。
- 重启窗口内，通过 Traefik 的外部请求不会被转发到返回 `503 starting` 的实例。
- 启动日志不再出现十万级 raw file 缺失逐条告警；每类问题只有聚合摘要与有限样本。
- 历史 missing raw file / invalid JSON / missing field 记录在首次 terminal skip 后，不会在后续启动中继续被同类 backfill 重扫。
- `failureClassification`、`reasoningEffort` 等后台 task 在长期 `updated=0` 时会自动降频，而不是每次重启都全量扫描。
- 101 的 Compose / rollout 记录中明确存在 `healthcheck`、shared-testbox 验证记录、101 `dry-run` / `live cleanup` / `VACUUM` 记录，以及 cleanup 前后 `summary?window=all` totals 一致证据。

## Task Orchestration

- wave: 1
  - main-agent => 固化本次启动/readiness/backfill/rollout 规格与验收口径，并登记索引 (skill: $fast-flow + $docs-plan)
- wave: 2
  - main-agent => 重构后端启动顺序、`/health` readiness 语义、后台 backfill supervisor 与持久化进度表 (skill: $fast-flow)
- wave: 3
  - main-agent => 补齐回归测试、日志聚合、镜像/部署文档与 shared-testbox readiness 验证脚本 (skill: $fast-flow)
- wave: 4
  - main-agent => push 分支、创建 PR、收敛 checks/review，并在共享测试与 101 维护窗口完成 rollout 记录闭环 (skill: $codex-review-loop + $fast-flow)

## 参考

- `src/main.rs`
- `Dockerfile`
- `README.md`
- `docs/deployment.md`
- `docs/specs/9aucy-db-retention-archive/SPEC.md`
