# raw 负载冷压缩与磁盘全文搜索（#jg7a5）

## 状态

- Status: 进行中
- Created: 2026-03-13
- Last: 2026-03-13

## 背景 / 问题陈述

- 线上 `proxy_raw_payloads` 在当前流量下增长过快，3 天窗口内也会快速堆积出大量 raw 文件。
- 已确认主要体积来自 `/v1/responses` 成功请求，很多 request raw 接近 10MiB，单靠现有 retention 无法显著缓解最近几天的磁盘占用。
- 现有在线 API 与 SQLite 主要依赖结构化字段，不需要对 raw 文件做在线索引搜索；但运维排障仍依赖 SSH 到宿主机后直接检索磁盘 raw 文件内容。
- 因此本轮方案必须同时满足“明显降低磁盘占用”和“保留按文件粒度的直接全文搜索能力”，不能把搜索能力退化成仅 UI 或仅数据库可用。

## 目标 / 非目标

### Goals

- 为 raw payload 文件引入单文件 gzip 冷压缩，显著降低 `proxy_raw_payloads` 的磁盘占用。
- 保留热数据明文写入，并在超过 24h 后由 retention 自动转为 `*.bin.gz`。
- 保持历史 `*.bin` 与新 `*.bin.gz` 在读取链路上的透明兼容。
- 提供统一的 Linux 运维搜索入口，同时命中文本 raw 与 gzip raw。
- 尽量不改现有 SQLite schema、在线 API 形状与 retention/archive 语义。

### Non-goals

- 不在本轮实现 UI 搜索、SQLite FTS 或额外全文索引系统。
- 不把 raw 文件打成 tar/zip 大包后统一检索。
- 不引入 `zstd`、`xz` 等额外 codec 作为本轮运行时依赖。
- 不重做已有 `INVOCATION_SUCCESS_FULL_DAYS` / `INVOCATION_MAX_DAYS` 数据生命周期策略。

## 范围（Scope）

### In scope

- `src/main.rs` 中 raw 写入、raw 读取、retention cold-compress、archive/prune 删除与 orphan sweep 的兼容性调整。
- `scripts/search-raw` 统一脚本与 `Dockerfile` 运行时打包。
- `README.md`、`docs/deployment.md`、`docs/specs/README.md` 的配置和运维说明同步。
- Rust 回归测试：历史明文读取、新 gzip 读取、dry-run/live 压缩、archive 删除 gzip raw、搜索脚本混合命中。

### Out of scope

- 停机状态下直接从宿主机绕过容器权限访问 volume 并搜索。
- raw 搜索结果聚合到 Web UI、数据库或额外服务。
- 按请求内容建立额外副本、sidecar 索引或异步文本抽取。

## 方案决策

### Codec

- 本轮固定使用 `gzip`。
- 理由：仓库已内置 `flate2`，运行镜像只需补一个 `gzip` CLI 即可支持运维搜索，不需要引入额外 codec 二进制或格式判定分支。
- `PROXY_RAW_COMPRESSION` 提供 `none | gzip` 两档，默认 `gzip`；`none` 作为唯一降级开关。

### 数据分层

- 新写入 raw 继续保持热数据明文：`store_raw_payload_file()` 仍写 `*.bin`。
- retention 在其它 prune/archive 动作之前执行冷压缩阶段。
- 超过 `PROXY_RAW_HOT_SECS`（默认 `86400`）且仍保留 raw path 的记录，按文件粒度转成 `*.bin.gz`。
- 压缩成功后更新 `request_raw_path` / `response_raw_path` 为真实 gzip 路径；`request_raw_size` / `response_raw_size` 继续表示原始 payload 字节。

## 功能与行为规格（Functional/Behavior Spec）

### Raw 写入与读取

- 热数据写入路径保持 `*.bin`，避免请求链路同步压缩带来的额外 CPU 与尾延迟风险。
- `read_proxy_raw_bytes()` 必须透明支持：
  - 直接读取历史 `*.bin`；
  - 读取 `*.bin.gz` 并自动解压；
  - 当数据库路径仍指向 `.bin`、但磁盘上已是 `.bin.gz` 时，优先尝试备用后缀路径，保证历史/中断场景兼容。
- preview、backfill、详情页 raw 读取等调用点统一复用透明 reader，不新增调用方分支。

### Retention cold-compress

