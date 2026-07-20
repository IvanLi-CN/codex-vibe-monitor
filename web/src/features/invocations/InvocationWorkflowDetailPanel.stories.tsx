import type { Meta, StoryObj } from "@storybook/react-vite";
import { type ReactNode, useEffect } from "react";
import { expect, within } from "storybook/test";
import { I18nProvider } from "../../i18n";
import type { ApiInvocation, ApiInvocationWorkflowDetailResponse } from "../../lib/api";
import { FullPageStorySurface } from "../../storybook/storybookPageHelpers";
import { InvocationWorkflowDetailPanel } from "./InvocationWorkflowDetailPanel";
import {
  failedWorkflowFinalResponseBodyText,
  failedWorkflowRequestBodySize,
  failedWorkflowRequestBodyText,
  failedWorkflowResponseBody,
  failedWorkflowResponseBodySize,
  failedWorkflowResponseBodyText,
} from "./InvocationWorkflowDetailPanel.fixtures";

function StorySurface({ children }: { children: ReactNode }) {
  return (
    <div className="min-h-screen bg-base-200 px-6 py-6 text-base-content sm:px-8">
      <div className="mx-auto max-w-6xl rounded-[28px] border border-base-300/70 bg-base-100/88 px-6 py-6 shadow-sm">
        {children}
      </div>
    </div>
  );
}

function WorkflowPageSurface({ children }: { children: ReactNode }) {
  return (
    <FullPageStorySurface>
      <div className="mx-auto max-w-7xl space-y-5">
        <header className="rounded-[28px] border border-base-300/70 bg-base-100/85 px-6 py-6 shadow-sm">
          <p className="text-xs font-semibold uppercase tracking-[0.22em] text-base-content/55">
            Invocation Review
          </p>
          <h1 className="mt-3 text-3xl font-semibold text-base-content">
            Upstream request compression evidence
          </h1>
          <p className="mt-2 max-w-3xl text-sm leading-6 text-base-content/70">
            Page-level fallback surface for reviewing compression ratio and approximate upstream
            upload or download bytes in the invocation workflow detail.
          </p>
        </header>

        <section className="rounded-[32px] border border-base-300/70 bg-base-100/82 px-6 py-6 shadow-sm">
          {children}
        </section>
      </div>
    </FullPageStorySurface>
  );
}

function WorkflowFetchMock({
  recordId,
  response,
}: {
  recordId: number;
  response: ApiInvocationWorkflowDetailResponse;
}) {
  useEffect(() => {
    const originalFetch = globalThis.fetch;
    globalThis.fetch = async (input: RequestInfo | URL, init?: RequestInit) => {
      const url =
        typeof input === "string" ? input : input instanceof URL ? input.toString() : input.url;
      if (url.includes(`/api/invocations/${recordId}/workflow-detail`)) {
        return new Response(JSON.stringify(response), {
          status: 200,
          headers: {
            "Content-Type": "application/json",
          },
        });
      }
      if (url.includes(`/api/invocations/${recordId}/request-body`)) {
        return new Response(
          JSON.stringify({
            available: true,
            bodyText: failedWorkflowRequestBodyText,
            headers: {
              userAgent: "monitor-ui/1.0",
              xForwardedFor: "203.0.113.10",
              forwarded: "for=203.0.113.10;proto=https",
            },
            routing: {
              routeMode: "pool",
              stickyKey: "sk-route-77",
              promptCacheKey: "019d5ea7-519d-7312-a2e8-ef07abb7c09f",
              proxyDisplayName: "tokyo-edge-01",
            },
            bodySize: failedWorkflowRequestBodySize,
            detailLevel: "full",
            captureSource: "raw_file",
          }),
          {
            status: 200,
            headers: {
              "Content-Type": "application/json",
            },
          },
        );
      }
      if (url.includes(`/api/invocations/${recordId}/response-body`)) {
        return new Response(
          JSON.stringify({
            available: true,
            bodyText: failedWorkflowResponseBodyText,
            headers: {
              contentEncoding: "gzip",
              upstreamRequestId: "req_77",
              cvmInvokeId: "invoke-workflow-77",
            },
            routing: {
              forwardedChunkCount: 12,
              downstreamClosePhase: "streaming",
            },
            bodySize: failedWorkflowResponseBodySize,
            detailLevel: "full",
            captureSource: "raw_file",
          }),
          {
            status: 200,
            headers: {
              "Content-Type": "application/json",
            },
          },
        );
      }
      return originalFetch(input, init);
    };
    return () => {
      globalThis.fetch = originalFetch;
    };
  }, [recordId, response]);

  return null;
}

