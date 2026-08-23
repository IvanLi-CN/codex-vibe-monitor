import type { Meta, StoryObj } from "@storybook/react-vite";
import { type ReactNode, useEffect, useRef } from "react";
import { MemoryRouter } from "react-router-dom";
import { expect, userEvent, waitFor, within } from "storybook/test";
import { I18nProvider } from "../../i18n";
import type {
  ApiPoolUpstreamRequestAttempt,
  UpstreamAccountAttemptListResponse,
} from "../../lib/api";
import {
  buildTopicDescriptor,
  getTopicDescriptorKey,
  type SubscriptionTopicEnvelope,
} from "../../lib/sse";
import {
  FullPageStorySurface,
  StorybookPageEnvironment,
} from "../../storybook/storybookPageHelpers";
import { getStorybookPageSseController } from "../../storybook/storybookPageSse";
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
  firstTokenMs: 780,
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
    firstTokenMs: 780,
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
      firstTokenMs: 780,
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
  const visualEvidenceTarget = new URLSearchParams(window.location.search).get("evidence");
  const visualEvidenceMode = visualEvidenceTarget != null;
  const visualEvidenceStoryId =
    visualEvidenceTarget === "mobile" ? "realtime-lifecycle-mobile" : "realtime-lifecycle";
  const visualEvidenceAnchorId = `anchor--account-pool-components-upstream-account-attempt-timeline--${visualEvidenceStoryId}`;
  const surfaceBackgroundClass = visualEvidenceMode ? "bg-[#e8dfd0]" : "bg-[#f6f1e7]";
  const storySurfacePaddingClass =
    visualEvidenceTarget === "mobile" ? "px-0 py-6" : "px-6 py-6 sm:px-8";
  const evidenceFrameClass =
    visualEvidenceTarget === "mobile"
      ? "mx-0 mt-3 mb-10 bg-[#d8e3f0] px-[36px] pt-[36px] pb-[35px]"
      : "mx-3 mt-3 mb-10 bg-[#d8e3f0] p-[18px]";
  const timelineSurfaceClass = visualEvidenceMode
    ? "mx-auto max-w-6xl bg-base-200 px-6 py-6"
    : "mx-auto max-w-6xl rounded-[28px] border border-base-300/70 bg-base-200 px-6 py-6 shadow-sm";
  const timelineSurface = <div className={timelineSurfaceClass}>{children}</div>;

  return (
    <>
      {visualEvidenceMode ? (
        <style>{`
          body:has([data-testid="upstream-account-attempt-story-surface"]),
          body:has([data-testid="upstream-account-attempt-story-surface"]) #storybook-docs,
          body:has([data-testid="upstream-account-attempt-story-surface"]) .sbdocs-wrapper,
          body:has([data-testid="upstream-account-attempt-story-surface"]) .sbdocs-content {
            background: #e8dfd0 !important;
          }

          body:has([data-testid="upstream-account-attempt-story-surface"]) .sbdocs-preview {
            background: #e8dfd0 !important;
            border: 0 !important;
            border-radius: 0 !important;
            box-shadow: none !important;
          }

          body:has([data-testid="upstream-account-attempt-story-surface"]) .sbdocs-content > :not(#${visualEvidenceAnchorId}),
          body:has([data-testid="upstream-account-attempt-story-surface"]) #${visualEvidenceAnchorId} > h3,
          body:has([data-testid="upstream-account-attempt-story-surface"]) .docblock-code-toggle {
            display: none !important;
          }

          body:has([data-testid="upstream-account-attempt-story-surface"])::-webkit-scrollbar {
            display: none;
          }
        `}</style>
      ) : null}
      <div className={`${surfaceBackgroundClass} ${storySurfacePaddingClass} text-base-content`}>
        {visualEvidenceMode ? (
          <div className={evidenceFrameClass} data-testid="upstream-account-attempt-story-surface">
            {timelineSurface}
          </div>
        ) : (
          timelineSurface
        )}
      </div>
    </>
  );
}

function AttemptTimelinePageSurface({ children }: { children: ReactNode }) {
  return (
    <FullPageStorySurface>
      <main className="app-shell-boundary px-4 py-6">{children}</main>
    </FullPageStorySurface>
  );
}

