# Codex Vibe Monitor Context

This context freezes the project-specific terms used in invocation observability and release automation. It exists to keep transport paths, request semantics, response outcomes, and publication surfaces from drifting into overloaded labels.

## Invocation Compaction

**Compact**:
The legacy dedicated compaction request path backed by `/v1/responses/compact`.
_Avoid_: 远程压缩V1, remote compaction v1, `/v1/responses` compaction

**远程压缩V2**:
Server-side compaction semantics that run inside `/v1/responses` without changing the transport endpoint.
_Avoid_: Compact, `/v1/responses/compact`, normal Responses badge

**压缩请求**:
The request-side declaration that a `/v1/responses` call enabled remote compaction V2 semantics.
_Avoid_: 压缩响应, 最终已压缩

**压缩响应**:
The response-side proof that the upstream actually emitted a compaction item for the invocation.
_Avoid_: 压缩请求, 已启用压缩

## Invocation Context Metadata

**推理强度（Reasoning Effort）**:
The request-side computation-effort value captured with an invocation from
`reasoning.effort` or `reasoning_effort`. It is distinct from the model identity,
FAST state, and the measured count of reasoning Tokens.
_Avoid_: 思考状态, 推理 Tokens, 模型等级

**模型上下文簇**:
The bounded read-only Dashboard group that presents a model identity with its
available 推理强度 and FAST state. Its separators are presentation-only and do not
alter the underlying invocation metadata.
_Avoid_: 模型标签, 路由上下文, 调用详情

**缺失推理强度**:
An invocation for which capture supplied no usable 推理强度 value. It does not mean
zero, neutral, or default effort; compact Dashboard metadata omits the absent
field rather than presenting a placeholder as a real value.
_Avoid_: 中性推理, 默认推理强度, 零推理

## Invocation Timing

**请求用时**:
The duration from an invocation entering the proxy to the first upstream response byte.
_Avoid_: 连接用时, TTFT, 总耗时

**TTFT**:
The duration from an invocation entering the proxy to its first valid model-output delta.
_Avoid_: TTFB, 首响应, 首字节耗时

**暂估 TTFT**:
The local, non-authoritative elapsed value after the first upstream response byte arrives and
before the first valid model-output delta is observed. It is not a measured TTFT.
_Avoid_: 已测 TTFT, 首响应时间

**缺失 TTFT**:
A terminal invocation with no valid model-output delta from which TTFT can be measured. It is
neither zero nor a substitute first-response measurement; the compact presentation uses a dash.
_Avoid_: TTFT 为零, 首响应, TTFB 回退

**响应耗时**:
The duration from the first upstream response byte to the end of that upstream stream.
_Avoid_: 总耗时, TTFT, 代理处理耗时

## Runtime Read Models

**Summary Projection**:
An exact, immutable in-memory `StatsResponse` read model for a validated Summary
selection. It is hydrated and reconciled outside the HTTP request path; it never
stands for an approximate aggregate or a request-time SQLite/file fallback.
_Avoid_: partial summary, zero fallback, request-time summary rebuild

**Canonical Source Record Admission**:
The bounded background acceptance of raw durable record text required to construct
an exact Summary Projection. It describes source work and coverage proof, not
resident Projection memory.
_Avoid_: resident payload budget, request-time source fallback

**Resident Preview Byte Budget**:
The shared cap on compact preview values retained by rolling and current Summary
views. Raw source payload text that is not retained is outside this budget.
_Avoid_: source scan quota, raw payload cap

**Range-Local Unavailable**:
The exact-unavailable result for only a Summary selection whose required source
coverage cannot be proven. Disjoint selections may remain available from the same
Projection.
_Avoid_: partial response, global hydration failure

**Exact Boundary**:
The temporal or current-rank source limit that separates proven coverage from an
unavailable Summary selection.
_Avoid_: approximate rollup edge, request-time repair

**Recoverable Projection State**:
A published Summary Projection that records an exact source-coverage gap as
range-local unavailable until a later hydration proves coverage. It never
represents incomplete coverage as a fresh result.
_Avoid_: fabricated success, permanent global outage

**Bootstrap Projection**:
The first immutable Summary Projection published after startup. It contains the
exact `current` legal prefix and every independently proven rolling/calendar
selection, but deliberately does not wait for all-time archive reconciliation.
_Avoid_: partial all-time response, global cold-start failure

**Projection Generation Fence**:
The stable live ID, global/account rollup cursors, archive manifest
high-watermark, and settled terminal sequence observed for one background
Projection pass. A later generation never extends an older coverage claim; it
starts a new bounded reconciliation from its own fence.
_Avoid_: mixed-source snapshot, implicit catch-up

