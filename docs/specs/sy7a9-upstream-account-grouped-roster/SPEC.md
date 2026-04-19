# 上游账号列表分组视图与代理徽章（#sy7a9）

## 背景 / 问题陈述

- 当前上游账号列表只有平铺分页表格；当主人需要按分组查看账号与节点排班关系时，必须手动依次筛选分组，无法直接得到“每组有哪些账号、每个账号当前挂在哪个正向代理”的全貌。
- 既有平铺列表也没有导出当前正向代理信息；对于启用 group-bound proxy / node shunt 的场景，只看工作态与阻断原因，仍不足以判断账号实际命中了哪个节点，或为什么还在候补。
- 现有分页表格已经解决了平铺模式的大列表首屏成本，但新的“分组 -> 组内账号”视图会把数据放大成双层列表；若直接完整渲染所有组卡片与所有成员行，前端会在大数据量下明显变慢。
- 账号列表响应里的 `forwardProxyNodes` 目前仍是空占位，组摘要与代理 badge 还没有一个统一可复用的 roster catalog 真相源。

## 目标 / 非目标

### Goals

- 在 `/account-pool/upstream-accounts` 列表标题区新增 `平铺 / 分组 / 网格` 切换，默认保持平铺。
- 分组模式下按当前筛选结果聚合为“每组一张卡片”，卡片纵向排布且不分页；组内成员继续复用现有账号列表行信息密度与交互。
- 网格模式下继续按分组聚合为“每组一张卡片”，左侧固定显示分组信息，右侧改为账号卡片网格；网格模式不提供批量选择。
- 分组卡片左侧固定展示：组名、账号数、非零 Free/Plus/Team 计数 badge、并发数、`独占节点` badge（仅 `nodeShuntEnabled=true` 时显示）。
- 每个分组摘要区都提供复用现有 `Group settings` 弹窗的设置按钮；点击后直接打开当前分组设置，不新增独立页面或第二套弹层。
- 平铺行与分组成员行统一新增当前正向代理 badge：已分配显示代理名，分组有可用节点但该账号未排到节点显示 `候补中`，没有可用代理显示 `未配置代理`。
- 网格成员卡片只展示高价值信息：账号名称、账号类型/套餐 badge、额度使用情况；误导性低价值信息（如 `planType=local` badge）不展示。
- 为 grouped roster 提供稳定性能边界：分组/网格视图改为页面级 `window` 滚动驱动的组卡片虚拟化；大数据量下 DOM 挂载的组卡片数必须显著低于总组数，且不再依赖组内纵向滚动容器。
- 扩展 `GET /api/pool/upstream-accounts`：支持 `includeAll=1` 跳过分页切片，并把当前代理状态与真实 `forwardProxyNodes` catalog 一并返回。

### Non-goals

- 不改造账号创建/编辑流程、详情抽屉字段排布或分组设置弹窗交互。
- 不把平铺模式改成虚拟列表；平铺继续沿用现有分页表格，只补代理 badge。
- 不改变 forward proxy 选择算法、node shunt 分配策略或 shared bound group 的切换规则。
- 不把 view mode 写入现有筛选持久化 payload。
- 不再为 grouped/grid 额外设计“组内成员独立滚动”的交互；分组页纵向浏览统一交给整页滚动。

## 范围（Scope）

### In scope

- `src/upstream_accounts/**`：账号列表 query 扩展、当前代理读模型、roster 级 forward-proxy catalog。
- `src/forward_proxy/**`：只读 helper，暴露 bound-group 当前 binding / 绑定节点 catalog 查询所需运行时状态。
- `web/src/lib/api/**`、`web/src/hooks/useUpstreamAccounts.ts`：`includeAll` 查询、代理字段与 grouped-mode 数据路径。
- `web/src/pages/account-pool/UpstreamAccounts.page-impl.tsx`
- `web/src/components/UpstreamAccountsTable.tsx` 与新增 grouped roster 相关组件
- `web/src/components/UpstreamAccountsPage*.stories.tsx`
- `web/src/components/UpstreamAccountsTable.test.tsx`
- `web/src/pages/account-pool/UpstreamAccounts.test.tsx`
- `web/src/hooks/useUpstreamAccounts.test.tsx`
- 相关 i18n 文案与 Rust tests

