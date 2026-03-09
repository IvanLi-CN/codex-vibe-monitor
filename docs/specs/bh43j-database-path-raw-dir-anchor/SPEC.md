# 数据库环境变量重命名与 raw 路径锚点修复（#bh43j）

## 状态

- Status: 已完成（5/5）
- Created: 2026-03-09
- Last: 2026-03-09

## 背景 / 问题陈述

- 运行时当前仍使用 `XY_DATABASE_PATH` 作为数据库路径环境变量，这个命名已经脱离现有产品语义，也容易和已移除的 XYAI 历史能力混淆。
- 生产只读排查确认：当 `PROXY_RAW_DIR` 保持默认相对路径 `proxy_raw_payloads` 时，应用会按当前工作目录落盘 raw 请求/响应文件，导致新文件进入容器层 `/srv/app/proxy_raw_payloads`，而不是随数据库一起进入持久卷目录。
- retention / orphan sweep 也沿用了当前工作目录语义，容易在运维侧继续误判“当前 raw 根目录”，放大容器重建后的文件漂移问题。
- 历史库里已经同时存在绝对路径、锚定数据库目录的相对路径，以及依赖 cwd 的旧相对路径；本次修复必须防再发，同时保持历史读取与清理兼容。

## 目标 / 非目标

### Goals

- 把数据库路径环境变量切换为 `DATABASE_PATH`，并让它成为唯一合法的数据库 env 名。
- 当环境中仍存在 `XY_DATABASE_PATH` 时，服务必须启动失败并给出明确迁移提示，避免静默回退到默认数据库文件。
- 把相对 `PROXY_RAW_DIR` 与 `XY_ARCHIVE_DIR` 的解析统一锚定到 `DATABASE_PATH` 同级目录，而不是当前工作目录。
- 保证新产生的 raw 请求/响应文件落到数据库同级持久化目录，避免继续写入 `/srv/app/proxy_raw_payloads` 之类的容器层路径。
- 保持历史旧相对路径记录的读取、backfill、retention 与 orphan sweep 兼容，不引入 missing-file 激增。

### Non-goals

- 不批量重命名其他 `XY_*` 通用环境变量。
- 不自动迁移现存 stray raw 文件，也不回写历史数据库路径。
- 不改变 `/api/*`、SSE 或前端类型结构。

## 范围（Scope）

### In scope

- `src/main.rs`：数据库 env 解析、旧变量 fail-fast、raw/archive 相对路径解析、raw 文件写入、orphan sweep 与相关测试。
- `Dockerfile`：镜像默认数据库环境变量切换到 `DATABASE_PATH`。
- `README.md`、`docs/deployment.md`、`docs/specs/README.md`：配置示例、相对路径基准与 breaking change 说明。
- fast-track 交付所需的提交、PR、checks 与 review-loop 收敛。

### Out of scope

- 生产已有 stray 文件的自动搬运或数据库回填修复。
- 非数据库 env 的系统性去 XY 前缀改名。
- 归档格式、release 流程或在线统计接口协议调整。

## 需求（Requirements）

### MUST

- `DATABASE_PATH` 成为唯一合法数据库环境变量；`XY_DATABASE_PATH` 一旦出现即阻断启动。
- fail-fast 错误文案必须明确指出：旧变量已移除，应该改为 `DATABASE_PATH`。
- 相对 `PROXY_RAW_DIR` 必须锚定到 `DATABASE_PATH.parent()`；绝对路径配置保持原样。
- 相对 `XY_ARCHIVE_DIR` 必须继续锚定到 `DATABASE_PATH.parent()`，与 raw 路径语义一致。
- 新写入的 raw 文件路径不得再依赖当前工作目录解析。
- 历史旧相对路径记录仍必须保持可读、可补数、可清理。

### SHOULD

- 新写入的 raw 文件路径在数据库路径为绝对路径时应落成稳定绝对路径，降低运维歧义。
- README 与部署文档应明确 `PROXY_RAW_DIR` 的默认相对路径语义，以及 `XY_DATABASE_PATH` 的 breaking change。
- orphan sweep 应只扫描当前数据库锚定后的 raw 根目录，不再把 cwd 当作“当前真实目录”。

## 功能与行为规格（Functional/Behavior Spec）

### Core flows

- 启动配置解析时，若检测到 `XY_DATABASE_PATH` 存在，则直接返回配置错误；只有 `DATABASE_PATH` 可以提供数据库路径。
- 当 `DATABASE_PATH` 为绝对路径且 `PROXY_RAW_DIR=proxy_raw_payloads` 时，raw 文件应落到 `<DATABASE_PATH 同级目录>/proxy_raw_payloads/`。
- `store_raw_payload_file()`、启动期 raw 目录创建与 retention orphan sweep 必须共用同一套 raw 目录解析逻辑。
- `resolved_archive_dir()` 继续负责 archive 根目录解析，但相对路径基准必须与 raw 路径保持一致。
- 对历史数据库里的旧相对路径，读取与删除候选路径仍要保留“当前工作目录 + fallback root”双候选兼容逻辑。

### Edge cases / errors

