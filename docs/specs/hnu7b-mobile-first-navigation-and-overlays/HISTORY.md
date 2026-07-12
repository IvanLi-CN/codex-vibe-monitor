# 移动优先导航与浮层收口演进历史（#hnu7b）

> 本文只记录影响长期维护的决策原因；行为规范以 `./SPEC.md` 为准。

## Decision Trace

- 紧凑布局边界统一为 `768px`，与主导航、dialog、drawer、表格和详情页面的呈现断点保持一致；从 `769px` 起不再混用桌面导航和移动端浮层。
- 移动端页面级 surface 改为扁平结构，避免页面 gutter、外层 panel padding 与内部数据卡叠加消耗内容宽度；仍保留有独立信息或操作边界的紧凑 card。
- 重型详情只页面化上游账号和 Prompt Cache 会话；其他详情保留 overlay，维持桌面工作流与已有调用入口。
- 全页证据选择已合入的 mock-only Web Demo，避免把 Storybook 画布背景误当成产品页面背景。

## References

- `./SPEC.md`
- `docs/specs/ykhfu-web-demo/SPEC.md`
