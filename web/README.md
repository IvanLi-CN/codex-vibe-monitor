# Codex Vibe Monitor Web UI

`web/` 承载 React + Vite 前端，以及用于页面状态与组件复核的 Storybook。

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

### Storybook development

```bash
bun run storybook
```

默认打开 `http://127.0.0.1:60082`。如需覆盖端口，可设置 `STORYBOOK_PORT`。脚本会拒绝 `6006`，避免和其他 worktree 的默认 Storybook 端口混用。

### Storybook static build

```bash
bun run storybook:build
```

静态产物会写入 `web/storybook-static/`。GitHub Pages 发布时，该目录会被装配到 public docs 站点的 `/storybook/` 子路径下；主入口通过 `/storybook.html` 跳转，导览页固定在 `/storybook-guide.html`。

## Storybook 范围

当前 Storybook 重点覆盖：

- `Shell/*`：应用布局与壳层
- `Dashboard/*`：首页 KPI 与摘要卡片
- `Monitoring/*`：Invocation / Forward Proxy 相关页面状态
- `Records/*`：记录列表、筛选与摘要
- `Settings/*`：设置页表单与配置状态
- `Account Pool/*`：账号列表、创建页与标签页
- `UI/*`：基础输入组件与表单反馈

## docs-site 关系

public docs 站点位于仓库根目录的 `docs-site/`。如果本地同时启动：

- `cd docs-site && bun run dev`
- `cd web && bun run storybook`

那么 docs-site 的 `storybook.html` 会跳到当前本地 Storybook dev server；否则在装配后的静态站点里会跳到 `/storybook/index.html`。
