# API Key 优先级迁移准入控制

> 当前有效规范以本文为准；实现覆盖与当前状态见 `./IMPLEMENTATION.md`，主题局部演进见 `./HISTORY.md`，持久决策的完整取舍见关联 ADR。

## 背景 / 问题陈述

现有 sticky 路由会让可复用的 `Fallback` 来源在每次请求时与更高优先级候选比较。目标在恢复、提升优先级或新近变为可用时，多个对话模型路由可以同时开始向同一目标尝试。sticky 写入虽只在成功后提交，却不限制这些并发尝试；一个不可靠而缓慢的目标因此可能在健康状态收敛前承接大量长请求。

优先级驱动的迁移与故障切换不同：前者发生时原来源仍然可用。因此，不能为等待迁移而阻塞客户端请求，也不能把数据库锁、持久队列或后台探测作为正确性前提。

## 目标 / 非目标

### Goals

- 对 API Key 目标的自动优先级迁移和新分配实行按目标账号与精确请求模型隔离的、无排队的准入控制。
- 让每个恢复中的目标组合逐个验证真实请求，并在失败后立即停止吸引更多新路由。
- 保持既有 `Fallback` 来源的优先级迁移范围、sticky 成功提交语义、人工绑定优先级和普通故障切换语义。
- 在数据库不可用时继续维持本进程的许可、冷却和恢复验证行为。
- 通过现有设置、路由审计和记录界面提供可解释且不泄露原始上游错误的运维证据。

### Non-goals

- 不把自动迁移扩大到 `normal` 或 `primary` sticky 来源。
- 不改变 OAuth 或其他非 API Key 账号的路由行为。
- 不改变 WebSocket 的选择、重试或会话完成语义。
- 不把闸门做成目标的绝对总请求限流器；已成功迁移的 sticky 路由继续正常发送。
- 不增加后台探针、持久 FIFO、跨进程租约恢复或数据库热路径读取。
- 不引入新的迁移请求时长阈值，也不改变既有 HTTP 请求超时合同。

## 范围（Scope）

### In scope

- HTTP 池路由中的 API Key `Fallback` sticky 优先级迁移。
- API Key 目标账号与精确请求模型组合的本地许可、冷却和三次成功恢复验证状态机。
- 受闸门影响的新分配绕行、单次迁移尝试、成功绑定、失败冷却和安全回放。
- 全局 Settings 开关、其本地运行时镜像，以及切换时的状态代际处理。
- 现有路由审计、模型路由事件、调用记录和设置 UI 的安全可观测性扩展。
- 后端并发、取消、重启、数据库故障隔离和前端设置/记录回归测试。

### Out of scope

- WebSocket 迁移准入、WebSocket 重试策略或长会话并发管理。
- OAuth 或非 API Key 的模型级闸门与冷却状态。
- 目标账号级全局许可证、跨实例分布式协调或持久化许可恢复。
- 将普通故障切换、人工强制绑定或既有 sticky 复用改为等待闸门。
- 修改已有模型映射、静态 `availableModels`、账号硬故障分类或常规超时设置。

## Related ADRs

- [ADR 0002: Stage automatic priority handoffs through local permits](../../adr/0002-stage-automatic-priority-handoffs.md)

## 需求（Requirements）

### MUST

