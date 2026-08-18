# Dashboard Hot Topic 内存投影与 SSE 稳定性实现状态

> 当前有效规范仍以 `./SPEC.md` 为准；这里记录实现覆盖、交付进度与 rollout 相关事实，避免这些细节散落到 PR / Git 历史里。

## Current Status

- Implementation: child tickets complete; aggregate rollout validation pending
- Lifecycle: active
- Catalog note: Dashboard 七条 hot topic 已具备 typed materializer、稳定 selection 与只读健康诊断

## Coverage / rollout summary

- working-conversations、open-range parallel-work 与 open-window timeseries 均使用 revision-aware typed materializer；健康 live path 不进入通用 builder。
- activity SSE descriptor 固定 `recentLimit=16`，组件按当前可见数量在客户端截断，避免 visibility 变化重建 topic key。
- working-conversations 卡片继续消费每 key 最多 16 条 recent 预览，但客户端固定映射 current/previous/earlier 三个槽位；缺失槽位使用两行中性占位，正常记录收紧为两行，失败记录保留无 label 的错误摘要行。
- 三槽位卡片保留调用详情、账号跳转、键盘可达性、完整值 title/aria 与 blocked/in-flight 诊断；该 owner-facing 信息密度变化不修改 HTTP/SSE wire shape 或后端 recent 上限。
- 工作对话卡片、上游账号卡片的 recent 行与账号详情调用记录共用紧凑延迟合同：四舍五入后达到 100 秒时省略小数。TTFT 使用 `firstTokenMs`，响应耗时始终使用 `tUpstreamStreamMs`；`0 ms` 保持为已测得 TTFT，只有 `null` / 缺失显示 `--`。持久化列表与工作流详情/账号 workflow hydration 都以 `attempt_index DESC, id DESC` 选择唯一的最终真实 upstream attempt 来投影调用级 TTFT 和调用级用量，排除 `budget_exhausted_final` 伪终态，较早 retry 保持 `--`。持久化 SQL、HTTP 运行时 hydration 叠加、SSE、客户端合并与 Demo mock 都保留有限的零值；只有非负、有限的已测得值可以输出或接受“响应中”，负值与非有限值显示不可用。请求或排队中的缺失字段显示 `--`，响应中必须保留已测得的 TTFT，未完成的响应耗时显示 `--`，不再显示 `occurredAt` elapsed。账号详情的 TTFT 汇总和记录行使用 `text-success`，与其他成功指标对齐；深色主题的 `surface-card` 使用不透明的 base-100/base-200 混合，避免记录行出现浅色残留。
- 账号详情由定位跳转激活的调用记录以顶层伪元素绘制聚焦轮廓，确保完整的四边圆角轮廓位于明细块之上；明细块和指标轨道继续各自裁切内部内容，Story 在截图前收起临时展开的响应体并释放其焦点。
- working-conversations Storybook 默认强制 `conversations` workspace view；需要验证上游账号视图的 Story 显式覆盖该默认值，避免持久化 `localStorage` 状态污染后续截图与交互断言。移动组件证据边距复用 Storybook `bg-base-200`，不注入任意颜色。
- Demo runtime 为账号详情请求页补齐 `call-attempts` 列表、筛选和分页 mock，并提供“已测得首 token、未完成流耗时”的响应中记录；页面级证据改从无登录、无真实后端依赖的 `ui_demo` 捕获，不再使用组件 Story 外壳。
- `runtimePressureHealth.dashboardHotTopics` 按七条 topic 报告 class、state、subscriber、build、fallback、live DB read、serialization、cadence miss 与 reconnect churn。
- System Status 以只读方式展示 healthy、deferred、hot-DB-read 与 cadence-miss；字段缺失时保持 unknown 兼容。
- 完整 Dashboard topic bundle 的 10,000 mutation、双订阅共享 frame 与零 fallback/DB-read 门禁由 stateful topology test 覆盖。
- Dashboard runtime 投影的延迟验证以 20 个更新样本的 `400ms` P95 为合同；单次传递、parallel-work materialization 与成功代理后的 SQLite sticky-route 写入使用独立的有界观察窗口，只隔离测试调度抖动，不放宽该产品目标或路由断言。

## Remaining Gaps

- 在 aggregate PR 完成 integration CI、owner acceptance 与线上受控 A/B 验收。

## Related Changes

- Ticket #785: working-conversations typed projection and bounded recovery.
- Ticket #786: parallel-work typed projection.
- Ticket #787: open-window timeseries typed materializer.
- Ticket #792: exhaustive topic classification, stable activity selection, hot-topic health and System Status diagnostics.

## References

- `./SPEC.md`
- `./HISTORY.md`
- `../high-frequency-runtime-data-plane/IMPLEMENTATION.md`
