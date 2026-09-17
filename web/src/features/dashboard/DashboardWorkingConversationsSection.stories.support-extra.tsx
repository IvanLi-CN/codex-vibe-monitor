import { useEffect, useId, useMemo, useState } from "react";
import type {
  ApiInvocation,
  ApiInvocationRecordDetailResponse,
  ApiInvocationResponseBodyResponse,
  ApiPoolUpstreamRequestAttempt,
  PromptCacheConversation,
  PromptCacheConversationInvocationPreview,
  PromptCacheConversationsResponse,
  UpstreamAccountActivityResponse,
} from "../../lib/api";
import {
  type DashboardWorkingConversationInvocationSelection,
  formatDashboardWorkingConversationSequenceId,
  hashDashboardWorkingConversationKey,
  mapPromptCacheConversationsToDashboardCards,
} from "../../lib/dashboardWorkingConversations";
import { AccountDetailDrawerShell } from "../account-pool/AccountDetailDrawerShell";
import { formatStoryAttemptId } from "../records/invocationRecordsStoryFixtures";
import { DashboardWorkingConversationsSection } from "./DashboardWorkingConversationsSection";
import {
  buildRecordFromPreview,
  createConversation,
  createPreview,
  createResponse,
  createUpstreamAccountActivityStoryResponse,
  currentAndPreviousResponse,
  requireFixture,
} from "./DashboardWorkingConversationsSection.stories.support-base";

const dashboardHistoryTopRecords = [
  createPreview({
    id: 910,
    invokeId: "invoke-history-910",
    occurredAt: "2026-05-12T08:15:57Z",
    status: "completed",
    model: "gpt-5.5",
    upstreamAccountId: 311,
    upstreamAccountName: "pool-ci-311@example.com",
    endpoint: "/v1/responses",
    inputTokens: 164_400,
    cacheInputTokens: 156_032,
    outputTokens: 37,
    reasoningTokens: 0,
    totalTokens: 164_437,
    cost: 0.121,
    reasoningEffort: "max",
    responseContentEncoding: "identity",
    requestedServiceTier: "auto",
    serviceTier: "priority",
    tTotalMs: 29_470,
  }),
  createPreview({
    id: 909,
    invokeId: "invoke-history-909",
    occurredAt: "2026-05-12T08:15:37Z",
    status: "http_502",
    failureClass: "service_failure",
    failureKind: "upstream_timeout",
    errorMessage: "[downstream_reset] upstream closed before first byte",
    model: "gpt-5.5",
    upstreamAccountId: 311,
    upstreamAccountName: "pool-ci-311@example.com",
    endpoint: "/v1/responses",
    inputTokens: 163_784,
    cacheInputTokens: 155_520,
    outputTokens: 570,
    reasoningTokens: 137,
    totalTokens: 164_354,
    cost: 0.1362,
    reasoningEffort: "high",
    responseContentEncoding: "identity",
    tTotalMs: 59_000,
  }),
  createPreview({
    id: 908,
    invokeId: "invoke-history-908",
    occurredAt: "2026-05-12T08:15:00Z",
    status: "completed",
    model: "gpt-5.5",
    upstreamAccountId: 312,
    upstreamAccountName: "pool-ci-312@example.com",
    endpoint: "/v1/responses",
    inputTokens: 163_496,
    cacheInputTokens: 155_520,
    outputTokens: 37,
    reasoningTokens: 0,
    totalTokens: 163_533,
    cost: 0.1188,
    reasoningEffort: "high",
    responseContentEncoding: "identity",
    tTotalMs: 16_280,
  }),
  createPreview({
    id: 907,
    invokeId: "invoke-history-907",
    occurredAt: "2026-05-12T08:14:00Z",
    status: "http_502",
    failureClass: "service_failure",
    failureKind: "downstream_reset",
    errorMessage: "[downstream_reset] response stream reset",
    model: "gpt-5.5",
    upstreamAccountId: 311,
    upstreamAccountName: "pool-ci-311@example.com",
    endpoint: "/v1/responses",
    inputTokens: 163_101,
    cacheInputTokens: 155_008,
    outputTokens: 348,
    reasoningTokens: 80,
    totalTokens: 163_449,
    cost: 0.1284,
    reasoningEffort: "high",
    responseContentEncoding: "identity",
    tTotalMs: 50_960,
  }),
  createPreview({
    id: 906,
    invokeId: "invoke-history-906",
    occurredAt: "2026-05-12T08:13:26Z",
    status: "completed",
    model: "gpt-5.5",
    upstreamAccountId: 312,
    upstreamAccountName: "pool-ci-312@example.com",
    endpoint: "/v1/responses",
    inputTokens: 162_990,
    cacheInputTokens: 154_880,
    outputTokens: 42,
    reasoningTokens: 0,
    totalTokens: 163_032,
    cost: 0.1171,
    reasoningEffort: "high",
    responseContentEncoding: "identity",
    tTotalMs: 33_760,
  }),
  createPreview({
    id: 905,
    invokeId: "invoke-history-905",
    occurredAt: "2026-05-12T08:13:10Z",
    status: "http_502",
    failureClass: "service_failure",
    failureKind: "upstream_timeout",
    errorMessage: "[upstream_read_timeout] upstream read timed out",
    model: "gpt-5.5",
    upstreamAccountId: 313,
    upstreamAccountName: "pool-ci-313@example.com",
    endpoint: "/v1/responses",
    inputTokens: 163_496,
    cacheInputTokens: 155_520,
    outputTokens: 37,
    reasoningTokens: 0,
    totalTokens: 163_533,
    cost: 0.1188,
    reasoningEffort: "high",
    responseContentEncoding: "identity",
    tTotalMs: 16_280,
  }),
  createPreview({
    id: 904,
    invokeId: "invoke-history-904",
    occurredAt: "2026-05-12T08:12:18Z",
    status: "completed",
    model: "gpt-5.5",
    upstreamAccountId: 312,
    upstreamAccountName: "pool-ci-312@example.com",
    endpoint: "/v1/responses",
    inputTokens: 162_880,
    cacheInputTokens: 154_752,
    outputTokens: 41,
    reasoningTokens: 0,
    totalTokens: 162_921,
    cost: 0.1167,
    reasoningEffort: "high",
    responseContentEncoding: "identity",
    tTotalMs: 35_520,
  }),
  createPreview({
    id: 903,
    invokeId: "invoke-history-903",
    occurredAt: "2026-05-12T08:11:42Z",
    status: "completed",
    model: "gpt-5.5",
    upstreamAccountId: 314,
    upstreamAccountName: "pool-ci-314@example.com",
    endpoint: "/v1/responses",
    inputTokens: 162_720,
    cacheInputTokens: 154_624,
    outputTokens: 46,
    reasoningTokens: 0,
    totalTokens: 162_766,
    cost: 0.1164,
    reasoningEffort: "high",
    responseContentEncoding: "identity",
    tTotalMs: 35_520,
  }),
  createPreview({
    id: 902,
    invokeId: "invoke-history-902",
    occurredAt: "2026-05-12T08:00:34Z",
    status: "completed",
    model: "gpt-5.5",
    upstreamAccountId: 315,
    upstreamAccountName: "pool-ci-315@example.com",
    endpoint: "/v1/responses",
    inputTokens: 160_104,
    cacheInputTokens: 151_920,
    outputTokens: 37,
    reasoningTokens: 0,
    totalTokens: 160_141,
    cost: 0.121,
    reasoningEffort: "high",
    responseContentEncoding: "identity",
    tTotalMs: 29_470,
  }),
] satisfies PromptCacheConversationInvocationPreview[];