- 若同时配置 `DATABASE_PATH` 与 `XY_DATABASE_PATH`，仍按旧变量非法处理并阻断启动。
- 若 `DATABASE_PATH` 未配置，则继续使用默认 `codex_vibe_monitor.db`；此时相对 raw/archive 目录保持仓库默认相对语义。
- 若 `PROXY_RAW_DIR` 或 `XY_ARCHIVE_DIR` 显式配置为绝对路径，则不得再拼接数据库父目录。
- 若历史记录引用的是 cwd-relative raw 文件，修复后仍应优先按 cwd 路径读取，不强制迁移。

## 接口契约（Interfaces & Contracts）

| 接口（Name）       | 类型（Kind）    | 范围（Scope） | 变更（Change） | 负责人（Owner） | 使用方（Consumers） | 备注（Notes）              |
| ------------------ | --------------- | ------------- | -------------- | --------------- | ------------------- | -------------------------- |
| `DATABASE_PATH`    | Runtime env var | public        | Add            | backend         | Docker / Compose    | 新的唯一数据库路径变量     |
| `XY_DATABASE_PATH` | Runtime env var | public        | Remove         | backend         | Docker / Compose    | 保留为非法配置并 fail-fast |
| `PROXY_RAW_DIR`    | Runtime env var | internal      | Clarify        | backend         | proxy capture       | 相对路径锚定到数据库目录   |
| `XY_ARCHIVE_DIR`   | Runtime env var | internal      | Clarify        | backend         | retention/archive   | 相对路径锚定到数据库目录   |

## 验收标准（Acceptance Criteria）

- Given 仅设置 `DATABASE_PATH`，When 服务启动，Then 使用指定 SQLite 文件并正常初始化。
- Given 环境中存在 `XY_DATABASE_PATH`，When 服务启动，Then 启动失败且错误文案明确要求改用 `DATABASE_PATH`。
- Given `DATABASE_PATH=/srv/app/data/codex_vibe_monitor.db` 且 `PROXY_RAW_DIR=proxy_raw_payloads`，When 新请求落盘，Then raw 文件进入 `/srv/app/data/proxy_raw_payloads`，而不是 `/srv/app/proxy_raw_payloads`。
- Given cwd 与数据库目录不同，When 执行 orphan sweep，Then 只扫描数据库锚定后的 raw 目录，不误删 cwd 下 stray 文件。
- Given 历史记录仍引用旧的 cwd-relative raw 路径，When 执行读取或 backfill，Then 旧文件仍可被识别。

## 实现前置条件（Definition of Ready / Preconditions）

- 数据库 env 新名称固定为 `DATABASE_PATH`：已确定。
- `XY_DATABASE_PATH` 不保留兼容别名：已确定。
- 本轮只改数据库 env 名，不扩展到其他 `XY_*`：已确定。
- 生产现存 stray raw 文件不做自动迁移：已确定。
- 需要更新的对外文档至少包含 `README.md` 与 `docs/deployment.md`：已确定。

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Rust tests：覆盖 `DATABASE_PATH` 生效、`XY_DATABASE_PATH` fail-fast、raw 目录锚定数据库父目录、cwd 兼容读取旧相对路径、orphan sweep 不再依赖 cwd。
- 构建验证：`cargo fmt --all -- --check`、`cargo test --locked --all-features`、`cargo check --locked --all-targets --all-features`。

### Quality checks

- 文档中的数据库 env 示例全部切换到 `DATABASE_PATH`。
- 文档中所有“相对 archive/raw 目录”描述都明确以 `DATABASE_PATH` 为锚点。

## 文档更新（Docs to Update）

- `README.md`
- `docs/deployment.md`
- `docs/specs/README.md`

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 新增规格并冻结 `DATABASE_PATH` / fail-fast / 路径锚点语义。
- [x] M2: 后端配置解析切换到 `DATABASE_PATH`，并阻断 `XY_DATABASE_PATH`。
- [x] M3: raw/archive 相对路径统一锚定数据库目录，修复写入与 orphan sweep。
- [x] M4: Rust 回归测试与构建检查通过。
- [x] M5: 完成 fast-track 远端交付（push / PR / checks / review-loop 状态明确）。

## 方案概述（Approach, high-level）

- 用单一的“相对路径锚定到数据库目录”帮助函数收敛 raw 与 archive 路径语义，避免不同子系统继续各自拼 cwd。
- 对新写入路径使用数据库锚定后的真实目录；对历史路径读取/删除保留双候选兼容，避免一次修复把旧数据读断。
- 把数据库 env 改名做成显式 breaking change，并通过 fail-fast 消除“旧变量悄悄失效后落回默认库”的风险。

## 风险 / 开放问题 / 假设（Risks, Open Questions, Assumptions）

- 风险：仍在使用 `XY_DATABASE_PATH` 的部署会在升级后立即启动失败，因此 PR 与文档必须明确这是 breaking change。
- 风险：历史 stray raw 文件仍会留在容器层目录，后续若需要回收，应由独立运维步骤处理。
- 假设：当前生产与主要部署路径都能接受 `DATABASE_PATH` 这一命名，不需要再引入别名过渡。
- 开放问题：101 的最终 live rollout 依赖新镜像可部署；若 PR 阶段尚未合并发布，只能先完成 PR / review 收敛与预部署验证。

## 参考（References）

- `docs/specs/2uaxk-remove-xyai-legacy-ingest/SPEC.md`
- `docs/specs/9aucy-db-retention-archive/SPEC.md`
- `docs/specs/fq45q-startup-readiness-backfill-gating/SPEC.md`