function AttemptTimelineFetchMock({
  accountId,
  relocateAfterInitialLocate = false,
}: {
  accountId: number;
  relocateAfterInitialLocate?: boolean;
}) {
  const locateRequestCountRef = useRef(0);

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
      if (url.includes(`/api/pool/upstream-accounts/${accountId}/call-attempts/locate`)) {
        const parsedUrl = new URL(url, "http://storybook.local");
        const locatedAttemptId = parsedUrl.searchParams.get("attemptId")?.trim();
        const page = relocateAfterInitialLocate && locateRequestCountRef.current > 0 ? 2 : 1;
        locateRequestCountRef.current += 1;
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
            page,
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
      if (
        url.includes("/api/invocations/77/attempts/ASUCC002/response-body") ||
        url.includes("/api/invocations/77/response-body")
      ) {
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
  }, [accountId, relocateAfterInitialLocate]);

  return null;
}

function AttemptTimelineSseMock({ accountId }: { accountId: number }) {
  useEffect(() => {
    const controller = getStorybookPageSseController();
    if (!controller) return;
    const timer = window.setTimeout(() => {
      // The docs page mounts every story in one SSE scope. Keep the lifecycle
      // story deterministic instead of letting the gallery fixtures overwrite it.
      if (document.querySelector('[data-name="Realtime Lifecycle"]')) return;
      const variants = [
        {},
        { type: "normal" },
        { type: "remote_v2" },
        { type: "compact" },
        { type: "image" },
        { model: "gpt-image-1" },
        { model: "missing-model" },
        { stickyKey: "sticky-image" },
      ];
      variants.forEach((variant, index) => {
        const descriptor = buildTopicDescriptor("upstream-account-attempts.window", {
          accountId,
          page: 1,
          pageSize: 50,
          ...variant,
        });
        const search = new URLSearchParams(descriptor.params as Record<string, string>);
        const filteredItems = filterAttemptItems(search).map((item) =>
          withAccountId(item, accountId),
        );
        controller.emit({
          type: "snapshot",
          topic: descriptor,
          topicKey: getTopicDescriptorKey(descriptor),
          schemaEpoch: "upstream-account-attempts.window/v1",
          cursor: index + 1,
          payload: {
            items: filteredItems,
            stickyKeyOptions: buildStickyKeyOptions(filteredItems),
            total: filteredItems.length,
            page: 1,
            pageSize: 50,
          },
        });
      });
    }, 50);
    return () => window.clearTimeout(timer);
  }, [accountId]);

  return null;
}

const realtimePendingAttempt: ApiPoolUpstreamRequestAttempt = {
  ...workflowSuccessAttemptItem,
  attemptId: "ALIVE0001",
  invokeId: "LIVE0001",
  occurredAt: "2026-07-11T12:04:00.000Z",
  createdAt: "2026-07-11T12:04:00.000Z",
  status: "pending",
  phase: "waiting_first_byte",
  httpStatus: null,
  downstreamHttpStatus: null,
  finishedAt: null,
  upstreamRequestId: null,
  invocationRecord: undefined,
  workflowEntry: undefined,
};

const realtimeTerminalAttempt: ApiPoolUpstreamRequestAttempt = {
  ...realtimePendingAttempt,
  status: "success",
  phase: "completed",
  httpStatus: 200,
  downstreamHttpStatus: 200,
  finishedAt: "2026-07-11T12:04:03.200Z",
};

const realtimeNewAttempt: ApiPoolUpstreamRequestAttempt = {
  ...realtimeTerminalAttempt,
  attemptId: "ALIVE0002",
  invokeId: "LIVE0002",
  occurredAt: "2026-07-11T12:04:04.000Z",
  createdAt: "2026-07-11T12:04:04.000Z",
  finishedAt: "2026-07-11T12:04:05.000Z",
};

const REALTIME_LIFECYCLE_ACCOUNT_ID = 919;
const REALTIME_LIFECYCLE_MOBILE_ACCOUNT_ID = 920;
const FOCUSED_RELOCATION_ACCOUNT_ID = 921;

