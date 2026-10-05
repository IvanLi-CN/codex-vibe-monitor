# Codex Vibe Monitor Context

This context freezes the project-specific terms used in invocation observability and release automation. It exists to keep transport paths, request semantics, response outcomes, and publication surfaces from drifting into overloaded labels.

## Invocation Identity

**对外调用（Invocation）**:
One logical unit of proxy-observed work counted once regardless of upstream retries. Its queue, upstream work, and terminal outcome share one invocation identity. Legacy WebSocket turns remain valid historical invocations, but WebSocket is no longer a live client-facing transport.
_Avoid_: 上游尝试, 重试次数

**提示缓存键（Prompt Cache Key）**:
The normalized request key used to attribute invocations to a prompt-cache conversation. It is distinct from the service-assigned Conversation ID.
_Avoid_: 对话 ID, 调用 ID

**提示缓存对话（Prompt-cache Conversation）**:
The retained grouping of invocations attributed to one Prompt Cache Key. Its identity and invocation statistics share a lifecycle governed by retained invocation records.
_Avoid_: 上游会话, 上游账号, 展示分组

**对话 ID（Conversation ID）**:
The service-assigned identity of a retained Prompt-cache Conversation. It supplies the common prefix of that conversation's Invocation IDs.
_Avoid_: Prompt Cache Key, 哈希展示值, 调用 ID

**近期活跃对话（Recently Active Conversation）**:
A Prompt-cache Conversation with at least one Invocation starting within the preceding 48 hours. This activity window is used to estimate allocation cache capacity and is distinct from having an invocation currently in progress.
_Avoid_: 在途对话, 正在调用的对话

**调用 ID（Invocation ID）**:
The identity of one Invocation, shared by all of its Upstream Attempts. It combines an allocation prefix with an Invocation Sequence.
_Avoid_: 上游尝试 ID, 对话 ID

**调用序号（Invocation Sequence）**:
The ordered value used within a conversation or an unbound allocation prefix to distinguish Invocation IDs. Skipped or reserved values do not represent completed invocations and must not be counted as usage.
_Avoid_: 调用数, 已完成调用数

**预占号段（Reserved Invocation Range）**:
A contiguous set of Invocation Sequences durably claimed before they may be issued. Reservation does not mean an Invocation has occurred, and unused values may be skipped after a restart.
_Avoid_: 已发出的调用, 调用计数

**预占上限（Reserved Invocation Ceiling）**:
The durable upper boundary of Invocation Sequences claimed for an allocation prefix, including issued sequences and outstanding reservations. It is distinct from the last issued sequence and the number of invocations.
_Avoid_: 最近调用序号, 调用数

**备用号段（Standby Invocation Range）**:
A Reserved Invocation Range confirmed for use after the current range is consumed. A refill that has not committed is not a standby range.
_Avoid_: 待提交号段, 当前可发号段

**退号（Reservation Return）**:
Releasing the never-issued trailing sequences of a Reserved Invocation Range so future allocation may claim them again. It does not release an issued Invocation ID, including one whose invocation was cancelled.
_Avoid_: 已发 ID 复用, 调用回滚

**无对话调用（Unbound Invocation）**:
An Invocation with no Prompt Cache Key and therefore no Prompt-cache Conversation attribution. It still has an Invocation ID and may use any eligible upstream account.
_Avoid_: 无账号调用, 匿名调用

**小时调用前缀（Hourly Invocation Prefix）**:
The identity shared by Unbound Invocations starting within one UTC hour. An invocation retains its assigned prefix when it continues across an hour boundary.
_Avoid_: 对话 ID, 每次调用随机前缀
**传输拒绝（Transport Rejection）**:
A client request rejected at the transport boundary before it enters account routing, upstream selection, invocation persistence, or retry handling. A retired transport produces an observable protocol error and structured telemetry, but does not create a synthetic invocation.
_Avoid_: 失败调用, 上游失败, 合成调用

**遗留状态（Legacy State）**:
Persisted configuration, tags, or historical records retained for migration safety, auditability, or read compatibility after the corresponding live capability has been removed. Legacy state is not a source of runtime capability truth.
_Avoid_: 当前能力开关, 活跃配置, 运行时路由依据

**上游尝试（Upstream Attempt）**:
One attempt to send an invocation to an upstream route or account. An invocation may contain several upstream attempts, so attempt counts do not equal invocation counts.
_Avoid_: 对外调用, 调用次数

**调用时间线（Invocation Timeline）**:
The target Dashboard read model for the natural-day count view. It renders one horizontal bar for each logical Invocation and keeps time, concurrency, duration, terminal state, and TTFT in one shared chart surface.
_Avoid_: 旧聚合图, 调用次数柱状图

**自然日数据域（Natural-day Data Scope）**:
The selected browser-local today or yesterday interval represented by UTC start and end bounds. It limits which timeline data may be displayed to one natural day; it is not the same as the shorter visible viewport.
_Avoid_: 滚动三十天, 默认视窗, 历史全量

**时间线视窗（Timeline Viewport）**:
The visible `from`/`to` interval inside the natural-day data domain. It defaults to 30 minutes and may move or zoom within that domain without changing the selected day.
_Avoid_: 数据域, 查询历史范围, 自由时间范围

**边界重叠（Boundary Overlap）**:
The bounded preceding-day read used to include an Invocation that began before the selected natural day and ended inside it. It is limited to one day and never expands the displayed data domain.
_Avoid_: 三十天回溯, 第二天图表, 上游重试

**时间线明细不可用（Timeline Detail Unavailable）**:
A visible state of the Invocation Timeline when detailed records cannot currently be read or completed. It is not zero calls, not an aggregate substitute, and not permission to mount the legacy chart.
_Avoid_: 空数据, 聚合回退, 数据转换中

**时间线快照（Timeline Snapshot）**:
The consistent set of Invocation records represented by one chart read, identified by its `asOf` point. Later records belong to a later snapshot and must not be mixed into the current one.
_Avoid_: 实时流片段, 分页临时结果, 旧图快照

**时间线实时修订（Timeline Live Revision）**:
The authoritative signal that today's Invocation data changed and the current Timeline Snapshot should be refreshed. It is not an Invocation, an Upstream Attempt, or a count of changes.
_Avoid_: 重试次数, 调用次数, 每条调用事件

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

**模型代次（Model Generation）**:
The complete version segment identifying a model family release line, such as
`5.6`, `6`, or `6.1`. A dated alias belongs to its base model's generation;
unrecognized models have no inferred generation.
_Avoid_: 模型等级, 推理强度, 发布日期

**模型家族图标（Model Family Icon）**:
The stable visual identity assigned to a model family such as Astra, Sol, Luna,
or Terra. Recognized generations of the same family share its icon and family
color; generation text, rather than icon shape or color, distinguishes versions.
_Avoid_: 代次图标, 推理状态, 上游品牌

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

## Performance Observation

**性能时间桶（Performance Bucket）**:
A bounded aggregate of one registered metric and its permitted low-cardinality dimensions over a UTC interval. Its resolution and coverage are part of its meaning.
_Avoid_: 调用明细, 原始事件, 完整请求链路

**观测覆盖率（Observation Coverage）**:
The proportion of expected samples or observation time actually represented by a performance bucket. Missing collection is unknown, not a measured zero.
_Avoid_: 成功率, 采样值为零, 无流量

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
selection. It may admit bounded archive-manifest metadata to establish exact
coverage or a local gap, but leaves paged raw archive recovery to the All-Time
Coverage Checkpoint. A missing account-manifest proof is an account-scoped gap;
it cannot invalidate an independently proven global compact response.
_Avoid_: partial all-time response, global cold-start failure