- 自动优先级迁移的单位为一个对话模型路由：同一 sticky conversation key 与精确请求模型的组合。不同模型互不共享迁移结果或许可。
- 仅当非强制 sticky 来源仍可复用、有效优先级为 `Fallback`、允许自动 cut-out，且解析器已经选出严格更高优先级的首选 API Key HTTP 目标时，才可进入该目标的迁移闸门。
- 闸门键必须复用现有模型路由健康所使用的规范化请求模型键，键空间为“目标账号 + 精确请求模型”。账号模型映射在候选选定后处理，不得另建映射后模型的闸门键或别名聚合规则。
- 每个处于优先级吸引周期的目标键在同一进程内最多有一个在途闸门许可。许可不可等待、不可排队、不可依赖数据库、不可由后台任务预占。
- 目标因恢复、优先级提升或新近可用而开始吸引流量时，必须从恢复验证期开始；恢复验证期仅在三个连续的、闸门准入的自动迁移或新分配获得完整终态成功后开放普通优先级准入。
- 闸门准入的优先级迁移必须只发起一次目标请求，禁止同账号重试、429 重试、自动故障切换和对其他目标的重试。既有 HTTP 首字节与完整流超时保持不变。
- 只有目标请求的完整终态成功才可提交目标 sticky 绑定并释放许可。首字节、部分流、请求已发送或经过的时间都不得提交绑定。
- 目标未完整成功时，来源 sticky 绑定必须保持原样。只有能确定目标未收到请求时，才允许向仍可用的来源安全回放一次；交付不确定时必须返回目标错误，不得自动回放或改绑。
- 闸门许可被占用、目标处于冷却，或全局闸门状态不允许迁移时，当前 sticky 请求必须立即继续其可用来源，不得等待，也不得转向解析器首选目标之后的次优高优先级候选。
- 受闸门限制的新分配必须立即选择其他健康合法候选；没有替代候选时直接按现有无候选路径结束，不得等待许可。
- 已完整成功迁移的对话模型路由立刻将目标视为 sticky 来源；该路由后续请求不重新占用恢复许可。闸门限制新增目标准入，而非目标的绝对总流量。
- 同一对话模型路由的另一请求在首次迁移仍在途时必须继续原来源，不得等待、合并或启动第二次迁移。
- 客户端取消必须立即释放本地许可，不得写入迁移失败、不得安全回放、不得修改 sticky 绑定。
- 迁移中的既有临时模型级失败类别必须使本地闸门立即进入现有模型路由冷却阶梯的第一档；后续这类迁移失败按同一阶梯推进。完整终态成功重置该组合的迁移失败证据。客户端取消和调用方校验错误不得改变此状态；模型特定及账号级硬错误继续执行既有健康行为。
- 本地闸门状态必须独立执行冷却和恢复验证。持久化模型路由健康与诊断事件可以最佳努力写入；写入失败不能阻塞许可获取、释放、延期迁移或来源继续发送。既有普通路由的数据库行为不因本功能改变。
- 冷却到期后的下一次准入必须来自真实请求并按既有候选资格处理；不得发送后台探针。人工模型健康 reset 同样只能重新进入串行恢复验证，不能直接全面开放。
- 普通故障切换不获取迁移许可，也不等待迁移许可，但仍遵守既有账号模型健康资格。人工强制绑定始终在迁移闸门之外。
- 进程重启必须丢弃许可与本地恢复计数，不恢复持久租约；随后新的 API Key 优先级移动必须重新进行恢复验证才可全面开放。
- 全局 `priorityHandoffAdmissionEnabled` 设置默认开启。它通过既有 Settings UI 与 `GET/PUT /api/pool/routing-settings` 管理；持久化只保存期望值，路由热路径只读取本地运行时镜像。设置写入失败时保留最后已生效镜像，现有请求不受影响。
- 全局开关关闭时必须回退到当前旧的自动 `Fallback` 迁移和新分配行为。重新开启时必须创建新的本地状态代际，旧代际在途请求可完成但不得贡献新的恢复验证计数。
- 路由审计和模型路由事件必须以最佳努力方式记录安全的迁移准入、延期、恢复进度、成功和冷却原因；不得把原始上游响应体、凭据或未规范化错误文本写入新字段。

### SHOULD

- 新的运行时状态应集中在一个小而显式的进程本地模块，并通过 RAII 或等价的取消安全清理保证许可释放。
- 候选选择应先遵守现有资格、模型健康、绑定与优先级比较，再执行闸门准入；闸门不应改写常规比较器的排序规则。
- 设置界面应明确显示这是全局 HTTP/API Key 的优先级迁移控制，不暗示它会关闭 WebSocket、人工绑定或所有目标流量。
- 审计应使用已有账号显示名与安全原因码，避免把内部数值标识当作运维界面的账号名称。

### COULD

- 在既有实况路由视图中显示受限组合的恢复进度摘要，只要它复用现有数据投影且不增加独立轮询。
- 为运行时日志补充低基数的阶段/决策字段，辅助没有持久化审计时的诊断。

## 功能与行为规格（Functional/Behavior Spec）

### Core flows

