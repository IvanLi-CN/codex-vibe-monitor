# Codex Vibe Monitor Web UI

`web/` 承载 React + Vite 前端，以及用于页面状态与组件复核的 Storybook。

当前前端同时承担 installable PWA runtime：manifest、service worker、prompt-style update、离线壳层与 Safari / iOS 手动安装指引都属于正式交付面。

## 命令

### App development

```bash
bun install
bun run dev -- --host 127.0.0.1 --port 60080
```

打开 `http://127.0.0.1:60080`。开发服务器默认代理到 `http://127.0.0.1:8080`，也可通过 `VITE_BACKEND_PROXY` 覆盖。

### App preview build

```bash
bun run build
bun run preview
```

### Installable PWA 验证

- 正式浏览器合同：
  - Desktop Chromium：原生 install prompt + 独立窗口
  - Android Chrome：原生安装入口
  - Safari / iOS：手动 `Add to Home Screen` 指引
- 离线边界：首次在线访问后，应用壳层与静态资源可离线打开；实时数据、SSE 与设置同步会明确降级。
- 更新策略：waiting service worker 只在用户确认时刷新，不做 mid-session auto takeover。

PWA 专项回归：

```bash
bun run test:e2e:pwa
```

### Mock-only Web Demo

```bash
bun run demo:dev -- --host 127.0.0.1 --port <leased-port>
bun run demo:build
```

`demo` runtime 在 React 渲染前启动 MSW，复用正式 HashRouter 的全部路由。它只使用确定性虚构数据与浏览器内存；OAuth、API Key 和其他敏感输入不会保存、回显或发送到后端。静态发布时设置 `VITE_DEPLOY_BASE=/demo/`（或 Pages 的 repo 子路径）以定位 assets 与 worker。

### Storybook development

```bash
bun run storybook
```

默认打开 `http://127.0.0.1:60082`。如需覆盖端口，可设置 `STORYBOOK_PORT`。脚本会拒绝 `6006`，避免和其他 worktree 的默认 Storybook 端口混用。

### Storybook static build

```bash
bun run storybook:build
```

静态产物会写入 `web/storybook-static/`。GitHub Pages 发布时，该目录会被装配到 public docs 站点的 `/storybook/` 子路径下；公共入口统一通过 `/storybook.html` 跳转。

## Storybook 范围

当前 Storybook 重点覆盖：

- `Shell/*`：应用布局与壳层
- `Dashboard/*`：首页 KPI 与摘要卡片
- `Monitoring/*`：Invocation / Forward Proxy 相关页面状态
- `Records/*`：记录列表、筛选与摘要
- `Settings/*`：设置页表单与配置状态
- `Account Pool/*`：账号列表、详情抽屉、创建页与系统标签只读筛选
- `UI/*`：基础输入组件与表单反馈

## docs-site 关系

public docs 站点位于仓库根目录的 `docs-site/`。如果本地同时启动：

- `cd docs-site && bun run dev`
- `cd web && bun run storybook`

那么 docs-site 的 `storybook.html` 会跳到当前本地 Storybook dev server；否则在装配后的静态站点里会跳到 `/storybook/index.html`。GitHub Pages 组装还会把 `web/demo-dist/` 放到 `/demo/`，不替代 docs 根或 Storybook。