**Cold Bootstrap Retry**:
A Summary maintenance attempt made while the hub has no published Projection.
It retains Bootstrap's initial-publication semantics until one exact immutable
Projection is atomically available; Rolling is a refresh of an already published
Projection, not a cold-start fallback.
_Avoid_: shortened cold retry, speculative warm refresh, global cold-start loop

**Historical Live Coverage Proof**:
The generation-fenced, immutable Projection proof for the bounded historical
live interval between the exact recent tail and the archive horizon. A Rolling
rebuild reuses only its unchanged overlap; a late historical record becomes a
range-local unavailable boundary until a background coverage pass re-establishes
the complete proof. This proof never authorizes request-time SQLite or file I/O.
_Avoid_: full historical scan on every Rolling refresh, stale global snapshot,
approximate historical aggregate

**Projection Generation Fence**:
The stable live ID, global/account rollup cursors, archive manifest
high-watermark, and settled terminal sequence observed for one background
Projection pass. A later generation never extends an older coverage claim; it
starts a new bounded reconciliation from its own fence.
_Avoid_: mixed-source snapshot, implicit catch-up

**Summary Coverage Fence**:
The immutable, scope-local coverage version for an all-time checkpoint. It
binds archive-manifest, archive-replay, and verified Snapshot V2 proof inputs
that establish historical exactness. Archive rollup materialization advances
the fence through its replay proof; ordinary hot/live rollup writes advance the
independent Summary Live Tail Cursor instead. A committed terminal tail does
not invalidate this fence; only a changed historical coverage input starts a
new reconciliation for the affected global or account scope.
The Projection records the coverage fence that actually produced each global
and account all-time aggregate; a newer rolling generation fence cannot make a
retained all-time aggregate appear published.
_Avoid_: live-tail watermark, full-history reset, mixed-generation proof

**Summary Coverage Scope Version**:
The coverage relation for one independently recoverable global or account
selection. The durable fence records completed-manifest and coverage-revision
inputs, while each Projection's temporal, account, and current-rank proofs
localize what a changed input can make unavailable. A live-tail change is not a
coverage-version change and cannot restart historical recovery.
_Avoid_: tail-driven reset, implicit exactness, global serving outage

**Summary Live Tail Cursor**:
The bounded live and rollup cursor set after a Summary Coverage Fence. It tracks
the committed terminal tail that `RollingDelta` can reconstruct without
re-reading historical archive pages.
_Avoid_: archive coverage fence, request-time cursor, raw-source replay

**Summary Source Identity**:
The durable identity of one live terminal contribution: `(row_id, invoke_id, occurred_at)`. ACKs and restart replays with the same identity are idempotent
after an immutable Projection swap. A source change without a durable row ID
cannot retire a gap proof and remains broad fail-closed until bounded
reconciliation proves its scope.
_Avoid_: invoke/time-only identity, sequence-only replay, guessed absorption

**Summary Live-Tail Reconciliation Checkpoint**:
The versioned, durable progress record for one bounded live-tail repair. It
binds the base Projection revision, recovery epoch, fixed terminal watermark,
source descriptor cursor, and state. A worker reconstructs at most the existing
`400`-ID/`64 MiB` page under the database-pressure permit, publishes with a
generation-fenced CAS, and retains post-target committed entries as an overlay.
It never falls back to generic `RollingDelta`, Bootstrap, full live admission,
or archive hydration.
_Avoid_: moving target rebuild, cursor-only freshness, request-time repair

**Live-Tail Readiness**:
The no-payload health classification for the current tail: `exact`,
`reconciling`, `live_tail_gap`, or `pressure_deferred`, together with gap
count, target watermark, epoch, stage, and elapsed time. It is diagnostic only;
HTTP and SSE continue to read the immutable Projection and availability overlay.
_Avoid_: health endpoint as a readiness bypass, payload telemetry

**Historical Summary Coverage Recovery Supervisor**:
The single off-request owner for AllTime checkpoint pages and Legacy Summary
Snapshot V2 backfill. It prioritizes pages intersecting the current 30-day
horizon from a durable due queue independent of HTTP/SSE client interest, then
uses a separate backlog sweep cursor. It acquires one low-priority recovery
permit per bounded page, commits each verified cursor/proof before yielding,
and resumes after pressure, restart, or a generation fence change. AllTime
finalization and Snapshot V2 backfill are independent workers; neither can
short-circuit the other. It never calls the generic Projection builder for
unfinished history; recent exact selections remain published while an
unproven historical scope is selection-local unavailable. A newly verified V2
proof is published through a generation-fenced immutable Coverage Publication
Overlay without a request owner, while an unchanged ready checkpoint is not
finalized again.
_Avoid_: startup full rebuild, request-time archive recovery, partial history

**Summary Coverage Due Queue**:
The durable eligibility view for historical recovery work whose
`next_probe_at` is due. It ranks coverage pages intersecting the supported
30-day horizon ahead of the general backlog and is evaluated independently of
the monotonic archive-ID sweep cursor. A deferred item stays eligible only
after its recorded backoff; a quarantined manifest is excluded until its
identity changes.
_Avoid_: ID cursor as priority, fixed ticker retry, request-triggered repair

**Summary Archive Proof Progress**:
The committed standard-hash and `(UTC occurred_at, id)` page state for one
archive manifest. Every bounded attempt starts from this state and atomically
advances it with the verified V2 page; a deadline, pressure, or lock leaves the
last verified state unchanged. The state is proof progress, not authority, until
the final digest and semantic coverage are complete.
_Avoid_: restart-from-zero hash, wall-clock-only progress, full proof rescans

**Resumable Summary Hash Proof**:
The exact-progress certificate for an archive's standard SHA-256 computation:
algorithm/state format, consumed byte offset, and the source identity to which
that state belongs. It may continue only for the same source identity; the final
digest must equal the manifest identity before any V2 authority or cleanup gate
can rely on it.
_Avoid_: chunk digest as manifest SHA, unbound hash state, partial hash authority

**Summary Archive Source Identity**:
The stable identity that binds a raw archive source to its manifest while
recovery spans attempts. It uses the available filesystem identity and size/time
metadata; a changed identity starts a new manifest evaluation, while an
unverifiable identity remains fail-closed.
_Avoid_: path-only identity, stale SHA marker, mixed source generations

**Summary Exact-Readiness Relation**:
The selection-specific proof relation for a published Projection: current,
1d, 7d, and today can be exact independently; 30d requires proof for every
intersecting coverage boundary; all requires complete historical proof and a
continuous live tail. A missing proof affects only selections whose required
source intersects it.
_Avoid_: global readiness flag, stale last-good renewal, partial aggregate

**Required Historical Coverage**:
The public historical selections that must be Exact-Ready at release
acceptance. For 30d, every intersecting Coverage Obligation requires a matching
V2 final proof. For all, the whole historical domain requires complete final
proof and the live tail must be continuous through the release watermark. A
terminal gap without an exact authority is a release blocker, never an accepted
permanent 503 or a reason to publish an approximate response.
_Avoid_: forward-only exception, tolerated historical outage, fabricated 200

**Authoritative Coverage Recovery**:
The non-request recovery path that resolves a Coverage Obligation only by
verifying an existing raw archive against its manifest and committing the V2
final proof atomically. If neither a valid V2 proof nor a readable authoritative
raw source exists, recovery records a bounded terminal gap and release
acceptance fails; it never reconstructs values from incomplete rollups or
requires a manual repair path.
_Avoid_: synthesized history, outcome-as-proof, operational-only correctness