const dashboardHistoryFillerSlots = [
  {
    startAt: "2026-05-12T08:05:30Z",
    count: 100,
    spacingMs: 15_000,
    kind: "recent",
  },
  {
    startAt: "2026-05-12T07:31:20Z",
    count: 90,
    spacingMs: 15_000,
    kind: "recent",
  },
  {
    startAt: "2026-05-12T06:56:10Z",
    count: 70,
    spacingMs: 15_000,
    kind: "recent",
  },
  {
    startAt: "2026-05-12T06:21:00Z",
    count: 46,
    spacingMs: 15_000,
    kind: "recent",
  },
  {
    startAt: "2026-05-11T16:00:00Z",
    count: 1,
    spacingMs: 60_000,
    kind: "first",
  },
] as const;

function buildDashboardHistoryEvidenceFixtures() {
  const promptCacheKey = "pck-dashboard-history-realistic";
  const fillerRecords = buildDashboardHistoryFillerRecords();

  const historyInvocations = [...dashboardHistoryTopRecords, ...fillerRecords].map((preview) => ({
    ...preview,
    upstreamAccountId: 311,
    upstreamAccountName: "CIII",
    proxyDisplayName: null,
  }));
  const totalTokens = historyInvocations.reduce(
    (sum, preview) => sum + Math.max(0, preview.totalTokens),
    0,
  );
  const totalCost = Number(
    historyInvocations.reduce((sum, preview) => sum + (preview.cost ?? 0), 0).toFixed(4),
  );
  const dashboardPreviewInvocations = historyInvocations.slice(0, 2);
  return {
    dashboardResponse: createResponse([
      createConversation(promptCacheKey, dashboardPreviewInvocations, {
        requestCount: historyInvocations.length,
        totalTokens,
        totalCost,
        createdAt: "2026-05-11T16:00:12Z",
        lastActivityAt: "2026-05-12T08:15:57Z",
        upstreamAccounts: [
          {
            upstreamAccountId: 311,
            upstreamAccountName: "CIII",
            requestCount: 142,
            totalTokens: 1_154_982,
            totalCost: 9.4211,
            lastActivityAt: "2026-05-12T08:15:57Z",
          },
        ],
      }),
    ]),
    historyInvocationsByPromptCacheKey: new Map([[promptCacheKey, historyInvocations]]),
  };
}

function buildDashboardHistoryFillerRecords() {
  const records: PromptCacheConversationInvocationPreview[] = [];
  let id = 801;
  for (const [slotIndex, slot] of dashboardHistoryFillerSlots.entries()) {
    const slotStartMs = Date.parse(slot.startAt);
    for (let index = 0; index < slot.count; index += 1) {
      const recordIndex = records.length;
      const occurredAt = new Date(slotStartMs - index * slot.spacingMs).toISOString();
      records.push(
        createDashboardHistoryFillerRecord({
          id,
          occurredAt,
          cycle: recordIndex % 6,
          upstreamAccountId: 320 + (recordIndex % 4),
          baseTokens: 82_000 + (recordIndex % 11) * 3_700,
          cost: Number((0.062 + (recordIndex % 7) * 0.0037).toFixed(4)),
          durationBase: slot.kind === "first" || slotIndex > 0 ? 46_000 : 17_000,
          recordIndex,
          first: slot.kind === "first",
        }),
      );
      id -= 1;
    }
  }
  return records;
}

