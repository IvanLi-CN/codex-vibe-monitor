import type { Meta, StoryObj } from "@storybook/react-vite";
import { type ReactNode, useLayoutEffect } from "react";
import { MemoryRouter } from "react-router-dom";
import { expect, userEvent, waitFor, within } from "storybook/test";
import App from "../../App";
import { I18nProvider } from "../../i18n";
import type {
  DashboardActivityResponse,
  ParallelWorkStatsResponse,
  PromptCacheConversation,
  PromptCacheConversationInvocationPreview,
  PromptCacheConversationsResponse,
  StatsResponse,
  TimeseriesPoint,
  TimeseriesResponse,
  UpstreamAccountActivityAccount,
} from "../../lib/api";
import { DASHBOARD_PERFORMANCE_DIAGNOSTICS_STORAGE_KEY } from "../../lib/dashboardPerformanceDiagnostics";
import DashboardPage from "../../pages/Dashboard";
import {
  FullPageStorySurface,
  StorybookPageEnvironment,
} from "../../storybook/storybookPageHelpers";
import { getStorybookPageSseController } from "../../storybook/storybookPageSse";
import { jsonResponse } from "../../storybook/storybookResponse";
import { DASHBOARD_ACTIVITY_RANGE_STORAGE_KEY } from "./dashboardActivityRange";

type DashboardScenario = "default" | "degraded" | "readmeDense";

type DashboardStoryParameters = {
  scenario?: DashboardScenario;
  enableDiagnostics?: boolean;
};

type ReadmeWorkingConversationSeed = {
  promptCacheKey: string;
  current: Partial<PromptCacheConversationInvocationPreview> & {
    id: number;
    invokeId: string;
    occurredAt: string;
    status: string;
  };
  previous: Partial<PromptCacheConversationInvocationPreview> & {
    id: number;
    invokeId: string;
    occurredAt: string;
    status: string;
  };
  extraHistory?: Array<
    Partial<PromptCacheConversationInvocationPreview> & {
      id: number;
      invokeId: string;
      occurredAt: string;
      status: string;
    }
  >;
};

const WORKING_CONVERSATION_STORY_BASE_SNAPSHOT = "2026-04-06T12:00:00.000Z";
const WORKING_CONVERSATION_STORY_BASE_SNAPSHOT_MS = Date.parse(
  WORKING_CONVERSATION_STORY_BASE_SNAPSHOT,
);

function shiftWorkingConversationStoryIso(value: string, snapshotAtMs: number) {
  const occurredAtMs = Date.parse(value);
  if (!Number.isFinite(occurredAtMs)) return value;
  const deltaMs = occurredAtMs - WORKING_CONVERSATION_STORY_BASE_SNAPSHOT_MS;
  return new Date(snapshotAtMs + deltaMs).toISOString();
}

function withRelativeOccurredAt<
  T extends {
    occurredAt: string;
  },
>(value: T, snapshotAtMs: number): T {
  return {
    ...value,
    occurredAt: shiftWorkingConversationStoryIso(value.occurredAt, snapshotAtMs),
  };
}

function DashboardRangeStorageReset({ children }: { children: ReactNode }) {
  useLayoutEffect(() => {
    const previousValue = window.localStorage.getItem(DASHBOARD_ACTIVITY_RANGE_STORAGE_KEY);
    window.localStorage.removeItem(DASHBOARD_ACTIVITY_RANGE_STORAGE_KEY);

    return () => {
      if (previousValue === null) {
        window.localStorage.removeItem(DASHBOARD_ACTIVITY_RANGE_STORAGE_KEY);
      } else {
        window.localStorage.setItem(DASHBOARD_ACTIVITY_RANGE_STORAGE_KEY, previousValue);
      }
    };
  }, []);

  return <>{children}</>;
}

function DashboardDiagnosticsStorageReset({
  children,
  enabled = false,
}: {
  children: ReactNode;
  enabled?: boolean;
}) {
  useLayoutEffect(() => {
    const previousValue = window.localStorage.getItem(
      DASHBOARD_PERFORMANCE_DIAGNOSTICS_STORAGE_KEY,
    );
    if (enabled) {
      window.localStorage.setItem(DASHBOARD_PERFORMANCE_DIAGNOSTICS_STORAGE_KEY, "1");
    } else {
      window.localStorage.removeItem(DASHBOARD_PERFORMANCE_DIAGNOSTICS_STORAGE_KEY);
    }

    return () => {
      if (previousValue === null) {
        window.localStorage.removeItem(DASHBOARD_PERFORMANCE_DIAGNOSTICS_STORAGE_KEY);
      } else {
        window.localStorage.setItem(DASHBOARD_PERFORMANCE_DIAGNOSTICS_STORAGE_KEY, previousValue);
      }
    };
  }, [enabled]);

  return <>{children}</>;
}

function buildSummary(overrides: Partial<StatsResponse>): StatsResponse {
  return {
    totalCount: 0,
    successCount: 0,
    failureCount: 0,
    totalCost: 0,
    totalTokens: 0,
    ...overrides,
  };
}

function buildDashboardActivityResponse({
  range,
  summary,
  includeAccounts,
}: {
  range: string;
  summary: StatsResponse;
  includeAccounts: boolean;
}): DashboardActivityResponse {
  const accounts: UpstreamAccountActivityAccount[] = [
    {
      accountKey: "upstream:42",
      upstreamAccountId: 42,
      displayName: "dzw",
      groupName: "Primary",
      planType: "enterprise",
      requestCount: 1920,
      successCount: 1840,
      failureCount: 80,
      nonSuccessCount: 80,
      totalTokens: 11_200_000,
      successTokens: 10_850_000,
      nonSuccessTokens: 350_000,
      failureTokens: 350_000,
      failureCost: 0.22,
      totalCost: 24.32,
      usageBreakdown: {
        cacheWriteTokens: 7_300_000,
        cacheReadTokens: 2_100_000,
        outputTokens: 1_800_000,
        costs: {
          input: 3.8,
          cacheWrite: 5.9,
          cacheRead: 0.62,
          output: 13.2,
          reasoning: 0.8,
          unknown: 0,
        },
        models: [],
      },
      cacheHitRate: 0.927,
      tokensPerMinute: 610,
      spendRate: 0.73,
      firstByteAvgMs: 2145,
      firstResponseByteTotalAvgMs: 2145,
      avgTotalMs: 12_650,
      inProgressInvocationCount: 8,
      retryInvocationCount: 1,
      effectiveRoutingRule: {
        allowCutOut: false,
        allowCutIn: false,
        priorityTier: "no_new",
        fastModeRewriteMode: "force_add",
        concurrencyLimit: 3,
        upstream429RetryEnabled: true,
        upstream429MaxRetries: 2,
        sourceTagIds: [],
        sourceTagNames: [],
      },
      recentInvocations: [],
    },
    {
      accountKey: "upstream:77",
      upstreamAccountId: 77,
      displayName: "CIII",
      groupName: "Overflow",
      planType: "team",
      requestCount: 1508,
      successCount: 1456,
      failureCount: 52,
      nonSuccessCount: 52,
      totalTokens: 7_564_200,
      successTokens: 7_390_000,
      nonSuccessTokens: 174_200,
      failureTokens: 174_200,
      failureCost: 0.12,
      totalCost: 18.54,
      usageBreakdown: {
        cacheWriteTokens: 4_850_000,
        cacheReadTokens: 1_600_000,
        outputTokens: 1_114_200,
        costs: {
          input: 2.6,
          cacheWrite: 4.2,
          cacheRead: 0.45,
          output: 10.7,
          reasoning: 0.59,
          unknown: 0,
        },
        models: [],
      },
      cacheHitRate: 0.914,
      tokensPerMinute: 490,
      spendRate: 0.45,
      firstByteAvgMs: 803,
      firstResponseByteTotalAvgMs: 803,
      avgTotalMs: 9140,
      inProgressInvocationCount: 3,
      retryInvocationCount: 0,
      effectiveRoutingRule: {
        allowCutOut: true,
        allowCutIn: true,
        priorityTier: "fallback",
        fastModeRewriteMode: "keep_original",
        concurrencyLimit: null,
        upstream429RetryEnabled: true,
        upstream429MaxRetries: 1,
        sourceTagIds: [],
        sourceTagNames: [],
      },
      recentInvocations: [],
    },
  ];
  return {
    range,
    rangeStart: "2026-04-09T00:00:00.000Z",
    rangeEnd: "2026-04-09T12:24:00.000Z",
    snapshotId: 1775718240000,
    rateWindow: {
      start: "2026-04-09T12:19:00.000Z",
      end: "2026-04-09T12:24:00.000Z",
      windowMinutes: 5,
      mode: "account_active_tail_sum",
    },
    summary: {
      stats: {
        ...summary,
        inProgressConversationCount: 11,
        inProgressRetryConversationCount: 1,
        inProgressAvgWaitMs: 1779,
      },
      tokensPerMinute: 1100,
      spendRate: 1.18,
    },
    accounts: includeAccounts ? accounts : undefined,
  };
}

