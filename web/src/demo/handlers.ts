import { HttpResponse, http, type JsonBodyType } from "msw";
import type {
  ApiInvocationWorkflowDetailResponse,
  LongTermMetrics,
  ModelRoutingTimelineRecord,
} from "../lib/api";
import { demoModel, demoNow } from "./model";
import {
  DEMO_MODEL_ROUTE_FIXTURES,
  DEMO_ROUTE_COMBINATIONS,
  type DemoModelRouteFixture,
} from "./model-routing-workload";

const DEMO_INVOCATION_REQUEST_BODY_SIZE = 8_681_416;
const DEMO_INVOCATION_REQUEST_BODY_TRANSMITTED_BYTES = 3_039_648;
const DEMO_INVOCATION_RESPONSE_BODY_SIZE = 138_649;
const demoResetModelRoutes = new Set<string>();
const DEMO_INVOCATION_RESPONSE_BODY_TEXT = JSON.stringify(
  {
    id: "resp_demo_9002",
    object: "response",
    status: "completed",
    model: "gpt-5.6-sol",
    service_tier: "default",
    output: [
      {
        type: "message",
        role: "assistant",
        content: [
          {
            type: "output_text",
            text: "Demo response body retained locally for visual inspection.",
          },
        ],
      },
    ],
    usage: {
      input_tokens: 9320,
      output_tokens: 882,
      total_tokens: 10202,
    },
  },
  null,
  2,
);

type DemoAccount = {
  id: number;
  kind: string;
  displayName: string;
  email: string | null;
  chatgptAccountId?: string | null;
  groupName: string | null;
  planType: string | null;
  enabled: boolean;
  displayStatus: string;
  enableStatus: string;
  workStatus: string;
  healthStatus: string;
  syncState: string;
  lastError?: string | null;
  boundProxyKeys?: string[];
  currentForwardProxyKey?: string | null;
  currentForwardProxyDisplayName?: string | null;
  lastSyncedAt?: string | null;
  primaryWindow?: { usedPercent: number } | null;
  secondaryWindow?: { usedPercent: number } | null;
  credits?: { balance?: string | null } | null;
  effectiveRoutingRule: Record<string, unknown>;
  [key: string]: unknown;
};

type DemoProxyNode = {
  key: string;
  source: string;
  displayName: string;
  endpointUrl?: string;
  weight: number;
  penalized: boolean;
  stats: Record<string, unknown>;
};

type DemoModelRoutingLiveQuery = {
  window?: string | null;
  model?: string | null;
  state?: string | null;
  limit?: string | null;
};

type DemoModelRoutingLiveState = ReturnType<typeof demoModelRoutingStates>[number] & {
  accountId: number;
  accountDisplayName: string;
};

function demoAccounts(): DemoAccount[] {
  return demoModel.snapshot.accounts as DemoAccount[];
}

function demoModelRouteTimestamp(minutesAgo: number) {
  return new Date(Date.parse(demoNow()) - minutesAgo * 60_000).toISOString();
}

function latestDemoRouteFixture(accountId: number, model: string) {
  return DEMO_MODEL_ROUTE_FIXTURES.filter(
    (fixture) => fixture.accountId === accountId && fixture.model === model,
  ).sort((left, right) => left.minutesAgo - right.minutesAgo)[0];
}

function demoModelRoutingStates(accountId: number) {
  return DEMO_ROUTE_COMBINATIONS.filter((route) => route.accountId === accountId)
    .map((route) => latestDemoRouteFixture(accountId, route.model))
    .filter((fixture): fixture is DemoModelRouteFixture => fixture != null)
    .map((fixture) => {
      const reset = demoResetModelRoutes.has(`${accountId}:${fixture.model}`);
      const state = reset ? "available" : fixture.flow === "recovered" ? "available" : fixture.flow;
      const coolingDown = state === "cooling_down";
      const degraded = state === "degraded";

      return {
        model: fixture.model,
        state,
        priority: coolingDown ? "excluded" : degraded ? "demoted" : "normal",
        failureCount: coolingDown ? 2 : degraded ? 1 : 0,
        changedAt: demoModelRouteTimestamp(fixture.minutesAgo),
        lastSeenAt: demoModelRouteTimestamp(fixture.minutesAgo),
        lastFailureAt: coolingDown || degraded ? demoModelRouteTimestamp(fixture.minutesAgo) : null,
        lastFailureKind: coolingDown || degraded ? "upstream_http_5xx" : null,
        lastFailureMessage: coolingDown || degraded ? "Demo request ended with HTTP 502." : null,
        cooldownUntil: coolingDown ? demoModelRouteTimestamp(fixture.minutesAgo - 60) : null,
        probeRequired: coolingDown,
      };
    });
}

function demoModelRouteFixtureTimeline(
  fixture: DemoModelRouteFixture,
): ModelRoutingTimelineRecord[] {
  const { accountId, model } = fixture;
  const accountDisplayName = `API Key #${accountId}`;
  const occurredAt = (offsetMinutes = 0) =>
    demoModelRouteTimestamp(fixture.minutesAgo + offsetMinutes);
  const invocation = invocations().find((record) => record.id === fixture.invocationId);
  const terminalLatencyMs = invocation?.tTotalMs ?? null;
  const retryLatencyMs =
    terminalLatencyMs == null ? null : Math.max(1, Math.floor(terminalLatencyMs * 0.75));
  const invokeId = `demo-invocation-${fixture.invocationId}`;
  const cooldownUntil = demoModelRouteTimestamp(fixture.minutesAgo - 60);
  const selectionAudit = {
    selectedAccountId: accountId,
    selectedAccountName: accountDisplayName,
    eligibleCandidateCount: DEMO_ROUTE_COMBINATIONS.filter((candidate) => candidate.model === model)
      .length,
    winnerReasonCode: "selected_eligible_route",
    excludedCandidates: [],
  };
  const firstFailure = {
    id: `attempt:${fixture.invocationId}:1`,
    kind: "attempt",
    occurredAt: occurredAt(4),
    accountId,
    accountDisplayName,
    model,
    attemptId: `demo-route-${fixture.invocationId}-1`,
    invokeId,
    attemptIndex: 1,
    sameAccountRetryIndex: 0,
    routingSource: "selection",
    status: "http_502",
    httpStatus: 502,
    totalLatencyMs: retryLatencyMs,
    failureKind: "upstream_http_5xx",
    reasonCode: "upstream_http_5xx",
    modelRouteStateBefore: "available",
    modelRouteStateAfter: "degraded",
    modelRouteFailureCount: 1,
    routingSelectionAudit: selectionAudit,
  };
  const cooldownEvent = {
    id: `event:${fixture.invocationId}:cooldown`,
    kind: "event",
    occurredAt: occurredAt(),
    accountId,
    accountDisplayName,
    model,
    invokeId,
    status: "cooling_down",
    action: "model_route_cooldown",
    source: "call",
    reasonCode: "upstream_http_5xx",
    modelRouteStateBefore: "degraded",
    modelRouteStateAfter: "cooling_down",
    modelRouteFailureCount: 2,
    modelRouteCooldownUntil: cooldownUntil,
  };

  if (fixture.flow === "available") {
    return [
      {
        id: `attempt:${fixture.invocationId}:1`,
        kind: "attempt",
        occurredAt: occurredAt(),
        accountId,
        accountDisplayName,
        model,
        attemptId: `demo-route-${fixture.invocationId}-1`,
        invokeId,
        attemptIndex: 1,
        sameAccountRetryIndex: 0,
        routingSource: "selection",
        status: "success",
        httpStatus: 200,
        totalLatencyMs: terminalLatencyMs,
        reasonCode: "selected_eligible_route",
        modelRouteStateBefore: "available",
        modelRouteStateAfter: "available",
        routingSelectionAudit: selectionAudit,
      },
    ];
  }

  if (fixture.flow === "recovered") {
    return [
      {
        id: `attempt:${fixture.invocationId}:2`,
        kind: "attempt",
        occurredAt: occurredAt(),
        accountId,
        accountDisplayName,
        model,
        attemptId: `demo-route-${fixture.invocationId}-2`,
        invokeId,
        attemptIndex: 2,
        sameAccountRetryIndex: 1,
        routingSource: "retry",
        status: "success",
        httpStatus: 200,
        totalLatencyMs: terminalLatencyMs,
        reasonCode: "model_route_recovery_succeeded",
        modelRouteStateBefore: "cooling_down",
        modelRouteStateAfter: "available",
        routingSelectionAudit: selectionAudit,
      },
      {
        ...cooldownEvent,
        occurredAt: occurredAt(1),
        modelRouteCooldownUntil: occurredAt(0.5),
      },
      firstFailure,
    ];
  }

  if (fixture.flow === "degraded") {
    return [
      {
        id: `event:${fixture.invocationId}:degraded`,
        kind: "event",
        occurredAt: occurredAt(),
        accountId,
        accountDisplayName,
        model,
        invokeId,
        status: "degraded",
        action: "model_route_degraded",
        source: "call",
        reasonCode: "upstream_http_5xx",
        modelRouteStateBefore: "available",
        modelRouteStateAfter: "degraded",
        modelRouteFailureCount: 1,
      },
      firstFailure,
    ];
  }

  return [
    {
      id: `attempt:${fixture.invocationId}:2`,
      kind: "attempt",
      occurredAt: occurredAt(0.1),
      accountId,
      accountDisplayName,
      model,
      attemptId: `demo-route-${fixture.invocationId}-2`,
      invokeId,
      attemptIndex: 2,
      sameAccountRetryIndex: 1,
      routingSource: "retry",
      status: "http_502",
      httpStatus: 502,
      totalLatencyMs: terminalLatencyMs,
      failureKind: "upstream_http_5xx",
      reasonCode: "upstream_http_5xx",
      modelRouteStateBefore: "degraded",
      modelRouteStateAfter: "cooling_down",
      modelRouteFailureCount: 2,
      modelRouteCooldownUntil: cooldownUntil,
      routingSelectionAudit: selectionAudit,
    },
    cooldownEvent,
    firstFailure,
  ];
}

function demoModelRoutingTimeline(accountId: number, model: string): ModelRoutingTimelineRecord[] {
  return DEMO_MODEL_ROUTE_FIXTURES.filter(
    (fixture) => fixture.accountId === accountId && fixture.model === model,
  )
    .flatMap(demoModelRouteFixtureTimeline)
    .sort((left, right) => Date.parse(right.occurredAt) - Date.parse(left.occurredAt));
}

function demoModelRoutingLiveTimeline(): ModelRoutingTimelineRecord[] {
  return DEMO_MODEL_ROUTE_FIXTURES.flatMap(demoModelRouteFixtureTimeline).sort(
    (left, right) => Date.parse(right.occurredAt) - Date.parse(left.occurredAt),
  );
}

function demoModelRoutingLive(query: DemoModelRoutingLiveQuery = {}) {
  const model = query.model?.trim() || null;
  const state = query.state?.trim() || null;
  const windowMinutes =
    query.window === "15m" ? 15 : query.window === "6h" ? 360 : query.window === "24h" ? 1_440 : 60;
  const parsedLimit = Number.parseInt(query.limit ?? "100", 10);
  const limit = Number.isFinite(parsedLimit) ? Math.min(100, Math.max(1, parsedLimit)) : 100;
  const cutoff = Date.parse(demoNow()) - windowMinutes * 60_000;
  const accounts = new Map(demoAccounts().map((account) => [account.id, account]));
  const groupsByModel = new Map<string, DemoModelRoutingLiveState[]>();

  for (const route of DEMO_ROUTE_COMBINATIONS) {
    if (model && route.model !== model) continue;
    const account = accounts.get(route.accountId);
    const routeState = demoModelRoutingStates(route.accountId).find(
      (candidate) => candidate.model === route.model,
    );
    if (!account || !routeState || (state && routeState.state !== state)) continue;
    const group = groupsByModel.get(route.model) ?? [];
    group.push({
      accountId: account.id,
      accountDisplayName: `API Key #${account.id}`,
      ...routeState,
    });
    groupsByModel.set(route.model, group);
  }
  const visibleRouteKeys = state
    ? new Set(
        Array.from(groupsByModel.values())
          .flat()
          .map((route) => `${route.accountId}:${route.model}`),
      )
    : null;

  return {
    generatedAt: demoNow(),
    groups: Array.from(groupsByModel, ([groupModel, groupAccounts]) => ({
      model: groupModel,
      accounts: groupAccounts,
    })),
    records: demoModelRoutingLiveTimeline()
      .filter(
        (record) =>
          (!model || record.model === model) &&
          Date.parse(record.occurredAt) >= cutoff &&
          (!visibleRouteKeys || visibleRouteKeys.has(`${record.accountId}:${record.model}`)),
      )
      .slice(0, limit),
  };
}

function demoForwardProxyNodes(): DemoProxyNode[] {
  return (demoModel.snapshot.settings.forwardProxy as { nodes: DemoProxyNode[] }).nodes;
}

function json(payload: unknown, init?: ResponseInit) {
  return HttpResponse.json(payload as JsonBodyType, init);
}

function apiPathname(pathname: string) {
  const apiIndex = pathname.indexOf("/api/");
  return apiIndex === -1 ? pathname : pathname.slice(apiIndex);
}

function formatDemoAttemptId(seed: number) {
  const compact = Math.abs(Math.trunc(seed)).toString(36).toUpperCase().slice(-8).padStart(8, "0");
  if (/[A-Z]/.test(compact)) return compact;
  return `${compact.slice(0, 4)}A${compact.slice(5)}`;
}

const DEMO_USAGE_BREAKDOWN = {
  cacheWriteTokens: 250_000_000,
  cacheReadTokens: 982_000_000,
  outputTokens: 149_240_000,
  costs: {
    input: 71.5,
    cacheWrite: 143,
    cacheRead: 46.5,
    output: 278,
    reasoning: 43.34,
    unknown: 0,
  },
  models: [
    {
      model: "gpt-5.6-sol",
      reasoningEffort: "high",
      cacheWriteTokens: 110_000_000,
      cacheReadTokens: 480_000_000,
      outputTokens: 65_000_000,
      costs: {
        input: 39.5,
        cacheWrite: 78,
        cacheRead: 24,
        output: 143,
        reasoning: 22.8,
        unknown: 0,
      },
    },
    {
      model: "gpt-5.6-sol",
      reasoningEffort: "medium",
      cacheWriteTokens: 80_000_000,
      cacheReadTokens: 350_000_000,
      outputTokens: 50_000_000,
      costs: {
        input: 22,
        cacheWrite: 45,
        cacheRead: 17.5,
        output: 95,
        reasoning: 12.3,
        unknown: 0,
      },
    },
    {
      model: "gpt-5.6-terra",
      reasoningEffort: null,
      cacheWriteTokens: 60_000_000,
      cacheReadTokens: 152_000_000,
      outputTokens: 34_240_000,
      costs: { input: 10, cacheWrite: 20, cacheRead: 5, output: 40, reasoning: 8.24, unknown: 0 },
    },
  ],
};

