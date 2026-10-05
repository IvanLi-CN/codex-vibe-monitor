# 系统工作区重构 - Implementation

主线任务工作量趋势与 SSE 保留，任务页移除旧性能摘要并提供 Grafana 深链接。
当前四张 mock 入口、移动、未配置与任务时间趋势证据已展示并获确认；新候选
完整 Web/后端门禁由 GitHub Actions 刷新，历史审查不能代替当前 head 证据。

项目存储测量保留主线的独立后台任务、API、缺测状态与文件身份去重。旧性能库退役后，
运行配置不再提供其外置路径；GNU du 集成 oracle 保留外置旧性能文件及侧文件作为
排除哨兵，维护库及其他已配置运行存储继续计入。退役离线归档位于应用挂载之外。

## Current State

- Canonical spec: `docs/specs/s7m3q-system-workspace/SPEC.md`
- Status: 功能实现与已完成的验收验证记录均已就绪；主人已确认并更新规范截图，当前 head 的 Tier 3 四条只读复核通过。PR 收敛待完成。当前 testbox 缺少 Chromium 运行库，本轮 Storybook 重跑受环境阻塞；前一候选曾通过且 rebase 未改变 web 源码。
- 项目存储总体积现由独立后台测量、只读快照 API 和独立页面读数提供；raw 业务盘点不再控制总体积是否可见。

## Implementation Summary

- 新增 `system` 顶层工作区与四个子页：`状态 / 任务 / 设置 / 代理`。
- 顶层导航由 `设置` 改为 `系统`，旧 `#/settings` 改为兼容跳转。
- 新增系统状态接口与系统后台任务记录接口。
- 系统任务列表保持既有 page/pageSize 语义，并增加以 `(startedAt, id)` 为锚点的 additive cursor 翻页；查询直接比较 UTC ISO 时间文本，schema 同时提供默认时间排序和 task/status 组合筛选排序索引。
- 原 settings 页按职责拆分为通用设置页与 forward-proxy 页，同时继续复用现有设置数据模型与写接口。
- 系统状态页 raw 统计使用持久化的已关联文件盘点，拆分为 `raw / request / response` 三组指标；由于盘点不枚举未关联物理残留，API 同时返回 `physicalCoverage=partial|unknown`。
- raw metrics 的 bytes 在盘点未就绪、后台延后、错误或状态未知时序列化为 `null`；Web normalizer 与页面都保持 fail-closed，不将缺失 raw bytes 或项目总量显示为 `0 B` 或“实际磁盘占用”。
- raw 指标已改为持久化增量快照：legacy path 由有界 cursor 补齐，启动时一次性回填旧 invocation/attempt owner link，新写入通过 response/request blob link 增量发现；状态页请求只读快照，不再枚举全部 raw 路径或逐文件读取元数据。pressure defer 或后台失败状态保留在内存 health override，避免诊断写抢占 SQLite。
- 业务 System Status 继续按既有 SQL 与 raw 盘点契约提供 last-good 快照；项目存储总体积使用单独的后台扫描和内存快照，不进入业务接口的 4 秒期限或 `503` 降级路径。
- 系统状态页布局已从 12 张等权卡片重构为“项目磁盘总览 + 数据库记录概况 + 归档与逻辑体量”。
- 系统状态接口补充 `liveInvocationsCount` 与 `completedArchiveBatchesCount`，用于解释 live 数据库与归档来源。
- 系统状态页通过独立 `ProjectStorageSummary` 显示全项目实测分配字节、采样时间与采集状态；raw 总量继续标为业务文件集合的“并集总量”，request / response 继续标为“侧向拆分”，不再以业务指标求和作为总体积公式。
- `raw payload 聚焦` 已改成“总量卡 + request 行 + response 行”的纵向层级，去掉窄列中的并排四小卡，避免 request-heavy 场景下数字区被长说明挤压变形。
- 总览首屏已进一步改成顺序流：主读数、项目级 breakdown、`raw payload 聚焦` 依次堆叠，避免左右上半区高度失衡导致的巨大空白。
- 新增 `GET /api/system/storage`，固定从内存快照返回状态、采样时间、stale 标记与可诊断原因；关闭 SQL pool 或业务状态不可用不影响该路由。
- `SystemStorageRuntime` 按实际解析范围流式统计 Unix `st_blocks × 512`，以设备/inode 全局去重；配置根符号链接解析到目标，内部符号链接按链接对象计量。
- 扫描包括主库父目录、解析后的 raw/archive、Xray runtime、性能库和维护库文件及侧文件。目录外数据库不扩展到父目录其他文件；单轮扫描有 128 深度、64 MiB 估算预算、协作让出、取消与单实例限制。
- 页面将业务状态和存储快照作为两路独立请求；10 秒超时、卸载取消、存储请求错误保留最近客户端成功值。raw preparing、业务 503、首次未知、延后和 last-good 错误状态都有页面及 Storybook 场景。
- `PROXY_RAW_DIR` 可配置代理 request/response raw payload 路径，`ARCHIVE_DIR` 可配置离线归档路径。二者默认分别为 `proxy_raw_payloads` 和 `archives`；相对路径按既有规则相对于 `DATABASE_PATH` 的父目录解析，绝对路径可指向容器内独立挂载点，例如 `PROXY_RAW_DIR=/mnt/cvm/raw` 与 `ARCHIVE_DIR=/mnt/cvm/archives`。部署时须把宿主机存储挂载到这些容器路径。