function buildTimeseriesPoints({
  count,
  bucketSeconds,
  startMs,
  valueOffset = 0,
}: {
  count: number;
  bucketSeconds: number;
  startMs: number;
  valueOffset?: number;
}) {
  return Array.from({ length: count }, (_, index) => {
    const bucketStartMs = startMs + index * bucketSeconds * 1000;
    const bucketEndMs = bucketStartMs + bucketSeconds * 1000;
    const pulse = (index + valueOffset) % 24;
    const totalCount =
      pulse >= 7 && pulse <= 11
        ? 24 + (index % 6)
        : pulse >= 18 && pulse <= 22
          ? 16 + (index % 5)
          : index % 4;
    const failureCount = totalCount === 0 ? 0 : index % 11 === 0 ? 2 : index % 7 === 0 ? 1 : 0;
    const successCount = Math.max(totalCount - failureCount, 0);
    return {
      bucketStart: new Date(bucketStartMs).toISOString(),
      bucketEnd: new Date(bucketEndMs).toISOString(),
      totalCount,
      successCount,
      failureCount,
      totalTokens: totalCount * 3200,
      cacheInputTokens: totalCount * 720,
      totalCost: Number((totalCount * 0.018).toFixed(4)),
      firstResponseByteTotalSampleCount: totalCount,
      firstResponseByteTotalAvgMs: totalCount > 0 ? 760 + ((index * 19) % 280) : null,
    } satisfies TimeseriesPoint;
  });
}

function buildTodayTimeseriesPoints({
  startMs,
  endMs,
  summary,
}: {
  startMs: number;
  endMs: number;
  summary: StatsResponse;
}) {
  const count = Math.floor((endMs - startMs) / 60_000) + 1;
  const minuteIndexes = Array.from({ length: count }, (_, index) => index);
  const successCounts = distributeInteger(
    summary.successCount,
    minuteIndexes.map((index) => buildActivityWeight(index, "success")),
  );
  const failureCounts = distributeInteger(
    summary.failureCount,
    minuteIndexes.map((index) => buildActivityWeight(index, "failure")),
  );
  const tokenTotals = distributeInteger(
    summary.totalTokens,
    minuteIndexes.map((index) =>
      buildUsageWeight(successCounts[index] + failureCounts[index], index, "tokens"),
    ),
  );
  const costCents = distributeInteger(
    Math.round(summary.totalCost * 100),
    minuteIndexes.map((index) =>
      buildUsageWeight(successCounts[index] + failureCounts[index], index, "cost"),
    ),
  );

  return minuteIndexes.map((index) => {
    const bucketStartMs = startMs + index * 60_000;
    const bucketEndMs = bucketStartMs + 60_000;
    const successCount = successCounts[index] ?? 0;
    const failureCount = failureCounts[index] ?? 0;
    return {
      bucketStart: new Date(bucketStartMs).toISOString(),
      bucketEnd: new Date(bucketEndMs).toISOString(),
      totalCount: successCount + failureCount,
      successCount,
      failureCount,
      totalTokens: tokenTotals[index] ?? 0,
      cacheInputTokens: Math.round((tokenTotals[index] ?? 0) * 0.23),
      totalCost: Number(((costCents[index] ?? 0) / 100).toFixed(2)),
      firstResponseByteTotalSampleCount: successCount + failureCount,
      firstResponseByteTotalAvgMs:
        successCount + failureCount > 0 ? 820 + ((index * 37) % 340) : null,
    } satisfies TimeseriesPoint;
  });
}

function buildActivityWeight(index: number, mode: "success" | "failure") {
  const hour = Math.floor(index / 60);
  const minute = index % 60;
  const rush = hour < 6 ? 2 : hour < 9 ? 5 : hour < 12 ? 9 : 4;
  const pulse = (index % 11) + 1;
  const boundaryBoost = minute % 15 === 0 ? 4 : minute % 5 === 0 ? 2 : 0;
  const failureBias = mode === "failure" ? (hour >= 9 && hour <= 11 ? 6 : 3) : 0;
  return rush + pulse + boundaryBoost + failureBias;
}

function buildUsageWeight(totalCount: number, index: number, mode: "tokens" | "cost") {
  const base = Math.max(totalCount, 1);
  if (mode === "tokens") {
    return base * (14 + (index % 17)) + ((index % 7) + 1) * 19;
  }
  return base * (6 + (index % 9)) + ((index % 5) + 1) * 7;
}

function distributeInteger(total: number, weights: number[]) {
  if (weights.length === 0) return [];
  const sanitizedWeights = weights.map((weight) =>
    Number.isFinite(weight) && weight > 0 ? weight : 1,
  );
  const weightSum = sanitizedWeights.reduce((sum, weight) => sum + weight, 0);
  if (weightSum <= 0) {
    const evenShare = Math.floor(total / weights.length);
    const remainder = total - evenShare * weights.length;
    return weights.map((_, index) => evenShare + (index < remainder ? 1 : 0));
  }

  const rawAllocations = sanitizedWeights.map((weight) => (total * weight) / weightSum);
  const allocations = rawAllocations.map((value) => Math.floor(value));
  let remainder = total - allocations.reduce((sum, value) => sum + value, 0);

  if (remainder > 0) {
    const remainders = rawAllocations
      .map((value, index) => ({
        index,
        fraction: value - Math.floor(value),
        weight: sanitizedWeights[index],
      }))
      .sort((left, right) => {
        if (right.fraction !== left.fraction) return right.fraction - left.fraction;
        if (right.weight !== left.weight) return right.weight - left.weight;
        return left.index - right.index;
      });

    for (let cursor = 0; cursor < remainders.length && remainder > 0; cursor += 1, remainder -= 1) {
      allocations[remainders[cursor].index] += 1;
    }
  }

  return allocations;
}

