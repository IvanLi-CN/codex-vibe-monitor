import type { Meta, StoryObj } from "@storybook/react-vite";
import { type ReactNode, useEffect, useMemo, useRef } from "react";
import { expect, userEvent, waitFor, within } from "storybook/test";
import { I18nProvider } from "../../i18n";
import type {
  ApiInvocation,
  ApiInvocationRecordDetailResponse,
  ApiInvocationResponseBodyResponse,
  ApiInvocationWorkflowDetailResponse,
  ApiPoolUpstreamRequestAttempt,
} from "../../lib/api";
import { InvocationRecordsTable } from "./InvocationRecordsTable";
import {
  createStoryForwardProxyBindingNodes,
  createStoryInvocationRecordDetailsById,
  createStoryInvocationResponseBodiesById,
  createStoryPoolAttemptsByInvokeId,
  STORYBOOK_FIRST_RESPONSE_BYTE_SEMANTICS_RECORDS,
  STORYBOOK_INVOCATION_RECORDS,
  STORYBOOK_PROXY_ERROR_CONTRACT_RECORDS,
} from "./invocationRecordsStoryFixtures";

type PoolAttemptsByInvokeId = Record<string, ApiPoolUpstreamRequestAttempt[]>;
type InvocationRecordDetailsById = Record<number, ApiInvocationRecordDetailResponse>;
type InvocationResponseBodiesById = Record<number, ApiInvocationResponseBodyResponse>;
type InvocationWorkflowDetailsById = Record<number, ApiInvocationWorkflowDetailResponse>;

const POOL_ROUTING_ACCOUNT_STATE_RECORDS: ApiInvocation[] = [
  {
    ...STORYBOOK_INVOCATION_RECORDS.find((record) => record.invokeId === "inv_story_6106")!,
    id: 6206,
    invokeId: "inv_story_pool_routing_account_named",
    status: "running",
    upstreamAccountId: 58,
    upstreamAccountName: "Pool Zeta 58",
  },
  {
    ...STORYBOOK_INVOCATION_RECORDS[0]!,
    id: 6207,
    invokeId: "inv_story_pool_routing_account_missing",
    status: "pending",
    upstreamAccountId: undefined,
    upstreamAccountName: undefined,
    totalTokens: 0,
    cost: undefined,
    tTotalMs: null,
  },
  {
    ...STORYBOOK_INVOCATION_RECORDS[0]!,
    id: 6208,
    invokeId: "inv_story_pool_routing_account_terminal",
    status: "success",
    upstreamAccountId: 17,
    upstreamAccountName: "Pool Alpha 17",
  },
];

const WARNING_SUCCESS_RECORDS: ApiInvocation[] = [
  {
    ...STORYBOOK_INVOCATION_RECORDS[0]!,
    id: 6401,
    invokeId: "inv_story_warning_success",
    status: "warning_success",
    failureKind: "downstream_closed",
    failureClass: "none",
    errorMessage: "[downstream_closed] downstream closed while streaming upstream response",
    upstreamAccountId: 42,
    upstreamAccountName: "Pool Alpha 42",
    totalTokens: 167_710,
    cost: 0.0629,
    tUpstreamTtfbMs: 1_131,
    tUpstreamStreamMs: 15_849,
    tTotalMs: 16_980,
  },
];

const ENDPOINT_CHIP_RECORDS: ApiInvocation[] = [
  {
    ...STORYBOOK_INVOCATION_RECORDS[0]!,
    id: 6501,
    invokeId: "inv_story_records_image_gen",
    endpoint: "/v1/images/generations",
    imageIntent: "yes",
    model: "gpt-image-1",
  },
  {
    ...STORYBOOK_INVOCATION_RECORDS[0]!,
    id: 6502,
    invokeId: "inv_story_records_image_edit",
    endpoint: "/v1/images/edits",
    imageIntent: "direct_image",
    model: "gpt-image-1",
  },
  {
    ...STORYBOOK_INVOCATION_RECORDS[0]!,
    id: 6503,
    invokeId: "inv_story_records_image_generic",
    endpoint: "/v1/images/variations",
    model: "gpt-image-1",
  },
  {
    ...STORYBOOK_INVOCATION_RECORDS[0]!,
    id: 6504,
    invokeId: "inv_story_records_raw_endpoint",
    endpoint: "/v1/responses/experimental",
  },
];

const TOKEN_COST_AUDIT_WARNING_RECORDS: ApiInvocation[] = [
  {
    ...STORYBOOK_INVOCATION_RECORDS[0]!,
    id: 6601,
    invokeId: "inv_story_cost_audit_warning",
    model: "gpt-5.5",
    requestModel: "gpt-5.4",
    responseModel: "gpt-5.5",
    imageIntent: "yes",
    inputTokens: 132_219,
    cacheWriteTokens: 5_632,
    cacheInputTokens: 126_587,
    outputTokens: 59,
    reasoningTokens: undefined,
    totalTokens: 132_278,
    cost: 0.6375,
    costAudit: {
      recorded: {
        total: 0.6375,
      },
      local: {
        input: 0,
        cacheWrite: 0.451,
        cacheRead: 0.1266,
        output: 0.0189,
        reasoning: 0,
        total: 0.5965,
      },
      mismatch: true,
      reason: "price_version_changed",
      absoluteDiffUsd: 0.041,
      recordedPriceVersion: "openai-standard-2026-07-01@response-tier",
      localPriceVersion: "openai-standard-2026-07-20@response-tier",
    },
  },
];