**Production-Copy Acceptance**:
The isolated validation of a full, authorized production data copy on the shared
testbox. It proves the current archive identities, V2 proof state, cursor
recovery, and supported Summary selections before a stable release proceeds.
The copy and its raw output remain outside Git; repository evidence contains
only test code and non-sensitive pass/fail conclusions.
_Avoid_: synthetic-only release proof, committed production data, request-time repair

**Coverage Publication Overlay**:
The generation-fenced immutable Projection update produced from an already
verified V2 final proof. It replaces only the affected coverage and availability
state after proving the published base and live-tail cursor still match; it does
not start a generic RollingDelta build or perform request-time I/O.
_Avoid_: deadline-bound generic rebuild, stale fence swap, handler fallback

**Legacy Summary Coverage Recovery**:
The low-priority supervisor that seek-pages completed legacy invocation archives
into verified Summary Archive Snapshot V2 pages. Its cursor and per-manifest
outcome are durable: a verified page is exact authority, a missing or mismatched
source becomes a finite unavailable proof, and a deadline or pressure defer does
not advance an uncommitted cursor. A committed page also records its next page
and source row key, so a later attempt resumes after the last verified page
instead of reopening the archive from the beginning.
_Avoid_: full historical rebuild, V1 cleanup proof, skipped archive

Snapshot V2 backfill uses a versioned `(UTC occurred_at, id)` seek cursor. The
cursor is advanced only with the page whose semantic proof was committed. A
legacy ID-only cursor without a complete V2 proof is discarded for that
manifest and replayed chronologically; a verification failure is quarantined
by manifest SHA while pressure, lock, deadline and source-read failures remain
bounded retryable outcomes.
_Avoid_: ID-only page proof, repeated corrupt reads, split page/cursor commit
**Projection Freshness Renewal**:
The in-memory extension of an already Exact-Ready Projection only after its
current Projection Generation Fence still matches durable state. It does not
publish new source data, weaken an unavailable boundary, or make an unready
all-time selection ready.
_Avoid_: stale-source renewal, hidden full refresh, broader stale budget

**Summary Delta Journal**:
The bounded, ordered in-memory sequence of compact terminal deltas. A pending
registration establishes write-side continuity before asynchronous enqueue but
is not readable by Summary. Only the matching SQLite commit acknowledgement
promotes it to the exact difference between a published rolling Projection and
the current recent terminal tail. Rejected enqueue removes its pending entry;
the journal is not an alternative durable source or request-time fallback.
_Avoid_: speculative read, dashboard-only truth, durable archive log

**Summary Source Change Journal**:
The durable, ordered record of every committed source change that can alter a
Summary Projection Generation Fence. It contains compact batch descriptors,
not raw source or duplicated Summary rows, and gives recovery a continuous,
restart-safe account of which selections need replacement without making the
journal a request-time source or Summary response payload.
_Avoid_: best-effort invalidation, terminal-only history, raw-payload mirror,
request-time replay

**Summary Change Descriptor**:
The compact, transaction-local payload appended to the Source Change Journal.
It identifies the committed source version and its exact account, UTC-range,
current-rank, and bounded reconstruction keys. The descriptor or its
compaction proof commits in the same source transaction; the source change
cannot commit without one. The in-process terminal delta may carry richer
data; a restart reconstructs only these keys in bounded pages.
_Avoid_: duplicate source row, per-request lookup, whole-live admission

**Summary Source Change Cursor**:
The single global sequence position through the Summary Source Change Journal
which a Projection or checkpoint has incorporated. A cursor advances only with
the exact replacement it proves, so a restart or a gap cannot silently skip a
committed source change.
_Avoid_: per-source ordering, timestamp watermark, independently advanced cursors

**Summary Source Change Compaction Proof**:
The durable, bounded replacement for a contiguous compacted interval of Source
Change Journal entries. It records the union of affected account, UTC-range,
and current-rank boundaries, so any unabsorbed part remains fail-closed until a
bounded reconciliation advances past it.
_Avoid_: time-to-live deletion, silent cursor jump, unbounded event history

**Unrepresented Durable Change**:
A committed Summary source change for which a published Projection has no
continuous Summary Source Change Journal proof. It creates a fail-closed
recovery boundary until bounded reconciliation establishes an exact
replacement; it never authorizes a stale successful response.
_Avoid_: harmless cache miss, inferred delta, global stale success
**Summary Delta Cursor**:
The monotonic terminal sequence attached to one Summary Delta Journal entry.
It establishes that a RollingDelta observes a continuous acknowledged tail.
_Avoid_: timestamp ordering, best-effort event ID, unordered overlay

**Delta Gap Proof**:
A bounded account/time/current-rank proof created when a known Summary Delta
Journal entry exceeds capacity or cannot be reduced. Only a selection that can
include that proof is unavailable. A missing cursor with no durable metadata is
an explicitly broad proof until reconciliation, because its range cannot be
proven; it must never be localized by guessing from a later entry.
When the proof budget is exhausted, that broad proof is retained instead of
evicting an older account/time/rank boundary.
_Avoid_: inferred range, partial response, stale success

**Summary Archive Snapshot**:
The immutable, identity-verified compact representation of one completed
invocation archive containing every field needed to reconstruct exact Summary
selections without reopening that archive's raw source. It is committed before
the archive becomes eligible for source cleanup and is not read on the HTTP or
SSE path.
_Avoid_: incomplete hourly rollup, raw-text mirror, request-time archive fallback

**Summary Archive Snapshot V1 / V2**:
V1 is a legacy payload marker usable only to identify backfill work and never a
cleanup authority. V2 pages are durable progress, not authority by themselves.
The identity-bound V2 final-proof marker is the only coverage and cleanup
authority: page order, manifest identity, coverage, row count, payload SHA and
semantic decoding must be verified before that marker is committed. Any page or
manifest mutation revokes the marker.
_Avoid_: V1 cleanup proof, raw-text snapshot, marker-only verification

**Summary Coverage Obligation**:
The durable promise that one `(archive_batch_id, manifest_sha256)` contributes
coverage which is not yet represented by a verified V2 final proof. An
obligation is independent from a backfill attempt outcome, carries its exact
time/account/current-rank impact, and remains a Supervisor candidate until it
is resolved by final proof or marked as a terminal range-local gap. A changed
manifest SHA creates a new obligation identity.
_Avoid_: outcome-as-authority, invisible candidate suppression, global outage

**Legacy Summary Snapshot Backfill**:
The low-priority, seek-paged durable recovery that creates Summary Archive
Snapshots for readable legacy archives during normal maintenance. It resumes
from its committed cursor and never fabricates a snapshot for a missing or
unreadable authority; that finite range remains unavailable until an exact
source exists. Every page commits only progress; the final proof transaction
resolves the corresponding Summary Coverage Obligation and is the point at
which coverage becomes exact-ready.
_Avoid_: release-blocking full scan, invented historical total, manual read path

**Typed Summary Backfill Outcome**:
The durable state for one `(archive_batch_id, manifest_sha256)` recovery item:
`Verified`, `InProgress`, `Deferred`, `TransientFailure`, or `Unrecoverable`.
`Deferred` and `TransientFailure` retain the last committed proof cursor and a
bounded `next_probe_at`; `Unrecoverable` is quarantined until the manifest
identity changes. `Complete` means only that an attempt returned; it does not
resolve a Summary Coverage Obligation without a matching final proof marker.
No outcome is allowed to advance a cursor without the page proof it describes.
_Avoid_: string-only retry reason, unbounded retry, cursor-on-failure