function buildTimeseriesResponse(options: {
  rangeStart: string;
  rangeEnd: string;
  bucketSeconds: number;
  effectiveBucket?: string;
  availableBuckets?: string[];
  points: TimeseriesPoint[];
}): TimeseriesResponse {
  return {
    rangeStart: options.rangeStart,
    rangeEnd: options.rangeEnd,
    bucketSeconds: options.bucketSeconds,
    effectiveBucket: options.effectiveBucket,
    availableBuckets: options.availableBuckets,
    bucketLimitedToDaily: false,
    points: options.points,
  };
}

function buildParallelWorkWindow(
  counts: number[],
  {
    rangeStart,
    bucketSeconds,
  }: {
    rangeStart: string;
    bucketSeconds: number;
  },
): ParallelWorkStatsResponse["current"] {
  const startMs = Date.parse(rangeStart);
  const rangeEnd = new Date(startMs + counts.length * bucketSeconds * 1000).toISOString();
  return {
    rangeStart,
    rangeEnd,
    bucketSeconds,
    completeBucketCount: counts.length,
    activeBucketCount: counts.filter((count) => count > 0).length,
    minCount: counts.length > 0 ? Math.min(...counts) : null,
    maxCount: counts.length > 0 ? Math.max(...counts) : null,
    avgCount:
      counts.length > 0
        ? Number((counts.reduce((sum, count) => sum + count, 0) / counts.length).toFixed(2))
        : null,
    effectiveTimeZone: "Asia/Shanghai",
    timeZoneFallback: false,
    points: counts.map((parallelCount, index) => ({
      bucketStart: new Date(startMs + index * bucketSeconds * 1000).toISOString(),
      bucketEnd: new Date(startMs + (index + 1) * bucketSeconds * 1000).toISOString(),
      parallelCount,
    })),
    conversations: [],
  };
}

function buildParallelWorkResponse(windows: {
  current: ParallelWorkStatsResponse["current"];
  minute7d: ParallelWorkStatsResponse["current"];
  hour30d: ParallelWorkStatsResponse["current"];
  dayAll: ParallelWorkStatsResponse["current"];
}): ParallelWorkStatsResponse {
  return windows;
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
    failureClass: overrides.failureClass ?? "none",
    routeMode: overrides.routeMode ?? "pool",
    model: overrides.model ?? "gpt-5.4",
    totalTokens: overrides.totalTokens ?? 280,
    cost: overrides.cost ?? 0.0178,
    proxyDisplayName: overrides.proxyDisplayName ?? "tokyo-edge-01",
    upstreamAccountId: overrides.upstreamAccountId ?? 42,
    upstreamAccountName: overrides.upstreamAccountName ?? "pool-alpha@example.com",
    endpoint: overrides.endpoint ?? "/v1/responses",
    source: overrides.source ?? "pool",
    inputTokens: overrides.inputTokens ?? 164,
    outputTokens: overrides.outputTokens ?? 116,
    cacheInputTokens: overrides.cacheInputTokens ?? 42,
    reasoningTokens: overrides.reasoningTokens ?? 18,
    reasoningEffort: overrides.reasoningEffort ?? "high",
    errorMessage: overrides.errorMessage,
    failureKind: overrides.failureKind,
    isActionable: overrides.isActionable,
    responseContentEncoding: overrides.responseContentEncoding ?? "gzip",
    requestedServiceTier: overrides.requestedServiceTier ?? "priority",
    serviceTier: overrides.serviceTier ?? "priority",
    tReqReadMs: overrides.tReqReadMs ?? 11,
    tReqParseMs: overrides.tReqParseMs ?? 7,
    tUpstreamConnectMs: overrides.tUpstreamConnectMs ?? 84,
    tUpstreamTtfbMs: overrides.tUpstreamTtfbMs ?? 91,
    tUpstreamStreamMs: overrides.tUpstreamStreamMs ?? 220,
    tRespParseMs: overrides.tRespParseMs ?? 10,
    tPersistMs: overrides.tPersistMs ?? 8,
    tTotalMs: overrides.tTotalMs ?? 431,
  };
}

function createConversation(
  promptCacheKey: string,
  recentInvocations: PromptCacheConversationInvocationPreview[],
): PromptCacheConversation {
  return {
    promptCacheKey,
    hasEncryptedSessionOwner: false,
    encryptedOwnerAccountId: null,
    encryptedOwnerAccountName: null,
    encryptedOwnerGroupName: null,
    requestCount: recentInvocations.length,
    totalTokens: recentInvocations.reduce((sum, invocation) => sum + invocation.totalTokens, 0),
    totalCost: Number(
      recentInvocations.reduce((sum, invocation) => sum + (invocation.cost ?? 0), 0).toFixed(4),
    ),
    createdAt: recentInvocations.at(-1)?.occurredAt ?? "2026-04-06T12:00:00.000Z",
    lastActivityAt: recentInvocations[0]?.occurredAt ?? "2026-04-06T12:00:00.000Z",
    upstreamAccounts: [],
    recentInvocations,
    last24hRequests: recentInvocations.map((invocation, index) => ({
      occurredAt: invocation.occurredAt,
      status: invocation.status,
      isSuccess: invocation.status === "completed" || invocation.status === "success",
      requestTokens: 180 + index * 24,
      cumulativeTokens: 180 + index * 24,
    })),
  };
}

