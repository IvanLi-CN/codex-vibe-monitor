/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import type {
  ApiInvocation,
  InvocationRecordsResponse,
  PromptCacheConversationInvocationPreview,
} from "../lib/api";
import type { DashboardWorkingConversationInvocationSelection } from "../lib/dashboardWorkingConversations";
import { DashboardInvocationDetailDrawer } from "./DashboardInvocationDetailDrawer";

const { apiMocks } = vi.hoisted(() => ({
  apiMocks: {
    fetchInvocationPoolAttempts: vi.fn(),
    fetchInvocationRecordDetail: vi.fn(),
    fetchInvocationRecords: vi.fn(),
    fetchInvocationResponseBody: vi.fn(),
  },
}));

vi.mock("../lib/api", async () => {
  const actual =
    await vi.importActual<typeof import("../lib/api")>("../lib/api");
  return {
    ...actual,
    fetchInvocationPoolAttempts: apiMocks.fetchInvocationPoolAttempts,
    fetchInvocationRecordDetail: apiMocks.fetchInvocationRecordDetail,
    fetchInvocationRecords: apiMocks.fetchInvocationRecords,
    fetchInvocationResponseBody: apiMocks.fetchInvocationResponseBody,
  };
});

vi.mock("../i18n", () => ({
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

afterEach(() => {
  act(() => {
    root?.unmount();
  });
  host?.remove();
  host = null;
  root = null;
  vi.restoreAllMocks();
  apiMocks.fetchInvocationPoolAttempts.mockReset();
  apiMocks.fetchInvocationRecordDetail.mockReset();
  apiMocks.fetchInvocationRecords.mockReset();
  apiMocks.fetchInvocationResponseBody.mockReset();
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
    totalTokens: overrides.totalTokens ?? 240,
    cost: overrides.cost ?? 0.0182,
    proxyDisplayName: overrides.proxyDisplayName ?? "tokyo-edge-01",
    upstreamAccountId: overrides.upstreamAccountId ?? 42,
    upstreamAccountName:
      overrides.upstreamAccountName ?? "pool-alpha@example.com",
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

function createRecordsResponse(
  records: ApiInvocation[],
): InvocationRecordsResponse {
  return {
    snapshotId: 1,
    total: records.length,
    page: 1,
    pageSize: 1,
    records,
  };
}

describe("DashboardInvocationDetailDrawer", () => {
  it("loads the full record by invoke id and keeps the account action clickable", async () => {
    const onOpenUpstreamAccount = vi.fn();
    apiMocks.fetchInvocationRecords.mockResolvedValue(
      createRecordsResponse([createRecord({ routeMode: "pool" })]),
    );
    apiMocks.fetchInvocationPoolAttempts.mockResolvedValue([]);

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
        document.body.querySelector(
          'button[title="pool-alpha@example.com"]',
        ) instanceof HTMLButtonElement,
    );

    expect(apiMocks.fetchInvocationRecords).toHaveBeenCalledWith({
      requestId: "invoke-dashboard-drawer",
      pageSize: 1,
      sortBy: "occurredAt",
      sortOrder: "desc",
    });

    const accountButton = document.body.querySelector(
      'button[title="pool-alpha@example.com"]',
    );
    if (!(accountButton instanceof HTMLButtonElement)) {
      throw new Error("missing account button");
    }

    act(() => {
      accountButton.click();
    });

    expect(onOpenUpstreamAccount).toHaveBeenCalledWith(
      42,
      "pool-alpha@example.com",
    );
  });

  it(
    "shows the bare conversation hash in the drawer header while keeping prompt cache key visible",
    async () => {
      apiMocks.fetchInvocationRecords.mockResolvedValue(
        createRecordsResponse([createRecord()]),
      );

      render(
        <DashboardInvocationDetailDrawer
          open
          selection={createSelection()}
          onClose={() => undefined}
        />,
      );

      await waitFor(
        () =>
          document.body.querySelector(
            '[data-testid="dashboard-invocation-detail-drawer"]',
          ) != null,
      );

      const drawer = document.body.querySelector(
        '[data-testid="dashboard-invocation-detail-drawer"]',
      );
      if (!(drawer instanceof HTMLElement)) {
        throw new Error("missing invocation drawer header");
      }

      expect(drawer.textContent ?? "").toContain("AB364A");
      expect(drawer.textContent ?? "").not.toContain("WC-AB364A");
      expect(drawer.textContent ?? "").toContain(
        "019d5ea7-519d-7312-a2e8-ef07abb7c09f",
      );
    },
  );

  it("renders interrupted status with the dedicated recovery badge", async () => {
    apiMocks.fetchInvocationRecords.mockResolvedValue(
      createRecordsResponse([
        createRecord({
          status: "interrupted",
          failureClass: "service_failure",
          failureKind: "proxy_interrupted",
          errorMessage:
            "proxy request was interrupted before completion and was recovered on startup",
        }),
      ]),
    );
    apiMocks.fetchInvocationRecordDetail.mockResolvedValue({
      id: 501,
      abnormalResponseBody: null,
    });
    apiMocks.fetchInvocationResponseBody.mockResolvedValue({
      available: false,
      unavailableReason: "No abnormal response body for interrupted story",
    });

    render(
      <DashboardInvocationDetailDrawer
        open
        selection={createSelection({
          status: "interrupted",
          failureClass: "service_failure",
          failureKind: "proxy_interrupted",
          errorMessage:
            "proxy request was interrupted before completion and was recovered on startup",
        })}
        onClose={() => undefined}
      />,
    );

    await waitFor(() =>
      (document.body.textContent ?? "").includes("table.status.interrupted"),
    );

    const text = document.body.textContent ?? "";
    expect(text).toContain("table.status.interrupted");
    expect(text).not.toContain("table.status.failed");
  });

  it("shows the empty state inside the drawer when the full lookup returns no record", async () => {
    apiMocks.fetchInvocationRecords.mockResolvedValue(
      createRecordsResponse([]),
    );

    render(
      <DashboardInvocationDetailDrawer
        open
        selection={createSelection()}
        onClose={() => undefined}
      />,
    );

    await waitFor(
      () =>
        document.body.querySelector(
          '[data-testid="dashboard-invocation-detail-empty"]',
        ) != null,
    );

    expect(
      document.body.querySelector(
        '[data-testid="dashboard-invocation-detail-error"]',
      ),
    ).toBeNull();
  });

  it("shows the lookup error inside the drawer when the full record request fails", async () => {
    apiMocks.fetchInvocationRecords.mockRejectedValue(
      new Error("lookup failed"),
    );

    render(
      <DashboardInvocationDetailDrawer
        open
        selection={createSelection()}
        onClose={() => undefined}
      />,
    );

    await waitFor(
      () =>
        document.body.querySelector(
          '[data-testid="dashboard-invocation-detail-error"]',
        ) != null,
    );

    expect(document.body.textContent ?? "").toContain("lookup failed");
  });

  it("loads abnormal response details only for abnormal records", async () => {
    apiMocks.fetchInvocationRecords.mockResolvedValue(
      createRecordsResponse([
        createRecord({
          status: "failed",
          failureClass: "service_failure",
          errorMessage: "upstream exploded",
        }),
      ]),
    );
    apiMocks.fetchInvocationRecordDetail.mockResolvedValue({
      id: 501,
      abnormalResponseBody: {
        available: true,
        previewText: '{"error":"preview"}',
        hasMore: false,
      },
    });
    apiMocks.fetchInvocationResponseBody.mockResolvedValue({
      available: true,
      bodyText: '{"error":"preview","trace":"full"}',
    });

    render(
      <DashboardInvocationDetailDrawer
        open
        selection={createSelection({
          status: "failed",
          failureClass: "service_failure",
          errorMessage: "upstream exploded",
        })}
        onClose={() => undefined}
      />,
    );

    await waitFor(
      () => apiMocks.fetchInvocationRecordDetail.mock.calls.length > 0,
    );

    expect(apiMocks.fetchInvocationRecordDetail).toHaveBeenCalledWith(501);
    expect(apiMocks.fetchInvocationResponseBody).toHaveBeenCalledWith(501);
    expect(document.body.textContent ?? "").toContain(
      '{"error":"preview","trace":"full"}',
    );
  });
});