const RESPONSE_BODY_WIDTH_GUARD_RECORD_ID = 6701;
const RESPONSE_BODY_WIDTH_GUARD_RECORDS: ApiInvocation[] = [
  {
    ...STORYBOOK_INVOCATION_RECORDS[0]!,
    id: RESPONSE_BODY_WIDTH_GUARD_RECORD_ID,
    invokeId: "inv_story_response_body_width_guard",
    status: "failed",
    failureClass: "service_failure",
    failureKind: "upstream_stream_error",
    responseContentEncoding: "zstd",
    upstreamAccountId: 77,
    upstreamAccountName: "Pool Width Guard",
  },
];
const RESPONSE_BODY_WIDTH_GUARD_BODY = [
  {
    type: "response.created",
    response: {
      id: `resp_${"width_guard_".repeat(36)}`,
      status: "in_progress",
      model: "gpt-5.4",
    },
    sequence_number: 0,
  },
  {
    type: "response.output_item.added",
    widthGuardProbe: `visible_unbroken_response_body_probe_${"0123456789abcdef".repeat(144)}`,
    output_index: 0,
    item: {
      id: `msg_${"unbroken_payload_segment_".repeat(42)}`,
      type: "message",
      content: [
        {
          type: "output_text",
          text: `This unbroken response body field must stay inside the payload scroller: ${"0123456789abcdef".repeat(96)}`,
        },
      ],
    },
    sequence_number: 1,
  },
  {
    type: "response.completed",
    response: {
      status: "completed",
      service_tier: "default",
    },
    sequence_number: 2,
  },
]
  .map((event) => `event: ${event.type}\ndata: ${JSON.stringify(event)}`)
  .join("\n\n");
const RESPONSE_BODY_WIDTH_GUARD_RESPONSE_BODIES_BY_ID: InvocationResponseBodiesById = {
  [RESPONSE_BODY_WIDTH_GUARD_RECORD_ID]: {
    available: true,
    bodyText: RESPONSE_BODY_WIDTH_GUARD_BODY,
    captureSource: "raw_file",
    bodySize: RESPONSE_BODY_WIDTH_GUARD_BODY.length,
    bodyTruncated: false,
    detailLevel: "full",
    headers: { contentEncoding: "zstd" },
    routing: { forwardedChunkCount: 83, usageObserved: true },
  },
};

function buildStoryWorkflowUsage(record: ApiInvocation) {
  if (!record.costAudit) return null;
  return {
    inputTokens: record.inputTokens ?? null,
    cacheWriteTokens: record.cacheWriteTokens ?? null,
    cacheInputTokens: record.cacheInputTokens ?? null,
    outputTokens: record.outputTokens ?? null,
    reasoningTokens: record.reasoningTokens ?? null,
    totalTokens: record.totalTokens ?? null,
    cost: record.cost ?? null,
    tokens: {
      input: record.inputTokens ?? null,
      cacheWrite: record.cacheWriteTokens ?? null,
      cacheRead: record.cacheInputTokens ?? null,
      output: record.outputTokens ?? null,
      reasoning: record.reasoningTokens ?? null,
      total: record.totalTokens ?? null,
    },
    costs: {
      recorded: record.costAudit.recorded ?? null,
      local: record.costAudit.local ?? null,
    },
    audit: record.costAudit,
  };
}