function createDashboardHistoryFillerRecord({
  id,
  occurredAt,
  cycle,
  upstreamAccountId,
  baseTokens,
  cost,
  durationBase,
  recordIndex,
  first,
}: {
  id: number;
  occurredAt: string;
  cycle: number;
  upstreamAccountId: number;
  baseTokens: number;
  cost: number;
  durationBase: number;
  recordIndex: number;
  first: boolean;
}) {
  const totalTokens = baseTokens + (cycle % 2 === 0 ? 37 : 348);
  const common = {
    id,
    invokeId: `invoke-history-${id}`,
    occurredAt,
    model: "gpt-5.5",
    upstreamAccountId,
    upstreamAccountName: `pool-ci-${upstreamAccountId}@example.com`,
    endpoint: "/v1/responses",
    inputTokens: baseTokens,
    reasoningEffort: "high",
    responseContentEncoding: "identity",
  } as const;
  if (first || cycle === 0) {
    return createPreview({
      ...common,
      status: "completed",
      cacheInputTokens: Math.max(0, baseTokens - 8_200),
      outputTokens: 37,
      reasoningTokens: 0,
      totalTokens,
      cost,
      tTotalMs: first ? durationBase : durationBase + (recordIndex % 5) * 800,
    });
  }
  if (cycle === 1 || cycle === 4) {
    return createPreview({
      ...common,
      status: "http_502",
      failureClass: "service_failure",
      failureKind: "downstream_reset",
      errorMessage: "[downstream_reset] upstream stream reset mid-flight",
      cacheInputTokens: Math.max(0, baseTokens - 8_000),
      outputTokens: 92 + (recordIndex % 6) * 11,
      reasoningTokens: 48 + (recordIndex % 5) * 9,
      totalTokens,
      cost,
      tTotalMs: durationBase + 2_400 + (recordIndex % 5) * 900,
    });
  }
  if (cycle === 2) {
    return createPreview({
      ...common,
      status: "interrupted",
      failureClass: "client_abort",
      failureKind: "proxy_interrupted",
      errorMessage: "proxy request was interrupted before completion",
      cacheInputTokens: Math.max(0, baseTokens - 8_100),
      outputTokens: 0,
      reasoningTokens: 0,
      totalTokens: baseTokens,
      cost: 0,
      tTotalMs: durationBase - 2_000 + (recordIndex % 4) * 600,
    });
  }
  return createPreview({
    ...common,
    status: "completed",
    cacheInputTokens: Math.max(0, baseTokens - 7_900),
    outputTokens: 37,
    reasoningTokens: 0,
    totalTokens: baseTokens + 37,
    cost,
    tTotalMs: durationBase + (recordIndex % 6) * 700,
  });
}

const interruptedRecoveryResponse = createResponse([
  createConversation("pck-interrupted-recovery", [
    createPreview({
      id: 49,
      invokeId: "invoke-49",
      occurredAt: "2026-04-04T10:03:52Z",
      status: "interrupted",
      failureClass: "service_failure",
      failureKind: "proxy_interrupted",
      errorMessage: "proxy request was interrupted before completion and was recovered on startup",
      upstreamAccountId: 77,
      upstreamAccountName: "pool-account-77@example.com",
      endpoint: "/v1/responses",
      requestedServiceTier: "priority",
      serviceTier: "priority",
      responseContentEncoding: "gzip",
      tUpstreamStreamMs: null,
      tPersistMs: null,
      tTotalMs: null,
    }),
    createPreview({
      id: 48,
      invokeId: "invoke-48",
      occurredAt: "2026-04-04T10:01:20Z",
      status: "completed",
      upstreamAccountId: 77,
      upstreamAccountName: "pool-account-77@example.com",
      model: "gpt-5.4-mini",
    }),
  ]),
]);

const assignedAccountFailureSemanticsResponse = createResponse([
  createConversation("pck-assigned-account-blocked", [
    createPreview({
      id: 53,
      invokeId: "invoke-assigned-account-blocked-current",
      occurredAt: "2026-04-04T10:04:36Z",
      status: "failed",
      failureClass: "service_failure",
      failureKind: "pool_assigned_account_blocked",
      errorMessage:
        '[pool_assigned_account_blocked] upstream account group "sticky-preflight-missing" has no bound forward proxy nodes',
      upstreamAccountId: 52,
      upstreamAccountName: "sticky-account-52@example.com",
      proxyDisplayName: "tokyo-edge-blocked",
      endpoint: "/v1/responses",
      requestedServiceTier: "priority",
      serviceTier: "priority",
      responseContentEncoding: "identity",
      tUpstreamTtfbMs: null,
      tUpstreamStreamMs: null,
      tTotalMs: 42,
    }),
    createPreview({
      id: 52,
      invokeId: "invoke-assigned-account-blocked-previous",
      occurredAt: "2026-04-04T10:02:12Z",
      status: "completed",
      upstreamAccountId: 52,
      upstreamAccountName: "sticky-account-52@example.com",
      model: "gpt-5.4-mini",
      requestedServiceTier: "priority",
      serviceTier: "priority",
    }),
  ]),
  createConversation("pck-true-no-account", [
    createPreview({
      id: 63,
      invokeId: "invoke-true-no-account-current",
      occurredAt: "2026-04-04T10:03:44Z",
      status: "failed",
      failureClass: "service_failure",
      failureKind: "pool_no_available_account",
      errorMessage: "[pool_no_available_account] no assignable upstream account remains",
      upstreamAccountId: null,
      upstreamAccountName: null,
      proxyDisplayName: null,
      endpoint: "/v1/responses",
      requestedServiceTier: "priority",
      serviceTier: "priority",
      responseContentEncoding: "identity",
      tUpstreamTtfbMs: null,
      tUpstreamStreamMs: null,
      tTotalMs: 38,
    }),
    createPreview({
      id: 62,
      invokeId: "invoke-true-no-account-previous",
      occurredAt: "2026-04-04T10:01:08Z",
      status: "completed",
      upstreamAccountId: null,
      upstreamAccountName: null,
      proxyDisplayName: null,
      model: "gpt-5.4-mini",
      requestedServiceTier: "priority",
      serviceTier: "priority",
    }),
  ]),
]);

