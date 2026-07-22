import type { Meta, StoryObj } from "@storybook/react-vite";
import { type ReactNode, useEffect } from "react";
import { MemoryRouter } from "react-router-dom";
import { expect, userEvent, waitFor, within } from "storybook/test";
import { I18nProvider } from "../../i18n";
import type { ApiPoolUpstreamRequestAttempt } from "../../lib/api";
import { FullPageStorySurface } from "../../storybook/storybookPageHelpers";
import { UpstreamAccountAttemptTimeline } from "./UpstreamAccountAttemptTimeline";

const workflowSuccessAttemptItem: ApiPoolUpstreamRequestAttempt = {
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
  downstreamHttpStatus: 200,
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
};

const workflowFailureAttemptItem: ApiPoolUpstreamRequestAttempt = {
  ...workflowSuccessAttemptItem,
  attemptId: "AFAIL001",
  attemptIndex: 1,
  sameAccountRetryIndex: 0,
  status: "http_failure",
  httpStatus: 500,
  downstreamHttpStatus: 502,
  failureKind: "upstream_response_failed",
  errorMessage: "upstream returned an oversized diagnostic payload",
  workflowEntry: {
    ...workflowSuccessAttemptItem.workflowEntry!,
    blockId: "attempt-AFAIL001",
    title: "Attempt #1",
    status: "http_failure",
    attempt: {
      ...workflowSuccessAttemptItem.workflowEntry!.attempt!,
      attemptId: "AFAIL001",
      attemptIndex: 1,
      sameAccountRetryIndex: 0,
      status: "http_failure",
      httpStatus: 500,
      downstreamHttpStatus: 502,
      failureKind: "upstream_response_failed",
      errorMessage: "upstream returned an oversized diagnostic payload",
      responseSummary: {
        ...workflowSuccessAttemptItem.workflowEntry!.attempt!.responseSummary!,
        status: "http_failure",
        httpStatus: 500,
        failureKind: "upstream_response_failed",
        errorMessage: "upstream returned an oversized diagnostic payload",
        responseBodyCapture: {
          availableAtInvocationLevel: false,
          size: 79_224,
          detailLevel: "attempt_metrics",
          unavailableReason: "non_final_attempt_response_body_not_captured",
        },
        usage: null,
      },
    },
  },
};

const imageAttemptItem: ApiPoolUpstreamRequestAttempt = {
  ...workflowSuccessAttemptItem,
  attemptId: "AIMAGE001",
  invokeId: "IMG7Y2QK",
  occurredAt: "2026-07-11T12:03:00.000Z",
  endpoint: "/v1/images/edits",
  stickyKey: "sticky-image",
  requestModel: "gpt-image-1",
  responseModel: "gpt-image-1",
  imageIntent: "direct_image",
  createdAt: "2026-07-11T12:03:00.000Z",
  invocationRecord: undefined,
  workflowEntry: undefined,
};

const remoteV2AttemptItem: ApiPoolUpstreamRequestAttempt = {
  ...workflowSuccessAttemptItem,
  attemptId: "AREMOTEV2",
  invokeId: "REMOTE2QK",
  occurredAt: "2026-07-11T12:02:00.000Z",
  endpoint: "/v1/responses",
  stickyKey: "sticky-remote",
  requestModel: "gpt-5.5",
  responseModel: "gpt-5.5-2026-07-01",
  compactionRequestKind: "remote_v2",
  compactionResponseKind: "remote_v2",
  imageIntent: "no",
  createdAt: "2026-07-11T12:02:00.000Z",
  invocationRecord: undefined,
  workflowEntry: undefined,
};

const compactAttemptItem: ApiPoolUpstreamRequestAttempt = {
  ...workflowSuccessAttemptItem,
  attemptId: "ACOMPACT1",
  invokeId: "COMPACT1QK",
  occurredAt: "2026-07-11T12:01:00.000Z",
  endpoint: "/v1/responses/compact",
  stickyKey: null,
  requestModel: "gpt-5-compact",
  responseModel: "gpt-5-compact",
  compactionRequestKind: "compact",
  compactionResponseKind: "compact",
  imageIntent: "no",
  createdAt: "2026-07-11T12:01:00.000Z",
  invocationRecord: undefined,
  workflowEntry: undefined,
};

