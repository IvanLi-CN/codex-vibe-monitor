import type { Meta, StoryObj } from "@storybook/react-vite";
import { type ReactNode, useEffect } from "react";
import { expect, userEvent, waitFor, within } from "storybook/test";
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
    <div className="bg-[#f6f1e7] px-6 py-6 text-base-content sm:px-8">
      <div className="mx-auto max-w-6xl rounded-[28px] border border-base-300/70 bg-base-200 px-6 py-6 shadow-sm">
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

const successfulWorkflowRecord: ApiInvocation = {
  id: 88,
  invokeId: "invoke-workflow-88",
  occurredAt: "2026-07-20T10:25:09Z",
  createdAt: "2026-07-20T10:25:09Z",
  status: "success",
  source: "proxy",
  routeMode: "pool",
  proxyDisplayName: "tokyo-edge-02",
  upstreamAccountId: 17,
  upstreamAccountName: "Pool 17",
  endpoint: "/v1/responses",
  model: "gpt-5.5",
  requestModel: "gpt-5.4",
  responseModel: "gpt-5.5",
  inputTokens: 132_219,
  cacheWriteTokens: 5_632,
  cacheInputTokens: 126_587,
  outputTokens: 59,
  reasoningTokens: undefined,
  totalTokens: 132_278,
  cost: 0.6375,
  responseContentEncoding: "identity",
  requestedServiceTier: "default",
  serviceTier: "default",
  billingServiceTier: "default",
  reasoningEffort: "high",
  imageIntent: "yes",
  promptCacheKey: "019f7ab7-350d-7602-a6da-c2616857f91e",
  stickyKey: "sk-route-88",
  tReqReadMs: 18,
  tReqParseMs: 7,
  tUpstreamConnectMs: 142,
  tUpstreamTtfbMs: 0,
  tUpstreamStreamMs: 3450,
  tRespParseMs: 9,
  tPersistMs: 6,
  tTotalMs: 3450,
};

