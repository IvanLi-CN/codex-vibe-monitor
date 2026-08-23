/** @vitest-environment jsdom */
import { act, useEffect, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MemoryRouter } from "react-router-dom";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useSubscriptionTopic } from "../../hooks/useSubscriptionTopic";
import { I18nProvider } from "../../i18n";
import {
  type ApiPoolUpstreamRequestAttempt,
  fetchForwardProxyBindingNodes,
  fetchInvocationAttemptResponseBody,
  fetchInvocationRequestBody,
  fetchInvocationResponseBody,
  locateUpstreamAccountAttempt,
  type UpstreamAccountAttemptListResponse,
} from "../../lib/api";
import {
  buildTopicDescriptor,
  getTopicDescriptorKey,
  type SubscriptionTopicDescriptor,
} from "../../lib/sse";
import { UpstreamAccountAttemptTimeline } from "./UpstreamAccountAttemptTimeline";

vi.mock("../../lib/api", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../lib/api")>()),
  fetchForwardProxyBindingNodes: vi.fn(),
  fetchInvocationRequestBody: vi.fn(),
  fetchInvocationAttemptResponseBody: vi.fn(),
  fetchInvocationResponseBody: vi.fn(),
  locateUpstreamAccountAttempt: vi.fn(),
}));

vi.mock("../../hooks/useSubscriptionTopic", () => ({
  useSubscriptionTopic: vi.fn(),
}));

type TopicSnapshotRequest = {
  type?: "normal" | "remote_v2" | "compact" | "image";
  model?: string;
  stickyKey?: string;
  page?: number;
  pageSize?: number;
};

const topicSnapshotMock =
  vi.fn<
    (
      accountId: number,
      request: TopicSnapshotRequest,
    ) => Promise<UpstreamAccountAttemptListResponse>
  >();
const fetchBindingNodesMock = vi.mocked(fetchForwardProxyBindingNodes);
const fetchRequestBodyMock = vi.mocked(fetchInvocationRequestBody);
const fetchAttemptResponseBodyMock = vi.mocked(fetchInvocationAttemptResponseBody);
const fetchResponseBodyMock = vi.mocked(fetchInvocationResponseBody);
const subscriptionTopicMock = vi.mocked(useSubscriptionTopic);
const originalScrollIntoView = HTMLElement.prototype.scrollIntoView;

const topicSnapshotCache = new Map<string, UpstreamAccountAttemptListResponse>();
const topicListeners = new Map<
  string,
  Set<(response: UpstreamAccountAttemptListResponse) => void>
>();

function useMockSubscriptionTopic(descriptor: SubscriptionTopicDescriptor | null, enabled = true) {
  const descriptorKey = descriptor ? getTopicDescriptorKey(descriptor) : null;
  const [data, setData] = useState<UpstreamAccountAttemptListResponse | null>(() =>
    descriptor && enabled ? (topicSnapshotCache.get(descriptorKey ?? "") ?? null) : null,
  );
  const [isLoading, setIsLoading] = useState(
    Boolean(descriptor && enabled && !topicSnapshotCache.has(descriptorKey ?? "")),
  );
  const [deliverySource, setDeliverySource] = useState<"cache" | "network" | null>(() =>
    descriptor && enabled && topicSnapshotCache.has(descriptorKey ?? "") ? "cache" : null,
  );

  useEffect(() => {
    if (!descriptor || !enabled || !descriptorKey) {
      setData(null);
      setIsLoading(false);
      setDeliverySource(null);
      return;
    }
    const cached = topicSnapshotCache.get(descriptorKey);
    setData(cached ?? null);
    setIsLoading(!cached);
    setDeliverySource(cached ? "cache" : null);
    const listeners = topicListeners.get(descriptorKey) ?? new Set();
    const listener = (next: UpstreamAccountAttemptListResponse) => {
      topicSnapshotCache.set(descriptorKey, next);
      setData(next);
      setIsLoading(false);
      setDeliverySource("network");
    };
    listeners.add(listener);
    topicListeners.set(descriptorKey, listeners);
    if (!cached) {
      const params = descriptor.params ?? {};
      void topicSnapshotMock(Number(params.accountId), {
        type: params.type as "normal" | "remote_v2" | "compact" | "image" | undefined,
        model: typeof params.model === "string" ? params.model : undefined,
        stickyKey: typeof params.stickyKey === "string" ? params.stickyKey : undefined,
        page: Number(params.page ?? 1),
        pageSize: Number(params.pageSize ?? 50),
      }).then((next) => listener(next));
    }
    return () => {
      listeners.delete(listener);
      if (listeners.size === 0) topicListeners.delete(descriptorKey);
    };
  }, [descriptor, descriptorKey, enabled]);

  return {
    data: enabled ? data : null,
    descriptorKey: enabled ? descriptorKey : null,
    lastReceivedAt: null,
    lastKind: null,
    deliverySource,
    isLoading: enabled ? isLoading : false,
    error: null,
    refresh: vi.fn(),
  };
}

function emitTopicSnapshot(
  descriptor: SubscriptionTopicDescriptor,
  response: UpstreamAccountAttemptListResponse,
) {
  const key = getTopicDescriptorKey(descriptor);
  topicSnapshotCache.set(key, response);
  topicListeners.get(key)?.forEach((listener) => {
    listener(response);
  });
}

let host: HTMLDivElement | null = null;
let root: Root | null = null;
let interactionBoundary: HTMLDivElement | null = null;
let scrollIntoViewMock = vi.fn();

function renderTimeline({
  focusedAttemptId = null,
  focusVersion = 0,
  onFocusRequestHandled,
  boundary = null,
  visible = true,
}: {
  focusedAttemptId?: string | null;
  focusVersion?: number;
  onFocusRequestHandled?: (version: number) => void;
  boundary?: HTMLElement | null;
  visible?: boolean;
} = {}) {
  if (!host) {
    host = document.createElement("div");
    document.body.appendChild(host);
  }
  if (!root) root = createRoot(host);
  act(() => {
    root?.render(
      <MemoryRouter>
        <I18nProvider>
          {visible ? (
            <UpstreamAccountAttemptTimeline
              accountId={101}
              focusedAttemptId={focusedAttemptId}
              focusVersion={focusVersion}
              interactionBoundary={boundary}
              onFocusRequestHandled={onFocusRequestHandled}
            />
          ) : null}
        </I18nProvider>
      </MemoryRouter>,
    );
  });
}

async function flushAsync() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();
  });
}

function attemptListResponse(
  overrides: Partial<UpstreamAccountAttemptListResponse> & {
    items?: ApiPoolUpstreamRequestAttempt[];
  } = {},
): UpstreamAccountAttemptListResponse {
  const items = overrides.items ?? [];
  return {
    items,
    stickyKeyOptions: [],
    total: items.length,
    page: 1,
    pageSize: 50,
    ...overrides,
  };
}