function buildAttemptTimelineSnapshot(
  accountId: number,
  items: ApiPoolUpstreamRequestAttempt[],
  page = 1,
): SubscriptionTopicEnvelope<UpstreamAccountAttemptListResponse> {
  const descriptor = buildTopicDescriptor("upstream-account-attempts.window", {
    accountId,
    page,
    pageSize: 50,
  });
  return {
    type: "live",
    topic: descriptor,
    topicKey: getTopicDescriptorKey(descriptor),
    schemaEpoch: "upstream-account-attempts.window/v1",
    cursor: 1,
    payload: {
      items,
      stickyKeyOptions: buildStickyKeyOptions(items),
      total: items.length,
      page,
      pageSize: 50,
    },
  };
}

function AttemptTimelineRealtimeLifecycleMock({ accountId }: { accountId: number }) {
  useEffect(() => {
    const controller = getStorybookPageSseController();
    if (!controller) return;
    let terminalTimer: number | null = null;
    const initialTimer = window.setTimeout(() => {
      controller.emit(
        buildAttemptTimelineSnapshot(accountId, [withAccountId(realtimePendingAttempt, accountId)]),
      );
      terminalTimer = window.setTimeout(() => {
        controller.emit(
          buildAttemptTimelineSnapshot(accountId, [
            withAccountId(realtimeNewAttempt, accountId),
            withAccountId(realtimeTerminalAttempt, accountId),
          ]),
        );
      }, 160);
    }, 50);
    return () => {
      window.clearTimeout(initialTimer);
      if (terminalTimer != null) window.clearTimeout(terminalTimer);
    };
  }, [accountId]);

  return null;
}

function AttemptTimelineFocusedRelocationMock({ accountId }: { accountId: number }) {
  useEffect(() => {
    const controller = getStorybookPageSseController();
    if (!controller) return;
    const initialTimer = window.setTimeout(() => {
      controller.emit(
        buildAttemptTimelineSnapshot(accountId, [withAccountId(realtimePendingAttempt, accountId)]),
      );
    }, 50);
    const shiftedPageTimer = window.setTimeout(() => {
      controller.emit(buildAttemptTimelineSnapshot(accountId, [], 1));
    }, 180);
    const relocatedPageTimer = window.setTimeout(() => {
      controller.emit(
        buildAttemptTimelineSnapshot(
          accountId,
          [withAccountId(realtimeTerminalAttempt, accountId)],
          2,
        ),
      );
    }, 320);
    return () => {
      window.clearTimeout(initialTimer);
      window.clearTimeout(shiftedPageTimer);
      window.clearTimeout(relocatedPageTimer);
    };
  }, [accountId]);

  return null;
}

const meta = {
  title: "Account Pool/Components/Upstream Account Attempt Timeline",
  component: UpstreamAccountAttemptTimeline,
  tags: ["autodocs"],
  decorators: [
    (Story, context) => (
      <StorybookPageEnvironment>
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
      </StorybookPageEnvironment>
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
    expect(canvasElement.textContent ?? "").toContain("TTFT 0.8 s");
    expect(canvasElement.textContent ?? "").not.toContain("TTFB 0.1 s");
    expect(canvas.getAllByText("TTFT 0.8 s")[0]).toHaveClass("text-success");
    expect(canvasElement.textContent ?? "").toContain("输入写 2,090");
    expect(canvasElement.textContent ?? "").toContain("upstream_response_failed");
  });
  const workflowCard = await canvas.findByTestId("account-attempt-record-ASUCC002");
  const requestBodyButton = within(workflowCard).getByRole("button", {
    name: /请求体|request body/i,
  });
  await userEvent.click(requestBodyButton);
  await waitFor(() => {
    expect(canvasElement.textContent ?? "").toContain("large request");
  });
  const responseBodyButton = within(workflowCard).getByRole("button", {
    name: /响应体|response body/i,
  });
  await userEvent.click(responseBodyButton);
  await waitFor(() => {
    expect(canvasElement.textContent ?? "").toContain("large response");
  });
  const closedResponseBodyButton = (
    await canvas.findAllByRole("button", { name: /响应体|response body/i })
  )[0];
  await userEvent.click(closedResponseBodyButton);
  await waitFor(() => {
    expect(canvasElement.textContent ?? "").not.toContain("large response");
  });
  closedResponseBodyButton.blur();
}