**Projection Freshness Renewal**:
The in-memory extension of an already Exact-Ready Projection only after its
current Projection Generation Fence still matches durable state. It does not
publish new source data, weaken an unavailable boundary, or make an unready
all-time selection ready.
_Avoid_: stale-source renewal, hidden full refresh, broader stale budget

**All-Time Coverage Checkpoint**:
The durable per-scope seek cursor for bounded manifest-proof reconciliation at a
Projection Generation Fence. It advances only after a whole page has exact
materialization and replay evidence, and a restart resumes from its committed
cursor.
_Avoid_: full-history startup retry, in-memory-only progress

**Exact-Ready**:
The state in which a requested Summary selection has all required source and
boundary proof in the published Projection. `all` becomes Exact-Ready only after
its own coverage checkpoint and final exact aggregate complete.
_Avoid_: approximate readiness, coupled rolling availability

**Archive Publication Proof**:
The durable certificate that an immutable `codex_invocations` archive has
finite coverage and a current manifest identity whose required Summary rollups
are exactly represented. A completed archive is Summary-eligible only when
this proof becomes visible atomically with completion; an older archive obtains
it only through automatic identity and source-closure reconciliation.
_Avoid_: materialized timestamp as proof, marker-only repair, eventually
consistent completed archive

**Summary-Eligible Archive**:
A completed invocation archive with Archive Publication Proof. It may supply
durable Summary rollups without request-time archive access.
_Avoid_: merely completed archive, readable archive, best-effort replay

**Authoritative Archive Source**:
An invocation archive whose records have left `codex_invocations`. It must carry
Archive Publication Proof before becoming completed and is eligible to supply
Summary coverage.
_Avoid_: detail mirror, optional replay source, completed file only

**Live Detail Mirror**:
An archive emitted while pruning invocation payload details even though each
canonical invocation remains in `codex_invocations`. A legacy mirror is certified
only by a current-SHA, complete archive-to-live `(id, invoke_id)` identity proof;
an encoded ID interval is only a fast compatibility shortcut. It preserves
observability only and is never a Summary, rollup-repair, or archive-admission
source.
_Avoid_: historical Summary source, second canonical copy, proof backlog

**Unknown Legacy Archive Source**:
A pre-role manifest whose source relationship cannot be proven automatically.
It remains fail-closed as a potential authoritative source until bounded
reconciliation proves it is a Live Detail Mirror through complete archive/live
identity coverage.
_Avoid_: assumed mirror, skipped source, marker-only classification

**Legacy Detail Mirror Identity Proof**:
The background certificate that the current archive SHA and row count match a
complete page-by-page `(id, invoke_id)` correspondence with live canonical rows.
It may classify an otherwise ambiguous legacy archive as a Live Detail Mirror;
any missing, changed, unreadable, or replaced record leaves the archive unknown.
_Avoid_: interval-density proof, count-only classification, Summary HTTP scan

**Summary Startup Recovery Gate**:
The bounded, cancellation-aware cold-start sweep that captures one stable
unknown-legacy-manifest high-watermark, completes exact Live Detail Mirror
identity attempts ahead of the first Summary Projection build, and persists only
proven classifications. An unresolved source remains unknown; it cannot block an
independent proof or be guessed into a mirror role. Periodic Summary maintenance
starts only after this gate has allowed an exact Projection to publish.
_Avoid_: generic-backfill starvation, global cold-start retry loop, inferred mirror

**Last-Good Snapshot**:
The most recent exact read-model value that remains internally retained while a
background refresh is unavailable. Its retention does not permit a stale or
partial value to be represented as a fresh projection.
_Avoid_: fabricated empty summary, implicit fresh cache

**Canonical Invocation Classification**:
The revisioned durable outcome fact for one terminal invocation: its failure
class and actionable state. It is written once by terminal persistence or by a
controlled compatibility materializer and is the only classification source for
Summary, rollup and aggregate readers.
_Avoid_: payload-derived reader classification, window-specific failure class

**Classification Coverage**:
Durable proof that a live or archived record range has current Canonical
Invocation Classification. A reader with incomplete coverage is exact-unavailable
rather than allowed to infer a result from raw payload bytes.
_Avoid_: implicit legacy fallback, partial exactness

**Archive Classification Overlay**:
The durable identity-keyed Canonical Invocation Classification for a record in an
immutable archive. It avoids rewriting archive files while making their outcome
available to a background projector or rollup builder.
_Avoid_: mutable archive, request-time archive repair

**SQLite Pressure Defer**:
A deliberate refusal of low-priority background database work before it acquires
SQLite because the shared pressure gate is closed. It leaves durable progress
unchanged and has one in-memory scheduler eligibility deadline plus an
event/deadline wake; it is not a failed database operation.
_Avoid_: lock retry, millisecond polling, work-completed audit

**SQLite Lock Failure**:
An actual SQLite `BUSY` or `LOCKED` result after work attempts database access.
It follows the operation's bounded error backoff and is distinct from a pressure
defer.
_Avoid_: pressure defer, successful no-op