### Out of scope

- OAuth / API Key 新建页与批量导入页的代理展示。
- Settings 页 forward proxy 管理 UI。
- invocation / records 页面代理 badge 风格对齐。

## 接口契约（Interfaces & Contracts）

### `GET /api/pool/upstream-accounts`

- 新增 query 参数：`includeAll`
  - `includeAll=1` 时，继续应用当前筛选语义，但不做 `page/pageSize` 分页切片。
  - `includeAll` 缺省或为假时，继续保持现有服务端分页语义。
- `UpstreamAccountSummary` 新增字段：
  - `currentForwardProxyKey?: string | null`
  - `currentForwardProxyDisplayName?: string | null`
  - `currentForwardProxyState: "assigned" | "pending" | "unconfigured"`
- `currentForwardProxyState` 口径固定为：
  - `assigned`：当前 live routing truth 已能定位到具体绑定代理。
  - `pending`：分组存在可用 bound proxy，但当前账号在 node shunt / live reservation 语义下尚未分配到节点。
  - `unconfigured`：账号/分组当前没有任何可用代理。
- `forwardProxyNodes` 必须返回与当前 roster 相关分组的真实 binding node catalog；缺失历史 key 继续复用现有 metadata 恢复语义。

## 功能规格

### 视图切换

- 列表标题区右上角新增 `SegmentedControl`，提供 `平铺`、`分组` 与 `网格` 三个选项。
- 默认进入 `平铺`。
- 切到 `分组` 或 `网格` 后：
  - 使用 `includeAll=1` 拉取当前筛选结果的全量账号。
  - 隐藏分页 footer。
  - 不清空既有筛选。
- 仅 `分组` 视图保留 bulk selection 语义；`网格` 视图不显示“选择当前页”、不显示批量操作工具条，也不在成员卡片里显示 checkbox。
- 切回 `平铺` 后：
  - 恢复此前的 `page/pageSize` 状态。
  - 继续显示分页 footer。

### 分组卡片

- 分组顺序沿用 `groups[]` catalog 顺序；未分组账号聚合成单独“未分组”伪卡片并追加在末尾。
- 每张卡片左侧显示：
  - 组名
  - 非零 `free / plus / team / enterprise / api` 计数 badge
  - `并发 <n>`
  - `独占节点` badge（仅 `nodeShuntEnabled=true`）
- 左侧统计 badge 不显示 `local`；`API Key` 账号数量统一以 `API` badge 表达。
- 每张卡片右侧显示当前分组全部成员，信息层级与平铺行保持一致，并额外带出代理 badge。
- 分组列表视图采用“单层组卡 + 左侧摘要栏 + 右侧扁平成员列表”样式：
  - 每个分组只保留一层主卡片外壳；
  - 左侧摘要栏与右侧成员列表直接集成在同一张主卡片内；
  - 不允许再额外包一层独立的“摘要子卡片”或“成员容器子卡片”；
  - 左侧摘要栏必须采用紧凑布局：组名与账号数同一行、套餐/并发/独占信息并入同一块、绑定代理保持单行信息流；摘要栏按内容高度收缩，不再被强制拉伸到与右侧成员区等高。
- 分组列表成员行应采用扁平列表风格，而不是独立小卡片堆叠：
  - 行与行之间通过分隔线或轻量 hover 背景区分；
  - 仅在选中态允许出现轻量强调，不使用厚描边和重复圆角边框；
  - 单账号分组应尽量让左侧摘要高度与右侧单行成员高度接近，不再通过备注或冗余留白把整组撑高。