const createdAtDescendingOrderResponse = createResponse([
  createConversation(
    "pck-created-middle",
    [
      createPreview({
        id: 52,
        invokeId: "invoke-created-middle-running",
        occurredAt: "2026-04-04T10:04:58Z",
        status: "running",
        upstreamAccountName: "ordering-middle@example.com",
        tTotalMs: null,
      }),
      createPreview({
        id: 51,
        invokeId: "invoke-created-middle-previous",
        occurredAt: "2026-04-04T10:03:40Z",
        status: "completed",
        upstreamAccountName: "ordering-middle@example.com",
      }),
    ],
    {
      createdAt: "2026-04-04T10:02:00Z",
    },
  ),
  createConversation(
    "pck-created-oldest",
    [
      createPreview({
        id: 61,
        invokeId: "invoke-created-oldest",
        occurredAt: "2026-04-04T10:03:20Z",
        status: "completed",
        upstreamAccountName: "ordering-oldest@example.com",
      }),
    ],
    {
      createdAt: "2026-04-04T09:58:00Z",
    },
  ),
  createConversation(
    "pck-created-newest",
    [
      createPreview({
        id: 71,
        invokeId: "invoke-created-newest",
        occurredAt: "2026-04-04T10:01:00Z",
        status: "completed",
        upstreamAccountName: "ordering-newest@example.com",
      }),
    ],
    {
      createdAt: "2026-04-04T10:03:00Z",
    },
  ),
]);

const wideDesktopRunningCurrent = createPreview({
  id: 81,
  invokeId: "invoke-wide-running-current",
  occurredAt: "2026-04-04T10:04:50Z",
  status: "running",
  reasoningEffort: "medium",
  upstreamAccountName: "paisleeeinar5710 Team sandbox workflow monitor",
  endpoint: "/v1/responses/compact",
  tTotalMs: null,
});
wideDesktopRunningCurrent.tUpstreamStreamMs = null;

const wideDesktopResponse = createResponse([
  createConversation(
    "pck-wide-running",
    [
      wideDesktopRunningCurrent,
      createPreview({
        id: 80,
        invokeId: "invoke-wide-running-previous",
        occurredAt: "2026-04-04T10:02:44Z",
        status: "completed",
        upstreamAccountName: "paisleeeinar5710 Team sandbox workflow monitor",
        endpoint: "/v1/responses/compact",
        model: "gpt-5.4-mini",
      }),
    ],
    {
      requestCount: 245,
      totalTokens: 34089123,
      totalCost: 32.1987,
    },
  ),
  createConversation("pck-wide-failed", [
    createPreview({
      id: 91,
      invokeId: "invoke-wide-failed-current",
      occurredAt: "2026-04-04T10:04:42Z",
      status: "http_502",
      failureClass: "service_failure",
      failureKind: "upstream_timeout",
      errorMessage: "upstream gateway closed before first byte",
      upstreamAccountId: 77,
      upstreamAccountName: "wide-failed@example.com",
      endpoint: "/v1/chat/completions",
      requestedServiceTier: "auto",
      serviceTier: "auto",
      responseContentEncoding: "identity",
      tUpstreamTtfbMs: null,
      tUpstreamStreamMs: null,
      tTotalMs: 30018,
    }),
    createPreview({
      id: 90,
      invokeId: "invoke-wide-failed-previous",
      occurredAt: "2026-04-04T10:02:10Z",
      status: "completed",
      upstreamAccountId: 77,
      upstreamAccountName: "wide-failed@example.com",
      model: "gpt-5.4-mini",
    }),
  ]),
  createConversation("pck-wide-placeholder", [
    createPreview({
      id: 101,
      invokeId: "invoke-wide-placeholder-current",
      occurredAt: "2026-04-04T10:04:21Z",
      status: "completed",
      upstreamAccountName: "wide-placeholder@example.com",
    }),
  ]),
  createConversation("pck-wide-success-a", [
    createPreview({
      id: 111,
      invokeId: "invoke-wide-success-a-current",
      occurredAt: "2026-04-04T10:04:10Z",
      status: "completed",
      upstreamAccountName: "wide-success-a@example.com",
      totalTokens: 322,
      cost: 0.0218,
      inputTokens: 186,
      outputTokens: 136,
      cacheInputTokens: 54,
      reasoningTokens: 28,
      tTotalMs: 514,
    }),
    createPreview({
      id: 110,
      invokeId: "invoke-wide-success-a-previous",
      occurredAt: "2026-04-04T10:01:48Z",
      status: "completed",
      upstreamAccountName: "wide-success-a@example.com",
      model: "gpt-5.4-mini",
      totalTokens: 248,
      cost: 0.0164,
    }),
  ]),
  createConversation("pck-wide-pending", [
    createPreview({
      id: 121,
      invokeId: "invoke-wide-pending-current",
      occurredAt: "2026-04-04T10:04:30Z",
      status: "pending",
      upstreamAccountName: "wide-pending@example.com",
      tTotalMs: null,
    }),
    createPreview({
      id: 120,
      invokeId: "invoke-wide-pending-previous",
      occurredAt: "2026-04-04T10:00:58Z",
      status: "completed",
      upstreamAccountName: "wide-pending@example.com",
    }),
  ]),
  createConversation("pck-wide-success-b", [
    createPreview({
      id: 131,
      invokeId: "invoke-wide-success-b-current",
      occurredAt: "2026-04-04T10:03:20Z",
      status: "completed",
      upstreamAccountName: "wide-success-b@example.com",
      totalTokens: 418,
      cost: 0.0276,
      inputTokens: 238,
      outputTokens: 180,
      cacheInputTokens: 76,
      reasoningTokens: 34,
      tTotalMs: 692,
    }),
    createPreview({
      id: 130,
      invokeId: "invoke-wide-success-b-previous",
      occurredAt: "2026-04-04T10:00:20Z",
      status: "completed",
      upstreamAccountName: "wide-success-b@example.com",
      model: "gpt-5.4-mini",
    }),
  ]),
  createConversation("pck-wide-running-b", [
    createPreview({
      id: 141,
      invokeId: "invoke-wide-running-b-current",
      occurredAt: "2026-04-04T10:04:05Z",
      status: "running",
      upstreamAccountName: "wide-running-b@example.com",
      tTotalMs: null,
    }),
    createPreview({
      id: 140,
      invokeId: "invoke-wide-running-b-previous",
      occurredAt: "2026-04-04T09:59:12Z",
      status: "completed",
      upstreamAccountName: "wide-running-b@example.com",
      model: "gpt-5.4-mini",
    }),
  ]),
  createConversation("pck-wide-warning", [
    createPreview({
      id: 151,
      invokeId: "invoke-wide-warning-current",
      occurredAt: "2026-04-04T10:02:06Z",
      status: "http_429",
      failureClass: "service_failure",
      failureKind: "upstream_rate_limit",
      errorMessage: "upstream rate limit reached for the current account",
      upstreamAccountName: "wide-warning@example.com",
      requestedServiceTier: "priority",
      serviceTier: "priority",
      tUpstreamTtfbMs: null,
      tUpstreamStreamMs: null,
      tTotalMs: 1820,
    }),
    createPreview({
      id: 150,
      invokeId: "invoke-wide-warning-previous",
      occurredAt: "2026-04-04T09:58:52Z",
      status: "completed",
      upstreamAccountName: "wide-warning@example.com",
    }),
  ]),
]);

