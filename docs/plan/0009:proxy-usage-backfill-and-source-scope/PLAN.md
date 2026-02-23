# Proxy usage 解析补全与默认来源口径回归

## Goal

- 修复代理链路在 `responses` 场景下 token 解析缺失（`input/output/cache/total_tokens` 大量为空）。
- 恢复默认统计口径为“合并全部来源（`xy + proxy`）”，确保历史数据持续可见。
- 提供启动期一次性回填能力，补齐已落库但 token 为空的历史 `proxy` 记录。

## In / Out

### In

- 为 proxy usage 解析增加 gzip 解码能力（优先 `Content-Encoding`，兜底 gzip magic bytes）。
- 启动期执行历史 `proxy` 空 token 回填（幂等）。
- 默认来源口径从“有 proxy 则仅 proxy”改为“全部来源”。
- 增加回归测试覆盖 gzip 解析、来源口径、回填幂等。

### Out

- 不修改 `/api/*` 路径与响应字段结构。
- 不做跨来源去重合并策略（可能保留少量重复计数）。
- 不进行历史数据重写或删除。

## Acceptance Criteria

1. Given `responses` 流式返回且响应体为 gzip，When 代理完成采集，Then `input_tokens/output_tokens/cache_input_tokens/total_tokens` 可正确写入。
2. Given 同时存在 `xy` 与 `proxy` 历史记录，When 请求 `/api/stats`，Then 默认统计包含两类来源，不再出现历史数据“消失”。
3. Given 历史 `proxy` 成功记录 `total_tokens IS NULL` 且有 `response_raw_path`，When 服务启动回填开启，Then 可补齐 token 字段。
4. Given 已回填记录，When 再次执行回填，Then 不会重复污染数据（幂等）。
5. Given gzip 解码失败，When 解析流程继续，Then 服务不崩溃且保留可诊断缺失原因。

## Testing

- `cargo fmt`
- `cargo test`
- `cargo check`
- 线上最小验证（部署后）：
  - `GET /api/stats`：`totalTokens > 0` 且总量不再只等于 proxy 小样本。
  - `GET /api/invocations?limit=20`：最新 `proxy` 记录 token 字段出现非空值。

## Risks

- 合并来源可能引入跨来源重复计数（当同一请求被多源记录）。
- 启动期回填在大数据库上可能延长启动时间。
- 响应编码异常可能导致个别记录仍无法解析 usage。

## Milestones

- [x] M1 gzip usage 解析修复与缺失原因可观测
- [x] M2 默认来源口径回归到 All
- [x] M3 启动期历史回填（含配置开关）
- [x] M4 回归测试与线上验证