function createReadmeDenseWorkingConversationSeeds(): ReadmeWorkingConversationSeed[] {
  return [
    {
      promptCacheKey: "wc-readme-batch-refactor",
      current: {
        id: 101,
        invokeId: "wc-r1-a",
        occurredAt: "2026-04-06T12:00:00.000Z",
        status: "running",
        model: "gpt-5.4",
        totalTokens: 1840,
        inputTokens: 962,
        outputTokens: 734,
        cacheInputTokens: 144,
        cost: 0.0941,
        upstreamAccountName: "paisleeeinar5710 Team sandbox workflow monitor",
        proxyDisplayName: "tokyo-edge-01",
        endpoint: "/v1/responses/compact",
        tTotalMs: null,
      },
      previous: {
        id: 102,
        invokeId: "wc-r1-b",
        occurredAt: "2026-04-06T11:58:42.000Z",
        status: "success",
        model: "gpt-5.4-mini",
        totalTokens: 1428,
        inputTokens: 812,
        outputTokens: 488,
        cacheInputTokens: 128,
        cost: 0.0624,
        upstreamAccountName: "paisleeeinar5710 Team sandbox workflow monitor",
        proxyDisplayName: "tokyo-edge-01",
        endpoint: "/v1/responses/compact",
      },
      extraHistory: [
        {
          id: 103,
          invokeId: "wc-r1-c",
          occurredAt: "2026-04-06T11:56:38.000Z",
          status: "success",
          model: "gpt-5.4-mini",
          totalTokens: 1194,
          cost: 0.0486,
          upstreamAccountName: "paisleeeinar5710 Team sandbox workflow monitor",
          proxyDisplayName: "tokyo-edge-01",
          endpoint: "/v1/responses/compact",
        },
      ],
    },
    {
      promptCacheKey: "wc-readme-rate-limit-recovery",
      current: {
        id: 104,
        invokeId: "wc-r2-a",
        occurredAt: "2026-04-06T11:59:46.000Z",
        status: "pending",
        model: "gpt-5.4",
        totalTokens: 1268,
        inputTokens: 780,
        outputTokens: 382,
        cacheInputTokens: 106,
        cost: 0.0518,
        upstreamAccountName: "pool-beta@ops.example.com",
        proxyDisplayName: "singapore-edge-02",
        requestedServiceTier: "auto",
        serviceTier: "auto",
        tTotalMs: null,
      },
      previous: {
        id: 105,
        invokeId: "wc-r2-b",
        occurredAt: "2026-04-06T11:57:58.000Z",
        status: "http_429",
        failureClass: "service_failure",
        failureKind: "upstream_rate_limit",
        errorMessage: "retry-after: 15s",
        model: "gpt-5.4",
        totalTokens: 908,
        inputTokens: 620,
        outputTokens: 188,
        cacheInputTokens: 100,
        cost: 0.0381,
        upstreamAccountName: "pool-beta@ops.example.com",
        proxyDisplayName: "singapore-edge-02",
        requestedServiceTier: "auto",
        serviceTier: "auto",
      },
      extraHistory: [
        {
          id: 106,
          invokeId: "wc-r2-c",
          occurredAt: "2026-04-06T11:56:26.000Z",
          status: "success",
          model: "gpt-5.4-mini",
          totalTokens: 1414,
          cost: 0.0542,
          upstreamAccountName: "pool-beta@ops.example.com",
          proxyDisplayName: "singapore-edge-02",
        },
      ],
    },
    {
      promptCacheKey: "wc-readme-enterprise-review",
      current: {
        id: 107,
        invokeId: "wc-r3-a",
        occurredAt: "2026-04-06T11:59:24.000Z",
        status: "running",
        model: "gpt-5.4",
        totalTokens: 2252,
        inputTokens: 1188,
        outputTokens: 896,
        cacheInputTokens: 168,
        cost: 0.1029,
        upstreamAccountName: "enterprise-review@team.example.com",
        proxyDisplayName: "osaka-edge-04",
        tTotalMs: null,
      },
      previous: {
        id: 108,
        invokeId: "wc-r3-b",
        occurredAt: "2026-04-06T11:57:11.000Z",
        status: "success",
        model: "gpt-5.4",
        totalTokens: 1712,
        inputTokens: 924,
        outputTokens: 664,
        cacheInputTokens: 124,
        cost: 0.0837,
        upstreamAccountName: "enterprise-review@team.example.com",
        proxyDisplayName: "osaka-edge-04",
      },
    },
    {
      promptCacheKey: "wc-readme-prompt-cache-drift",
      current: {
        id: 109,
        invokeId: "wc-r4-a",
        occurredAt: "2026-04-06T11:58:53.000Z",
        status: "http_502",
        failureClass: "service_failure",
        failureKind: "upstream_timeout",
        errorMessage: "upstream gateway closed before first byte",
        model: "gpt-5.4",
        totalTokens: 1136,
        inputTokens: 680,
        outputTokens: 352,
        cacheInputTokens: 104,
        cost: 0.0473,
        upstreamAccountName: "cache-hotfix@infra.example.com",
        proxyDisplayName: "sfo-edge-08",
      },
      previous: {
        id: 110,
        invokeId: "wc-r4-b",
        occurredAt: "2026-04-06T11:56:44.000Z",
        status: "success",
        model: "gpt-5.4-mini",
        totalTokens: 1322,
        inputTokens: 772,
        outputTokens: 448,
        cacheInputTokens: 102,
        cost: 0.0568,
        upstreamAccountName: "cache-hotfix@infra.example.com",
        proxyDisplayName: "sfo-edge-08",
      },
      extraHistory: [
        {
          id: 111,
          invokeId: "wc-r4-c",
          occurredAt: "2026-04-06T11:55:30.000Z",
          status: "success",
          model: "gpt-5.4-mini",
          totalTokens: 984,
          cost: 0.0419,
          upstreamAccountName: "cache-hotfix@infra.example.com",
          proxyDisplayName: "sfo-edge-08",
        },
      ],
    },
    {
      promptCacheKey: "wc-readme-bulk-import-audit",
      current: {
        id: 112,
        invokeId: "wc-r5-a",
        occurredAt: "2026-04-06T11:58:21.000Z",
        status: "success",
        model: "gpt-5.4",
        totalTokens: 2088,
        inputTokens: 1048,
        outputTokens: 918,
        cacheInputTokens: 122,
        cost: 0.0974,
        upstreamAccountName: "audit-ops@example.com",
        proxyDisplayName: "seoul-edge-03",
      },
      previous: {
        id: 113,
        invokeId: "wc-r5-b",
        occurredAt: "2026-04-06T11:56:59.000Z",
        status: "running",
        model: "gpt-5.4-mini",
        totalTokens: 1472,
        inputTokens: 810,
        outputTokens: 558,
        cacheInputTokens: 104,
        cost: 0.0613,
        upstreamAccountName: "audit-ops@example.com",
        proxyDisplayName: "seoul-edge-03",
        tTotalMs: null,
      },
    },
    {
      promptCacheKey: "wc-readme-team-shared-replay",
      current: {
        id: 114,
        invokeId: "wc-r6-a",
        occurredAt: "2026-04-06T11:58:02.000Z",
        status: "pending",
        model: "gpt-5.4-mini",
        totalTokens: 1098,
        inputTokens: 662,
        outputTokens: 356,
        cacheInputTokens: 80,
        cost: 0.0437,
        upstreamAccountName: "team-shared@example.com",
        proxyDisplayName: "tokyo-edge-02",
        tTotalMs: null,
      },
      previous: {
        id: 115,
        invokeId: "wc-r6-b",
        occurredAt: "2026-04-06T11:56:34.000Z",
        status: "success",
        model: "gpt-5.4-mini",
        totalTokens: 1246,
        inputTokens: 742,
        outputTokens: 424,
        cacheInputTokens: 80,
        cost: 0.0496,
        upstreamAccountName: "team-shared@example.com",
        proxyDisplayName: "tokyo-edge-02",
      },
      extraHistory: [
        {
          id: 116,
          invokeId: "wc-r6-c",
          occurredAt: "2026-04-06T11:55:12.000Z",
          status: "success",
          model: "gpt-5.4-mini",
          totalTokens: 904,
          cost: 0.0361,
          upstreamAccountName: "team-shared@example.com",
          proxyDisplayName: "tokyo-edge-02",
        },
      ],
    },
    {
      promptCacheKey: "wc-readme-nightly-migration",
      current: {
        id: 117,
        invokeId: "wc-r7-a",
        occurredAt: "2026-04-06T11:57:46.000Z",
        status: "running",
        model: "gpt-5.4",
        totalTokens: 1964,
        inputTokens: 1010,
        outputTokens: 802,
        cacheInputTokens: 152,
        cost: 0.0886,
        upstreamAccountName: "nightly-migration@example.com",
        proxyDisplayName: "frankfurt-edge-01",
        tTotalMs: null,
      },
      previous: {
        id: 118,
        invokeId: "wc-r7-b",
        occurredAt: "2026-04-06T11:56:02.000Z",
        status: "success",
        model: "gpt-5.4-mini",
        totalTokens: 1388,
        inputTokens: 814,
        outputTokens: 462,
        cacheInputTokens: 112,
        cost: 0.0589,
        upstreamAccountName: "nightly-migration@example.com",
        proxyDisplayName: "frankfurt-edge-01",
      },
    },
    {
      promptCacheKey: "wc-readme-sandbox-evals",
      current: {
        id: 119,
        invokeId: "wc-r8-a",
        occurredAt: "2026-04-06T11:57:18.000Z",
        status: "success",
        model: "gpt-5.4-mini",
        totalTokens: 1182,
        inputTokens: 706,
        outputTokens: 384,
        cacheInputTokens: 92,
        cost: 0.0468,
        upstreamAccountName: "sandbox-evals@example.com",
        proxyDisplayName: "singapore-edge-03",
      },
      previous: {
        id: 120,
        invokeId: "wc-r8-b",
        occurredAt: "2026-04-06T11:55:51.000Z",
        status: "http_429",
        failureClass: "service_failure",
        failureKind: "burst_quota_exhausted",
        errorMessage: "window quota exhausted",
        model: "gpt-5.4-mini",
        totalTokens: 812,
        inputTokens: 540,
        outputTokens: 184,
        cacheInputTokens: 88,
        cost: 0.0322,
        upstreamAccountName: "sandbox-evals@example.com",
        proxyDisplayName: "singapore-edge-03",
      },
    },
    {
      promptCacheKey: "wc-readme-rag-rebuild",
      current: {
        id: 121,
        invokeId: "wc-r9-a",
        occurredAt: "2026-04-06T11:56:58.000Z",
        status: "pending",
        model: "gpt-5.4",
        totalTokens: 1626,
        inputTokens: 902,
        outputTokens: 596,
        cacheInputTokens: 128,
        cost: 0.0695,
        upstreamAccountName: "rag-rebuild@example.com",
        proxyDisplayName: "sydney-edge-02",
        tTotalMs: null,
      },
      previous: {
        id: 122,
        invokeId: "wc-r9-b",
        occurredAt: "2026-04-06T11:55:36.000Z",
        status: "success",
        model: "gpt-5.4",
        totalTokens: 1744,
        inputTokens: 988,
        outputTokens: 620,
        cacheInputTokens: 136,
        cost: 0.0742,
        upstreamAccountName: "rag-rebuild@example.com",
        proxyDisplayName: "sydney-edge-02",
      },
      extraHistory: [
        {
          id: 123,
          invokeId: "wc-r9-c",
          occurredAt: "2026-04-06T11:55:04.000Z",
          status: "success",
          model: "gpt-5.4-mini",
          totalTokens: 1012,
          cost: 0.0391,
          upstreamAccountName: "rag-rebuild@example.com",
          proxyDisplayName: "sydney-edge-02",
        },
      ],
    },
    {
      promptCacheKey: "wc-readme-proxy-regression",
      current: {
        id: 124,
        invokeId: "wc-r10-a",
        occurredAt: "2026-04-06T11:56:30.000Z",
        status: "http_502",
        failureClass: "service_failure",
        failureKind: "compact_first_chunk_timeout",
        errorMessage: "upstream closed during compact handshake",
        model: "gpt-5.4",
        totalTokens: 944,
        inputTokens: 584,
        outputTokens: 254,
        cacheInputTokens: 106,
        cost: 0.0394,
        upstreamAccountName: "proxy-ops@example.com",
        proxyDisplayName: "dallas-edge-01",
      },
      previous: {
        id: 125,
        invokeId: "wc-r10-b",
        occurredAt: "2026-04-06T11:55:18.000Z",
        status: "success",
        model: "gpt-5.4-mini",
        totalTokens: 1268,
        inputTokens: 768,
        outputTokens: 398,
        cacheInputTokens: 102,
        cost: 0.0516,
        upstreamAccountName: "proxy-ops@example.com",
        proxyDisplayName: "dallas-edge-01",
      },
    },
    {
      promptCacheKey: "wc-readme-oauth-refresh-batch",
      current: {
        id: 126,
        invokeId: "wc-r11-a",
        occurredAt: "2026-04-06T11:56:12.000Z",
        status: "running",
        model: "gpt-5.4-mini",
        totalTokens: 1364,
        inputTokens: 784,
        outputTokens: 478,
        cacheInputTokens: 102,
        cost: 0.0558,
        upstreamAccountName: "oauth-refresh@example.com",
        proxyDisplayName: "mumbai-edge-02",
        tTotalMs: null,
      },
      previous: {
        id: 127,
        invokeId: "wc-r11-b",
        occurredAt: "2026-04-06T11:55:00.000Z",
        status: "success",
        model: "gpt-5.4-mini",
        totalTokens: 1188,
        inputTokens: 692,
        outputTokens: 394,
        cacheInputTokens: 102,
        cost: 0.0472,
        upstreamAccountName: "oauth-refresh@example.com",
        proxyDisplayName: "mumbai-edge-02",
      },
      extraHistory: [
        {
          id: 128,
          invokeId: "wc-r11-c",
          occurredAt: "2026-04-06T11:54:18.000Z",
          status: "success",
          model: "gpt-5.4-mini",
          totalTokens: 964,
          cost: 0.0385,
          upstreamAccountName: "oauth-refresh@example.com",
          proxyDisplayName: "mumbai-edge-02",
        },
      ],
    },
    {
      promptCacheKey: "wc-readme-design-review-late",
      current: {
        id: 129,
        invokeId: "wc-r12-a",
        occurredAt: "2026-04-06T11:55:44.000Z",
        status: "success",
        model: "gpt-5.4",
        totalTokens: 1862,
        inputTokens: 980,
        outputTokens: 752,
        cacheInputTokens: 130,
        cost: 0.0846,
        upstreamAccountName: "design-review@example.com",
        proxyDisplayName: "london-edge-03",
      },
      previous: {
        id: 130,
        invokeId: "wc-r12-b",
        occurredAt: "2026-04-06T11:54:36.000Z",
        status: "pending",
        model: "gpt-5.4-mini",
        totalTokens: 1094,
        inputTokens: 642,
        outputTokens: 364,
        cacheInputTokens: 88,
        cost: 0.0433,
        upstreamAccountName: "design-review@example.com",
        proxyDisplayName: "london-edge-03",
        tTotalMs: null,
      },
    },
  ];
}