const DEMO_MODEL_PERFORMANCE_MODELS = [
  {
    model: "gpt-5.6-sol",
    reasoningEffort: "high",
    tokensPerMinute: 22_480,
    streamingResponseRate: 71.4,
    avgResponseMs: 3_280,
    avgFirstTokenMs: 820,
    wallClockUsageDurationMs: 6_540_000,
    cumulativeUsageDurationMs: 10_482_000,
  },
  {
    model: "gpt-5.6-sol",
    reasoningEffort: "medium",
    tokensPerMinute: 15_920,
    streamingResponseRate: 64.8,
    avgResponseMs: 2_680,
    avgFirstTokenMs: 694,
    wallClockUsageDurationMs: 4_872_000,
    cumulativeUsageDurationMs: 7_246_000,
  },
  {
    model: "gpt-5.6-terra",
    reasoningEffort: null,
    tokensPerMinute: 7_641,
    streamingResponseRate: 46.2,
    avgResponseMs: 4_910,
    avgFirstTokenMs: 1_104,
    wallClockUsageDurationMs: 3_120_000,
    cumulativeUsageDurationMs: 4_038_000,
  },
] as const;

const DEMO_MODEL_PERFORMANCE_PAIR_OVERLAPS_MS = [
  [0, 1, 2_724_000],
  [0, 2, 420_000],
] as const;

function computeDemoParallelism(
  wallClockUsageDurationMs: number | null | undefined,
  cumulativeUsageDurationMs: number | null | undefined,
) {
  if (
    wallClockUsageDurationMs == null ||
    cumulativeUsageDurationMs == null ||
    !Number.isFinite(wallClockUsageDurationMs) ||
    !Number.isFinite(cumulativeUsageDurationMs) ||
    wallClockUsageDurationMs <= 0
  ) {
    return null;
  }
  return cumulativeUsageDurationMs / wallClockUsageDurationMs;
}

function demoModelPerformanceForModels(modelIndexes: number[]) {
  const baseModels =
    demoModel.snapshot.scene === "empty"
      ? []
      : modelIndexes
          .map((index) => DEMO_MODEL_PERFORMANCE_MODELS[index])
          .filter((model) => model != null);
  if (baseModels.length === 0) {
    return {
      available: true,
      total: {
        tokensPerMinute: 0,
        streamingResponseRate: null,
        avgResponseMs: null,
        avgFirstTokenMs: null,
        wallClockUsageDurationMs: null,
        cumulativeUsageDurationMs: null,
        parallelism: null,
      },
      models: [],
    };
  }
  const includedIndexes = [...new Set(modelIndexes)].sort((left, right) => left - right);
  const overlapMs = DEMO_MODEL_PERFORMANCE_PAIR_OVERLAPS_MS.reduce(
    (total, [left, right, pairOverlapMs]) =>
      includedIndexes.includes(left) && includedIndexes.includes(right)
        ? total + pairOverlapMs
        : total,
    0,
  );
  const models = baseModels.map((model) => ({
    ...model,
    parallelism: computeDemoParallelism(
      model.wallClockUsageDurationMs,
      model.cumulativeUsageDurationMs,
    ),
  }));
  const cumulativeUsageDurationMs = models.reduce(
    (total, model) => total + model.cumulativeUsageDurationMs,
    0,
  );
  const wallClockUsageDurationMs = Math.max(
    0,
    models.reduce((total, model) => total + model.wallClockUsageDurationMs, 0) - overlapMs,
  );
  return {
    available: true,
    total: {
      tokensPerMinute: models.reduce((total, model) => total + model.tokensPerMinute, 0),
      streamingResponseRate:
        models.reduce(
          (total, model) => total + model.streamingResponseRate * model.cumulativeUsageDurationMs,
          0,
        ) / cumulativeUsageDurationMs,
      avgResponseMs:
        models.reduce(
          (total, model) => total + model.avgResponseMs * model.cumulativeUsageDurationMs,
          0,
        ) / cumulativeUsageDurationMs,
      avgFirstTokenMs:
        models.reduce(
          (total, model) => total + model.avgFirstTokenMs * model.cumulativeUsageDurationMs,
          0,
        ) / cumulativeUsageDurationMs,
      wallClockUsageDurationMs,
      cumulativeUsageDurationMs,
      parallelism: computeDemoParallelism(wallClockUsageDurationMs, cumulativeUsageDurationMs),
    },
    models,
  };
}

function demoUsageBreakdownForModels(modelIndexes: number[]) {
  const models = modelIndexes
    .map((index) => DEMO_USAGE_BREAKDOWN.models[index])
    .filter((model) => model != null);
  const costs = models.reduce(
    (totals, model) => ({
      input: totals.input + model.costs.input,
      cacheWrite: totals.cacheWrite + model.costs.cacheWrite,
      cacheRead: totals.cacheRead + model.costs.cacheRead,
      output: totals.output + model.costs.output,
      reasoning: totals.reasoning + model.costs.reasoning,
      unknown: totals.unknown + model.costs.unknown,
    }),
    { input: 0, cacheWrite: 0, cacheRead: 0, output: 0, reasoning: 0, unknown: 0 },
  );
  return {
    cacheWriteTokens: models.reduce((total, model) => total + model.cacheWriteTokens, 0),
    cacheReadTokens: models.reduce((total, model) => total + model.cacheReadTokens, 0),
    outputTokens: models.reduce((total, model) => total + model.outputTokens, 0),
    costs,
    models,
  };
}

function demoAverage(values: Array<number | null | undefined>) {
  const defined = values.filter((value): value is number => typeof value === "number");
  if (defined.length === 0) return null;
  return Number((defined.reduce((total, value) => total + value, 0) / defined.length).toFixed(2));
}

function demoPercentile(values: Array<number | null | undefined>, percentile: number) {
  const defined = values
    .filter((value): value is number => typeof value === "number")
    .sort((left, right) => left - right);
  if (defined.length === 0) return null;
  return defined[Math.min(defined.length - 1, Math.ceil(defined.length * percentile) - 1)] ?? null;
}

function demoInvocationSummary(records: ReturnType<typeof invocations>) {
  const totalCount = records.length;
  const successCount = records.filter((record) => record.status === "success").length;
  const failureRecords = records.filter((record) => record.failureClass !== "none");
  const totalTokens = records.reduce((total, record) => total + (record.totalTokens ?? 0), 0);
  const totalCost = Number(
    records.reduce((total, record) => total + (record.cost ?? 0), 0).toFixed(4),
  );
  const cacheWriteTokens = records.reduce(
    (total, record) => total + (record.cacheWriteTokens ?? 0),
    0,
  );
  const cacheInputTokens = records.reduce(
    (total, record) => total + (record.cacheInputTokens ?? 0),
    0,
  );
  const outputTokens = records.reduce((total, record) => total + (record.outputTokens ?? 0), 0);
  const totalDurations = records.map((record) => record.tTotalMs);
  const firstByteDurations = records.map((record) => record.tUpstreamTtfbMs);
  const rangeStart = records.reduce<string | null>(
    (earliest, record) =>
      earliest == null || record.occurredAt < earliest ? record.occurredAt : earliest,
    null,
  );
  const rangeEnd = records.reduce<string | null>(
    (latest, record) => (latest == null || record.occurredAt > latest ? record.occurredAt : latest),
    null,
  );

  return {
    rangeStart: rangeStart ?? demoNow(),
    rangeEnd: rangeEnd ?? demoNow(),
    totalCount,
    successCount,
    failureCount: failureRecords.length,
    totalCost,
    totalTokens,
    usageBreakdown: null,
    inProgressConversationCount: records.filter((record) => record.status === "running").length,
    token: {
      requestCount: totalCount,
      totalTokens,
      avgTokensPerRequest: totalCount === 0 ? 0 : totalTokens / totalCount,
      cacheWriteTokens,
      cacheInputTokens,
      outputTokens,
      totalCost,
      maxTokensPerRequest:
        totalCount === 0 ? null : Math.max(...records.map((record) => record.totalTokens ?? 0)),
    },
    network: {
      avgTtfbMs: demoAverage(firstByteDurations),
      p95TtfbMs: demoPercentile(firstByteDurations, 0.95),
      avgFirstTokenMs: null,
      p95FirstTokenMs: null,
      avgResponseDurationMs: demoAverage(totalDurations),
      p95ResponseDurationMs: demoPercentile(totalDurations, 0.95),
      avgTotalMs: demoAverage(totalDurations),
      p95TotalMs: demoPercentile(totalDurations, 0.95),
      maxTotalMs:
        totalCount === 0 ? null : Math.max(...records.map((record) => record.tTotalMs ?? 0)),
    },
    exception: {
      failureCount: failureRecords.length,
      serviceFailureCount: failureRecords.filter(
        (record) => record.failureClass === "service_failure",
      ).length,
      clientFailureCount: failureRecords.filter(
        (record) => record.failureClass === "client_failure",
      ).length,
      clientAbortCount: failureRecords.filter((record) => record.failureClass === "client_abort")
        .length,
      actionableFailureCount: failureRecords.filter((record) => record.isActionable).length,
    },
  };
}

function demoUsageBreakdownFromInvocations(records: ReturnType<typeof invocations>) {
  const grouped = new Map<
    string,
    {
      model: string;
      reasoningEffort: string | null;
      cacheWriteTokens: number;
      cacheReadTokens: number;
      outputTokens: number;
      costs: {
        input: number;
        cacheWrite: number;
        cacheRead: number;
        output: number;
        reasoning: number;
        unknown: number;
      };
    }
  >();
  for (const record of records) {
    const reasoningEffort = record.reasoningEffort ?? null;
    const key = `${record.model}:${reasoningEffort ?? "none"}`;
    const entry = grouped.get(key) ?? {
      model: record.model,
      reasoningEffort,
      cacheWriteTokens: 0,
      cacheReadTokens: 0,
      outputTokens: 0,
      costs: { input: 0, cacheWrite: 0, cacheRead: 0, output: 0, reasoning: 0, unknown: 0 },
    };
    entry.cacheWriteTokens += record.cacheWriteTokens ?? 0;
    entry.cacheReadTokens += record.cacheInputTokens ?? 0;
    entry.outputTokens += record.outputTokens ?? 0;
    entry.costs.input += record.costInput ?? 0;
    entry.costs.cacheWrite += record.costCacheWrite ?? 0;
    entry.costs.cacheRead += record.costCacheRead ?? 0;
    entry.costs.output += record.costOutput ?? 0;
    entry.costs.reasoning += record.costReasoning ?? 0;
    grouped.set(key, entry);
  }
  const models = Array.from(grouped.values())
    .sort((left, right) => left.model.localeCompare(right.model))
    .map((entry) => ({
      ...entry,
      costs: Object.fromEntries(
        Object.entries(entry.costs).map(([key, value]) => [key, Number(value.toFixed(4))]),
      ) as typeof entry.costs,
    }));
  return {
    cacheWriteTokens: models.reduce((total, model) => total + model.cacheWriteTokens, 0),
    cacheReadTokens: models.reduce((total, model) => total + model.cacheReadTokens, 0),
    outputTokens: models.reduce((total, model) => total + model.outputTokens, 0),
    costs: models.reduce(
      (total, model) => ({
        input: Number((total.input + model.costs.input).toFixed(4)),
        cacheWrite: Number((total.cacheWrite + model.costs.cacheWrite).toFixed(4)),
        cacheRead: Number((total.cacheRead + model.costs.cacheRead).toFixed(4)),
        output: Number((total.output + model.costs.output).toFixed(4)),
        reasoning: Number((total.reasoning + model.costs.reasoning).toFixed(4)),
        unknown: 0,
      }),
      { input: 0, cacheWrite: 0, cacheRead: 0, output: 0, reasoning: 0, unknown: 0 },
    ),
    models,
  };
}

export function demoSummary() {
  const records = demoModel.snapshot.scene === "empty" ? [] : invocations();
  const summary = demoInvocationSummary(records);
  const usageBreakdown = demoUsageBreakdownFromInvocations(records);
  return {
    ...summary,
    usageBreakdown,
    token: {
      ...summary.token,
      cacheInputTokens: usageBreakdown.cacheReadTokens,
    },
  };
}

