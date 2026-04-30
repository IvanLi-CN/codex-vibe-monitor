# Product

## Register

product

## Users

Codex Vibe Monitor 面向自部署 OpenAI 兼容代理的维护者、开发者和小型运维负责人。用户通常同时关心实时调用、失败原因、上游账号健康、费用与 token 消耗、归档保留、运行配置是否稳定。

典型使用场景不是营销浏览，而是在已经有真实流量的服务旁边打开工作台，快速回答三个问题：现在是否正常，问题在哪里，下一步应该调哪个配置。界面需要支持高密度扫描、跨页面排障、长时间驻留观察，也要让偶发维护者能在不读源码的情况下完成基础操作。

## Product Purpose

Codex Vibe Monitor 是一套自部署的 OpenAI 兼容代理观测工作台。它把 `/v1/*` 流量接入、调用留证、实时 SSE、历史统计、请求排障、上游账号池、forward proxy 配置、价格目录维护、SQLite 持久化与归档放在同一个产品里。

产品成功不是做出一个漂亮总览，而是让用户能看得到、查得到、调得动。Dashboard 和 Live 负责运行态判断，Records 和 Stats 负责历史分析，Account Pool 和 Settings 负责把判断转化为配置动作。

## Brand Personality

默认人格是 `precise / observant / restrained`。中文语气应当清晰、直接、专业，允许保留必要英文术语，例如 `proxy`、`token`、`latency`、`SSE`、`routing`，避免为了翻译而降低工程含义。

视觉方向是“观测实验室”：像一张可信的实验台，而不是舞台。它可以有实时信号、仪表感和轻微的技术气质，但所有视觉效果都必须服务于读数、定位和操作。

## Anti-references

- 不要做成通用 SaaS dashboard 模板：大号 hero metric、四张同款卡片、渐变强调和空泛文案会削弱可信度。
- 不要落入“观测工具等于深蓝黑底霓虹”的类别反射。暗色主题可以存在，但不能用黑蓝荧光感替代信息层级。
- 不要把 glassmorphism 当默认语言。模糊、半透明和光晕只能用于少量浮层或运行态反馈，不能成为所有容器的装饰。
- 不要使用 gradient text、厚侧边彩条、无意义的 orb 背景、弹跳动效或为了显得高级而发明的控件。
- 不要牺牲可读性换取酷感。小字号、低对比、过度截断、移动端只能横向拖动，都应被视为技术债。

## Design Principles

1. **Signal before ornament.** 先让状态、趋势、失败原因和下一步动作可见，再考虑视觉气质。
2. **Dense but legible.** 产品允许高密度信息，但密度必须来自分组、对齐和稳定节奏，而不是缩小触控目标或压低对比。
3. **One vocabulary, many surfaces.** Dashboard、Live、Records、Account Pool 和 Settings 应共享同一套按钮、表单、surface、状态色和图表语义。
4. **Evidence over mood.** 每个图表、徽标、警告、空态和加载态都应能解释系统状态，不用装饰性动效制造“实时感”。
5. **Operate in place.** 用户应尽量在当前上下文完成筛选、查看、重连、同步、编辑和配置，不把简单任务推给无必要的 modal。

## Accessibility & Inclusion

目标默认按 WCAG AA。界面要保留键盘可达、清晰 focus-visible、可读 `aria-label`、稳定 heading/landmark 结构，以及 light/dark 两套主题下的文字与状态对比。

数据密集区域应考虑色弱用户：颜色只能辅助表达，不能成为唯一状态来源。图表与表格要保留文字、图例、tooltip 或数值标签来承载关键判断。

动效应短、直接、可预测。加载和实时反馈可以使用 spinner、pulse 或 skeleton，但不能阻挡任务，也不能依赖长序列动画。后续若引入 reduced motion 策略，应优先覆盖全局 pulse、spin、dialog、popover 和图表 hover 反馈。
