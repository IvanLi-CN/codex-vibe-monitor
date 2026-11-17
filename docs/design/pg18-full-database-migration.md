---
status: proposed
---

# PostgreSQL 18 全量数据库迁移设计

本文定义将现行服务的全部数据库状态迁移到 PostgreSQL 18 的目标、边界、CLI 能力、导入阶段和验收门禁。迁移只允许从旧服务及其数据源流向新服务及其 PostgreSQL 目标库；本文不设计目标库回写旧数据源的路径。

## 已确认的设计决策

| 项目              | 决策                                                                                         |
| ----------------- | -------------------------------------------------------------------------------------------- |
| 数据范围          | PostgreSQL 承担全部数据库状态；大型文件继续由现有文件资产存储，PG 保存完整索引、元数据和引用 |
| 目标库结构        | 一个 PostgreSQL 18 数据库，按领域划分 schema                                                 |
| 服务关系          | 旧服务是迁移源，新服务是 PG-only 目标服务                                                    |
| 首次导入          | 旧服务继续运行时执行一致性首次导入                                                           |
| 最终追平          | 停止旧服务后执行最终追平导入                                                                 |
| 数据方向          | 旧服务 → 新服务，单向迁移                                                                    |
| 回退              | 不提供回退功能；导入失败时暂停、修复、继续或重建目标导入                                     |
| 部署位置          | 继续部署在 101，但使用扩容后的独立 volume 和独立 stack                                       |
| 当前 analytics PG | 只作为对照和验证对象，不作为全量迁移输入                                                     |

## 目标与非目标

目标是让新服务可以独立使用 PostgreSQL 18 完成业务数据的读取和写入，并保持现有业务 ID、关联关系、时间语义、配置语义、统计语义和文件引用语义。目标库必须可以通过 CLI 进行初始化、断点续跑、最终追平、校验和完成标记。

本文不把当前 PostgreSQL analytics 副本扩展成全库迁移源。本文只规定两个服务之间的数据迁移和新服务使用 PG18 的数据库边界。

## 数据边界

PG18 目标库必须承载当前数据库中的业务状态、配置状态、账户状态、路由状态、任务状态、调用记录、上游请求记录、统计投影、归档索引和其他需要持久化的关系数据。目标库不直接承载现有的大型文件资产；这些文件继续保存在文件资产目录中，PG 保存文件身份、路径、大小、摘要、生命周期和引用关系。

每个迁移对象必须声明自己的同步属性。当前 CLI 会根据对象名生成初始分类，正式目标库上线前仍应由对象清单复核分类：

- `immutable`：导入后内容不再变化，只需要校验和引用完整性。
- `append_only`：只追加新记录，导入器可以按稳定主键或源序号分批处理。
- `mutable`：已有记录可能被修改，最终追平时必须执行完整的更新和删除校验。
- `derived`：可以从已迁移的权威数据重建，导入器需要记录重建状态而不是把旧投影当作权威事实。

当前 analytics PG 中的事实表属于 derived 数据，不能替代旧服务的完整业务数据。

## 目标库结构

目标使用一个数据库和多个 schema，避免跨数据库事务和备份编排复杂化，同时保留领域边界：

```text
cvm_core       业务实体、配置、账户、路由和运行状态
cvm_activity   调用、上游请求、过程记录、时间线和 PG runtime 代理请求摘要
cvm_stats      summary、rollup、timeseries 和统计投影
cvm_archive    归档索引、归档状态和历史覆盖信息
cvm_assets     文件资产索引、摘要、引用和生命周期
cvm_migration  迁移运行、批次、断点、校验和错误状态
```

目标 schema 的每次结构变更都必须有迁移编号、checksum、兼容范围和可重入行为；当前 migration schema version 为 12，保存 `nul-escape-v1` 文本布局、target profile、源列声明类型、外键、唯一索引、外部资产清单、source root、资产索引、批次 source cursor 和失败后的恢复阶段。旧试验布局保持原样并明确拒绝混用，须从同一源快照导入新的目标数据库。目标库的业务角色不得执行 schema DDL；DDL 由一次性的迁移角色执行，导入角色只获得目标表所需的写权限。两个连接 URL 必须指向同一个 PostgreSQL 数据库，避免迁移 ledger 与导入数据分离。

业务连接池和迁移连接池分别由 `POSTGRES_PRIMARY_MAX_CONNECTIONS` 与 `POSTGRES_PRIMARY_MIGRATION_MAX_CONNECTIONS` 限制，两个值都必须为正整数。