## Dashboard 上游账号活动

**统计卡片**:
Dashboard「上游账号」视图中呈现单个账号聚合指标的卡片，包含 TTFT、请求数、成本与 Token 等，每张卡片都有一个主值。
_Avoid_: 指标块, KPI 卡, 小卡片

**紧凑主值**:
统计卡片中按可用宽度自适应呈现的主数值文本；可从完整值切换为带量级的短文本，但不改变原始聚合值。
_Avoid_: 截断值, 省略值

**指标语义量级**:
紧凑主值按指标类型使用的单位体系：计数和 Token 使用 K/M/B/T，成本使用 $K/$M/$B/$T，耗时使用 ms/s/min/h。
_Avoid_: 通用单位, K 秒

**精度预算**:
紧凑主值默认保留三位有效数字；可用宽度不足时逐级降低精度，并在舍入进位跨越量级时同步升级单位。
_Avoid_: 固定小数位, 1000K

**常驻指标值**:
统计卡片在卡面默认可见的主值与明细值；二者均服从紧凑主值合同，标识符、时间戳与百分比不属于此类。
_Avoid_: Tooltip 明细, 原始数据

**可用内容宽度**:
统计卡片扣除内边距、图标及间距后，数值文本实际可占用的单行宽度；它不是固定视口或卡片断点。
_Avoid_: 窄卡片阈值, 固定像素宽度

## Release Automation

**Release 正文**:
GitHub Release 页面中面向使用者的发布说明；只包含明确的用户向 release notes，不混入流程元数据。
_Avoid_: PR 元数据, 发布审计, CI 诊断

**自动生成发布说明**:
基于相邻发布 tag 之间变更生成的用户向 Release 正文；它是当前 Release 正文的来源。
_Avoid_: PR 正文转抄, 流程元数据拼接

**发布决策快照**:
与主线提交绑定的不可变自动发布决策记录，保存版本意图与发布计算所需字段；手工覆盖字段只存在于本次 workflow 的临时快照，不写入该记录。
_Avoid_: Release 正文, 手工覆盖审计, 公开变更说明

**PR 发布评论**:
附在源 PR 上的版本交付追溯记录；它独立于 GitHub Release 页面。
_Avoid_: Release 正文, 发布说明
## Routing Affinity

**优先级迁移（Priority Handoff）**:
An automatic, non-forced attempt to move one sticky conversation from its current eligible `Fallback` upstream to a higher-priority eligible upstream. The source binding remains authoritative until the target attempt succeeds.
_Avoid_: 故障切换, 强制绑定, 立即换号

**HTTP 优先级迁移范围（HTTP Handoff Scope）**:
The transport boundary in which the handoff admission gate applies: HTTP pool requests only. WebSocket routing, retry, and session-completion behavior remain unchanged.
_Avoid_: WebSocket 同步改造, 跨传输隐式复用, 长会话迁移锁

**延期优先级迁移（Deferred Priority Handoff）**:
A priority handoff whose selected highest-ranked target is not admitted for the current request; the request continues on its eligible source upstream rather than migrating to a lower-ranked target, and is never held awaiting the transfer.
_Avoid_: 请求排队, 等待迁移, 阻塞对话

**对话模型路由（Conversation-Model Route）**:
The exact pair of a sticky conversation and its requested model. It is the unit of an automatic priority handoff; another model in the same conversation is independent.
_Avoid_: 整个对话换号, 账号级迁移

**闸门模型键（Handoff Gate Model Key）**:
The target account paired with the same normalized requested-model key used by current model-route health. Model mapping occurs after candidate selection and does not create an independent gate key or alias aggregation.
_Avoid_: 映射后模型键, 别名合并闸门, 账号全局闸门

**迁移准入闸门（Handoff Admission Gate）**:
The automatic admission control for a target API Key account-model during recovery. It governs priority handoffs and fresh assignments, while an operator-forced binding remains outside the gate; non-API-Key targets retain their existing routing behavior.
_Avoid_: 请求队列, 人工绑定拦截, 全账号类型改造, 全局模型锁

**优先级吸引周期（Priority Attraction Epoch）**:
The period beginning when a target becomes a higher-priority eligible choice through recovery, a priority change, or becoming newly eligible. Automatic handoffs and fresh assignments enter through the handoff gate until its stability policy opens the target.
_Avoid_: 永久串行, 仅故障恢复, 账号全局周期

**恢复验证期（Recovery Verification Phase）**:
The serialized portion of a priority attraction epoch after a model-route cooldown expires or an operator resets health. It requires three consecutive complete terminal successes from gate-admitted automatic priority handoffs or fresh assignments before ordinary priority admission resumes; a failed admitted attempt immediately returns the exact target account-model pair to its model-route cooldown. A route already rebound by a successful handoff continues directly on its new sticky target; the gate controls new target admission rather than all target traffic.
_Avoid_: 一次成功即全面开放, 固定等待时长, 无限制恢复流量

