/** @vitest-environment jsdom */
import { act, type ComponentProps } from "react";
import { createRoot, type Root } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
import { MemoryRouter } from "react-router-dom";
import { afterEach, beforeAll, beforeEach, vi } from "vitest";
import { I18nProvider, useTranslation } from "../../i18n";
import type {
  ApiInvocation,
  ApiInvocationRequestBodyResponse,
  ApiInvocationResponseBodyResponse,
  ApiInvocationWorkflowDetailResponse,
  UpstreamAccountDetail,
} from "../../lib/api";
import { InvocationTable } from "./InvocationTable";
import {
  buildInvocationDetailViewModel,
  InvocationExpandedDetails,
} from "./invocation-details-shared";

const apiMocks = vi.hoisted(() => ({
  fetchUpstreamAccountDetail: vi.fn<(accountId: number) => Promise<UpstreamAccountDetail>>(),
  fetchInvocationWorkflowDetail: vi.fn(),
  fetchInvocationRequestBody: vi.fn(),
  fetchInvocationAttemptResponseBody: vi.fn(),
  fetchInvocationResponseBody: vi.fn(),
}));
vi.mock("../../lib/api", async () => {
  const actual = await vi.importActual<typeof import("../../lib/api")>("../../lib/api");
  return {
    ...actual,
    fetchUpstreamAccountDetail: apiMocks.fetchUpstreamAccountDetail,
    fetchInvocationWorkflowDetail: apiMocks.fetchInvocationWorkflowDetail,
    fetchInvocationRequestBody: apiMocks.fetchInvocationRequestBody,
    fetchInvocationAttemptResponseBody: apiMocks.fetchInvocationAttemptResponseBody,
    fetchInvocationResponseBody: apiMocks.fetchInvocationResponseBody,
  };
});
const LONG_PROXY_NAME = "ivan-hkl-vless-vision-01KFXRNYWYXKN4JHCF3CCV78GD";
let host: HTMLDivElement | null = null;
let root: Root | null = null;
beforeAll(() => {
  Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", {
    configurable: true,
    writable: true,
    value: true,
  });
  Object.defineProperty(window, "matchMedia", {
    configurable: true,
    writable: true,
    value: vi.fn().mockImplementation((query: string) => ({
      matches: query === "(min-width: 1280px)",
      media: query,
      onchange: null,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
      addListener: vi.fn(),
      removeListener: vi.fn(),
      dispatchEvent: vi.fn(),
    })),
  });
});
beforeEach(() => {
  Object.defineProperty(window, "innerWidth", {
    configurable: true,
    writable: true,
    value: 1024,
  });
  apiMocks.fetchUpstreamAccountDetail.mockReset();
  apiMocks.fetchInvocationWorkflowDetail.mockReset();
  apiMocks.fetchInvocationRequestBody.mockReset();
  apiMocks.fetchInvocationAttemptResponseBody.mockReset();
  apiMocks.fetchInvocationResponseBody.mockReset();
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  apiMocks.fetchInvocationWorkflowDetail.mockImplementation(async (id: number) =>
    createWorkflowDetailFixture(createInvocationRecord(id)),
  );
  apiMocks.fetchInvocationRequestBody.mockResolvedValue(createRequestBodyFixture());
  apiMocks.fetchInvocationResponseBody.mockResolvedValue(createResponseBodyFixture());
  apiMocks.fetchInvocationAttemptResponseBody.mockResolvedValue(createResponseBodyFixture());
});
afterEach(async () => {
  vi.useRealTimers();
  await act(async () => {
    root?.unmount();
  });
  host?.remove();
  host = null;
  root = null;
  document.body.innerHTML = "";
});
function renderTable(records: ApiInvocation[]) {
  return renderToStaticMarkup(
    <I18nProvider>
      <InvocationTable records={records} isLoading={false} error={null} />
    </I18nProvider>,
  );
}
function createInvocationRecord(index: number): ApiInvocation {
  const occurredAt = new Date(Date.parse("2026-03-07T03:13:51Z") - index * 1_000).toISOString();
  return {
    id: index + 1,
    invokeId: `virtual-row-${index + 1}`,
    occurredAt,
    createdAt: occurredAt,
    source: "proxy",
    proxyDisplayName: `virtual-proxy-${index + 1}`,
    endpoint: "/v1/responses",
    model: "gpt-5.5",
    status: "completed",
    inputTokens: 1024 + index,
    cacheInputTokens: 512,
    outputTokens: 128,
    totalTokens: 1152 + index,
    cost: 0.001 + index * 0.0001,
    tTotalMs: 1_200 + index,
  };
}