const fourCardParallelThreeSlotProofResponse = createResponse([
  requireFixture(currentAndPreviousResponse.conversations[0]),
  ...wideDesktopResponse.conversations.slice(0, 3),
]);

const summaryThresholdResponse = createResponse([
  createConversation("pck-story-threshold-default", [
    createPreview({
      id: 301,
      invokeId: "story-threshold-default-current",
      occurredAt: "2026-04-04T10:06:00Z",
      status: "running",
      totalTokens: 1_000,
      inputTokens: 700,
      outputTokens: 200,
      cacheInputTokens: 955,
      cost: 0.0586,
      reasoningTokens: 20,
    }),
    createPreview({
      id: 300,
      invokeId: "story-threshold-warning-previous",
      occurredAt: "2026-04-04T10:05:00Z",
      status: "completed",
      totalTokens: 1_000,
      inputTokens: 700,
      outputTokens: 200,
      cacheInputTokens: 899,
      cost: 0.1001,
      reasoningTokens: 20,
    }),
  ]),
  createConversation("pck-story-threshold-error", [
    createPreview({
      id: 311,
      invokeId: "story-threshold-error-current",
      occurredAt: "2026-04-04T10:04:30Z",
      status: "completed",
      totalTokens: 1_000,
      inputTokens: 700,
      outputTokens: 200,
      cacheInputTokens: 499,
      cost: 0.5001,
      reasoningTokens: 20,
    }),
  ]),
]);

const summaryThresholdUpstreamActivity = {
  ...createUpstreamAccountActivityStoryResponse(4),
  accounts: [
    {
      ...createUpstreamAccountActivityStoryResponse(4).accounts[0],
      recentInvocations: [
        createPreview({
          id: 320,
          invokeId: "story-upstream-threshold-warning",
          promptCacheKey: "story-upstream-threshold-warning",
          occurredAt: "2026-04-04T10:05:00Z",
          status: "success",
          totalTokens: 1_000,
          inputTokens: 700,
          outputTokens: 200,
          cacheInputTokens: 899,
          cost: 0.1001,
          reasoningTokens: 20,
        }),
        createPreview({
          id: 321,
          invokeId: "story-upstream-threshold-error",
          promptCacheKey: "story-upstream-threshold-error",
          occurredAt: "2026-04-04T10:04:00Z",
          status: "failed",
          totalTokens: 1_000,
          inputTokens: 700,
          outputTokens: 200,
          cacheInputTokens: 499,
          cost: 0.5001,
          reasoningTokens: 20,
        }),
        createPreview({
          id: 322,
          invokeId: "story-upstream-threshold-default",
          promptCacheKey: "story-upstream-threshold-default",
          occurredAt: "2026-04-04T10:03:00Z",
          status: "success",
          totalTokens: 1_000,
          inputTokens: 700,
          outputTokens: 200,
          cacheInputTokens: 900,
          cost: 0.1,
          reasoningTokens: 20,
        }),
        createPreview({
          id: 323,
          invokeId: "story-upstream-threshold-boundary",
          promptCacheKey: "story-upstream-threshold-boundary",
          occurredAt: "2026-04-04T10:02:00Z",
          status: "success",
          totalTokens: 1_000,
          inputTokens: 700,
          outputTokens: 200,
          cacheInputTokens: 500,
          cost: 0.5,
          reasoningTokens: 20,
        }),
      ],
    },
  ],
} satisfies UpstreamAccountActivityResponse;

function buildVirtualizedLargeResponse(
  prefix: string,
  total: number,
): PromptCacheConversationsResponse {
  const conversations = Array.from({ length: total }, (_, index) => {
    const currentAt = new Date(Date.UTC(2026, 3, 4, 10, 59, 0) - index * 70_000).toISOString();
    const previousAt = new Date(Date.parse(currentAt) - 160_000).toISOString();
    const inFlight = index % 7 === 0 ? "running" : index % 5 === 0 ? "pending" : null;
    const currentStatus = inFlight ?? (index % 6 === 0 ? "http_429" : "completed");
    return createConversation(
      `${prefix}-${String(index + 1).padStart(3, "0")}`,
      [
        createPreview({
          id: 2_000 + index * 2,
          invokeId: `${prefix}-invoke-${index + 1}-current`,
          occurredAt: currentAt,
          status: currentStatus,
          upstreamAccountName: `${prefix}-account-${(index % 9) + 1}@example.com`,
          reasoningEffort: index % 2 === 0 ? "medium" : "high",
          totalTokens: 280 + index * 7,
          cost: Number((0.014 + index * 0.0006).toFixed(4)),
          tTotalMs: inFlight ? null : 420 + index * 9,
        }),
        createPreview({
          id: 2_001 + index * 2,
          invokeId: `${prefix}-invoke-${index + 1}-previous`,
          occurredAt: previousAt,
          status: "completed",
          model: "gpt-5.4-mini",
          upstreamAccountName: `${prefix}-account-${(index % 9) + 1}@example.com`,
          totalTokens: 180 + index * 5,
          cost: Number((0.009 + index * 0.0004).toFixed(4)),
          tTotalMs: 360 + index * 7,
        }),
      ],
      {
        createdAt: currentAt,
        lastActivityAt: currentAt,
        lastTerminalAt: inFlight ? previousAt : currentAt,
        lastInFlightAt: inFlight ? currentAt : null,
        cursor: `${prefix}-cursor-${index + 1}`,
        requestCount: 12 + index,
        totalTokens: 2_400 + index * 55,
        totalCost: Number((0.12 + index * 0.006).toFixed(4)),
      },
    );
  });

  return createResponse(conversations);
}

