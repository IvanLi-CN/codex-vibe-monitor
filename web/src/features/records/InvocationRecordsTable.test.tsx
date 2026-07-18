/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  ApiInvocation,
  ApiInvocationRequestBodyResponse,
  ApiInvocationResponseBodyResponse,
  ApiInvocationWorkflowDetailResponse,
} from "../../lib/api";
import { InvocationRecordsTable } from "./InvocationRecordsTable";

const { apiMocks } = vi.hoisted(() => ({
  apiMocks: {
    fetchInvocationWorkflowDetail: vi.fn(),
    fetchInvocationRequestBody: vi.fn(),
    fetchInvocationResponseBody: vi.fn(),
  },
}));

vi.mock("../../lib/api", async () => {
  const actual = await vi.importActual<typeof import("../../lib/api")>("../../lib/api");
  return {
    ...actual,
    fetchInvocationWorkflowDetail: apiMocks.fetchInvocationWorkflowDetail,
    fetchInvocationRequestBody: apiMocks.fetchInvocationRequestBody,
    fetchInvocationResponseBody: apiMocks.fetchInvocationResponseBody,
  };
});

vi.mock("../../i18n", () => ({
  useTranslation: () => ({
    locale: "zh",
    t: (key: string, params?: Record<string, string | number>) => {
      if (params?.error) return `${key}: ${params.error}`;
      if (params?.value) return `${key}: ${params.value}`;
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
  apiMocks.fetchInvocationWorkflowDetail.mockReset();
  apiMocks.fetchInvocationRequestBody.mockReset();
  apiMocks.fetchInvocationResponseBody.mockReset();
  apiMocks.fetchInvocationWorkflowDetail.mockImplementation(async (id: number) =>
    createWorkflowDetailFixture(createRecord({ id })),
  );
  apiMocks.fetchInvocationRequestBody.mockResolvedValue(createRequestBodyFixture());
  apiMocks.fetchInvocationResponseBody.mockResolvedValue(createResponseBodyFixture());
});

afterEach(() => {
  act(() => {
    root?.unmount();
  });
  host?.remove();
  host = null;
  root = null;
  vi.useRealTimers();
});

function render(ui: React.ReactNode) {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => {
    root?.render(ui);
  });
}

function createRecord(overrides: Partial<ApiInvocation> = {}): ApiInvocation {
  return {
    id: 1,
    invokeId: "invoke-1",
    occurredAt: "2026-03-10T00:00:00Z",
    createdAt: "2026-03-10T00:00:00Z",
    status: "success",
    source: "proxy",
    proxyDisplayName: "jp-relay-01",
    model: "gpt-5.4",
    requestModel: "gpt-5.4",
    responseModel: "gpt-5.4",
    endpoint: "/v1/responses",
    inputTokens: 2400,
    cacheInputTokens: 400,
    outputTokens: 320,
    reasoningTokens: 88,
    reasoningEffort: "high",
    totalTokens: 2720,
    cost: 0.1234,
    requesterIp: "203.0.113.10",
    promptCacheKey: "pck-test",
    requestedServiceTier: "priority",
    serviceTier: "priority",
    billingServiceTier: "priority",
    responseContentEncoding: "gzip, br",
    tReqReadMs: 12,
    tReqParseMs: 30,
    tUpstreamConnectMs: 55,
    tUpstreamTtfbMs: 142,
    tUpstreamStreamMs: 480,
    tRespParseMs: 20,
    tPersistMs: 12,
    tTotalMs: 741,
    ...overrides,
  };
}

function createWorkflowDetailFixture(
  record: ApiInvocation,
  overrides: Partial<ApiInvocationWorkflowDetailResponse> = {},
): ApiInvocationWorkflowDetailResponse {
  const requestModel = record.requestModel ?? record.responseModel ?? record.model ?? "gpt-5.4";
  const responseModel = record.responseModel ?? record.requestModel ?? record.model ?? requestModel;
  const routeMode = record.routeMode ?? "pool";
  const attemptStatus = record.status ?? "success";
  const base: ApiInvocationWorkflowDetailResponse = {
    hero: {
      recordId: record.id,
      invokeId: record.invokeId,
      promptCacheKey: record.promptCacheKey ?? "pck-test",
      routeMode,
      endpoint: record.endpoint ?? "/v1/responses",
      requestModel,
      responseModel,
      finalStatus: attemptStatus,
      failureClass: record.failureClass ?? null,
      downstreamStatusCode: record.downstreamStatusCode ?? null,
      upstreamAccountId: record.upstreamAccountId ?? 42,
      upstreamAccountName: record.upstreamAccountName ?? "pool-account-42",
      totalDurationMs: record.tTotalMs ?? 741,
      timelineAttemptCount: 1,
      poolAttemptCount: record.poolAttemptCount ?? 1,
      totalTokens: record.totalTokens ?? 2720,
      cost: record.cost ?? 0.1234,
      occurredAt: record.occurredAt,
    },
    timeline: [
      {
        blockId: `route-${record.id ?? 1}`,
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
        blockId: `attempt-${record.id ?? 1}`,
        kind: "attempt",
        occurredAt: record.occurredAt,
        title: "Attempt 1",
        status: attemptStatus,
        attempt: {
          synthetic: false,
          attemptId: `attempt-${record.id ?? 1}`,
          occurredAt: record.occurredAt,
          endpoint: record.endpoint ?? "/v1/responses",
          upstreamAccountId: record.upstreamAccountId ?? 42,
          upstreamAccountName: record.upstreamAccountName ?? "pool-account-42",
          requestModel,
          responseModel,
          attemptIndex: 1,
          distinctAccountIndex: 1,
          sameAccountRetryIndex: 1,
          status: attemptStatus,
          phase: attemptStatus === "failed" ? "streaming" : "completed",
          httpStatus: 200,
          firstByteLatencyMs: record.tUpstreamTtfbMs ?? 142,
          streamLatencyMs: record.tTotalMs ?? 741,
          upstreamRequestId: record.upstreamRequestId ?? "req_records_workflow",
          requestSummary: {
            endpoint: record.endpoint ?? "/v1/responses",
            transport: record.transport ?? "http",
            requestModel,
            responseModel,
            requestedServiceTier: record.requestedServiceTier ?? "priority",
            reasoningEffort: record.reasoningEffort ?? "high",
            routing: {
              routeMode,
              proxyDisplayName: record.proxyDisplayName ?? "jp-relay-01",
              promptCacheKey: record.promptCacheKey ?? "pck-test",
            },
            headers: {
              userAgent: "monitor-ui/1.0",
              xForwardedFor: record.requesterIp ?? "203.0.113.10",
            },
            bodyCapture: {
              availableAtInvocationLevel: true,
              size: 3568,
              detailLevel: "full",
            },
            compactionRequestKind: "remote_v2",
          },
          responseSummary: {
            status: attemptStatus,
            serviceTier: record.serviceTier ?? record.requestedServiceTier ?? "priority",
            failureKind: record.failureKind ?? null,
            responseContentEncoding: record.responseContentEncoding ?? "gzip",
            compactionResponseKind: "remote_v2",
            toolCalls: ["web_search", "search_docs"],
            outputItems: 3,
            headers: {
              contentEncoding: record.responseContentEncoding ?? "gzip",
              upstreamRequestId: record.upstreamRequestId ?? "req_records_workflow",
            },
            responseBodyCapture: {
              size: 7271,
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

function createRequestBodyFixture(
  overrides: Partial<ApiInvocationRequestBodyResponse> = {},
): ApiInvocationRequestBodyResponse {
  return {
    available: true,
    bodyText: JSON.stringify({ model: "gpt-5.4", input: [{ role: "user", content: "hello" }] }),
    captureSource: "raw_file",
    bodySize: 3568,
    bodyTruncated: false,
    detailLevel: "full",
    headers: { userAgent: "monitor-ui/1.0" },
    routing: { routeMode: "pool", proxyDisplayName: "jp-relay-01" },
    ...overrides,
  };
}

function createResponseBodyFixture(
  overrides: Partial<ApiInvocationResponseBodyResponse> = {},
): ApiInvocationResponseBodyResponse {
  return {
    available: true,
    bodyText: JSON.stringify({ id: "resp_records_test", status: "completed" }),
    captureSource: "raw_file",
    bodySize: 7271,
    bodyTruncated: false,
    detailLevel: "full",
    headers: { contentEncoding: "gzip" },
    routing: { forwardedChunkCount: 12 },
    ...overrides,
  };
}

function clickFirstToggle() {
  const button = host?.querySelector(
    'button[aria-label="records.table.showDetails"]',
  ) as HTMLButtonElement | null;
  expect(button).not.toBeNull();
  act(() => {
    button?.click();
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

async function waitFor(check: () => boolean, timeoutMs = 500) {
  const startedAt = Date.now();
  while (Date.now() - startedAt < timeoutMs) {
    await flushAsyncWork();
    if (check()) return;
  }
}

describe("InvocationRecordsTable", () => {
  it("renders the WS transport badge for websocket records", () => {
    render(
      <InvocationRecordsTable
        focus="network"
        isLoading={false}
        records={[
          createRecord({
            id: 1,
            invokeId: "invoke-ws-transport",
            transport: "websocket",
          }),
        ]}
      />,
    );

    const badges = host?.querySelectorAll('[data-testid="invocation-transport-badge"]');
    expect((badges?.length ?? 0) > 0).toBe(true);
    expect(
      Array.from(badges ?? []).every(
        (badge) =>
          badge.querySelector('[aria-hidden="true"]')?.textContent === "WS" &&
          badge.textContent?.includes("WebSocket transport") &&
          badge.getAttribute("title") === "WebSocket",
      ),
    ).toBe(true);
  });

  it("does not render the WS transport badge for http or legacy records", () => {
    render(
      <InvocationRecordsTable
        focus="network"
        isLoading={false}
        records={[
          createRecord({
            id: 2,
            invokeId: "invoke-http-transport",
            transport: "http",
          }),
          createRecord({
            id: 3,
            invokeId: "invoke-legacy-transport",
            transport: null,
          }),
        ]}
      />,
    );

    expect(host?.querySelectorAll('[data-testid="invocation-transport-badge"]')).toHaveLength(0);
  });

  it("renders image endpoints as shared chips while keeping non-image unknown endpoints on the raw fallback", () => {
    render(
      <InvocationRecordsTable
        focus="network"
        isLoading={false}
        records={[
          createRecord({
            id: 10,
            invokeId: "invoke-image-gen",
            endpoint: "/v1/images/generations",
            imageIntent: "yes",
            model: "gpt-image-1",
          }),
          createRecord({
            id: 11,
            invokeId: "invoke-image-edit",
            endpoint: "/v1/images/edits",
            imageIntent: "direct_image",
            model: "gpt-image-1",
          }),
          createRecord({
            id: 12,
            invokeId: "invoke-image-generic",
            endpoint: "/v1/images/variations",
            model: "gpt-image-1",
          }),
          createRecord({
            id: 13,
            invokeId: "invoke-raw-experimental",
            endpoint: "/v1/responses/experimental",
          }),
        ]}
      />,
    );

    expect(host?.querySelector('[data-endpoint-kind="image_gen"]')).not.toBeNull();
    expect(host?.querySelector('[data-endpoint-kind="image_edit"]')).not.toBeNull();
    expect(host?.querySelector('[data-endpoint-kind="image"]')).not.toBeNull();
    expect(host?.querySelector('[data-endpoint-kind="raw"]')).not.toBeNull();
    expect(host?.textContent ?? "").toContain("table.endpoint.imageGenBadge");
    expect(host?.textContent ?? "").toContain("table.endpoint.imageEditBadge");
    expect(host?.textContent ?? "").toContain("table.endpoint.imageBadge");
    expect(host?.textContent ?? "").toContain("/v1/responses/experimental");
    expect(host?.textContent ?? "").not.toContain("/v1/images/generations");
    expect(host?.textContent ?? "").not.toContain("/v1/images/edits");
    expect(host?.textContent ?? "").not.toContain("/v1/images/variations");
  });

  it("treats completed rows as success in the shared records table", () => {
    render(
      <InvocationRecordsTable
        focus="token"
        isLoading={false}
        records={[
          createRecord({
            status: "completed",
          }),
        ]}
      />,
    );

    const text = host?.textContent ?? "";
    expect(text).toContain("table.status.success");
    expect(text).not.toContain("completed");
  });

  it("renders warning_success rows with the dedicated warning success badge label", () => {
    render(
      <InvocationRecordsTable
        focus="token"
        isLoading={false}
        records={[
          createRecord({
            status: "warning_success",
            failureClass: "none",
            failureKind: "downstream_closed",
            downstreamErrorMessage:
              "[downstream_closed] downstream closed while streaming upstream response",
          }),
        ]}
      />,
    );

    const text = host?.textContent ?? "";
    expect(text).toContain("table.status.warningSuccess");
    expect(text).not.toContain("warning_success");
  });

  it("renders interrupted rows with the dedicated interrupted badge label", () => {
    render(
      <InvocationRecordsTable
        focus="exception"
        isLoading={false}
        records={[
          createRecord({
            status: "interrupted",
            failureClass: "service_failure",
            failureKind: "proxy_interrupted",
            errorMessage:
              "proxy request was interrupted before completion and was recovered on startup",
          }),
        ]}
      />,
    );

    const text = host?.textContent ?? "";
    expect(text).toContain("table.status.interrupted");
    expect(text).not.toContain("table.status.failed");
  });

  it("shows the routing icon when request and response models differ", () => {
    render(
      <InvocationRecordsTable
        focus="token"
        isLoading={false}
        records={[
          createRecord({
            requestModel: "gpt-5.4",
            responseModel: "gpt-5.5",
            model: "gpt-5.5",
          }),
        ]}
      />,
    );

    expect(
      host?.querySelector('[data-testid="invocation-records-model-routing-indicator"]'),
    ).not.toBeNull();
  });

  it("expands into the workflow detail panel and keeps response-model fallback visible", async () => {
    const record = createRecord({
      requestModel: undefined,
      responseModel: undefined,
      model: "gpt-5-legacy",
    });
    apiMocks.fetchInvocationWorkflowDetail.mockResolvedValueOnce(
      createWorkflowDetailFixture(record, {
        hero: {
          requestModel: "gpt-5-legacy",
          responseModel: "gpt-5-legacy",
        },
      }),
    );

    render(<InvocationRecordsTable focus="token" isLoading={false} records={[record]} />);

    clickFirstToggle();
    await waitFor(() => (host?.textContent ?? "").includes("工作流时间线"));

    expect(apiMocks.fetchInvocationWorkflowDetail).toHaveBeenCalledWith(1);
    expect(host?.textContent ?? "").toContain("gpt-5-legacy");
  });

  it("renders failed workflow detail in the expanded row and exposes the full-details drawer entry", async () => {
    const record = createRecord({
      status: "failed",
      failureClass: "service_failure",
      failureKind: "upstream_stream_error",
      errorMessage: "[upstream_stream_error] upstream reset",
      detailLevel: "structured_only",
      detailPrunedAt: "2026-03-11T08:09:10Z",
      detailPruneReason: "success_over_30d",
    });
    apiMocks.fetchInvocationWorkflowDetail.mockResolvedValueOnce(
      createWorkflowDetailFixture(record, {
        hero: {
          finalStatus: "failed",
          failureClass: "service_failure",
        },
      }),
    );

    render(<InvocationRecordsTable focus="token" isLoading={false} records={[record]} />);

    clickFirstToggle();
    await waitFor(
      () => host?.querySelector('[data-testid="records-expanded-detail-panel"]') != null,
    );

    expect(host?.querySelector('[data-testid="records-detail-summary-strip"]')).not.toBeNull();
    expect(host?.textContent ?? "").toContain("table.responseBody.openFullDetails");
    expect(host?.textContent ?? "").toContain("工作流时间线");
  });

  it("opens the full-details drawer and reuses the workflow detail panel", async () => {
    const record = createRecord({
      status: "failed",
      failureClass: "service_failure",
      failureKind: "downstream_closed",
      errorMessage: "preview only",
    });
    apiMocks.fetchInvocationWorkflowDetail.mockImplementation(async () =>
      createWorkflowDetailFixture(record),
    );

    render(<InvocationRecordsTable focus="exception" isLoading={false} records={[record]} />);

    clickFirstToggle();
    await waitFor(() => (host?.textContent ?? "").includes("table.responseBody.openFullDetails"));

    const button = Array.from(document.body.querySelectorAll("button")).find(
      (candidate): candidate is HTMLButtonElement =>
        candidate instanceof HTMLButtonElement &&
        candidate.textContent === "table.responseBody.openFullDetails",
    );
    expect(button).not.toBeNull();

    act(() => {
      button?.click();
    });

    await waitFor(
      () => document.body.textContent?.includes("records.table.fullDetails.title") ?? false,
    );
    await waitFor(() => (apiMocks.fetchInvocationWorkflowDetail.mock.calls.length ?? 0) >= 2);

    expect(document.body.textContent ?? "").toContain(record.invokeId);
    expect(document.body.textContent ?? "").toContain("工作流时间线");
  });

  it("does not fetch workflow detail for transient live records", async () => {
    render(
      <InvocationRecordsTable
        focus="exception"
        isLoading={false}
        records={[
          createRecord({
            id: 0,
            status: "failed",
            failureClass: "service_failure",
            errorMessage: "preview pending placeholder flush",
          }),
        ]}
      />,
    );

    clickFirstToggle();
    await flushAsyncWork();

    expect(apiMocks.fetchInvocationWorkflowDetail).not.toHaveBeenCalled();
    expect(apiMocks.fetchInvocationResponseBody).not.toHaveBeenCalled();
    expect(host?.textContent ?? "").toContain("调用未落盘");
    expect(host?.textContent ?? "").not.toContain("table.responseBody.openFullDetails");
  });

  it("uses downstream-facing diagnostics as the collapsed exception summary when upstream is empty", () => {
    render(
      <InvocationRecordsTable
        focus="exception"
        isLoading={false}
        records={[
          createRecord({
            id: 33,
            invokeId: "invoke-downstream-summary",
            status: "failed",
            failureClass: "client_abort",
            failureKind: "downstream_closed",
            errorMessage: undefined,
            downstreamStatusCode: 200,
            downstreamErrorMessage:
              "[downstream_closed] downstream closed while streaming upstream response",
          }),
        ]}
      />,
    );

    expect(host?.textContent ?? "").toContain(
      "[downstream_closed] downstream closed while streaming upstream response",
    );
  });

  it('renders unknown actionable state as a fallback instead of "no"', () => {
    render(
      <InvocationRecordsTable
        focus="exception"
        isLoading={false}
        records={[
          createRecord({
            id: 6,
            status: "failed",
            failureClass: "service_failure",
            failureKind: "upstream_timeout",
            isActionable: undefined,
            errorMessage: "upstream timeout",
          }),
        ]}
      />,
    );

    const text = host?.textContent ?? "";
    expect(text).toContain("records.table.exception.actionable");
    expect(text).not.toContain("records.table.exception.actionableNo");
    expect(text).toContain("—");
  });
});