代码侧的 `PrimaryDatabase` 提供 PG 连接、18.x 版本校验、六个业务 schema、角色 USAGE 权限、schema checksum 和 JSON 状态 store 边界；`PrimaryStateStore` 已提供 SQLite/PostgreSQL 双实现，统一 JSON 状态读写契约，并注入 `AppState` 作为后续 domain store 的共享边界。PG backend 启动路径会先执行 runtime schema 校验，再进入独立的 PG runtime；当前已提供健康检查、系统状态、系统存储、系统任务、managed tasks runtime/timeline、可观测性、调用记录分页、locate、pool-attempts、suggestions、new-count、summary/detail/workflow/body 读取、基础统计、dashboard activity/network、长期统计 overview、统计时间序列、性能统计、失败统计、其他错误、账户活动、forward-proxy 统计、并行工作、invocation timeline、最新配额、pool tags 和 model-routing-live 读取、聚合 settings、定价目录、代理模型 preset/delete、models.dev sync preview/apply/state、forward-proxy、pool routing settings、external API key 设置读写，以及上游账户列表、详情、API key 账户创建、批量 enable/disable/set_group/delete、单账户 patch/delete、账户事件、转发代理绑定节点和账户调用记录读取，其中 pool routing secret 与账户凭据使用 primary runtime 的加密密钥保存，响应不会返回密文。PG-only runtime 还提供 `/v1/{path}` 的上游转发：按 priority_tier（policy 覆盖）和账户 ID 稳定顺序构造启用且有凭据的候选，解密其 API key/OAuth access token，按账户的 upstream base URL 构造目标请求；连接失败、响应体读取失败或 429/5xx 会继续尝试后续候选，没有可用账户时沿用全局 upstream URL 和调用方授权头。请求沿用方法、查询参数和安全请求头，应用配置的请求体上限与上游超时，并将方法、路径、状态、字节数、耗时和错误摘要写入 `cvm_activity.proxy_requests`；该摘要不保存请求或响应正文。PG-only 路由会在 JWT access token 过期时使用 OAuth refresh token 更新加密凭据；账户的稳定前向代理绑定键已接入 PG 请求客户端，PG-only 上游响应支持流式转发和流结束摘要记录。managed task 的 PG 路由支持目录读取、单任务详情、`PATCH` 控制字段和 `POST /run` 请求入队，控制值与队列写入 `cvm_core.state_kv`，队列追加通过存储层事务原子完成，运行时接口会返回最多 100 条待执行请求；`verify_archive_storage` 已提供 PG-only 手动执行路径，会校验外部资产索引中的路径、文件大小和 SHA-256，并将结果映射回任务详情的最近执行信息。其他维护任务仍需后续维护域替换工作接入。迁移 materialize 会为整数单列主键创建并校准 PG identity sequence，为复合主键创建唯一索引；后续 domain store 应在该边界上逐域替换 SQLite 直接访问，SQLite store 继续保留给现有兼容测试。

## 迁移运行模型

一次迁移由 `run_id` 和 `target_profile` 标识。`cvm_migration.runs` 保存目标 profile，后续所有命令必须使用相同 profile；这样不同目标连接配置不能误操作同一个迁移 run。`cvm_migration` 至少保存以下状态：

| 状态             | 含义                                             |
| ---------------- | ------------------------------------------------ |
| `prepared`       | 源检查和目标 schema 检查完成，可以开始首次导入   |
| `seeding`        | 旧服务仍运行，正在导入一致性首次数据             |
| `seeded`         | 首次导入完成，已生成首轮校验结果                 |
| `source_stopped` | 旧服务已停止，允许执行最终追平                   |
| `catching_up`    | 正在读取停止后的最终源状态并修正目标             |
| `verified`       | 最终追平和全部验收检查完成                       |
| `finalized`      | 迁移完成已确认，运行账本已标记完成               |
| `paused`         | 当前批次停止，允许使用同一 `run_id` 继续         |
| `failed`         | 当前批次不可继续，必须修复后重试或重建该目标运行 |

每个批次记录源对象、源主键范围、目标表、处理行数、提交时间、checksum 和错误码。`verify` 还会检查整数主键对应的 PG sequence 不低于已导入最大值。重复执行同一批次必须得到相同结果，不能产生重复行；不同源或不同迁移运行必须使用独立目标数据库。

## CLI 设计

CLI 作为一次性迁移工具运行，不作为第三个长期服务。目标 PG 连接信息通过受保护的环境或 service profile 提供，不放入 shell 历史和命令行参数。