**机会式优先级迁移（Opportunistic Priority Handoff）**:
A handoff whose next candidate is the next eligible real request, not a durable FIFO list of conversations.
_Avoid_: 严格迁移队列, 后台迁移任务

**迁移确认（Handoff Confirmation）**:
A complete terminal success from the target request. It commits that target as the conversation-model route's new sticky upstream and releases the handoff permit; partial output or elapsed time does not change the binding.
_Avoid_: 首字节成功, 请求已发出, 目标已选中, 时间阈值

**单次迁移尝试（Single-Attempt Handoff）**:
The one target-upstream request made under a handoff permit. It never enters automatic retry.
_Avoid_: 同账号重试, 429 重试, 自动故障切换

**迁移许可（Handoff Permit）**:
The process-local exclusive permission held by one single-attempt handoff while an automatic migration is being evaluated. Optional database coordination is secondary and never blocks acquiring or releasing it; client cancellation releases the permit without changing the source binding or recording a target failure.
_Avoid_: HTTP 请求锁, 必需数据库锁, 持有至超时, 取消即失败, 全局上游锁

**迁移失败冷却（Handoff Failure Cooldown）**:
The immediate model-route cooldown entered by an exact target API Key account-model pair after a terminal failed priority handoff. It uses the ordinary model-route failure streak and cooldown ladder from its first failure; later failed handoffs escalate it and a complete terminal success resets it. The pair is ineligible wherever ordinary model health excludes a cooling route, while the source binding remains authoritative.
_Avoid_: 只降权, 对话级冷却, 账号全局冷却, 非 API Key 扩展, 独立冷却序列, 立即重试

**临时模型级迁移失败（Temporary Model-Scoped Handoff Failure）**:
A terminal priority-handoff failure in the existing temporary account-model failure classes, including retryable upstream overload and transport-path failures. It enters handoff failure cooldown; client cancellation and caller validation errors do not, while model-specific and account-scoped hard failures retain their ordinary health behavior.
_Avoid_: 所有非成功, 客户端取消, 调用方错误, 账号级硬错误

**安全回放（Safe Source Replay）**:
One replay of a failed single-attempt handoff to its still-authoritative source upstream, permitted only when the system can establish that the target did not receive the request.
_Avoid_: 无条件重试, 跨账号重试, 失败后必定回源

**并发回源（Concurrent Source Continuation）**:
The behavior for another request of a conversation-model route while its priority handoff is in flight. The later request continues immediately on the authoritative source and does not wait for or join the target attempt.
_Avoid_: 对话请求排队, 等待迁移, 并发迁移

**易失迁移许可（Ephemeral Handoff Permit）**:
A handoff permit whose lifetime is confined to the current process. Process restart discards it rather than recovering it from persistent storage, and new priority movement starts recovery verification before unrestricted admission resumes.
_Avoid_: 持久锁, 启动恢复锁, 重启即全面开放, 数据库依赖许可

**受控人工重开（Controlled Manual Re-entry）**:
An operator health reset that clears model-route cooldown but starts recovery verification rather than immediately restoring unrestricted priority admission.
_Avoid_: 重置即全量开放, 必须等待自动故障切换

**新分配绕行（Fresh Assignment Bypass）**:
Selecting another healthy eligible upstream for a new conversation while a preferred target's handoff gate is occupied. If no alternative exists, the request terminates without waiting for the permit.
_Avoid_: 等待迁移, 绕过闸门, 并发恢复

**故障切换（Fault Failover）**:
The existing recovery path for a request whose assigned upstream has actually failed. It is not a priority handoff and does not enter the priority-handoff gate, but continues to observe ordinary account-model health eligibility.
_Avoid_: 优先级迁移, 等待迁移许可, 原上游可用时的迁移

**无阻断迁移审计（Non-Blocking Handoff Audit）**:
Best-effort diagnostic records on the existing routing-audit path. They use safe structured reason codes and recovery progress for handoff admission, deferral, verification, and cooldown; a persistence failure never changes routing, permit acquisition, or release.
_Avoid_: 审计数据库锁, 诊断失败即拒绝请求, 原始上游错误泄露

**全局本地镜像迁移开关（Globally Mirrored Handoff Switch）**:
One operator-controlled global setting exposed through the existing settings surface and enabled by default. Persistent storage holds the desired configuration, while routing reads a process-local runtime mirror; a database outage cannot block active request routing, permit transitions, or the last known switch state. Disabling restores the pre-gate routing behavior; re-enabling starts a new local verification generation without cancelling an in-flight request.
_Avoid_: 热路径查数据库, 数据库不可用即停流, 按账号模型开关, WebSocket 开关