function buildCards(response: PromptCacheConversationsResponse) {
  return mapPromptCacheConversationsToDashboardCards(response);
}

const createdAtDescendingOrderCards = buildCards(createdAtDescendingOrderResponse);
const createdAtDescendingOrderKeys = [...createdAtDescendingOrderResponse.conversations]
  .sort(
    (left, right) =>
      right.createdAt.localeCompare(left.createdAt) ||
      right.promptCacheKey.localeCompare(left.promptCacheKey),
  )
  .map((conversation) => conversation.promptCacheKey);

const gpt56ModelContextResponse = createResponse([
  createConversation("story-gpt56-model-context", [
    createPreview({
      id: 9_560,
      invokeId: "story-gpt56-model-context-invoke",
      occurredAt: "2026-04-04T10:04:00Z",
      status: "completed",
      model: "gpt-5.6-sol",
      requestModel: "gpt-5.6-sol",
      responseModel: "gpt-5.6-sol",
      reasoningEffort: "max",
      requestedServiceTier: "priority",
      serviceTier: "priority",
      transport: "websocket",
      totalTokens: 12_520,
      cost: 0.014,
    }),
  ]),
]);

const upstreamAccountSortBaseResponse = createUpstreamAccountActivityStoryResponse(2);
const upstreamAccountSortOrderingResponse: UpstreamAccountActivityResponse = {
  ...upstreamAccountSortBaseResponse,
  accounts: [
    {
      ...requireFixture(upstreamAccountSortBaseResponse.accounts[0]),
      accountKey: "assigned-mid",
      upstreamAccountId: 101,
      isUnassigned: false,
      displayName: "Pool Mid",
      latestConversationCreatedAt: "2026-04-04T10:03:00Z",
      lastInvocationAt: "2026-04-04T10:03:30Z",
      totalCost: 6,
      totalTokens: 600,
    },
    {
      ...requireFixture(upstreamAccountSortBaseResponse.accounts[0]),
      accountKey: "unassigned",
      upstreamAccountId: null,
      isUnassigned: true,
      displayName: "未分配上游账号",
      latestConversationCreatedAt: "2026-04-04T10:05:00Z",
      lastInvocationAt: "2026-04-04T10:05:00Z",
      totalCost: 999,
      totalTokens: 99_999,
    },
    {
      ...requireFixture(upstreamAccountSortBaseResponse.accounts[0]),
      accountKey: "assigned-high",
      upstreamAccountId: 102,
      isUnassigned: false,
      displayName: "Pool High",
      latestConversationCreatedAt: "2026-04-04T10:04:00Z",
      lastInvocationAt: "2026-04-04T10:04:30Z",
      totalCost: 9,
      totalTokens: 900,
    },
  ],
};

function getStorySequenceIdForPromptCacheKey(promptCacheKey: string) {
  return formatDashboardWorkingConversationSequenceId(
    `WC-${hashDashboardWorkingConversationKey(promptCacheKey).slice(0, 6)}`,
  );
}

const virtualizedLargeDatasetResponse = buildVirtualizedLargeResponse("pck-virtual", 72);
const virtualizedLargeDatasetCards = buildCards(virtualizedLargeDatasetResponse);
const headInsertBaseResponse = buildVirtualizedLargeResponse("pck-anchor", 56);

function HeadInsertAnchorStory() {
  const baseConversations = useMemo(() => headInsertBaseResponse.conversations, []);
  const [cards, setCards] = useState(() => buildCards(createResponse(baseConversations)));
  const [status, setStatus] = useState("waiting");

  useEffect(() => {
    const timer = window.setTimeout(() => {
      setCards(
        buildCards(
          createResponse([
            createConversation(
              "pck-anchor-new-head",
              [
                createPreview({
                  id: 9_991,
                  invokeId: "invoke-anchor-new-head-current",
                  occurredAt: "2026-04-04T11:00:12Z",
                  status: "running",
                  upstreamAccountName: "anchor-new-head@example.com",
                  reasoningEffort: "high",
                  tTotalMs: null,
                }),
                createPreview({
                  id: 9_990,
                  invokeId: "invoke-anchor-new-head-previous",
                  occurredAt: "2026-04-04T10:58:02Z",
                  status: "completed",
                  model: "gpt-5.4-mini",
                  upstreamAccountName: "anchor-new-head@example.com",
                  totalTokens: 222,
                  cost: 0.0142,
                }),
              ],
              {
                createdAt: "2026-04-04T11:00:12Z",
                lastActivityAt: "2026-04-04T11:00:12Z",
                lastTerminalAt: "2026-04-04T10:58:02Z",
                lastInFlightAt: "2026-04-04T11:00:12Z",
                cursor: "pck-anchor-new-head",
                requestCount: 27,
                totalTokens: 4_220,
                totalCost: 0.2042,
              },
            ),
            ...baseConversations,
          ]),
        ),
      );
      setStatus("prepended:pck-anchor-new-head");
    }, 450);

    return () => window.clearTimeout(timer);
  }, [baseConversations]);

  return (
    <div data-testid="story-head-insert-anchor" className="space-y-3">
      <div
        data-testid="story-head-insert-status"
        className="rounded-xl border border-base-300/75 bg-base-100/70 px-4 py-3 text-sm text-base-content/75"
      >
        Auto prepend status: <span className="font-mono">{status}</span>
      </div>
      <DashboardWorkingConversationsSection
        activeRange="today"
        cards={cards}
        totalMatched={cards.length}
        isLoading={false}
        error={null}
      />
    </div>
  );
}