function buildStoryWorkflowDetailResponse(
  record: ApiInvocation,
): ApiInvocationWorkflowDetailResponse {
  return {
    hero: {
      recordId: record.id,
      invokeId: record.invokeId,
      promptCacheKey: record.promptCacheKey ?? null,
      routeMode: record.routeMode ?? null,
      endpoint: record.endpoint ?? null,
      requestModel: record.requestModel ?? record.model ?? null,
      responseModel: record.responseModel ?? record.model ?? null,
      finalStatus: record.status,
      failureClass: record.failureClass ?? null,
      downstreamStatusCode: record.downstreamStatusCode ?? null,
      upstreamAccountId: record.upstreamAccountId ?? null,
      upstreamAccountName: record.upstreamAccountName ?? null,
      totalDurationMs: record.tTotalMs ?? null,
      timelineAttemptCount: 1,
      poolAttemptCount: record.routeMode === "pool" ? 1 : null,
      totalTokens: record.totalTokens ?? null,
      cost: record.cost ?? null,
      occurredAt: record.occurredAt,
    },
    timeline: [
      {
        blockId: `route-${record.id}`,
        kind: "routingDecision",
        occurredAt: record.occurredAt,
        title: "Route summary",
        subtitle: `${record.routeMode ?? "direct"} ${record.endpoint ?? ""}`.trim(),
        status: record.routeMode ?? "direct",
        detail: {
          request: {
            endpoint: record.endpoint ?? null,
            routeMode: record.routeMode ?? null,
            requestModel: record.requestModel ?? record.model ?? null,
            responseModel: record.responseModel ?? record.model ?? null,
            reasoningEffort: record.reasoningEffort ?? null,
            imageIntent: record.imageIntent ?? null,
            promptCacheKey: record.promptCacheKey ?? null,
            requesterIp: record.requesterIp ?? null,
            routing: {
              proxyDisplayName: record.proxyDisplayName ?? null,
              transport: record.transport ?? null,
            },
          },
        },
      },
      {
        blockId: `attempt-${record.id}`,
        kind: "attempt",
        occurredAt: record.occurredAt,
        title: "Attempt 1",
        subtitle: record.upstreamAccountName ?? record.proxyDisplayName ?? "Direct",
        status: record.status,
        attempt: {
          synthetic: true,
          occurredAt: record.occurredAt,
          endpoint: record.endpoint ?? "/v1/responses",
          stickyKey: record.stickyKey ?? null,
          upstreamAccountId: record.upstreamAccountId ?? null,
          upstreamAccountName: record.upstreamAccountName ?? null,
          requestModel: record.requestModel ?? record.model ?? null,
          responseModel: record.responseModel ?? record.model ?? null,
          attemptIndex: 1,
          distinctAccountIndex: 1,
          sameAccountRetryIndex: 1,
          requesterIp: record.requesterIp ?? null,
          status: record.status ?? "success",
          httpStatus: record.downstreamStatusCode ?? 200,
          downstreamHttpStatus: record.downstreamStatusCode ?? 200,
          connectLatencyMs: record.tUpstreamConnectMs ?? null,
          firstByteLatencyMs: record.tUpstreamTtfbMs ?? null,
          streamLatencyMs: record.tUpstreamStreamMs ?? null,
          responseSummary: {
            status: record.status ?? "success",
            httpStatus: record.downstreamStatusCode ?? 200,
            responseContentEncoding: record.responseContentEncoding ?? null,
            serviceTier: record.serviceTier ?? null,
            billingServiceTier: record.billingServiceTier ?? null,
            usage: buildStoryWorkflowUsage(record),
          },
        },
      },
    ],
    reconstructed: true,
    partial: false,
  };
}

function createStoryWorkflowDetailsById(
  records: readonly ApiInvocation[],
): InvocationWorkflowDetailsById {
  return Object.fromEntries(
    records
      .filter((record) => Number.isFinite(record.id) && record.id > 0)
      .map((record) => [record.id, buildStoryWorkflowDetailResponse(record)]),
  );
}

function buildResponseBodyWidthGuardWorkflowDetails(): InvocationWorkflowDetailsById {
  const record = RESPONSE_BODY_WIDTH_GUARD_RECORDS[0]!;
  const detail = buildStoryWorkflowDetailResponse(record);
  const attempt = detail.timeline.find((entry) => entry.attempt)?.attempt;

  if (attempt) {
    attempt.responseSummary = {
      ...(attempt.responseSummary ?? {}),
      responseBodyCapture: {
        availableAtInvocationLevel: true,
        size: RESPONSE_BODY_WIDTH_GUARD_BODY.length,
        truncated: false,
        detailLevel: "full",
      },
    };
  }

  return { [RESPONSE_BODY_WIDTH_GUARD_RECORD_ID]: detail };
}

type StorybookPoolAttemptsRegistry = {
  originalFetch: typeof window.fetch;
  providers: Map<
    symbol,
    () => {
      poolAttemptsByInvokeId: PoolAttemptsByInvokeId;
      detailsById: InvocationRecordDetailsById;
      responseBodiesById: InvocationResponseBodiesById;
      workflowDetailsById: InvocationWorkflowDetailsById;
    }
  >;
};

declare global {
  interface Window {
    __storybookPoolAttemptsRegistry__?: StorybookPoolAttemptsRegistry;
  }
}

function StorySurface({ children }: { children: React.ReactNode }) {
  return (
    <div className="min-h-screen bg-base-200 px-6 py-6 text-base-content">
      <div className="app-shell-boundary">{children}</div>
    </div>
  );
}

function jsonResponse(payload: unknown) {
  return new Response(JSON.stringify(payload), {
    status: 200,
    headers: {
      "Content-Type": "application/json",
    },
  });
}

