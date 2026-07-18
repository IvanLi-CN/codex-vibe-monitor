# Installable PWA 运行时与 Dashboard 概览离线快照演进历史（#m9p2w）

> 本文只记录影响长期维护的决策原因；行为规范以 `./SPEC.md` 为准。

## Decision Trace

- `Codex Vibe Monitor` 先从 `metadata-only` 提升到 `installable-runtime`，再进一步把 `Dashboard overview snapshots readable` 纳入正式合同；这一步解决的是“离线时只有壳层、没有最近概览可读”的 owner-facing 缺口。
- 产品定义仍刻意停在“installable-runtime + overview snapshots readable”，而不是完整 `offline-capable` 数据应用。原因是 working conversations、详情抽屉、写操作与 SSE 实时性都仍依赖在线真相，把它们误写成离线可用会制造错误承诺。
- Dashboard 历史概览采用应用层 IndexedDB 快照，而不是把 `/api/*` 数据缓存塞进 service worker。这样可以继续让 SW 专注在壳层与静态资源，同时把数据快照的范围、版本和替换策略收束在前端模块里。
- “多范围历史”被固定定义为五个现有 range 各保留一份最新成功快照，不做多版本历史回放。这样离线语义足够清晰，也避免把 snapshot store 演化成难以维护的本地时间旅行仓库。
- Safari / iOS 继续只提供 manual Add to Home Screen guidance；更新与离线快照扩容并没有改变这一浏览器边界。
- install prompt 从头栏常驻 button 改为自动弹出的一次性 modal / guidance。原因是 install 是首次交付动作，不应长期占用 owner-facing 头栏空间，也不该让“已安装 / 未安装”状态伪装成普通工具按钮。
- 自动安装提示在窄屏上改为居中 modal，而不是继续复用移动端贴底 dialog。原因是这块语义属于“状态说明 + 操作确认”，owner-facing 预期是弹窗，不是会与页面滚动/抽屉语义混淆的 bottom sheet。

## References

- `./SPEC.md`
- `web/src/pwa/sw.ts`
- `web/src/hooks/usePwaRuntime.ts`
- `web/src/hooks/useDashboardOverviewSnapshotRuntime.ts`
- `web/src/features/dashboard/dashboardOverviewSnapshots.ts`
- `web/src/features/app-shell/PwaInstallControl.tsx`
- `web/src/components/ui/dialog.tsx`