function createWorkflowTimelineAttempt(
  record: ApiInvocation,
  requestModel: string,
  responseModel: string,
  routeMode: string,
) {
  const attemptStatus = record.status ?? "success";
  return {
    synthetic: false,
    attemptId: `attempt-${record.id ?? 1}`,
    occurredAt: record.occurredAt,
    endpoint: record.endpoint ?? "/v1/responses",
    upstreamAccountId: record.upstreamAccountId ?? 7,
    upstreamAccountName: record.upstreamAccountName ?? "pool-account-a",
    requestModel,
    responseModel,
    attemptIndex: 1,
    distinctAccountIndex: 1,
    sameAccountRetryIndex: 1,
    status: attemptStatus,
    phase: attemptStatus === "failed" ? "streaming" : "completed",
    httpStatus: 200,
    connectLatencyMs: record.tUpstreamConnectMs ?? null,
    firstTokenMs: record.firstTokenMs ?? null,
    firstByteLatencyMs: record.tUpstreamTtfbMs ?? 640,
    streamLatencyMs: record.tUpstreamStreamMs ?? record.tTotalMs ?? 5430,
    upstreamRequestId: record.upstreamRequestId ?? "req_test_workflow",
    requestSummary: {
      endpoint: record.endpoint ?? "/v1/responses",
      transport: record.transport ?? "http",
      requestModel,
      responseModel,
      requestedServiceTier: record.requestedServiceTier ?? "priority",
      reasoningEffort: record.reasoningEffort ?? "high",
      routing: {
        routeMode,
        proxyDisplayName: record.proxyDisplayName ?? "codex-relay-01",
        promptCacheKey: record.promptCacheKey ?? "pck-test",
      },
      headers: { userAgent: "monitor-ui/1.0", xForwardedFor: record.requesterIp ?? "203.0.113.10" },
      bodyCapture: { availableAtInvocationLevel: true, size: 3568, detailLevel: "full" },
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
        upstreamRequestId: record.upstreamRequestId ?? "req_test_workflow",
      },
      responseBodyCapture: { size: 7271, detailLevel: "full" },
    },
  };
}