const attemptItems = [
  imageAttemptItem,
  remoteV2AttemptItem,
  compactAttemptItem,
  workflowSuccessAttemptItem,
  workflowFailureAttemptItem,
];

function isAttemptTypeMatch(item: ApiPoolUpstreamRequestAttempt, type: string | null) {
  if (!type) return true;
  const isImage =
    item.endpoint.startsWith("/v1/images/") ||
    item.imageIntent === "yes" ||
    item.imageIntent === "direct_image";
  const isRemoteV2 =
    item.endpoint === "/v1/responses" &&
    (item.compactionRequestKind === "remote_v2" || item.compactionResponseKind === "remote_v2");
  const isCompact =
    item.endpoint === "/v1/responses/compact" ||
    item.compactionRequestKind === "compact" ||
    item.compactionResponseKind === "compact";
  if (type === "image") return isImage;
  if (type === "remote_v2") return isRemoteV2;
  if (type === "compact") return isCompact;
  if (type === "normal") return !isImage && !isRemoteV2 && !isCompact;
  return true;
}

function filterAttemptItems(searchParams: URLSearchParams) {
  const type = searchParams.get("type");
  const model = searchParams.get("model")?.trim().toLowerCase() ?? "";
  const stickyKey = searchParams.get("stickyKey")?.trim() ?? "";
  return attemptItems.filter((item) => {
    if (!isAttemptTypeMatch(item, type)) return false;
    if (
      model &&
      ![item.requestModel, item.responseModel, item.model].some(
        (candidate) => candidate?.trim().toLowerCase() === model,
      )
    ) {
      return false;
    }
    if (stickyKey === "__unbound__") {
      return item.stickyKey == null || item.stickyKey.trim() === "";
    }
    if (stickyKey && item.stickyKey !== stickyKey) return false;
    return true;
  });
}

function buildStickyKeyOptions(items: ApiPoolUpstreamRequestAttempt[]) {
  const latestByKey = new Map<string, string>();
  for (const item of items) {
    const value = item.stickyKey?.trim() || "__unbound__";
    const current = latestByKey.get(value);
    if (!current || item.createdAt > current) latestByKey.set(value, item.createdAt);
  }
  return Array.from(latestByKey.entries())
    .sort((left, right) => right[1].localeCompare(left[1]) || left[0].localeCompare(right[0]))
    .map(([value, latestCreatedAt]) => ({ value, latestCreatedAt }));
}

function withAccountId(item: ApiPoolUpstreamRequestAttempt, accountId: number) {
  return {
    ...item,
    upstreamAccountId: accountId,
    invocationRecord: item.invocationRecord
      ? { ...item.invocationRecord, upstreamAccountId: accountId }
      : item.invocationRecord,
    workflowEntry: item.workflowEntry
      ? {
          ...item.workflowEntry,
          attempt: item.workflowEntry.attempt
            ? { ...item.workflowEntry.attempt, upstreamAccountId: accountId }
            : item.workflowEntry.attempt,
        }
      : item.workflowEntry,
  };
}

function StorySurface({ children }: { children: ReactNode }) {
  return (
    <div className="bg-[#f6f1e7] px-6 py-6 text-base-content sm:px-8">
      <div className="mx-auto max-w-6xl rounded-[28px] border border-base-300/70 bg-base-200 px-6 py-6 shadow-sm">
        {children}
      </div>
    </div>
  );
}

function AttemptTimelinePageSurface({ children }: { children: ReactNode }) {
  return (
    <FullPageStorySurface>
      <div className="mx-auto max-w-7xl space-y-5">
        <header className="rounded-[28px] border border-base-300/70 bg-base-100/85 px-6 py-6 shadow-sm">
          <p className="text-xs font-semibold uppercase tracking-[0.22em] text-base-content/55">
            Account Pool Review
          </p>
          <h1 className="mt-3 text-3xl font-semibold text-base-content">
            Upstream attempt compression evidence
          </h1>
          <p className="mt-2 max-w-3xl text-sm leading-6 text-base-content/70">
            Page-level fallback surface for reviewing retry attempt compression ratio and
            approximate upstream transfer bytes.
          </p>
        </header>

        <section className="rounded-[32px] border border-base-300/70 bg-base-100/82 px-6 py-6 shadow-sm">
          {children}
        </section>
      </div>
    </FullPageStorySurface>
  );
}