function buildStoryMockData(
  response: PromptCacheConversationsResponse,
  historyInvocationsByPromptCacheKey = new Map<
    string,
    PromptCacheConversationInvocationPreview[]
  >(),
) {
  const recordsByInvokeId = new Map<string, ApiInvocation>();
  const recordsByPromptCacheKey = new Map<string, ApiInvocation[]>();
  const detailByRecordId = new Map<number, ApiInvocationRecordDetailResponse>();
  const responseBodyByRecordId = new Map<number, ApiInvocationResponseBodyResponse>();
  const poolAttemptsByInvokeId = new Map<string, ApiPoolUpstreamRequestAttempt[]>();
  const ingestPreview = (
    conversation: PromptCacheConversation,
    preview: PromptCacheConversationInvocationPreview,
  ) => {
    const record = {
      ...buildRecordFromPreview(preview),
      promptCacheKey: conversation.promptCacheKey,
    };
    recordsByInvokeId.set(record.invokeId, record);
    const promptCacheKey = record.promptCacheKey?.trim();
    if (promptCacheKey) {
      recordsByPromptCacheKey.set(promptCacheKey, [
        ...(recordsByPromptCacheKey.get(promptCacheKey) ?? []),
        record,
      ]);
    }
    const normalizedStatus = (record.status ?? "").trim().toLowerCase();
    const isAbnormal =
      record.failureClass === "service_failure" ||
      normalizedStatus === "failed" ||
      normalizedStatus.startsWith("http_");
    if (isAbnormal) {
      detailByRecordId.set(record.id, {
        id: record.id,
        abnormalResponseBody: {
          available: true,
          previewText: JSON.stringify({
            error: {
              message: record.errorMessage ?? "upstream failure",
            },
          }),
          hasMore: false,
        },
      });
      responseBodyByRecordId.set(record.id, {
        available: true,
        bodyText: JSON.stringify({
          error: {
            message: record.errorMessage ?? "upstream failure",
          },
          invokeId: record.invokeId,
        }),
      });
    }
    if (
      (record.routeMode ?? "").trim().toLowerCase() === "pool" &&
      typeof record.upstreamAccountId === "number"
    ) {
      poolAttemptsByInvokeId.set(record.invokeId, [
        {
          attemptId: formatStoryAttemptId(record.id * 10 + 1),
          invokeId: record.invokeId,
          occurredAt: record.occurredAt,
          endpoint: record.endpoint ?? "/v1/responses",
          attemptIndex: 1,
          distinctAccountIndex: 1,
          sameAccountRetryIndex: 1,
          status: isAbnormal ? "failed" : "success",
          httpStatus: normalizedStatus.startsWith("http_")
            ? Number(normalizedStatus.slice("http_".length))
            : 200,
          createdAt: record.createdAt,
          upstreamAccountId: record.upstreamAccountId ?? null,
          upstreamAccountName: record.upstreamAccountName ?? null,
          firstByteLatencyMs: record.tUpstreamTtfbMs ?? null,
        },
      ]);
    }
  };
  for (const conversation of response.conversations) {
    const historyInvocations =
      historyInvocationsByPromptCacheKey.get(conversation.promptCacheKey) ??
      conversation.recentInvocations;
    for (const preview of historyInvocations) {
      ingestPreview(conversation, preview);
    }
  }
  return {
    recordsByInvokeId,
    recordsByPromptCacheKey,
    detailByRecordId,
    responseBodyByRecordId,
    poolAttemptsByInvokeId,
  };
}

function buildStoryInvocationSummary(records: ApiInvocation[]) {
  const resolvedFailureClass = (record: ApiInvocation) => {
    const failureClass = (record.failureClass ?? "").trim().toLowerCase();
    if (
      failureClass === "service_failure" ||
      failureClass === "client_failure" ||
      failureClass === "client_abort"
    ) {
      return failureClass;
    }
    return "none";
  };
  const isSuccessRecord = (record: ApiInvocation) => {
    const status = (record.status ?? "").trim().toLowerCase();
    const errorMessage = (record.errorMessage ?? "").trim();
    return (
      resolvedFailureClass(record) === "none" &&
      (status === "success" ||
        status === "completed" ||
        (status === "http_200" && errorMessage === ""))
    );
  };
  const failureRecords = records.filter((record) => resolvedFailureClass(record) !== "none");
  const successRecords = records.filter(isSuccessRecord);
  const totalMsRecords = records.filter(
    (record) => typeof record.tTotalMs === "number" && Number.isFinite(record.tTotalMs),
  );
  const avgTotalMs =
    totalMsRecords.length === 0
      ? null
      : totalMsRecords.reduce((sum, record) => sum + (record.tTotalMs ?? 0), 0) /
        totalMsRecords.length;

  return {
    snapshotId: 1,
    newRecordsCount: 0,
    totalCount: records.length,
    successCount: successRecords.length,
    failureCount: failureRecords.length,
    totalCost: records.reduce((sum, record) => sum + (record.cost ?? 0), 0),
    totalTokens: records.reduce((sum, record) => sum + (record.totalTokens ?? 0), 0),
    token: {
      requestCount: records.length,
      totalTokens: records.reduce((sum, record) => sum + (record.totalTokens ?? 0), 0),
      avgTokensPerRequest:
        records.length === 0
          ? 0
          : records.reduce((sum, record) => sum + (record.totalTokens ?? 0), 0) / records.length,
      cacheInputTokens: records.reduce((sum, record) => sum + (record.cacheInputTokens ?? 0), 0),
      totalCost: records.reduce((sum, record) => sum + (record.cost ?? 0), 0),
    },
    network: {
      avgTtfbMs: null,
      p95TtfbMs: null,
      avgTotalMs,
      p95TotalMs: avgTotalMs,
    },
    exception: {
      failureCount: failureRecords.length,
      serviceFailureCount: failureRecords.filter(
        (record) => resolvedFailureClass(record) === "service_failure",
      ).length,
      clientFailureCount: failureRecords.filter(
        (record) => resolvedFailureClass(record) === "client_failure",
      ).length,
      clientAbortCount: failureRecords.filter(
        (record) => resolvedFailureClass(record) === "client_abort",
      ).length,
      actionableFailureCount: failureRecords.filter(
        (record) => resolvedFailureClass(record) === "service_failure",
      ).length,
    },
  };
}