1. 解析器按既有规则得到一个可复用 `Fallback` sticky 来源和一个严格更高优先级的首选 API Key HTTP 目标。
2. 若全局开关开启，系统读取目标键的本地状态。处于验证期且许可空闲时，当前真实请求获得唯一许可并作为单次迁移尝试发送；处于开放期时，按普通优先级准入发送。
3. 目标完整终态成功时，现有 generation-guarded sticky 成功写入将绑定改为目标。系统释放许可，记录成功，并将该闸门准入的成功计入连续验证数。
4. 三次连续的合格成功后，目标键在当前吸引周期对新增迁移和新分配开放。已迁移路由的后续请求从第一次成功起就一直正常发送。
5. 目标发生可归属的临时失败时，本地状态立即进入模型路由冷却；当前来源绑定不变。下一次候选仅在冷却过期并通过普通资格检查后才能成为真实验证请求。
6. 当目标不能准入时，sticky 请求直接保留来源；新分配转向其他健康候选或直接结束。两条路径均不创建 HTTP 等待队列。

### Edge cases / errors

- 首选目标许可被占用时，sticky 路由不会扫描到较次的高优先级目标，因为一旦成功迁到次优目标，该路由将不再符合自动 `Fallback` 迁移范围。
- 目标已经收到请求但在终态前断开、超时或返回失败时，交付不确定；来源不重放，sticky 不变，临时失败则进入冷却。
- 全局开关关闭或重新开启不会取消已向目标发送的请求。切换代际会阻止旧在途结果改变新一代的恢复计数。
- 数据库或审计写入不可用时，本地许可、冷却、取消释放、来源继续发送和全局镜像保持可用；仅持久化诊断或健康同步可能延后。
- 多实例未来可分别持有本地许可；本主题不承诺跨实例串行。当前正确性不依赖跨实例数据库锁。

## 接口契约（Interfaces & Contracts）

### 接口清单（Inventory）

| 接口（Name）                     | 类型（Kind）            | 范围（Scope） | 变更（Change） | 契约文档（Contract Doc） | 负责人（Owner）          | 使用方（Consumers）         | 备注（Notes）                          |
| -------------------------------- | ----------------------- | ------------- | -------------- | ------------------------ | ------------------------ | --------------------------- | -------------------------------------- |
| `GET /api/pool/routing-settings` | HTTP JSON               | external      | Modify         | 本节                     | backend routing settings | Settings UI                 | 返回全局开关的当前有效值               |
| `PUT /api/pool/routing-settings` | HTTP JSON               | external      | Modify         | 本节                     | backend routing settings | Settings UI                 | 更新持久配置与本地镜像；失败不部分应用 |
| `routingSelectionAudit`          | persisted/read API JSON | external      | Modify         | 本节                     | routing/records          | Records、对话事件、实况路由 | 增加可选的安全迁移准入快照             |
| 模型路由事件 reason code         | persisted/read API JSON | external      | Modify         | 本节                     | model routing            | Records、实况路由           | 记录迁移成功、冷却和恢复结果           |

### 契约文档（按 Kind 拆分）

- `GET /api/pool/routing-settings` 与 `PUT /api/pool/routing-settings` 增加 `priorityHandoffAdmissionEnabled: boolean`。缺失的历史值按 `true` 解释；`PUT` 按既有局部设置更新语义处理该字段。
- 成功的 `PUT` 先完成持久化，再原子更新本地镜像。失败响应不得改变运行时镜像；运行中的请求继续使用其开始时读取的代际。
- `routingSelectionAudit` 可选增加 `handoffAdmission`：`decision` 为 `admitted`、`deferredPermitBusy`、`deferredCooldown` 或 `bypassedDisabled`；`phase` 为 `verifying`、`open` 或 `coolingDown`；`verificationSuccessCount` 为 `0..=3`。历史记录可省略该对象。
- 现有模型路由事件新增安全 reason code，以表达 `priorityHandoffSucceeded`、`priorityHandoffFailureCooldown` 与 `priorityHandoffRecoveryProgress`。事件仅携带既有安全上下文和模型范围，不携带原始上游错误。

## 验收标准（Acceptance Criteria）

- Given 多个可用 `Fallback` 对话模型路由同时将同一 API Key 目标视为首选，When 目标处于恢复验证期，Then 同一进程最多一个请求向该目标发送，其他 sticky 请求立即在各自来源继续，没有迁移 FIFO 或 HTTP 等待。

- Given 首次闸门准入的迁移请求完整终态成功，When 另一个来源路由随后请求，Then 该成功才会提交第一个 sticky 绑定，并且下一次真实请求才可获得下一个恢复许可。