function buildReadmeDenseWorkingConversationsResponse(): PromptCacheConversationsResponse {
  const snapshotAt = new Date().toISOString();
  const snapshotAtMs = Date.parse(snapshotAt);
  const conversations = createReadmeDenseWorkingConversationSeeds().map((seed) =>
    createConversation(seed.promptCacheKey, [
      createPreview(withRelativeOccurredAt(seed.current, snapshotAtMs)),
      createPreview(withRelativeOccurredAt(seed.previous, snapshotAtMs)),
      ...(seed.extraHistory ?? []).map((invocation) =>
        createPreview(withRelativeOccurredAt(invocation, snapshotAtMs)),
      ),
    ]),
  );

  return {
    rangeStart: shiftWorkingConversationStoryIso("2026-04-06T11:55:00.000Z", snapshotAtMs),
    rangeEnd: snapshotAt,
    snapshotAt,
    selectionMode: "activityWindow",
    selectedLimit: null,
    selectedActivityHours: null,
    selectedActivityMinutes: 5,
    implicitFilter: { kind: null, filteredCount: 0 },
    totalMatched: conversations.length,
    hasMore: false,
    nextCursor: null,
    conversations,
  };
}

function buildWorkingConversationsResponse(empty = false): PromptCacheConversationsResponse {
  const snapshotAt = new Date().toISOString();
  const snapshotAtMs = Date.parse(snapshotAt);
  return {
    rangeStart: shiftWorkingConversationStoryIso("2026-04-06T11:55:00.000Z", snapshotAtMs),
    rangeEnd: snapshotAt,
    snapshotAt,
    selectionMode: "activityWindow",
    selectedLimit: null,
    selectedActivityHours: null,
    selectedActivityMinutes: 5,
    implicitFilter: { kind: null, filteredCount: 0 },
    totalMatched: empty ? 0 : 2,
    hasMore: false,
    nextCursor: null,
    conversations: empty
      ? []
      : [
          createConversation("wc-current-1", [
            createPreview(
              withRelativeOccurredAt(
                {
                  id: 1,
                  invokeId: "wc-1-a",
                  occurredAt: "2026-04-06T12:00:00.000Z",
                  status: "running",
                  upstreamAccountName: "pool-alpha@example.com",
                  tTotalMs: null,
                },
                snapshotAtMs,
              ),
            ),
            createPreview(
              withRelativeOccurredAt(
                {
                  id: 2,
                  invokeId: "wc-1-b",
                  occurredAt: "2026-04-06T11:57:20.000Z",
                  status: "success",
                  model: "gpt-5.4-mini",
                },
                snapshotAtMs,
              ),
            ),
          ]),
          createConversation("wc-current-2", [
            createPreview(
              withRelativeOccurredAt(
                {
                  id: 3,
                  invokeId: "wc-2-a",
                  occurredAt: "2026-04-06T11:59:10.000Z",
                  status: "http_502",
                  failureClass: "service_failure",
                  failureKind: "upstream_timeout",
                  errorMessage: "upstream gateway closed before first byte",
                  upstreamAccountName: "pool-beta@example.com",
                  requestedServiceTier: "auto",
                  serviceTier: "auto",
                },
                snapshotAtMs,
              ),
            ),
            createPreview(
              withRelativeOccurredAt(
                {
                  id: 4,
                  invokeId: "wc-2-b",
                  occurredAt: "2026-04-06T11:56:10.000Z",
                  status: "success",
                  upstreamAccountName: "pool-beta@example.com",
                },
                snapshotAtMs,
              ),
            ),
          ]),
        ],
  };
}