**Summary Recovery Permit**:
The single low-priority database-pressure admission held while one recovery
page performs its due check, bounded source read, and atomic durable commit.
Pressure refusal happens before SQLite/archive I/O and records only in-memory
eligibility; actual `BUSY`/`LOCKED` is reported separately after an attempted
operation.
_Avoid_: pressure bypass, nested permits, millisecond retry loop

**Summary Archive Snapshot Coverage Gate**:
The archive lifecycle condition that prevents source cleanup until the matching
Summary Archive Snapshot, coverage, and identity proof are durably committed.
Snapshot pressure or failure retains the authoritative archive and advances a
recoverable backfill checkpoint; it never creates a new Summary coverage gap.
_Avoid_: cleanup-before-proof, dropped Snapshot, capacity-driven undercount

**RollingDelta**:
The bounded recovery mode that composes an immutable rolling base with a
continuous in-memory Summary Delta Journal or durable Summary Source Change
Journal descriptor tail. It does not run complete live-source admission,
archive hydration, or request-time I/O, and it never makes `all` ready.
_Avoid_: full rolling rebuild, all-time finalization, stale-cache success

**All-Time Coverage Checkpoint**:
The durable generation-fenced state for bounded all-time reconciliation. It
contains independent global/account manifest-proof cursors, rollup seek cursors
and committed aggregate accumulators. A restart resumes from the last complete
microbatch; a changed fence discards only that stale generation before the next
bounded page begins.
_Avoid_: full-history restart, mixed-generation aggregate, in-memory-only progress

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
The cold-start boundary that prohibits raw Legacy Detail Mirror identity reads
before the first Summary Projection build. Bootstrap retains unknown manifests
as range-local fail-closed evidence and publishes every independently exact
selection; the pressure-gated generic backfill starts its durable identity
cursor only after publication. An unresolved source remains unknown; it cannot
block an independent proof or be guessed into a mirror role.
_Avoid_: generic-backfill starvation, blocking raw preflight, inferred mirror

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

**Runtime SQLite Write Admission**:
The single process-local turn that classifies and orders a short main-database write as P1
terminal, interactive proxy, derived, or maintenance work. It is independent from a SQLx pool
connection and is not held while performing network or file I/O.
_Avoid_: connection-pool slot, long-lived task lock, generic background permit

**P1 Terminal Write**:
The durable completion or final failure record for an accepted proxy invocation. It dominates
every other Runtime SQLite Write Admission class and must not be discarded for derived work.
_Avoid_: best-effort batch, maintenance write

**Interactive Proxy Write**:
A short durable state change needed while routing or processing a proxy request. It follows P1
terminal work but precedes derived and maintenance work.
_Avoid_: background write, terminal batch

**Derived Write**:
A rebuildable projection or index change whose inputs are already durable. It may defer whenever
P1 terminal or Interactive Proxy Write work is waiting.
_Avoid_: terminal persistence, request-time read fallback

**Maintenance Write**:
A bounded retention, repair, or housekeeping update. It runs after the higher-priority Runtime
SQLite Write Admission classes, with explicit fairness only when no P1 work is waiting.
_Avoid_: interactive write, unlimited maintenance job

**Database Pressure Incident**:
A classified `SQLITE_BUSY`, `SQLITE_LOCKED`, or pool-acquisition timeout that starts the
background cooldown. It carries operational timing and class evidence only, never SQL text,
bound values, proxy payloads, or account identifiers.
_Avoid_: generic database error, request trace

## Validation and delivery contracts

**Project Test Entrypoint**:
The repository-owned command contract for running its complete, resource-profiled
backend validation. It names the stable test operation and its required result,
without relying on a caller's shell or incidental machine setup.
_Avoid_: ad hoc cargo command, CI-only test recipe

**Runner Isolation**:
The boundary that gives one validation run private resources, paths, capacity
accounting, and cleanup. It does not redefine the project's test command or
silently change its concurrency and coverage.
_Avoid_: project toolchain, test semantics, host takeover

**Toolchain Contract**:
The pinned compiler, test runner, system libraries, and executable identity that
make a Project Test Entrypoint reproducible across approved environments.
_Avoid_: ambient PATH, latest tool, login-shell setup

**Writable Test Workspace**:
The private run area where validation may create build outputs, schema templates,
temporary archives, and copied source inputs while the original checkout remains
unchanged.
_Avoid_: repository mutation, shared target directory, production data path

**Representative-Scale Acceptance**:
Validation using deterministic data with production-shaped cardinality and source
boundaries to prove timing, memory, and exact response behavior. It is stronger
than a small unit fixture but does not require raw production data in a PR.
_Avoid_: smoke-only status check, production-data upload

**CI-contained Validation**:
A validation decision derived from the target commit and evidence generated by
the same trusted CI workflow. It never depends on a host path, testbox result,
production-copy, or manually supplied validation result.
_Avoid_: external receipt, local test dependency, production-copy gate

**External Validation Receipt**:
A validation result created outside the workflow that would consume it. It is
not an input to PR, Main, or Release gates.
_Avoid_: CI evidence, release prerequisite, testbox handoff

**Projection Readiness**:
The per-selection state describing whether all required source and boundary proof
is present in the published immutable Summary Projection. Process health alone
does not imply Projection Readiness.
_Avoid_: service started, global readiness flag, approximate response

**Range-Local Unavailable**:
An exact fail-closed result for only a Summary selection whose required source
coverage is unproven; disjoint selections remain independently serviceable.
_Avoid_: partial response, global hydration failure, request-time repair

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

**用量明细自适应值**:
Dashboard「用量明细」浮层中的 Token、成本和比例等数值单元格，按单元格实际可用宽度从完整本地化文本切换到共享自适应格式的紧凑候选；它保留原始数值语义，并在切换后通过项目自定义 Tooltip 提供完整值。
_Avoid_: 字符串截断, 固定表格列宽, 浏览器原生 title 提示

## 账号路由控制面

**账号有效路由规则（Account Effective Routing Rule）**:
一个上游账号在账号覆盖、标签、分组和池级默认策略解析后的完整路由策略。它是 Dashboard 快捷策略 chip 与账号选择共同展示的权威值，而不是某次写入提交的局部 `routingRule` 覆盖。
_Avoid_: Dashboard chip 状态, 原始 routingRule, 优先级迁移

**账号有效路由规则变更（Account Effective Routing Rule Change）**:
已提交且改变一个或多个上游账号有效路由规则的控制面变化。它不等同于调用统计的高频更新，也不等同于对话级的优先级迁移。
_Avoid_: 单个 chip 点击, 统计刷新, 优先级迁移

**路由状态版本（Routing State Version）**:
一份已发布账号有效路由规则快照的有序服务端标识。客户端用它拒绝较早的当前态，同时仍接受较晚提交的规则变化。
_Avoid_: 客户端时间戳, 数据库行版本, 永久全局序号

## Dashboard 对话状态

**会话进行中相位摘要（Conversation In-Flight Phase Summary）**:
同一 Prompt Cache 对话内处于 `queued`、`requesting`、`responding` 的调用数量。它只描述当前未终结调用；只要任一数量非零，卡片头部以该摘要表达实时状态而不表达成功、失败或其他终态。
_Avoid_: 最新记录状态, 最近调用列表统计, 前端状态计数, 对话终态摘要

## 代理模型清单