function invocations() {
  if (demoModel.snapshot.scene === "empty") return [];
  const attention = demoModel.snapshot.scene === "attention";
  const accounts = new Map(demoAccounts().map((account) => [account.id, account]));
  const proxyName = (key: string) =>
    key === "demo-tokyo"
      ? "Tokyo demo relay"
      : key === "demo-frankfurt"
        ? "Frankfurt recovery relay"
        : key === "demo-sydney"
          ? "Sydney analytics relay"
          : key === "demo-virginia"
            ? "Virginia batch relay"
            : "Singapore warm standby";
  const routingRows = DEMO_MODEL_ROUTE_FIXTURES.map((fixture, index) => {
    const proxyKey =
      fixture.accountId === 108
        ? "demo-tokyo"
        : fixture.accountId === 112
          ? "demo-virginia"
          : fixture.accountId === 115
            ? "demo-singapore"
            : "demo-frankfurt";
    const modelBaseInput =
      fixture.model === "gpt-5.5" ? 6_400 : fixture.model === "gpt-5.4-mini" ? 4_100 : 5_250;
    const inputTokens = modelBaseInput + (index % 7) * 280;
    const outputTokens = 280 + (index % 5) * 95;
    const cacheInputTokens = Math.round(inputTokens * (0.58 + (index % 4) * 0.06));
    const cacheWriteTokens = Math.max(0, inputTokens - cacheInputTokens);
    const ttfb = 150 + (index % 6) * 23 + (fixture.terminalStatus === "http_502" ? 110 : 0);
    const total = ttfb + 620 + (index % 5) * 115;
    const cost = Number(
      (
        (inputTokens * 0.0000009 + outputTokens * 0.0000042) *
        (fixture.model === "gpt-5.5" ? 1.2 : 1)
      ).toFixed(4),
    );
    return [
      fixture.invocationId,
      fixture.accountId,
      "routing-workload",
      proxyKey,
      fixture.model === "gpt-5.4-mini" ? "/v1/chat/completions" : "/v1/responses",
      fixture.model,
      fixture.terminalStatus,
      inputTokens,
      outputTokens,
      cacheInputTokens,
      cacheWriteTokens,
      cost,
      ttfb,
      total,
      `routing-session-${(index % 12) + 1}`,
    ] as const;
  });
  const rows = [
    [
      9001,
      101,
      "09:30",
      "demo-tokyo",
      "/v1/responses",
      "gpt-5.6-sol",
      "running",
      12520,
      0,
      10880,
      1640,
      0.014,
      null,
      null,
      "demo-conversation-a",
    ],
    [
      9002,
      101,
      "09:23",
      "demo-tokyo",
      "/v1/responses",
      "gpt-5.6-sol",
      attention ? "http_502" : "success",
      9320,
      882,
      7311,
      2009,
      0.0092,
      184,
      1882,
      "demo-conversation-a",
    ],
    [
      9003,
      102,
      "09:17",
      "demo-frankfurt",
      "/v1/chat/completions",
      "gpt-5.6-terra",
      "success",
      4110,
      295,
      2980,
      1130,
      0.0037,
      146,
      1095,
      "demo-conversation-b",
    ],
    [
      9004,
      103,
      "09:11",
      "demo-tokyo",
      "/v1/responses",
      "gpt-5.6-sol",
      "success",
      15680,
      1210,
      13240,
      2440,
      0.0156,
      202,
      2401,
      "demo-conversation-a",
    ],
    [
      9005,
      104,
      "09:06",
      "demo-singapore",
      "/v1/images/generations",
      "gpt-5.4-mini",
      "success",
      2880,
      512,
      610,
      2270,
      0.0068,
      244,
      3850,
      "demo-image-workflow",
    ],
    [
      9006,
      105,
      "09:02",
      "demo-tokyo",
      "/v1/responses",
      "gpt-5.6-terra",
      "success",
      7890,
      463,
      5210,
      2680,
      0.0062,
      172,
      1492,
      "demo-research-batch",
    ],
    [
      9007,
      106,
      "08:58",
      "demo-frankfurt",
      "/v1/chat/completions",
      "gpt-5.6-terra",
      "success",
      3590,
      216,
      2440,
      1150,
      0.0029,
      268,
      1779,
      "demo-research-batch",
    ],
    [
      9008,
      107,
      "08:53",
      "demo-singapore",
      "/v1/responses",
      "gpt-5.6-sol",
      "success",
      6120,
      638,
      4590,
      1530,
      0.0077,
      229,
      2264,
      "demo-conversation-c",
    ],
    [
      9009,
      108,
      "08:49",
      "demo-tokyo",
      "/v1/embeddings",
      "text-embedding-3-large",
      "success",
      42350,
      0,
      39800,
      2550,
      0.0041,
      92,
      508,
      "demo-indexing",
    ],
    [
      9010,
      109,
      "08:44",
      "demo-singapore",
      "/v1/responses",
      "gpt-5.6-sol",
      attention ? "http_401" : "success",
      10380,
      742,
      8220,
      2160,
      0.0114,
      338,
      3028,
      "demo-image-workflow",
    ],
    [
      9011,
      110,
      "08:40",
      "demo-frankfurt",
      "/v1/chat/completions",
      "gpt-5.4-mini",
      "success",
      1870,
      126,
      1200,
      670,
      0.0012,
      179,
      884,
      "demo-sandbox",
    ],
    [
      9012,
      101,
      "08:34",
      "demo-tokyo",
      "/v1/responses",
      "gpt-5.6-sol",
      "success",
      19240,
      1638,
      17120,
      2120,
      0.0218,
      216,
      3211,
      "demo-conversation-a",
    ],
    [
      9013,
      103,
      "08:30",
      "demo-tokyo",
      "/v1/responses",
      "gpt-5.6-terra",
      "success",
      8640,
      391,
      6550,
      2090,
      0.007,
      157,
      1314,
      "demo-conversation-d",
    ],
    [
      9014,
      105,
      "08:26",
      "demo-tokyo",
      "/v1/responses",
      "gpt-5.6-sol",
      "http_429",
      5520,
      0,
      4900,
      620,
      0.0048,
      111,
      644,
      "demo-research-batch",
    ],
    [
      9015,
      102,
      "08:20",
      "demo-frankfurt",
      "/v1/chat/completions",
      "gpt-5.6-terra",
      "success",
      2910,
      184,
      1780,
      1130,
      0.0025,
      276,
      1682,
      "demo-conversation-b",
    ],
    [
      9016,
      104,
      "08:14",
      "demo-singapore",
      "/v1/images/generations",
      "gpt-5.4-mini",
      "success",
      2100,
      342,
      420,
      1680,
      0.0051,
      241,
      3427,
      "demo-image-workflow",
    ],
    [
      9017,
      106,
      "08:08",
      "demo-frankfurt",
      "/v1/chat/completions",
      "gpt-5.6-terra",
      "client_cancelled",
      6230,
      0,
      5400,
      830,
      0.0046,
      121,
      459,
      "demo-research-batch",
    ],
    [
      9018,
      108,
      "08:02",
      "demo-tokyo",
      "/v1/embeddings",
      "text-embedding-3-large",
      "success",
      38000,
      0,
      36000,
      2000,
      0.0038,
      87,
      467,
      "demo-indexing",
    ],
    [
      9019,
      111,
      "07:58",
      "demo-sydney",
      "/v1/responses",
      "gpt-5.6-sol",
      "success",
      7340,
      521,
      5880,
      1460,
      0.0081,
      312,
      2814,
      "demo-edge-monitor",
    ],
    [
      9020,
      112,
      "07:53",
      "demo-virginia",
      "/v1/chat/completions",
      "gpt-5.6-terra",
      "success",
      4960,
      380,
      3320,
      1640,
      0.0049,
      164,
      1204,
      "demo-batch-west",
    ],
    [
      9021,
      113,
      "07:49",
      "demo-virginia",
      "/v1/responses",
      "gpt-5.6-sol",
      "success",
      11240,
      1044,
      9300,
      1940,
      0.0132,
      188,
      1940,
      "demo-research-batch",
    ],
    [
      9022,
      114,
      "07:44",
      "demo-sydney",
      "/v1/responses",
      "gpt-5.6-sol",
      "success",
      17440,
      1324,
      15120,
      2320,
      0.0198,
      275,
      2698,
      "demo-conversation-d",
    ],
    [
      9023,
      115,
      "07:40",
      "demo-singapore",
      "/v1/chat/completions",
      "gpt-5.4-mini",
      "success",
      2460,
      198,
      1490,
      970,
      0.0019,
      233,
      1072,
      "demo-recovery",
    ],
    [
      9024,
      111,
      "07:36",
      "demo-sydney",
      "/v1/responses",
      "gpt-5.6-terra",
      "success",
      6840,
      474,
      5140,
      1700,
      0.0065,
      319,
      2488,
      "demo-edge-monitor",
    ],
    [
      9025,
      112,
      "07:31",
      "demo-virginia",
      "/v1/embeddings",
      "text-embedding-3-large",
      "success",
      55600,
      0,
      53300,
      2300,
      0.0053,
      102,
      556,
      "demo-batch-west",
    ],
    [
      9026,
      113,
      "07:27",
      "demo-virginia",
      "/v1/chat/completions",
      "gpt-5.6-terra",
      "success",
      3720,
      304,
      2490,
      1230,
      0.0031,
      157,
      1356,
      "demo-research-batch",
    ],
    [
      9027,
      114,
      "07:22",
      "demo-sydney",
      "/v1/responses",
      "gpt-5.6-sol",
      "success",
      13680,
      946,
      11720,
      1960,
      0.0157,
      289,
      3120,
      "demo-mobile-e2e",
    ],
    [
      9028,
      115,
      "07:18",
      "demo-singapore",
      "/v1/chat/completions",
      "gpt-5.4-mini",
      "success",
      3210,
      253,
      2070,
      1140,
      0.0027,
      241,
      1298,
      "demo-recovery",
    ],
    [
      9029,
      110,
      "07:13",
      "demo-frankfurt",
      "/v1/responses",
      "gpt-5.6-terra",
      "success",
      4430,
      362,
      3100,
      1330,
      0.0043,
      271,
      1888,
      "demo-sandbox",
    ],
    [
      9030,
      109,
      "07:09",
      "demo-singapore",
      "/v1/images/generations",
      "gpt-5.4-mini",
      "success",
      2640,
      421,
      740,
      1900,
      0.0061,
      248,
      3669,
      "demo-image-workflow",
    ],
    ...routingRows,
  ] as const;

  return rows
    .map(
      ([
        id,
        rowAccountId,
        ,
        proxyKey,
        endpoint,
        rowModel,
        rowStatus,
        inputTokens,
        outputTokens,
        cacheInputTokens,
        cacheWriteTokens,
        cost,
        ttfb,
        total,
        promptCacheKey,
      ]) => {
        const routingRequest = DEMO_MODEL_ROUTE_FIXTURES.find(
          (fixture) => fixture.invocationId === id,
        );
        const accountId = routingRequest?.accountId ?? rowAccountId;
        const model = routingRequest?.model ?? rowModel;
        const status = routingRequest?.terminalStatus ?? rowStatus;
        const account = accounts.get(accountId);
        const isFailure =
          status === "http_502" ||
          status === "http_401" ||
          status === "http_429" ||
          status === "client_cancelled";
        const failureClass =
          status === "client_cancelled" ? "client_abort" : isFailure ? "service_failure" : "none";
        const failureKind =
          status === "http_429"
            ? "rate_limited"
            : status === "client_cancelled"
              ? "downstream_cancelled"
              : status === "http_401"
                ? "upstream_auth_rejected"
                : status === "http_502"
                  ? "upstream_timeout"
                  : null;
        const occurredAt = routingRequest
          ? demoModelRouteTimestamp(routingRequest.minutesAgo)
          : new Date(Date.parse(demoNow()) - (id - 9001) * 8_000).toISOString();
        return {
          id,
          invokeId: `demo-invocation-${id}`,
          occurredAt,
          createdAt: occurredAt,
          source: "proxy",
          proxyDisplayName: proxyName(proxyKey),
          upstreamAccountId: accountId,
          upstreamAccountName: account?.displayName ?? null,
          upstreamAccountPlanType: account?.planType ?? null,
          endpoint,
          model,
          requestModel: model,
          responseModel: status === "success" ? model : null,
          status,
          livePhase: status === "running" ? "responding" : null,
          requestedServiceTier: accountId === 101 ? "priority" : "auto",
          serviceTier: accountId === 101 ? "priority" : "auto",
          billingServiceTier: accountId === 101 ? "priority" : "standard",
          inputTokens,
          outputTokens,
          cacheInputTokens,
          cacheWriteTokens,
          reasoningTokens: model === "gpt-5.6-sol" ? Math.round(inputTokens * 0.05) : 0,
          reasoningEffort: model === "gpt-5.6-sol" ? (id % 2 === 0 ? "medium" : "high") : null,
          totalTokens: inputTokens + outputTokens,
          cost,
          costInput: Number((cost * 0.31).toFixed(4)),
          costCacheWrite: Number((cost * 0.19).toFixed(4)),
          costCacheRead: Number((cost * 0.08).toFixed(4)),
          costOutput: Number((cost * 0.34).toFixed(4)),
          costReasoning: Number((cost * 0.08).toFixed(4)),
          failureClass,
          failureKind,
          isActionable: isFailure && status !== "client_cancelled",
          errorMessage:
            failureKind === "upstream_timeout"
              ? "Simulated upstream timeout after 1.8 seconds."
              : failureKind === "upstream_auth_rejected"
                ? "Simulated upstream authorization rejection."
                : failureKind === "rate_limited"
                  ? "Simulated upstream rate limit."
                  : failureKind === "downstream_cancelled"
                    ? "Simulated client cancellation."
                    : null,
          downstreamStatusCode:
            status === "http_502"
              ? 502
              : status === "http_401"
                ? 401
                : status === "http_429"
                  ? 429
                  : null,
          requesterIp: id % 2 === 0 ? "203.0.113.24" : "198.51.100.86",
          promptCacheKey,
          stickyKey: promptCacheKey,
          routeMode: account?.groupName === "standby" ? "fallback" : "pool",
          poolAttemptCount: status === "http_429" ? 2 : status === "http_502" ? 3 : 1,
          poolDistinctAccountCount: status === "http_502" ? 2 : 1,
          poolAttemptTerminalReason: isFailure ? failureKind : "completed",
          transport: status === "running" ? "websocket" : "http",
          tUpstreamConnectMs: ttfb == null ? null : Math.max(24, Math.round(ttfb * 0.24)),
          tUpstreamTtfbMs: ttfb,
          firstTokenMs:
            ttfb == null ||
            total == null ||
            !["/v1/responses", "/v1/chat/completions"].includes(endpoint)
              ? null
              : Math.min(total, ttfb + 420 + (id % 5) * 75),
          tUpstreamStreamMs: total == null ? null : Math.max(0, total - (ttfb ?? 0)),
          tTotalMs: total,
          timings:
            total == null
              ? undefined
              : {
                  upstreamConnectMs: Math.max(24, Math.round((ttfb ?? 120) * 0.24)),
                  upstreamFirstByteMs: ttfb,
                  upstreamStreamMs: Math.max(0, total - (ttfb ?? 0)),
                  totalMs: total,
                },
          rawMetadata: {
            request: {
              demo: true,
              routeMode: account?.groupName === "standby" ? "fallback" : "pool",
            },
            response: { model, requestId: `req_demo_${id}` },
          },
        };
      },
    )
    .sort((left, right) => Date.parse(right.occurredAt) - Date.parse(left.occurredAt));
}

function demoDashboardActivityAccounts() {
  if (demoModel.snapshot.scene === "empty") return [];
  const attention = demoModel.snapshot.scene === "attention";
  const recent = invocations();
  return demoAccounts()
    .slice(0, 12)
    .map((account, index) => {
      const accountRecords = recent.filter((record) => record.upstreamAccountId === account.id);
      const failureCount = accountRecords.filter(
        (record) => record.failureClass && record.failureClass !== "none",
      ).length;
      const isLastAccount = index === 11;
      const requestCount = 1_620 - index * 100 + (isLastAccount ? 6 : 0);
      const aggregateFailureCount =
        failureCount * 39 + (attention && account.id === 102 ? 410 : 18);
      const totalTokens = 180_000_000 - index * 12_000_000 + (isLastAccount ? 13_240_000 : 0);
      const totalCost = 75 - index * 5 + (isLastAccount ? 12.34 : 0);
      const modelIndexes = account.planType === "api" ? [2] : index % 2 === 0 ? [0, 1] : [1];
      return {
        accountKey: `upstream:${account.id}`,
        upstreamAccountId: account.id,
        displayName: account.displayName,
        groupName: account.groupName,
        planType: account.planType,
        enabled: account.enabled,
        displayStatus: account.displayStatus,
        enableStatus: account.enableStatus,
        workStatus: account.workStatus,
        healthStatus: account.healthStatus,
        syncState: account.syncState,
        lastError: account.lastError,
        requestCount,
        successCount: requestCount - aggregateFailureCount,
        failureCount: aggregateFailureCount,
        nonSuccessCount: aggregateFailureCount,
        totalTokens,
        successTokens: Math.round(totalTokens * 0.976),
        nonSuccessTokens: Math.round(totalTokens * 0.024),
        failureTokens: Math.round(totalTokens * 0.024),
        failureCost: Number((totalCost * 0.032).toFixed(2)),
        totalCost,
        usageBreakdown: demoUsageBreakdownForModels(modelIndexes),
        modelPerformance: demoModelPerformanceForModels(modelIndexes),
        cacheHitRate: Number((0.814 - index * 0.012).toFixed(3)),
        tokensPerMinute: Math.max(2_100, 37_852 - index * 3_710),
        spendRate: Number(Math.max(1.1, 15.82 - index * 1.43).toFixed(2)),
        firstByteAvgMs: 198 + index * 18,
        firstTokenAvgMs: 780 + index * 54,
        avgTotalMs: 2_536 + index * 184,
        currentFirstTokenAvgMs: 720 + index * 61,
        currentAvgTotalMs: 2_536 + index * 184,
        inProgressInvocationCount:
          account.id === 101
            ? attention
              ? 7
              : 3
            : accountRecords.filter((record) => record.status === "running").length,
        inProgressPhaseCounts: {
          queued: index % 2,
          requesting: index === 1 ? 1 : 0,
          responding: account.id === 101 ? 1 : 0,
        },
        retryInvocationCount: accountRecords.filter(
          (record) => record.poolAttemptCount && record.poolAttemptCount > 1,
        ).length,
        effectiveRoutingRule: account.effectiveRoutingRule,
        recentInvocations: accountRecords,
      };
    });
}

