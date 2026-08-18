# Dashboard Hot Topic 内存投影与 SSE 稳定性演进历史

> 这里记录会影响 Agent 理解“为什么一步步变成现在这样”的关键演进；单次任务流水账不放这里，规范正文仍以 `./SPEC.md` 为准。

## Decision Trace

- 将 working-conversations、parallel-work 与 open-window timeseries 从通用订阅构建路径中独立出来，定义为必须使用 typed projection 的 Dashboard HotProjection。
- 将 activity topic 的 `recentLimit` 固定为 16，动态显示数量改由客户端本地截断，避免数据变化触发 SSE descriptor 重建。
- 将 working-conversations 卡片固定为 current/previous/earlier 三槽位：三条记录共享既有 16 条 recent 预览，缺失时保留紧凑占位，从而让卡片扫描密度稳定且不引入新的后端数据合同。
- 将每个正常槽位收紧为“时间 + 模型 + 读取状态”与“账号 + 右端用量”两行；失败槽位额外保留无 label 的错误摘要，以保持诊断能力并消除重复的 owner-facing label。
- 将工作对话卡片、上游账号卡片 recent 行及账号详情调用记录的紧凑延迟显示收敛为最多一位小数，并在四舍五入后达到 100 秒时显示整数；TTFT 与响应耗时分别统一为 `firstTokenMs` 与 `tUpstreamStreamMs`，`0 ms` 保持为合法的已测得 TTFT，只有缺失值显示 `--`。持久化列表和两个工作流 hydration 以 `attempt_index DESC, id DESC` 选择唯一最终真实 upstream attempt 承载调用级 TTFT 与用量，并排除 `budget_exhausted_final` 伪终态，较早 retry 显示 `--`。请求或排队中缺失字段显示 `--`，响应中保留已测得的 TTFT 而未完成响应耗时显示 `--`。持久化 SQL、HTTP 运行时 hydration 叠加、SSE、客户端合并与 Demo mock 都不再丢失有限的零毫秒首 token；负值或非有限值不被作为已测得 TTFT 或成功色展示，格式化与成功色共用有限、非负测量谓词，Demo workflow 也不再为进行中调用伪造流耗时；同样不以 `tUpstreamTtfbMs`、流耗时或经过时长判定“响应中”，避免以不同的指标冒充 TTFT；账号详情 TTFT 统一使用绿色，并修复深色主题 `surface-card` 的浅色残留。
- 将账号详情定位跳转的聚焦轮廓提升至调用记录的顶层伪元素，避免全宽明细块覆盖父级 `border` / `ring-inset` 而造成轮廓缺边；截图状态在读取响应体后重新收起并失焦，防止临时按钮焦点进入静态证据。
- 将 working-conversations Storybook 的默认 workspace view 固定为 `conversations`，并让移动视觉证据沿用 `bg-base-200` 边距，避免前序 Story 的持久化视图或临时截图边框制造错误的留白状态。
- 将账号详情调用的页面级视觉证据从 `InvocationTable` 组件 Story 外壳迁回 mock-only `ui_demo`；Demo runtime 因而必须覆盖账号 `call-attempts` 接口，并用首 token 已到达、流耗时未完成的记录验证响应中展示。
- 将 System Status additive 诊断视为允许的 owner-facing 只读变更；Dashboard 的既有交互和公开数据合同保持不变。

## Key Reasons / Replacements

- 既有 activity、summary 和 network typed materializer 只能证明部分 topic 已收口，不能证明完整 Dashboard 页面不会触发数据库或通用 JSON builder。
- 生产观测显示 Dashboard 页面活跃与 CPU/SQLx 占用相关；剩余三条通用 topic 是需要单独验收的架构缺口。
- 本主题使用完整 Dashboard topic bundle、per-topic build/SQL/serialization 计数和页面 A/B 作为完成证明，替代单组件“零 SQL”结论。

## References

- `./SPEC.md`
- `./IMPLEMENTATION.md`
- `../high-frequency-runtime-data-plane/HISTORY.md`