**模型管理列表（Managed Model List）**:
价格目录条目与代理预置模型候选项按模型 ID 合并形成的管理视图。一个模型可以只有预置状态而没有价格，价格与预置状态独立维护。删除列表项会同时移除该模型的价格并取消预置，但不改写历史调用成本；若 models.dev 仍提供该模型，下次同步可重新作为候选出现。
_Avoid_: 上游实时模型列表, 价格目录, 代理预置模型名单

**代理预置模型名单（Proxy Preset Model List）**:
由代理配置选中、可加入被劫持的 `GET /v1/models` 响应的模型 ID 集合；除应用内置模型外，也可包含用户加入的模型。价格同步不会自动改变名单成员状态；开启上游模型合并时，最终响应还可能包含上游返回的其他模型。
_Avoid_: 价格目录, 上游可用性, 模型能力清单

## 代理成本估算

**成本估算价格目录（Cost Estimation Pricing Catalog）**:
以模型 ID 为键、供代理调用成本估算使用的单位价格集合；每个模型 ID 只有一份生效价格，供应商只标识价格候选来源，不参与运行时匹配。它是有限的本地估算数据，不代表上游账单真值或完整的供应商模型目录。
_Avoid_: 上游账单, 完整模型目录, 代理预置模型名单

**价格同步候选（Price Sync Candidate）**:
models.dev 提供、等待用户审阅后才可能写入价格目录的供应商报价。一个模型 ID 可以有多个供应商报价，但本地价格目录只保留该模型的一份生效价格。
_Avoid_: 生效价格, 自动覆盖, 按供应商分别计价

**供应商筛选（Provider Filter）**:
价格同步审阅中选定的报价来源范围；它与是否将某项报价写入本地价格目录的同步勾选是不同概念。
_Avoid_: 上游账号选择, 运行时供应商路由, 同步勾选

**同步勾选（Sync Selection）**:
用户希望在确认同步时应用到本地价格目录的候选报价选择；勾选本身不是一次价格写入。
_Avoid_: 供应商筛选, 代理预置状态, 已同步价格

**系统同步记忆（System Sync Memory）**:
同一服务实例共同维护的价格同步选择与模型发现记录；它包括模型勾选、供应商筛选及曾出现的模型，独立于任何单个浏览器或设备。
_Avoid_: 浏览器本地偏好, 临时弹窗状态, 生效价格目录

**报价选择记忆（Quote Selection Memory）**:
针对同一模型 ID 与同一供应商 ID 的同步勾选意愿；另一供应商的报价是独立选择，即使模型 ID 相同也不继承该意愿。
_Avoid_: 模型全供应商勾选, 生效价格, 自动改选供应商

**本次可同步变更（Applicable Sync Changes）**:
本次审阅范围内，已勾选且来源明确、可导入，并且会新增或改变本地价格的候选报价；它不等同于所有已记忆的勾选。
_Avoid_: 所有历史选择, 当前可见模型数, 待查看新模型数

**本地未定价模型（Locally Unpriced Model）**:
本地成本估算价格目录中没有价格条目的模型；它可能已经存在于模型管理列表，也可能早已在外部目录中出现。
_Avoid_: 首次发现模型, 新发布模型, 未纳管模型

**目录首次发现模型（First-Discovered Catalog Model）**:
初始目录基线建立后，价格同步目录中出现、此前没有出现记录的模型 ID；它不表示该模型刚在市场发布，也不表示本地没有价格。
_Avoid_: 新发布模型, 本地未定价模型, 新供应商报价

**待查看新模型（Unviewed New Model）**:
目录首次发现模型中尚未得到查看确认的模型；供应商筛选、搜索隐藏或位于当前可视范围之外都不构成查看确认。
_Avoid_: 未同步模型, 未勾选模型, 所有未定价模型

**已废弃报价（Deprecated Quote）**:
外部目录明确标记为已废弃的供应商模型报价；该状态描述这条来源报价，不证明同模型在其他供应商下已下架，也不证明没有标记的模型仍可调用。
_Avoid_: 全供应商下架模型, 按发布日期判断过时, 上游实时不可用

**价格差异（Price Difference）**:
同一模型的候选报价与已有本地估算价格之间，在系统支持的 token 计价维度上的值差异。
_Avoid_: 上游账单差异, 供应商间价差, 模型首次发现

**实际模型标识**:
调用记录中由请求或上游响应报告的原始模型 ID。它表达实际调用的模型身份，不因成本估算而改写。
_Avoid_: 计价模型, 规范化模型, 价格别名

**缓存写入 Tokens**:
由上游明确报告、实际写入 Prompt Cache 的输入 Tokens 数；它与普通输入及命中缓存后读取的 Tokens 分开计量。显式报告的 `0` 也是精确用量。
_Avoid_: 缓存读取 Tokens, 普通输入 Tokens, 缺失字段推算值

**缺失缓存写入估值**:
上游未报告缓存写入用量时，按 `max(inputTokens - cacheInputTokens, 0)` 得出的兼容性估值；它描述估算器的分配，不代表上游报告的真实写入数量。
_Avoid_: 精确缓存写入 Tokens, 上游用量事实, 实际缓存写入数

**同值预设价格**:
仓库管理的实际模型价格条目，其默认单位价格与明确指定的既有模型相同。它仍是该实际模型自己的预设价格，不把模型身份改写为价格来源模型。
_Avoid_: 运行时价格回退, 通配符定价, 模糊模型匹配

**缺失成本补算**:
仅为已有可计费 Token、但未持久化成本的在线代理调用补写估算成本的可恢复维护过程。已有成本记录不属于该过程的修改对象。
_Avoid_: 全量重定价, 历史成本覆盖, 实时计费

**在线调用历史**:
当前主 SQLite 数据库中仍可直接更新并重新投影统计的调用记录。
_Avoid_: 归档历史, 全量历史, 不可变记录

**归档调用历史**:
已导出到受完整性约束的归档批次中的调用记录。它不属于在线缺失成本补算的范围。
_Avoid_: 在线调用历史, 可直接回填记录, 可变历史

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

**候选镜像**:
由一个主线提交按目标架构构建、完成本地 smoke 后写入 GHCR 的不可变 SHA 镜像；它不是版本 tag、`latest`、manifest、Git tag、GitHub Release 或部署来源。
_Avoid_: 临时发布, 预发布版本, 正式镜像

**正式提升**:
在同一提交的 CI Main 全部成功后，将其已 smoke 验证的候选镜像创建为正式多架构 manifest，并依次完成 tag 与 GitHub Release 的发布操作。
_Avoid_: 候选构建, 镜像重建, 提前发布

**Stateful SQLite 测试分片**:
完整 Stateful SQLite profile 的一个确定性、互不重叠的 nextest 子集；所有分片的并集才构成完整 profile，任一分片失败都会使同一个 required check 失败。
_Avoid_: 可选测试, 新的 required check, 不完整 profile

**PR 发布评论**:
附在源 PR 上的版本交付追溯记录；它独立于 GitHub Release 页面。
_Avoid_: Release 正文, 发布说明

## Routing Affinity

**优先级迁移（Priority Handoff）**:
An automatic, non-forced attempt to move one sticky conversation from its current eligible `Fallback` upstream to a higher-priority eligible upstream. The source binding remains authoritative until the target attempt succeeds.
_Avoid_: 故障切换, 强制绑定, 立即换号

**HTTP 优先级迁移范围（HTTP Handoff Scope）**:
The live client-facing transport boundary in which the handoff admission gate applies: HTTP pool requests only. Legacy WebSocket invocations remain readable historical data and are not a routing or retry path.
_Avoid_: WebSocket 同步改造, 跨传输隐式复用, 长会话迁移锁