function demoDashboardActivitySummary(accounts: ReturnType<typeof demoDashboardActivityAccounts>) {
  const base = demoSummary();
  const sum = (read: (account: (typeof accounts)[number]) => number) =>
    accounts.reduce((total, account) => total + read(account), 0);
  const totalCount = sum((account) => account.requestCount);
  const successCount = sum((account) => account.successCount);
  const failureCount = sum((account) => account.failureCount);
  const totalTokens = sum((account) => account.totalTokens);
  const totalCost = Number(sum((account) => account.totalCost).toFixed(2));
  const cacheInputTokens = Math.round(sum((account) => account.totalTokens * account.cacheHitRate));
  return {
    ...base,
    totalCount,
    successCount,
    failureCount,
    totalTokens,
    totalCost,
    inProgressConversationCount: sum((account) => account.inProgressInvocationCount),
    token: {
      ...base.token,
      requestCount: totalCount,
      totalTokens,
      avgTokensPerRequest: totalCount === 0 ? 0 : Math.round(totalTokens / totalCount),
      cacheInputTokens,
      totalCost,
    },
    exception: {
      ...base.exception,
      failureCount,
      serviceFailureCount: failureCount,
      clientFailureCount: 0,
      clientAbortCount: 0,
      actionableFailureCount: failureCount,
    },
  };
}

function timeseries() {
  const empty = demoModel.snapshot.scene === "empty";
  const start = Date.parse(demoNow()) - 24 * 3_600_000;
  return {
    rangeStart: new Date(start).toISOString(),
    rangeEnd: demoNow(),
    bucketSeconds: 3600,
    effectiveBucket: "1h",
    availableBuckets: ["1m", "15m", "1h", "1d"],
    points: empty
      ? []
      : Array.from({ length: 24 }, (_, index) => ({
          bucketStart: new Date(start + index * 3_600_000).toISOString(),
          bucketEnd: new Date(start + (index + 1) * 3_600_000).toISOString(),
          totalCount: 920 + index * 61,
          successCount: 886 + index * 57,
          failureCount: 34 + (index % 3),
          totalTokens: 104_000_000 + index * 4_200_000,
          totalCost: 42.1 + index * 1.2,
          avgLatencyMs: 210 + index * 4,
        })),
  };
}

function parallelWork() {
  const start = Date.parse(demoNow()) - 24 * 3_600_000;
  const points =
    demoModel.snapshot.scene === "empty"
      ? []
      : Array.from({ length: 24 }, (_, index) => ({
          bucketStart: new Date(start + index * 3_600_000).toISOString(),
          bucketEnd: new Date(start + (index + 1) * 3_600_000).toISOString(),
          parallelCount: 2 + (index % 5),
        }));
  const current = {
    rangeStart: new Date(start).toISOString(),
    rangeEnd: demoNow(),
    bucketSeconds: 3600,
    completeBucketCount: points.length,
    activeBucketCount: points.length,
    activeMinuteCount: points.length,
    minCount: points.length ? 2 : null,
    maxCount: points.length ? 6 : null,
    avgCount: points.length ? 4 : null,
    effectiveTimeZone: "Asia/Shanghai",
    timeZoneFallback: false,
    points,
    conversations: [],
  };
  return { current, minute7d: current, hour30d: current, dayAll: current };
}

function promptCacheConversations() {
  const nowMs = Date.parse(demoNow());
  if (demoModel.snapshot.scene === "empty") {
    return {
      rangeStart: new Date(nowMs - 24 * 3_600_000).toISOString(),
      rangeEnd: demoNow(),
      selectionMode: "count",
      selectedLimit: 50,
      selectedActivityHours: null,
      selectedActivityMinutes: null,
      implicitFilter: { kind: null, filteredCount: 0 },
      totalMatched: 0,
      hasMore: false,
      nextCursor: null,
      conversations: [],
    };
  }
  const records = invocations();
  const accounts = new Map(demoAccounts().map((account) => [account.id, account]));
  const conversation = (
    promptCacheKey: string,
    encryptedOwnerAccountId: number | null,
    requestCount: number,
  ) => {
    const recent = records.filter((record) => record.promptCacheKey === promptCacheKey).slice(0, 4);
    const owner = encryptedOwnerAccountId == null ? null : accounts.get(encryptedOwnerAccountId);
    const upstreamAccounts = Array.from(
      new Set(
        recent.map((record) => record.upstreamAccountId).filter((id) => typeof id === "number"),
      ),
    ).map((id) => {
      const account = accounts.get(id);
      const accountRecords = recent.filter((record) => record.upstreamAccountId === id);
      return {
        upstreamAccountId: id,
        upstreamAccountName: account?.displayName ?? null,
        requestCount: accountRecords.length,
        totalTokens: accountRecords.reduce((total, record) => total + (record.totalTokens ?? 0), 0),
        totalCost: Number(
          accountRecords.reduce((total, record) => total + (record.cost ?? 0), 0).toFixed(4),
        ),
        lastActivityAt: accountRecords[0]?.occurredAt ?? demoNow(),
      };
    });
    return {
      promptCacheKey,
      hasEncryptedSessionOwner: owner != null,
      encryptedOwnerAccountId,
      encryptedOwnerAccountName: owner?.displayName ?? null,
      encryptedOwnerGroupName: owner?.groupName ?? null,
      requestCount,
      totalTokens: recent.reduce((total, record) => total + (record.totalTokens ?? 0), 0),
      totalCost: Number(recent.reduce((total, record) => total + (record.cost ?? 0), 0).toFixed(4)),
      createdAt: new Date(nowMs - requestCount * 90_000).toISOString(),
      lastActivityAt: recent[0]?.occurredAt ?? demoNow(),
      lastTerminalAt: recent.find((record) => record.status !== "running")?.occurredAt ?? null,
      lastInFlightAt: recent.find((record) => record.status === "running")?.occurredAt ?? null,
      upstreamAccounts,
      recentInvocations: recent,
      last24hRequests: Array.from({ length: 12 }, (_, index) => ({
        occurredAt: new Date(nowMs - (12 - index) * 90 * 60_000).toISOString(),
        status: index === 6 && promptCacheKey === "demo-research-batch" ? "http_429" : "success",
        isSuccess: !(index === 6 && promptCacheKey === "demo-research-batch"),
        outcome: index === 6 && promptCacheKey === "demo-research-batch" ? "failure" : "success",
      })),
    };
  };
  return {
    rangeStart: new Date(nowMs - 24 * 3_600_000).toISOString(),
    rangeEnd: demoNow(),
    selectionMode: "count",
    selectedLimit: 50,
    selectedActivityHours: null,
    selectedActivityMinutes: null,
    implicitFilter: { kind: null, filteredCount: 0 },
    totalMatched: 11,
    hasMore: false,
    nextCursor: null,
    conversations: [
      conversation("demo-conversation-a", 101, 38),
      conversation("demo-research-batch", 105, 27),
      conversation("demo-image-workflow", 104, 19),
      conversation("demo-indexing", null, 14),
      conversation("demo-conversation-b", null, 11),
      conversation("demo-conversation-c", 107, 9),
      conversation("demo-conversation-d", 114, 12),
      conversation("demo-edge-monitor", 111, 17),
      conversation("demo-batch-west", 112, 14),
      conversation("demo-mobile-e2e", 114, 8),
      conversation("demo-recovery", 115, 10),
    ],
  };
}

function forwardProxyLive() {
  if (demoModel.snapshot.scene === "empty") {
    return {
      rangeStart: "2026-07-10T00:00:00Z",
      rangeEnd: demoNow(),
      bucketSeconds: 3600,
      nodes: [],
    };
  }
  const nodes = demoForwardProxyNodes();
  return {
    rangeStart: "2026-07-10T00:00:00Z",
    rangeEnd: demoNow(),
    bucketSeconds: 3600,
    nodes: nodes.map((node, nodeIndex) => ({
      ...node,
      last24h: Array.from({ length: 8 }, (_, index) => ({
        bucketStart: `2026-07-10T${String(index + 1).padStart(2, "0")}:00:00Z`,
        bucketEnd: `2026-07-10T${String(index + 2).padStart(2, "0")}:00:00Z`,
        successCount: 11 + nodeIndex * 3 + index,
        failureCount:
          demoModel.snapshot.scene === "attention" && nodeIndex === 1 && index >= 6
            ? 3
            : index % 4 === 0
              ? 1
              : 0,
      })),
      weight24h: Array.from({ length: 8 }, (_, index) => ({
        bucketStart: `2026-07-10T${String(index + 1).padStart(2, "0")}:00:00Z`,
        bucketEnd: `2026-07-10T${String(index + 2).padStart(2, "0")}:00:00Z`,
        sampleCount: 11 + nodeIndex * 3 + index,
        minWeight: Number(((node.weight as number) - 0.12).toFixed(2)),
        maxWeight: Number(((node.weight as number) + 0.04).toFixed(2)),
        avgWeight: Number(((node.weight as number) - 0.02).toFixed(2)),
        lastWeight: node.weight,
      })),
    })),
  };
}

function accountList() {
  const items = demoModel.snapshot.scene === "empty" ? [] : demoAccounts();
  return {
    items,
    total: items.length,
    page: 1,
    pageSize: 50,
    groups: [
      {
        groupName: "production",
        note: "Primary workload with priority capacity.",
        accountCount: items.filter((item) => item.groupName === "production").length,
        boundProxyKeys: ["demo-tokyo", "demo-singapore"],
        concurrencyLimit: 12,
        nodeShuntEnabled: true,
        singleAccountRotationEnabled: false,
        upstream429RetryEnabled: true,
        upstream429MaxRetries: 2,
        routingRule: {
          allowCutIn: true,
          allowCutOut: true,
          priorityTier: "primary",
          fastModeRewriteMode: "keep_original",
          concurrencyLimit: 12,
          upstream429RetryEnabled: true,
          upstream429MaxRetries: 2,
        },
      },
      {
        groupName: "research",
        note: "Long-running research and batch jobs.",
        accountCount: items.filter((item) => item.groupName === "research").length,
        boundProxyKeys: ["demo-tokyo", "demo-frankfurt"],
        concurrencyLimit: 8,
        nodeShuntEnabled: true,
        singleAccountRotationEnabled: true,
        upstream429RetryEnabled: true,
        upstream429MaxRetries: 3,
        routingRule: {
          allowCutIn: true,
          allowCutOut: true,
          priorityTier: "normal",
          fastModeRewriteMode: "keep_original",
          concurrencyLimit: 8,
          upstream429RetryEnabled: true,
          upstream429MaxRetries: 3,
        },
      },
      {
        groupName: "standby",
        note: "Fallback capacity retained for recovery routing.",
        accountCount: items.filter((item) => item.groupName === "standby").length,
        boundProxyKeys: ["demo-frankfurt", "demo-singapore"],
        concurrencyLimit: 4,
        nodeShuntEnabled: false,
        singleAccountRotationEnabled: false,
        upstream429RetryEnabled: true,
        upstream429MaxRetries: 1,
        routingRule: {
          allowCutIn: false,
          allowCutOut: true,
          priorityTier: "fallback",
          fastModeRewriteMode: "keep_original",
          concurrencyLimit: 4,
          upstream429RetryEnabled: true,
          upstream429MaxRetries: 1,
        },
      },
      {
        groupName: "edge",
        note: "Regional monitoring and mobile smoke checks.",
        accountCount: items.filter((item) => item.groupName === "edge").length,
        boundProxyKeys: ["demo-sydney", "demo-virginia"],
        concurrencyLimit: 6,
        nodeShuntEnabled: true,
        singleAccountRotationEnabled: false,
        upstream429RetryEnabled: true,
        upstream429MaxRetries: 2,
        routingRule: {
          allowCutIn: true,
          allowCutOut: true,
          priorityTier: "normal",
          fastModeRewriteMode: "fill_missing",
          concurrencyLimit: 6,
          upstream429RetryEnabled: true,
          upstream429MaxRetries: 2,
        },
      },
    ],
    forwardProxyNodes: demoForwardProxyNodes(),
    writesEnabled: true,
    availableModels: ["gpt-5.6-sol", "gpt-5.6-terra", "gpt-5.4-mini"],
    hasUngroupedAccounts: items.some((item) => item.groupName == null),
    metrics: {
      total: items.length,
      oauth: items.filter((item) => item.kind === "oauth_codex").length,
      apiKey: items.filter((item) => item.kind === "api_key_codex").length,
      attention: items.filter((item) => item.healthStatus !== "normal").length,
    },
    routing: {
      writesEnabled: true,
      apiKeyConfigured: true,
      maskedApiKey: "cvm_pool••••••",
      maintenance: {
        primarySyncIntervalSecs: 300,
        secondarySyncIntervalSecs: 1800,
        priorityAvailableAccountCap: 100,
      },
      timeouts: {
        responsesFirstByteTimeoutSecs: 30,
        compactFirstByteTimeoutSecs: 45,
        imageFirstByteTimeoutSecs: 300,
        responsesStreamTimeoutSecs: 300,
        compactStreamTimeoutSecs: 420,
      },
    },
  };
}

