# Forward Proxy 新增后异步首轮探测补齐（#wv3m7）

## 状态

- Status: 已完成（3/3）
- Created: 2026-03-02
- Last: 2026-03-02

## 背景 / 问题陈述

- 当前 forward proxy 在新增节点后不会立即执行首轮探测，节点可能在“长期未探测”状态下直接进入调度。
- 这会在起步阶段放大失败风险，尤其是新增手工代理或订阅刷新引入的新节点。
- 需要在不阻塞保存接口的前提下，尽快产出节点首轮可用性信号与权重修正。

## 目标 / 非目标

### Goals

- 仅针对“新增 forward proxy 节点”异步触发 1 轮后台探测。
- 同时覆盖两条入口：设置保存后（手工新增）与订阅刷新后（新增订阅节点）。
- 探测成功/失败都持久化到 `forward_proxy_attempts`（`is_probe=1`）并同步 `forward_proxy_runtime` 权重。
- 复用现有探测路径 `probe_forward_proxy_endpoint` 与单条代理验证超时预算（5 秒）。

### Non-goals

- 不修改前端 API/交互，不新增实时进度 UI。
- 不将保存流程改为阻塞等待探测完成。
- 不新增“未测节点调度门禁”。

## 范围（Scope）

### In scope

- `src/main.rs`：新增“新增节点差异识别”与“异步首轮探测调度”内部 helper。
- `src/main.rs`：在 `put_forward_proxy_settings` 与 `refresh_forward_proxy_subscriptions` 接入首轮探测触发。
- `src/main.rs`：新增 Rust 回归测试（新增触发、无新增不触发、订阅新增触发、失败降权）。
- `docs/specs/README.md`：新增本规格索引。

### Out of scope

- 调整公开 HTTP 接口结构。
- 新增数据库表或列。

## 需求（Requirements）

### MUST

- 仅对新增节点触发异步首轮探测。
- 探测失败必须写入失败尝试记录并触发权重惩罚。
- `PUT /api/settings/forward-proxy` 响应结构与同源校验行为保持兼容。

### SHOULD

- 失败原因映射尽量复用既有 `forward proxy` 失败类型。

### COULD

- 记录触发来源日志（settings-update / subscription-refresh）便于排障。

## 功能与行为规格（Functional/Behavior Spec）

### Core flows

- 保存设置后若出现新增手工节点：后台异步触发首轮探测。
- 订阅刷新后若出现新增订阅节点：后台异步触发首轮探测。
- 探测成功：写 `is_probe=1` 成功记录并更新 runtime 权重。
- 探测失败：写 `is_probe=1` 失败记录并更新 runtime 权重。

### Edge cases / errors

- 无新增节点时不触发额外首轮探测。
- 探测异常不影响设置保存/订阅刷新主流程返回。

## 接口契约（Interfaces & Contracts）

### 接口清单（Inventory）

| 接口（Name）                                | 类型（Kind） | 范围（Scope） | 变更（Change） | 契约文档（Contract Doc） | 负责人（Owner） | 使用方（Consumers） | 备注（Notes）                      |
| ------------------------------------------- | ------------ | ------------- | -------------- | ------------------------ | --------------- | ------------------- | ---------------------------------- |
| `GET /api/settings`                         | HTTP API     | external      | Modify         | None                     | backend         | web                 | 响应字段不变，仅运行时统计更快出现 |
| `PUT /api/settings/forward-proxy`           | HTTP API     | external      | Modify         | None                     | backend         | web                 | 响应字段不变，新增后台异步首轮探测 |
| `POST /api/settings/forward-proxy/validate` | HTTP API     | external      | No-change      | None                     | backend         | web                 | 保持原行为                         |

### 契约文档（按 Kind 拆分）

- None

## 验收标准（Acceptance Criteria）

- Given 新增手工代理 URL，When 保存成功，Then 短时间内出现该节点 `is_probe=1` 的首轮尝试记录。
- Given 新增订阅并刷新出新节点，When 刷新完成，Then 新节点被异步触发首轮探测。
- Given 首轮探测失败（超时/5xx/连接失败），When 记录完成，Then 节点权重被惩罚且失败记录可查询。
- Given 未新增节点（如重复保存同一配置），When 保存，Then 不产生额外首轮探测记录。

## 实现前置条件（Definition of Ready / Preconditions）

- 目标、范围、验收标准已冻结。
- 现有 forward proxy 运行时权重更新逻辑可复用。

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Unit tests: 新增 forward proxy 首轮探测触发与失败惩罚回归测试。
- Integration tests: 复用本地测试 server 覆盖 settings/refresh 入口。

### Quality checks

- `cargo fmt`
- `cargo test`（至少覆盖新增 forward proxy 用例）

## 文档更新（Docs to Update）

- `docs/specs/README.md`: 新增索引并更新状态。

## 计划资产（Plan assets）

- None

## 资产晋升（Asset promotion）

- None

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 后端新增“新增节点差异识别 + 异步首轮探测”并接入设置保存入口
- [x] M2: 接入订阅刷新入口并保证无新增节点不触发
- [x] M3: 补齐回归测试与验证（成功触发/失败惩罚/无新增不触发）

## 方案概述（Approach, high-level）

- 以 endpoint key 做 before/after 差异识别，锁定“新增节点集合”。
- 新增异步探测入口，复用现有探测函数并统一通过 `record_forward_proxy_attempt(..., is_probe=true)` 落库。
- 探测失败不阻断主流程，仅记录日志和统计结果。

## 风险 / 开放问题 / 假设（Risks, Open Questions, Assumptions）

- 风险：短时间多次保存可能触发多轮异步探测；通过“仅新增节点触发”减少重复。
- 需要决策的问题：None。
- 假设（需主人确认）：保留“未测节点仍可参与调度”的现状。

## 变更记录（Change log）

- 2026-03-02: 初版规格创建，冻结异步首轮探测范围与验收口径。
- 2026-03-02: 完成后端实现与回归测试，新增 settings/refresh 触发的异步首轮探测闭环。

## 参考（References）

- `docs/specs/k52tw-forward-proxy-validation-allow-404/SPEC.md`
