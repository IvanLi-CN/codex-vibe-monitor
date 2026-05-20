# 账号池维护执行记录与出口限频 - History

## Key Decisions

- 2026-05-10: 采用全局列表而不是账号详情内聚合视图，便于跨账号排查维护请求。
- 2026-05-10: 限频采用固定间隔，不新增配置项。
- 2026-05-11: 限频固定间隔调整为 10 秒，仍不新增配置项。
- 2026-05-10: 限频粒度使用最终网络出口；无代理/direct 作为单独出口。
- 2026-05-10: OAuth 维护流程中的后续外呼若被限频，应写入 deferred 记录并恢复账号运行状态，避免停留在 `syncing`。
- 2026-05-11: 生产发现 reset due 的 OAuth quota exhausted 账号会被同 egress 维护批量调度反复饿死，只产生 `sync_deferred / egress_throttled` 而拿不到后续 usage snapshot。运行期维护改为在有界预算内等待同出口槽位，预算耗尽后才沿用 deferred 行为；限流进入/退出状态机保持不变。
- 2026-05-11: `sync_deferred / egress_throttled` 不再消耗 reset catch-up 窗口；它仍作为普通维护间隔 anchor，避免非 reset-due 账号无限重排。
- 2026-05-20: 生产排查发现账号维护轮次被 DB pressure gate 的唯一后台槽位稳定饿死：startup backfill 在任务未 due 时也先占槽，upstream account maintenance 遇到 `BackgroundBusy` 又立即跳过整轮。调度语义调整为 startup backfill preflight 不占槽，账号维护只对 `BackgroundBusy` 做短预算等待，pressure cooldown 继续 fail-soft skip；quota exhausted 账号仍必须等真实 usage snapshot 后才恢复路由。
