# 移动优先导航与浮层收口演进历史（#hnu7b）

> 本文只记录影响长期维护的决策原因；行为规范以 `./SPEC.md` 为准。

## Decision Trace

- 紧凑布局边界统一为 `1024px`，覆盖手机和纵向平板，并避免 769px 至 1023px 之间仍出现桌面级横向导航或居中浮层。
- 重型详情只页面化上游账号和 Prompt Cache 会话；其他详情保留 overlay，维持桌面工作流与已有调用入口。
- 全页证据选择已合入的 mock-only Web Demo，避免把 Storybook 画布背景误当成产品页面背景。

## References

- `./SPEC.md`
- `docs/specs/ykhfu-web-demo/SPEC.md`