function ensureStorybookPoolAttemptsRegistry() {
  if (typeof window === "undefined") return null;

  const existingRegistry = window.__storybookPoolAttemptsRegistry__;
  if (existingRegistry) return existingRegistry;

  const originalFetch = window.fetch.bind(window);
  const providers: StorybookPoolAttemptsRegistry["providers"] = new Map();

  const mockedFetch: typeof window.fetch = async (input, init) => {
    const requestUrl =
      typeof input === "string" ? input : input instanceof URL ? input.toString() : input.url;
    const url = new URL(requestUrl, window.location.origin);
    const poolAttemptsMatch = url.pathname.match(/^\/api\/invocations\/([^/]+)\/pool-attempts$/);
    const proxyBindingNodesMatch = url.pathname === "/api/pool/forward-proxy-binding-nodes";
    const detailMatch = url.pathname.match(/^\/api\/invocations\/(\d+)\/detail$/);
    const responseBodyMatch = url.pathname.match(/^\/api\/invocations\/(\d+)\/response-body$/);
    const workflowDetailMatch = url.pathname.match(/^\/api\/invocations\/(\d+)\/workflow-detail$/);

    if (poolAttemptsMatch) {
      const invokeId = decodeURIComponent(poolAttemptsMatch[1] ?? "");
      const providerGetters = Array.from(providers.values()).reverse();

      for (const getAttemptsByInvokeId of providerGetters) {
        const attempts = getAttemptsByInvokeId().poolAttemptsByInvokeId[invokeId];
        if (attempts) {
          return jsonResponse(attempts);
        }
      }

      return jsonResponse([]);
    }

    if (proxyBindingNodesMatch) {
      return jsonResponse(createStoryForwardProxyBindingNodes(url.searchParams.getAll("key")));
    }

    if (detailMatch) {
      const recordId = Number(detailMatch[1] ?? "0");
      const providerGetters = Array.from(providers.values()).reverse();

      for (const getDetailsById of providerGetters) {
        const detail = getDetailsById().detailsById[recordId];
        if (detail) {
          return jsonResponse(detail);
        }
      }

      return jsonResponse({ id: recordId, abnormalResponseBody: null });
    }

    if (responseBodyMatch) {
      const recordId = Number(responseBodyMatch[1] ?? "0");
      const providerGetters = Array.from(providers.values()).reverse();

      for (const getResponseBodiesById of providerGetters) {
        const responseBody = getResponseBodiesById().responseBodiesById[recordId];
        if (responseBody) {
          return jsonResponse(responseBody);
        }
      }

      return jsonResponse({ available: false, unavailableReason: "missing_body" });
    }

    if (workflowDetailMatch) {
      const recordId = Number(workflowDetailMatch[1] ?? "0");
      const providerGetters = Array.from(providers.values()).reverse();

      for (const getWorkflowDetailsById of providerGetters) {
        const workflowDetail = getWorkflowDetailsById().workflowDetailsById[recordId];
        if (workflowDetail) {
          return jsonResponse(workflowDetail);
        }
      }

      return new Response(JSON.stringify({ error: "missing workflow detail story fixture" }), {
        status: 404,
        headers: {
          "Content-Type": "application/json",
        },
      });
    }

    return originalFetch(input, init);
  };

  window.fetch = mockedFetch;
  window.__storybookPoolAttemptsRegistry__ = {
    originalFetch,
    providers,
  };

  return window.__storybookPoolAttemptsRegistry__;
}

function StorybookPoolAttemptsMock({
  children,
  records,
  responseBodiesByIdOverrides,
  workflowDetailsByIdOverrides,
}: {
  children: ReactNode;
  records: typeof STORYBOOK_INVOCATION_RECORDS;
  responseBodiesByIdOverrides?: InvocationResponseBodiesById;
  workflowDetailsByIdOverrides?: InvocationWorkflowDetailsById;
}) {
  const poolAttemptsByInvokeId = useMemo(
    () => createStoryPoolAttemptsByInvokeId(records),
    [records],
  );
  const detailsById = useMemo(() => createStoryInvocationRecordDetailsById(records), [records]);
  const responseBodiesById = useMemo(
    () => ({
      ...createStoryInvocationResponseBodiesById(records),
      ...(responseBodiesByIdOverrides ?? {}),
    }),
    [records, responseBodiesByIdOverrides],
  );
  const workflowDetailsById = useMemo(
    () => ({
      ...createStoryWorkflowDetailsById(records),
      ...(workflowDetailsByIdOverrides ?? {}),
    }),
    [records, workflowDetailsByIdOverrides],
  );
  const poolAttemptsByInvokeIdRef = useRef(poolAttemptsByInvokeId);
  const detailsByIdRef = useRef(detailsById);
  const responseBodiesByIdRef = useRef(responseBodiesById);
  const workflowDetailsByIdRef = useRef(workflowDetailsById);
  const providerIdRef = useRef<symbol>(Symbol("storybook-pool-attempts"));

  poolAttemptsByInvokeIdRef.current = poolAttemptsByInvokeId;
  detailsByIdRef.current = detailsById;
  responseBodiesByIdRef.current = responseBodiesById;
  workflowDetailsByIdRef.current = workflowDetailsById;

  useEffect(() => {
    const registry = ensureStorybookPoolAttemptsRegistry();
    if (!registry) return;

    const providerId = providerIdRef.current;

    registry.providers.set(providerId, () => ({
      poolAttemptsByInvokeId: poolAttemptsByInvokeIdRef.current,
      detailsById: detailsByIdRef.current,
      responseBodiesById: responseBodiesByIdRef.current,
      workflowDetailsById: workflowDetailsByIdRef.current,
    }));

    return () => {
      const activeRegistry = window.__storybookPoolAttemptsRegistry__;
      if (!activeRegistry) return;

      activeRegistry.providers.delete(providerId);
      if (activeRegistry.providers.size === 0) {
        window.fetch = activeRegistry.originalFetch;
        delete window.__storybookPoolAttemptsRegistry__;
      }
    };
  }, []);

  return <>{children}</>;
}

