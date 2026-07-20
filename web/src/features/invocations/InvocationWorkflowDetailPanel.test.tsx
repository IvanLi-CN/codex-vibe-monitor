/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import type { ApiInvocation, ApiInvocationWorkflowDetailResponse } from "../../lib/api";
import {
  formatDashboardWorkingConversationSequenceId,
  hashDashboardWorkingConversationKey,
} from "../../lib/dashboardWorkingConversations";
import { InvocationWorkflowDetailPanel } from "./InvocationWorkflowDetailPanel";
import {
  failedWorkflowFinalResponseBodyText,
  failedWorkflowRequestBodySize,
  failedWorkflowRequestBodyText,
  failedWorkflowResponseBody,
  failedWorkflowResponseBodySize,
  failedWorkflowResponseBodyText,
} from "./InvocationWorkflowDetailPanel.fixtures";

const { apiMocks } = vi.hoisted(() => ({
  apiMocks: {
    fetchInvocationRequestBody: vi.fn(),
    fetchInvocationResponseBody: vi.fn(),
    fetchInvocationWorkflowDetail: vi.fn(),
  },
}));

vi.mock("../../lib/api", async () => {
  const actual = await vi.importActual<typeof import("../../lib/api")>("../../lib/api");
  return {
    ...actual,
    fetchInvocationRequestBody: apiMocks.fetchInvocationRequestBody,
    fetchInvocationResponseBody: apiMocks.fetchInvocationResponseBody,
    fetchInvocationWorkflowDetail: apiMocks.fetchInvocationWorkflowDetail,
  };
});

vi.mock("../../i18n", () => ({
  useTranslation: () => ({
    locale: "zh",
    t: (key: string) => key,
  }),
}));

let host: HTMLDivElement | null = null;
let root: Root | null = null;

beforeAll(() => {
  Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", {
    configurable: true,
    writable: true,
    value: true,
  });
});

afterEach(() => {
  act(() => {
    root?.unmount();
  });
  host?.remove();
  host = null;
  root = null;
  apiMocks.fetchInvocationRequestBody.mockReset();
  apiMocks.fetchInvocationResponseBody.mockReset();
  apiMocks.fetchInvocationWorkflowDetail.mockReset();
});

function render(ui: React.ReactNode) {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => {
    root?.render(ui);
  });
}

async function flushAsyncWork(rounds = 4) {
  await act(async () => {
    for (let index = 0; index < rounds; index += 1) {
      await Promise.resolve();
    }
    await new Promise<void>((resolve) => window.setTimeout(resolve, 0));
  });
}

async function waitFor(check: () => boolean, timeoutMs = 1000) {
  const startedAt = Date.now();
  while (Date.now() - startedAt < timeoutMs) {
    await flushAsyncWork();
    if (check()) return;
  }
  throw new Error("timed out waiting for workflow detail UI");
}

function createRecord(overrides: Partial<ApiInvocation> = {}): ApiInvocation {
  return {
    id: 77,
    invokeId: "invoke-workflow-77",
    occurredAt: "2026-07-15T13:55:59Z",
    createdAt: "2026-07-15T13:55:59Z",
    status: "failed",
    source: "proxy",
    routeMode: "pool",
    proxyDisplayName: "tokyo-edge-01",
    upstreamAccountId: 42,
    upstreamAccountName: "pool-alpha@example.com",
    endpoint: "/v1/responses",
    model: "gpt-5.4",
    requestModel: "gpt-5.4",
    responseModel: "gpt-5.4",
    inputTokens: 148,
    outputTokens: 92,
    cacheInputTokens: 36,
    reasoningTokens: 24,
    reasoningEffort: "high",
    totalTokens: 240,
    cost: 0.0182,
    responseContentEncoding: "gzip",
    requestedServiceTier: "priority",
    serviceTier: "priority",
    billingServiceTier: "priority",
    promptCacheKey: "019d5ea7-519d-7312-a2e8-ef07abb7c09f",
    stickyKey: "sk-route-77",
    tReqReadMs: 14,
    tReqParseMs: 8,
    tUpstreamConnectMs: 136,
    tUpstreamTtfbMs: 98,
    tUpstreamStreamMs: 324,
    tRespParseMs: 12,
    tPersistMs: 9,
    tTotalMs: 17430,
    errorMessage: "downstream closed while streaming",
    failureClass: "service_failure",
    failureKind: "downstream_closed",
    downstreamStatusCode: 502,
    downstreamErrorMessage: "downstream closed while streaming",
    ...overrides,
  };
}