- Given 目标返回可归属的临时失败或发生传输失败，When 迁移尝试结束，Then 目标组合立即进入既有模型路由冷却阶梯，来源绑定保持不变，且本次不执行任何自动重试。

- Given 目标请求的交付状态不确定，When 请求失败或超时，Then 系统把目标错误返回客户端且不向来源回放；Given 已证明目标未收到请求，Then 最多安全回放一次到原来源。

- Given 恢复期已有成功迁移的路由，When 它发出后续请求，Then 该请求直接使用新 sticky 目标而不等待许可；三次验证只限制新的迁移和新分配准入。

- Given 首选目标许可被占用但存在较次的高优先级目标，When 一个 `Fallback` sticky 路由请求，Then 它继续原来源而不迁移到较次目标。

- Given 新分配的首选目标不能准入，When 仍存在健康合法候选，Then 选择替代候选；When 不存在替代候选，Then 直接走无候选结果而不等待。

- Given 客户端在迁移尝试中取消，When drop/cancellation 发生，Then 许可立即释放，来源绑定和健康证据均不因取消改变。

- Given 数据库写入或审计持久化失败，When 迁移许可、冷却或恢复验证需要转换，Then 本地状态照常转换、请求不等待数据库；只有持久化诊断可能缺失。

- Given 全局开关关闭，When 新请求解析路由，Then 使用升级前的自动 `Fallback` 迁移与新分配语义；Given 重新开启，Then 新代际从恢复验证开始，旧代际的在途结果不污染新代际。

- Given WebSocket 或非 API Key 账号请求，When 解析路由，Then 它们维持升级前的路由、重试和完成语义。

## 验收清单（Acceptance checklist）

- [ ] 核心路径的长期行为已被明确描述。
- [ ] 关键边界、取消、交付不确定、数据库不可用与进程重启场景已被覆盖。
- [ ] Settings 与审计接口契约已写清楚。
- [ ] 相关验收条件可以用于实现与 review 对齐。

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Rust unit tests：本地闸门状态机、状态代际、冷却推进/重置、取消释放与安全回放判定。
- Stateful SQLite tests：`Fallback` 首选迁移、同目标并发、次优目标不迁移、新分配绕行、sticky generation 防陈旧覆盖、既有模型路由健康兼容和数据库故障隔离。
- HTTP integration coverage：单次迁移尝试禁止各类自动重试、完整终态才绑定、交付不确定不回放、标准超时不被重写。
- Compatibility tests：WebSocket、OAuth、人工绑定、普通故障切换和全局开关关闭时保持既有行为。

### UI / Storybook

- 在既有 Pool Routing Settings 卡片增加全局开关的 enabled/disabled、保存失败和窄屏状态。
- 更新 Settings API Vitest 覆盖、翻译文案和 Records/路由审计的迁移准入显示状态。
- 为修改过的设置卡与记录卡补充 Storybook `play` 覆盖和受控桌面/移动端视觉证据。

### Quality checks

- `cargo fmt --all -- --check`
- `cargo check --locked --all-targets --all-features`
- 相关 Rust 目标测试，必要时使用仓库约定的 `stateful-sqlite` profile。
- `cd web && bun run test`
- `bun run typecheck:web`
- `bun run lint:web`
- `cd web && bun run build`
- 文档与 spec drift 检查。

## Visual Evidence

当前无 UI 实现；实现交付时按 UI visual evidence 合同补充受控证据。

## Related PRs

- None

## 风险 / 开放问题 / 假设（Risks, Open Questions, Assumptions）

- 风险：本地许可不跨多实例；当前部署为单进程，未来横向扩展前必须另行设计跨实例协调或接受每实例一个验证请求的边界。
- 风险：成功迁移的单一高频对话可继续向目标发送，请求闸门不试图把它变成总流量限额。
- 假设：现有模型路由健康键与候选缓存继续提供稳定的规范化请求模型身份；模型映射不改变此身份。
- 假设：全局设置持久化失败时，保留的运行时镜像足以让已启动进程继续服务；设置修改本身可以返回既有错误。

## 参考（References）

- `../../adr/0002-stage-automatic-priority-handoffs.md`
- `../zr9jd-api-key-model-routing-health/SPEC.md`
- `../r4p9x-upstream-account-policy-inheritance/SPEC.md`
- `../../../CONTEXT.md`