const failedWorkflowRecord: ApiInvocation = {
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
};

const failedWorkflowResponse: ApiInvocationWorkflowDetailResponse = {
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
            upstreamRouteKey: "route-pool-alpha",
            proxyBindingKey: "fpb_tokyo_alpha",
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
          upstreamRequestId: "req_77",
          serviceTier: "priority",
          billingServiceTier: "priority",
          compactionResponseKind: "remote_v2",
          toolCalls: ["web_search_preview", "function:search_docs"],
          outputItems: failedWorkflowResponseBody.output.length,
          responseContentEncoding: "gzip",
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

const blockedWorkflowRecord: ApiInvocation = {
  ...failedWorkflowRecord,
  occurredAt: "2026-07-18T02:03:02Z",
  status: "http_503",
  upstreamAccountId: null,
  upstreamAccountName: undefined,
  totalTokens: undefined,
  cost: undefined,
  tTotalMs: 810,
  errorMessage: "pool assigned account blocked",
  failureKind: "pool_assigned_account_blocked",
  downstreamStatusCode: 503,
  downstreamErrorMessage: "pool assigned account blocked",
};

const blockedWorkflowResponse: ApiInvocationWorkflowDetailResponse = {
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
    upstreamAccountId: null,
    upstreamAccountName: null,
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

const meta = {
  title: "Invocations/InvocationWorkflowDetailPanel",
  component: InvocationWorkflowDetailPanel,
  tags: ["autodocs"],
  decorators: [
    (Story, context) => (
      <I18nProvider>
        {context.parameters.pageSurface ? (
          <Story />
        ) : (
          <StorySurface>
            <Story />
          </StorySurface>
        )}
      </I18nProvider>
    ),
  ],
  parameters: {
    viewport: { defaultViewport: "desktop1280" },
  },
} satisfies Meta<typeof InvocationWorkflowDetailPanel>;

export default meta;

type Story = StoryObj<typeof meta>;

export const FailedPoolWorkflow: Story = {
  args: {
    record: failedWorkflowRecord,
    size: "default",
  },
  decorators: [
    (Story) => (
      <>
        <WorkflowFetchMock recordId={77} response={failedWorkflowResponse} />
        <Story />
      </>
    ),
  ],
};

export const FailedPoolWorkflowPage: Story = {
  ...FailedPoolWorkflow,
  parameters: {
    layout: "fullscreen",
    viewport: { defaultViewport: "desktop1660" },
    pageSurface: true,
  },
  render: (args) => (
    <WorkflowPageSurface>
      <InvocationWorkflowDetailPanel
        record={args.record ?? failedWorkflowRecord}
        size={args.size ?? "default"}
      />
    </WorkflowPageSurface>
  ),
};

export const BlockedPoolWorkflow: Story = {
  args: {
    record: blockedWorkflowRecord,
    size: "default",
  },
  decorators: [
    (Story) => (
      <>
        <WorkflowFetchMock recordId={77} response={blockedWorkflowResponse} />
        <Story />
      </>
    ),
  ],
};

export const FailedPoolWorkflowDark: Story = {
  ...FailedPoolWorkflow,
  globals: {
    themeMode: "dark",
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByText(/Final Result|最终结果/)).toBeVisible();
    await expect(canvas.getByText(/Final Account|最终账号/)).toBeVisible();
    await expect(canvas.getAllByText(/pool-alpha@example\.com/i)[0]).toBeVisible();
    await expect(canvas.getByText(/Final adjudication/i)).toBeVisible();
  },
};

export const BlockedPoolWorkflowDark: Story = {
  ...BlockedPoolWorkflow,
  globals: {
    themeMode: "dark",
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByText(/Final Result|最终结果/)).toBeVisible();
    await expect(canvas.getAllByText(/HTTP 503/i)[0]).toBeVisible();
    await expect(canvas.getAllByText(/pool_assigned_account_blocked/i)[0]).toBeVisible();
    await expect(canvas.getByText(/Final downstream response/i)).toBeVisible();
  },
};

export const TransientPending: Story = {
  args: {
    record: {
      ...failedWorkflowRecord,
      id: 0,
      status: "pending",
      errorMessage: undefined,
      failureClass: undefined,
      failureKind: undefined,
      downstreamStatusCode: undefined,
      downstreamErrorMessage: undefined,
    },
    size: "compact",
  },
};
