# API Key 优先级迁移准入控制 主题历史

> 这里记录主题局部生命周期、替换、兼容性与必要背景；完整 ADR 取舍保留在 `docs/adr/`。单次任务流水账不放这里，规范正文仍以 `./SPEC.md` 为准。

## Lifecycle / Compatibility

- 本主题将既有 `Fallback` sticky 的主动优先级比较收敛为按目标 API Key 账号模型串行验证的新增行为。
- HTTP/API Key 范围内新增本地优先级迁移状态，不改变 WebSocket、OAuth、人工绑定或普通故障切换的兼容性合同。
- 全局开关关闭时保留升级前的路由行为；重新开启不恢复旧许可或恢复计数。

## Replacements / Background

- 既有“成功才改写 sticky”不能限制同一恢复目标同时接收多少迁移尝试；本主题以无队列的本地许可和真实请求验证补足该缺口。
- 模型路由健康继续承担持久化的兼容性健康语义；优先级迁移许可不以数据库为运行时权威。

## References

- `./SPEC.md`
- `./IMPLEMENTATION.md`
- `../../adr/0002-stage-automatic-priority-handoffs.md`