function createDashboardRequestHandler(scenario: DashboardScenario = "default") {
  const now = Date.parse("2026-04-09T12:24:00+08:00");
  const rangeYesterdayStart = Date.parse("2026-04-08T00:00:00+08:00");
  const rangeYesterdayEnd = Date.parse("2026-04-09T00:00:00+08:00");
  const rangeTodayStart = Date.parse("2026-04-09T00:00:00+08:00");
  const range1dStart = now - 24 * 60 * 60 * 1000;
  const range7dStart = now - 7 * 24 * 60 * 60 * 1000;
  const range6moStart = now - 180 * 24 * 60 * 60 * 1000;

  const todaySummary = buildSummary({
    totalCount: 12474,
    successCount: 9949,
    failureCount: 2525,
    totalCost: 539.42,
    totalTokens: 1314275579,
    inProgressConversationCount: 11,
  });
  const yesterdaySummary = buildSummary({
    totalCount: 10864,
    successCount: 9532,
    failureCount: 1332,
    totalCost: 418.76,
    totalTokens: 1092456123,
    inProgressConversationCount: 8,
  });

  const responses = {
    today: todaySummary,
    yesterday: yesterdaySummary,
    "1d": buildSummary({
      totalCount: 13564,
      successCount: 10948,
      failureCount: 2616,
      totalCost: 605.33,
      totalTokens: 1456067763,
    }),
    "7d": buildSummary({
      totalCount: 76421,
      successCount: 70115,
      failureCount: 6306,
      totalCost: 3128.74,
      totalTokens: 8764311220,
    }),
    timeseriesToday: buildTimeseriesResponse({
      rangeStart: new Date(rangeTodayStart).toISOString(),
      rangeEnd: new Date(now).toISOString(),
      bucketSeconds: 60,
      effectiveBucket: "1m",
      availableBuckets: ["1m"],
      points: buildTodayTimeseriesPoints({
        startMs: rangeTodayStart,
        endMs: now,
        summary: todaySummary,
      }),
    }),
    timeseriesYesterday: buildTimeseriesResponse({
      rangeStart: new Date(rangeYesterdayStart).toISOString(),
      rangeEnd: new Date(rangeYesterdayEnd).toISOString(),
      bucketSeconds: 60,
      effectiveBucket: "1m",
      availableBuckets: ["1m"],
      points: buildTodayTimeseriesPoints({
        startMs: rangeYesterdayStart,
        endMs: rangeYesterdayEnd - 60 * 1000,
        summary: yesterdaySummary,
      }),
    }),
    timeseries1d: buildTimeseriesResponse({
      rangeStart: new Date(range1dStart).toISOString(),
      rangeEnd: new Date(now).toISOString(),
      bucketSeconds: 60,
      effectiveBucket: "1m",
      availableBuckets: ["1m"],
      points: buildTimeseriesPoints({
        count: 24 * 60,
        bucketSeconds: 60,
        startMs: range1dStart,
      }),
    }),
    timeseries7d: buildTimeseriesResponse({
      rangeStart: new Date(range7dStart).toISOString(),
      rangeEnd: new Date(now).toISOString(),
      bucketSeconds: 3600,
      effectiveBucket: "1h",
      availableBuckets: ["1h"],
      points: buildTimeseriesPoints({
        count: 7 * 24,
        bucketSeconds: 3600,
        startMs: range7dStart,
        valueOffset: 7,
      }),
    }),
    timeseries6mo: buildTimeseriesResponse({
      rangeStart: new Date(range6moStart).toISOString(),
      rangeEnd: new Date(now).toISOString(),
      bucketSeconds: 86400,
      effectiveBucket: "1d",
      availableBuckets: ["1d"],
      points: buildTimeseriesPoints({
        count: 180,
        bucketSeconds: 86400,
        startMs: range6moStart,
        valueOffset: 11,
      }),
    }),
    parallelWorkToday: buildParallelWorkResponse({
      current: buildParallelWorkWindow([8, 10, 9], {
        rangeStart: "2026-04-09T00:00:00.000Z",
        bucketSeconds: 60,
      }),
      minute7d: buildParallelWorkWindow([6, 7, 8, 9], {
        rangeStart: "2026-04-03T00:00:00.000Z",
        bucketSeconds: 60,
      }),
      hour30d: buildParallelWorkWindow([5, 6, 7], {
        rangeStart: "2026-03-11T00:00:00.000Z",
        bucketSeconds: 3600,
      }),
      dayAll: buildParallelWorkWindow([7], {
        rangeStart: "2026-04-08T00:00:00.000Z",
        bucketSeconds: 86400,
      }),
    }),
    parallelWorkYesterday: buildParallelWorkResponse({
      current: buildParallelWorkWindow([7, 8, 9], {
        rangeStart: "2026-04-08T00:00:00.000Z",
        bucketSeconds: 60,
      }),
      minute7d: buildParallelWorkWindow([5, 6, 7, 8], {
        rangeStart: "2026-04-02T00:00:00.000Z",
        bucketSeconds: 60,
      }),
      hour30d: buildParallelWorkWindow([4, 5, 6], {
        rangeStart: "2026-03-10T00:00:00.000Z",
        bucketSeconds: 3600,
      }),
      dayAll: buildParallelWorkWindow([8], {
        rangeStart: "2026-04-07T00:00:00.000Z",
        bucketSeconds: 86400,
      }),
    }),
  };

  return ({ url }: { url: URL }) => {
    if (url.pathname === "/api/stats/summary") {
      const window = url.searchParams.get("window") ?? "today";
      if (scenario === "degraded" && window === "today") {
        return new Response("dashboard today summary unavailable", {
          status: 500,
        });
      }
      return jsonResponse(
        responses[window as keyof Pick<typeof responses, "today" | "yesterday" | "1d" | "7d">] ??
          responses.today,
      );
    }

    if (url.pathname === "/api/stats/dashboard-activity") {
      const range = url.searchParams.get("range") ?? "today";
      const includeAccounts = url.searchParams.get("includeAccounts") === "true";
      const summary =
        responses[range as keyof Pick<typeof responses, "today" | "yesterday" | "1d" | "7d">] ??
        responses.today;
      return jsonResponse(buildDashboardActivityResponse({ range, summary, includeAccounts }));
    }

    if (url.pathname === "/api/stats/dashboard-activity/recent") {
      const response = buildDashboardActivityResponse({
        range: "today",
        summary: responses.today,
        includeAccounts: true,
      });
      return jsonResponse({
        rangeStart: url.searchParams.get("rangeStart") ?? response.rangeStart,
        rangeEnd: url.searchParams.get("rangeEnd") ?? response.rangeEnd,
        snapshotId: Number(url.searchParams.get("snapshotId") ?? response.snapshotId),
        accounts: (response.accounts ?? []).map((account) => ({
          accountKey: account.accountKey,
          recentInvocations: account.recentInvocations,
        })),
      });
    }

    if (url.pathname === "/api/stats/timeseries") {
      const range = url.searchParams.get("range");
      if (range === "today") return jsonResponse(responses.timeseriesToday);
      if (range === "yesterday") return jsonResponse(responses.timeseriesYesterday);
      if (range === "1d") return jsonResponse(responses.timeseries1d);
      if (range === "7d") return jsonResponse(responses.timeseries7d);
      if (range === "6mo") return jsonResponse(responses.timeseries6mo);
    }

    if (url.pathname === "/api/stats/parallel-work") {
      const range = url.searchParams.get("range") ?? "today";
      if (range === "yesterday") return jsonResponse(responses.parallelWorkYesterday);
      return jsonResponse(responses.parallelWorkToday);
    }

    if (url.pathname === "/api/stats/prompt-cache-conversations") {
      if (scenario === "readmeDense") {
        return jsonResponse(buildReadmeDenseWorkingConversationsResponse());
      }
      return jsonResponse(buildWorkingConversationsResponse(scenario === "degraded"));
    }

    if (url.pathname === "/api/version") {
      return jsonResponse({ backend: "v0.2.0" });
    }

    return undefined;
  };
}