function AttemptTimelineFetchMock({ accountId }: { accountId: number }) {
  useEffect(() => {
    const originalFetch = globalThis.fetch;
    globalThis.fetch = async (input: RequestInfo | URL, init?: RequestInit) => {
      const url =
        typeof input === "string" ? input : input instanceof URL ? input.toString() : input.url;
      if (url.includes("/api/pool/forward-proxy-binding-nodes")) {
        return new Response(
          JSON.stringify([
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
          ]),
          {
            status: 200,
            headers: {
              "Content-Type": "application/json",
            },
          },
        );
      }
      if (
        url.includes(`/api/pool/upstream-accounts/${accountId}/call-attempts/locate`) ||
        url.includes(`/api/pool/upstream-accounts/${accountId}/call-attempts?`)
      ) {
        const parsedUrl = new URL(url, "http://storybook.local");
        const locatedAttemptId = parsedUrl.searchParams.get("attemptId")?.trim();
        const filteredItems = locatedAttemptId
          ? attemptItems.filter(
              (item) => item.attemptId === locatedAttemptId || item.attemptId === "AFAIL001",
            )
          : filterAttemptItems(parsedUrl.searchParams);
        const items = filteredItems.map((item) => withAccountId(item, accountId));
        return new Response(
          JSON.stringify({
            items,
            stickyKeyOptions: buildStickyKeyOptions(filteredItems),
            total: items.length,
            page: 1,
            pageSize: 50,
          }),
          {
            status: 200,
            headers: {
              "Content-Type": "application/json",
            },
          },
        );
      }
      if (url.includes("/api/invocations/77/request-body")) {
        return new Response(
          JSON.stringify({
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
          }),
          {
            status: 200,
            headers: {
              "Content-Type": "application/json",
            },
          },
        );
      }
      if (url.includes("/api/invocations/77/response-body")) {
        return new Response(
          JSON.stringify({
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
  }, [accountId]);

  return null;
}

const meta = {
  title: "Account Pool/Components/Upstream Account Attempt Timeline",
  component: UpstreamAccountAttemptTimeline,
  decorators: [
    (Story, context) => (
      <I18nProvider>
        <MemoryRouter>
          {context.parameters.pageSurface ? (
            <Story />
          ) : (
            <StorySurface>
              <Story />
            </StorySurface>
          )}
        </MemoryRouter>
      </I18nProvider>
    ),
  ],
  parameters: {
    viewport: { defaultViewport: "desktop1280" },
  },
} satisfies Meta<typeof UpstreamAccountAttemptTimeline>;

export default meta;

type Story = StoryObj<typeof meta>;

async function verifyWorkflowParitySurface(canvasElement: HTMLElement) {
  const canvas = within(canvasElement);
  await waitFor(() => {
    expect(canvasElement.textContent ?? "").toContain("217,958 B");
    expect(canvasElement.textContent ?? "").toContain("79,224 B");
    expect(canvasElement.textContent ?? "").toContain("输入写 2,090");
    expect(canvasElement.textContent ?? "").toContain("upstream_response_failed");
  });
  const requestBodyButton = (
    await canvas.findAllByRole("button", { name: /请求体|request body/i })
  )[0];
  await userEvent.click(requestBodyButton);
  await waitFor(() => {
    expect(canvasElement.textContent ?? "").toContain("large request");
  });
  const responseBodyButton = (
    await canvas.findAllByRole("button", { name: /响应体|response body/i })
  )[0];
  await userEvent.click(responseBodyButton);
  await waitFor(() => {
    expect(canvasElement.textContent ?? "").toContain("large response");
  });
}

function withAttemptTimelineFetchMock(Story: () => ReactNode) {
  return (
    <>
      <AttemptTimelineFetchMock accountId={101} />
      <Story />
    </>
  );
}

async function selectStoryOption(canvasElement: HTMLElement, testId: string, optionName: RegExp) {
  const canvas = within(canvasElement);
  await userEvent.click(await canvas.findByTestId(testId));
  await userEvent.click(await within(document.body).findByRole("option", { name: optionName }));
}

async function selectStoryModel(canvasElement: HTMLElement, optionName: RegExp) {
  const input = canvasElement.querySelector<HTMLInputElement>("#upstream-attempt-model-filter");
  if (!input) throw new Error("missing model filter input");
  await userEvent.click(input);
  await userEvent.click(await within(canvasElement).findByRole("option", { name: optionName }));
}

async function typeStoryModel(canvasElement: HTMLElement, value: string) {
  const input = canvasElement.querySelector<HTMLInputElement>("#upstream-attempt-model-filter");
  if (!input) throw new Error("missing model filter input");
  await userEvent.clear(input);
  await userEvent.type(input, value);
}

export const DefaultRequestAttempts: Story = {
  tags: ["test"],
  args: {
    accountId: 101,
    focusedAttemptId: null,
  },
  decorators: [withAttemptTimelineFetchMock],
  play: async ({ canvasElement }) => {
    await waitFor(() => {
      expect(canvasElement.textContent ?? "").toMatch(/一般|Normal/);
      expect(canvasElement.textContent ?? "").toContain("image/edit");
      expect(canvasElement.textContent ?? "").toMatch(/远程压缩V2|Remote compaction V2/);
    });
  },
};

export const TypeFilteredImageAttempts: Story = {
  ...DefaultRequestAttempts,
  play: async ({ canvasElement }) => {
    await selectStoryOption(canvasElement, "upstream-attempt-type-filter", /image/i);
    await waitFor(() => {
      expect(canvasElement.textContent ?? "").toContain("AIMAGE001");
      expect(canvasElement.textContent ?? "").not.toContain("AREMOTEV2");
    });
  },
};

export const ModelFilteredAttempts: Story = {
  ...DefaultRequestAttempts,
  play: async ({ canvasElement }) => {
    await selectStoryModel(canvasElement, /gpt-image-1/i);
    await waitFor(() => {
      expect(canvasElement.textContent ?? "").toContain("AIMAGE001");
      expect(canvasElement.textContent ?? "").not.toContain("ACOMPACT1");
    });
  },
};

export const ConversationFilteredAttempts: Story = {
  ...DefaultRequestAttempts,
  play: async ({ canvasElement }) => {
    await selectStoryOption(canvasElement, "upstream-attempt-conversation-filter", /sticky-image/i);
    await waitFor(() => {
      expect(canvasElement.textContent ?? "").toContain("AIMAGE001");
      expect(canvasElement.textContent ?? "").not.toContain("AREMOTEV2");
    });
  },
};

export const EmptyFilteredAttempts: Story = {
  ...DefaultRequestAttempts,
  play: async ({ canvasElement }) => {
    await typeStoryModel(canvasElement, "missing-model");
    await waitFor(() => {
      expect(canvasElement.textContent ?? "").toMatch(/没有该账号的尝试请求|No request attempts/);
      expect(
        canvasElement.querySelector('[data-testid="upstream-account-attempt-filter-bar"]'),
      ).not.toBeNull();
    });
  },
};

export const FullWorkflowSuccessAttempt: Story = {
  args: {
    accountId: 101,
    focusedAttemptId: "ASUCC002",
    focusVersion: 1,
  },
  decorators: [withAttemptTimelineFetchMock],
  play: async ({ canvasElement }) => {
    await verifyWorkflowParitySurface(canvasElement);
  },
};

export const FullWorkflowSuccessAttemptPage: Story = {
  ...FullWorkflowSuccessAttempt,
  tags: ["test"],
  parameters: {
    layout: "fullscreen",
    viewport: { defaultViewport: "desktop1660" },
    pageSurface: true,
  },
  render: (args) => (
    <AttemptTimelinePageSurface>
      <UpstreamAccountAttemptTimeline
        accountId={args.accountId ?? 101}
        focusedAttemptId={args.focusedAttemptId ?? "ASUCC002"}
        focusVersion={args.focusVersion ?? 1}
      />
    </AttemptTimelinePageSurface>
  ),
  play: async ({ canvasElement }) => {
    await verifyWorkflowParitySurface(canvasElement);
  },
};

export const FullWorkflowSuccessAttemptMobile: Story = {
  ...FullWorkflowSuccessAttemptPage,
  tags: ["test"],
  parameters: {
    layout: "fullscreen",
    viewport: { defaultViewport: "mobile390" },
    pageSurface: true,
  },
};
