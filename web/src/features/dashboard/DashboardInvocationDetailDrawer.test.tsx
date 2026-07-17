/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  ApiInvocation,
  ApiInvocationResponseBodyResponse,
  ApiInvocationWorkflowDetailResponse,
  InvocationRecordsResponse,
  PromptCacheConversationInvocationPreview,
} from "../../lib/api";
import type { DashboardWorkingConversationInvocationSelection } from "../../lib/dashboardWorkingConversations";
import { DashboardInvocationDetailDrawer } from "./DashboardInvocationDetailDrawer";

const { apiMocks } = vi.hoisted(() => ({
  apiMocks: {
    fetchInvocationRecords: vi.fn(),
    fetchInvocationRequestBody: vi.fn(),
    fetchInvocationResponseBody: vi.fn(),
    fetchInvocationWorkflowDetail: vi.fn(),
  },
}));

vi.mock("../../lib/api", async () => {
  const actual = await vi.importActual<typeof import("../../lib/api")>("../../lib/api");
  return {
    ...actual,
    fetchInvocationRecords: apiMocks.fetchInvocationRecords,
    fetchInvocationRequestBody: apiMocks.fetchInvocationRequestBody,
    fetchInvocationResponseBody: apiMocks.fetchInvocationResponseBody,
    fetchInvocationWorkflowDetail: apiMocks.fetchInvocationWorkflowDetail,
  };
});