### 源检查

```bash
codex-vibe-monitor migrate inspect-source \
  --source-root /srv/old-service/data
```

`inspect-source` 只读检查源数据库 schema、表目录、文件资产目录、对象类型、估算规模、主键范围、时间范围和可读性，输出机器可读的 source manifest。它不修改旧服务或源数据。

### 准备目标

```bash
codex-vibe-monitor migrate prepare \
  --target-profile cvm-pg18-migration \
  --run-id <run-id> \
  --source-manifest <source-manifest.json>
```

`prepare` 校验目标 PostgreSQL 版本、schema checksum、角色权限、容量门禁和迁移对象清单，然后创建或恢复该 `run_id` 的迁移状态。目标不满足条件时命令必须失败，并保留明确错误码。

### 首次导入

```bash
codex-vibe-monitor migrate seed \
  --target-profile cvm-pg18-migration \
  --run-id <run-id> \
  --source-root /srv/old-service/data \
  --batch-rows 5000
```

`seed` 在旧服务仍运行时执行。源适配器取得 SQLite online backup 一致性快照，manifest、导入与内部内容校验复用该快照。按业务主键游标分页后，以原生列类型分批 `COPY FROM STDIN` 直接写业务表；不创建持久 staging 或 imported ledger，也不在业务行保存迁移标识和整行 JSON。批次数据与 checkpoint 在同一事务提交，完整写入后建立业务索引、外键并校准 sequence。

大库导入可以把已完成的一致性导出通过 `MIGRATION_SNAPSHOT_PATH` 传给 CLI，避免重复 online backup。默认每批 5000 行，可用 `--batch-rows` 调整；批次内存随行内容大小变化。`MIGRATION_SKIP_SOURCE_COUNTS=1` 跳过导入前的全表计数，由实际导入和校验记录行数。列索引的部分唯一索引保留原 WHERE 谓词并在 PG 建立；表达式索引仍明确拒绝。源快照 SHA-256 与阶段绑定，恢复不能混用新快照。

首次导入不能直接复制正在使用的数据库文件，也不能把运行中的临时文件当作一致快照。导入器必须使用受支持的只读快照接口或源数据库提供的一致性读边界。

### 停止源服务

旧服务停止后，运维记录停止确认并执行：

```bash
codex-vibe-monitor migrate mark-source-stopped \
  --target-profile cvm-pg18-migration \
  --run-id <run-id>
```

该命令只记录迁移阶段和源状态，不执行数据导入。未进入 `source_stopped` 的运行不能执行最终追平。

### 最终追平导入

```bash
codex-vibe-monitor migrate catch-up \
  --target-profile cvm-pg18-migration \
  --run-id <run-id> \
  --source-root /srv/old-service/data \
  --batch-rows 5000
```

`catch-up` 在旧服务停止后读取最终一致性快照。对非 NULL 业务主键，以主键游标扫描源端，每次只建立一个原生列临时批次，插入新增行、更新内容变化的行，并删除完整扫描范围内源端不存在的行；最后清理尾部删除。未变化的行不重复写入。无主键或包含 NULL 主键的表采用直接重放，不保留全量中间副本。

`cvm_migration.object_columns` 保存源列顺序、声明类型和主键位置，`object_targets` 保存对象到领域表的映射。业务表保留原业务列，业务主键及索引不附带运行标识；一个 PG 数据库只允许一个迁移运行。断点由批次元数据保存，运行时按原业务主键读写。包含旧 `imported_rows` 的试验库保持原样并明确拒绝混用，应从同一源快照重新导入新数据库，见 [直接导入设计](../adr/0056-pg18-direct-import.md)。

每个对象直接写入六个目标 domain schema 之一，`cvm_migration.object_targets` 仅保存源对象到目标 schema/table 的映射。业务表保留源列和业务键，不保存 `run_id`、`source_key` 或整行 JSONB 副本。

### 校验、状态和续跑

```bash
codex-vibe-monitor migrate verify \
  --target-profile cvm-pg18-migration \
  --run-id <run-id> \
  --strict

codex-vibe-monitor migrate status \
  --target-profile cvm-pg18-migration \
  --run-id <run-id>

codex-vibe-monitor migrate resume \
  --target-profile cvm-pg18-migration \
  --run-id <run-id> \
  --batch-rows 5000

codex-vibe-monitor migrate finalize \
  --target-profile cvm-pg18-migration \
  --run-id <run-id>
```

