# 修正剩余 `XY_*` 环境变量命名（#ts4zf）

## 状态

- Status: 待实现
- Created: 2026-03-10
- Last: 2026-03-10
- Supersedes: `docs/specs/2uaxk-remove-xyai-legacy-ingest/SPEC.md` 中“仍然通用的 `XY_*` 配置键不重命名”的旧非目标；该 supersede 仅限公开环境变量命名，不恢复任何 XYAI 采集逻辑。

## 背景 / 问题陈述

- 仓库在移除 XYAI 新写入后，仍保留了一组对外公开且运行时继续接受的 `XY_*` 环境变量；它们已经不再对应当前产品边界，却仍然是部署文档、Docker 默认值和配置解析的 canonical 名称。
- 这种“功能已去 XYAI 化、公开配置仍带 `XY_` 前缀”的状态会误导部署者，以为这些配置仍和历史 XYAI 功能有关，也让 `DATABASE_PATH` 的中性化命名显得不完整。
- 主人已明确要求对全部剩余公开 `XY_*` runtime env 做一次性 hard cutover：新 canonical 名称统一改成裸名中性键；旧键不做 fallback，不 silently ignore，而是在启动期直接报出一对一 rename 指引。

## 目标 / 非目标

### Goals

- 把所有仍被运行时接受的公开 `XY_*` 环境变量切换为裸名中性的 canonical 命名。
- 旧 `XY_*` 键一旦出现在环境中就 fail-fast，并统一输出 `rename <old> to <new>` 风格的迁移错误。
- 保持默认值、运行行为、数据保留策略、proxy / CRS / retention / xray 语义不变；变化仅限公开配置命名与报错口径。
- README、部署文档、Docker 运行时默认 env、测试样例与用户可见注释只发布新名字；旧名字只出现在专门 migration 说明里。

### Non-goals

- 不回退或恢复任何 XYAI 采集、quota 拉取或 scheduler 写入逻辑。
- 不修改历史数据库中的 `source='xy'`、历史 `xy` 聚合口径或 quota snapshot 读取语义。
- 不重命名已经中性的公开键：`DATABASE_PATH`、`OPENAI_UPSTREAM_BASE_URL`、`OPENAI_PROXY_*`、`PROXY_RAW_*`、`PROXY_ENFORCE_STREAM_INCLUDE_USAGE`、`PROXY_USAGE_BACKFILL_ON_STARTUP`、`FORWARD_PROXY_ALGO`、`CRS_STATS_*`。
- 不把旧键保留为兼容别名，不提供过渡期双读。

## 需求（Requirements）

### MUST

- General/runtime 组 canonical 名称改为：
  - `POLL_INTERVAL_SECS`
  - `REQUEST_TIMEOUT_SECS`
  - `MAX_PARALLEL_POLLS`
  - `SHARED_CONNECTION_PARALLELISM`
  - `HTTP_BIND`
  - `CORS_ALLOWED_ORIGINS`
  - `LIST_LIMIT_MAX`
  - `USER_AGENT`
  - `STATIC_DIR`
- Retention/archive 组 canonical 名称改为：
  - `RETENTION_ENABLED`
  - `RETENTION_DRY_RUN`
  - `RETENTION_INTERVAL_SECS`
  - `RETENTION_BATCH_ROWS`
  - `ARCHIVE_DIR`
  - `INVOCATION_SUCCESS_FULL_DAYS`
  - `INVOCATION_MAX_DAYS`
  - `FORWARD_PROXY_ATTEMPTS_RETENTION_DAYS`
  - `STATS_SOURCE_SNAPSHOTS_RETENTION_DAYS`
  - `QUOTA_SNAPSHOT_FULL_DAYS`
- Xray/runtime 组 canonical 名称改为：
  - `XRAY_BINARY`
  - `XRAY_RUNTIME_DIR`
- 以下 legacy 键继续 hard-error，且报错风格要与本次新增 cutover 键完全统一：
  - `XY_DATABASE_PATH -> DATABASE_PATH`
  - `XY_FORWARD_PROXY_ALGO -> FORWARD_PROXY_ALGO`
- 对所有被移除的旧键，启动期错误文案必须精确到一对一 rename 指引，避免“use ...”这类非矩阵化提示。

### SHOULD

- 用集中化的 legacy-env 校验表维护 rename matrix，避免 README、实现和测试出现漏改或口径漂移。
- 针对 canonical 解析、legacy hard-fail、Docker / README / 部署文档残留扫描补上回归覆盖。

## 迁移矩阵（Migration Matrix）

