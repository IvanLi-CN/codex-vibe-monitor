# Forward Proxy 节点延迟与订阅刷新 - Implementation（#kmr5z）

> 当前有效规范仍以 `./SPEC.md` 为准；这里记录实现覆盖、交付进度与 rollout 相关事实，避免这些细节散落到 PR / Git 历史里。

## Current Status

- Implementation: 已实现
- Lifecycle: active
- Catalog note: Settings 页已新增 forward proxy 延迟测试和订阅刷新入口。

## Coverage / rollout summary

- 后端新增强制刷新订阅接口、单节点/批量节点延迟测试 SSE 接口。
- 单节点测试最多 5 轮，每轮 5 秒预算，整体 15 秒预算；批量测试按 round-first 顺序调度。
- 延迟测试目标集包含出站 IP、OAuth `/models` 与 Codex `/responses`；每轮必须三个目标全部可达才写入成功 probe attempt。
- 延迟均值只统计成功样本，出站 IP、OAuth `/models` 与 Codex `/responses` 成功耗时均可贡献展示样本；任一目标失败会让节点最终显示异常。
- Settings 页新增延迟列、测试全部、刷新订阅按钮、渐进结果状态和中英文文案。
- Storybook `SettingsPage` 新增 forward proxy latency/refresh 场景并作为视觉证据来源。

## Remaining Gaps

- None

## Verification

- `cargo fmt --check`: passed
- `cargo test manual_latency -- --test-threads=1`: passed
- `cargo test`: passed
- `cd web && bun run test`: passed
- `cd web && bun run build`: passed
- `cd web && bun run build-storybook`: passed
- `cd web && bun run test-storybook`: passed
- Storybook canvas evidence captured from `Settings/SettingsPage/Forward Proxy Latency And Refresh`: passed; committed visual evidence shows `trojan.example.com:443` as `--` and the hover/focus tooltip with `Codex /responses` failure details.
- Codex review loop: fixed URL path preservation and budget-exhaustion result preservation findings; latest review reported no actionable correctness issues

## Related Changes

- Backend: `src/forward_proxy/`, `src/maintenance/hourly_rollups.rs`
- Frontend: `web/src/pages/Settings.tsx`, `web/src/lib/api/`, `web/src/features/settings/SettingsPage.stories.tsx`
- Visual evidence: `./assets/settings-forward-proxy-latency-refresh.png`

## References

- `./SPEC.md`
- `./HISTORY.md`