const meta = {
  title: "Pages/DashboardPage",
  component: DashboardPage,
  tags: ["autodocs"],
  parameters: {
    layout: "fullscreen",
    viewport: { defaultViewport: "desktop1660" },
    scenario: "default",
  },
  decorators: [
    (Story, context) => {
      const parameters = context.parameters as DashboardStoryParameters;
      const scenario = (parameters.scenario ?? "default") as DashboardScenario;
      return (
        <I18nProvider>
          <StorybookPageEnvironment onRequest={createDashboardRequestHandler(scenario)}>
            <MemoryRouter initialEntries={["/dashboard"]}>
              <FullPageStorySurface>
                <DashboardDiagnosticsStorageReset enabled={parameters.enableDiagnostics === true}>
                  <DashboardRangeStorageReset>
                    <Story />
                  </DashboardRangeStorageReset>
                </DashboardDiagnosticsStorageReset>
              </FullPageStorySurface>
            </MemoryRouter>
          </StorybookPageEnvironment>
        </I18nProvider>
      );
    },
  ],
} satisfies Meta<typeof DashboardPage>;

export default meta;

type Story = StoryObj<typeof meta>;

export const Default: Story = {
  render: () => <DashboardPage />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByTestId("dashboard-activity-overview")).toBeVisible();
    await expect(canvas.getByTestId("dashboard-working-conversations")).toBeVisible();
    await expect(canvas.getByTestId("dashboard-activity-range-today")).toHaveAttribute(
      "data-active",
      "true",
    );
    await expect(canvas.getByTestId("dashboard-today-activity-chart")).toBeVisible();
    await expect(canvas.queryByTestId("usage-calendar-card")).toBeNull();

    const historyTab = canvas.getByRole("tab", { name: "历史" });
    await userEvent.click(historyTab);
    await expect(historyTab).toHaveAttribute("aria-selected", "true");
    await expect(canvas.getByTestId("usage-calendar-card")).toBeVisible();

    const yesterdayTab = canvas.getByRole("tab", { name: "昨日" });
    await userEvent.click(yesterdayTab);
    await expect(yesterdayTab).toHaveAttribute("aria-selected", "true");
    await expect(canvas.getByTestId("dashboard-activity-range-yesterday")).toHaveAttribute(
      "data-active",
      "true",
    );

    const range7d = canvas.getByRole("tab", { name: "7 日" });
    await userEvent.click(range7d);
    await expect(range7d).toHaveAttribute("aria-selected", "true");

    const todayTab = canvas.getByRole("tab", { name: "今日" });
    await userEvent.click(todayTab);
    await expect(todayTab).toHaveAttribute("aria-selected", "true");
    await expect(canvas.getByTestId("dashboard-activity-range-today")).toHaveAttribute(
      "data-active",
      "true",
    );
  },
};

