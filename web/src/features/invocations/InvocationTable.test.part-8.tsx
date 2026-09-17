/** @vitest-environment jsdom */

import { act } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { expect, it, vi } from "vitest";
import { I18nProvider } from "../../i18n";
import type { ApiInvocation } from "../../lib/api";
import {
  apiMocks,
  createInvocationRecord,
  createWorkflowDetailFixture,
  host,
  InvocationDetailProbe,
  renderInteractiveTable,
  renderTable,
  waitForCondition,
} from "./InvocationTable.test-support";

it("always shows the invocation ID without repeating conversation identity", async () => {
  const records = [createInvocationRecord(0)];
  await renderInteractiveTable(records);
  const invokeId = document.querySelector('[data-testid="invocation-id"]');
  expect(invokeId?.textContent).toBe("virtual-row-1");
  expect(invokeId?.className).toContain("select-text");
  expect(invokeId?.className).toContain("whitespace-nowrap");
  expect(invokeId?.className).toContain("overflow-hidden");
  expect(invokeId?.className).not.toContain("truncate");
  expect(invokeId?.className).not.toContain("break-all");
  expect(invokeId?.getAttribute("title")).toBe(invokeId?.textContent);
  expect(document.querySelector("table")).toBeNull();
  expect(document.querySelector('[data-testid="invocation-card-list"]')).toBeTruthy();
});
it("virtualizes large desktop datasets without mounting every row", async () => {
  await renderInteractiveTable(
    Array.from({ length: 1_000 }, (_, index) => createInvocationRecord(index)),
  );

  const mountedRows = Array.from(document.querySelectorAll('[data-testid="invocation-card"]'));
  const mountedText = mountedRows.map((row) => row.textContent ?? "").join(" ");

  expect(mountedRows.length).toBeGreaterThan(0);
  expect(mountedRows.length).toBeLessThan(80);
  expect(mountedText).toMatch(/virtual-proxy-\d+/);
  expect(document.body.textContent).not.toContain("virtual-proxy-1000");
  expect(document.querySelector('[data-testid="invocation-table-scroll"]')).toBeTruthy();
  expect(document.querySelector('[data-testid="invocation-list"]')).toBeNull();
});
it("mounts only the mobile card layout below the md breakpoint", async () => {
  Object.defineProperty(window, "innerWidth", {
    configurable: true,
    writable: true,
    value: 500,
  });

  await renderInteractiveTable(
    Array.from({ length: 20 }, (_, index) => createInvocationRecord(index)),
  );

  expect(document.querySelector('[data-testid="invocation-card-list"]')).toBeTruthy();
  expect(document.querySelectorAll('[data-testid="invocation-card"]').length).toBeGreaterThan(0);
  const invokeId = document.querySelector('[data-testid="invocation-id"]');
  expect(invokeId?.className).toContain("whitespace-nowrap");
  expect(invokeId?.className).toContain("overflow-hidden");
  expect(invokeId?.className).not.toContain("truncate");
  expect(invokeId?.className).not.toContain("break-all");
  expect(invokeId?.getAttribute("title")).toBe(invokeId?.textContent);
});
it("uses one clean highlight layer for a located desktop row", async () => {
  const records = [createInvocationRecord(0)];
  await renderInteractiveTable(records, {
    scrollTarget: { invokeId: "virtual-row-1", version: 1 },
  });

  await vi.waitFor(() => {
    expect(document.querySelector('[aria-current="true"]')).toBeTruthy();
  });
  const highlightedRow = document.querySelector('[aria-current="true"]');
  expect(highlightedRow?.className).toContain("outline-none");
  expect(highlightedRow?.className).toContain("border-primary/55");
});
it("uses a single border highlight for a located mobile card", async () => {
  Object.defineProperty(window, "innerWidth", {
    configurable: true,
    writable: true,
    value: 500,
  });
  const records = [createInvocationRecord(0)];
  await renderInteractiveTable(records, {
    scrollTarget: { invokeId: "virtual-row-1", version: 1 },
  });

  await vi.waitFor(() => {
    expect(document.querySelector('[aria-current="true"]')).toBeTruthy();
  });
  const highlightedCard = document.querySelector('[aria-current="true"]');
  expect(highlightedCard?.className).toContain("outline-none");
  expect(highlightedCard?.className).toContain("border-primary/55");
  expect(highlightedCard?.className).toContain("border-primary/55");
});
it("auto-expands a located card and clears its highlight after interaction", async () => {
  vi.useFakeTimers();
  const record = createInvocationRecord(0);
  await renderInteractiveTable([record], {
    scrollTarget: { invokeId: record.invokeId, attemptId: "attempt-target", version: 1 },
  });

  const card = document.querySelector('[data-testid="invocation-card"]');
  expect(card?.getAttribute("data-expanded")).toBe("true");
  expect(card?.querySelector("[data-invocation-detail]")).toBeTruthy();
  expect(card?.getAttribute("aria-current")).toBe("true");

  await act(async () => {
    vi.advanceTimersByTime(5_000);
  });
  expect(card?.getAttribute("aria-current")).toBe("true");

  await act(async () => {
    card?.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
    vi.advanceTimersByTime(1_499);
  });
  expect(card?.getAttribute("aria-current")).toBe("true");
  await act(async () => {
    vi.advanceTimersByTime(1);
  });
  expect(card?.getAttribute("aria-current")).toBeNull();
});
it("renders the WS transport badge for websocket records", () => {
  const websocketHtml = renderTable([
    {
      id: 21,
      invokeId: "invocation-ws-transport",
      occurredAt: "2026-03-07T03:13:52Z",
      createdAt: "2026-03-07T03:13:52Z",
      source: "proxy",
      endpoint: "/v1/responses",
      model: "gpt-5.5",
      status: "success",
      transport: "websocket",
    },
  ]);

  expect(websocketHtml).toContain('data-testid="invocation-transport-badge"');
  expect(websocketHtml).toContain('aria-hidden="true">WS</span>');
  expect(websocketHtml).toContain("WebSocket transport");
  expect(websocketHtml).toContain('title="WebSocket"');
});
it("does not render the WS transport badge for http or legacy records", () => {
  const html = renderTable([
    {
      id: 22,
      invokeId: "invocation-http-transport",
      occurredAt: "2026-03-07T03:13:51Z",
      createdAt: "2026-03-07T03:13:51Z",
      source: "proxy",
      endpoint: "/v1/responses",
      model: "gpt-5.5",
      status: "success",
      transport: "http",
    },
    {
      id: 23,
      invokeId: "invocation-legacy-transport",
      occurredAt: "2026-03-07T03:13:50Z",
      createdAt: "2026-03-07T03:13:50Z",
      source: "proxy",
      endpoint: "/v1/responses",
      model: "gpt-5.5",
      status: "success",
    },
  ]);

  expect(html).not.toContain('data-testid="invocation-transport-badge"');
});
it("renders resolved failure rows as failed even when the raw status still says running", () => {
  const html = renderTable([
    {
      id: 24,
      invokeId: "invocation-resolved-failure-running",
      occurredAt: "2026-03-07T03:13:50Z",
      createdAt: "2026-03-07T03:13:50Z",
      source: "proxy",
      proxyDisplayName: "legacy-edge",
      endpoint: "/v1/responses",
      model: "gpt-5.4",
      status: "running",
      failureClass: "service_failure",
      errorMessage: "[upstream_response_failed] server_error",
      totalTokens: 1024,
      cost: 0.0021,
    },
  ]);

  expect(html).toContain("失败");
  expect(html).not.toContain("运行中");
});
it("preserves nonstandard terminal statuses in the badge label", () => {
  const html = renderTable([
    {
      id: 25,
      invokeId: "invocation-http-502",
      occurredAt: "2026-03-07T03:13:49Z",
      createdAt: "2026-03-07T03:13:49Z",
      source: "proxy",
      proxyDisplayName: "legacy-edge",
      endpoint: "/v1/responses",
      model: "gpt-5.4",
      status: "http_502",
      failureClass: "service_failure",
      totalTokens: 1024,
      cost: 0.0021,
    },
  ]);

  expect(html).toContain("HTTP 502");
  expect(html).not.toContain(">失败<");
});
it("falls back to downstream-facing diagnostics in collapsed summaries when canonical upstream text is empty", () => {
  const html = renderTable([
    {
      id: 26,
      invokeId: "invocation-downstream-summary",
      occurredAt: "2026-03-07T03:13:48Z",
      createdAt: "2026-03-07T03:13:48Z",
      source: "proxy",
      proxyDisplayName: "legacy-edge",
      endpoint: "/v1/responses",
      model: "gpt-5.4",
      status: "failed",
      failureClass: "client_abort",
      failureKind: "downstream_closed",
      downstreamStatusCode: 200,
      downstreamErrorMessage:
        "[downstream_closed] downstream closed while streaming upstream response",
    },
  ]);

  expect(html).toContain("[downstream_closed] downstream closed while streaming upstream response");
});
it("renders reasoning effort and reasoning-token output breakdown in the summary rows", () => {
  const records: ApiInvocation[] = [
    {
      id: 1,
      invokeId: "invocation-reasoning-high",
      occurredAt: "2026-03-07T03:13:59Z",
      createdAt: "2026-03-07T03:13:59Z",
      source: "proxy",
      proxyDisplayName: "tokyo-edge-1",
      endpoint: "/v1/responses",
      model: "gpt-5.4",
      status: "success",
      inputTokens: 45559,
      cacheInputTokens: 43520,
      outputTokens: 83,
      reasoningTokens: 41,
      reasoningEffort: "high",
      totalTokens: 45642,
      cost: 0.0172,
      tReqReadMs: 3200,
      tReqParseMs: 20,
      tUpstreamConnectMs: 456.7,
      tUpstreamTtfbMs: 149.5,
      firstTokenMs: 3830,
      tUpstreamStreamMs: 3964,
      tTotalMs: 7794.1,
    },
    {
      id: 2,
      invokeId: "invocation-reasoning-missing",
      occurredAt: "2026-03-07T03:13:56Z",
      createdAt: "2026-03-07T03:13:56Z",
      source: "proxy",
      proxyDisplayName: "singapore-edge-2",
      endpoint: "/v1/chat/completions",
      model: "gpt-5.4",
      status: "failed",
      inputTokens: 61402,
      cacheInputTokens: 41216,
      outputTokens: 286,
      totalTokens: 61688,
      errorMessage: "upstream timeout",
      tUpstreamTtfbMs: 186.5,
      tTotalMs: 8444.2,
    },
  ];

  const html = renderTable(records);

  expect(html).toContain("high");
  expect(html).toContain("推理 41");
  expect(html).toContain("推理 —");
  expect(html).toContain("4 s");
  expect(html).toContain("3.8 s");
  expect(html).toContain("/v1/responses");
  expect(html).toContain("/v1/chat/completions");
  expect(html).toContain('data-reasoning-effort-tone="high"');
  expect(html).toContain("chip-tone-warning");
  expect(html).toContain(">—</span>");
});
it("renders account/proxy and TTFT/response-duration summaries", () => {
  const html = renderTable([
    {
      id: 31,
      invokeId: "invocation-account-summary",
      occurredAt: "2026-03-07T03:13:53Z",
      createdAt: "2026-03-07T03:13:53Z",
      source: "proxy",
      routeMode: "pool",
      upstreamAccountId: 7,
      upstreamAccountName: "pool-account-a",
      proxyDisplayName: "codex-relay-01",
      requesterIp: "203.0.113.10",
      responseContentEncoding: "gzip, br",
      requestCompressionAlgorithm: "zstd",
      endpoint: "/v1/responses",
      model: "gpt-5.4",
      status: "success",
      totalTokens: 2048,
      cost: 0.0042,
      tReqReadMs: 31,
      tReqParseMs: 1,
      tUpstreamConnectMs: 9330,
      tUpstreamTtfbMs: 118.2,
      firstTokenMs: 648.2,
      tUpstreamStreamMs: 260.4,
      tTotalMs: 910.4,
    },
    {
      id: 32,
      invokeId: "invocation-reverse-proxy-summary",
      occurredAt: "2026-03-07T03:13:52Z",
      createdAt: "2026-03-07T03:13:52Z",
      source: "proxy",
      routeMode: "forward_proxy",
      proxyDisplayName: "codex-relay-02",
      endpoint: "/v1/responses",
      model: "gpt-5.4",
      status: "success",
      totalTokens: 1024,
      cost: 0.0021,
      tReqReadMs: 120,
      tReqParseMs: 18,
      tUpstreamConnectMs: 512,
      tUpstreamTtfbMs: 96.5,
      tTotalMs: 804.4,
    },
  ]);

  expect(html).toContain("TTFT");
  expect(html).toContain("响应");
  expect(html).toContain("0.6 s");
  expect(html).toContain("0.3 s");
  expect(html).toContain("pool-account-a");
  expect(html).toContain("反向代理");
  expect(html).toContain("zstd");
  expect(html).not.toContain("gzip, br");
  expect(html).toContain('data-testid="invocation-account-name"');
});
it("shows a neutral pool-routing label before the upstream account identity is known", () => {
  const html = renderTable([
    {
      id: 34,
      invokeId: "invocation-pool-routing-pending",
      occurredAt: "2026-03-07T03:13:50Z",
      createdAt: "2026-03-07T03:13:50Z",
      source: "proxy",
      routeMode: "pool",
      proxyDisplayName: "codex-relay-03",
      endpoint: "/v1/responses",
      model: "gpt-5.4",
      status: "running",
    },
    {
      id: 35,
      invokeId: "invocation-pool-routing-id-only",
      occurredAt: "2026-03-07T03:13:49Z",
      createdAt: "2026-03-07T03:13:49Z",
      source: "proxy",
      routeMode: "pool",
      upstreamAccountId: 19,
      proxyDisplayName: "codex-relay-04",
      endpoint: "/v1/responses",
      model: "gpt-5.4",
      status: "running",
    },
    {
      id: 36,
      invokeId: "invocation-forward-proxy-fallback",
      occurredAt: "2026-03-07T03:13:48Z",
      createdAt: "2026-03-07T03:13:48Z",
      source: "proxy",
      routeMode: "forward_proxy",
      proxyDisplayName: "codex-relay-05",
      endpoint: "/v1/responses",
      model: "gpt-5.4",
      status: "running",
    },
    {
      id: 37,
      invokeId: "invocation-pool-account-unknown",
      occurredAt: "2026-03-07T03:13:47Z",
      createdAt: "2026-03-07T03:13:47Z",
      source: "proxy",
      routeMode: "pool",
      proxyDisplayName: "codex-relay-06",
      endpoint: "/v1/responses",
      model: "gpt-5.4",
      status: "success",
    },
  ]);

  expect(html).toContain("号池路由中");
  expect(html).toContain("账号 #19");
  expect(html).toContain("号池账号未知");
  expect(html).toContain("反向代理");
  expect(html).toContain("invocation-account-routing-in-progress");
});
it("uses the resolved display status when deciding whether a pool label is still pending", () => {
  const html = renderTable([
    {
      id: 38,
      invokeId: "invocation-pool-resolved-failure",
      occurredAt: "2026-03-07T03:13:46Z",
      createdAt: "2026-03-07T03:13:46Z",
      source: "proxy",
      routeMode: "pool",
      proxyDisplayName: "codex-relay-07",
      endpoint: "/v1/responses",
      model: "gpt-5.4",
      status: "running",
      failureClass: "service_failure",
      errorMessage: "[upstream_response_failed] server_error",
    },
  ]);

  expect(html).toContain("失败");
  expect(html).toContain("号池账号未知");
  expect(html).not.toContain("号池路由中");
});
it("shows the concrete upstream account on pool failures when identity is preserved", () => {
  const html = renderTable([
    {
      id: 39,
      invokeId: "invocation-pool-assigned-account-blocked",
      occurredAt: "2026-03-07T03:13:45Z",
      createdAt: "2026-03-07T03:13:45Z",
      source: "proxy",
      routeMode: "pool",
      upstreamAccountId: 52,
      upstreamAccountName: "sticky-account-52@example.com",
      proxyDisplayName: "codex-relay-08",
      endpoint: "/v1/responses",
      model: "gpt-5.4",
      status: "failed",
      failureClass: "service_failure",
      failureKind: "pool_assigned_account_blocked",
      errorMessage:
        '[pool_assigned_account_blocked] upstream account group "sticky-preflight-missing" has no bound forward proxy nodes',
    },
  ]);

  expect(html).toContain("sticky-account-52@example.com");
  expect(html).not.toContain("未分配上游账号");
});
it("uses the unassigned-account label only for true no-account pool failures", () => {
  const html = renderTable([
    {
      id: 40,
      invokeId: "invocation-pool-no-available-account",
      occurredAt: "2026-03-07T03:13:44Z",
      createdAt: "2026-03-07T03:13:44Z",
      source: "proxy",
      routeMode: "pool",
      proxyDisplayName: "codex-relay-09",
      endpoint: "/v1/responses",
      model: "gpt-5.4",
      status: "failed",
      failureClass: "service_failure",
      failureKind: "pool_no_available_account",
      errorMessage: "[pool_no_available_account] no assignable upstream account remains",
    },
    {
      id: 41,
      invokeId: "invocation-pool-routing-blocked",
      occurredAt: "2026-03-07T03:13:43Z",
      createdAt: "2026-03-07T03:13:43Z",
      source: "proxy",
      routeMode: "pool",
      proxyDisplayName: "codex-relay-10",
      endpoint: "/v1/responses",
      model: "gpt-5.4",
      status: "failed",
      failureClass: "service_failure",
      failureKind: "pool_routing_blocked",
      errorMessage:
        '[pool_routing_blocked] upstream account group "node-shunt-live" has no selectable forward proxy nodes',
    },
  ]);

  expect(html).toContain("未分配上游账号");
  expect(html).not.toContain("号池账号未知");
});
it("shows proxyDisplayName in both summary and expanded details when present", async () => {
  const record: ApiInvocation = {
    id: 33,
    invokeId: "invocation-proxy-detail-visible",
    occurredAt: "2026-03-07T03:13:51Z",
    createdAt: "2026-03-07T03:13:51Z",
    source: "proxy",
    routeMode: "pool",
    upstreamAccountId: 7,
    upstreamAccountName: "pool-account-a",
    proxyDisplayName: "codex-relay-01",
    responseContentEncoding: "gzip, br",
    endpoint: "/v1/responses",
    model: "gpt-5.4",
    status: "success",
    totalTokens: 2048,
    cost: 0.0042,
    tUpstreamTtfbMs: 118.2,
    tTotalMs: 910.4,
  };
  apiMocks.fetchInvocationWorkflowDetail.mockResolvedValueOnce(createWorkflowDetailFixture(record));

  await renderInteractiveTable([record]);

  const beforeExpandMatches = document.body.textContent?.match(/codex-relay-01/g) ?? [];
  expect(beforeExpandMatches.length).toBeGreaterThanOrEqual(1);

  const toggle = document.querySelector(
    '[data-testid="invocation-table-scroll"] button[aria-expanded="false"]',
  ) as HTMLButtonElement | null;
  expect(toggle).toBeTruthy();

  await act(async () => {
    toggle?.click();
    await Promise.resolve();
    await Promise.resolve();
  });

  await waitForCondition(() => document.body.textContent?.includes("工作流时间线") === true);
  expect(apiMocks.fetchInvocationWorkflowDetail).toHaveBeenCalledWith(33);

  const requestButton = Array.from(document.querySelectorAll("button")).find(
    (button) =>
      button.textContent?.includes("请求") &&
      button.textContent?.includes("gpt-5.4") &&
      !button.textContent?.includes("请求体"),
  ) as HTMLButtonElement | undefined;
  expect(requestButton).toBeTruthy();

  await act(async () => {
    requestButton?.click();
    await Promise.resolve();
    await Promise.resolve();
  });

  await waitForCondition(() => document.body.textContent?.includes("代理显示名") === true);
  const afterExpandMatches = document.body.textContent?.match(/codex-relay-01/g) ?? [];
  expect(afterExpandMatches.length).toBeGreaterThanOrEqual(2);
});
it("does not display source as a request-detail proxy fallback", () => {
  const record: ApiInvocation = {
    id: 36,
    invokeId: "invocation-source-not-proxy-fallback",
    occurredAt: "2026-03-24T06:50:52Z",
    createdAt: "2026-03-24T06:50:52Z",
    source: "xy-custom-source",
    routeMode: "pool",
    upstreamAccountId: 17,
    upstreamAccountName: "API Keys Pool",
    endpoint: "/v1/responses",
    model: "gpt-5.4",
    status: "failed",
    totalTokens: 4096,
    cost: 0.1024,
  };

  const html = renderToStaticMarkup(
    <I18nProvider>
      <InvocationDetailProbe record={record} />
    </I18nProvider>,
  );

  expect(html).not.toContain("来源");
  expect(html).not.toContain("xy-custom-source");
});
it("shows raw endpoint plus compaction request and response semantics in request details", () => {
  const record: ApiInvocation = {
    id: 37,
    invokeId: "invocation-remote-v2-detail",
    occurredAt: "2026-03-24T06:51:52Z",
    createdAt: "2026-03-24T06:51:52Z",
    source: "proxy",
    routeMode: "pool",
    upstreamAccountId: 17,
    upstreamAccountName: "API Keys Pool",
    proxyDisplayName: "api-keys-gateway",
    endpoint: "/v1/responses",
    compactionRequestKind: "remote_v2",
    compactionResponseKind: "remote_v2",
    model: "gpt-5.4",
    status: "success",
    totalTokens: 4096,
    cost: 0.1024,
  };

  const html = renderToStaticMarkup(
    <I18nProvider>
      <InvocationDetailProbe record={record} />
    </I18nProvider>,
  );

  expect(html).toContain("/v1/responses");
  expect(html).toMatch(/压缩请求|Compaction request/);
  expect(html).toMatch(/压缩响应|Compaction response/);
  expect(html).toMatch(/远程压缩V2|Remote compaction V2/);
});
it("shows image tool detail semantics independently from endpoint badges", () => {
  const record: ApiInvocation = {
    id: 38,
    invokeId: "invocation-image-tool-detail",
    occurredAt: "2026-03-24T06:52:52Z",
    createdAt: "2026-03-24T06:52:52Z",
    source: "proxy",
    routeMode: "pool",
    upstreamAccountId: 17,
    upstreamAccountName: "API Keys Pool",
    proxyDisplayName: "api-keys-gateway",
    endpoint: "/v1/responses",
    compactionRequestKind: "remote_v2",
    compactionResponseKind: "remote_v2",
    imageIntent: "direct_image",
    model: "gpt-image-1",
    status: "success",
    totalTokens: 4096,
    cost: 0.1024,
  };

  const html = renderToStaticMarkup(
    <I18nProvider>
      <InvocationDetailProbe record={record} />
    </I18nProvider>,
  );

  expect(html).toMatch(/图片工具|Image tool/);
  expect(html).toContain("direct_image");
  expect(html).toMatch(/远程压缩V2|Remote compaction V2/);
});
it("keeps the image tool badge out of the model column", async () => {
  await renderInteractiveTable([
    {
      id: 39,
      invokeId: "invocation-model-column-image-badge",
      occurredAt: "2026-03-24T06:53:52Z",
      createdAt: "2026-03-24T06:53:52Z",
      source: "proxy",
      proxyDisplayName: "codex-image-edge",
      endpoint: "/v1/responses",
      imageIntent: "yes",
      model: "gpt-image-1",
      status: "success",
      totalTokens: 1024,
      cost: 0.0102,
    },
  ]);

  const modelCell = host?.querySelector('[data-testid="invocation-table-model"]')?.parentElement;
  const endpointCell = host?.querySelector(
    '[data-testid="invocation-image-tool-badge"]',
  )?.parentElement;

  expect(modelCell?.textContent).toContain("gpt-image-1");
  expect(modelCell?.querySelector('[data-testid="invocation-image-tool-badge"]')).toBeNull();
  expect(endpointCell?.textContent).toMatch(/图片工具|Image tool/);
});
it("keeps request and response model identities visible when routing changes the model", async () => {
  await renderInteractiveTable([
    {
      id: 40,
      invokeId: "invocation-model-routing-identity",
      occurredAt: "2026-03-24T06:54:52Z",
      createdAt: "2026-03-24T06:54:52Z",
      source: "proxy",
      proxyDisplayName: "routing-proxy",
      endpoint: "/v1/responses",
      model: "gpt-5.6-sol",
      requestModel: "gpt-5.6-terra",
      responseModel: "gpt-5.6-sol",
      status: "success",
      totalTokens: 1024,
      cost: 0.0102,
    },
  ]);

  const identities = [
    ...(host?.querySelectorAll('[data-testid="invocation-table-model"] [data-model-identity]') ??
      []),
  ].map((element) => element.getAttribute("data-model-identity"));

  expect(identities).toEqual(["gpt-5.6-terra", "gpt-5.6-sol"]);
});