function resolveInitialSelection(
  cards: ReturnType<typeof buildCards>,
  target?: {
    promptCacheKey: string;
    slotKind: "current" | "previous" | "earlier";
  },
): DashboardWorkingConversationInvocationSelection | null {
  if (!target) return null;
  const card = cards.find((candidate) => candidate.promptCacheKey === target.promptCacheKey);
  if (!card) return null;
  const invocation =
    target.slotKind === "earlier"
      ? card.earlierInvocation
      : target.slotKind === "previous"
        ? card.previousInvocation
        : card.currentInvocation;
  if (!invocation) return null;
  return {
    slotKind: target.slotKind,
    conversationSequenceId: card.conversationSequenceId,
    promptCacheKey: card.promptCacheKey,
    invocation,
  };
}

function StoryAccountDrawer({
  account,
  onClose,
}: {
  account: {
    id: number;
    label: string;
    tab: "overview" | "routing" | "healthEvents";
  } | null;
  onClose: () => void;
}) {
  const titleId = useId();

  return (
    <AccountDetailDrawerShell
      open={account != null}
      labelledBy={titleId}
      closeLabel="Close account drawer"
      onClose={onClose}
      header={null}
    >
      {account ? (
        <div
          data-testid="story-account-drawer"
          className="space-y-4 rounded-[1.6rem] border border-base-300/80 bg-base-100/85 p-5"
        >
          <div className="space-y-2">
            <p className="text-xs font-semibold uppercase tracking-[0.18em] text-primary/70">
              Shared Account Drawer
            </p>
            <h2 id={titleId} className="text-xl font-semibold text-base-content">
              {account.label}
            </h2>
            <p className="font-mono text-sm text-base-content/60">Account ID {account.id}</p>
            <p
              data-testid="story-account-drawer-tab"
              className="font-mono text-sm text-base-content/60"
            >
              Tab {account.tab}
            </p>
          </div>
          <p className="text-sm leading-6 text-base-content/70">
            Mock shared account detail drawer used to verify that Dashboard account clicks switch
            away from the invocation drawer without opening both drawers at once.
          </p>
        </div>
      ) : null}
    </AccountDetailDrawerShell>
  );
}

class StoryNoopEventSource implements EventTarget {
  static CONNECTING = 0;
  static OPEN = 1;
  static CLOSED = 2;

  readonly url: string;
  readonly withCredentials = false;
  readyState = StoryNoopEventSource.CONNECTING;
  onerror: ((this: EventSource, ev: Event) => unknown) | null = null;
  onmessage: ((this: EventSource, ev: MessageEvent<string>) => unknown) | null = null;
  onopen: ((this: EventSource, ev: Event) => unknown) | null = null;

  private listeners = new Map<string, Set<EventListenerOrEventListenerObject>>();

  constructor(url: string | URL) {
    this.url = typeof url === "string" ? url : url.toString();
    window.setTimeout(() => {
      if (this.readyState === StoryNoopEventSource.CLOSED) return;
      this.readyState = StoryNoopEventSource.OPEN;
      this.dispatchEvent(new Event("open"));
    }, 0);
  }

  addEventListener(type: string, listener: EventListenerOrEventListenerObject | null) {
    if (!listener) return;
    const bucket = this.listeners.get(type) ?? new Set<EventListenerOrEventListenerObject>();
    bucket.add(listener);
    this.listeners.set(type, bucket);
  }

  removeEventListener(type: string, listener: EventListenerOrEventListenerObject | null) {
    if (!listener) return;
    this.listeners.get(type)?.delete(listener);
  }

  dispatchEvent(event: Event) {
    if (event.type === "open") {
      this.onopen?.call(this as unknown as EventSource, event);
    }
    if (event.type === "message") {
      this.onmessage?.call(this as unknown as EventSource, event as MessageEvent<string>);
    }
    if (event.type === "error") {
      this.onerror?.call(this as unknown as EventSource, event);
    }
    for (const listener of this.listeners.get(event.type) ?? []) {
      if (typeof listener === "function") {
        listener(event);
      } else {
        listener.handleEvent(event);
      }
    }
    return true;
  }

  close() {
    this.readyState = StoryNoopEventSource.CLOSED;
  }
}

export {
  assignedAccountFailureSemanticsResponse,
  buildCards,
  buildDashboardHistoryEvidenceFixtures,
  buildStoryInvocationSummary,
  buildStoryMockData,
  buildVirtualizedLargeResponse,
  createdAtDescendingOrderCards,
  createdAtDescendingOrderKeys,
  createdAtDescendingOrderResponse,
  fourCardParallelThreeSlotProofResponse,
  getStorySequenceIdForPromptCacheKey,
  gpt56ModelContextResponse,
  HeadInsertAnchorStory,
  headInsertBaseResponse,
  interruptedRecoveryResponse,
  resolveInitialSelection,
  StoryAccountDrawer,
  StoryNoopEventSource,
  summaryThresholdResponse,
  summaryThresholdUpstreamActivity,
  upstreamAccountSortBaseResponse,
  upstreamAccountSortOrderingResponse,
  virtualizedLargeDatasetCards,
  virtualizedLargeDatasetResponse,
  wideDesktopResponse,
  wideDesktopRunningCurrent,
};
