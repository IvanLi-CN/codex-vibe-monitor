# Installable PWA 运行时与应用壳层交付演进历史（#m9p2w）

> 本文只记录影响长期维护的决策原因；行为规范以 `./SPEC.md` 为准。

## Decision Trace

- `Codex Vibe Monitor` 从 `metadata-only` 提升到 `installable-runtime`，但刻意停在“离线壳层 + 明确降级提示”边界，不把真实数据可用性误说成 offline-capable。
- 仍保留 `HashRouter`，通过 `start_url=./#/dashboard` 和 shortcuts 绑定安装后入口，而不是为 PWA 单独改写路由体系。
- service worker 采用 prompt-style update；waiting worker 只在用户确认后接管，避免正在排障的会话被 mid-session 刷新打断。
- Safari / iOS 路径只提供 manual Add to Home Screen guidance，不伪装 native install prompt，也不承诺所有浏览器拥有一致安装手感。

## References

- `./SPEC.md`
- `web/src/pwa/sw.ts`
- `web/src/hooks/usePwaRuntime.ts`
