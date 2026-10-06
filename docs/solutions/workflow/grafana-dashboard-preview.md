---
title: Reproducible Grafana dashboard preview
module: observability-tooling
problem_type: reproducible-grafana-dashboard-preview
component: Grafana and Prometheus preview stack
tags:
  - grafana
  - prometheus
  - observability
  - preview
  - synthetic-data
status: active
related_specs: []
---

# Reproducible Grafana dashboard preview

## Context

Grafana dashboard layout 和 PromQL 调整需要真实的 Grafana 页面与 Prometheus 查询结果；只看 JSON 或用静态截图无法发现空面板、变量不匹配、响应式布局和时序数据问题。同时，正式 observability Compose 依赖真实应用、认证 token、私有网络和运维配置，不应为了本地预览而复用。

## Symptoms

- 正式 dashboard 在本地打开后没有数据，无法判断面板布局是否正确。
- 临时手写容器命令容易丢失端口、provisioning、数据源或 dashboard 挂载关系。
- 合成 Prometheus 文本若在同一抓取中重复输出相同的 metric name 和 labelset，Prometheus 会丢弃样本并记录 `different value but same timestamp`。
- 临时 fixture、未固定镜像或复用生产卷会让后续预览不可复现，甚至混入真实数据。

## Root cause

生产部署合同和本地视觉预览有不同的安全边界：生产栈需要真实应用和密钥，预览栈需要可丢弃、无密钥、可重复的指标来源。两者若共用 Compose、网络、卷或 configuration，预览命令就会同时承担不必要的认证和数据隔离风险。

## Resolution

将预览实现放在 `ops/observability/preview/`，并直接挂载正常的 `ops/observability/grafana/dashboards` 和 provisioning 文件。预览 Compose 固定 Grafana、Prometheus 和 Python fixture 镜像 digest，使用独立网络和 `tmpfs` 状态；Prometheus target 的环境标签固定为 `synthetic-preview`，fixture 额外输出带 `fixture_version` 的 `cvm_preview_fixture_info` 标识。

`metrics_server.py` 在 9091 和 6772 提供覆盖五个 dashboard 查询族的动态合成 series。每个抓取端口的完整 metric name + labelset 必须唯一；路由状态、设备等会造成多个样本的维度必须进入 labelset，不能只靠同名 metric 区分。fixture 不包含请求 payload、账号、模型、token、数据库或生产 endpoint。

用 `scripts/cvm-grafana-preview` 统一生命周期：

```bash
scripts/cvm-grafana-preview up
scripts/cvm-grafana-preview status
scripts/cvm-grafana-preview url
scripts/cvm-grafana-preview logs grafana
scripts/cvm-grafana-preview down
```

该命令通过 Agent VM 同步当前工作树、创建精确的端口租约，并以 `--force-recreate` 启动 preview project，因此 dashboard JSON 或 PromQL 修改后再次执行 `up` 就会加载最新内容。端口占用时使用 `scripts/cvm-grafana-preview up --port 3102`；修改端口前先执行 `down`。

## Guardrails / Reuse notes

- 预览只能连接 `synthetic-preview` fixture；不要把生产 Prometheus URL、真实应用网络、token 文件或持久化生产卷写入 preview Compose。
- dashboard JSON 只有一份，以正式 provisioning 文件为 source of truth；不要为预览复制一套容易漂移的 dashboard。
- 新增 fixture metric 时，先做同一抓取响应的完整序列唯一性检查，并保持标签白名单和有限基数；禁止加入 account、model、user、IP、request ID 或原始 URL。
- 预览的 `tmpfs` 数据是 disposable state，`down` 只清理 `cvm-grafana-preview` project、网络和对应端口映射。
- 预览页面必须使用真实 Grafana 渲染来检查效果；合成数据要在回复、截图和文档中明确标注，不能拿静态假图冒充生产观测结果。
- 这套 preview 证明 dashboard 的查询和呈现可用，不是生产数据验收、性能预算或告警阈值证书。

## References

- `ops/observability/preview/README.md`
- `ops/observability/preview/compose.yml`
- `ops/observability/preview/prometheus.yml`
- `ops/observability/preview/metrics_server.py`
- `ops/observability/README.md`
- `scripts/cvm-grafana-preview`
- `scripts/test-observability-tools.py`
- `docs/adr/0025-external-performance-observability.md`