**延期优先级迁移（Deferred Priority Handoff）**:
A priority handoff whose selected highest-ranked target is not admitted for the current request; the request continues on its eligible source upstream rather than migrating to a lower-ranked target, and is never held awaiting the transfer.
_Avoid_: 请求排队, 等待迁移, 阻塞对话

**对话模型路由（Conversation-Model Route）**:
The exact pair of a sticky conversation and its requested model. It is the unit of an automatic priority handoff; another model in the same conversation is independent.
_Avoid_: 整个对话换号, 账号级迁移

**权威 Sticky 来源（Authoritative Sticky Source）**:
The persisted upstream currently assigned to a Conversation-Model Route. A cache miss, qualification failure, or uncertain promotion metadata does not turn it into a new assignment; while eligible, only an admitted Priority Handoff or an established Fault Failover may replace it.
_Avoid_: 临时候选, 缓存命中, 新分配来源

**Sticky 所有权栅栏（Sticky Ownership Fence）**:
The generation check that prevents a completed older request from replacing a newer Authoritative Sticky Source. A target's valid success evidence may still affect its account-model health and current recovery generation even when this fence rejects the binding mutation.
_Avoid_: 旧请求覆盖新绑定, 绑定失败即否定目标成功, 完成顺序决定所有权

**未准入来源置换（Unadmitted Source Displacement）**:
An invalid routing outcome in which a request with an Authoritative Sticky Source is treated as a fresh assignment and moves to another upstream without handoff admission or an established source failure.
_Avoid_: 故障切换, 真正新分配, 正常优先级迁移

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
The serialized portion of a priority attraction epoch after a temporary model-route degradation, an expired model-route cooldown, or an operator health reset. It requires three consecutive complete terminal successes from gate-admitted automatic priority handoffs or fresh assignments before ordinary priority admission resumes; the first complete recovery success counts as the first of those three. A route already rebound by a successful handoff continues directly on its new sticky target because the gate controls new target admission rather than all target traffic.
_Avoid_: 一次成功即全面开放, 固定等待时长, 无限制恢复流量

**恢复吸引候选（Recovery Attraction Candidate）**:
A strictly higher-priority API Key account-model target whose ordinary hard eligibility still holds but whose existing temporary model-route failure evidence currently demotes it. It may seek Request-Driven Recovery Admission without becoming an unrestricted routing winner; cache protection, unsupported capability, hard account failure, and caller cancellation do not create this status.
_Avoid_: 普通健康候选, 后台探针, 硬错误重试, 故障切换目标

**恢复首选目标（Preferred Recovery Target）**:
The single highest-ranked Recovery Attraction Candidate for one routing decision after effective priority and existing same-tier ordering are applied. Only this target may seek recovery admission; a busy or cooling permit does not make a lower-ranked recovery candidate the preferred target for that request.
_Avoid_: 恢复候选列表轮询, 许可忙后改试次优目标, 并行多目标探测

**请求驱动恢复准入（Request-Driven Recovery Admission）**:
The admission of one eligible real request to a Recovery Attraction Candidate through the existing target account-model Handoff Permit. A degraded target may seek it immediately and a cooling target only after cooldown expiry; a busy or cooling gate never queues the request, and no background probe is created.
_Avoid_: 后台健康检查, 定时探针, 恢复请求队列, 全量目标并发锁

**恢复代际栅栏（Recovery Generation Fence）**:
The local recovery-verification boundary created by each newly accepted temporary model-route failure. Request-start time and existing reset fences decide whether terminal evidence belongs after the latest failure; an older in-flight permit remains exclusive until completion but its result cannot recover health or contribute success evidence to the newer generation.
_Avoid_: 发生时间无关的计数, 旧请求覆盖新故障, 重置即并发放行

**机会式优先级迁移（Opportunistic Priority Handoff）**:
A handoff whose next candidate is the next eligible real request, not a durable FIFO list of conversations.
_Avoid_: 严格迁移队列, 后台迁移任务

**迁移确认（Handoff Confirmation）**:
A complete terminal success from the target request. It releases the handoff permit and provides target account-model success evidence; it commits the target as the new sticky upstream only when the Sticky Ownership Fence still permits that mutation. Partial output or elapsed time confirms neither result.
_Avoid_: 首字节成功, 请求已发出, 目标已选中, 时间阈值

**单次迁移尝试（Single-Attempt Handoff）**:
The one target-upstream request made under a handoff permit. It never enters automatic retry.
_Avoid_: 同账号重试, 429 重试, 自动故障切换

**迁移许可（Handoff Permit）**:
The process-local exclusive permission held by one single-attempt handoff or Request-Driven Recovery Admission while a new target admission is being evaluated. It does not limit traffic from Conversation-Model Routes already bound to the target; optional database coordination is secondary and client cancellation releases the permit without changing the source binding or recording a target failure.
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
A handoff permit whose lifetime is confined to the current process. Process restart discards permits and success counts rather than recovering them from persistent storage, starts local verification at `0/3`, and uses persisted model health only to decide whether the next target opportunity is degraded, cooling, or ordinarily eligible.
_Avoid_: 持久锁, 启动恢复锁, 重启即全面开放, 数据库依赖许可

**受控人工重开（Controlled Manual Re-entry）**:
An operator health reset that clears model-route cooldown but starts recovery verification rather than immediately restoring unrestricted priority admission.
_Avoid_: 重置即全量开放, 必须等待自动故障切换

**新分配绕行（Fresh Assignment Bypass）**:
Selecting another healthy eligible upstream for a new conversation while a preferred target's handoff gate is occupied. If no alternative exists, the request terminates without waiting for the permit.
_Avoid_: 等待迁移, 绕过闸门, 并发恢复

**故障切换（Fault Failover）**:
The existing recovery path for a request whose Authoritative Sticky Source has actually failed or become ordinarily ineligible. It may replace that source without waiting for the priority-handoff gate, even while a recovery permit exists for the same target, but continues to observe ordinary model-health eligibility; a higher-priority target, a cache miss, or uncertain routing metadata is not Fault Failover.
_Avoid_: 优先级迁移, 等待迁移许可, 原上游可用时的迁移, 未准入来源置换

**无阻断迁移审计（Non-Blocking Handoff Audit）**:
Best-effort diagnostic records on the existing routing-audit path. Its trigger records why admission was considered, independently from the decision made by the gate; safe reason codes and recovery progress cover admission, deferral, verification, and cooldown, while a persistence failure never changes routing, permit acquisition, or release.
_Avoid_: 审计数据库锁, 诊断失败即拒绝请求, 原始上游错误泄露

**全局本地镜像迁移开关（Globally Mirrored Handoff Switch）**:
One operator-controlled global setting exposed through the existing settings surface and enabled by default. Disabling restores pre-gate routing without cancelling an in-flight request; that request may still publish current health evidence and a generation-safe sticky mutation, but cannot contribute success evidence after the switch generation changes. Re-enabling starts local verification at `0/3`.
_Avoid_: 热路径查数据库, 数据库不可用即停流, 按账号模型开关, WebSocket 开关

## Backend Test Isolation

**Source Snapshot**:
The immutable, run-specific backend-test source tree supplied to the test process. Its host and container path are execution-environment details, not part of the project test contract.
_Avoid_: writable checkout, shared source tree, cache workspace, runner mount path

**Run-local Fixture Workspace**:
The disposable writable workspace supplied through `BACKEND_TEST_WORKSPACE` for one backend-test run's SQLite schema templates, archive fixtures, and other mutable test data.
_Avoid_: Cargo target cache, shared database, durable fixture store, fixed host path