vi.mock("../../i18n", () => ({
  useTranslation: () => ({
    locale: "zh",
    t: (key: string, values?: Record<string, string | number>) => {
      if (values?.error) return `${key}: ${values.error}`;
      if (values?.value) return `${key}: ${values.value}`;
      return key;
    },
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

beforeEach(() => {
  apiMocks.fetchInvocationRecords.mockReset();
  apiMocks.fetchInvocationRequestBody.mockReset();
  apiMocks.fetchInvocationResponseBody.mockReset();
  apiMocks.fetchInvocationWorkflowDetail.mockReset();
  apiMocks.fetchInvocationRequestBody.mockResolvedValue({
    available: false,
    unavailableReason: "request body not loaded in drawer test",
  });
  apiMocks.fetchInvocationResponseBody.mockResolvedValue(createResponseBodyFixture());
  apiMocks.fetchInvocationWorkflowDetail.mockImplementation(async () =>
    createWorkflowDetailFixture(createRecord()),
  );
});

afterEach(() => {
  act(() => {
    root?.unmount();
  });
  host?.remove();
  host = null;
  root = null;
  vi.restoreAllMocks();
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
  throw new Error("timed out waiting for async UI state");
}

function createPreview(
  overrides: Partial<PromptCacheConversationInvocationPreview> & {
    id: number;
    invokeId: string;
    occurredAt: string;
    status: string;
  },
): PromptCacheConversationInvocationPreview {
  return {
    id: overrides.id,
    invokeId: overrides.invokeId,
    occurredAt: overrides.occurredAt,
    status: overrides.status,
    failureClass: overrides.failureClass ?? null,
    routeMode: overrides.routeMode ?? "forward_proxy",
    model: overrides.model ?? "gpt-5.4",
    requestModel: "requestModel" in overrides ? (overrides.requestModel ?? null) : "gpt-5.4",
    responseModel:
      "responseModel" in overrides
        ? (overrides.responseModel ?? null)
        : (overrides.model ?? "gpt-5.4"),
    totalTokens: overrides.totalTokens ?? 240,
    cost: overrides.cost ?? 0.0182,
    proxyDisplayName: overrides.proxyDisplayName ?? "tokyo-edge-01",
    upstreamAccountId: overrides.upstreamAccountId ?? 42,
    upstreamAccountName: overrides.upstreamAccountName ?? "pool-alpha@example.com",
    endpoint: overrides.endpoint ?? "/v1/responses",
    source: overrides.source ?? "proxy",
    inputTokens: overrides.inputTokens ?? 148,
    outputTokens: overrides.outputTokens ?? 92,
    cacheInputTokens: overrides.cacheInputTokens ?? 36,
    reasoningTokens: overrides.reasoningTokens ?? 24,
    reasoningEffort: overrides.reasoningEffort ?? "high",
    errorMessage: overrides.errorMessage,
    failureKind: overrides.failureKind,
    isActionable: overrides.isActionable,
    responseContentEncoding: overrides.responseContentEncoding ?? "gzip",
    requestedServiceTier: overrides.requestedServiceTier ?? "priority",
    serviceTier: overrides.serviceTier ?? "priority",
    tReqReadMs: overrides.tReqReadMs ?? 14,
    tReqParseMs: overrides.tReqParseMs ?? 8,
    tUpstreamConnectMs: overrides.tUpstreamConnectMs ?? 136,
    tUpstreamTtfbMs: overrides.tUpstreamTtfbMs ?? 98,
    tUpstreamStreamMs: overrides.tUpstreamStreamMs ?? 324,
    tRespParseMs: overrides.tRespParseMs ?? 12,
    tPersistMs: overrides.tPersistMs ?? 9,
    tTotalMs: overrides.tTotalMs ?? 601,
  };
}

function createRecord(overrides: Partial<ApiInvocation> = {}): ApiInvocation {
  return {
    id: 501,
    invokeId: "invoke-dashboard-drawer",
    occurredAt: "2026-04-06T10:24:37Z",
    createdAt: "2026-04-06T10:24:37Z",
    status: "completed",
    source: "proxy",
    routeMode: "forward_proxy",
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
    tReqReadMs: 14,
    tReqParseMs: 8,
    tUpstreamConnectMs: 136,
    tUpstreamTtfbMs: 98,
    tUpstreamStreamMs: 324,
    tRespParseMs: 12,
    tPersistMs: 9,
    tTotalMs: 601,
    ...overrides,
  };
}

function createSelection(
  recordOverrides: Partial<ApiInvocation> = {},
): DashboardWorkingConversationInvocationSelection {
  const record = createRecord(recordOverrides);
  const preview = createPreview({
    id: record.id,
    invokeId: record.invokeId,
    occurredAt: record.occurredAt,
    status: record.status ?? "completed",
    upstreamAccountId: record.upstreamAccountId ?? null,
    upstreamAccountName: record.upstreamAccountName ?? null,
    proxyDisplayName: record.proxyDisplayName ?? null,
    routeMode: record.routeMode ?? null,
    model: record.model ?? null,
    requestModel: record.requestModel ?? null,
    responseModel: record.responseModel ?? null,
    endpoint: record.endpoint ?? null,
    totalTokens: record.totalTokens ?? 0,
    cost: record.cost ?? null,
    inputTokens: record.inputTokens,
    outputTokens: record.outputTokens,
    cacheInputTokens: record.cacheInputTokens,
    reasoningTokens: record.reasoningTokens,
    reasoningEffort: record.reasoningEffort,
    responseContentEncoding: record.responseContentEncoding,
    requestedServiceTier: record.requestedServiceTier,
    serviceTier: record.serviceTier,
    tReqReadMs: record.tReqReadMs,
    tReqParseMs: record.tReqParseMs,
    tUpstreamConnectMs: record.tUpstreamConnectMs,
    tUpstreamTtfbMs: record.tUpstreamTtfbMs,
    tUpstreamStreamMs: record.tUpstreamStreamMs,
    tRespParseMs: record.tRespParseMs,
    tPersistMs: record.tPersistMs,
    tTotalMs: record.tTotalMs,
    failureClass: record.failureClass ?? null,
    errorMessage: record.errorMessage,
    failureKind: record.failureKind,
    isActionable: record.isActionable,
  });

  return {
    slotKind: "current",
    conversationSequenceId: "WC-AB364A",
    promptCacheKey: "019d5ea7-519d-7312-a2e8-ef07abb7c09f",
    invocation: {
      preview,
      record,
      displayStatus:
        record.status === "running"
          ? "running"
          : record.status === "pending"
            ? "pending"
            : record.status === "failed"
              ? "failed"
              : record.status === "interrupted"
                ? "interrupted"
                : "success",
      occurredAtEpoch: Date.parse(record.occurredAt),
      isInFlight: record.status === "running" || record.status === "pending",
      isTerminal: record.status !== "running" && record.status !== "pending",
      tone:
        record.status === "running"
          ? "running"
          : record.status === "pending"
            ? "pending"
            : record.status === "failed" || record.status === "interrupted"
              ? "error"
              : "success",
    },
  };
}

function createRecordsResponse(records: ApiInvocation[]): InvocationRecordsResponse {
  return {
    snapshotId: 1,
    total: records.length,
    page: 1,
    pageSize: 1,
    records,
  };
}

function createWorkflowDetailFixture(
  record: ApiInvocation,
  overrides: Partial<ApiInvocationWorkflowDetailResponse> = {},
): ApiInvocationWorkflowDetailResponse {
  const requestModel = record.requestModel ?? record.model ?? "gpt-5.4";
  const responseModel = record.responseModel ?? record.model ?? requestModel;
  const routeMode = record.routeMode ?? "forward_proxy";
  const finalStatus = record.status ?? "completed";
  const base: ApiInvocationWorkflowDetailResponse = {
    hero: {
      recordId: record.id,
      invokeId: record.invokeId,
      promptCacheKey: "019d5ea7-519d-7312-a2e8-ef07abb7c09f",
      routeMode,
      endpoint: record.endpoint ?? "/v1/responses",
      requestModel,
      responseModel,
      finalStatus,
      failureClass: record.failureClass ?? null,
      downstreamStatusCode: record.downstreamStatusCode ?? null,
      upstreamAccountId: record.upstreamAccountId ?? 42,
      upstreamAccountName: record.upstreamAccountName ?? "pool-alpha@example.com",
      totalDurationMs: record.tTotalMs ?? 601,
      timelineAttemptCount: 1,
      poolAttemptCount: record.poolAttemptCount ?? 1,
      totalTokens: record.totalTokens ?? 240,
      cost: record.cost ?? 0.0182,
      occurredAt: record.occurredAt,
    },
    timeline: [
      {
        blockId: "route-1",
        kind: "routingDecision",
        occurredAt: record.occurredAt,
        title: "Pool route selected",
        subtitle: `${routeMode} ${record.endpoint ?? "/v1/responses"}`,
        status: "completed",
        detail: {
          routeMode,
          poolAttemptCount: record.poolAttemptCount ?? 1,
        },
      },
      {
        blockId: "attempt-1",
        kind: "attempt",
        occurredAt: record.occurredAt,
        title: "Attempt 1",
        status: finalStatus,
        attempt: {
          synthetic: false,
          attemptId: "attempt-1",
          occurredAt: record.occurredAt,
          endpoint: record.endpoint ?? "/v1/responses",
          upstreamAccountId: record.upstreamAccountId ?? 42,
          upstreamAccountName: record.upstreamAccountName ?? "pool-alpha@example.com",
          requestModel,
          responseModel,
          attemptIndex: 1,
          distinctAccountIndex: 1,
          sameAccountRetryIndex: 1,
          status: finalStatus,
          phase: finalStatus === "failed" ? "streaming" : "completed",
          httpStatus: 200,
          connectLatencyMs: record.tUpstreamConnectMs ?? 136,
          firstByteLatencyMs: record.tUpstreamTtfbMs ?? 98,
          streamLatencyMs: record.tUpstreamStreamMs ?? 324,
          upstreamRequestId: "req_dashboard_drawer",
          requestSummary: {
            endpoint: record.endpoint ?? "/v1/responses",
            transport: record.transport ?? "http",
            requestModel,
            responseModel,
            requestedServiceTier: record.requestedServiceTier ?? "priority",
            reasoningEffort: record.reasoningEffort ?? "high",
            routing: {
              routeMode,
              proxyDisplayName: record.proxyDisplayName ?? "tokyo-edge-01",
              promptCacheKey: "019d5ea7-519d-7312-a2e8-ef07abb7c09f",
            },
            headers: {
              userAgent: "monitor-ui/1.0",
              xForwardedFor: "203.0.113.10",
            },
            bodyCapture: {
              availableAtInvocationLevel: true,
              size: 2219,
              detailLevel: "full",
            },
            compactionRequestKind: "remote_v2",
          },
          responseSummary: {
            status: finalStatus,
            serviceTier: record.serviceTier ?? "priority",
            failureKind: record.failureKind ?? null,
            responseContentEncoding: record.responseContentEncoding ?? "gzip",
            compactionResponseKind: "remote_v2",
            toolCalls: ["web_search_preview", "function:search_docs"],
            outputItems: 2,
            headers: {
              contentEncoding: record.responseContentEncoding ?? "gzip",
              upstreamRequestId: "req_dashboard_drawer",
            },
            responseBodyCapture: {
              size: 1625,
              detailLevel: "full",
            },
          },
        },
      },
    ],
    reconstructed: false,
    partial: false,
  };

  return {
    ...base,
    ...overrides,
    hero: { ...base.hero, ...overrides.hero },
    timeline: overrides.timeline ?? base.timeline,
  };
}

function createResponseBodyFixture(
  overrides: Partial<ApiInvocationResponseBodyResponse> = {},
): ApiInvocationResponseBodyResponse {
  return {
    available: true,
    bodyText: JSON.stringify({
      id: "resp_dashboard_drawer",
      status: "completed",
      model: "gpt-5.4",
      output: [{ type: "message", role: "assistant" }],
    }),
    captureSource: "raw_file",
    bodySize: 1625,
    bodyTruncated: false,
    detailLevel: "full",
    headers: { contentEncoding: "gzip" },
    routing: { forwardedChunkCount: 12 },
    ...overrides,
  };
}

describe("DashboardInvocationDetailDrawer model details", () => {
  it("shows separate request and response model labels for mismatched records", async () => {
    const selection = createSelection({
      model: "gpt-5.5",
      requestModel: "gpt-5.4",
      responseModel: "gpt-5.5",
    });
    const record = selection.invocation.record;
    apiMocks.fetchInvocationRecords.mockResolvedValue(createRecordsResponse([record]));
    apiMocks.fetchInvocationWorkflowDetail.mockResolvedValue(createWorkflowDetailFixture(record));

    render(<DashboardInvocationDetailDrawer open selection={selection} onClose={() => {}} />);

    await waitFor(() =>
      Boolean(
        document.body.textContent?.includes("工作流时间线") &&
          document.body.textContent?.includes("gpt-5.4") &&
          document.body.textContent?.includes("gpt-5.5"),
      ),
    );
  });
});

describe("DashboardInvocationDetailDrawer", () => {
  it("loads the full record by invoke id and keeps the account action clickable", async () => {
    const onOpenUpstreamAccount = vi.fn();
    const record = createRecord({ routeMode: "pool" });
    apiMocks.fetchInvocationRecords.mockResolvedValue(createRecordsResponse([record]));
    apiMocks.fetchInvocationWorkflowDetail.mockResolvedValue(createWorkflowDetailFixture(record));

    render(
      <DashboardInvocationDetailDrawer
        open
        selection={createSelection({ routeMode: "pool" })}
        onClose={() => undefined}
        onOpenUpstreamAccount={onOpenUpstreamAccount}
      />,
    );

    await waitFor(
      () =>
        document.body.querySelector('button[title="pool-alpha@example.com"]') instanceof
        HTMLButtonElement,
    );

    expect(apiMocks.fetchInvocationRecords).toHaveBeenCalledWith({
      requestId: "invoke-dashboard-drawer",
      pageSize: 1,
      sortBy: "occurredAt",
      sortOrder: "desc",
    });

    const accountButton = document.body.querySelector('button[title="pool-alpha@example.com"]');
    if (!(accountButton instanceof HTMLButtonElement)) {
      throw new Error("missing account button");
    }

    act(() => {
      accountButton.click();
    });

    expect(onOpenUpstreamAccount).toHaveBeenCalledWith(42, "pool-alpha@example.com");
  });

  it("shows the bare conversation hash in the drawer header while keeping prompt cache key visible", async () => {
    apiMocks.fetchInvocationRecords.mockResolvedValue(createRecordsResponse([createRecord()]));

    render(
      <DashboardInvocationDetailDrawer
        open
        selection={createSelection()}
        onClose={() => undefined}
      />,
    );

    await waitFor(
      () =>
        document.body.querySelector('[data-testid="dashboard-invocation-detail-drawer"]') != null,
    );

    const drawer = document.body.querySelector(
      '[data-testid="dashboard-invocation-detail-drawer"]',
    );
    if (!(drawer instanceof HTMLElement)) {
      throw new Error("missing invocation drawer header");
    }

    expect(drawer.textContent ?? "").toContain("AB364A");
    expect(drawer.textContent ?? "").not.toContain("WC-AB364A");
    expect(drawer.textContent ?? "").toContain("019d5ea7-519d-7312-a2e8-ef07abb7c09f");

    const drawerBody = drawer.closest("section")?.querySelector(".drawer-body");
    expect(drawerBody?.classList.contains("overflow-x-hidden")).toBe(true);
    expect(drawerBody?.classList.contains("overflow-y-auto")).toBe(true);
    expect(drawerBody?.textContent ?? "").toContain("调用详情");
    expect(drawerBody?.textContent ?? "").toContain("工作流时间线");
  });

  it("renders interrupted status with the dedicated recovery badge", async () => {
    const record = createRecord({
      status: "interrupted",
      failureClass: "service_failure",
      failureKind: "proxy_interrupted",
      errorMessage: "proxy request was interrupted before completion and was recovered on startup",
    });
    apiMocks.fetchInvocationRecords.mockResolvedValue(createRecordsResponse([record]));
    apiMocks.fetchInvocationWorkflowDetail.mockResolvedValue(createWorkflowDetailFixture(record));

    render(
      <DashboardInvocationDetailDrawer
        open
        selection={createSelection(record)}
        onClose={() => undefined}
      />,
    );

    await waitFor(() => (document.body.textContent ?? "").includes("table.status.interrupted"));

    const text = document.body.textContent ?? "";
    expect(text).toContain("table.status.interrupted");
    expect(text).not.toContain("table.status.failed");
  });

  it("shows the empty state inside the drawer when the full lookup returns no record", async () => {
    apiMocks.fetchInvocationRecords.mockResolvedValue(createRecordsResponse([]));

    render(
      <DashboardInvocationDetailDrawer
        open
        selection={createSelection()}
        onClose={() => undefined}
      />,
    );

    await waitFor(
      () =>
        document.body.querySelector('[data-testid="dashboard-invocation-detail-empty"]') != null,
    );

    expect(
      document.body.querySelector('[data-testid="dashboard-invocation-detail-error"]'),
    ).toBeNull();
  });

  it("shows the lookup error inside the drawer when the full record request fails", async () => {
    apiMocks.fetchInvocationRecords.mockRejectedValue(new Error("lookup failed"));

    render(
      <DashboardInvocationDetailDrawer
        open
        selection={createSelection()}
        onClose={() => undefined}
      />,
    );

    await waitFor(
      () =>
        document.body.querySelector('[data-testid="dashboard-invocation-detail-error"]') != null,
    );

    expect(document.body.textContent ?? "").toContain("lookup failed");
  });

  it("loads abnormal response details only for abnormal records", async () => {
    const record = createRecord({
      status: "failed",
      failureClass: "service_failure",
      errorMessage: "upstream exploded",
      failureKind: "downstream_closed",
    });
    apiMocks.fetchInvocationRecords.mockResolvedValue(createRecordsResponse([record]));
    apiMocks.fetchInvocationWorkflowDetail.mockResolvedValue(createWorkflowDetailFixture(record));
    apiMocks.fetchInvocationResponseBody.mockResolvedValue({
      available: true,
      bodyText: '{"error":"preview","trace":"full"}',
    });

    render(
      <DashboardInvocationDetailDrawer
        open
        selection={createSelection(record)}
        onClose={() => undefined}
      />,
    );

    await waitFor(() => (document.body.textContent ?? "").includes("工作流时间线"));

    const responseBodyButton = Array.from(document.querySelectorAll("button")).find((button) =>
      button.textContent?.includes("响应体"),
    );
    expect(responseBodyButton).toBeTruthy();

    act(() => {
      responseBodyButton?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });

    await waitFor(() => apiMocks.fetchInvocationResponseBody.mock.calls.length > 0);
    expect(apiMocks.fetchInvocationWorkflowDetail).toHaveBeenCalledWith(501);
    expect(apiMocks.fetchInvocationResponseBody).toHaveBeenCalledWith(501);
  });

  it("does not request DB-backed abnormal details for transient live records", async () => {
    const liveRecord = createRecord({
      id: 0,
      status: "failed",
      failureClass: "service_failure",
      errorMessage: "upstream exploded before placeholder flush",
    });
    apiMocks.fetchInvocationRecords.mockResolvedValue(createRecordsResponse([liveRecord]));

    render(
      <DashboardInvocationDetailDrawer
        open
        selection={createSelection(liveRecord)}
        onClose={() => undefined}
      />,
    );

    await waitFor(() => (document.body.textContent ?? "").includes("调用未落盘"));

    expect(apiMocks.fetchInvocationWorkflowDetail).not.toHaveBeenCalled();
    expect(apiMocks.fetchInvocationResponseBody).not.toHaveBeenCalled();
  });
});