function createWorkflowDetailFixture(
  record: ApiInvocation,
  overrides: Partial<ApiInvocationWorkflowDetailResponse> = {},
): ApiInvocationWorkflowDetailResponse {
  const requestModel = record.requestModel ?? record.model ?? "gpt-5.4";
  const responseModel = record.responseModel ?? record.model ?? requestModel;
  const routeMode = record.routeMode ?? "pool";
  const attemptOccurredAt = record.occurredAt;
  const attemptStatus = record.status ?? "success";
  const timelineAttempt = createWorkflowTimelineAttempt(
    record,
    requestModel,
    responseModel,
    routeMode,
  );
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
      upstreamAccountId: record.upstreamAccountId ?? 7,
      upstreamAccountName: record.upstreamAccountName ?? "pool-account-a",
      totalDurationMs: record.tTotalMs ?? 5430,
      timelineAttemptCount: 1,
      poolAttemptCount: record.poolAttemptCount ?? 1,
      totalTokens: record.totalTokens ?? 2048,
      cost: record.cost ?? 0.0042,
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
        occurredAt: attemptOccurredAt,
        title: "Attempt 1",
        status: attemptStatus,
        attempt: timelineAttempt,
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
    bodyText: JSON.stringify({
      model: "gpt-5.4",
      stream: true,
      service_tier: "priority",
      input: [{ role: "user", content: "hello from workflow table test" }],
    }),
    captureSource: "raw_file",
    bodySize: 3568,
    bodyTruncated: false,
    detailLevel: "full",
    headers: { userAgent: "monitor-ui/1.0" },
    routing: { routeMode: "pool", proxyDisplayName: "codex-relay-01" },
    ...overrides,
  };
}
function createResponseBodyFixture(
  overrides: Partial<ApiInvocationResponseBodyResponse> = {},
): ApiInvocationResponseBodyResponse {
  return {
    available: true,
    bodyText: JSON.stringify({
      id: "resp_table_test",
      status: "completed",
      model: "gpt-5.4",
      output: [{ type: "message", role: "assistant" }],
    }),
    captureSource: "raw_file",
    bodySize: 7271,
    bodyTruncated: false,
    detailLevel: "full",
    headers: { contentEncoding: "gzip" },
    routing: { forwardedChunkCount: 12 },
    ...overrides,
  };
}
async function renderInteractiveTable(
  records: ApiInvocation[],
  props: Partial<ComponentProps<typeof InvocationTable>> = {},
) {
  await act(async () => {
    root?.render(
      <MemoryRouter>
        <I18nProvider>
          <InvocationTable records={records} isLoading={false} error={null} {...props} />
        </I18nProvider>
      </MemoryRouter>,
    );
  });
}
function InvocationDetailProbe({ record }: { record: ApiInvocation }) {
  const { t, locale } = useTranslation();
  const localeTag = locale === "zh" ? "zh-CN" : "en-US";
  const detailView = buildInvocationDetailViewModel({
    record,
    normalizedStatus: (record.status ?? "unknown").toLowerCase(),
    t,
    locale,
    localeTag,
    numberFormatter: new Intl.NumberFormat(localeTag),
    currencyFormatter: new Intl.NumberFormat(localeTag, {
      style: "currency",
      currency: "USD",
      minimumFractionDigits: 4,
      maximumFractionDigits: 4,
    }),
    renderAccountValue: (accountLabel) => <span>{accountLabel}</span>,
  });
  return (
    <InvocationExpandedDetails
      record={record}
      detailId="invocation-detail-probe"
      detailPairs={detailView.detailPairs}
      timingPairs={detailView.timingPairs}
      errorMessage={detailView.errorMessage}
      detailNotice={detailView.detailNotice}
      size="default"
      poolAttemptsState={{
        attemptsByInvokeId: {},
        loadingByInvokeId: {},
        errorByInvokeId: {},
      }}
      t={t}
    />
  );
}
async function waitForCondition(
  predicate: () => boolean,
  options?: { attempts?: number; delayMs?: number },
) {
  const attempts = options?.attempts ?? 25;
  const delayMs = options?.delayMs ?? 0;
  for (let index = 0; index < attempts; index += 1) {
    if (predicate()) return;
    await act(async () => {
      await new Promise((resolve) => window.setTimeout(resolve, delayMs));
    });
  }
  throw new Error("Condition was not met before timeout");
}
function buildDetailViewForAccount(record: ApiInvocation) {
  return buildInvocationDetailViewModel({
    record,
    normalizedStatus: (record.status ?? "unknown").toLowerCase(),
    t: (key) => key,
    locale: "zh",
    localeTag: "zh-CN",
    numberFormatter: new Intl.NumberFormat("zh-CN"),
    currencyFormatter: new Intl.NumberFormat("zh-CN", {
      style: "currency",
      currency: "USD",
      minimumFractionDigits: 4,
      maximumFractionDigits: 4,
    }),
    renderAccountValue: (accountLabel) => <span>{accountLabel}</span>,
  });
}

export {
  apiMocks,
  buildDetailViewForAccount,
  createInvocationRecord,
  createRequestBodyFixture,
  createResponseBodyFixture,
  createWorkflowDetailFixture,
  host,
  InvocationDetailProbe,
  LONG_PROXY_NAME,
  renderInteractiveTable,
  renderTable,
  root,
  waitForCondition,
};
