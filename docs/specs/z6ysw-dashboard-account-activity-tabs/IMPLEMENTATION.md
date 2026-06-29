# Dashboard 工作区卡片双视图与上游账号活动聚合 实现状态（#z6ysw）

> 当前有效规范仍以 `./SPEC.md` 为准；这里记录实现覆盖、交付进度与 rollout 相关事实，避免这些细节散落到 PR / Git 历史里。

## Current Status

- Implementation: 已实现
- Lifecycle: active
- Catalog note: dashboard workspace dual tabs, account activity aggregation, invocation-based in-progress semantics

## Coverage / rollout summary

- 已实现：Dashboard 页面提升并共享顶部 range 状态，工作区 section 接入 `对话 / 上游账号` 双 tabs，并保留既有对话 working-set 行为。
- 已实现：新增 `GET /api/stats/upstream-account-activity` 批量接口，返回账号级聚合摘要、recent 4 bounded query，以及 `yesterday` closed-range 的空 live count 语义。
- 已实现：summary `inProgressConversationCount` / `inProgressRetryConversationCount` 改为 invocation-based 语义，Dashboard owner-facing 文案同步为“进行中调用 / 重试调用”。
- 已实现：账号 tab 懒加载、`usage` disabled + 自动回退、独立账号卡布局、Storybook 交互场景、视觉证据与 targeted validation。
- 已实现：账号卡收敛为紧凑信息卡，移除状态说明条和解释性废话；请求数 / Token 分解仅保留色点与数值，连单字 / 缩写短标签也不常驻显示，完整标签通过 hover 暴露；recent 区标题行右侧统计保留完整状态文字并与标题垂直对齐。
- 已实现：账号卡内部结构描边统一压回低对比中性边框，外框、摘要格子、recent 行与分隔线不再复用主题蓝或其他语义色作为结构边界；颜色仅保留在状态点、数值和 badge 上。
- 已实现：账号卡底部 4 条 recent 调用记录全部留在卡内可见，单条 recent 行补齐 endpoint、Token 摘要与 `RQ / UP / ED / TT` 时序摘要，使信息密度不低于对话卡片中的调用记录。
- 已实现：桌面宽屏账号卡固定高度收敛到更紧凑值，避免整页面板观感，同时保持 4 条 recent 记录完整可见。
- 已实现：上游账号 recent 行改为“对话短 ID + 请求 ID”主标识布局，短 ID 基于真实 `promptCacheKey` 计算并去掉 `WC-` 前缀；点击详情时传递的 `selection.promptCacheKey` 也已修正为真实对话键。
- 已实现：上游账号 recent 行的对话短 ID 从“连续色圆点 + 短码”收口为轻量 identity chip；chip 以短码文本为主识别，颜色降为离散辅助槽位，不再与状态徽标混淆语义。
- 已实现：identity chip 的离散槽位改为基于完整稳定 hash 做高低位混合后再映射，修正真实线上数据因低位 `% 8` 偏置导致的短码成片撞色问题。
- 已实现：上游账号 recent 行不再重复显示账号名；当 `requestModel` / `responseModel` 规范化后仍不一致时，recent 行改为同时展示请求模型、切换图标与响应模型。
- 已实现：上游账号 recent 行的 endpoint、reasoning effort 与双模型 badge 统一复用 compact 尺寸 recipe，消除同一行内 badge 高度不一致问题。
- 已实现：上游账号卡片标题区改为账号名 + 文本型实时 `TPM / 消费速率` 指标，删除卡内 `渠道 / 分组` 行和顶部 `调用` 指标；周期统计重排为首字用时、请求数、成本、Token 四组，并沿用滚动数字效果。
- 已实现：账号活动接口补出 `avgTotalMs`、`totalCost`、严格失败 `failureCost` 与 `failureTokens`；失败比率由前端按 `failureCount / requestCount` 计算，`其他` 按 `nonSuccessCount - failureCount` 下限归零。
- 已实现：账号卡列表按 `totalTokens` 倒序排列，并用最近调用时间与账号 ID 作稳定排序兜底。

## Remaining Gaps

- 无已知功能缺口；后续仅保留常规回归与数据量增长下的聚合性能观察。

## Related Changes

- `src/api/slices/invocations_and_summary.rs`
- `src/api/slices/settings_models_and_cache.rs`
- `src/maintenance/hourly_rollups.rs`
- `web/src/pages/Dashboard.tsx`
- `web/src/components/DashboardActivityOverview.tsx`
- `web/src/components/DashboardWorkingConversationsSection.tsx`
- `web/src/hooks/useDashboardUpstreamAccountActivity.ts`
- `web/src/lib/api/core-foundation.ts`

## References

- `./SPEC.md`
- `./HISTORY.md`