### 虚拟化

- 分组模式与网格模式统一采用页面级 `window` 滚动驱动的组卡片虚拟化，不再在 roster 主容器或组成员区保留纵向内部滚动条。
- 虚拟化层级收敛为单层：
  - 外层：分组卡片虚拟列表
  - 内层：成员区使用正常文档流渲染，不再做组内纵向虚拟列表
- 组卡片虚拟化必须满足：
  - 使用真实 DOM 高度测量，而不是只依赖固定估算值
  - 切换 `平铺 / 分组 / 网格`、筛选变更、窗口 `resize` 后都要重新测量，避免旧高度缓存导致的覆盖或错位
  - 可见组卡片必须在正常文档流中排布，禁止通过嵌套绝对定位列表制造同层卡片重叠
- 分组列表视图中，成员区高度应按实际成员数自然展开；仅 1 个成员时不得再预留第 2 条的空白高度。
- 网格视图中，右侧成员区高度应跟随内容自然展开，不得因为左侧分组信息更高而被强制拉伸，也不得再出现组内滚动容器。
- 页面级虚拟化不得破坏现有整行点击、checkbox、chevron、detail drawer route、bulk selection 语义。

### 网格视图布局

- 网格视图沿用分组卡片外壳：左侧为分组信息，右侧为账号成员卡片网格。
- 当前桌面视口下，右侧成员区应优先呈现 3 列网格；当可用宽度不足时允许按响应式规则退化为 2 列。
- 网格视图为了把整卡高度压到“右侧 1~5 行”的范围内，左侧分组信息采用紧凑版，不显示分组备注。
- 单个账号卡片展示：
  - 账号名称
  - 账号类型 badge（如 `OAuth` / `API Key`）
  - 套餐 badge（如 `Free` / `Plus` / `Team` / `Enterprise`；`local` 不展示）
  - 5h / 7d 额度使用情况
- 网格成员卡片继续保留点击打开详情抽屉的行为。

### 代理 badge

- 平铺表格行与 grouped 成员行统一显示代理 badge。
- badge 文案优先级：
  1. `assigned` => `currentForwardProxyDisplayName`
  2. `pending` => `候补中`
  3. `unconfigured` => `未配置代理`
- 代理 badge 必须来自后端 live routing 读模型，不允许前端根据 `routingBlockReasonMessage` 或历史调用记录自行推断。

## 验收标准（Acceptance Criteria）