function systemStatus() {
  const pressureState = demoModel.snapshot.scene.replace("runtime-pressure-", "");
  const runtimeState = pressureState === demoModel.snapshot.scene ? "healthy" : pressureState;
  const accountingError = runtimeState === "accounting-error";
  return {
    liveInvocationsCount: 128_076,
    successCount: 124_882,
    nonSuccessCount: 3_194,
    completedArchiveBatchesCount: 384,
    archivedBodies: { count: 118_420, bytes: 8_441_053_184 },
    rawBodies: { count: 1_482, bytes: 84_221_184 },
    requestRawBodies: { count: 812, bytes: 76_221_184 },
    responseRawBodies: { count: 670, bytes: 8_000_000 },
    databaseBytes: 618_659_840,
    otherFilesBytes: 142_344_192,
    rawMetricsHealth: { state: "ready", inventoryCursor: 128_076 },
    projectionHealth: {
      terminal: {
        state: "healthy",
        cursorLag: 0,
        dirtyBucketCount: 0,
        pendingEventCount: 0,
      },
      longTerm: {
        state: "healthy",
        cursorLag: 0,
        dirtyBucketCount: 0,
        pendingEventCount: 0,
        lastFlushElapsedMs: 72,
        lastFlushAgeMs: 1_200,
      },
    },
    runtimePressureHealth: {
      state: accountingError ? "accounting_error" : runtimeState,
      process: {
        rssBytes: 1_073_741_824,
        rssAnonBytes: 805_306_368,
        swapBytes: runtimeState === "degraded" ? 268_435_456 : 0,
        peakRssBytes: 1_342_177_280,
        threads: 18,
        managedBytes: 536_870_912,
        unattributedAnonBytes: 268_435_456,
        pressureLevel: runtimeState === "degraded" ? "elevated" : "normal",
      },
      allocator: { mallocArenaMax: "8" },
      writerAccounting: {
        state: accountingError ? "degraded" : "healthy",
        pendingDepth: accountingError ? 24 : 3,
        pendingBytes: accountingError ? 12_582_912 : 524_288,
        transferBytes: 67_108_864,
        retryCount: accountingError ? 4 : 0,
        invariantViolationCount: accountingError ? 1 : 0,
        degradedReason: accountingError ? "pending_bytes_underflow" : undefined,
      },
      retentionWriteHealth: {
        state:
          runtimeState === "deferred"
            ? "deferred"
            : runtimeState === "degraded"
              ? "degraded"
              : "healthy",
        operation: "invocation_detail_prune",
        admissionMode: runtimeState === "degraded" ? "fairness" : "normal",
        batchRows: 4,
        estimatedBytes: 16_384,
        prepareElapsedMs: 36,
        lockWaitMs: runtimeState === "degraded" ? 15_004 : 2,
        executeMs: runtimeState === "degraded" ? 251 : 47,
        commitMs: runtimeState === "degraded" ? 36 : 18,
        budgetBreachCount: runtimeState === "degraded" ? 1 : 0,
        deferReason: runtimeState === "deferred" ? "pressure_cooldown:30000ms" : undefined,
        starvationAgeMs: runtimeState === "degraded" ? 15_004 : undefined,
        p1WaiterCount: runtimeState === "degraded" ? 1 : 0,
        candidateRemainingHint: 1,
      },
      dashboardProjection: {
        mode: "auto",
        state: runtimeState === "degraded" ? "degraded" : "healthy",
        producerState: runtimeState === "deferred" ? "idle" : "running",
        activeSubscriberCount: 2,
        livePathDbReadCount: 0,
        buildCount: 418,
        revision: 771,
        snapshotOrigin: "runtime_projection",
        lastGoodAgeMs: 320,
        degradedReason: runtimeState === "degraded" ? "projection_stale" : undefined,
        lastDeferReason: runtimeState === "deferred" ? "writer_pressure" : undefined,
        sliceCounters: {
          current: {
            buildCount: 418,
            revisionCount: 771,
            cadenceMissCount: runtimeState === "degraded" ? 2 : 0,
          },
          network: { buildCount: 42, revisionCount: 104, cadenceMissCount: 0 },
          terminal: { buildCount: 8, revisionCount: 29, cadenceMissCount: 0 },
        },
      },
      delivery: {
        activity: {
          materializationCount: 418,
          serializationCount: 418,
          payloadCloneCount: 0,
          frameBytesCount: 1_048_576,
          laggedCount: runtimeState === "degraded" ? 1 : 0,
          skippedCount: 0,
          businessPayloadCount: 418,
          jsonOverlayCount: 0,
        },
        summary: {
          materializationCount: 29,
          serializationCount: 29,
          payloadCloneCount: 0,
          frameBytesCount: 65_536,
          laggedCount: 0,
          skippedCount: 0,
          businessPayloadCount: 29,
          jsonOverlayCount: 0,
        },
        networkTimeseries: {
          materializationCount: 104,
          serializationCount: 104,
          payloadCloneCount: 0,
          frameBytesCount: 131_072,
          laggedCount: 0,
          skippedCount: 0,
          businessPayloadCount: 104,
          jsonOverlayCount: 0,
        },
        networkRecent: {
          materializationCount: 104,
          serializationCount: 104,
          payloadCloneCount: 0,
          frameBytesCount: 65_536,
          laggedCount: 0,
          skippedCount: 0,
          businessPayloadCount: 104,
          jsonOverlayCount: 0,
        },
      },
      eventBus: {
        state: runtimeState === "degraded" ? "degraded" : "healthy",
        publishedCount: 912,
        processedEventCount: 856,
        coalescedEventCount: 56,
        businessPayloadCloneCount: 0,
        topicWorkCount: 856,
        routerLaggedCount: runtimeState === "degraded" ? 2 : 0,
        routerGapCount: runtimeState === "degraded" ? 1 : 0,
        cursorRecoveryCount: runtimeState === "degraded" ? 1 : 0,
      },
      backfill: {
        state: runtimeState === "deferred" ? "deferred" : "healthy",
        wakeGeneration: 14,
        wakeCount: 14,
        dueDispatchCount: 28,
        noopSuppressedCount: 42,
        pressureDeferCount: runtimeState === "deferred" ? 3 : 0,
        failureCount: 0,
        wokenTaskCount: 0,
        scheduledTaskCount: 5,
        deferredTaskCount: runtimeState === "deferred" ? 1 : 0,
        failedTaskCount: 0,
      },
    },
    refreshedAt: demoNow(),
  };
}

function forwardProxyBindingNodes() {
  const nodes = demoForwardProxyNodes();
  return nodes.map((node, index) => ({
    key: node.key,
    aliasKeys: [],
    source: node.source,
    displayName: node.displayName,
    protocolLabel: node.endpointUrl?.toString().startsWith("http:") ? "HTTP" : "SOCKS5",
    egressIp: `198.51.100.${31 + index}`,
    egressIpCheckedAt: `2026-07-10T09:${String(12 + index).padStart(2, "0")}:00Z`,
    egressIpProvider: "demo resolver",
    egressIpError: null,
    egressIpErrorAt: null,
    penalized: node.penalized,
    selectable: true,
    last24h: Array.from({ length: 6 }, (_, bucketIndex) => ({
      bucketStart: `2026-07-10T${String(bucketIndex + 3).padStart(2, "0")}:00:00Z`,
      bucketEnd: `2026-07-10T${String(bucketIndex + 4).padStart(2, "0")}:00:00Z`,
      successCount: 14 + index * 4 + bucketIndex,
      failureCount:
        demoModel.snapshot.scene === "attention" && index === 1 && bucketIndex === 5 ? 3 : 0,
    })),
  }));
}

function accountEvents() {
  if (demoModel.snapshot.scene === "empty") return [];
  const accounts = demoAccounts();
  const routingEvents = demoModelRoutingLiveTimeline()
    .filter((event) => event.kind === "event")
    .flatMap((event, index) => {
      const account = accounts.find((candidate) => candidate.id === event.accountId);
      if (!account) return [];
      const proxyKey = account.boundProxyKeys?.[0] ?? null;
      const stateAfter = event.modelRouteStateAfter ?? null;
      return [
        {
          id: 7100 + index,
          action: event.action ?? "model_route_state_changed",
          source: event.source ?? "call",
          result: stateAfter === "available" ? "success" : "failed",
          upstreamAccountId: account.id,
          accountDisplayName: account.displayName,
          accountGroupName: account.groupName,
          forwardProxyKey: proxyKey,
          forwardProxyDisplayName: account.currentForwardProxyDisplayName ?? null,
          forwardProxyEgressIp:
            proxyKey === "demo-tokyo"
              ? "198.51.100.31"
              : proxyKey === "demo-frankfurt"
                ? "198.51.100.32"
                : "198.51.100.33",
          reasonCode: event.reasonCode ?? null,
          reasonMessage: event.reasonCode
            ? "Route state changed after an upstream HTTP 502."
            : null,
          httpStatus: event.reasonCode === "upstream_http_5xx" ? 502 : null,
          model: event.model,
          modelRouteStateBefore: event.modelRouteStateBefore ?? null,
          modelRouteStateAfter: stateAfter,
          modelRoutePriorityBefore:
            event.modelRouteStateBefore === "available" ? "normal" : "demoted",
          modelRoutePriorityAfter:
            stateAfter === "cooling_down"
              ? "excluded"
              : stateAfter === "degraded"
                ? "demoted"
                : "normal",
          modelRouteFailureCount: event.modelRouteFailureCount ?? null,
          modelRouteCooldownUntil: event.modelRouteCooldownUntil ?? null,
          failureKind: stateAfter === "available" ? null : "model",
          invokeId: event.invokeId ?? null,
          stickyKey: null,
          occurredAt: event.occurredAt,
          createdAt: event.occurredAt,
        },
      ];
    });
  const maintenanceActions: Array<readonly [string, string, number]> = [
    ["sync_succeeded", "success", 30],
    ["usage_snapshot_updated", "success", 60],
    ["forward_proxy_assigned", "success", 120],
    ["routing_rule_updated", "success", 180],
    ["forward_proxy_health_checked", "success", 240],
    ["quota_window_reset_observed", "success", 300],
    ["sync_succeeded", "success", 360],
    ["usage_snapshot_updated", "success", 420],
    ["sync_succeeded", "success", 480],
    ["routing_rule_updated", "success", 540],
    ["forward_proxy_health_checked", "success", 600],
    ["usage_snapshot_updated", "success", 660],
  ];
  const maintenanceEvents = maintenanceActions.map(([action, result, minutesAgo], index) => {
    const account = accounts[(index + 1) % accounts.length] ?? accounts[0];
    const proxyKey = account.boundProxyKeys?.[0] ?? null;
    const occurredAt = demoModelRouteTimestamp(minutesAgo);
    return {
      id: 7200 + index,
      action,
      source: "sync_maintenance",
      result,
      upstreamAccountId: account.id,
      accountDisplayName: account.displayName,
      accountGroupName: account.groupName,
      forwardProxyKey: proxyKey,
      forwardProxyDisplayName: account.currentForwardProxyDisplayName ?? null,
      forwardProxyEgressIp:
        proxyKey === "demo-tokyo"
          ? "198.51.100.31"
          : proxyKey === "demo-frankfurt"
            ? "198.51.100.32"
            : "198.51.100.33",
      reasonCode: null,
      reasonMessage: null,
      httpStatus: null,
      model: null,
      modelRouteStateBefore: null,
      modelRouteStateAfter: null,
      modelRoutePriorityBefore: null,
      modelRoutePriorityAfter: null,
      modelRouteFailureCount: null,
      modelRouteCooldownUntil: null,
      failureKind: null,
      invokeId: null,
      stickyKey: null,
      occurredAt,
      createdAt: occurredAt,
    };
  });
  return [...routingEvents, ...maintenanceEvents].sort(
    (left, right) => Date.parse(right.occurredAt) - Date.parse(left.occurredAt),
  );
}

function systemTasks() {
  if (demoModel.snapshot.scene === "empty") return [];
  const at = (minutesAgo: number) =>
    new Date(Date.parse(demoNow()) - minutesAgo * 60_000).toISOString();
  return [
    {
      id: 1,
      taskKind: "archive_rollup",
      triggerKind: "scheduler",
      status: "success",
      summary: "Hourly invocation archive rollup completed.",
      detail: "Rolled up 12 completed archive batches and compacted aggregate counters.",
      startedAt: at(3),
      finishedAt: at(2),
      durationMs: 14_203,
    },
    {
      id: 2,
      taskKind: "upstream_account_sync",
      triggerKind: "scheduler",
      status: "success",
      summary: "Production pool quota snapshot completed.",
      detail: "Synchronized 6 production accounts through the assigned relay nodes.",
      startedAt: at(9),
      finishedAt: at(8),
      durationMs: 22_118,
    },
    {
      id: 3,
      taskKind: "forward_proxy_subscription_refresh",
      triggerKind: "manual",
      status: "success",
      summary: "Relay subscription refreshed.",
      detail: "Five demo relay nodes were retained and their health probes completed.",
      startedAt: at(18),
      finishedAt: at(18),
      durationMs: 8_447,
    },
    {
      id: 4,
      taskKind: "raw_body_compression",
      triggerKind: "scheduler",
      status: "running",
      summary: "Compressing retained invocation response bodies.",
      detail: "The demo task is intentionally in progress to populate active task status.",
      startedAt: at(32),
      durationMs: 1_920_000,
    },
    {
      id: 5,
      taskKind: "pricing_catalog_refresh",
      triggerKind: "scheduler",
      status: "success",
      summary: "Pricing catalog is current.",
      detail: "Validated three configured models against the demo pricing catalog.",
      startedAt: at(47),
      finishedAt: at(47),
      durationMs: 3_126,
    },
    {
      id: 6,
      taskKind: "upstream_account_sync",
      triggerKind: "scheduler",
      status: "failed",
      summary: "Standby account health check timed out.",
      detail:
        "The recovery relay exceeded the simulated upstream timeout threshold; retry is queued.",
      startedAt: at(66),
      finishedAt: at(65),
      durationMs: 31_022,
    },
    {
      id: 7,
      taskKind: "historical_backfill",
      triggerKind: "manual",
      status: "skipped",
      summary: "No historical gaps require backfill.",
      detail: "The demo datastore already contains all required hourly buckets.",
      startedAt: at(91),
      finishedAt: at(91),
      durationMs: 862,
    },
    {
      id: 8,
      taskKind: "forward_proxy_latency_probe",
      triggerKind: "scheduler",
      status: "success",
      summary: "All relay latency probes completed.",
      detail: "Measured egress, OAuth upstream, and responses latency for five relay nodes.",
      startedAt: at(113),
      finishedAt: at(112),
      durationMs: 42_907,
    },
    {
      id: 9,
      taskKind: "prompt_cache_cleanup",
      triggerKind: "scheduler",
      status: "success",
      summary: "Prompt cache retention sweep completed.",
      detail: "Retained active conversations and removed no demo records.",
      startedAt: at(146),
      finishedAt: at(145),
      durationMs: 9_441,
    },
    {
      id: 10,
      taskKind: "usage_snapshot_reconciliation",
      triggerKind: "manual",
      status: "success",
      summary: "Usage window reconciliation completed.",
      detail: "Compared current primary and secondary windows across all demo accounts.",
      startedAt: at(188),
      finishedAt: at(187),
      durationMs: 27_630,
    },
  ];
}

function poolAttempts(invokeId: string) {
  const record = invocations().find((item) => item.invokeId === invokeId);
  if (!record) return [];
  const accountId = record.upstreamAccountId ?? 101;
  const fallback = accountId === 105 ? 106 : 102;
  const needsRetry = (record.poolAttemptCount ?? 1) > 1;
  const startedAt = record.occurredAt;
  const base = {
    invokeId,
    occurredAt: startedAt,
    endpoint: record.endpoint ?? "/v1/responses",
    stickyKey: record.stickyKey ?? null,
    requesterIp: record.requesterIp ?? null,
    createdAt: startedAt,
  };
  const first = {
    ...base,
    id: record.id * 10 + 1,
    attemptId: record.id === 9002 ? "qPvNNAK8" : formatDemoAttemptId(record.id * 100 + 1),
    upstreamAccountId: accountId,
    upstreamAccountName: record.upstreamAccountName ?? null,
    upstreamRouteKey: "pool",
    proxyBindingKeySnapshot:
      record.proxyDisplayName === "Tokyo demo relay"
        ? "demo-tokyo"
        : record.proxyDisplayName === "Frankfurt recovery relay"
          ? "demo-frankfurt"
          : "demo-singapore",
    attemptIndex: 1,
    distinctAccountIndex: 1,
    sameAccountRetryIndex: 0,
    startedAt,
    finishedAt: needsRetry
      ? `2026-07-10T09:24:00Z`
      : record.status === "running"
        ? null
        : `2026-07-10T09:25:00Z`,
    status: needsRetry ? "failed" : (record.status ?? "success"),
    phase: record.status === "running" ? "responding" : "completed",
    httpStatus: needsRetry ? 429 : (record.downstreamStatusCode ?? 200),
    downstreamHttpStatus: needsRetry ? 429 : (record.downstreamStatusCode ?? 200),
    failureKind: needsRetry ? "rate_limited" : (record.failureKind ?? null),
    errorMessage: needsRetry ? "Simulated retry after rate limit." : (record.errorMessage ?? null),
    connectLatencyMs: record.tUpstreamConnectMs ?? 42,
    firstByteLatencyMs: record.tUpstreamTtfbMs ?? null,
    streamLatencyMs: record.tUpstreamStreamMs ?? null,
    upstreamRequestId: `up_demo_${record.id}_1`,
  };
  if (!needsRetry) return [first];
  return [
    first,
    {
      ...first,
      id: record.id * 10 + 2,
      attemptId: record.id === 9002 ? "DEMO-SUCCESS-1" : formatDemoAttemptId(record.id * 100 + 2),
      upstreamAccountId: record.id === 9002 ? 2890 : fallback,
      upstreamAccountName:
        record.id === 9002
          ? "dzw"
          : (demoAccounts().find((account) => account.id === fallback)?.displayName ?? null),
      attemptIndex: 2,
      distinctAccountIndex: 2,
      sameAccountRetryIndex: 0,
      routingSource: "freshAssignment",
      routingSelectionAudit:
        record.id === 9002
          ? {
              selectedAccountId: 2890,
              selectedAccountName: "dzw",
              eligibleCandidateCount: 1,
              winnerReasonCode: "onlyEligibleCandidate",
              comparedAccountId: null,
              comparedAccountName: null,
              excludedCandidates: [
                {
                  accountId: 2805,
                  accountName: "CIII",
                  reasonCode: "modelNotAllowed",
                },
              ],
            }
          : null,
      proxyBindingKeySnapshot: "demo-frankfurt",
      status: record.status === "http_502" ? "failed" : "success",
      httpStatus: record.status === "http_502" ? 502 : 200,
      downstreamHttpStatus: record.status === "http_502" ? 502 : 200,
      failureKind: record.status === "http_502" ? "upstream_timeout" : null,
      errorMessage: record.status === "http_502" ? "Simulated recovery relay timeout." : null,
      startedAt: "2026-07-10T09:24:02Z",
      finishedAt: "2026-07-10T09:24:05Z",
      upstreamRequestId: `up_demo_${record.id}_2`,
    },
  ];
}