function makeAttempt(
  overrides: Partial<ApiPoolUpstreamRequestAttempt>,
): ApiPoolUpstreamRequestAttempt {
  return {
    attemptId: "ATEST0001",
    invokeId: "K7QM9ZD4HP",
    occurredAt: "2026-07-11T12:00:00.000Z",
    endpoint: "/v1/responses",
    upstreamAccountId: 101,
    requestModel: "gpt-5.5",
    responseModel: "gpt-5.5",
    proxyBindingKeySnapshot: "__direct__",
    attemptIndex: 1,
    distinctAccountIndex: 1,
    sameAccountRetryIndex: 0,
    status: "success",
    phase: "completed",
    createdAt: "2026-07-11T12:00:00.000Z",
    ...overrides,
  };
}

async function selectOptionByText(triggerSelector: string, label: RegExp) {
  const trigger = document.body.querySelector(triggerSelector);
  if (!(trigger instanceof HTMLButtonElement)) {
    throw new Error(`missing select trigger: ${triggerSelector}`);
  }
  act(() => {
    trigger.click();
  });
  await flushAsync();
  const option = Array.from(document.body.querySelectorAll('[role="option"]')).find((candidate) =>
    label.test(candidate.textContent ?? ""),
  );
  if (!(option instanceof HTMLDivElement)) {
    throw new Error(`missing select option: ${label}`);
  }
  act(() => {
    option.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
    option.dispatchEvent(new PointerEvent("pointerup", { bubbles: true }));
    option.click();
  });
  await flushAsync();
}

