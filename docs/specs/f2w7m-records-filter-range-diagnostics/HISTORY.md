# 请求记录筛选范围控件与诊断维度增强演进历史（#f2w7m）

> 这里记录影响长期理解的关键演进原因；规范正文仍以 `./SPEC.md` 为准。

## Decision Trace

- 2026-07-18：创建 active topic spec，明确 records 筛选增强继续保留抽屉承载，但把范围控件与诊断维度收口为长期共享能力。
- 2026-07-18：实现单字段时间/数值范围控件，重排 Records 抽屉 IA，并把上游账号 label/value suggestions 与新增诊断维度一并贯通到前后端契约、测试与 Storybook 证据面。
- 2026-07-19：将数值范围控件调整为双端 slider 主交互，并让 slider 数值域跟随 records summary 的真实最大值，而不是仅靠静态输入框。
- 2026-07-19：将 Records owner-facing ID 筛选切换为短 `调用 ID / 尝试 ID`，退役筛选抽屉中的 `请求 ID / Sticky Key`，并让 locate 链路与 records 详情默认不再暴露非短 ID。
- 2026-07-19：根据 UI audit 补齐范围控件的可访问性错误语义、轨道直调交互与当前区间摘要，并移除抽屉标题下的冗余说明文案。
- 2026-07-19：为 `NumericRangeField` 增加嵌入态 surface，并让 Records 范围分组改用无内层卡片的嵌入式布局，消除嵌套卡片感。
- 2026-07-20：修正 Records 模型筛选器内层标签选择器的 overlay host 继承，避免嵌套 popup 被 portal 到 `body` 后落到抽屉遮罩下方，并补充新的 mock-only 视觉证据。

## Key Reasons / Replacements

- archived records specs 已分别解决 stable snapshot、请求 ID、异常响应体与抽屉遮挡等问题；本 spec 继续承接“筛选 IA 与能力面”的长期合同，而不是重开一次性任务文档。
- `upstreamScope` 继续作为 owner-facing 的高层路由范围语义，避免与底层 `routeMode` 一起暴露造成认知重复。

## References

- `./SPEC.md`
- `./IMPLEMENTATION.md`