function buildDemoInvocationWorkflowDetail(
  record: ReturnType<typeof invocations>[number],
): ApiInvocationWorkflowDetailResponse {
  const finalStatus =
    record.status === "success"
      ? "completed"
      : record.status === "running"
        ? "running"
        : record.status;
  const requestModel = record.requestModel ?? record.model ?? "gpt-5.6-sol";
  const responseModel = record.responseModel ?? record.model ?? requestModel;
  const requestHeaders = {
    userAgent: "monitor-ui/1.0",
    xForwardedFor: record.requesterIp ?? "203.0.113.24",
    forwarded: `for=${record.requesterIp ?? "203.0.113.24"};proto=https`,
  };
  const requestCompression = {
    algorithm: "zstd",
    mode: "recompressed",
    logicalBodyBytes: DEMO_INVOCATION_REQUEST_BODY_SIZE,
    transmittedBodyBytes: DEMO_INVOCATION_REQUEST_BODY_TRANSMITTED_BYTES,
    savedBytes: DEMO_INVOCATION_REQUEST_BODY_SIZE - DEMO_INVOCATION_REQUEST_BODY_TRANSMITTED_BYTES,
    ratioPct: -63,
    approxUploadBytes: DEMO_INVOCATION_REQUEST_BODY_TRANSMITTED_BYTES,
    approxDownloadBytes: 135_800,
  };
  const requestRouting = {
    routeMode: record.routeMode ?? "pool",
    proxyDisplayName: record.proxyDisplayName ?? "Tokyo demo relay",
    upstreamRouteKey: `route-${record.routeMode ?? "pool"}-primary`,
    proxyBindingKey: "fpb_demo_tokyo_primary",
    promptCacheKey: record.promptCacheKey ?? null,
    stickyKey: record.stickyKey ?? null,
  };
  const routeRequest = {
    endpoint: record.endpoint ?? "/v1/responses",
    routeMode: record.routeMode ?? "pool",
    transport: record.transport ?? "http",
    requestModel,
    responseModel,
    requestedServiceTier: record.requestedServiceTier ?? "priority",
    reasoningEffort: record.reasoningEffort ?? "high",
    compactionRequestKind: "remote_v2",
    promptCacheKey: record.promptCacheKey ?? null,
    stickyKey: record.stickyKey ?? null,
    requesterIp: record.requesterIp ?? null,
    routing: requestRouting,
    headers: requestHeaders,
    bodyCapture: {
      availableAtInvocationLevel: false,
      size: DEMO_INVOCATION_REQUEST_BODY_SIZE,
      truncated: false,
      detailLevel: "full",
    },
  };
  const attemptResponseSummary = {
    status: finalStatus,
    phase: record.status === "running" ? "responding" : "completed",
    serviceTier: "default",
    billingServiceTier: record.billingServiceTier ?? "standard",
    responseContentEncoding: "identity",
    compactionResponseKind: "remote_v2",
    outputItems: 1,
    headers: {
      contentEncoding: "identity",
      upstreamRequestId: `req_demo_${record.id}`,
      cvmInvokeId: record.invokeId,
    },
    delivery: {
      forwardedChunkCount: 12,
      forwardedBytes: 138_649,
      usageObserved: true,
      downstreamClosePhase: null,
    },
    responseBodyCapture: {
      availableAtInvocationLevel: true,
      availableAtAttemptLevel: true,
      size: DEMO_INVOCATION_RESPONSE_BODY_SIZE,
      truncated: false,
      detailLevel: "full",
    },
    usage: {
      totalTokens: record.totalTokens ?? null,
    },
  };

  const timeline: ApiInvocationWorkflowDetailResponse["timeline"] = [
    {
      blockId: `route-${record.id}`,
      kind: "routingDecision",
      occurredAt: record.occurredAt,
      title: "Route resolution",
      subtitle: `${requestModel} · ${record.endpoint ?? "/v1/responses"}`,
      status: record.routeMode ?? "pool",
      detail: {
        request: routeRequest,
        requestHeaders,
        requestBody: {
          availableAtInvocationLevel: false,
          size: DEMO_INVOCATION_REQUEST_BODY_SIZE,
          truncated: false,
          detailLevel: "full",
        },
        routeMode: record.routeMode ?? "pool",
        poolAttemptCount: record.poolAttemptCount ?? 1,
      },
    },
    {
      blockId: `attempt-${record.id}-1`,
      kind: "attempt",
      occurredAt: record.occurredAt,
      title: "Attempt 1",
      subtitle: record.upstreamAccountName ?? record.proxyDisplayName ?? "Demo account",
      status: finalStatus,
      attempt: {
        synthetic: false,
        attemptId: "qPvNNAK8",
        occurredAt: record.occurredAt,
        endpoint: record.endpoint ?? "/v1/responses",
        stickyKey: record.stickyKey ?? null,
        upstreamAccountId: record.upstreamAccountId ?? null,
        upstreamAccountName: record.upstreamAccountName ?? null,
        requestModel,
        responseModel,
        upstreamRouteKey: requestRouting.upstreamRouteKey,
        proxyBindingKeySnapshot: requestRouting.proxyBindingKey,
        attemptIndex: 1,
        distinctAccountIndex: 1,
        sameAccountRetryIndex: 0,
        requesterIp: record.requesterIp ?? null,
        startedAt: record.occurredAt,
        finishedAt: record.occurredAt,
        status: finalStatus,
        phase: "streaming",
        httpStatus: 200,
        downstreamHttpStatus: record.downstreamStatusCode ?? 200,
        failureKind: record.failureKind ?? null,
        errorMessage: record.errorMessage ?? null,
        connectLatencyMs: record.tUpstreamConnectMs ?? 184,
        firstByteLatencyMs: record.tUpstreamTtfbMs ?? 0,
        streamLatencyMs: 4_830,
        upstreamRequestId: `req_demo_${record.id}`,
        requestSummary: {
          endpoint: record.endpoint ?? "/v1/responses",
          routeMode: record.routeMode ?? "pool",
          transport: record.transport ?? "http",
          requestModel,
          responseModel,
          stickyKey: record.stickyKey ?? null,
          requestedServiceTier: record.requestedServiceTier ?? "priority",
          reasoningEffort: record.reasoningEffort ?? "high",
          compactionRequestKind: "remote_v2",
          headers: requestHeaders,
          routing: requestRouting,
          compression: requestCompression,
          bodyCapture: {
            availableAtInvocationLevel: false,
            size: DEMO_INVOCATION_REQUEST_BODY_SIZE,
            truncated: false,
            detailLevel: "full",
          },
        },
        responseSummary: attemptResponseSummary,
      },
    },
  ];

  if (record.failureClass && record.failureClass !== "none") {
    timeline.push({
      blockId: `final-${record.id}`,
      kind: "systemFinalFailure",
      occurredAt: record.occurredAt,
      title: "Final downstream response",
      subtitle: record.failureKind ?? "service_failure",
      status: record.status ?? "failed",
      detail: {
        downstreamStatusCode: record.downstreamStatusCode ?? 502,
        failureClass: record.failureClass,
        failureKind: record.failureKind ?? null,
        errorMessage: record.errorMessage ?? null,
      },
      responseBody: {
        available: true,
        bodyText: JSON.stringify(
          {
            error: record.errorMessage ?? "demo invocation failed",
            invoke_id: record.invokeId,
            code: record.failureKind ?? "service_failure",
            status: record.downstreamStatusCode ?? 502,
          },
          null,
          2,
        ),
      },
    });
  }

  return {
    hero: {
      recordId: record.id,
      invokeId: record.invokeId,
      promptCacheKey: record.promptCacheKey ?? null,
      routeMode: record.routeMode ?? null,
      endpoint: record.endpoint ?? null,
      requestModel,
      responseModel,
      finalStatus,
      failureClass: record.failureClass ?? null,
      downstreamStatusCode: record.downstreamStatusCode ?? null,
      upstreamAccountId: record.upstreamAccountId ?? null,
      upstreamAccountName: record.upstreamAccountName ?? null,
      totalDurationMs: record.tTotalMs ?? null,
      timelineAttemptCount: 1,
      poolAttemptCount: record.poolAttemptCount ?? 1,
      totalTokens: record.totalTokens ?? null,
      cost: record.cost ?? null,
      occurredAt: record.occurredAt,
    },
    timeline,
    reconstructed: false,
    partial: false,
    partialReason: null,
  };
}

function recordsToSuggestionCounts<T>(
  records: T[],
  selector: (record: T) => string | null | undefined,
) {
  const counts = new Map<string, number>();
  for (const record of records) {
    const value = selector(record);
    if (!value) continue;
    counts.set(value, (counts.get(value) ?? 0) + 1);
  }
  return Array.from(counts.entries()).sort(
    ([, leftCount], [, rightCount]) => rightCount - leftCount,
  );
}

function demoLongTermMetrics(tokens: number, calls: number, cost: number): LongTermMetrics {
  return {
    calls,
    tokens,
    tokenSamples: calls,
    cost,
    costSamples: calls,
    usageTimeMs: calls * 820,
    usageTimeSamples: calls,
    wallTimeMs: calls * 460,
    wallTimeSamples: calls,
    outputSpeedTokensPerSecond: 42.5,
    outputSpeedSamples: calls,
    firstByteMs: 312,
    firstByteSamples: calls,
    responseMs: 1_420,
    responseSamples: calls,
  };
}

function demoLongTermOverview(range: string) {
  const empty = demoModel.snapshot.scene === "empty";
  const length = range === "7d" ? 7 : range === "30d" ? 30 : range === "180d" ? 180 : 365;
  const endDate = new Date(demoNow());
  const endDateUtc = new Date(
    Date.UTC(endDate.getUTCFullYear(), endDate.getUTCMonth(), endDate.getUTCDate()),
  );
  const startDateUtc = new Date(endDateUtc);
  startDateUtc.setUTCDate(startDateUtc.getUTCDate() - length + 1);
  const days = Array.from({ length }, (_, index) => {
    const date = new Date(startDateUtc);
    date.setUTCDate(startDateUtc.getUTCDate() + index);
    return date.toISOString().slice(0, 10);
  });
  const models = empty
    ? []
    : [
        ["model:gpt-5.6-sol|reasoning:high", "gpt-5.6-sol", 128_000, 211, 9.8],
        ["model:gpt-5.6-sol|reasoning:medium", "gpt-5.6-sol", 86_000, 142, 6.4],
        ["model:gpt-5.6-sol|reasoning:low", "gpt-5.6-sol", 72_000, 117, 5.3],
        ["model:gpt-5.6-terra|reasoning:high", "gpt-5.6-terra", 61_000, 98, 4.7],
        ["model:gpt-5.6-luna|reasoning:medium", "gpt-5.6-luna", 48_000, 76, 3.8],
        ["model:o3|reasoning:high", "o3", 39_000, 63, 2.9],
        ["model:claude-sonnet-4|reasoning:medium", "claude-sonnet-4", 31_000, 51, 2.4],
        [
          "model:very-long-model-name-for-legend-wrapping|reasoning:minimal",
          "very-long-model-name-for-legend-wrapping",
          22_000,
          38,
          1.7,
        ],
      ].map(([seriesKey, displayName, tokens, calls, cost]) => ({
        seriesKey,
        displayName,
        reasoningEffort: String(seriesKey).split("|reasoning:")[1] ?? null,
        ...demoLongTermMetrics(Number(tokens), Number(calls), Number(cost)),
      }));
  const upstreams = empty
    ? []
    : [
        ["account:1", "Primary API key", 154_000, 260, 11.2],
        ["account:2", "Research API key", 61_000, 108, 5.1],
        ["account:3", "Staging API key", 52_000, 81, 4.1],
        ["account:4", "Batch workloads", 43_000, 70, 3.5],
        ["account:5", "Partner integration", 35_000, 61, 2.8],
        ["account:6", "Archive importer", 28_000, 49, 2.2],
        ["account:7", "Automation service", 25_000, 43, 2],
        ["other", "Other upstream account with a deliberately long display name", 22_000, 44, 1.8],
      ].map(([seriesKey, displayName, tokens, calls, cost]) => ({
        seriesKey,
        displayName,
        ...demoLongTermMetrics(Number(tokens), Number(calls), Number(cost)),
      }));
  const totals = models.reduce(
    (total, item) => ({
      tokens: total.tokens + (item.tokens ?? 0),
      calls: total.calls + item.calls,
      cost: total.cost + (item.cost ?? 0),
    }),
    { tokens: 0, calls: 0, cost: 0 },
  );
  return {
    status: empty ? "empty" : "ready",
    statisticsStartDate: "2026-01-01",
    processedRows: empty ? 0 : totals.calls,
    totalRows: empty ? 0 : totals.calls,
    timezone: "Asia/Shanghai",
    range,
    global: empty
      ? demoLongTermMetrics(0, 0, 0)
      : demoLongTermMetrics(totals.tokens, totals.calls, totals.cost),
    daily: empty
      ? []
      : days.map((date, index) => ({
          date,
          ...demoLongTermMetrics(
            Math.round(totals.tokens / length) + index * 400,
            Math.round(totals.calls / length) + (index % 7),
            Number((totals.cost / length + index / 100).toFixed(2)),
          ),
        })),
    models,
    upstreams,
  };
}