function withAttemptTimelineFetchMock(Story: () => ReactNode) {
  return (
    <>
      <AttemptTimelineFetchMock accountId={101} />
      <AttemptTimelineSseMock accountId={101} />
      <Story />
    </>
  );
}

function withAttemptTimelineRealtimeLifecycleMock(Story: () => ReactNode) {
  return (
    <>
      <AttemptTimelineRealtimeLifecycleMock accountId={REALTIME_LIFECYCLE_ACCOUNT_ID} />
      <Story />
    </>
  );
}

function withAttemptTimelineRealtimeLifecycleMobileMock(Story: () => ReactNode) {
  return (
    <>
      <AttemptTimelineRealtimeLifecycleMock accountId={REALTIME_LIFECYCLE_MOBILE_ACCOUNT_ID} />
      <Story />
    </>
  );
}

function withAttemptTimelineFocusedRelocationMock(Story: () => ReactNode) {
  return (
    <>
      <AttemptTimelineFetchMock
        accountId={FOCUSED_RELOCATION_ACCOUNT_ID}
        relocateAfterInitialLocate
      />
      <AttemptTimelineFocusedRelocationMock accountId={FOCUSED_RELOCATION_ACCOUNT_ID} />
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

export const RealtimeLifecycle: Story = {
  tags: ["test"],
  args: {
    accountId: REALTIME_LIFECYCLE_ACCOUNT_ID,
    focusedAttemptId: null,
  },
  decorators: [withAttemptTimelineRealtimeLifecycleMock],
  play: async ({ canvasElement }) => {
    const pendingCard = await within(canvasElement).findByTestId(
      "account-attempt-record-ALIVE0001",
    );
    expect(
      canvasElement.querySelectorAll('[data-testid="account-attempt-record-ALIVE0001"]'),
    ).toHaveLength(1);
    expect(pendingCard.textContent ?? "").toContain("waiting_first_byte");

    await waitFor(() => {
      expect(canvasElement.querySelector('[data-testid="account-attempt-record-ALIVE0001"]')).toBe(
        pendingCard,
      );
      expect(
        canvasElement.querySelectorAll('[data-testid="account-attempt-record-ALIVE0002"]'),
      ).toHaveLength(1);
      expect(
        canvasElement.querySelectorAll('[data-testid="account-attempt-record-ALIVE0001"]'),
      ).toHaveLength(1);
      expect(canvasElement.textContent ?? "").toContain("HTTP 200");
      expect(canvasElement.textContent ?? "").not.toContain("waiting_first_byte");
    });
  },
};

export const RealtimeLifecycleMobile: Story = {
  ...RealtimeLifecycle,
  tags: ["test"],
  args: {
    accountId: REALTIME_LIFECYCLE_MOBILE_ACCOUNT_ID,
    focusedAttemptId: null,
  },
  decorators: [withAttemptTimelineRealtimeLifecycleMobileMock],
  parameters: {
    viewport: { defaultViewport: "mobile390" },
  },
};

export const FocusedAttemptRelocatesAfterAuthoritativeShift: Story = {
  tags: ["test"],
  args: {
    accountId: FOCUSED_RELOCATION_ACCOUNT_ID,
    focusedAttemptId: "ALIVE0001",
    focusVersion: 1,
  },
  decorators: [withAttemptTimelineFocusedRelocationMock],
  play: async ({ canvasElement }) => {
    const initialCard = await within(canvasElement).findByTestId(
      "account-attempt-record-ALIVE0001",
    );
    expect(initialCard.textContent ?? "").toContain("waiting_first_byte");

    await waitFor(() => {
      const relocatedCard = canvasElement.querySelector<HTMLElement>(
        '[data-testid="account-attempt-record-ALIVE0001"]',
      );
      expect(relocatedCard).not.toBe(initialCard);
      expect(relocatedCard?.dataset.focusVisible).toBe("true");
      expect(relocatedCard?.textContent ?? "").toContain("HTTP 200");
      expect(relocatedCard?.textContent ?? "").not.toContain("waiting_first_byte");
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
