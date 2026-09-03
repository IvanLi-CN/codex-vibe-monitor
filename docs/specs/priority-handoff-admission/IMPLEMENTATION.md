# API Key 优先级迁移准入控制 实现状态

> 当前有效规范仍以 `./SPEC.md` 为准；这里记录实现覆盖、交付进度与 rollout 相关事实，避免这些细节散落到 PR / Git 历史里。

## Current Status

- Implementation: the original admission gate, request-driven recovery path, generation fences, audit contract, and record-detail diagnostics are implemented
- Lifecycle: active
- Catalog note: topic anchor: API Key / routing / sticky priority handoff
- Owner-facing surface: recovery admission diagnostics are shown in record details; the existing global Settings control remains the only control-plane toggle.

## Coverage / rollout summary

- 当前准入闸门只在候选已经通过普通排序成为首选后运行。模型路由健康比较早于账号优先级，因此一个严格更高优先级、但因临时模型故障处于 `degraded/demoted` 或冷却刚到期的目标会先输给普通健康来源，永远到不了单槽闸门。这正是线上超过一小时没有再次尝试 Ciii2 的实现缺口。
- 当前模型路由健康已经按 API Key 的精确请求模型维护冷却、时间栅栏与恢复状态；新增恢复通道复用这些事实，只绕过普通模型降权排序到达既有闸门，不改变常规候选比较器。
- 全局 Settings 开关 `priorityHandoffAdmissionEnabled` 已实现并默认开启。新增恢复通道继续由同一开关控制，不增加第二个设置；关闭时保留旧路由行为，重新开启时以新的本地状态代际从 `0/3` 验证。
- 运行时许可、冷却与恢复计数只存在于当前进程。设置和诊断可持久化，但没有持久化可用性时不得阻断请求。

## Implemented behavior map

### 1. Expose recovery eligibility in the immutable routing snapshot

- Extend `ModelRouteRuntimeSnapshot` in `src/upstream_accounts/routing/model_health.rs` with the persisted failure classification needed to distinguish temporary model-route recovery from cache protection, capability/model rejection, cancellation, and account-level hard failure.
- Add one helper that returns `degraded`, `cooling`, `cooldownExpired`, or `ineligible` for request-driven recovery. Keep the existing `penalty_at` and ordinary comparator behavior unchanged.
- Reuse the accepted result of `record_model_route_failure_inner`: only a new temporary model-route failure that survives the existing request-start, newer-success, and reset fences restarts local verification. A stale or reset-fenced failure must not allocate a new recovery generation.

### 2. Generalize the local generation fence

- In `src/upstream_accounts/routing/priority_handoff.rs`, generalize `reset_priority_handoff_for_model` into a recovery-verification restart operation used by both manual reset and accepted temporary failures.
- Preserve the existing in-flight permit across a restart so another recovery request cannot enter concurrently. Move the entry to `0/3 verifying` under a new generation; the old permit may release itself at terminal state but cannot change health-independent verification progress.
- Keep the existing global switch as the sole control plane. Closing it does not cancel an in-flight request; reopening allocates a fresh switch generation and starts at `0/3`.

### 3. Add a source-first recovery admission path

- In `src/upstream_accounts/routing/selection.rs`, derive the Authoritative Sticky Source before treating a cache miss or uncertain qualification as a fresh assignment.
- After hard account/model/binding/transport eligibility has been resolved, but before returning the ordinary model-penalty winner, calculate at most one Preferred Recovery Target. It must have strictly higher effective routing priority than the authoritative source or ordinary fresh-assignment alternative and must be eligible only because of an accepted temporary model-route failure.
- For `degraded`, let the next eligible real request attempt admission immediately. For `cooling_down`, do nothing until expiry; after expiry the next real request may attempt admission. Never schedule a timer, background probe, or waiting request.
- Pass only that target to the existing `admit_priority_handoff` gate. If admitted, route once with source `PriorityHandoff`. If busy or cooling, keep a sticky request on its source; for true fresh assignment, choose an ordinary healthy alternative or return the existing no-candidate outcome. Do not cascade to a second recovery target.
- Leave established Fault Failover outside this gate, including when it independently chooses the same account-model while one recovery request is in flight.

### 4. Couple terminal evidence without coupling sticky ownership

- In `src/upstream_accounts/routing/failure_recording.rs`, carry the boolean result of `record_model_route_success_from_attempt` into priority-handoff completion. Advance recovery verification only when the success passed the model-health request-start/reset fences and still belongs to the current recovery generation.
- Always release the old permit. A success that began no later than a newly accepted failure may not recover health or count toward the new generation even if it completes later.
- Keep the existing generation-guarded sticky mutation independent: a valid current success may recover model health and count even when a newer manual binding or Fault Failover causes the sticky ownership write to be rejected. It must never overwrite the newer binding.
- Preserve existing cancellation and uncertain-delivery behavior: cancellation only releases; an accepted temporary failure enters cooldown; no automatic replay is added.

### 5. Make the decision observable

- Extend the existing optional `handoffAdmission` audit object with optional `trigger`: `priorityAttraction` for the existing path and `modelRouteRecovery` for the new path. Historical payloads without the field remain valid.
- Use `requestDrivenRecoveryAdmission` as the winner reason when the recovery path sends the target. Keep `decision`, `phase`, `generation`, and `verificationSuccessCount` as the gate outcome.
- Update the Rust audit schema and the corresponding `web/src/lib/api/` types/normalizers. The Settings UI does not gain another toggle; only record-detail rendering needs adjustment if it enumerates these values.

### 6. Regression coverage and rollout

- Add stateful SQLite resolver tests for degraded immediate recovery, active versus expired cooldown, exactly one concurrent recovery request, multiple recovery targets, sticky versus true fresh fallback behavior, and exclusion of cache-protected or hard-ineligible targets.
- Add terminal-ordering tests for first recovery success becoming `1/3`, accepted newer failure fencing an older success, stale failure not resetting progress, concurrent sticky rebind, process restart, switch close/reopen, and Fault Failover to the same target.
- Add priority-handoff unit tests for permit preservation across generation restart and audit serialization compatibility for missing `trigger`.
- Run focused named Rust tests first, then the `stateful-sqlite` backend profile. Run Web typecheck and unit tests only if the audit payload consumer changes; no UI visual-evidence gate is needed unless rendered record details change.
- Roll out behind the already-default-on global switch. After deployment, verify that a degraded higher-priority target receives at most one request-driven attempt, busy requests remain on their sources, successful attempts progress `1/3 -> 2/3 -> 3/3`, and no one-hour retry gap recurs after cooldown expiry.

## Remaining Gaps

- Real-upstream cancellation and uncertain-delivery behavior continues to rely on the existing transport harness; the change preserves its conservative terminal rules.

## Related Changes

- `e46acd37 docs(routing): define priority handoff admission contract`

## References

- `./SPEC.md`
- `./HISTORY.md`
- `../../adr/0002-stage-automatic-priority-handoffs.md`
