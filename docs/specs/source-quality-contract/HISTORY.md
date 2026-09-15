# 全源码结构质量合同主题历史

> 本文只记录主题生命周期、兼容性和必要背景；单次任务流水账与执行状态不放在这里。

## Lifecycle / Compatibility

- 该主题以 canonical slug `source-quality-contract` 新建，生命周期为 `active`。
- 过渡期允许一次性 ratchet baseline；最终状态切换为 `zero`，不保留兼容性 waiver。

## Replacements / Background

- 既有 backend structure specs 只覆盖部分模块拆分，不能作为全仓源码质量合同；本主题独立拥有跨 Rust、Web、Docs、测试和工具的结构边界。
- 参考项目的 Rust source quality 做法被采用为 Rust 阈值、入口预算和禁止结构性 suppression 的标准；Web/Docs 额外采用 Biome 与多语言 AST companion checker。

## Related Changes

- None. Record future PR, commit, review, and compatibility references here without copying execution state.

## References

- `./SPEC.md`
- `./IMPLEMENTATION.md`