- Given 账号页打开，When 查看列表标题区，Then 可见 `平铺 / 分组 / 网格` 切换，且默认激活 `平铺`。
- Given 切到分组模式，When 当前筛选结果包含多个分组，Then 页面展示为一列组卡片且不再显示分页 footer。
- Given 切到网格模式，When 当前筛选结果包含多个分组，Then 页面展示为一列组卡片、右侧为账号卡片网格、且不再显示分页 footer。
- Given 处于分组或网格模式，When 查看分组摘要栏，Then 每个现有分组都可见 `编辑分组设置` 按钮，点击后直接打开复用的 `Group settings` 弹窗并带入当前分组设置。
- Given 某组存在 `free/team` 账号且 `plus=0`，When 渲染组卡片，Then 左侧只显示 `free/team` 计数 badge，不显示 `plus`。
- Given 某组包含 `API Key` 账号，When 渲染组卡片左侧统计，Then 显示 `API <n>` 而不是 `local <n>`。
- Given 某组开启 `nodeShuntEnabled`，When 渲染组卡片，Then 左侧出现 `独占节点` badge；关闭时不显示。
- Given 某账号已命中具体代理，When 在平铺或分组模式查看该账号行，Then 显示对应代理名 badge。
- Given 某账号所在分组有可用节点但当前未排到节点，When 查看该账号行，Then 代理 badge 显示 `候补中`。
- Given 某账号/分组没有任何可用代理，When 查看该账号行，Then 代理 badge 显示 `未配置代理`。
- Given 处于分组列表视图，When 查看单个分组卡片，Then 主视觉层级应为“一张组卡片 + 一列扁平成员行”，而不是多层嵌套子卡片。
- Given 处于分组列表视图，When 查看左侧摘要栏，Then 不显示分组备注，且组名/账号数/统计/绑定代理采用紧凑信息流，避免把单账号分组撑成明显高于右侧单行成员的卡片。
- Given 某个分组在分组列表视图中只有 1 个成员，When 渲染该组，Then 右侧成员区不得再强制预留第二行的最小高度。
- Given 某个分组在分组列表视图中只有 2 个较短成员行，When 渲染该组，Then 成员区应随内容自然收缩，而不是继续保留旧卡片时代的 300px+ 空白容器。
- Given 处于网格模式且某组只有 1~少量成员，When 渲染组卡片，Then 卡片右侧成员区应随内容自然收缩，不得因为左栏更高而出现右侧大块空白。
- Given 处于网格模式，When 查看左侧分组信息，Then 不显示分组备注，以避免分组卡高度被左栏文本拉高。
- Given 处于当前桌面视口，When 查看网格模式右侧成员区，Then 优先呈现 3 列成员卡片。
- Given 处于分组或网格模式，When 浏览长列表，Then 纵向滚动应由整页滚动条承载，而不是 roster 主容器或组成员区内部滚动。
- Given 分组模式加载大数据 Storybook 场景，When 检查 DOM，Then 已挂载的组卡片数显著少于总数据量，且滚动、切 tab、筛选变化、窗口 resize 后内容继续正确测量与交互，不出现重叠。
- Given 用户在任一视图勾选账号、点击整行或 chevron 打开详情，When 来回切换视图，Then 现有 bulk selection 与 detail drawer route 行为保持一致。

## 质量门槛（Quality Gates）

- `cargo check`
- `cargo test upstream_accounts -- --nocapture`
- `cd web && bunx vitest run src/hooks/useUpstreamAccounts.test.tsx src/components/UpstreamAccountsTable.test.tsx src/pages/account-pool/UpstreamAccounts.test.tsx`
- `cd web && bun run build`
- `cd web && bun run build-storybook`
- Storybook + 浏览器 smoke：验证 `平铺 / 分组` 切换、代理 badge 三态、虚拟化大数据场景。

## 里程碑（Milestones）

- [x] M1: 新建增量 spec，冻结视图切换、代理 badge 与 grouped roster 契约。
- [x] M2: 后端补齐 `includeAll`、当前代理读模型与 roster `forwardProxyNodes` catalog。
- [x] M3: 前端落地平铺/分组切换、分组卡片、分组设置入口与页面级组卡虚拟化。
- [x] M4: 补齐 Storybook 场景、Vitest/Rust 回归与视觉证据。
- [ ] M5: 快车道收敛到 merge-ready。

## Visual Evidence

- source_type: storybook_canvas
  target_program: mock-only
  capture_scope: browser-viewport
  requested_viewport: 1600x1400
  viewport_strategy: devtools-emulate
  sensitive_exclusion: N/A
  submission_gate: pending-owner-approval
  story_id_or_title: Account Pool/Pages/Upstream Accounts/List — Grouped View
  state: grouped roster page-level scroll integration view
  evidence_note: 验证账号页标题区的 `平铺 / 分组` 切换、分组摘要区新增设置按钮、整页滚动承载分组列表、单层分组主卡片与紧凑摘要栏，以及组成员列表不再出现内部滚动容器。

![账号页分组视图（页面级滚动）](./assets/grouped-view-page-level-scroll.png)

- source_type: storybook_canvas
  target_program: mock-only
  capture_scope: browser-viewport
  requested_viewport: 1600x1400
  viewport_strategy: devtools-emulate
  sensitive_exclusion: N/A
  submission_gate: pending-owner-approval
  story_id_or_title: Account Pool/Components/UpstreamAccountsGroupedRoster — Virtualized Large Roster
  state: grouped roster window virtualization stress case
  evidence_note: 验证大数据量场景改为页面级组卡虚拟化后，首屏仅挂载可见组卡片，列表滚动仍保持稳定且不会出现组卡重叠。