## Storage Footprint Investigation

### Findings

- `OverviewPanel` 在 raw 盘点不是 `ready` 或 raw bytes 为 `null` 时，将顶部 `projectDiskBytes` 设为 `null`。这是页面、Web normalizer、后端响应和调查时的旧总量契约一致执行的结果，不是格式化组件丢失了可用数值；当前 Spec 已改为独立的项目存储总体积契约。
- 顶部总量没有独立的目录测量来源。文件采集仅汇总已完成 `codex_invocations` 归档、主库及其 WAL/SHM、Xray runtime；raw 读取已关联文件的持久盘点。未关联 raw、其他数据集归档、性能库、维护库及其他目录内文件不能由这一公式完整表达。
- ready 的 raw 盘点仍只证明已关联文件集合；后端返回 `physicalCoverage=partial`。它可以给出带限制标记的已追踪总量，但不能证明数据目录总量。
- 文件体积采集使用 `metadata.len()`，而目录磁盘占用可以使用实际分配字节。两者在小文件、稀疏文件等情况下不同，不能混用后声称 breakdown 可加回同一个总量。
- 状态快照有 4 秒刷新期限和 60 秒服务边界。直接把无界目录扫描加入当前 SQL/文件刷新会扩大整页不可用风险。

### Runtime Evidence

以下读数来自运行中的 `2.85.0` 服务在 2026-10-03 的只读调查；服务继续写入，独立采样之间不构成原子快照。

| 观测                                          | 读数                                                      |
| --------------------------------------------- | --------------------------------------------------------- |
| 数据目录实际分配占用，`du -s -B1`             | `58,650,222,592 B`，约 `54.6 GiB`                         |
| 数据目录表观大小，`du -sb`                    | `58,096,474,613 B`，约 `54.1 GiB`                         |
| raw 目录实际分配占用                          | `34,030,391,296 B`，约 `31.7 GiB`                         |
| 全部 archive 目录实际分配占用                 | `4,863,225,856 B`，约 `4.5 GiB`                           |
| 状态 API 的已完成 invocation archive 文件大小 | `1,451,969,644 B`                                         |
| 状态 API 的主库/WAL/SHM 文件大小              | `17,878,207,304 B`                                        |
| 状态 API 的 raw 盘点                          | `preparing`、cursor `0`、bytes `null`、coverage `unknown` |

数据目录可读取且具有明确占用时，顶部总量仍显示未知。调查期间状态 API 曾返回 `503` 后恢复 `200`；受控日志核验确认近期出现数据库锁与刷新期限失败。raw 盘点任务为 enabled，但单次观测和有限日志不足以证明它永久卡死。

### Confirmed Requirements and Implementation

主人明确要求总体积包含数据目录和目录外单独配置的 raw/archive，并避免重复计算。实现现已按 `SPEC.md` 的 `REQ-STORAGE-*` / `VER-STORAGE-*` 覆盖目录和外置数据库范围，按稳定文件身份进行全局去重，并将业务分类数据保留为解释性指标。