function demoLongTermSeries(url: URL) {
  const overview = demoLongTermOverview(url.searchParams.get("range") ?? "7d") as ReturnType<
    typeof demoLongTermOverview
  >;
  const dimension = url.searchParams.get("dimension") ?? "model";
  const keys = url.searchParams.getAll("key").filter(Boolean);
  const source = dimension === "upstream" ? overview.upstreams : overview.models;
  const sparseDates = new Set(
    overview.daily
      .filter(
        (_, index) =>
          index === 0 ||
          index === Math.floor(overview.daily.length * 0.2) ||
          index >= Math.floor(overview.daily.length * 0.72),
      )
      .map((point) => point.date),
  );
  const pointCount = Math.max(1, sparseDates.size);
  return {
    status: overview.status,
    statisticsStartDate: overview.statisticsStartDate,
    processedRows: overview.processedRows,
    totalRows: overview.totalRows,
    timezone: overview.timezone,
    range: overview.range,
    dimension,
    series: source
      .filter((item) => keys.includes(String(item.seriesKey)))
      .map((item) => ({
        seriesKey: String(item.seriesKey),
        displayName: item.displayName,
        reasoningEffort: "reasoningEffort" in item ? item.reasoningEffort : null,
        points: overview.daily
          .filter((point) => sparseDates.has(point.date))
          .map((point) => ({
            ...point,
            tokens: Math.round((item.tokens ?? 0) / pointCount),
            cost: Number(((item.cost ?? 0) / pointCount).toFixed(2)),
            calls: Math.round(item.calls / pointCount),
          })),
      })),
  };
}

function filterDemoInvocations(url: URL) {
  let records = invocations();
  const model = url.searchParams.get("model");
  const status = url.searchParams.get("status");
  const endpoint = url.searchParams.get("endpoint");
  const invokeId = url.searchParams.get("invokeId") ?? url.searchParams.get("requestId");
  const attemptId = url.searchParams.get("attemptId");
  const upstreamAccountId = Number(url.searchParams.get("upstreamAccountId"));
  const promptCacheKey = url.searchParams.get("promptCacheKey");
  const stickyKey = url.searchParams.get("stickyKey");
  const keyword = url.searchParams.get("keyword")?.toLowerCase();
  if (model) records = records.filter((record) => record.model === model);
  if (status) records = records.filter((record) => record.status === status);
  if (endpoint) records = records.filter((record) => record.endpoint === endpoint);
  if (invokeId) records = records.filter((record) => record.invokeId === invokeId);
  if (attemptId) {
    records = records.filter((record) =>
      poolAttempts(record.invokeId).some((attempt) => attempt.attemptId === attemptId),
    );
  }
  if (Number.isFinite(upstreamAccountId) && upstreamAccountId > 0)
    records = records.filter((record) => record.upstreamAccountId === upstreamAccountId);
  if (promptCacheKey)
    records = records.filter((record) => record.promptCacheKey === promptCacheKey);
  if (stickyKey) records = records.filter((record) => record.stickyKey === stickyKey);
  if (keyword)
    records = records.filter((record) => JSON.stringify(record).toLowerCase().includes(keyword));
  return records;
}