- maintenance 顺序调整为：`cold-compress -> structured prune -> archive -> orphan sweep`。
- cold-compress 仅处理超过热窗口且 raw path 非空的 live row；已经是 `.gz` 的路径跳过。
- live 模式：
  - 先流式写入 `*.gz.tmp`；
  - 成功后原子 rename 为 `*.gz`；
  - 再更新数据库路径；
  - 最后删除旧 `*.bin`。
- dry-run 模式只输出候选数与 gzip 后体积估算，不修改数据库或磁盘文件。
- 若发现数据库仍指向 `.bin`，但磁盘上只有同名 `.bin.gz`，maintenance 允许把路径修复到 gzip 文件，不重新压缩。

### 删除与 orphan sweep

- prune/archive 删除 raw 文件时，必须同时兼容 `.bin` 与 `.bin.gz`，避免旧逻辑只删一种后缀。
- orphan sweep 只保护数据库当前引用的真实路径，不主动把 `.bin` 和 `.bin.gz` 互相视为同一引用，以避免双份文件长期共存而无法回收。
- retention dry-run/live summary 需要新增 raw 压缩统计：
  - `raw_files_compression_candidates`
  - `raw_files_compressed`
  - `raw_bytes_before`
  - `raw_bytes_after`
  - `raw_bytes_after_estimated`

## 运维全文搜索约定

- 镜像内提供 `search-raw`，固定搜索根目录默认为 `/srv/app/data/proxy_raw_payloads`，可用 `--root` 覆盖。
- 默认模式为 fixed-string；使用 `--regex` 时才启用正则搜索。
- 搜索同时扫描 `*.bin` 与 `*.bin.gz`，输出统一为 `path:line:text`。
- 生产推荐固定命令：
  - `docker exec ai-codex-vibe-monitor search-raw '<needle>'`
  - `docker exec ai-codex-vibe-monitor search-raw --regex '<pattern>'`

## 接口契约（Interfaces & Contracts）

| Name                                     | Kind            | Scope    | Change   | Notes                          |
| ---------------------------------------- | --------------- | -------- | -------- | ------------------------------ |
| `PROXY_RAW_COMPRESSION`                  | env             | runtime  | 新增     | `none                          |
| `PROXY_RAW_HOT_SECS`                     | env             | runtime  | 新增     | 默认 `86400`                   |
| `request_raw_path` / `response_raw_path` | DB/API field    | existing | 兼容扩展 | 值可能为 `*.bin` 或 `*.bin.gz` |
| `search-raw`                             | runtime command | ops      | 新增     | 同时搜索 plain + gzip raw      |

## 验收标准（Acceptance Criteria）

- Given 历史 `*.bin` 文件与旧记录，When 现有 raw 读取链路访问它们，Then 结果与现状一致。
- Given 超过热窗口的 raw 文件，When retention live 模式执行，Then 磁盘文件转成 `*.bin.gz`、数据库路径同步更新、原始内容读取不变。
- Given retention dry-run，When 执行 maintenance，Then 数据库与文件不变，但能输出 gzip 后体积估算。
- Given prune/archive 清理老记录，When raw path 指向 `*.bin` 或 `*.bin.gz`，Then 对应磁盘文件都能被删除。
- Given 一个 token 同时存在于 plain raw 与 gzip raw，When 执行 `search-raw`，Then 两种文件都能命中，并统一输出 `path:line:text`。

## 文档更新（Docs to Update）

- `docs/specs/README.md`
- `README.md`
- `docs/deployment.md`

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 增加 raw cold-compress 配置、透明解压读取与 `.bin/.bin.gz` 路径兼容层。
- [x] M2: 在 retention 中插入 cold-compress 阶段，并补齐 dry-run/live 统计。
- [x] M3: 新增 `search-raw` 脚本与 Docker runtime 打包。
- [x] M4: 完成 README / deployment / spec 索引同步。
- [ ] M5: 完成测试、镜像 smoke、PR 与 review-loop 收敛。

## 变更记录（Change log）

- 2026-03-13: 创建 spec，锁定 `gzip + 24h 热明文 + 单文件 gzip + docker exec search-raw` 作为本轮唯一方案。
- 2026-03-13: 已完成后端冷压缩、透明解压、retention 统计与搜索脚本实现，等待文档与验证收口。
- 2026-03-13: 已完成 README / deployment / spec 索引同步，`cargo fmt --check`、`cargo check`、`cargo test` 全部通过；本地 Docker daemon 不可用，镜像 smoke 待有 daemon 环境时补跑。

## 参考

- `README.md`
- `docs/deployment.md`
- `src/main.rs`
- `src/tests/mod.rs`