**Persistent Cargo Home**:
An optional durable `CARGO_HOME` provided by a test execution environment for Cargo registry and Git dependencies. It is separate from every test run's source and fixture data; the project does not prescribe its storage or mount mechanism.
_Avoid_: source cache, test workspace, target directory, runner cache path

**Persistent Cargo Target Cache**:
An optional durable `CARGO_TARGET_DIR` provided by a test execution environment for Cargo compilation artifacts. Cargo fingerprints and locks remain the sole validity and concurrency authority; the project does not prescribe its storage or mount mechanism.
_Avoid_: fixture workspace, shared test database, custom content-addressed cache, runner cache path

**Production-copy Runtime Workspace**:
The disposable directory created by the production-copy validator for the service process, response captures, logs, and default Cargo directories. It is separate from the staged production copy and from any externally provided Cargo cache.
_Avoid_: runner scratch path, staged production data, durable cache, source snapshot

## Storage Observation

**数据目录（Data Directory）**:
本项目集中存放持久化业务数据和运行数据的目录树。目录内的文件是否已经关联到调用记录，不改变其归属。
_Avoid_: 数据库文件, 已关联 raw 文件集合, 服务器文件系统

**数据目录体积（Data Directory Footprint）**:
数据目录内全部内容去重后的实测存储量，包含尚未被业务盘点识别的文件；它是项目存储总体积的一部分。
_Avoid_: 已追踪项目存储, raw payload 逻辑体量, 已完成调用归档体积

**项目存储总体积（Project Storage Footprint）**:
数据目录与项目单独配置的存储位置共同覆盖的实际存储量，同一存储对象只计一次。它包含目录外的 raw、archive 和项目其他持久化运行文件，业务关联是否完整不改变其归属。
_Avoid_: 业务指标相加, 服务器磁盘总使用量, 仅数据目录体积

**已追踪 Raw 盘点（Tracked Raw Inventory）**:
已关联 request 与 response raw 文件集合的持久化盘点；并集总量与两侧拆分表达不同的去重范围。盘点未就绪表示该业务视图暂不可用，不表示数据目录无法测量或其中没有 raw 文件。
_Avoid_: 数据目录体积, 完整 raw 目录占用, request 与 response 直接相加

## Autonomous Retention Recovery

**Verified Archive**:
An immutable invocation archive whose artifact, manifest, Summary proof, and source-row transition have committed as one durable publication. It is the sole authority that permits removal of its corresponding live detail and raw payload files.
_Avoid_: Written archive, prepared archive

**Retention Backlog**:
Expired live invocation detail or raw payloads that remain because the automated retention workflow has not yet completed their Verified Archive transition.
_Avoid_: Disk leak, stale data

**Raw Capture Circuit Breaker**:
An automatic protective state that stops persisting new raw request and response bodies when retention cannot keep the diagnostic store within its budget, while preserving proxy delivery and structured invocation facts.
_Avoid_: Proxy shutdown, data-loss mode

**Physical Raw Inventory**:
The durable, incrementally reconciled count of bytes physically owned by the raw payload store, including bounded in-flight reservations. It is the authority for raw-storage watermarks, not a logical payload-byte estimate or an on-request directory scan.
_Avoid_: Raw metric, file walk

**Prepared Archive**:
An immutable archive artifact whose source identities and digest are recorded for recovery but whose source rows and raw-owner links remain live. It may become a Verified Archive only after the final atomic publication transaction succeeds.
_Avoid_: Completed archive, orphan archive

**Retention Recovery**:
The autonomous, pressure-aware background process that resumes a Retention Backlog without an operator CLI, restart, or manual database repair.
_Avoid_: Manual cleanup, maintenance window

**归档核心（Retention Core）**:
The retention stages that establish a Verified Archive and commit the corresponding source-row transition. Their completion is distinct from the freshness or cleanup of derived prompt-cache conversation data.
_Avoid_: 会话统计刷新, 全部维护完成, 文件写出即完成

**会话派生维护（Conversation Derived Maintenance）**:
The aggregate refresh and orphan-conversation cleanup required after retained invocation facts change. It preserves exact conversation statistics and deletion safety independently of Retention Core completion.
_Avoid_: Prompt token 缓存失效, 调用归档, 在线请求阻塞步骤

**运行积压快照（Run Backlog Snapshot）**:
The observed expired invocation population associated with a retention run, with its observation time and scope. It is a denominator in invocation rows, not a sum of conversation keys, archive batches, or raw files.
_Avoid_: 永久总量, 全局完成率, 混合单位总数

**待归档调用积压（Invocation Archive Backlog）**:
Invocations still in the live store that are eligible for archival under the applicable retention policy at an observation point. Its quantity is measured in invocation rows; other retention stages retain their own units and are not added to this count.
_Avoid_: 全部维护待办, Prompt 会话积压, 文件与调用混合总数

**最长归档逾期时长（Maximum Archive Overdue Duration）**:
The longest time an invocation still awaiting archival has remained eligible for archival under its retention policy, measured at an observation point. It is the delay beyond the policy deadline, not the age of the invocation since it occurred.
_Avoid_: 最老请求年龄, 请求耗时, 未知即零

**自动归档追赶（Automatic Archive Catch-up）**:
Continuation of bounded retention rounds while eligible archival backlog remains, with one execution owner, between-round yielding, and pressure-aware retry. An inspection schedule discovers work; it does not restrict catch-up to scheduled occurrences. Disabling retention stops admission of subsequent catch-up work at a safe committed boundary.
_Avoid_: 错过计划补跑, 第二套归档调度器, 绕过压力保护

**小时积压观测（Hourly Backlog Observation）**:
The last successful exact observation in a UTC hour of Invocation Archive Backlog and Maximum Archive Overdue Duration, measured together under the recorded retention policy. Its actual observation time is retained; absent observations are gaps rather than zero backlog or reconstructed history.
_Avoid_: 小时归档成果, 小时平均积压, 最近运行结果代替历史观测

**运行完成度（Run Completion Outcome）**:
The extent of work completed within a retention run's captured scope: completed, partial, or deferred, alongside its execution status. It does not rewrite an earlier run when background maintenance later finishes.
_Avoid_: 成功即全部完成, 延迟即失败, 后台完成后改写历史

## Task Operations

**纳管任务（Managed Task）**:
A background task with an independently observable boundary and operator controls for enablement, triggering, progress, and run history. A task may be interval-based, cron-based, event-driven, or startup-only; the trigger mode is part of its identity.
_Avoid_: 任意后台线程, 单次 SQL, 页面刷新

**任务运行（Task Run）**:
One execution of a Managed Task with a trigger, start and finish timestamps, terminal status, duration, summary, and bounded error detail. A Task Run is an operational record, not a business invocation.
_Avoid_: 对外调用, 上游尝试, 日志行

**任务待处理量（Task Pending Population）**:
The complete population awaiting a task's work at a recorded observation point, with an explicit unit and eligibility scope. A bounded scan or candidate window describes only part of this population and does not establish its total.
_Avoid_: 本页候选数即总量, 扫描上限即积压, 未知即零

**本次发现量（Run Discovered Candidates）**:
The distinct eligible work items identified within one task run, including previously queued candidates selected by that run. It is a count within the run, not newly arriving work between runs or every item inspected regardless of eligibility.
_Avoid_: 跨轮新增量, 全量待处理量, 原始扫描量