- `PROXY_RAW_DIR` 保存代理请求/响应 raw payload 文件；`ARCHIVE_DIR` 保存归档文件。二者可分别设为独立的绝对路径或挂载点；相对值遵循既有解析规则，以 `DATABASE_PATH` 的父目录为基准。
- 外置性能库和维护库只测数据库文件及 WAL/SHM/journal，不扫描旁边无关文件；Xray runtime 使用其已有运行路径语义。
- 完整扫描成功才更新内存 last-good。失败和资源延后保留旧 bytes 与时间；首轮无成功值时返回空值。HTTP 路径只读内存，不执行 SQL 或文件系统访问。
- 定向 Linux 文件夹具使用 GNU `du -s -c -B1` 对照，覆盖普通/小型/稀疏文件、目录自身、嵌套与别名根、根链接、内部链接、硬链接、独立副本和外置数据库侧文件。
- 定向后端回归还验证失败保留与 stale、资源/深度上限、取消后单实例许可、关闭 SQLite 后接口可读。Linux GNU `du` 夹具 oracle 覆盖完整目录范围与唯一对象分配字节。

### Diagnostic Validation

- 修复前页面渲染诊断：复用 `SystemStatusPage` 的真实组件和 preparing fixture，仅断言首屏应保留可测总量，得到 `actualHeadline="未知"` 的预期失败；用例执行约 `35 ms`，整次 Vitest 运行约 `815 ms`。
- 从真实页面提取总量计算和格式化代码的临时回路，重复运行并缩小到一个 `4,096 B` 文件，仍在 `preparing/deferred/error/unknown` 判红，在 `ready` 显示 `4.0 KB`。归档、raw 文件和其他 runtime 文件都不是触发未知所必需的条件。
- `cargo test system_status_aggregates_counts_and_file_sizes -- --nocapture` 通过，验证现有分类口径。
- `cargo test system_status_surfaces_runtime_raw_metrics_deferral_without_a_db_write -- --nocapture` 通过，验证 raw defer 返回 `null/unknown`。
- 临时诊断用例和脚本仅用于调查，收尾删除；此前诊断阶段未修改产品代码。诊断红灯表示待修正需求，后续修复由本实现及其验证门禁覆盖。

## Quality Gates

- `bash .github/scripts/run-backend-tests.sh`：通过，共享 Linux testbox 上 lightweight、stateful-sqlite、archive-file-io profiles 全部通过。
- `cargo check --locked --all-targets --all-features`、`cargo fmt --all -- --check`、`bash .github/scripts/run-rust-source-quality.sh`：通过，包含 Clippy 与显式 source-quality 行数门禁。
- `cd web && bun run test`：共享 Linux testbox 完整回归通过，171 个测试文件、1,749 项通过、6 项跳过；包含独立请求超时、卸载取消及 404/非法响应保留旧值用例。
- `cd web && bun run test-storybook -- src/features/system/ProjectStorageSummary.stories.tsx src/features/system/SystemWorkspace.stories.tsx`：通过，69 项通过。
- `bun run typecheck:web`、`bun run lint:web`、`bun run build`、`bun run demo:build`、`bun run build-storybook`：通过；Web lint 有 96 条非阻塞 warning。
- `E2E_BASE_URL=http://127.0.0.1:48350 bunx playwright test tests/e2e/system-storage.spec.ts`：4 个受控存储状态场景通过。
- `bun run lint:docs`、本次修改文档的定向 `bunx dprint check`、Spec contract、Spec drift 与 visual-evidence 文档检查：在记录和版本影响文档更新后通过。全仓 `bunx dprint check` 仍报告既存的 `docs-site/docs/troubleshooting.md` 与 `.codex/skills/style-topic-pr-label-release/SKILL.md` 格式问题，均不属于本次修改范围。
- UI 视觉证据：主人已确认桌面 1440×1000、移动 393×852 的 `ui_demo` 页面截图及 Storybook 状态组件截图；三张图已写入 SPEC canonical assets。视觉对照报告审计通过；mock 样本数据不代表生产读数。

## Disposition

- `spec_disposition=create`
- `project_doc_disposition=none`
- `solution_disposition=none`

## Memory Diagnostics

- `MemoryDiagnosticsRuntime` 在 runtime 启动后执行一次采样，之后每 30 秒采样一次；采样只访问 proc/cgroup 文件和现有内存容器，不增加 System Status 的 SQLite 读。
- 已知组件估算包含 terminal hub/journal、runtime store、Dashboard cache、long-term interval、prompt/network/routing cache、raw writer occupancy 与 SQLite writer queue。timeseries staging 复用 terminal hub pending bytes，避免重复计算。
