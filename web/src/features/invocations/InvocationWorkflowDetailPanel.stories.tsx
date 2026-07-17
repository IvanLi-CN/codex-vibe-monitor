import type { Meta, StoryObj } from "@storybook/react-vite";
import { type ReactNode, useEffect } from "react";
import { I18nProvider } from "../../i18n";
import type { ApiInvocation, ApiInvocationWorkflowDetailResponse } from "../../lib/api";
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
    <div className="bg-transparent px-4 py-4 text-base-content sm:px-6">
      <div className="mx-auto max-w-6xl">{children}</div>
    </div>
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
        routeMode: "pool",
        poolAttemptCount: 1,
        promptCacheKey: "019d5ea7-519d-7312-a2e8-ef07abb7c09f",
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
            mode: "streaming",
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

const meta = {
  title: "Invocations/InvocationWorkflowDetailPanel",
  component: InvocationWorkflowDetailPanel,
  decorators: [
    (Story) => (
      <I18nProvider>
        <StorySurface>
          <Story />
        </StorySurface>
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