const meta = {
  title: "Records/InvocationRecordsTable",
  component: InvocationRecordsTable,
  tags: ["autodocs"],
  parameters: {
    layout: "fullscreen",
  },
  decorators: [
    (Story, context) => (
      <I18nProvider>
        <StorybookPoolAttemptsMock
          records={
            (context.args.records as typeof STORYBOOK_INVOCATION_RECORDS | undefined) ??
            STORYBOOK_INVOCATION_RECORDS
          }
          responseBodiesByIdOverrides={
            context.parameters.invocationResponseBodiesById as
              | InvocationResponseBodiesById
              | undefined
          }
          workflowDetailsByIdOverrides={
            context.parameters.invocationWorkflowDetailsById as
              | InvocationWorkflowDetailsById
              | undefined
          }
        >
          <StorySurface>
            <Story />
          </StorySurface>
        </StorybookPoolAttemptsMock>
      </I18nProvider>
    ),
  ],
} satisfies Meta<typeof InvocationRecordsTable>;

export default meta;

type Story = StoryObj<typeof meta>;

export const TokenFocus: Story = {
  args: {
    focus: "token",
    records: STORYBOOK_INVOCATION_RECORDS,
    isLoading: false,
    error: null,
  },
};

export const TransportBadgeMixed: Story = {
  args: {
    focus: "token",
    records: STORYBOOK_INVOCATION_RECORDS,
    isLoading: false,
    error: null,
  },
  parameters: {
    docs: {
      description: {
        story:
          "Mixed transport records for verifying that the WebSocket badge appears immediately after the model name in the records table while non-WS rows stay unchanged.",
      },
    },
  },
  play: async ({ canvasElement }) => {
    const badges = canvasElement.querySelectorAll('[data-testid="invocation-transport-badge"]');
    expect(badges.length).toBeGreaterThanOrEqual(1);
    expect(
      Array.from(badges).every(
        (badge) => badge.querySelector('[aria-hidden="true"]')?.textContent === "WS",
      ),
    ).toBe(true);
  },
};

export const ModelRoutingMismatch: Story = {
  args: {
    focus: "token",
    records: STORYBOOK_INVOCATION_RECORDS.map((record, index) =>
      index === 0
        ? {
            ...record,
            model: "gpt-5.5",
            requestModel: "gpt-5.4",
            responseModel: "gpt-5.5",
          }
        : record,
    ),
    isLoading: false,
    error: null,
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(canvas.getAllByRole("button", { name: /展开详情|show details/i })[0]!);
    await waitFor(() => {
      expect(
        canvasElement.querySelector('[data-testid="invocation-model-route-summary"]'),
      ).not.toBeNull();
      expect(document.body.textContent ?? "").toContain("gpt-5.4");
      expect(document.body.textContent ?? "").toContain("gpt-5.5");
    });
  },
};