`verify --strict` 检查行数、源快照身份、逐批原生列内容、外键、唯一索引、文件资产和 sequence。内容比较使用独立 SQLite 读取器，对照运行时解码后的 PostgreSQL 原生列；每批最多 2,000 行，临时表仅存本批主键，不复制完整期望行。文本采用可逆零字符编码，BLOB 按原始二进制保存并在 JSON 读取时转为 base64。不依赖全量 JSON 副本或空 staging。新服务 API 重启读写和备份恢复是独立验收项。`finalize` 只接受 `verified`，不修改源端。

`resume` 接受 `paused`、`failed` 及进程中断留下的 `seeding`、`catching_up`，根据原阶段继续首次导入或追平。每批写入与业务主键游标原子提交，恢复使用同一不可变快照，跳过已提交批次。无主键表按确定性排序和行数断点重放。旧全量 JSON ledger 不自动续接到新布局。

## 两个服务的运行顺序

```mermaid
sequenceDiagram
    participant Old as 旧服务
    participant CLI as migration CLI
    participant PG as PG18 目标库
    participant New as 新服务

    Old->>CLI: 继续提供源数据
    CLI->>Old: 一致性只读读取
    CLI->>PG: seed 分批导入
    CLI->>PG: verify 首次导入
    Old-->>Old: 停止服务
    CLI->>CLI: mark-source-stopped
    CLI->>Old: 读取停止后的最终状态
    CLI->>PG: catch-up 修正新增、修改、删除
    CLI->>PG: verify --strict
    New->>PG: 以 PG-only 模式启动
    CLI->>PG: finalize
    Old-->>Old: 不再承担生产写入
```

首次导入阶段允许旧服务继续运行；最终追平阶段要求旧服务已经停止。PG-only 运行时可以读取 `seeded`、`verified` 或 `finalized` 的目标库，支持首次迁入后的隔离验证。`catch-up` 以停止后的源快照为同步依据，不提供双向写入或目标库向旧服务回写。CLI 状态不替代实际服务的启停操作。

## 校验门禁

首次导入与最终追平都执行原生列内容校验。完整交付验收还包括服务读写和备份恢复，不以单个 CLI 成功替代。验收项包括：

- 源和目标的对象清单一致；
- 每个对象的行数和主键范围一致；
- mutable 对象的新增、修改和删除结果一致；
- 关键时间桶和业务维度 checksum 一致；
- 外键、唯一键和引用关系没有孤儿记录；
- 文件资产索引与实际文件清单一致；
- 目标 sequence 不会生成已存在的主键；
- 新服务的代表性读取和写入能够提交并在重启后保持；
- 目标数据库备份可以恢复到可查询状态。

任何校验失败都阻止 `finalize`。失败运行可以使用同一 `run_id` 继续或重建目标运行，但不提供目标到旧服务的回退功能。

## 容量和部署门禁

迁移完成后的 `pg_database_size` 必须不超过 30,000,000,000 字节。每批提交与最终校验执行容量门禁，`MIGRATION_MAX_TARGET_BYTES` 可降低但不能提高此上限。压缩备份大小不能作为数据库占用。PG 表、索引、WAL、单批临时空间、外部文件、源快照和备份仍须分别计入部署磁盘预算；不在目标库保留全量迁移副本。生产部署和扩容属于后续运维任务。

新 PG 服务和迁移 CLI 使用独立的数据库、volume、容器名和配置 profile。当前 analytics PG 不被清空、不被改造成全库目标，也不作为新目标的导入源。

## 实施顺序

1. 完成源对象清单和文件资产清单。
2. 完成 PG18 schema、角色、迁移 ledger 和目标容量门禁。
3. 实现 `inspect-source`、`prepare`、`status` 和 `verify`。
4. 实现 `seed` 的一致性读取、原生列 COPY、批次 checkpoint 和断点续跑。
5. 实现 `mark-source-stopped` 和 `catch-up` 的最终状态 reconcile。
6. 实现新服务的 PG-only 运行模式和启动检查。
7. 在离线测试数据上跑通完整 CLI 流程，并保留 PG18 隔离环境的验证证据。
8. 将生产首次导入、源服务停止后的最终追平和 `finalize` 作为后续运维任务；本实现交付不执行这些操作。

## 失败处理

迁移 CLI 的失败处理是暂停、修复、继续或重建目标导入。旧服务在首次导入阶段保持原有运行方式；最终追平阶段停止后，不再向源库写入。目标导入失败不会自动修改旧数据源，也不会启动目标到源的反向复制。
