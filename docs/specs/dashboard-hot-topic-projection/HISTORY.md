# Dashboard Hot Topic 内存投影与 SSE 稳定性演进历史

> 这里记录会影响 Agent 理解“为什么一步步变成现在这样”的关键演进；单次任务流水账不放这里，规范正文仍以 `./SPEC.md` 为准。

## Decision Trace

- 将 working-conversations、parallel-work 与 open-window timeseries 从通用订阅构建路径中独立出来，定义为必须使用 typed projection 的 Dashboard HotProjection。
- 将 activity topic 的 `recentLimit` 固定为 16，动态显示数量改由客户端本地截断，避免数据变化触发 SSE descriptor 重建。
- 将 working-conversations 卡片固定为 current/previous/earlier 三槽位：三条记录共享既有 16 条 recent 预览，缺失时保留紧凑占位，从而让卡片扫描密度稳定且不引入新的后端数据合同。
- 将缺失历史从两条灰色骨架线替换为有明确位置语义的静态历史说明，并让缺失槽位与普通无方案调用行共享 57px 基线；同时将紧凑时长统一为无空格的 `4.7s` 记法并固定 TTFT/响应时间组间距为 4px，避免局促卡片中的加载错觉与数值粘连。
- 将每个正常槽位收紧为“时间 + 模型 + 读取状态”与“账号 + 右端用量”两行；失败槽位额外保留无 label 的错误摘要，以保持诊断能力并消除重复的 owner-facing label。
- 将账号 recent 行的整行调用入口改为独立原生按钮，并把账号、状态与错误摘要保留为同级子控件，消除 nested interactive 结构，同时保持调用详情、账号跳转、错误读取和键盘访问。
- 将工作对话卡片、上游账号卡片 recent 行及账号详情调用记录的紧凑延迟显示收敛为最多一位小数，并在四舍五入后达到 100 秒时显示整数；TTFT 与响应耗时分别统一为 `firstTokenMs` 与 `tUpstreamStreamMs`，有限且非负的 TTFT（包括 `0 ms`）有效，响应耗时仅有限且严格大于零有效，零、负值或非有限值显示 `--`。持久化列表和两个工作流 hydration 以 `attempt_index DESC, id DESC` 选择唯一最终真实 upstream attempt 承载调用级 TTFT 与用量，最终 attempt 必须是终态，并排除 `budget_exhausted_final` 伪终态，较早 retry 显示 `--`；失败终态的零毫秒 TTFT 仅在最终 attempt first-byte 同为零时保留，正值需要最终 attempt stream 证据。请求或排队中缺失字段显示 `--`，响应中保留已测得的 TTFT 而未完成响应耗时显示 `--`。持久化 SQL、HTTP 运行时 hydration 叠加、SSE、客户端合并与 Demo mock 都不再丢失有限的零毫秒首 token；JSON DTO、客户端合并和 hourly `upstreamStream` rollup 在边界处拒绝负值/非有限值及零流耗时。负值或非有限值不被作为已测得 TTFT 或成功色展示，格式化、成功色、汇总样本与工作流的流耗时优先级分别使用有限非负 TTFT 与有限正响应耗时判定，因此无效流耗时不会遮蔽有效 TTFT，SQL 汇总和账号性能汇总也排除非有限值，Demo workflow 也不再为进行中调用伪造流耗时；同样不以 `tUpstreamTtfbMs`、流耗时或经过时长判定“响应中”，避免以不同的指标冒充 TTFT；账号详情 TTFT 统一使用绿色，并修复深色主题 `surface-card` 的浅色残留。
- 明确最终 attempt 的实时例外：进行中的最终真实 attempt 只有自身 phase 已进入 `responding` / `streaming_response` 时才承接已测得 TTFT；仅为 `running` 或 `waiting_first_byte` 的 retry 不继承 earlier attempt 值，终态 delta 保留最终 retry 已写回的合法 TTFT。普通调用 summary 均值/P95、Dashboard 汇总与账号 model-performance 均按最终 attempt 过滤 TTFT 与响应耗时，workflow final-row 选择同样无条件排除 `budget_exhausted_final`；可执行 SQLite fixture 覆盖 persisted Dashboard baseline、终态 retry delta 和聚合隔离。
- 将仅含 `codex_invocations` 的历史归档明确为重试明细不可用的兼容来源：聚合保留归档已存储且有效的 TTFT 与响应耗时，不向旧归档注入最终 attempt 子查询。
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
