# Forward Proxy 节点延迟与订阅刷新 - Implementation（#kmr5z）

> 当前有效规范仍以 `./SPEC.md` 为准；这里记录实现覆盖、交付进度与 rollout 相关事实，避免这些细节散落到 PR / Git 历史里。

## Current Status

- Implementation: 已实现
- Lifecycle: active
- Catalog note: Settings 页已新增 forward proxy 延迟测试和订阅刷新入口。

## Coverage / rollout summary

- 后端新增强制刷新订阅接口、单节点/批量节点延迟测试 SSE 接口。
- 单节点测试最多 5 轮，每轮 5 秒预算，整体 15 秒预算；批量测试按 round-first 顺序调度。
- 延迟均值只统计成功样本，出站 IP 与 OAuth 上游成功耗时均可贡献样本。
- Settings 页新增延迟列、测试全部、刷新订阅按钮、渐进结果状态和中英文文案。
- Storybook `SettingsPage` 新增 forward proxy latency/refresh 场景并作为视觉证据来源。

## Remaining Gaps

- None

## Verification

- `cargo fmt --check`: passed
- `cargo test manual_latency -- --test-threads=1`: passed
- `cargo test`: blocked by existing timing-sensitive `tests::pool_route_body_sticky_wait_timeout_returns_total_timeout_error_before_first_attempt`
- `cd web && bun run test`: passed
- `cd web && bun run build`: passed
- `cd web && bun run build-storybook`: passed
- Storybook canvas evidence captured from `Settings/SettingsPage/Forward Proxy Latency And Refresh`
- Codex review loop: fixed two backend/UI P2 findings and converged to clear

## Related Changes

- Backend: `src/forward_proxy/`, `src/maintenance/hourly_rollups.rs`
- Frontend: `web/src/pages/Settings.tsx`, `web/src/lib/api/`, `web/src/components/SettingsPage.stories.tsx`
- Visual evidence: `./assets/settings-forward-proxy-latency-refresh.png`

## References

- `./SPEC.md`
- `./HISTORY.md`
