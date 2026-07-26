# Product

## Product Definition

Codex Vibe Monitor 是一套自部署的 OpenAI 兼容流量网关，以及围绕调用、路由、账号池和运行状态的一体化观测与运营工作台。

它统一承接 `/v1/*` HTTP 与 WebSocket 流量，按上游账号、模型能力、会话归属和代理策略完成路由，同时保留调用证据、实时状态、历史统计、成本、原始 payload、维护任务与归档数据。用户可以在同一个界面里判断服务是否正常、定位请求经过了什么路径，并直接调整运行配置。

## Users

产品首先服务于单实例 owner-operator、开发者和小型维护团队。典型用户已经在运行自己的 OpenAI 兼容代理，需要同时照看真实流量、上游账号健康、路由稳定性、延迟、失败原因、token 与费用、数据保留和运行配置。

使用场景以日常值守和排障为主：用户会长时间驻留观察，也会在故障发生时跨页面追查一次调用、一个会话或一组账号。界面必须支持高密度扫描，让偶发维护者不读源码也能完成常见诊断与配置操作。

## Core Jobs

1. **判断当前是否正常。** 查看当天活动、实时调用、并行会话、代理节点、上游账号和系统存储状态。
2. **解释问题发生在哪里。** 从汇总趋势下钻到调用详情、pool attempts、路由 payload、失败分类、原始请求与响应证据。
3. **把判断转化为操作。** 在当前上下文维护账号、分组、路由策略、模型能力、价格、forward proxy、留证、retention 与归档配置。
4. **保留可回看的运行历史。** 通过 SQLite、时间序列汇总、稳定查询快照、后台维护记录和不可变归档支撑长期分析。

## Product Surfaces

- **Dashboard**：自然日关键指标、活动趋势、网络速度、账号活动与正在工作的对话。
- **Stats**：按时间范围分析请求量、token、费用、成功与非成功结果、延迟和并行工作。
- **Live**：观察实时调用流、Prompt Cache 会话聚合以及 forward proxy 节点的短窗口表现。
- **Records**：通过稳定搜索快照筛选、排序和分页调用记录，并下钻到完整请求、响应与路由证据。
- **Account Pool**：管理 OAuth 与 API Key 上游账号、分组、标签、模型与路由策略、同步状态和维护记录。
- **System**：集中查看运行状态与存储占用、后台任务，并维护通用设置和 forward proxy 配置。

应用支持 light/dark 双主题、中文与英文界面、桌面与紧凑移动布局，以及可安装 PWA。Web Demo 和 Storybook 提供无需真实后端的产品路由与组件验收面。

## Product Principles

1. **Signal before ornament.** 先让状态、趋势、失败原因和下一步动作可见，再考虑视觉气质。
2. **Evidence over inference.** 汇总必须能下钻到调用、路由、账号或任务证据，避免只有结论没有解释。
3. **Dense but legible.** 密度来自分组、对齐和稳定节奏，不依赖更小字号、低对比或过度截断。
4. **Operate in place.** 筛选、查看、重连、同步、编辑和配置尽量保留在当前工作上下文。
5. **One operational vocabulary.** Dashboard、Live、Records、Account Pool 和 System 共享状态、指标、表单与反馈语义。
6. **Protect the hot path.** 后台统计、维护、retention 与归档不能以牺牲代理请求和 OAuth 等前台关键路径为代价。

## Brand Personality

产品人格是 `precise / observant / restrained`。视觉方向是“观测实验室”：像一张可信的实验台，而不是舞台。中文表达清晰、直接、专业，保留 `proxy`、`token`、`latency`、`SSE`、`routing` 等能减少歧义的工程术语。

界面可以使用实时信号、仪表感和受控的环境层次，但所有效果都必须服务于读数、定位或操作。不要做成通用 SaaS dashboard 模板，也不要用深蓝霓虹、glassmorphism、渐变文字、厚侧边彩条或无语义动效替代信息层级。

## Accessibility

界面以 WCAG AA 为默认目标，保留键盘可达、清晰的 `focus-visible`、可读的 `aria-label`、稳定的 heading/landmark 结构，以及 light/dark 两套主题下的文字与状态对比。

颜色只能辅助表达状态，不能成为唯一信息来源。图表、表格、徽标与告警需要通过文字、图例、tooltip、数值或形状承载关键判断。紧凑移动布局需要保留安全区、可读内容顺序和可操作的触控目标。

## Current Boundaries

- 产品是自部署的单实例工作台，不是托管型 SaaS。
- 当前不提供多租户、RBAC 或企业级权限控制面。
- 当前不把告警编排、通知升级和自动修复作为已交付能力。
- 产品不以替代上游控制台或大规模分布式可观测平台为目标。