async function selectModelOption(label: RegExp) {
  const input = document.body.querySelector<HTMLInputElement>("#upstream-attempt-model-filter");
  if (!input) throw new Error("missing model filter input");
  act(() => {
    input.focus();
    input.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
  await flushAsync();
  const option = Array.from(document.body.querySelectorAll('button[role="option"]')).find(
    (candidate) => label.test(candidate.textContent ?? ""),
  );
  if (!(option instanceof HTMLButtonElement)) {
    throw new Error(`missing model option: ${label}`);
  }
  act(() => {
    option.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
  });
  await flushAsync();
}

describe("UpstreamAccountAttemptTimeline", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    topicSnapshotCache.clear();
    topicListeners.clear();
    topicSnapshotMock.mockReset();
    topicSnapshotMock.mockResolvedValue(attemptListResponse());
    subscriptionTopicMock.mockImplementation(useMockSubscriptionTopic);
    vi.useRealTimers();
    scrollIntoViewMock = vi.fn();
    Object.defineProperty(HTMLElement.prototype, "scrollIntoView", {
      configurable: true,
      value: scrollIntoViewMock,
    });
    vi.mocked(locateUpstreamAccountAttempt).mockResolvedValue(
      attemptListResponse({
        items: [],
        total: 0,
        page: 1,
        pageSize: 50,
      }),
    );
    fetchAttemptResponseBodyMock.mockResolvedValue({
      available: false,
      unavailableReason: "attempt_response_body_not_captured",
      detailLevel: "attempt_metrics",
    });
    fetchBindingNodesMock.mockResolvedValue([
      {
        key: "jp-edge-01",
        source: "manual",
        displayName: "JP Edge 01",
        protocolLabel: "HTTP",
        egressIp: null,
        egressIpCheckedAt: null,
        egressIpProvider: null,
        egressIpError: null,
        egressIpErrorAt: null,
        penalized: false,
        selectable: true,
        last24h: [],
      },
    ]);
  });

  afterEach(() => {
    vi.useRealTimers();
    act(() => {
      root?.unmount();
    });
    host?.remove();
    interactionBoundary?.remove();
    interactionBoundary = null;
    root = null;
    host = null;
    Object.defineProperty(HTMLElement.prototype, "scrollIntoView", {
      configurable: true,
      value: originalScrollIntoView,
    });
  });

  it("keeps the primary row focused on upstream evidence and reveals complete failure context on demand", async () => {
    topicSnapshotMock.mockResolvedValue(
      attemptListResponse({
        items: [
          {
            attemptId: "4V7MYPJG",
            invokeId: "K7QM9ZD4HP",
            occurredAt: "2026-07-11T12:00:00.000Z",
            endpoint: "/v1/responses",
            upstreamAccountId: 101,
            requestModel: "gpt-5.4",
            responseModel: "gpt-5.4-2026-07-01",
            proxyBindingKeySnapshot: "jp-edge-01",
            attemptIndex: 1,
            distinctAccountIndex: 0,
            sameAccountRetryIndex: 0,
            status: "http_failure",
            phase: "failed",
            httpStatus: 500,
            downstreamHttpStatus: 502,
            failureKind: "upstream_response_failed",
            errorMessage: "upstream returned an oversized diagnostic payload",
            connectLatencyMs: 120,
            firstByteLatencyMs: 480,
            streamLatencyMs: 810,
            downstreamRequestContentEncoding: "gzip",
            upstreamRequestCompressionAlgorithm: "zstd",
            upstreamRequestCompressionMode: "recompressed",
            logicalBodyBytes: 1000,
            transmittedBodyBytes: 580,
            savedBytes: 420,
            ratioPct: -42,
            approxUploadBytes: 644,
            approxDownloadBytes: 812,
            upstreamRequestId: "req_upstream_123",
            upstreamRouteKey: "route-tokyo-primary",
            createdAt: "2026-07-11T12:00:00.000Z",
          },
        ],
        total: 1,
        page: 1,
        pageSize: 50,
      }),
    );

    renderTimeline();
    await flushAsync();

    const list = host?.querySelector<HTMLElement>('[data-testid="upstream-account-attempt-list"]');
    expect(list).not.toBeNull();
    const card = host?.querySelector<HTMLElement>(
      '[data-testid="account-attempt-record-4V7MYPJG"]',
    );
    expect(card).not.toBeNull();
    expect(card?.textContent).toMatch(/上游 HTTP 500|upstream http 500/i);
    expect(card?.textContent).toContain("K7QM9ZD4HP");
    expect(card?.textContent).not.toContain("POOLCALL001");
    const requestHeadersButton = Array.from(card?.querySelectorAll("button") ?? []).find((button) =>
      /请求头|headers/i.test(button.textContent ?? ""),
    );
    expect(requestHeadersButton).toBeDefined();
    act(() => {
      requestHeadersButton?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    expect(card?.textContent).toContain("JP Edge 01");
    expect(card?.textContent).toContain("route-tokyo-primary");
    const requestBodyButton = Array.from(card?.querySelectorAll("button") ?? []).find((button) =>
      /请求体|body/i.test(button.textContent ?? ""),
    );
    expect(requestBodyButton).toBeDefined();
    act(() => {
      requestBodyButton?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    expect(card?.textContent).toMatch(/http request compression|http 请求压缩/i);
    expect(card?.textContent).toMatch(/zstd/i);
    expect(card?.textContent).toContain("-42% (1,000 B -> 580 B)");
    const timingButton = Array.from(card?.querySelectorAll("button") ?? []).find((button) =>
      /时间|timing/i.test(button.textContent ?? ""),
    );
    expect(timingButton).toBeDefined();
    act(() => {
      timingButton?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    expect(card?.textContent).toContain("req_upstream_123");
    const responseButton = Array.from(card?.querySelectorAll("button") ?? []).find((button) =>
      /^(响应|response)/i.test((button.textContent ?? "").trim()),
    );
    expect(responseButton).toBeDefined();
    act(() => {
      responseButton?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    expect(card?.textContent).toMatch(/upstream returned an oversized diagnostic payload/i);
  });

  it("uses backend workflow entries so account attempts match invocation detail summaries and lazy body loading", async () => {
    fetchRequestBodyMock.mockResolvedValue({
      available: true,
      bodyText: '{"model":"gpt-5.5","input":"large request"}',
      headers: {
        userAgent: "codex-vibe-monitor-test/1.0",
        xForwardedFor: "192.168.31.6",
      },
      routing: {
        routeMode: "pool",
        stickyKey: "sticky-a",
      },
      bodySize: 217_958,
      detailLevel: "full",
      captureSource: "raw_file",
    });
    fetchResponseBodyMock.mockResolvedValue({
      available: true,
      bodyText: '{"status":"success","output":"large response"}',
      headers: {
        contentEncoding: "identity",
        upstreamRequestId: "req_upstream_account_workflow",
      },
      routing: {
        forwardedChunkCount: 7,
      },
      bodySize: 79_224,
      detailLevel: "full",
      captureSource: "raw_file",
    });
    fetchAttemptResponseBodyMock.mockResolvedValue({
      available: true,
      bodyText: '{"status":"success","output":"large response"}',
      headers: {
        contentEncoding: "identity",
        upstreamRequestId: "req_upstream_account_workflow",
      },
      bodySize: 79_224,
      detailLevel: "full",
      captureSource: "attempt_raw_file",
    });
    topicSnapshotMock.mockResolvedValue(
      attemptListResponse({
        items: [
          {
            attemptId: "ASUCC002",
            invokeId: "ACCOUNTWF1",
            occurredAt: "2026-07-11T12:00:00.000Z",
            endpoint: "/v1/responses",
            upstreamAccountId: 101,
            upstreamAccountName: "CIII",
            requestModel: "gpt-5.5",
            responseModel: "gpt-5.5",
            proxyBindingKeySnapshot: "__direct__",
            attemptIndex: 2,
            distinctAccountIndex: 1,
            sameAccountRetryIndex: 1,
            status: "success",
            phase: "completed",
            httpStatus: 200,
            connectLatencyMs: 45,
            firstByteLatencyMs: 120,
            streamLatencyMs: 3_280,
            upstreamRequestId: "req_upstream_account_workflow",
            upstreamRequestCompressionAlgorithm: "zstd",
            upstreamRequestCompressionMode: "recompressed",
            logicalBodyBytes: 217_958,
            transmittedBodyBytes: 53_295,
            savedBytes: 164_663,
            ratioPct: -75.55,
            approxUploadBytes: 54_319,
            approxDownloadBytes: 80_000,
            createdAt: "2026-07-11T12:00:00.000Z",
            invocationRecord: {
              id: 77,
              invokeId: "ACCOUNTWF1",
              occurredAt: "2026-07-11T12:00:00.000Z",
              createdAt: "2026-07-11T12:00:00.000Z",
              source: "proxy",
              routeMode: "pool",
              endpoint: "/v1/responses",
              requestModel: "gpt-5.5",
              responseModel: "gpt-5.5",
              status: "success",
              requesterIp: "192.168.31.6",
              upstreamAccountId: 101,
              upstreamAccountName: "CIII",
              inputTokens: 49_042,
              cacheInputTokens: 46_952,
              outputTokens: 87,
              totalTokens: 48_769,
              cost: 0.0364,
              responseContentEncoding: "identity",
              tReqReadMs: 11,
              tReqParseMs: 13,
              tUpstreamConnectMs: 45,
              tUpstreamTtfbMs: 120,
              tUpstreamStreamMs: 3_280,
              tRespParseMs: 18,
              tPersistMs: 22,
              tTotalMs: 3_280,
            },
            workflowEntry: {
              blockId: "attempt-ASUCC002",
              kind: "attempt",
              occurredAt: "2026-07-11T12:00:00.000Z",
              title: "Attempt #2",
              subtitle: "CIII",
              status: "success",
              attempt: {
                synthetic: false,
                attemptId: "ASUCC002",
                occurredAt: "2026-07-11T12:00:00.000Z",
                endpoint: "/v1/responses",
                stickyKey: "sticky-a",
                routingSource: "failover",
                upstreamAccountId: 101,
                upstreamAccountName: "CIII",
                requestModel: "gpt-5.5",
                responseModel: "gpt-5.5",
                upstreamRouteKey: "route-direct",
                proxyBindingKeySnapshot: "__direct__",
                attemptIndex: 2,
                distinctAccountIndex: 1,
                sameAccountRetryIndex: 1,
                requesterIp: "192.168.31.6",
                startedAt: "2026-07-11T12:00:00.000Z",
                finishedAt: "2026-07-11T12:00:03.280Z",
                status: "success",
                phase: "completed",
                httpStatus: 200,
                downstreamHttpStatus: 200,
                connectLatencyMs: 45,
                firstByteLatencyMs: 120,
                streamLatencyMs: 3_280,
                upstreamRequestId: "req_upstream_account_workflow",
                requestSummary: {
                  endpoint: "/v1/responses",
                  routeMode: "pool",
                  requestModel: "gpt-5.5",
                  responseModel: "gpt-5.5",
                  requestedServiceTier: "low",
                  reasoningEffort: "low",
                  promptCacheKey: "019f89ab-b67e-71a2-9633-324247eec56e",
                  requesterIp: "192.168.31.6",
                  routing: {
                    proxyDisplayName: "Direct",
                    upstreamRouteKey: "route-direct",
                    proxyBindingKey: "__direct__",
                  },
                  headers: {
                    userAgent: "codex-vibe-monitor-test/1.0",
                    xForwardedFor: "192.168.31.6",
                  },
                  compression: {
                    algorithm: "zstd",
                    mode: "recompressed",
                    logicalBodyBytes: 217_958,
                    transmittedBodyBytes: 53_295,
                    savedBytes: 164_663,
                    ratioPct: -75.55,
                    approxUploadBytes: 54_319,
                    approxDownloadBytes: 80_000,
                  },
                  bodyCapture: {
                    availableAtInvocationLevel: true,
                    size: 217_958,
                    truncated: false,
                    detailLevel: "full",
                  },
                },
                responseSummary: {
                  status: "success",
                  phase: "completed",
                  httpStatus: 200,
                  responseContentEncoding: "identity",
                  headers: {
                    contentEncoding: "identity",
                    upstreamRequestId: "req_upstream_account_workflow",
                  },
                  delivery: {
                    forwardedChunkCount: 7,
                    usageObserved: true,
                  },
                  latencyMs: {
                    connect: 45,
                    firstByte: 120,
                    stream: 3_280,
                    requestRead: 11,
                    requestParse: 13,
                    responseParse: 18,
                    persist: 22,
                    total: 3_280,
                  },
                  responseBodyCapture: {
                    availableAtInvocationLevel: true,
                    size: 79_224,
                    truncated: false,
                    detailLevel: "full",
                  },
                  usage: {
                    inputTokens: 49_042,
                    cacheWriteTokens: 2_090,
                    cacheInputTokens: 46_952,
                    outputTokens: 87,
                    totalTokens: 48_769,
                    cost: 0.0364,
                    tokens: {
                      input: 49_042,
                      cacheWrite: 2_090,
                      cacheRead: 46_952,
                      output: 87,
                      total: 48_769,
                    },
                    costs: {
                      recorded: {
                        total: 0.0364,
                      },
                    },
                    audit: {
                      mismatch: false,
                    },
                  },
                },
              },
              detail: null,
              responseBody: null,
            },
          },
        ],
        total: 1,
        page: 1,
        pageSize: 50,
      }),
    );

    renderTimeline();
    await flushAsync();

    const card = host?.querySelector<HTMLElement>(
      '[data-testid="account-attempt-record-ASUCC002"]',
    );
    expect(card).not.toBeNull();
    expect(card?.textContent).toContain("217,958 B");
    expect(card?.textContent).toContain("79,224 B");
    expect(card?.textContent).toContain("输入写 2,090");
    expect(card?.textContent).toContain("输入读 46,952");
    expect(card?.textContent).toContain("输出 87");
    expect(card?.textContent).toContain("金额 US$0.0364");

    const requestBodyButton = Array.from(card?.querySelectorAll("button") ?? []).find((button) =>
      /请求体|request body/i.test(button.textContent ?? ""),
    );
    expect(requestBodyButton).toBeDefined();
    act(() => {
      requestBodyButton?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    await flushAsync();
    expect(fetchRequestBodyMock).toHaveBeenCalledWith(77);
    expect(card?.textContent).toContain("codex-vibe-monitor-test/1.0");

    const responseBodyButton = Array.from(card?.querySelectorAll("button") ?? []).find((button) =>
      /响应体|response body/i.test(button.textContent ?? ""),
    );
    expect(responseBodyButton).toBeDefined();
    act(() => {
      responseBodyButton?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    await flushAsync();
    expect(fetchAttemptResponseBodyMock).toHaveBeenCalledWith(77, "ASUCC002");
    expect(card?.textContent).toContain("large response");
  });

  it("does not lazy-load the final invocation response body for non-final retry attempts", async () => {
    topicSnapshotMock.mockResolvedValue(
      attemptListResponse({
        items: [
          {
            attemptId: "AFAIL001",
            invokeId: "ACCOUNTWF1",
            occurredAt: "2026-07-11T12:00:00.000Z",
            endpoint: "/v1/responses",
            upstreamAccountId: 101,
            upstreamAccountName: "CIII",
            requestModel: "gpt-5.5",
            responseModel: "gpt-5.5",
            proxyBindingKeySnapshot: "__direct__",
            attemptIndex: 1,
            distinctAccountIndex: 1,
            sameAccountRetryIndex: 0,
            status: "http_failure",
            phase: "completed",
            httpStatus: 500,
            failureKind: "upstream_response_failed",
            streamLatencyMs: 3_280,
            approxDownloadBytes: 80_000,
            createdAt: "2026-07-11T12:00:00.000Z",
            invocationRecord: {
              id: 77,
              invokeId: "ACCOUNTWF1",
              occurredAt: "2026-07-11T12:00:00.000Z",
              createdAt: "2026-07-11T12:00:00.000Z",
              source: "proxy",
              routeMode: "pool",
              endpoint: "/v1/responses",
              requestModel: "gpt-5.5",
              responseModel: "gpt-5.5",
              status: "success",
            },
            workflowEntry: {
              blockId: "attempt-AFAIL001",
              kind: "attempt",
              occurredAt: "2026-07-11T12:00:00.000Z",
              title: "Attempt #1",
              subtitle: "CIII",
              status: "http_failure",
              attempt: {
                synthetic: false,
                attemptId: "AFAIL001",
                occurredAt: "2026-07-11T12:00:00.000Z",
                endpoint: "/v1/responses",
                upstreamAccountId: 101,
                upstreamAccountName: "CIII",
                requestModel: "gpt-5.5",
                responseModel: "gpt-5.5",
                attemptIndex: 1,
                distinctAccountIndex: 1,
                sameAccountRetryIndex: 0,
                status: "http_failure",
                phase: "completed",
                httpStatus: 500,
                failureKind: "upstream_response_failed",
                streamLatencyMs: 3_280,
                requestSummary: {
                  endpoint: "/v1/responses",
                  requestModel: "gpt-5.5",
                },
                responseSummary: {
                  status: "http_failure",
                  phase: "completed",
                  httpStatus: 500,
                  failureKind: "upstream_response_failed",
                  responseBodyCapture: {
                    availableAtInvocationLevel: false,
                    size: 79_224,
                    detailLevel: "attempt_metrics",
                    unavailableReason: "non_final_attempt_response_body_not_captured",
                  },
                  usage: null,
                },
              },
              detail: null,
              responseBody: null,
            },
          },
        ],
        total: 1,
        page: 1,
        pageSize: 50,
      }),
    );

    renderTimeline();
    await flushAsync();

    const card = host?.querySelector<HTMLElement>(
      '[data-testid="account-attempt-record-AFAIL001"]',
    );
    expect(card).not.toBeNull();
    expect(card?.textContent).toContain("79,224 B");

    const responseBodyButton = Array.from(card?.querySelectorAll("button") ?? []).find((button) =>
      /响应体|response body/i.test(button.textContent ?? ""),
    );
    expect(responseBodyButton).toBeDefined();
    act(() => {
      responseBodyButton?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    await flushAsync();

    expect(fetchAttemptResponseBodyMock).toHaveBeenCalledWith(77, "AFAIL001");
    expect(fetchResponseBodyMock).not.toHaveBeenCalled();
    expect(card?.textContent).toContain("该次上游尝试未保留可展示的响应体");
  });

  it("shows the pending attempt phase without adding another permanent column", async () => {
    topicSnapshotMock.mockResolvedValue(
      attemptListResponse({
        items: [
          {
            attemptId: "QADKN5Z9",
            invokeId: "M8R7XZ4Q2W",
            occurredAt: "2026-07-11T12:00:00.000Z",
            endpoint: "/v1/responses",
            upstreamAccountId: 101,
            requestModel: "gpt-5.4",
            proxyBindingKeySnapshot: "__direct__",
            attemptIndex: 1,
            distinctAccountIndex: 0,
            sameAccountRetryIndex: 0,
            status: "pending",
            phase: "waiting_first_byte",
            connectLatencyMs: 80,
            createdAt: "2026-07-11T12:00:00.000Z",
          },
        ],
        total: 1,
        page: 1,
        pageSize: 50,
      }),
    );

    renderTimeline();
    await flushAsync();

    const card = host?.querySelector<HTMLElement>(
      '[data-testid="account-attempt-record-QADKN5Z9"]',
    );
    expect(card).not.toBeNull();
    expect(card?.textContent).toContain("waiting_first_byte");
    expect(card?.textContent).not.toMatch(/阶段|phase/i);
  });

  it("keeps filters visible and sends type filters through pagination", async () => {
    const normalAttempt = makeAttempt({
      attemptId: "ANORMAL001",
      endpoint: "/v1/responses",
      requestModel: "gpt-5.5",
      responseModel: "gpt-5.5",
    });
    const imageAttempt = makeAttempt({
      attemptId: "AIMAGE001",
      endpoint: "/v1/images/edits",
      requestModel: "gpt-image-1",
      responseModel: "gpt-image-1",
      imageIntent: "direct_image",
    });
    topicSnapshotMock
      .mockResolvedValueOnce(
        attemptListResponse({
          items: [normalAttempt],
          total: 1,
          stickyKeyOptions: [
            { value: "sticky-normal", latestCreatedAt: "2026-07-11T12:00:00.000Z" },
          ],
        }),
      )
      .mockResolvedValueOnce(
        attemptListResponse({
          items: [imageAttempt],
          total: 75,
          page: 1,
          pageSize: 50,
          stickyKeyOptions: [
            { value: "sticky-image", latestCreatedAt: "2026-07-11T12:00:00.000Z" },
          ],
        }),
      )
      .mockResolvedValueOnce(
        attemptListResponse({
          items: [{ ...imageAttempt, attemptId: "AIMAGE002" }],
          total: 75,
          page: 2,
          pageSize: 50,
        }),
      );

    renderTimeline();
    await flushAsync();
    expect(
      host?.querySelector('[data-testid="upstream-account-attempt-filter-bar"]'),
    ).not.toBeNull();

    await selectOptionByText('[data-testid="upstream-attempt-type-filter"]', /image/i);
    expect(topicSnapshotMock).toHaveBeenLastCalledWith(
      101,
      expect.objectContaining({
        type: "image",
        page: 1,
        pageSize: 50,
      }),
    );

    const nextButton = Array.from(host?.querySelectorAll("button") ?? []).find((button) =>
      /下一页|next/i.test(button.textContent ?? ""),
    );
    expect(nextButton).toBeDefined();
    act(() => {
      nextButton?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    await flushAsync();
    expect(topicSnapshotMock).toHaveBeenLastCalledWith(
      101,
      expect.objectContaining({
        type: "image",
        page: 2,
        pageSize: 50,
      }),
    );
  });

  it("offers request and response models and keeps empty results inside the list body", async () => {
    topicSnapshotMock
      .mockResolvedValueOnce(
        attemptListResponse({
          items: [
            makeAttempt({
              attemptId: "AMODEL001",
              requestModel: "gpt-5.4",
              responseModel: "gpt-5.6",
            }),
          ],
          total: 1,
        }),
      )
      .mockResolvedValueOnce(
        attemptListResponse({
          items: [],
          total: 0,
        }),
      );

    renderTimeline();
    await flushAsync();
    await selectModelOption(/gpt-5\.6/);

    expect(topicSnapshotMock).toHaveBeenLastCalledWith(
      101,
      expect.objectContaining({
        model: "gpt-5.6",
        page: 1,
      }),
    );
    expect(
      host?.querySelector('[data-testid="upstream-account-attempt-filter-bar"]'),
    ).not.toBeNull();
    expect(
      host?.querySelector('[data-testid="upstream-account-attempt-list"]')?.textContent,
    ).toMatch(/最近 7 天没有该账号的尝试请求|No request attempts/i);
  });

  it("preserves backend conversation option order and filters the unbound bucket", async () => {
    topicSnapshotMock
      .mockResolvedValueOnce(
        attemptListResponse({
          items: [makeAttempt({ attemptId: "ACONVO001" })],
          total: 1,
          stickyKeyOptions: [
            { value: "sticky-new", latestCreatedAt: "2026-07-11T12:03:00.000Z" },
            { value: "__unbound__", latestCreatedAt: "2026-07-11T12:02:00.000Z" },
            { value: "sticky-old", latestCreatedAt: "2026-07-11T12:01:00.000Z" },
          ],
        }),
      )
      .mockResolvedValueOnce(
        attemptListResponse({
          items: [],
          total: 0,
        }),
      );

    renderTimeline();
    await flushAsync();
    const trigger = document.body.querySelector(
      '[data-testid="upstream-attempt-conversation-filter"]',
    );
    if (!(trigger instanceof HTMLButtonElement)) throw new Error("missing conversation filter");
    act(() => {
      trigger.click();
    });
    await flushAsync();
    const optionTexts = Array.from(document.body.querySelectorAll('[role="option"]')).map(
      (option) => option.textContent ?? "",
    );
    const newIndex = optionTexts.findIndex((text) => text.includes("sticky-new"));
    const unboundIndex = optionTexts.findIndex((text) =>
      /Unbound conversation|未绑定对话/.test(text),
    );
    const oldIndex = optionTexts.findIndex((text) => text.includes("sticky-old"));
    expect(newIndex).toBeGreaterThanOrEqual(0);
    expect(unboundIndex).toBeGreaterThan(newIndex);
    expect(oldIndex).toBeGreaterThan(unboundIndex);

    const unboundOption = Array.from(document.body.querySelectorAll('[role="option"]')).find(
      (option) => /Unbound conversation|未绑定对话/.test(option.textContent ?? ""),
    );
    if (!(unboundOption instanceof HTMLDivElement)) throw new Error("missing unbound option");
    act(() => {
      unboundOption.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
      unboundOption.click();
    });
    await flushAsync();

    expect(topicSnapshotMock).toHaveBeenLastCalledWith(
      101,
      expect.objectContaining({
        stickyKey: "__unbound__",
        page: 1,
      }),
    );
  });

  it("scrolls, highlights, and fades the focused attempt after the next drawer interaction", async () => {
    vi.useFakeTimers();
    const focusedAttempt = {
      attemptId: "YG7P25XG",
      invokeId: "YG7P25XG9K",
      occurredAt: "2026-07-11T12:00:00.000Z",
      endpoint: "/v1/responses",
      upstreamAccountId: 101,
      requestModel: "gpt-5.4",
      proxyBindingKeySnapshot: "jp-edge-01",
      attemptIndex: 1,
      distinctAccountIndex: 0,
      sameAccountRetryIndex: 0,
      status: "http_failure",
      phase: "failed",
      httpStatus: 500,
      errorMessage: "focused failure details",
      createdAt: "2026-07-11T12:00:00.000Z",
    };
    topicSnapshotMock.mockImplementation(async (_accountId, options) =>
      attemptListResponse({
        total: 100,
        page: options?.page ?? 1,
        pageSize: 50,
      }),
    );
    vi.mocked(locateUpstreamAccountAttempt).mockResolvedValue(
      attemptListResponse({
        items: [focusedAttempt],
        total: 100,
        page: 2,
        pageSize: 50,
      }),
    );
    const onFocusRequestHandled = vi.fn();
    interactionBoundary = document.createElement("div");
    document.body.appendChild(interactionBoundary);

    renderTimeline();
    await flushAsync();
    await selectOptionByText('[data-testid="upstream-attempt-type-filter"]', /image/i);
    expect(topicSnapshotMock).toHaveBeenLastCalledWith(
      101,
      expect.objectContaining({
        type: "image",
        page: 1,
      }),
    );
    scrollIntoViewMock.mockClear();
    renderTimeline({
      focusedAttemptId: "YG7P25XG",
      focusVersion: 1,
      boundary: interactionBoundary,
      onFocusRequestHandled,
    });
    await flushAsync();

    expect(locateUpstreamAccountAttempt).toHaveBeenCalledWith(
      101,
      "YG7P25XG",
      expect.objectContaining({
        pageSize: 50,
        signal: expect.any(AbortSignal),
      }),
    );
    expect(onFocusRequestHandled).not.toHaveBeenCalled();
    expect(host?.textContent).toMatch(/All types|全部类型/);
    const topicAfterLocate = subscriptionTopicMock.mock.calls.at(-1)?.[0];
    expect(topicAfterLocate?.params?.page).toBe("2");
    expect(
      host?.querySelector<HTMLElement>('[data-testid="account-attempt-record-YG7P25XG"]'),
    ).toBeNull();
    expect(scrollIntoViewMock).not.toHaveBeenCalled();

    act(() => {
      emitTopicSnapshot(
        buildTopicDescriptor("upstream-account-attempts.window", {
          accountId: 101,
          page: 2,
          pageSize: 50,
        }),
        attemptListResponse({
          items: [focusedAttempt],
          total: 100,
          page: 2,
          pageSize: 50,
        }),
      );
    });
    await flushAsync();

    const record = host?.querySelector<HTMLElement>(
      '[data-testid="account-attempt-record-YG7P25XG"]',
    );
    expect(record).not.toBeNull();
    expect(onFocusRequestHandled).toHaveBeenCalledWith(1);
    const snapshotCallsAfterLocate = topicSnapshotMock.mock.calls.length;
    renderTimeline({
      boundary: interactionBoundary,
      onFocusRequestHandled,
    });
    await flushAsync();
    expect(topicSnapshotMock).toHaveBeenCalledTimes(snapshotCallsAfterLocate);
    const topicAfterFocusAcknowledged = subscriptionTopicMock.mock.calls.at(-1)?.[0];
    expect(topicAfterFocusAcknowledged?.params?.page).toBe("2");
    expect(
      host?.querySelector<HTMLElement>('[data-testid="account-attempt-record-YG7P25XG"]'),
    ).not.toBeNull();
    expect(scrollIntoViewMock).toHaveBeenCalledWith({
      behavior: "smooth",
      block: "nearest",
    });
    expect(record?.dataset.focusVisible).toBe("true");
    expect(record?.getAttribute("aria-current")).toBe("true");
    expect(record?.classList.contains("invocation-workflow-attempt--focused")).toBe(true);
    expect(record?.textContent).toMatch(/关键诊断|key diagnostics/i);
    expect(record?.textContent).toMatch(/上游 HTTP 状态|upstream http/i);

    act(() => {
      interactionBoundary?.dispatchEvent(new Event("pointerdown", { bubbles: true }));
    });
    expect(record?.dataset.focusVisible).toBe("true");

    act(() => {
      vi.advanceTimersByTime(1_499);
    });
    expect(record?.dataset.focusVisible).toBe("true");

    act(() => {
      vi.advanceTimersByTime(1);
    });
    expect(record?.dataset.focusVisible).toBe("false");

    renderTimeline({
      focusedAttemptId: "YG7P25XG",
      focusVersion: 2,
      boundary: interactionBoundary,
    });
    await flushAsync();
    const refocusedRecord = host?.querySelector<HTMLElement>(
      '[data-testid="account-attempt-record-YG7P25XG"]',
    );
    expect(refocusedRecord?.dataset.focusVisible).toBe("true");
    expect(scrollIntoViewMock.mock.calls.length).toBeGreaterThanOrEqual(2);
  });

  it("acknowledges an in-flight deep link when the user changes a filter", async () => {
    const focusedAttempt = makeAttempt({
      attemptId: "FILTERCANCEL1",
      invokeId: "FILTERCANCEL1INVOKE",
      createdAt: "2026-07-11T12:00:00.000Z",
    });
    let resolvePageTwo: ((response: UpstreamAccountAttemptListResponse) => void) | undefined;
    topicSnapshotMock.mockImplementation(async (_accountId, options) => {
      if (options.page === 2) {
        return await new Promise<UpstreamAccountAttemptListResponse>((resolve) => {
          resolvePageTwo = resolve;
        });
      }
      return attemptListResponse({
        items: [],
        total: 0,
        page: options.page ?? 1,
        pageSize: 50,
      });
    });
    vi.mocked(locateUpstreamAccountAttempt).mockResolvedValue(
      attemptListResponse({
        items: [focusedAttempt],
        total: 100,
        page: 2,
        pageSize: 50,
      }),
    );
    const onFocusRequestHandled = vi.fn();

    renderTimeline({
      focusedAttemptId: "FILTERCANCEL1",
      focusVersion: 1,
      onFocusRequestHandled,
    });
    await flushAsync();

    expect(resolvePageTwo).toBeDefined();
    expect(onFocusRequestHandled).not.toHaveBeenCalled();

    await selectOptionByText('[data-testid="upstream-attempt-type-filter"]', /image/i);

    expect(onFocusRequestHandled).toHaveBeenCalledWith(1);
    expect(onFocusRequestHandled).toHaveBeenCalledTimes(1);
    expect(topicSnapshotMock).toHaveBeenLastCalledWith(
      101,
      expect.objectContaining({
        type: "image",
        page: 1,
        pageSize: 50,
      }),
    );

    act(() => {
      resolvePageTwo?.(
        attemptListResponse({
          items: [focusedAttempt],
          total: 100,
          page: 2,
          pageSize: 50,
        }),
      );
    });
  });

  it("does not restore a cancelled deep link when its locate request completes", async () => {
    const focusedAttempt = makeAttempt({
      attemptId: "LATELOCATE1",
      invokeId: "LATELOCATE1INVOKE",
      createdAt: "2026-07-11T12:00:00.000Z",
    });
    let resolveLocate: ((response: UpstreamAccountAttemptListResponse) => void) | undefined;
    vi.mocked(locateUpstreamAccountAttempt).mockImplementation(
      async () =>
        await new Promise<UpstreamAccountAttemptListResponse>((resolve) => {
          resolveLocate = resolve;
        }),
    );
    topicSnapshotMock.mockResolvedValue(
      attemptListResponse({
        items: [],
        total: 0,
        page: 1,
        pageSize: 50,
      }),
    );
    const onFocusRequestHandled = vi.fn();

    renderTimeline({
      focusedAttemptId: "LATELOCATE1",
      focusVersion: 1,
      onFocusRequestHandled,
    });
    await flushAsync();

    expect(resolveLocate).toBeDefined();
    await selectOptionByText('[data-testid="upstream-attempt-type-filter"]', /image/i);
    expect(onFocusRequestHandled).toHaveBeenCalledWith(1);

    act(() => {
      resolveLocate?.(
        attemptListResponse({
          items: [focusedAttempt],
          total: 100,
          page: 2,
          pageSize: 50,
        }),
      );
    });
    await flushAsync();

    const topicAfterLateLocate = subscriptionTopicMock.mock.calls.at(-1)?.[0];
    expect(topicAfterLateLocate?.params).toEqual(
      expect.objectContaining({
        type: "image",
        page: "1",
        pageSize: "50",
      }),
    );
    expect(
      host?.querySelector<HTMLElement>('[data-testid="account-attempt-record-LATELOCATE1"]'),
    ).toBeNull();
  });

  it("relocates a deep link when its first topic snapshot has shifted to the next page", async () => {
    const focusedAttempt = makeAttempt({
      attemptId: "SHIFT0001",
      invokeId: "SHIFT0001INVOKE",
      createdAt: "2026-07-11T12:00:00.000Z",
    });
    let resolvePageTwo: ((response: UpstreamAccountAttemptListResponse) => void) | undefined;
    topicSnapshotMock.mockImplementation(async (_accountId, options) => {
      if (options.page === 2) {
        return await new Promise<UpstreamAccountAttemptListResponse>((resolve) => {
          resolvePageTwo = resolve;
        });
      }
      return attemptListResponse({
        items: [focusedAttempt],
        total: 101,
        page: 3,
        pageSize: 50,
      });
    });
    vi.mocked(locateUpstreamAccountAttempt)
      .mockResolvedValueOnce(
        attemptListResponse({
          items: [focusedAttempt],
          total: 101,
          page: 2,
          pageSize: 50,
        }),
      )
      .mockResolvedValueOnce(
        attemptListResponse({
          items: [focusedAttempt],
          total: 101,
          page: 3,
          pageSize: 50,
        }),
      );
    const onFocusRequestHandled = vi.fn();

    renderTimeline({
      focusedAttemptId: "SHIFT0001",
      focusVersion: 1,
      onFocusRequestHandled,
    });
    await flushAsync();

    expect(resolvePageTwo).toBeDefined();
    expect(onFocusRequestHandled).not.toHaveBeenCalled();
    act(() => {
      resolvePageTwo?.(
        attemptListResponse({
          items: [],
          total: 101,
          page: 2,
          pageSize: 50,
        }),
      );
    });
    await flushAsync();
    await flushAsync();
    await flushAsync();

    expect(locateUpstreamAccountAttempt).toHaveBeenCalledTimes(2);
    const topicAfterRelocate = subscriptionTopicMock.mock.calls.at(-1)?.[0];
    expect(topicAfterRelocate?.params?.page).toBe("3");
    const record = host?.querySelector<HTMLElement>(
      '[data-testid="account-attempt-record-SHIFT0001"]',
    );
    expect(record).not.toBeNull();
    expect(record?.dataset.focusVisible).toBe("true");
    expect(onFocusRequestHandled).toHaveBeenCalledWith(1);
    expect(scrollIntoViewMock).toHaveBeenCalledWith({
      behavior: "smooth",
      block: "nearest",
    });
  });

  it("relocates an acknowledged deep link when a later authoritative snapshot shifts its page", async () => {
    const focusedAttempt = makeAttempt({
      attemptId: "SHIFTACK1",
      invokeId: "SHIFTACK1INVOKE",
      createdAt: "2026-07-11T12:00:00.000Z",
    });
    const pageTwoDescriptor = buildTopicDescriptor("upstream-account-attempts.window", {
      accountId: 101,
      page: 2,
      pageSize: 50,
    });
    topicSnapshotCache.set(
      getTopicDescriptorKey(pageTwoDescriptor),
      attemptListResponse({
        items: [focusedAttempt],
        total: 101,
        page: 2,
        pageSize: 50,
      }),
    );
    let resolvePageThree: ((response: UpstreamAccountAttemptListResponse) => void) | undefined;
    topicSnapshotMock.mockImplementation(async (_accountId, options) => {
      if (options.page === 3) {
        return await new Promise<UpstreamAccountAttemptListResponse>((resolve) => {
          resolvePageThree = resolve;
        });
      }
      return attemptListResponse({
        items: [focusedAttempt],
        total: 101,
        page: options.page ?? 1,
        pageSize: 50,
      });
    });
    vi.mocked(locateUpstreamAccountAttempt)
      .mockResolvedValueOnce(
        attemptListResponse({
          items: [focusedAttempt],
          total: 101,
          page: 2,
          pageSize: 50,
        }),
      )
      .mockResolvedValueOnce(
        attemptListResponse({
          items: [focusedAttempt],
          total: 101,
          page: 3,
          pageSize: 50,
        }),
      );
    const onFocusRequestHandled = vi.fn();

    renderTimeline({
      focusedAttemptId: "SHIFTACK1",
      focusVersion: 1,
      onFocusRequestHandled,
    });
    await flushAsync();
    await flushAsync();

    const firstFocusedCard = host?.querySelector<HTMLElement>(
      '[data-testid="account-attempt-record-SHIFTACK1"]',
    );
    expect(firstFocusedCard?.dataset.focusVisible).toBe("true");
    expect(onFocusRequestHandled).toHaveBeenCalledTimes(1);
    const scrollCallsBeforeShift = scrollIntoViewMock.mock.calls.length;

    act(() => {
      emitTopicSnapshot(
        pageTwoDescriptor,
        attemptListResponse({
          items: [],
          total: 101,
          page: 2,
          pageSize: 50,
        }),
      );
    });
    await flushAsync();
    await flushAsync();
    await flushAsync();

    expect(locateUpstreamAccountAttempt).toHaveBeenCalledTimes(2);
    expect(onFocusRequestHandled).toHaveBeenCalledTimes(1);
    expect(scrollIntoViewMock).toHaveBeenCalledTimes(scrollCallsBeforeShift);
    const topicAfterRelocate = subscriptionTopicMock.mock.calls.at(-1)?.[0];
    expect(topicAfterRelocate?.params?.page).toBe("3");
    expect(resolvePageThree).toBeDefined();

    act(() => {
      resolvePageThree?.(
        attemptListResponse({
          items: [focusedAttempt],
          total: 101,
          page: 3,
          pageSize: 50,
        }),
      );
    });
    await flushAsync();
    await flushAsync();

    const relocatedCard = host?.querySelector<HTMLElement>(
      '[data-testid="account-attempt-record-SHIFTACK1"]',
    );
    expect(relocatedCard?.dataset.focusVisible).toBe("true");
    expect(onFocusRequestHandled).toHaveBeenCalledTimes(1);
    expect(scrollIntoViewMock.mock.calls.length).toBeGreaterThan(scrollCallsBeforeShift);
  });

  it("waits for a network frame before spending a deep-link relocation", async () => {
    const focusedAttempt = makeAttempt({
      attemptId: "CACHEMOVE1",
      invokeId: "CACHEMOVE1INVOKE",
      createdAt: "2026-07-11T12:00:00.000Z",
    });
    const pageTwoDescriptor = buildTopicDescriptor("upstream-account-attempts.window", {
      accountId: 101,
      page: 2,
      pageSize: 50,
    });
    topicSnapshotCache.set(
      getTopicDescriptorKey(pageTwoDescriptor),
      attemptListResponse({
        items: [],
        total: 101,
        page: 2,
        pageSize: 50,
      }),
    );
    topicSnapshotMock.mockImplementation(async (_accountId, options) =>
      attemptListResponse({
        items: options.page === 3 ? [focusedAttempt] : [],
        total: 101,
        page: options.page ?? 1,
        pageSize: 50,
      }),
    );
    vi.mocked(locateUpstreamAccountAttempt)
      .mockResolvedValueOnce(
        attemptListResponse({
          items: [focusedAttempt],
          total: 101,
          page: 2,
          pageSize: 50,
        }),
      )
      .mockResolvedValueOnce(
        attemptListResponse({
          items: [focusedAttempt],
          total: 101,
          page: 3,
          pageSize: 50,
        }),
      );
    const onFocusRequestHandled = vi.fn();

    renderTimeline({
      focusedAttemptId: "CACHEMOVE1",
      focusVersion: 1,
      onFocusRequestHandled,
    });
    await flushAsync();
    await flushAsync();

    expect(locateUpstreamAccountAttempt).toHaveBeenCalledTimes(1);
    expect(onFocusRequestHandled).not.toHaveBeenCalled();

    act(() => {
      emitTopicSnapshot(
        pageTwoDescriptor,
        attemptListResponse({
          items: [focusedAttempt],
          total: 101,
          page: 2,
          pageSize: 50,
        }),
      );
    });
    await flushAsync();
    await flushAsync();

    expect(onFocusRequestHandled).toHaveBeenCalledWith(1);
    expect(locateUpstreamAccountAttempt).toHaveBeenCalledTimes(1);

    act(() => {
      emitTopicSnapshot(
        pageTwoDescriptor,
        attemptListResponse({
          items: [],
          total: 101,
          page: 2,
          pageSize: 50,
        }),
      );
    });
    await flushAsync();
    await flushAsync();
    await flushAsync();

    expect(locateUpstreamAccountAttempt).toHaveBeenCalledTimes(2);
    const topicAfterRelocate = subscriptionTopicMock.mock.calls.at(-1)?.[0];
    expect(topicAfterRelocate?.params?.page).toBe("3");
    const relocatedCard = host?.querySelector<HTMLElement>(
      '[data-testid="account-attempt-record-CACHEMOVE1"]',
    );
    expect(relocatedCard?.dataset.focusVisible).toBe("true");
    expect(onFocusRequestHandled).toHaveBeenCalledTimes(1);
  });

  it("shows locate unavailable feedback when the focused attempt is outside the locate window", async () => {
    vi.mocked(locateUpstreamAccountAttempt).mockRejectedValue(new Error("404 not found"));

    renderTimeline({
      focusedAttemptId: "MISS1234",
      focusVersion: 1,
    });
    await flushAsync();

    expect(host?.textContent).toMatch(/7-day retention window|7 天保留范围|7 天窗口/i);
  });

  it("reconciles a focused pending attempt in place without duplicating its expanded card", async () => {
    const pending = makeAttempt({
      attemptId: "LIVE0001",
      status: "pending",
      phase: "waiting_first_byte",
      httpStatus: null,
    });
    topicSnapshotMock.mockResolvedValue(attemptListResponse({ items: [pending] }));
    vi.mocked(locateUpstreamAccountAttempt).mockResolvedValue(
      attemptListResponse({ items: [pending], total: 1, page: 1, pageSize: 50 }),
    );

    renderTimeline({ focusedAttemptId: "LIVE0001", focusVersion: 1 });
    await flushAsync();

    const pendingCard = host?.querySelector<HTMLElement>(
      '[data-testid="account-attempt-record-LIVE0001"]',
    );
    expect(pendingCard).not.toBeNull();
    expect(pendingCard?.textContent).toMatch(/时间细分|timing breakdown/i);
    expect(pendingCard?.dataset.focusVisible).toBe("true");

    const terminal = makeAttempt({
      ...pending,
      status: "success",
      phase: "completed",
      httpStatus: 200,
      downstreamHttpStatus: 200,
      finishedAt: "2026-07-11T12:00:03.000Z",
    });
    const newer = makeAttempt({
      attemptId: "LIVE0002",
      invokeId: "LIVE0002INVOKE",
      status: "pending",
      phase: "connecting",
      httpStatus: null,
      createdAt: "2026-07-11T12:00:04.000Z",
    });
    act(() => {
      emitTopicSnapshot(
        buildTopicDescriptor("upstream-account-attempts.window", {
          accountId: 101,
          page: 1,
          pageSize: 50,
        }),
        attemptListResponse({ items: [newer, terminal], total: 2 }),
      );
    });
    await flushAsync();

    const updatedCard = host?.querySelector<HTMLElement>(
      '[data-testid="account-attempt-record-LIVE0001"]',
    );
    expect(updatedCard).toBe(pendingCard);
    expect(host?.querySelectorAll('[data-testid="account-attempt-record-LIVE0001"]')).toHaveLength(
      1,
    );
    expect(host?.querySelectorAll('[data-testid="account-attempt-record-LIVE0002"]')).toHaveLength(
      1,
    );
    expect(updatedCard?.textContent).toMatch(/HTTP 200|上游 HTTP 200|success/i);
    expect(updatedCard?.textContent).toMatch(/时间细分|timing breakdown/i);
    expect(updatedCard?.dataset.focusVisible).toBe("true");
  });

  it("releases and reacquires the account attempt topic listener with the request tab", async () => {
    topicSnapshotMock.mockResolvedValue(attemptListResponse({ items: [] }));
    renderTimeline();
    await flushAsync();
    expect(topicListeners.size).toBe(1);

    renderTimeline({ visible: false });
    await flushAsync();
    expect(topicListeners.size).toBe(0);

    renderTimeline({ visible: true });
    await flushAsync();
    expect(topicListeners.size).toBe(1);

    act(() => {
      root?.unmount();
    });
    root = null;
    expect(topicListeners.size).toBe(0);
  });
});