function createWorkflowDetailResponse(): ApiInvocationWorkflowDetailResponse {
  return {
    hero: {
      recordId: 77,
      invokeId: "invoke-workflow-77",
      promptCacheKey: "019d5ea7-519d-7312-a2e8-ef07abb7c09f",
      routeMode: "pool",
      endpoint: "/v1/responses",
      requestModel: "gpt-5.4",
      responseModel: "gpt-5.4",
      finalStatus: "failed",
      failureClass: "service_failure",
      downstreamStatusCode: 502,
      upstreamAccountId: 42,
      upstreamAccountName: "pool-alpha@example.com",
      totalDurationMs: 17430,
      timelineAttemptCount: 1,
      poolAttemptCount: 1,
      totalTokens: 240,
      cost: 0.0182,
      occurredAt: "2026-07-15T13:55:59Z",
    },
    timeline: [
      {
        blockId: "route-1",
        kind: "routingDecision",
        occurredAt: "2026-07-15T13:55:59Z",
        title: "Pool route selected",
        subtitle: "pool /v1/responses",
        status: "pool",
        detail: {
          request: {
            endpoint: "/v1/responses",
            routeMode: "pool",
            requestModel: "gpt-5.4",
            responseModel: "gpt-5.4",
            requestedServiceTier: "priority",
            reasoningEffort: "high",
            compactionRequestKind: "remote_v2",
            promptCacheKey: "019d5ea7-519d-7312-a2e8-ef07abb7c09f",
            requesterIp: "203.0.113.10",
            routing: {
              routeMode: "pool",
              proxyDisplayName: "tokyo-edge-01",
              upstreamRouteKey: "route-pool-alpha",
              proxyBindingKey: "fpb_tokyo_alpha",
            },
            headers: {
              userAgent: "monitor-ui/1.0",
              xForwardedFor: "203.0.113.10",
            },
            bodyCapture: {
              availableAtInvocationLevel: true,
              size: failedWorkflowRequestBodySize,
              truncated: false,
              detailLevel: "full",
            },
          },
          requestHeaders: {
            userAgent: "monitor-ui/1.0",
            xForwardedFor: "203.0.113.10",
          },
          requestBody: {
            availableAtInvocationLevel: true,
            size: failedWorkflowRequestBodySize,
            truncated: false,
            detailLevel: "full",
          },
        },
      },
      {
        blockId: "attempt-1",
        kind: "attempt",
        occurredAt: "2026-07-15T13:56:02Z",
        title: "Attempt 1",
        subtitle: "pool-alpha@example.com",
        status: "transport_failure",
        attempt: {
          synthetic: false,
          attemptId: "attempt-1",
          occurredAt: "2026-07-15T13:56:02Z",
          endpoint: "/v1/responses",
          stickyKey: "sk-route-77",
          upstreamAccountId: 42,
          upstreamAccountName: "pool-alpha@example.com",
          requestModel: "gpt-5.4",
          responseModel: "gpt-5.4",
          upstreamRouteKey: "route-pool-alpha",
          proxyBindingKeySnapshot: "fpb_tokyo_alpha",
          attemptIndex: 1,
          distinctAccountIndex: 1,
          sameAccountRetryIndex: 1,
          requesterIp: "203.0.113.10",
          startedAt: "2026-07-15T13:56:02Z",
          finishedAt: "2026-07-15T13:56:08Z",
          status: "transport_failure",
          phase: "streaming",
          httpStatus: 200,
          failureKind: "downstream_closed",
          errorMessage: "upstream stream aborted",
          downstreamErrorMessage: "downstream closed while streaming",
          connectLatencyMs: 120,
          firstByteLatencyMs: 640,
          streamLatencyMs: 5430,
          upstreamRequestId: "req_77",
          requestSummary: {
            endpoint: "/v1/responses",
            routeMode: "pool",
            transport: "http",
            requestModel: "gpt-5.4",
            responseModel: "gpt-5.4",
            stickyKey: "sk-route-77",
            requestedServiceTier: "priority",
            reasoningEffort: "high",
            compactionRequestKind: "remote_v2",
            account: {
              id: 42,
              name: "pool-alpha@example.com",
            },
            headers: {
              userAgent: "monitor-ui/1.0",
              xForwardedFor: "203.0.113.10",
            },
            routing: {
              upstreamRouteKey: "route-pool-alpha",
              proxyBindingKey: "fpb_tokyo_alpha",
              proxyDisplayName: "tokyo-edge-01",
            },
            compression: {
              algorithm: "gzip",
              mode: "recompressed",
              logicalBodyBytes: 1000,
              transmittedBodyBytes: 580,
              savedBytes: 420,
              ratioPct: -42,
              approxUploadBytes: 644,
              approxDownloadBytes: 812,
            },
            bodyCapture: {
              availableAtInvocationLevel: true,
              size: failedWorkflowRequestBodySize,
              truncated: false,
              detailLevel: "full",
            },
          },
          responseSummary: {
            status: "transport_failure",
            phase: "streaming",
            failureKind: "downstream_closed",
            downstreamErrorMessage: "downstream closed while streaming",
            responseContentEncoding: "gzip",
            serviceTier: "priority",
            billingServiceTier: "priority",
            compactionResponseKind: "remote_v2",
            toolCalls: ["web_search_preview", "function:search_docs"],
            outputItems: failedWorkflowResponseBody.output.length,
            headers: {
              contentEncoding: "gzip",
              upstreamRequestId: "req_77",
            },
            delivery: {
              forwardedChunkCount: 12,
              streamFailureOrigin: "downstream",
              downstreamClosePhase: "streaming",
            },
            responseBodyCapture: {
              availableAtInvocationLevel: true,
              size: failedWorkflowResponseBodySize,
              truncated: false,
              detailLevel: "full",
            },
            usage: {
              cacheWriteTokens: 112,
              cacheInputTokens: 36,
              outputTokens: 92,
              totalTokens: 240,
            },
          },
        },
      },
      {
        blockId: "final-1",
        kind: "systemFinalFailure",
        occurredAt: "2026-07-15T13:56:08Z",
        title: "Final adjudication",
        subtitle: "Returned to caller",
        status: "failed",
        detail: {
          downstreamStatusCode: 502,
          failureClass: "service_failure",
          errorMessage: "downstream closed while streaming",
        },
        responseBody: {
          available: true,
          bodyText: failedWorkflowFinalResponseBodyText,
        },
      },
    ],
    reconstructed: false,
    partial: false,
    partialReason: null,
  };
}

