# 前端运行时图标内置打包（#k7kpk）

## 状态

- Status: 部分完成（3/4）
- Created: 2026-03-14
- Last: 2026-03-14

## 背景 / 问题陈述

- 当前前端在 17 个运行时代码文件里通过 `@iconify/react` 直接传入 `mdi:*` 字符串图标名。
- Iconify 官方 React 组件在这种字符串用法下会在运行时向 Iconify API 拉取图标数据，导致页面依赖第三方图标源。
- 主人要求图标必须随构建产物内置，浏览器运行时不得再从第三方获取图标数据。

## 目标 / 非目标

### Goals

- 所有前端运行时图标均从仓库内构建产物加载，不再依赖第三方图标请求。
- 保留现有图标语义、尺寸、旋转动画、`aria-hidden`、`data-testid` 与交互行为。
- 增加统一的本地图标入口，避免后续代码重新引入 `mdi:*` 运行时字符串路径。
- 提供自动化验证与浏览器网络证据，证明关键页面不再请求第三方图标源。

### Non-goals

- 不重做 favicon、manifest、apple-touch-icon 等已在仓库内的静态资源。
- 不调整页面视觉设计、图标含义、布局或文案。
- 不引入新的图标系统、手写 SVG 库或后端接口改动。

## 范围（Scope）

### In scope

- `web/src/components/AppIcon.tsx`：新增本地图标注册表与受控 `AppIconName`。
- `web/src/components/**`、`web/src/pages/**` 中所有运行时 `@iconify/react` 用法替换为 `AppIcon`。
- `web/package.json`：补充构建期本地图标依赖。
- 前端测试、构建与浏览器网络验证；当前 spec 与 `docs/specs/README.md` 状态同步。

### Out of scope

- Storybook 视觉主题改版或页面级 UI 调整。
- 非运行时文档、spec、测试文本中对 `mdi:*` 名称的历史记录清理。
- 其它第三方静态资源治理。

## 需求（Requirements）

### MUST

- 运行时代码不得再直接向 `Icon` 组件传入 `mdi:*` 字符串。
- 本地图标层必须只暴露受控名称集合，动态图标也只能从该集合中选择。
- `Dashboard`、`Records`、`Settings`、`Account Pool` 页面加载时不得出现 Iconify API 或其它第三方图标请求。
- `cd web && bun run test` 与 `cd web && bun run build` 必须通过。

### SHOULD

- 图标替换保持最小改动，不顺手重构业务逻辑。
- 增加防回归检查，阻止运行时代码重新引入 `mdi:*` 或直接 `@iconify/react` 用法。

## 功能与行为规格（Functional/Behavior Spec）

- 新增统一 `AppIcon` 组件，由本地 registry 把受控图标名映射到随包导入的 icon data。
- 运行时代码统一改用 `<AppIcon name="..." />`；条件分支、列表配置和 helper 返回值改为 `AppIconName`。
- `AppIcon` 继续复用 `@iconify/react` 作为渲染壳，但图标数据全部来自本地导入，不触发运行时远程获取。
- 对 loading / chevron / crown 等动态场景，仅允许在本地 union name 范围内切换。

## 验收标准（Acceptance Criteria）

- Given 打开任一包含图标的主页面，When 浏览器记录网络请求，Then 不存在对 Iconify API / CDN 等第三方图标源的请求。
- Given 查看运行时代码，When 搜索 `mdi:` 与 `@iconify/react`，Then 仅允许保留在本地图标封装层或测试/文档中，不允许出现在其它运行时代码文件。
- Given 现有按钮、badge、折叠箭头、loading 态和提示图标，When 完成替换，Then 交互与视觉语义保持不变。
- Given 执行前端测试与生产构建，When 命令结束，Then 全部通过。

## 非功能性验收 / 质量门槛（Quality Gates）

- Unit tests: `cd web && bun run test`
- Build: `cd web && bun run build`
- Browser verification: 打开生产预览并检查关键页面网络面板，确认无第三方图标请求
- Review: 运行 `$codex-review-loop` 收敛实现范围内问题

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 新增本地图标 registry / `AppIcon`，覆盖当前所需的运行时图标集合。
- [x] M2: 替换全部运行时代码中的 `mdi:*` 图标调用，并收敛动态图标类型。
- [x] M3: 补充防回归测试与构建验证，确认关键页面无第三方图标请求。
- [ ] M4: 完成快车道提交、PR、checks 与 review-loop 收敛。

## 风险 / 假设

- 假设：构建期从 npm 安装图标数据包是允许的，限制仅针对浏览器运行时第三方请求。
- 风险：若直接整包注册 icon set，bundle 体积会明显膨胀；因此优先采用按图标导入。
- 风险：动态图标位若继续使用宽泛 `string`，会绕过本地 registry 约束，因此需要同步收窄类型。

## 变更记录

- 2026-03-14: 创建 spec，冻结“运行时图标必须内置打包、禁止第三方拉取”的范围与验收口径。
- 2026-03-14: 新增 `AppIcon` 本地图标注册层，按图标导入 `@iconify-icons/mdi` 并替换全部运行时代码中的 `mdi:*` 字符串调用。
- 2026-03-14: 完成 `cd web && bun run lint`、`cd web && bun run test`、`cd web && bun run build`，并通过 Playwright 预览页检查 `#/dashboard`、`#/records`、`#/settings`、`#/account-pool` 无第三方图标请求。
