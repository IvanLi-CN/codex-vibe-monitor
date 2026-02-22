# MITM 请求级计费与性能采集（Proxy as Source of Truth）

## 背景

当前统计主数据来自外部轮询源（quota/CRS），而内置 `/v1/*` 反向代理仅透传不采集。目标是将代理链路升级为唯一统计来源，直接在“中间人”路径采集请求、响应、token、成本估算与阶段耗时。

## 目标

- 以内置 `/v1/*` 代理链路作为统计主来源（一期覆盖 `chat.completions` 与 `responses`）。
- 记录请求级 token / 成本估算 / 失败原因 / 阶段耗时（用于性能统计）。
- 原文采集采用“DB 元数据 + 文件原文”，默认 7 天留存，尽量避免截断。

## 非目标

- 一期不实现 `embeddings/images/audio` 等端点的深度 usage 解析。
- 不接入外部账单 API 对账。
- 不引入多租户隔离。

## 范围

### In

- 新增代理采集与解析逻辑（请求侧、响应侧、流式 usage 处理）。
- 支持 `chat.completions` 流式自动注入 `stream_options.include_usage=true`（可配置）。
- 扩展存储结构以记录阶段耗时、原文索引、估算成本与价目版本。
- 新增性能统计接口（阶段耗时聚合）。
- 默认停用旧轮询写入路径（历史数据保留只读）。

### Out

- 旧历史数据重写迁移为新来源。
- 复杂限流/熔断/重试治理。
- 前端大规模改版。

## 需求列表

### MUST

- `PROXY_RAW_MAX_BYTES` 具备高上限并尽量避免截断；支持 `0=unlimited`。
- 请求记录必须包含多阶段耗时字段，至少覆盖：请求读取、请求解析、上游连接、上游首字节、上游流传输、响应解析、持久化、总耗时。
- `chat.completions` 与 `responses` 的 usage 解析可落库并可聚合。
- 成本按 usage + 本地价目表估算，并标记 `cost_estimated` 与 `price_version`。
- 默认统计查询以代理来源为主，不再依赖 quota/CRS 增量写入。

### SHOULD

- 流中断时保留记录并标记 usage 缺失原因。
- 原文留存到期后只清理原文文件/字段，不影响结构化统计。
- 提供兼容字段，避免现有前端页面崩溃。

### COULD

- 后续增加 `embeddings/images/audio` usage 解析器。
- 增加按 endpoint/model 的更细粒度性能看板。

## 验收标准

1. Given 调用 `POST /v1/chat/completions`（非流式），When 请求完成，Then 记录包含 usage、成本估算、阶段耗时。
2. Given 调用 `POST /v1/chat/completions`（流式），When 请求完成，Then 代理自动注入 `include_usage` 且尾块 usage 可被采集（中断场景除外并有标记）。
3. Given 调用 `POST /v1/responses`，When 请求完成，Then 记录包含 usage 与阶段耗时。
4. Given 请求体/响应体较大，When 落盘原文，Then 优先完整落文件并仅在硬阈值触发时标记截断原因。
5. Given 旧源环境变量未配置，When 服务启动并仅走代理流量，Then 核心统计接口仍可工作。
6. Given 查询性能统计接口，When 指定时间范围，Then 返回各阶段耗时统计（count/avg/P50/P90/P99/max）。

## 测试策略

- Rust 单元测试：usage 提取、流式 include_usage 注入、成本估算、阶段耗时聚合。
- Rust 集成测试：`chat`/`responses` 代理采集链路、流式中断与降级行为。
- 回归验证：`cargo test`、`cargo check`、前端最小类型检查（如触及 web）。

## 风险与缓解

- 风险：流式响应中断导致 usage 缺失。
  - 缓解：记录缺失原因并保证请求级记录可落库。
- 风险：原文高体积导致磁盘增长。
  - 缓解：TTL 清理 + 可配置上限 + 截断原因可观测。
- 风险：切换到代理来源后统计口径抖动。
  - 缓解：记录 `source` 并保留旧历史只读。

## 里程碑

- [ ] M1 代理采集框架与 schema 扩展（含阶段耗时字段）
- [ ] M2 chat/responses usage 解析 + 成本估算 + 原文落盘
- [ ] M3 聚合接口与性能统计接口
- [ ] M4 验证、文档与回归测试