function createBlockedPoolWorkflowResponse(): ApiInvocationWorkflowDetailResponse {
  return {
    hero: {
      recordId: 77,
      invokeId: "invoke-workflow-77",
      promptCacheKey: "019d5ea7-519d-7312-a2e8-ef07abb7c09f",
      routeMode: "pool",
      endpoint: "/v1/responses",
      requestModel: "gpt-5.4",
      responseModel: "gpt-5.4",
      finalStatus: "http_503",
      failureClass: "service_failure",
      downstreamStatusCode: 503,
      totalDurationMs: 810,
      timelineAttemptCount: 0,
      poolAttemptCount: 0,
      totalTokens: null,
      cost: null,
      occurredAt: "2026-07-18T02:03:02Z",
    },
    timeline: [
      {
        blockId: "route-terminal",
        kind: "routingDecision",
        occurredAt: "2026-07-18T02:03:02Z",
        title: "Route resolution",
        subtitle: "gpt-5.4 · /v1/responses",
        detail: {
          request: {
            endpoint: "/v1/responses",
            routeMode: "pool",
            requestModel: "gpt-5.4",
            responseModel: "gpt-5.4",
            requestedServiceTier: "priority",
            reasoningEffort: "high",
            compactionRequestKind: "remote_v2",
            promptCacheKey: "019d5ea7-519d-7312-a2e8-ef07abb7c09f",
            requesterIp: "203.0.113.10",
            routing: {
              routeMode: "pool",
              proxyDisplayName: "tokyo-edge-01",
            },
            headers: {
              userAgent: "monitor-ui/1.0",
              xForwardedFor: "203.0.113.10",
            },
            bodyCapture: {
              availableAtInvocationLevel: true,
              size: failedWorkflowRequestBodySize,
              truncated: false,
              detailLevel: "full",
            },
          },
          requestHeaders: {
            userAgent: "monitor-ui/1.0",
            xForwardedFor: "203.0.113.10",
          },
          requestBody: {
            availableAtInvocationLevel: true,
            size: failedWorkflowRequestBodySize,
            truncated: false,
            detailLevel: "full",
          },
        },
      },
      {
        blockId: "system-final-failure",
        kind: "systemFinalFailure",
        occurredAt: "2026-07-18T02:03:02Z",
        title: "Final downstream response",
        subtitle: "pool_assigned_account_blocked",
        status: "http_503",
        detail: {
          downstreamStatusCode: 503,
          failureClass: "service_failure",
          failureKind: "pool_assigned_account_blocked",
          errorMessage: "[pool_assigned_account_blocked] pool assigned account blocked",
          downstreamErrorMessage: "pool assigned account blocked",
        },
        responseBody: {
          available: true,
          bodyText:
            '{"error":"pool assigned account blocked","cvmId":"invoke-workflow-77","code":"pool_assigned_account_blocked"}',
        },
      },
    ],
    reconstructed: false,
    partial: false,
    partialReason: null,
  };
}

