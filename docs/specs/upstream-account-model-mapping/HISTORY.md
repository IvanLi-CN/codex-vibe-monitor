# 上游账号模型映射 演进历史

> 这里记录影响长期理解的关键演进；规范正文仍以 `./SPEC.md` 为准。

## Decision Trace

- 账号本地映射独立于现有 root -> group -> account -> conversation 策略继承，以避免映射目标在跨账号故障转移中泄漏。
- 可用模型资格并集与映射缓存分离：映射可扩大单账号对原模型的候选资格，但不会把目标或模式写入全局可用模型目录。
- 路由、健康和审计的稳定键保持原始请求模型；上游目标模型作为尝试级事实记录。

## Key Reasons / Replacements

- 新主题；没有替代既有 Spec。

## References

- `./SPEC.md`
- `./IMPLEMENTATION.md`