export async function handleDemoRequest(request: Request) {
  const url = new URL(request.url);
  const pathname = apiPathname(url.pathname);
  if (demoModel.snapshot.scene === "network-failure") return HttpResponse.error();

  if (pathname === "/api/version") return json({ backend: "demo", frontend: "demo" });
  if (pathname === "/api/stats" || pathname === "/api/stats/summary") return json(demoSummary());
  if (pathname === "/api/stats/long-term/overview") {
    return json(demoLongTermOverview(url.searchParams.get("range") ?? "7d"));
  }
  if (pathname === "/api/stats/long-term/series") return json(demoLongTermSeries(url));
  if (pathname === "/api/stats/dashboard-activity") {
    const includeAccounts = url.searchParams.get("includeAccounts") === "true";
    const includeRecent = url.searchParams.get("includeRecent") !== "false";
    if (includeAccounts && demoModel.snapshot.scene === "progressive-loading") {
      await new Promise((resolve) => setTimeout(resolve, 2_000));
    }
    const accounts = demoDashboardActivityAccounts().map((account) =>
      includeRecent ? account : { ...account, recentInvocations: [] },
    );
    const accountSummary = demoDashboardActivitySummary(accounts);
    return json({
      range: url.searchParams.get("range") ?? "today",
      snapshotId: 901,
      rangeStart: "2026-07-10T00:00:00Z",
      rangeEnd: demoNow(),
      rateWindow: {
        start: "2026-07-10T11:59:00Z",
        end: demoNow(),
        windowMinutes: 1,
        mode: "rolling_60s_live_mean",
      },
      summary: {
        stats: accountSummary,
        tokensPerMinute: includeAccounts
          ? accounts.reduce((total, account) => total + account.tokensPerMinute, 0)
          : 46_041,
        spendRate: includeAccounts
          ? Number(accounts.reduce((total, account) => total + account.spendRate, 0).toFixed(2))
          : 19.41,
        currentFirstTokenAvgMs: 1280,
        currentAvgTotalMs: 6920,
        modelPerformance: demoModelPerformanceForModels([0, 1, 2]),
      },
      accounts: includeAccounts ? accounts : undefined,
    });
  }
  if (pathname === "/api/stats/dashboard-activity/recent") {
    if (demoModel.snapshot.scene === "progressive-loading") {
      await new Promise((resolve) => setTimeout(resolve, 3_000));
    }
    return json({
      rangeStart: url.searchParams.get("rangeStart") ?? "2026-07-10T00:00:00Z",
      rangeEnd: url.searchParams.get("rangeEnd") ?? demoNow(),
      snapshotId: Number(url.searchParams.get("snapshotId") ?? 901),
      accounts: demoDashboardActivityAccounts().map((account) => ({
        accountKey: account.accountKey,
        recentInvocations: account.recentInvocations,
      })),
    });
  }
  if (pathname === "/api/stats/upstream-account-activity") {
    return json({
      range: url.searchParams.get("range") ?? "today",
      rangeStart: "2026-07-10T00:00:00Z",
      rangeEnd: demoNow(),
      accounts: demoDashboardActivityAccounts(),
    });
  }
  if (pathname === "/api/stats/timeseries") return json(timeseries());
  if (pathname === "/api/stats/parallel-work")
    return json(parallelWork(), { headers: { ETag: "demo-parallel-work" } });
  if (pathname === "/api/stats/errors")
    return json({
      rangeStart: "2026-07-10T00:00:00Z",
      rangeEnd: demoNow(),
      items:
        demoModel.snapshot.scene === "empty"
          ? []
          : [
              { reason: "upstream_timeout", count: 24 },
              { reason: "rate_limited", count: 11 },
            ],
    });
  if (pathname === "/api/stats/failures/summary")
    return json({
      rangeStart: "2026-07-10T00:00:00Z",
      rangeEnd: demoNow(),
      totalFailures: 35,
      serviceFailureCount: 24,
      clientFailureCount: 7,
      clientAbortCount: 4,
      actionableFailureCount: 31,
      actionableFailureRate: 0.88,
    });
  if (pathname === "/api/stats/forward-proxy") return json(forwardProxyLive());
  if (pathname === "/api/stats/forward-proxy/timeseries") {
    const live = forwardProxyLive();
    return json({
      rangeStart: live.rangeStart,
      rangeEnd: live.rangeEnd,
      bucketSeconds: 3600,
      effectiveBucket: "1h",
      availableBuckets: ["1h", "6h", "1d"],
      nodes: live.nodes.map((node) => ({
        key: node.key,
        source: node.source,
        displayName: node.displayName,
        endpointUrl: node.endpointUrl,
        weight: node.weight,
        penalized: node.penalized,
        buckets: node.last24h,
        weightBuckets: node.weight24h,
      })),
    });
  }
  if (pathname === "/api/stats/prompt-cache-conversations") return json(promptCacheConversations());
  if (pathname.startsWith("/api/stats/prompt-cache-conversation-binding-events/")) {
    const promptCacheKey = decodeURIComponent(pathname.split("/").at(-1) ?? "");
    const items = [
      {
        id: 9303,
        promptCacheKey,
        action: "stickyMutationSuppressed",
        origin: "systemAuto",
        infoTypes: ["routing"],
        occurredAt: "2026-08-02T09:41:08.000Z",
        headline: "Sticky mutation suppressed",
        changedFields: [],
        bindingBefore: null,
        bindingAfter: null,
        stickyBefore: { upstreamAccountId: 22, upstreamAccountName: "demo-primary@monitor.test" },
        stickyAfter: { upstreamAccountId: 22, upstreamAccountName: "demo-primary@monitor.test" },
        invokeId: "demo-concurrent-late",
        routingContext: {
          reasonCode: "staleConcurrentCompletion",
          routingSource: "freshAssignment",
          routingSelectionAudit: {
            selectedAccountId: 2890,
            selectedAccountName: "dzw",
            eligibleCandidateCount: 1,
            winnerReasonCode: "onlyEligibleCandidate",
            comparedAccountId: null,
            comparedAccountName: null,
            excludedCandidates: [
              { accountId: 2805, accountName: "CIII", reasonCode: "modelNotAllowed" },
            ],
          },
          httpStatus: null,
          triggerAttemptId: "DEMO-LATE-2",
          causingAttemptId: null,
          causingHttpStatus: null,
        },
      },
      {
        id: 9302,
        promptCacheKey,
        action: "stickyTargetChanged",
        origin: "systemAuto",
        infoTypes: ["routing"],
        occurredAt: "2026-08-02T09:41:05.000Z",
        headline: "Sticky target changed",
        changedFields: ["stickyTarget"],
        bindingBefore: null,
        bindingAfter: null,
        stickyBefore: null,
        stickyAfter: { upstreamAccountId: 22, upstreamAccountName: "demo-primary@monitor.test" },
        invokeId: "demo-fresh-success",
        routingContext: {
          reasonCode: "freshAssignmentAfterFailure",
          routingSource: "freshAssignment",
          routingSelectionAudit: {
            selectedAccountId: 2890,
            selectedAccountName: "dzw",
            eligibleCandidateCount: 1,
            winnerReasonCode: "onlyEligibleCandidate",
            comparedAccountId: null,
            comparedAccountName: null,
            excludedCandidates: [
              { accountId: 2805, accountName: "CIII", reasonCode: "modelNotAllowed" },
            ],
          },
          httpStatus: null,
          triggerAttemptId: "DEMO-SUCCESS-1",
          causingAttemptId: "DEMO-FAILED-0",
          causingHttpStatus: 429,
        },
      },
      {
        id: 9301,
        promptCacheKey,
        action: "stickyTargetCleared",
        origin: "systemAuto",
        infoTypes: ["routing"],
        occurredAt: "2026-08-02T09:41:01.000Z",
        headline: "Sticky target cleared",
        changedFields: ["stickyTarget"],
        bindingBefore: null,
        bindingAfter: null,
        stickyBefore: { upstreamAccountId: 21, upstreamAccountName: "demo-fallback@monitor.test" },
        stickyAfter: null,
        invokeId: null,
      },
    ];
    const infoType = url.searchParams.get("infoType");
    const filtered = infoType ? items.filter((item) => item.infoTypes.includes(infoType)) : items;
    return json({ items: filtered, total: filtered.length, page: 1, pageSize: 20 });
  }
  if (pathname.startsWith("/api/stats/prompt-cache-conversation-bindings/")) {
    const promptCacheKey = decodeURIComponent(pathname.split("/").at(-1) ?? "");
    const conversation = promptCacheConversations().conversations.find(
      (item) => item.promptCacheKey === promptCacheKey,
    );
    const owner = conversation?.encryptedOwnerAccountId ?? null;
    const account = owner == null ? null : demoAccounts().find((item) => item.id === owner);
    return json({
      promptCacheKey,
      bindingKind: account ? "upstreamAccount" : "none",
      groupName: account?.groupName ?? null,
      upstreamAccountId: owner,
      upstreamAccountName: account?.displayName ?? null,
      hasEncryptedSessionOwner: account != null,
      encryptedOwnerAccountId: owner,
      encryptedOwnerAccountName: account?.displayName ?? null,
      encryptedOwnerGroupName: account?.groupName ?? null,
      timeouts: {
        responsesFirstByteTimeoutSecs: 30,
        compactFirstByteTimeoutSecs: 45,
        imageFirstByteTimeoutSecs: 300,
        responsesStreamTimeoutSecs: 300,
        compactStreamTimeoutSecs: 420,
      },
      timeoutFieldSources: {
        responsesFirstByteTimeoutSecs: "root",
        compactFirstByteTimeoutSecs: "root",
        imageFirstByteTimeoutSecs: "root",
        responsesStreamTimeoutSecs: "root",
        compactStreamTimeoutSecs: "root",
      },
      allowSwitchUpstream: true,
      fastModeRewriteMode: "keep_original",
      imageToolRewriteMode: "keep_original",
      availableModels: ["gpt-5.6-sol", "gpt-5.6-terra"],
      forwardProxyKey: account?.currentForwardProxyKey ?? null,
      forwardProxyKeys: account?.boundProxyKeys ?? [],
      policyFieldSources: {
        allowSwitchUpstream: "root",
        fastModeRewriteMode: "root",
        imageToolRewriteMode: "root",
        availableModels: "root",
        forwardProxyKey: "account",
      },
      updatedAt: "2026-07-10T09:20:00Z",
    });
  }
  if (pathname === "/api/quota/latest")
    return json({
      capturedAt: demoNow(),
      accounts: demoAccounts().map((account) => ({
        accountId: account.id,
        displayName: account.displayName,
        primaryWindow: account.primaryWindow,
        secondaryWindow: account.secondaryWindow,
      })),
    });

  if (pathname === "/api/invocations") {
    const records = filterDemoInvocations(url);
    const pageSize = Number(
      url.searchParams.get("pageSize") ?? url.searchParams.get("limit") ?? 50,
    );
    const page = Number(url.searchParams.get("page") ?? 1);
    const start = Math.max(0, (page - 1) * pageSize);
    return json({
      snapshotId: 901,
      total: records.length,
      page,
      pageSize,
      records: records.slice(start, start + pageSize),
    });
  }
  if (pathname === "/api/invocations/summary") {
    return json({
      snapshotId: 901,
      newRecordsCount: 0,
      ...demoInvocationSummary(filterDemoInvocations(url)),
    });
  }
  if (pathname === "/api/invocations/new-count")
    return json({ snapshotId: 901, newRecordsCount: 0 });
  if (pathname === "/api/invocations/suggestions") {
    const bucket = (
      selector: (record: ReturnType<typeof invocations>[number]) => string | null | undefined,
    ) => ({
      items: Array.from(recordsToSuggestionCounts(invocations(), selector), ([value, count]) => ({
        value,
        count,
      })),
      hasMore: false,
    });
    return json({
      model: bucket((record) => record.model),
      endpoint: bucket((record) => record.endpoint),
      failureKind: bucket((record) => record.failureKind),
      promptCacheKey: bucket((record) => record.promptCacheKey),
      requesterIp: bucket((record) => record.requesterIp),
    });
  }
  if (pathname.endsWith("/detail")) {
    const id = Number(pathname.split("/").at(-2));
    const record = invocations().find((item) => item.id === id);
    return json({
      id,
      abnormalResponseBody:
        record?.failureClass && record.failureClass !== "none"
          ? {
              available: true,
              previewText: record.errorMessage ?? "Simulated non-success response.",
              hasMore: false,
              unavailableReason: null,
            }
          : {
              available: false,
              previewText: null,
              hasMore: false,
              unavailableReason: "Only non-success invocations retain a demo abnormal preview.",
            },
    });
  }
  if (pathname.endsWith("/workflow-detail")) {
    const id = Number(pathname.split("/").at(-2));
    const record = invocations().find((item) => item.id === id);
    if (!record) return json({ error: `Demo invocation ${id} not found.` }, { status: 404 });
    return json(buildDemoInvocationWorkflowDetail(record));
  }
  if (pathname.endsWith("/request-body")) {
    const id = Number(pathname.split("/").at(-2));
    const record = invocations().find((item) => item.id === id);
    if (!record) return json({ error: `Demo invocation ${id} not found.` }, { status: 404 });
    if (id === 9002) {
      return json({
        available: false,
        unavailableReason: "missing_body",
      });
    }
    return json({
      available: true,
      bodyText: JSON.stringify(
        {
          model: record.requestModel ?? record.model,
          endpoint: record.endpoint,
          invoke_id: record.invokeId,
          demo: true,
        },
        null,
        2,
      ),
      headers: {
        userAgent: "monitor-ui/1.0",
        xForwardedFor: record.requesterIp ?? "203.0.113.24",
      },
      routing: {
        routeMode: record.routeMode ?? "pool",
        promptCacheKey: record.promptCacheKey ?? null,
        proxyDisplayName: record.proxyDisplayName ?? null,
      },
      bodySize: 412,
      bodyTruncated: false,
      detailLevel: "full",
      captureSource: "raw_file",
    });
  }
  const attemptResponseBodyMatch = pathname.match(
    /^\/api\/invocations\/(\d+)\/attempts\/([^/]+)\/response-body$/,
  );
  if (attemptResponseBodyMatch) {
    const id = Number(attemptResponseBodyMatch[1]);
    const attemptId = decodeURIComponent(attemptResponseBodyMatch[2] ?? "");
    const record = invocations().find((item) => item.id === id);
    const attempt = record
      ? poolAttempts(record.invokeId).find((item) => item.attemptId === attemptId)
      : null;
    if (!record || !attempt) {
      return json({ error: `Demo attempt ${attemptId} not found.` }, { status: 404 });
    }
    return json({
      available: true,
      bodyText: DEMO_INVOCATION_RESPONSE_BODY_TEXT,
      headers: {
        contentEncoding: "identity",
        upstreamRequestId: attempt.upstreamRequestId ?? `req_demo_${record.id}`,
        cvmInvokeId: record.invokeId,
      },
      routing: {
        forwardedChunkCount: 12,
      },
      bodySize: DEMO_INVOCATION_RESPONSE_BODY_SIZE,
      bodyTruncated: false,
      detailLevel: "full",
      captureSource: "attempt_raw_file",
      availableAtAttemptLevel: true,
    });
  }
  if (pathname.endsWith("/response-body")) {
    const id = Number(pathname.split("/").at(-2));
    const record = invocations().find((item) => item.id === id);
    if (id === 9002) {
      return json({
        available: true,
        bodyText: DEMO_INVOCATION_RESPONSE_BODY_TEXT,
        headers: {
          contentEncoding: "identity",
          upstreamRequestId: `req_demo_${id}`,
          cvmInvokeId: record?.invokeId ?? null,
        },
        routing: {
          forwardedChunkCount: 12,
        },
        bodySize: DEMO_INVOCATION_RESPONSE_BODY_SIZE,
        bodyTruncated: false,
        detailLevel: "full",
        captureSource: "raw_file",
      });
    }
    const isFailure = record?.failureClass && record.failureClass !== "none";
    return json(
      isFailure
        ? {
            available: true,
            bodyText:
              id === 9002
                ? [
                    ": keepalive",
                    "",
                    "event: response.output_item.done",
                    `data: ${JSON.stringify({
                      type: "response.output_item.done",
                      output_index: 0,
                      item: {
                        id: "msg_demo_9002",
                        type: "message",
                        content: [
                          {
                            type: "output_text",
                            text: "A long streamed response remains contained inside the payload inspector without widening the invocation drawer.",
                          },
                        ],
                      },
                    })}`,
                    "",
                    "event: response.failed",
                    `data: ${JSON.stringify({
                      type: "response.failed",
                      error: {
                        message: record?.errorMessage,
                        type: record?.failureKind,
                        request_id: `req_demo_${id}`,
                      },
                    })}`,
                  ].join("\n")
                : JSON.stringify(
                    {
                      error: {
                        message: record?.errorMessage,
                        type: record?.failureKind,
                        request_id: `req_demo_${id}`,
                      },
                    },
                    null,
                    2,
                  ),
            unavailableReason: null,
          }
        : {
            available: true,
            bodyText: JSON.stringify(
              {
                id: `resp_demo_${id}`,
                object: "response",
                model: record?.model,
                status: record?.status,
                output: [
                  {
                    type: "message",
                    content: [
                      {
                        type: "output_text",
                        text: "Demo response body retained locally for visual inspection.",
                      },
                    ],
                  },
                ],
              },
              null,
              2,
            ),
            unavailableReason: null,
          },
    );
  }
  if (pathname.endsWith("/pool-attempts"))
    return json(poolAttempts(decodeURIComponent(pathname.split("/").at(-2) ?? "")));

  if (pathname === "/api/settings" && request.method === "GET")
    return json(demoModel.snapshot.settings);
  if (pathname === "/api/settings/external-api-keys" && request.method === "GET")
    return json({ items: demoModel.snapshot.externalApiKeys });
  if (pathname === "/api/settings/external-api-keys" && request.method === "POST")
    return json(demoModel.createExternalApiKey(), { status: 201 });
  if (/^\/api\/settings\/external-api-keys\/\d+\/(rotate|disable)$/.test(pathname)) {
    const id = Number(pathname.split("/").at(-2));
    const key =
      demoModel.snapshot.externalApiKeys.find((item) => item.id === id) ??
      demoModel.snapshot.externalApiKeys[0];
    const action = pathname.endsWith("/disable") ? "disable" : "rotate";
    demoModel.record(`模拟 ${action === "disable" ? "禁用" : "轮换"}外部 API Key`);
    return json(
      action === "disable"
        ? { key: { ...key, status: "disabled", updatedAt: demoNow() } }
        : { key: { ...key, updatedAt: demoNow() }, secret: "demo-rotated-key-not-valid" },
    );
  }
  if (pathname === "/api/system/status") return json(systemStatus());
  if (pathname === "/api/system/tasks") {
    let items = systemTasks();
    const taskKind = url.searchParams.get("taskKind");
    const status = url.searchParams.get("status");
    if (taskKind) items = items.filter((item) => item.taskKind.includes(taskKind));
    if (status) items = items.filter((item) => item.status === status);
    const pageSize = Number(
      url.searchParams.get("pageSize") ?? url.searchParams.get("limit") ?? 20,
    );
    const page = Number(url.searchParams.get("page") ?? 1);
    return json({
      total: items.length,
      page,
      pageSize,
      items: items.slice((page - 1) * pageSize, page * pageSize),
    });
  }

  if (pathname === "/api/pool/upstream-accounts" && request.method === "GET")
    return json(accountList());
  if (pathname === "/api/pool/upstream-account-events") {
    let items = accountEvents();
    const account = url.searchParams.get("account")?.toLowerCase();
    const group = url.searchParams.get("group")?.toLowerCase();
    const proxyKey = url.searchParams.get("proxyKey");
    const result = url.searchParams.get("result");
    if (account)
      items = items.filter((item) => item.accountDisplayName?.toLowerCase().includes(account));
    if (group) items = items.filter((item) => item.accountGroupName?.toLowerCase().includes(group));
    if (proxyKey) items = items.filter((item) => item.forwardProxyKey === proxyKey);
    if (result) items = items.filter((item) => item.result === result);
    const pageSize = Number(url.searchParams.get("pageSize") ?? 20);
    const page = Number(url.searchParams.get("page") ?? 1);
    return json({
      total: items.length,
      page,
      pageSize,
      items: items.slice((page - 1) * pageSize, page * pageSize),
    });
  }
  if (pathname === "/api/pool/upstream-accounts/window-usage")
    return json({
      items: demoAccounts().map((account) => ({
        accountId: account.id,
        primaryActualUsage: {
          requestCount: 1080 + account.id,
          totalTokens: 842_000 + account.id * 100,
          totalCost: 12.4,
          inputTokens: 320_000,
          outputTokens: 182_000,
          cacheInputTokens: 340_000,
        },
        secondaryActualUsage: account.secondaryWindow
          ? {
              requestCount: 142,
              totalTokens: 98_000,
              totalCost: 1.62,
              inputTokens: 42_000,
              outputTokens: 21_000,
              cacheInputTokens: 35_000,
            }
          : null,
      })),
    });
  if (pathname === "/api/pool/forward-proxy-binding-nodes") return json(forwardProxyBindingNodes());
  if (pathname === "/api/pool/tags" && request.method === "GET")
    return json({
      writesEnabled: true,
      items: [
        {
          id: 1,
          name: "primary",
          accountCount: 3,
          groupCount: 1,
          updatedAt: demoNow(),
          routingRule: { allowCutIn: true, allowCutOut: true, priorityTier: "primary" },
        },
        {
          id: 2,
          name: "fallback",
          accountCount: 2,
          groupCount: 1,
          updatedAt: demoNow(),
          routingRule: { allowCutIn: false, allowCutOut: true, priorityTier: "fallback" },
        },
        {
          id: 3,
          name: "image",
          accountCount: 2,
          groupCount: 2,
          updatedAt: demoNow(),
          routingRule: { allowCutIn: true, allowCutOut: true, priorityTier: "normal" },
        },
        {
          id: 4,
          name: "research",
          accountCount: 2,
          groupCount: 1,
          updatedAt: demoNow(),
          routingRule: { allowCutIn: true, allowCutOut: true, priorityTier: "normal" },
        },
        {
          id: 5,
          name: "sandbox",
          accountCount: 1,
          groupCount: 0,
          updatedAt: demoNow(),
          routingRule: { allowCutIn: false, allowCutOut: false, priorityTier: "no_new" },
        },
      ],
    });
  if (pathname === "/api/pool/routing-settings")
    return json({
      writesEnabled: true,
      apiKeyConfigured: true,
      maskedApiKey: "cvm_pool••••••",
      maintenance: {
        primarySyncIntervalSecs: 300,
        secondarySyncIntervalSecs: 1800,
        priorityAvailableAccountCap: 100,
      },
      timeouts: {
        responsesFirstByteTimeoutSecs: 30,
        compactFirstByteTimeoutSecs: 45,
        imageFirstByteTimeoutSecs: 300,
        responsesStreamTimeoutSecs: 300,
        compactStreamTimeoutSecs: 420,
      },
    });
  if (pathname === "/api/pool/model-routing-live" && request.method === "GET") {
    return json(
      demoModelRoutingLive({
        window: url.searchParams.get("window"),
        model: url.searchParams.get("model"),
        state: url.searchParams.get("state"),
        limit: url.searchParams.get("limit"),
      }),
    );
  }
  if (pathname.includes("/sticky-keys"))
    return json({
      rangeStart: "2026-07-10T00:00:00Z",
      rangeEnd: demoNow(),
      selectionMode: "count",
      selectedLimit: 50,
      selectedActivityHours: null,
      implicitFilter: { kind: null, filteredCount: 0 },
      totalMatched: 3,
      conversations: promptCacheConversations().conversations.slice(0, 3),
      hasMore: false,
      nextCursor: null,
    });
  if (
    /^\/api\/pool\/upstream-accounts\/\d+\/model-routing$/.test(pathname) &&
    request.method === "GET"
  ) {
    const accountId = Number(pathname.split("/").at(-2));
    const account = demoAccounts().find((item) => item.id === accountId) ?? demoAccounts()[0];
    return json(account.kind === "api_key_codex" ? demoModelRoutingStates(accountId) : []);
  }
  if (
    /^\/api\/pool\/upstream-accounts\/\d+\/model-routing-events$/.test(pathname) &&
    request.method === "GET"
  ) {
    const accountId = Number(pathname.split("/").at(-2));
    const account = demoAccounts().find((item) => item.id === accountId);
    const model = url.searchParams.get("model")?.trim();
    if (account?.kind !== "api_key_codex" || !model) {
      return json(
        { error: "Model routing history is unavailable for this account." },
        { status: 404 },
      );
    }

    const items = demoModelRoutingTimeline(accountId, model);
    const cursor = url.searchParams.get("cursor");
    if (cursor === "demo-model-routing-page-2") {
      return json({ items: items.slice(2), nextCursor: null });
    }
    return json({
      items: items.slice(0, 2),
      nextCursor: items.length > 2 ? "demo-model-routing-page-2" : null,
    });
  }
  if (/^\/api\/pool\/upstream-accounts\/\d+$/.test(pathname) && request.method === "GET") {
    const accountId = Number(pathname.split("/").at(-1));
    const account = demoAccounts().find((item) => item.id === accountId) ?? demoAccounts()[0];
    return json({
      ...account,
      note: `Demo fixture for ${account.displayName}.`,
      upstreamBaseUrl: "https://api.openai.com",
      chatgptUserId: account.chatgptAccountId ? `user-${account.id}` : null,
      verifiedEmail: account.email,
      lastRefreshedAt: account.lastSyncedAt,
      history: Array.from({ length: 8 }, (_, index) => ({
        capturedAt: `2026-07-${String(index + 3).padStart(2, "0")}T08:00:00Z`,
        primaryUsedPercent: Math.min(94, (account.primaryWindow?.usedPercent ?? 0) + index * 3),
        secondaryUsedPercent: account.secondaryWindow
          ? Math.min(94, account.secondaryWindow.usedPercent + index * 2)
          : null,
        creditsBalance: account.credits?.balance ?? null,
      })),
      recentActions: accountEvents()
        .filter((event) => event.accountDisplayName === account.displayName)
        .slice(0, 4),
      modelRoutingStates: account.kind === "api_key_codex" ? demoModelRoutingStates(accountId) : [],
    });
  }

  if (request.method !== "GET" && request.method !== "HEAD") {
    let body: unknown = null;
    try {
      body = await request.clone().json();
    } catch {
      /* no JSON body */
    }
    if (pathname === "/api/settings" || pathname.startsWith("/api/settings/"))
      return json(demoModel.updateSettings(pathname, body));
    if (pathname === "/api/pool/upstream-accounts")
      return json(demoModel.createAccount(), { status: 201 });
    if (/^\/api\/pool\/upstream-accounts\/\d+\/model-routing\/reset$/.test(pathname)) {
      const accountId = Number(pathname.split("/").at(-3));
      const model =
        body && typeof body === "object" && typeof (body as { model?: unknown }).model === "string"
          ? (body as { model: string }).model
          : "gpt-5.4-mini";
      demoResetModelRoutes.add(`${accountId}:${model}`);
      demoModel.record(`模拟恢复账号 ${accountId} 的模型 ${model}`);
      return json({
        model,
        state: "available",
        priority: "normal",
        failureCount: 0,
        changedAt: demoNow(),
        lastSeenAt: demoNow(),
        cooldownUntil: null,
      });
    }
    demoModel.record(`模拟 ${request.method} ${pathname.split("/").slice(-1)[0]}`);
    return json({ ok: true, simulated: true, updatedAt: demoNow() });
  }

  return json({ error: `Unhandled demo API route: ${pathname}` }, { status: 501 });
}

export const apiHandlers = [
  http.get("/favicon.ico", () => new HttpResponse(null, { status: 204 })),
  http.all(/\/api\/.*/, ({ request }) => handleDemoRequest(request)),
];