const successfulWorkflowResponse: ApiInvocationWorkflowDetailResponse = {
  hero: {
    recordId: 88,
    invokeId: "invoke-workflow-88",
    promptCacheKey: "019f7ab7-350d-7602-a6da-c2616857f91e",
    routeMode: "pool",
    endpoint: "/v1/responses",
    requestModel: "gpt-5.4",
    responseModel: "gpt-5.5",
    finalStatus: "success",
    failureClass: null,
    downstreamStatusCode: 200,
    upstreamAccountId: 17,
    upstreamAccountName: "Pool 17",
    totalDurationMs: 3450,
    timelineAttemptCount: 1,
    poolAttemptCount: 1,
    totalTokens: 132_278,
    cost: 0.6375,
    occurredAt: "2026-07-20T10:25:09Z",
  },
  timeline: [
    {
      blockId: "route-88",
      kind: "routingDecision",
      occurredAt: "2026-07-20T10:25:09Z",
      title: "Route Pool 17",
      subtitle: "gpt-5.4 · binding-17 · route-17",
      detail: {
        request: {
          endpoint: "/v1/responses",
          routeMode: "pool",
          requestModel: "gpt-5.4",
          responseModel: "gpt-5.5",
          requestedServiceTier: "default",
          reasoningEffort: "high",
          imageIntent: "yes",
          promptCacheKey: "019f7ab7-350d-7602-a6da-c2616857f91e",
          requesterIp: "192.168.31.6",
          routing: {
            routeMode: "pool",
            upstreamRouteKey: "route-17",
            proxyBindingKey: "binding-17",
            proxyDisplayName: "tokyo-edge-02",
          },
          headers: {
            userAgent: "monitor-ui/1.0",
            xForwardedFor: "192.168.31.6",
          },
          bodyCapture: {
            availableAtInvocationLevel: true,
            size: 735368,
            truncated: false,
            detailLevel: "full",
          },
        },
      },
    },
    {
      blockId: "attempt-88",
      kind: "attempt",
      occurredAt: "2026-07-20T10:25:09Z",
      title: "Attempt #1",
      subtitle: "Pool 17",
      status: "success",
      attempt: {
        synthetic: false,
        attemptId: "pKQAKgc8",
        occurredAt: "2026-07-20T10:25:09Z",
        endpoint: "/v1/responses",
        stickyKey: "sk-route-88",
        upstreamAccountId: 17,
        upstreamAccountName: "Pool 17",
        requestModel: "gpt-5.4",
        responseModel: "gpt-5.5",
        upstreamRouteKey: "route-17",
        proxyBindingKeySnapshot: "binding-17",
        attemptIndex: 1,
        distinctAccountIndex: 1,
        sameAccountRetryIndex: 0,
        requesterIp: "192.168.31.6",
        startedAt: "2026-07-20T10:25:09Z",
        finishedAt: "2026-07-20T10:25:12Z",
        status: "success",
        phase: "completed",
        httpStatus: 200,
        downstreamHttpStatus: 200,
        failureKind: null,
        errorMessage: null,
        downstreamErrorMessage: null,
        connectLatencyMs: 142,
        firstByteLatencyMs: 0,
        streamLatencyMs: 3450,
        upstreamRequestId: "req_88",
        requestSummary: {
          endpoint: "/v1/responses",
          transport: "http",
          requestModel: "gpt-5.4",
          responseModel: "gpt-5.5",
          requestedServiceTier: "default",
          reasoningEffort: "high",
          imageIntent: "yes",
          routing: {
            routeMode: "pool",
            proxyDisplayName: "tokyo-edge-02",
            upstreamRouteKey: "route-17",
            proxyBindingKey: "binding-17",
          },
          headers: {
            userAgent: "monitor-ui/1.0",
            xForwardedFor: "192.168.31.6",
          },
          bodyCapture: {
            availableAtInvocationLevel: true,
            size: 735368,
            truncated: false,
            detailLevel: "full",
          },
        },
        responseSummary: {
          status: "success",
          phase: "completed",
          serviceTier: "default",
          billingServiceTier: "default",
          responseContentEncoding: "identity",
          headers: {
            contentEncoding: "identity",
            upstreamRequestId: "req_88",
          },
          delivery: {
            forwardedChunkCount: 19,
            forwardedBytes: 113858,
            usageObserved: true,
          },
          responseBodyCapture: {
            availableAtInvocationLevel: true,
            size: 113858,
            truncated: false,
            detailLevel: "full",
          },
          usage: {
            inputTokens: 132219,
            cacheWriteTokens: 5632,
            cacheInputTokens: 126587,
            outputTokens: 59,
            reasoningTokens: null,
            totalTokens: 132278,
            cost: 0.6375,
            tokens: {
              input: 132219,
              cacheWrite: 5632,
              cacheRead: 126587,
              output: 59,
              reasoning: null,
              total: 132278,
            },
            costs: {
              recorded: {
                input: null,
                cacheWrite: null,
                cacheRead: null,
                output: null,
                reasoning: null,
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
            },
            audit: {
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
        },
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

export const SuccessfulTokenCostAudit: Story = {
  args: {
    record: successfulWorkflowRecord,
    size: "default",
  },
  decorators: [
    (Story) => (
      <>
        <WorkflowFetchMock recordId={88} response={successfulWorkflowResponse} />
        <Story />
      </>
    ),
  ],
  parameters: {
    viewport: { defaultViewport: "desktop1660" },
    docs: {
      description: {
        story:
          "Success-path workflow detail with the response Token and cost audit panel expanded from the attempt rail. It demonstrates rerouted request/response models, mismatch warning, and null reasoning Tokens staying empty instead of faking zero.",
      },
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await waitFor(() => {
      expect(canvas.getByText("输入写 5,632")).toBeTruthy();
      expect(canvas.getByText("输入读 126,587")).toBeTruthy();
    });
    await userEvent.hover(canvas.getByText("输入写 5,632"));
    await waitFor(() => {
      expect(document.body.textContent ?? "").toContain("输入（未命中缓存）");
    });
    await userEvent.unhover(canvas.getByText("输入写 5,632"));
    await userEvent.hover(canvas.getByText("输入读 126,587"));
    await waitFor(() => {
      expect(document.body.textContent ?? "").toContain("输入（命中缓存）");
    });
    const responseSummaryButton = Array.from(canvasElement.querySelectorAll("button")).find(
      (candidate): candidate is HTMLButtonElement =>
        candidate instanceof HTMLButtonElement &&
        candidate.textContent?.startsWith("响应") &&
        candidate.textContent.includes("gpt-5.5"),
    );
    if (!responseSummaryButton) {
      throw new Error("response summary button not found");
    }
    await userEvent.click(responseSummaryButton);
    await waitFor(() => {
      expect(document.body.textContent ?? "").toContain("Token 与成本");
      expect(document.body.textContent ?? "").toContain("未命中缓存输入 Token");
      expect(
        canvasElement.querySelectorAll('[data-testid="workflow-usage-cost-warning"]').length,
      ).toBe(1);
    });
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