export const UnifiedActivitySnapshot: Story = {
  render: () => <DashboardPage />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByTestId("dashboard-activity-overview")).toBeVisible();
    await expect(canvas.getByTestId("today-stats-value-tpm")).toBeVisible();

    const accountTab = canvas.getByRole("tab", { name: "上游账号" });
    await userEvent.click(accountTab);
    await expect(accountTab).toHaveAttribute("aria-selected", "true");
    await waitFor(() => {
      expect(canvas.getAllByTestId("dashboard-upstream-account-card")).toHaveLength(2);
    });
    const accountHeaders = canvas.getAllByTestId("dashboard-upstream-account-header-row");
    await expect(accountHeaders[0]).toHaveTextContent("dzw");
    await expect(accountHeaders[1]).toHaveTextContent("CIII");
    await expect(accountHeaders[0]?.querySelector('[aria-label="进行中 8"]')).not.toBeNull();
    await expect(accountHeaders[1]?.querySelector('[aria-label="进行中 3"]')).not.toBeNull();
    await expect(accountHeaders[0]?.querySelector('[aria-label="TPM 610"]')).not.toBeNull();
    await expect(accountHeaders[1]?.querySelector('[aria-label="TPM 490"]')).not.toBeNull();
  },
};

export const Degraded: Story = {
  parameters: {
    scenario: "degraded",
  },
  render: () => <DashboardPage />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByTestId("dashboard-activity-overview")).toBeVisible();
    await expect(canvas.getByTestId("dashboard-working-conversations")).toBeVisible();
    await expect(canvas.getAllByRole("alert").at(0)).toBeVisible();
    await expect(canvas.queryAllByTestId("dashboard-working-conversation-card")).toHaveLength(0);
  },
};

export const LiveRefreshDiagnostics: Story = {
  parameters: {
    enableDiagnostics: true,
  },
  render: () => <DashboardPage />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByTestId("dashboard-performance-diagnostics")).toBeVisible();

    const initialSummaryRefreshCount = Number(
      canvas.getByTestId("dashboard-performance-diagnostics-today-summary-refresh-count")
        .textContent ?? "0",
    );
    const initialChartRenderCount = Number(
      canvas.getByTestId("dashboard-performance-diagnostics-today-chart-render-count")
        .textContent ?? "0",
    );
    const controller = getStorybookPageSseController();
    if (!controller) {
      throw new Error("storybook page SSE controller unavailable");
    }

    controller.emit({
      type: "records",
      records: [
        {
          id: 901,
          invokeId: "wc-1-live-after-snapshot",
          promptCacheKey: "wc-current-1",
          occurredAt: "2026-04-06T12:00:20.000Z",
          createdAt: "2026-04-06T12:00:20.000Z",
          status: "running",
          source: "pool",
          routeMode: "pool",
          model: "gpt-5.4",
          endpoint: "/v1/responses",
          totalTokens: 640,
          cost: 0.0284,
        },
      ],
    });
    controller.emit({
      type: "dashboardActivityLive",
      snapshot: {
        revision: 12,
        generatedAt: "2026-04-06T12:00:21.000Z",
        inProgressInvocationCount: 4,
        inProgressPhaseCounts: { queued: 0, requesting: 2, responding: 2 },
        retryInvocationCount: 1,
        accounts: [
          {
            accountKey: "upstream:42",
            upstreamAccountId: 42,
            inProgressInvocationCount: 2,
            inProgressPhaseCounts: { queued: 0, requesting: 1, responding: 1 },
            retryInvocationCount: 0,
          },
          {
            accountKey: "upstream:77",
            upstreamAccountId: 77,
            inProgressInvocationCount: 2,
            inProgressPhaseCounts: { queued: 0, requesting: 1, responding: 1 },
            retryInvocationCount: 1,
          },
        ],
      },
    });

    await waitFor(
      () => {
        expect(
          Number(
            canvas.getByTestId(
              "dashboard-performance-diagnostics-working-conversations-patch-bucket-count",
            ).textContent ?? "0",
          ),
        ).toBe(1);
        expect(
          Number(
            canvas.getByTestId(
              "dashboard-performance-diagnostics-working-conversations-patch-entry-count",
            ).textContent ?? "0",
          ),
        ).toBe(1);
        expect(
          Number(
            canvas.getByTestId("dashboard-performance-diagnostics-today-summary-refresh-count")
              .textContent ?? "0",
          ),
        ).toBeGreaterThan(initialSummaryRefreshCount);
      },
      { timeout: 4000 },
    );

    await expect(
      Number(
        canvas.getByTestId("dashboard-performance-diagnostics-today-chart-render-count")
          .textContent ?? "0",
      ),
    ).toBe(initialChartRenderCount);

    const accountTab = canvas.getByRole("tab", { name: "上游账号" });
    await userEvent.click(accountTab);
    await waitFor(() => {
      const headers = canvas.getAllByTestId("dashboard-upstream-account-header-row");
      expect(headers[1]?.querySelector('[aria-label="进行中 2"]')).not.toBeNull();
    });
  },
};

export const ReadmeDense: Story = {
  parameters: {
    scenario: "readmeDense",
  },
  render: () => <DashboardPage />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByTestId("dashboard-activity-overview")).toBeVisible();
    await expect(canvas.getByTestId("dashboard-working-conversations")).toBeVisible();
    await waitFor(
      async () => {
        expect(await canvas.findAllByTestId("dashboard-working-conversation-card")).toHaveLength(
          12,
        );
      },
      { timeout: 4000 },
    );
    await expect(canvas.getByText(/当前 12 条/i)).toBeVisible();
    await expect(canvas.getByText(/paisleeeinar5710 Team sandbox workflow monitor/i)).toBeVisible();
    await expect(canvas.getByText(/enterprise-review@team.example.com/i)).toBeVisible();
    await waitFor(() => {
      expect(
        canvasElement.querySelector(
          '[data-testid="invocation-endpoint-badge"][data-endpoint-kind="compact"]',
        ),
      ).not.toBeNull();
    });
  },
};

export const FullPageDesktop: Story = {
  render: () => <App />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByTestId("app-header-inner")).toBeVisible();
    await expect(canvas.getByTestId("dashboard-activity-overview")).toBeVisible();
    await expect(canvas.getByTestId("dashboard-working-conversations")).toBeVisible();
    await expect(canvas.getByTestId("app-footer-inner")).toBeVisible();
    await expect(
      canvas.getByTestId("today-stats-secondary-in-progress-day-average"),
    ).toHaveTextContent("9");
    await expect(canvas.getByTestId("today-stats-secondary-in-progress-delta")).toHaveTextContent(
      "+37.5%",
    );
    await expect(canvas.getByTestId("today-stats-value-response-time")).not.toHaveTextContent("—");
  },
};