export const TokenCostAuditWarningMobile390: Story = {
  args: {
    focus: "token",
    records: TOKEN_COST_AUDIT_WARNING_RECORDS,
    isLoading: false,
    error: null,
  },
  parameters: {
    viewport: { defaultViewport: "mobile390" },
    docs: {
      description: {
        story:
          "390px regression surface for the shared Token and cost warning state. It keeps the reroute model summary, mismatch warning, and local-vs-recorded cost copy visible without horizontal overflow.",
      },
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(canvas.getByRole("button", { name: /展开详情|show details/i }));
    await waitFor(() => {
      expect(canvasElement.querySelector('[data-testid="records-mobile-cost-warning"]')).toBeNull();
      expect(canvasElement.querySelector('[data-testid="records-table-cost-warning"]')).toBeNull();
      expect(
        canvasElement.querySelector('[data-testid="records-detail-strip-cost-warning"]'),
      ).not.toBeNull();
      expect(document.body.textContent ?? "").toContain("Token 与成本");
      expect(document.body.textContent ?? "").toContain("本地");
    });
  },
};

export const TokenCostAuditWarningDesktop: Story = {
  args: {
    focus: "token",
    records: TOKEN_COST_AUDIT_WARNING_RECORDS,
    isLoading: false,
    error: null,
  },
  parameters: {
    docs: {
      description: {
        story:
          "Desktop regression surface for the shared Token and cost warning state after removing duplicate warning markers and reverting mismatch amounts to normal text color.",
      },
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(canvas.getByRole("button", { name: /展开详情|show details/i }));
    await waitFor(() => {
      expect(canvasElement.querySelector('[data-testid="records-mobile-cost-warning"]')).toBeNull();
      expect(canvasElement.querySelector('[data-testid="records-table-cost-warning"]')).toBeNull();
      expect(
        canvasElement.querySelector('[data-testid="records-detail-strip-cost-warning"]'),
      ).not.toBeNull();
      expect(document.body.textContent ?? "").toContain("Token 与成本");
      expect(document.body.textContent ?? "").toContain("本地");
    });
  },
};

export const ResponseBodyWidthGuard: Story = {
  args: {
    focus: "exception",
    records: RESPONSE_BODY_WIDTH_GUARD_RECORDS,
    isLoading: false,
    error: null,
  },
  parameters: {
    invocationResponseBodiesById: RESPONSE_BODY_WIDTH_GUARD_RESPONSE_BODIES_BY_ID,
    invocationWorkflowDetailsById: buildResponseBodyWidthGuardWorkflowDetails(),
    viewport: { defaultViewport: "desktop1280" },
    docs: {
      description: {
        story:
          "Desktop regression surface for long SSE response bodies. Opening the response-body panel must keep page width stable while the payload inspector owns any horizontal scrolling.",
      },
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const initialRootOverflow =
      document.documentElement.scrollWidth - document.documentElement.clientWidth;
    expect(initialRootOverflow).toBeLessThanOrEqual(1);

    await userEvent.click(canvas.getByRole("button", { name: /展开详情|show details/i }));
    await waitFor(() => {
      expect(
        canvasElement.querySelector('[data-testid="records-expanded-detail-panel"]'),
      ).not.toBeNull();
      expect(document.body.textContent ?? "").toContain("响应体");
    });

    const responseBodyButton = Array.from(canvasElement.querySelectorAll("button")).find(
      (candidate): candidate is HTMLButtonElement =>
        candidate instanceof HTMLButtonElement &&
        candidate.getClientRects().length > 0 &&
        (candidate.textContent ?? "").includes("响应体"),
    );
    if (!responseBodyButton) {
      throw new Error("missing response body metric button");
    }

    await userEvent.click(responseBodyButton);

    await waitFor(() => {
      const payloadScroll = Array.from(
        document.querySelectorAll(".structured-payload-scroll"),
      ).find(
        (candidate): candidate is HTMLElement =>
          candidate instanceof HTMLElement &&
          candidate.offsetWidth > 0 &&
          candidate.offsetHeight > 0,
      );
      expect(payloadScroll).not.toBeNull();
      expect(payloadScroll?.scrollWidth ?? 0).toBeGreaterThan(payloadScroll?.clientWidth ?? 0);
      expect(
        document.documentElement.scrollWidth - document.documentElement.clientWidth,
      ).toBeLessThanOrEqual(1);
    });
  },
};

export const PoolRoutingAccountStates: Story = {
  args: {
    focus: "network",
    records: POOL_ROUTING_ACCOUNT_STATE_RECORDS,
    isLoading: false,
    error: null,
  },
  parameters: {
    docs: {
      description: {
        story:
          "Records table state gallery for a pool request currently routed to a concrete upstream account, a pending request with no account yet, and a terminal account record. The expanded detail panel shares the same routing-in-progress text class.",
      },
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const runningAccount = await canvas.findByRole("button", { name: "Pool Zeta 58" });
    await expect(runningAccount.className).toContain("invocation-account-routing-in-progress");
    await expect(canvas.getByText(/号池路由中|pool routing/i)).toBeInTheDocument();
    const terminalAccount = await canvas.findByRole("button", { name: "Pool Alpha 17" });
    await expect(terminalAccount.className).not.toContain("invocation-account-routing-in-progress");

    await userEvent.click(canvas.getAllByRole("button", { name: /展开详情|show details/i })[0]!);
    await waitFor(() => {
      const expandedAccount = canvasElement.querySelector('[title="Pool Zeta 58"]');
      expect(expandedAccount?.className ?? "").toContain("invocation-account-routing-in-progress");
    });
  },
};

export const LegacyModelOnly: Story = {
  args: {
    focus: "token",
    records: [
      {
        ...STORYBOOK_INVOCATION_RECORDS[0],
        invokeId: "inv_story_legacy_model_only",
        model: "gpt-5-legacy",
        requestModel: undefined,
        responseModel: undefined,
      },
    ],
    isLoading: false,
    error: null,
  },
};

export const NetworkFocus: Story = {
  args: {
    focus: "network",
    records: STORYBOOK_INVOCATION_RECORDS,
    isLoading: false,
    error: null,
  },
};

export const FirstResponseByteSemantics: Story = {
  args: {
    focus: "network",
    records: STORYBOOK_FIRST_RESPONSE_BYTE_SEMANTICS_RECORDS,
    isLoading: false,
    error: null,
  },
  parameters: {
    docs: {
      description: {
        story:
          "Focused network view for the new first-response-byte-total semantics. The first row deliberately keeps `上游首字节 = 0.0 ms` while the cumulative `首字总耗时` stays near `9.36 s`, matching the user-facing clarification in the monitoring table.",
      },
    },
  },
};

export const WarningSuccessStatus: Story = {
  args: {
    focus: "network",
    records: WARNING_SUCCESS_RECORDS,
    isLoading: false,
    error: null,
  },
  parameters: {
    docs: {
      description: {
        story:
          "Dedicated warning-success terminal row for future pure_downstream_closed records, preserving downstream diagnostics while rendering the owner-facing status as success-like warning.",
      },
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByText(/警告成功|Warning success/i)).toBeInTheDocument();
    await expect(canvas.getByText("Pool Alpha 42")).toBeInTheDocument();
    await expect(canvas.getByText(/167,710/)).toBeInTheDocument();
  },
};

export const EndpointChipStates: Story = {
  args: {
    focus: "network",
    records: ENDPOINT_CHIP_RECORDS,
    isLoading: false,
    error: null,
  },
  parameters: {
    docs: {
      description: {
        story:
          "Records-table view of the shared endpoint chip contract. Image-family endpoints render as `image/gen`, `image/edit`, or `image`, while an unknown non-image endpoint keeps the raw path fallback.",
      },
    },
  },
};

export const ExceptionFocus: Story = {
  args: {
    focus: "exception",
    records: STORYBOOK_INVOCATION_RECORDS,
    isLoading: false,
    error: null,
  },
};

export const StructuredOnlyFocus: Story = {
  args: {
    focus: "exception",
    records: STORYBOOK_INVOCATION_RECORDS.filter(
      (record) => record.detailLevel === "structured_only",
    ),
    isLoading: false,
    error: null,
  },
};

export const AbnormalResponseDrawer: Story = {
  args: {
    focus: "exception",
    records: STORYBOOK_INVOCATION_RECORDS.filter((record) => record.status === "failed"),
    isLoading: false,
    error: null,
  },
  parameters: {
    docs: {
      description: {
        story:
          "Failed-record state with inline abnormal response preview and the full-details drawer backed by stable Storybook mocks.",
      },
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);

    await userEvent.click(canvas.getByRole("button", { name: /展开详情|show details/i }));
    await waitFor(() => {
      expect(document.body.textContent ?? "").toContain("table.responseBody.openFullDetails");
    });

    const fullDetailsButton = Array.from(document.body.querySelectorAll("button")).find(
      (candidate): candidate is HTMLButtonElement =>
        candidate instanceof HTMLButtonElement &&
        candidate.textContent === "table.responseBody.openFullDetails",
    );
    if (!fullDetailsButton) {
      throw new Error("missing full details button");
    }

    await userEvent.click(fullDetailsButton);

    await waitFor(() => {
      expect(document.body.textContent ?? "").toContain("records.table.fullDetails.title");
    });
  },
};

export const PoolRouteFocus: Story = {
  args: {
    focus: "network",
    records: STORYBOOK_INVOCATION_RECORDS.filter((record) => record.routeMode === "pool"),
    isLoading: false,
    error: null,
  },
};

export const BudgetExhaustedTerminalRecord: Story = {
  args: {
    focus: "exception",
    records: STORYBOOK_INVOCATION_RECORDS.filter((record) => record.invokeId === "inv_story_6110"),
    isLoading: false,
    error: null,
  },
  parameters: {
    docs: {
      description: {
        story:
          "Focused fixture for seven real pool upstream attempts followed by one synthetic budget-exhausted terminal record. The terminal row is rendered as a neutral terminal state, not as another retry card.",
      },
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);

    await userEvent.click(canvas.getByRole("button", { name: /展开详情|show details/i }));
    await waitFor(() => {
      expect(document.querySelector('[data-testid="pool-attempt-terminal-record"]')).not.toBeNull();
      const visibleAttemptsList = Array.from(
        document.querySelectorAll('[data-testid="pool-attempts-list"]'),
      ).find((candidate) => candidate.getBoundingClientRect().width > 0);
      expect(
        visibleAttemptsList?.querySelectorAll('[data-testid="pool-attempt-item"]'),
      ).toHaveLength(7);
    });

    const visibleTerminal = Array.from(
      document.querySelectorAll('[data-testid="pool-attempt-terminal-record"]'),
    ).find((candidate) => candidate.getBoundingClientRect().width > 0);
    const terminalText = visibleTerminal?.textContent ?? "";
    expect(terminalText).toContain("未发起新请求");
    expect(terminalText).toContain("上一失败账号");
    expect(terminalText).toContain("solacebambi9197 Team");
    expect(terminalText).not.toContain("同账号重试 / 账号序号");
    expect(terminalText).not.toContain("0/3");
    expect(terminalText).not.toContain("HTTP 失败");
    expect(terminalText).not.toContain("连接耗时");
  },
};

export const SplitProxyErrorSemantics: Story = {
  args: {
    focus: "exception",
    records: STORYBOOK_PROXY_ERROR_CONTRACT_RECORDS,
    isLoading: false,
    error: null,
  },
  parameters: {
    docs: {
      description: {
        story:
          "Compares three failure contracts in one controlled surface: a synthetic oauth-bridge 502 split into upstream transport facts + downstream wrapper text, a true upstream HTTP 502, and a downstream-closed client abort with downstream-only diagnostics.",
      },
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);

    await userEvent.click(canvas.getByRole("button", { name: /展开详情|show details/i }));
    await waitFor(() => {
      expect(
        document.querySelector('[data-testid="invocation-upstream-error-section"]'),
      ).not.toBeNull();
      expect(
        document.querySelector('[data-testid="invocation-downstream-error-section"]'),
      ).not.toBeNull();
      expect(
        document.querySelector('[data-testid="pool-attempt-downstream-error"]')?.textContent ?? "",
      ).toContain("pool upstream responded with 502");
    });
  },
};

const DETAIL_LAYOUT_GALLERY_RECORDS = [
  STORYBOOK_INVOCATION_RECORDS[0]!,
  STORYBOOK_INVOCATION_RECORDS.find((record) => record.invokeId === "inv_story_6106")!,
  STORYBOOK_INVOCATION_RECORDS.find((record) => record.invokeId === "inv_story_6110")!,
];

export const DetailLayoutStateGallery: Story = {
  args: {
    focus: "exception",
    records: DETAIL_LAYOUT_GALLERY_RECORDS,
    isLoading: false,
    error: null,
  },
  parameters: {
    docs: {
      description: {
        story:
          "Curated state gallery for the reorganized shared invocation detail panel. It covers a successful call, an in-flight call, and a failed pool-terminal call using the same `InvocationExpandedDetails` component shared by Live and Dashboard drawers.",
      },
    },
    viewport: { defaultViewport: "desktop1280" },
  },
  render: () => (
    <div className="space-y-5">
      {DETAIL_LAYOUT_GALLERY_RECORDS.map((record) => (
        <div
          key={record.invokeId}
          className="rounded-xl border border-base-300/70 bg-base-100/62 p-3"
        >
          <InvocationRecordsTable
            focus={record.status === "failed" ? "exception" : "network"}
            records={[record]}
            isLoading={false}
            error={null}
          />
        </div>
      ))}
    </div>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const toggles = await canvas.findAllByRole("button", { name: /展开详情|show details/i });

    await userEvent.click(toggles[0]!);

    await waitFor(() => {
      const text = document.body.textContent ?? "";
      expect(text).toContain("路由与模型");
      expect(text).toContain("失败信号");
      expect(text).toContain("细节保留");
    });
  },
};

export const DetailLayoutMobileLongFields: Story = {
  args: {
    focus: "network",
    records: [
      {
        ...STORYBOOK_INVOCATION_RECORDS[0]!,
        invokeId: "inv_story_long_detail_fields",
        promptCacheKey:
          "019f1832-8956-7d73-9e4e-7cba6f9cd8c2-extra-long-cache-key-for-mobile-wrap-proof",
        endpoint:
          "/v1/responses/with/a/very/long/path/that/should/wrap/without/forcing-horizontal-overflow",
        requesterIp: "2001:db8:85a3:0000:0000:8a2e:0370:7334",
      },
    ],
    isLoading: false,
    error: null,
  },
  parameters: {
    docs: {
      description: {
        story:
          "Mobile-width regression surface for long IDs, prompt cache keys, endpoint paths, and IPv6 values inside the reorganized detail sections.",
      },
    },
    viewport: { defaultViewport: "mobile390" },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(canvas.getByRole("button", { name: /展开详情|show details/i }));
    await waitFor(() => {
      expect(document.body.textContent ?? "").toContain("019f1832-8956-7d73");
      expect(document.body.textContent ?? "").toContain("路由与模型");
    });
  },
};

export const DetailLayoutDarkPoolTerminal: Story = {
  args: {
    focus: "exception",
    records: STORYBOOK_INVOCATION_RECORDS.filter((record) => record.invokeId === "inv_story_6110"),
    isLoading: false,
    error: null,
  },
  globals: {
    themeMode: "dark",
  },
  parameters: {
    docs: {
      description: {
        story:
          "Dark-theme focused state for the reorganized detail hierarchy and pool terminal record boundary.",
      },
    },
    viewport: { defaultViewport: "desktop1280" },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(canvas.getByRole("button", { name: /展开详情|show details/i }));
    await waitFor(() => {
      expect(document.querySelector('[data-testid="pool-attempt-terminal-record"]')).not.toBeNull();
      expect(document.body.textContent ?? "").toContain("号池终态");
    });
  },
};

export const Loading: Story = {
  args: {
    focus: "token",
    records: [],
    isLoading: true,
    error: null,
  },
};

export const Empty: Story = {
  args: {
    focus: "token",
    records: [],
    isLoading: false,
    error: null,
  },
};
