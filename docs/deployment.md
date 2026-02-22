# Deployment Guide

本文档说明 `codex-vibe-monitor` 在生产环境的推荐部署方式，以及 `/api/settings/proxy-models` 写接口的安全边界。

## Recommended Topology

- 对外只暴露网关（例如 Traefik）。
- 应用服务只在内网监听（容器网络 / 私网），不要直接暴露到公网。
- 浏览器仅访问网关域名（例如 `https://app.example.com`）。

示意：

```text
Browser -> Traefik (public 80/443) -> codex-vibe-monitor (private :8080)
```

## Security Boundary For Settings Writes

`PUT /api/settings/proxy-models` 会修改全局代理行为，属于状态变更接口。

服务端会执行来源校验：

- 优先使用 `Origin` 对比请求主机信息。
- 在网关场景下，可使用网关写入的 `X-Forwarded-Host` / `X-Forwarded-Proto` / `X-Forwarded-Port` 参与同源判断。
- 对明显跨站请求（`Sec-Fetch-Site: cross-site`）拒绝写入。

这意味着以下前提必须成立：

1. 应用服务不能被外部客户端直连。
2. `X-Forwarded-*` 只应由受信任网关产生。
3. 网关与应用之间的链路是受控内网链路。

如果外部可以绕过网关直连应用端口，攻击者可伪造请求头，安全前提失效。

## Reverse Proxy Requirements

以 Traefik 为例，建议满足：

1. 仅 Traefik 暴露公网端口。
2. `codex-vibe-monitor` 不做 `ports` 直接映射到公网。
3. Traefik 路由按固定 Host 转发到应用服务。
4. 不允许旁路访问应用容器（安全组、防火墙、网络策略）。

## Proxy Capture Runtime

建议在部署清单中显式配置以下变量（未配置时使用服务默认值）：

- `PROXY_RAW_MAX_BYTES`：单次请求/响应原文采集上限；默认 `0=unlimited`（支持显式配置正整数上限）。
- `PROXY_RAW_RETENTION_DAYS`：原文留存天数（到期清理原文字段/文件，不影响结构化统计）。
- `PROXY_ENFORCE_STREAM_INCLUDE_USAGE`：是否在 `chat.completions` 流式请求中强制注入 `stream_options.include_usage=true`。
- `PROXY_PRICING_CATALOG_PATH`：本地价目表文件路径（用于成本估算）。
- `XY_LEGACY_POLL_ENABLED`：legacy 轮询写入开关（默认关闭；开启后会并行写入旧来源统计）。

统计接口行为：

- `GET /api/stats`、`/api/stats/summary`、`/api/stats/timeseries` 默认以代理采集（`source=proxy`）为主。
- `GET /api/stats/perf` 返回代理链路阶段耗时聚合统计。

## Header Relay Policy

应用在转发 `/v1/*` 到上游时，不会透传以下代理身份相关头：

- `Forwarded`
- `Via`
- `X-Real-IP`
- `X-Forwarded-For`
- `X-Forwarded-Host`
- `X-Forwarded-Proto`
- `X-Forwarded-Port`
- `X-Forwarded-Client-Cert`

因此：

- 上游不会通过这些头识别当前服务的代理层信息。
- 下游（浏览器）也不会从应用响应里看到这些头被回传。

## Verification Checklist

部署后至少检查：

1. 外部无法直接访问应用监听端口（例如 `:8080`）。
2. 通过网关域名访问应用页面和 API 正常。
3. 正常同源写入（页面设置保存）返回成功。
4. 非同源 `Origin` 请求写入返回 `403`。