| Legacy key                                 | Canonical key                           |
| ------------------------------------------ | --------------------------------------- |
| `XY_POLL_INTERVAL_SECS`                    | `POLL_INTERVAL_SECS`                    |
| `XY_REQUEST_TIMEOUT_SECS`                  | `REQUEST_TIMEOUT_SECS`                  |
| `XY_MAX_PARALLEL_POLLS`                    | `MAX_PARALLEL_POLLS`                    |
| `XY_SHARED_CONNECTION_PARALLELISM`         | `SHARED_CONNECTION_PARALLELISM`         |
| `XY_HTTP_BIND`                             | `HTTP_BIND`                             |
| `XY_CORS_ALLOWED_ORIGINS`                  | `CORS_ALLOWED_ORIGINS`                  |
| `XY_LIST_LIMIT_MAX`                        | `LIST_LIMIT_MAX`                        |
| `XY_USER_AGENT`                            | `USER_AGENT`                            |
| `XY_STATIC_DIR`                            | `STATIC_DIR`                            |
| `XY_RETENTION_ENABLED`                     | `RETENTION_ENABLED`                     |
| `XY_RETENTION_DRY_RUN`                     | `RETENTION_DRY_RUN`                     |
| `XY_RETENTION_INTERVAL_SECS`               | `RETENTION_INTERVAL_SECS`               |
| `XY_RETENTION_BATCH_ROWS`                  | `RETENTION_BATCH_ROWS`                  |
| `XY_ARCHIVE_DIR`                           | `ARCHIVE_DIR`                           |
| `XY_INVOCATION_SUCCESS_FULL_DAYS`          | `INVOCATION_SUCCESS_FULL_DAYS`          |
| `XY_INVOCATION_MAX_DAYS`                   | `INVOCATION_MAX_DAYS`                   |
| `XY_FORWARD_PROXY_ATTEMPTS_RETENTION_DAYS` | `FORWARD_PROXY_ATTEMPTS_RETENTION_DAYS` |
| `XY_STATS_SOURCE_SNAPSHOTS_RETENTION_DAYS` | `STATS_SOURCE_SNAPSHOTS_RETENTION_DAYS` |
| `XY_QUOTA_SNAPSHOT_FULL_DAYS`              | `QUOTA_SNAPSHOT_FULL_DAYS`              |
| `XY_XRAY_BINARY`                           | `XRAY_BINARY`                           |
| `XY_XRAY_RUNTIME_DIR`                      | `XRAY_RUNTIME_DIR`                      |
| `XY_DATABASE_PATH`                         | `DATABASE_PATH`                         |
| `XY_FORWARD_PROXY_ALGO`                    | `FORWARD_PROXY_ALGO`                    |

## 功能与行为规格（Functional/Behavior Spec）

### Core flows

- 配置解析时仅接受新的 canonical 裸名键；旧 `XY_*` 键一旦出现即直接返回配置错误。
- CLI override 继续优先于环境变量；本次不改 CLI 参数名。
- 所有默认值维持现状，例如：轮询节奏 `10s`、请求超时 `60s`、retention 关闭、archive 根目录 `archives`、xray binary 默认 `xray`。
- `CRS_STATS_POLL_INTERVAL_SECS` 仍然默认跟随 `POLL_INTERVAL_SECS`，而不是保留对旧 `XY_POLL_INTERVAL_SECS` 的任何语义依赖。

### Edge cases / errors

- 如果同时配置新旧键，仍然按旧键非法处理并阻断启动；不能“新键覆盖旧键后继续运行”。
- 旧键报错必须采用统一口径：`<OLD> is not supported; rename it to <NEW>`。
- 对新 canonical 键的非法值处理保持现状，不把本次命名调整顺手升级为“更严格解析”。

## 验收标准（Acceptance Criteria）

- Given 仅配置新的 canonical 键，When 服务启动解析配置，Then 所有对应字段可正常生效，默认值与既有行为不变。
- Given 环境中存在任意一个被移除的 `XY_*` 公开键，When 服务启动，Then 启动直接失败，且错误文案精确指出要迁移到哪个 canonical 键。
- Given 历史数据库中存在 `source='xy'` 调用记录或 quota snapshot，When 请求相关只读接口与聚合接口，Then 读取与聚合行为保持现状，不受 env rename 影响。
- Given README、部署文档、Docker 默认 env 与代码中的公开 env 示例，When 进行残留扫描，Then 除专门 migration 说明外，不再发布仍被支持的 `XY_*` 名称。

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- `cargo test`
- 针对 config parsing 的定向回归：新 canonical 名称可成功解析；被移除的旧键会 fail-fast。
- 文档/代码残留扫描：针对 README、Dockerfile、部署文档与源码中的公开 env 名称执行检索确认。

### Quality checks

- `cargo fmt --check`

## 文档更新（Docs to Update）

- `docs/specs/README.md`：新增本 spec，并在交付完成后写入 PR / checks / review-loop 状态。
- `README.md`：把公开 env 示例、retention/archive 说明与 migration note 统一到新命名。
- `docs/deployment.md`：把运维配置说明与 archive 路径描述统一到新命名。
- `Dockerfile`：把 runtime 默认 env 与 Xray 相关注释更新到新命名。

## 实现里程碑（Milestones / Delivery checklist）

- [ ] M1: 新建 spec，冻结 rename matrix、breaking policy 与 supersede 边界。
- [ ] M2: 后端配置解析切换到新 canonical 名称，并为全部 legacy `XY_*` 键补齐统一 hard-fail。
- [ ] M3: README / 部署文档 / Docker / 测试样例同步更新，并完成残留扫描。
- [ ] M4: `cargo test`、`cargo fmt --check`、review-loop、push、PR、checks 与标签全部收口到 merge-ready。

## 风险 / 假设

- 风险：仍在部署系统中保留旧 `XY_*` 键的实例会在升级后立即启动失败，因此 PR 与迁移说明必须明确这是 breaking public config change。
- 风险：若实现遗漏任一公开旧键，会出现“部分新命名、部分旧命名仍被接受”的灰区，必须用集中化矩阵和残留扫描兜底。
- 假设：当前唯一需要保留的 legacy hard-error 键就是 `XY_DATABASE_PATH` 与 `XY_FORWARD_PROXY_ALGO`，其余旧 `XY_*` 键均应迁移到本 spec 的新 canonical 名称。

## 变更记录（Change log）

- 2026-03-10: 创建 spec，冻结剩余公开 `XY_*` env 的 rename matrix、immediate cutover 策略与 breaking migration 口径。

## 参考（References）

- `docs/specs/2uaxk-remove-xyai-legacy-ingest/SPEC.md`
- `docs/specs/bh43j-database-path-raw-dir-anchor/SPEC.md`
- `README.md`
- `docs/deployment.md`
- `Dockerfile`
- `src/main.rs`