![分组视图大数据页面级虚拟列表示意](./assets/virtualized-large-roster-page-level.png)

- source_type: storybook_canvas
  target_program: mock-only
  capture_scope: browser-viewport
  requested_viewport: 1600x1400
  viewport_strategy: devtools-emulate
  sensitive_exclusion: N/A
  submission_gate: pending-owner-approval
  story_id_or_title: Account Pool/Pages/Upstream Accounts/List — Grid View
  state: grouped roster compact grid layout without overlap
  evidence_note: 验证网格视图在当前桌面视口下优先呈现 3 列，左侧分组信息保持紧凑，右侧成员区随内容自然收缩且不再出现内容重叠或内部滚动。

![账号页网格视图（无重叠）](./assets/grid-view-page-level-scroll.png)

- source_type: storybook_canvas
  target_program: mock-only
  capture_scope: element
  requested_viewport: 1600x1400
  viewport_strategy: storybook-viewport
  sensitive_exclusion: N/A
  submission_gate: pending-owner-approval
  story_id_or_title: Account Pool/Pages/Upstream Accounts/List — Dynamic · Flat
  state: flat roster live refresh with 46 accounts
  evidence_note: 验证动态平铺 Story 使用 46 条账号 mock 数据，播放刷新后表格内容会实时重排并出现新的分组文案，用于观察平铺布局在 live refresh 下不会异常抖动或错位。

![账号页平铺视图（动态刷新）](./assets/dynamic-layout-flat-live-refresh.png)

- source_type: storybook_canvas
  target_program: mock-only
  capture_scope: element
  requested_viewport: 1600x1400
  viewport_strategy: storybook-viewport
  sensitive_exclusion: N/A
  submission_gate: pending-owner-approval
  story_id_or_title: Account Pool/Pages/Upstream Accounts/List — Dynamic · Grouped
  state: grouped roster live refresh after re-measure
  evidence_note: 验证动态分组 Story 在 46 条账号数据、分组成员数与代理绑定实时变化后，页面级虚拟化会重新测量组卡高度，后续组卡继续顺序排布且不发生内容重叠。

![账号页分组视图（动态刷新无重叠）](./assets/dynamic-layout-grouped-live-refresh.png)

- source_type: storybook_canvas
  target_program: mock-only
  capture_scope: element
  requested_viewport: 1600x1400
  viewport_strategy: storybook-viewport
  sensitive_exclusion: N/A
  submission_gate: pending-owner-approval
  story_id_or_title: Account Pool/Pages/Upstream Accounts/List — Dynamic · Grid
  state: grouped grid live refresh after reflow
  evidence_note: 验证动态网格 Story 在 46 条账号数据持续刷新时，左侧分组摘要与右侧成员网格会一起重排，组卡之间保持文档流顺序且不出现重叠。

![账号页网格视图（动态刷新无重叠）](./assets/dynamic-layout-grid-live-refresh.png)

## 风险 / 假设

- 假设：共享 bound-group 的“当前代理”以当前 group runtime `current_binding_key` 作为真相源；若组从未发生 live selection，则可退化为 `unconfigured`，而不是伪造一个默认代理。
- 假设：分组模式下的全量查询只用于当前筛选结果，不扩展为跨筛选的全局一次性加载。
- 风险：页面级组卡虚拟化仍依赖测量稳定性；若窗口尺寸频繁变化，可能出现短暂的高度回流或滚动锚点漂移，需要依赖真实测量与重新计算收敛。
- 风险：若 bulk selection 与 detail route 直接绑定平铺表格 DOM 结构，分组模式可能需要补额外桥接层保证行为一致。