describe("InvocationWorkflowDetailPanel", () => {
  it("renders hero information, timeline blocks, and detail sections from the workflow API", async () => {
    apiMocks.fetchInvocationWorkflowDetail.mockResolvedValue(createWorkflowDetailResponse());
    apiMocks.fetchInvocationRequestBody.mockResolvedValue({
      available: true,
      bodyText: failedWorkflowRequestBodyText,
      headers: {
        userAgent: "monitor-ui/1.0",
        xForwardedFor: "203.0.113.10",
      },
      routing: {
        routeMode: "pool",
        stickyKey: "sk-route-77",
      },
      bodySize: failedWorkflowRequestBodySize,
      detailLevel: "full",
      captureSource: "raw_file",
    });
    apiMocks.fetchInvocationResponseBody.mockResolvedValue({
      available: true,
      bodyText: failedWorkflowResponseBodyText,
      headers: {
        contentEncoding: "gzip",
        upstreamRequestId: "req_77",
      },
      routing: {
        forwardedChunkCount: 12,
      },
      bodySize: failedWorkflowResponseBodySize,
      detailLevel: "full",
      captureSource: "raw_file",
    });
    const record = createRecord();
    const requestBodySizeLabel = `${failedWorkflowRequestBodySize.toLocaleString("zh")} B`;
    const responseBodySizeLabel = `${failedWorkflowResponseBodySize.toLocaleString("zh")} B`;
    const expectedConversationId = formatDashboardWorkingConversationSequenceId(
      `WC-${hashDashboardWorkingConversationKey(record.promptCacheKey ?? "").slice(0, 6)}`,
    );

    render(<InvocationWorkflowDetailPanel record={record} />);

    await waitFor(() => (host?.textContent ?? "").includes("Final adjudication"));

    expect(apiMocks.fetchInvocationWorkflowDetail).toHaveBeenCalledWith(77);
    expect(host?.textContent ?? "").toContain("调用 ID");
    expect(host?.textContent ?? "").toContain("invoke-workflow-77");
    expect(host?.textContent ?? "").toContain(expectedConversationId);
    expect(host?.textContent ?? "").toContain("17.4 s");
    expect(host?.textContent ?? "").toContain("019d5ea7-519d-7312-a2e8-ef07abb7c09f");
    expect(host?.textContent ?? "").toContain(requestBodySizeLabel);
    expect(host?.textContent ?? "").toContain("priority");
    expect(host?.textContent ?? "").toContain(responseBodySizeLabel);
    expect(host?.textContent ?? "").toContain("monitor-ui/1.0");
    expect(host?.textContent ?? "").toContain("tokyo-edge-01");
    expect(host?.textContent ?? "").toContain("gzip");
    expect(host?.textContent ?? "").toContain("远程压缩V2");
    expect(host?.textContent ?? "").toContain("web_search");
    expect(host?.textContent ?? "").toContain("+1");
    expect(host?.textContent ?? "").toContain("上游 HTTP 200");
    expect(host?.textContent ?? "").toContain("attempt-1");
    expect(host?.textContent ?? "").toContain("输入写 112");
    expect(host?.textContent ?? "").toContain("输入读 36");
    expect(host?.textContent ?? "").toContain("-42% (1,000 B -> 580 B)");
    expect(host?.textContent ?? "").not.toContain("Attempt 1");
    expect(host?.textContent ?? "").not.toContain("12 块转发");
    expect(host?.textContent ?? "").not.toContain("240 Token");
    expect(host?.textContent ?? "").not.toContain("200 → 502");
    expect(host?.textContent ?? "").not.toContain("首字");
    expect(host?.textContent ?? "").not.toContain("remote_v2");
    expect(host?.textContent ?? "").not.toContain("HTTP gzip");
    expect(host?.textContent ?? "").not.toContain("5,430 ms");

    const requestBodyButton = Array.from(host?.querySelectorAll("button") ?? []).find(
      (candidate): candidate is HTMLButtonElement =>
        candidate instanceof HTMLButtonElement &&
        candidate.textContent?.includes("请求体") &&
        candidate.textContent?.includes("gzip") &&
        candidate.textContent?.includes(requestBodySizeLabel),
    );
    expect(requestBodyButton).not.toBeNull();
    expect(requestBodyButton?.textContent ?? "").toContain("-42% (1,000 B -> 580 B)");
    expect(requestBodyButton?.textContent ?? "").not.toContain("远程压缩V2");
    expect(requestBodyButton?.textContent ?? "").not.toContain("调用级存档");
    expect(requestBodyButton?.textContent ?? "").not.toContain("未截断");
    act(() => {
      requestBodyButton?.click();
    });
    await flushAsyncWork();
    await waitFor(() => apiMocks.fetchInvocationRequestBody.mock.calls.length > 0);

    expect(apiMocks.fetchInvocationRequestBody).toHaveBeenCalledWith(77);
    expect(host?.textContent ?? "").toContain("归档");
    expect(host?.textContent ?? "").toContain("调用级");
    expect(host?.textContent ?? "").toContain("近似上传");
    expect(host?.textContent ?? "").toContain("644 B");
    expect(host?.textContent ?? "").toContain("近似下载");
    expect(host?.textContent ?? "").toContain("812 B");
    expect(host?.textContent ?? "").not.toContain("原始请求体：调用级存档。");
    const compressionPanel = Array.from(host?.querySelectorAll("section") ?? []).find((candidate) =>
      candidate.textContent?.includes("HTTP 请求压缩"),
    );
    expect(compressionPanel?.querySelector("dl")?.className).toContain("lg:grid-cols-5");

    act(() => {
      requestBodyButton?.click();
    });
    await flushAsyncWork();

    expect(host?.textContent ?? "").not.toContain("归档调用级");

    const responseBodyButton = Array.from(host?.querySelectorAll("button") ?? []).find(
      (candidate): candidate is HTMLButtonElement =>
        candidate instanceof HTMLButtonElement &&
        candidate.textContent?.includes("响应体") &&
        candidate.textContent?.includes("gzip") &&
        candidate.textContent?.includes(responseBodySizeLabel),
    );
    expect(responseBodyButton).not.toBeNull();
    expect(responseBodyButton?.textContent ?? "").toContain("downstream_closed");
    expect(responseBodyButton?.textContent ?? "").not.toContain("调用级存档");
    expect(responseBodyButton?.textContent ?? "").not.toContain("未截断");
    expect(responseBodyButton?.textContent ?? "").not.toContain("240 Token");

    const responseHeadersButton = Array.from(host?.querySelectorAll("button") ?? []).find(
      (candidate): candidate is HTMLButtonElement =>
        candidate instanceof HTMLButtonElement &&
        candidate.textContent?.includes("响应头") &&
        candidate.textContent?.includes("HTTP 200"),
    );
    expect(responseHeadersButton).not.toBeNull();
    expect(responseHeadersButton?.textContent ?? "").not.toContain("req_77");
    expect(responseHeadersButton?.textContent ?? "").not.toContain("12 块转发");
    act(() => {
      responseBodyButton?.click();
    });
    await flushAsyncWork();
    await waitFor(() => apiMocks.fetchInvocationResponseBody.mock.calls.length > 0);

    expect(apiMocks.fetchInvocationResponseBody).toHaveBeenCalledWith(77);
    expect(host?.textContent ?? "").toContain("归档");
    expect(host?.textContent ?? "").toContain("调用级");
    expect(host?.textContent ?? "").not.toContain("最终响应体：调用级存档。");
  });

  it("does not request workflow detail for transient records without a persisted id", async () => {
    render(<InvocationWorkflowDetailPanel record={createRecord({ id: 0 })} />);

    await waitFor(() => (host?.textContent ?? "").includes("调用未落盘"));

    expect(apiMocks.fetchInvocationWorkflowDetail).not.toHaveBeenCalled();
  });

  it("renders route-only blocked workflows without a pseudo attempt and replays the final body", async () => {
    apiMocks.fetchInvocationWorkflowDetail.mockResolvedValue(createBlockedPoolWorkflowResponse());
    apiMocks.fetchInvocationRequestBody.mockResolvedValue({
      available: true,
      bodyText: failedWorkflowRequestBodyText,
      headers: {
        userAgent: "monitor-ui/1.0",
        xForwardedFor: "203.0.113.10",
      },
      routing: {
        routeMode: "pool",
        stickyKey: "sk-route-77",
      },
      bodySize: failedWorkflowRequestBodySize,
      detailLevel: "full",
      captureSource: "raw_file",
    });

    render(
      <InvocationWorkflowDetailPanel
        record={createRecord({
          status: "http_503",
          tTotalMs: 810,
          failureKind: "pool_assigned_account_blocked",
          errorMessage: "pool assigned account blocked",
          downstreamStatusCode: 503,
          downstreamErrorMessage: "pool assigned account blocked",
          totalTokens: null,
          cost: null,
          upstreamAccountId: null,
          upstreamAccountName: null,
          poolAttemptCount: 0,
        })}
      />,
    );

    await waitFor(() => (host?.textContent ?? "").includes("Final downstream response"));

    expect(host?.textContent ?? "").toContain("尝试次数");
    expect(host?.textContent ?? "").toContain("0");
    expect(host?.textContent ?? "").not.toContain("attempt-1");
    expect(host?.textContent ?? "").not.toContain("Attempt 1");

    const routeBodyButton = Array.from(host?.querySelectorAll("button") ?? []).find(
      (candidate): candidate is HTMLButtonElement =>
        candidate instanceof HTMLButtonElement &&
        candidate.textContent?.includes("请求体") &&
        candidate.textContent?.includes(`${failedWorkflowRequestBodySize.toLocaleString("zh")} B`),
    );
    expect(routeBodyButton).not.toBeNull();
    act(() => {
      routeBodyButton?.click();
    });
    await waitFor(() => apiMocks.fetchInvocationRequestBody.mock.calls.length > 0);
    expect(apiMocks.fetchInvocationRequestBody).toHaveBeenCalledWith(77);
    expect(host?.textContent ?? "").toContain("调用级");

    const finalBodyButton = Array.from(host?.querySelectorAll("button") ?? []).find(
      (candidate): candidate is HTMLButtonElement =>
        candidate instanceof HTMLButtonElement &&
        candidate.textContent?.includes("返回体") &&
        candidate.textContent?.includes("HTTP 503"),
    );
    expect(finalBodyButton).not.toBeNull();
    act(() => {
      finalBodyButton?.click();
    });
    await flushAsyncWork();
    expect(host?.textContent ?? "").toContain("pool assigned account blocked");
    expect(host?.textContent ?? "").not.toContain("missing_body");
  });
});