**本次处理量（Run Committed Work）**:
The distinct work items whose task-specific completion boundary was successfully reached within one run. Failed attempts and retries do not increase it; confirmed committed work remains part of the run's result even when later work fails.
_Avoid_: 尝试次数, 发现即完成, 失败整轮成果归零

**任务计量范围（Task Measurement Scope）**:
The work population described by one task metric, including its unit, eligibility rules, observation boundary, and coverage. Discovery and completion form subsets of a pending population only when those boundaries and item identities are compatible.
_Avoid_: 同名字段即可相加, 行与批次混用, 截断值即完整值

**固定清理存量（Fixed Clearance Cohort）**:
A population of eligible work items captured at one observation point and tracked through their completion. Later arrivals belong outside that cohort; its completion fraction is distinct from the current replenished backlog or a single scan's processing ratio.
_Avoid_: 滚动积压即固定总量, 本次处理除以本次发现即总体进度

**任务执行／让行级别（Task Execution/Deferral Class）**:
A read-only description of the execution or resource-deferral rules that actually apply to a Managed Task. It remains undefined when no corresponding rule exists and does not grant operators a configurable cross-task execution rank.
_Avoid_: 可编辑任务优先级, 人工重要性标签, 跨任务排队顺序

**待执行请求（Queued Task Request）**:
An accepted request for a Managed Task that is waiting in its dispatcher's execution queue. Its request time and queue position describe waiting rather than actual work or a guaranteed execution time.
_Avoid_: 到期即入队, 下次计划检查, 已请求即运行中

**准入延后任务（Admission-Deferred Task）**:
A Managed Task waiting for pressure or execution-resource admission at an observed scheduling boundary. This wait establishes neither a dispatcher queue position nor an actual execution interval.
_Avoid_: 统一 FIFO 队列, 等待下次定时, 未观测的积压

**任务让行状态（Task Deferral State）**:
The observed pressure or resource-admission condition that delays Managed Task scheduling or work, together with its reason and affected tasks when known. It describes task admission independently of overall runtime health.
_Avoid_: 系统整体健康等级, 任意降级告警, 单个任务执行结果

**当前执行实例（Current Task Execution）**:
The actual ongoing work of a Managed Task, with its execution start and elapsed time independent of when a request was queued or a history record was published. A backfill parent and its active child describe one execution instance rather than two concurrent tasks.
_Avoid_: 已请求即运行中, 最新历史记录, 父子重复计数

**任务执行区间（Task Execution Interval）**:
The observed interval of actual work within a Task Run, beginning when execution starts and ending when execution stops; an ongoing execution has an open end. Request time, queue waiting, and observation publication time are separate from this interval.
_Avoid_: 入队即开始, 计划执行时段, 历史写入时段

**任务标识色（Task Identity Color）**:
A stable visual identity assigned to a Managed Task and shared by its catalog dot and execution bars across runs. It identifies the task independently of its trigger, enablement, execution outcome, or system pressure state.
_Avoid_: 每次运行随机配色, 状态色, 目录排序色

**任务进度快照（Task Progress Snapshot）**:
The latest durable operational state for a Managed Task, including phase, unit-aware total and completed counts, cursor or checkpoint, last update, and an optional estimate. Unknown or open-ended work remains explicitly unknown instead of being represented as zero or complete.
_Avoid_: 数据库行数, 实时日志, 伪造百分比

**持续任务（Continuous Task）**:
A Managed Task whose work is continuously replenished or whose total cannot be bounded for one run. It exposes health, activity, throughput, and recent runs while leaving total progress and remaining time unknown when no sound estimate exists.
_Avoid_: 永不完成任务, 无限循环

**安全边界暂停（Safe-Boundary Pause）**:
A task control state that prevents new work and lets the active batch or checkpoint commit before the run becomes paused. It does not forcefully interrupt an in-flight database operation.
_Avoid_: 强制取消, 事务中断, 立即杀死任务

**任务触发模式（Task Trigger Mode）**:
The declared way a Managed Task becomes eligible: event wake, fixed interval, cron schedule, or startup. Event-driven tasks retain their event wake path and may use interval or cron only for bounded fallback probes.
_Avoid_: 固定轮询, 触发来源混用, 把所有任务改成 cron

**任务自定义计划（Configured Task Schedule）**:
An operator-selected interval or UTC cron override for a Managed Task that supports that control. Its absence means no operator override and does not mean the task has no automatic scheduling policy.
_Avoid_: 系统默认周期, 当前生效策略, 空值即无计划

**任务生效策略（Effective Task Policy）**:
The actual eligibility and scheduling rules currently governing a Managed Task, including their source and any fixed, event-driven, startup, or adaptive behavior. A check cadence describes opportunities to consider work rather than a guarantee that work executes at every check.
_Avoid_: 目录静态标签, 自定义计划值, 保证执行时间

**主库业务事实（Business Main Facts）**:
The durable request, account, archive, and projection facts that serve product behavior. New task controls, progress snapshots, and run history must not add synchronous writes to this store.
_Avoid_: 任务审计库, 性能指标库, 页面缓存

**运维任务数据（Operational Task Data）**:
The newly introduced task configuration, progress, run history, and bounded diagnostic summaries used by the management pages. It is stored outside the Business Main Facts store so task observability does not increase main-database write pressure.
_Avoid_: 业务事实, 原始请求载荷, 性能时间桶

**维护库（Maintenance Database）**:
The separate local SQLite database for Operational Task Data, distinct from both the Business Main Facts store and the Performance Telemetry Database. Task controls, progress snapshots, run history, and bounded error summaries are written here asynchronously on a best-effort basis; unavailability never falls back to the main database.
_Avoid_: 主库旁路表, 性能时间桶, 任务执行前置依赖

The configurable path is `MAINTENANCE_DATABASE_PATH`. When omitted, the service derives a sibling file by appending `.maintenance.sqlite` to the main database stem: `codex.sqlite` becomes `codex.maintenance.sqlite`; this is a suffix, not a hidden filename beginning with `.`.

**性能指标库（Performance Telemetry Database）**:
The existing separate SQLite database for bounded, low-cardinality aggregate performance metrics. It records task overhead, throughput, database waits, pressure, latency distributions, failures, and deferrals, but does not become the source for task configuration, progress, or run history.
_Avoid_: 任务状态库, 运行明细库, 账号级指标

**运维观测降级（Operational Observability Degradation）**:
A state in which a task continues its authorized work while the Maintenance Database is unavailable or a write is dropped. The management page shows missing or stale observation coverage; the system never synchronously writes the missing record to the Business Main Facts store just to make the page complete.
_Avoid_: 阻断任务, 主库回退, 伪造成功记录

**手工维护操作（Manual Maintenance Operation）**:
A Managed Task that has no interval or cron trigger and runs only after an operator invokes it. Its enable switch controls whether invocation is allowed; it uses the same safe-boundary pause and run-history contract as scheduled tasks.
_Avoid_: 隐藏 CLI, 伪计划任务, 自动补跑

**单实例任务运行（Single-Instance Task Run）**:
The concurrency rule that permits at most one active run for a Managed Task. A second manual request is rejected while one is active, and a missed scheduled occurrence is not queued for a later burst.
_Avoid_: 并发重复运行, 任务队列, 追赶补跑

**无追赶调度（No-Catch-Up Scheduling）**:
The scheduling policy that does not replay occurrences missed while a task was disabled or already active. Re-enabling waits for the next normal eligibility; an operator can request an explicit one-off run when backfill is intended.
_Avoid_: 恢复即补跑, 补偿风暴, 隐式追赶
