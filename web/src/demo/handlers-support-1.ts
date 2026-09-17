import { HttpResponse, type JsonBodyType } from "msw";
import type { ModelRoutingTimelineRecord } from "../lib/api";
import { isFiniteNonNegativeMilliseconds } from "../lib/invocationTiming";
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

export type DemoAccount = {
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

function simulatedAccountDisplayName(accountId: number) {
  const displayName = demoAccounts()
    .find((account) => account.id === accountId)
    ?.displayName.trim();
  return displayName || `API Key #${accountId}`;
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
        cacheConcurrencyLimit: coolingDown ? 1 : null,
        cacheRecoveryLimit: coolingDown ? 4 : null,
        cacheUsageMissingSince: coolingDown ? demoModelRouteTimestamp(fixture.minutesAgo) : null,
        cacheUsageMissingReason: coolingDown ? "missing_cache_input_tokens" : null,
      };
    });
}

type DemoRouteFixtureContext = {
  fixture: DemoModelRouteFixture;
  accountId: number;
  accountDisplayName: string;
  model: string;
  occurredAt: (offsetMinutes?: number) => string;
  invokeId: string;
  terminalLatencyMs: number | null;
  retryLatencyMs: number | null;
  cooldownUntil: string;
  selectionAudit: NonNullable<ModelRoutingTimelineRecord["routingSelectionAudit"]>;
  firstFailure: ModelRoutingTimelineRecord;
  cooldownEvent: ModelRoutingTimelineRecord;
};

function buildDemoRouteFixtureContext(fixture: DemoModelRouteFixture): DemoRouteFixtureContext {
  const { accountId, model } = fixture;
  const accountDisplayName = simulatedAccountDisplayName(accountId);
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
    modelRoutePriorityBefore: "normal",
    modelRoutePriorityAfter: "demoted",
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
    status: "cooling_down",
    action: "model_route_cooldown",
    source: "call",
    reasonCode: "upstream_http_5xx",
    modelRouteStateBefore: "degraded",
    modelRouteStateAfter: "cooling_down",
    modelRoutePriorityBefore: "demoted",
    modelRoutePriorityAfter: "excluded",
    modelRouteFailureCount: 2,
    modelRouteCooldownUntil: cooldownUntil,
  };
  return {
    fixture,
    accountId,
    accountDisplayName,
    model,
    occurredAt,
    invokeId,
    terminalLatencyMs,
    retryLatencyMs,
    cooldownUntil,
    selectionAudit,
    firstFailure,
    cooldownEvent,
  };
}

function demoAvailableRouteTimeline(
  context: DemoRouteFixtureContext,
): ModelRoutingTimelineRecord[] {
  const {
    fixture,
    accountId,
    accountDisplayName,
    model,
    occurredAt,
    invokeId,
    terminalLatencyMs,
    selectionAudit,
  } = context;
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
      modelRoutePriorityBefore: "normal",
      modelRoutePriorityAfter: "normal",
      routingSelectionAudit: selectionAudit,
    },
  ];
}

function demoRecoveredRouteTimeline(
  context: DemoRouteFixtureContext,
): ModelRoutingTimelineRecord[] {
  const {
    fixture,
    accountId,
    accountDisplayName,
    model,
    occurredAt,
    invokeId,
    terminalLatencyMs,
    selectionAudit,
    cooldownEvent,
    firstFailure,
  } = context;
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
      modelRoutePriorityBefore: "excluded",
      modelRoutePriorityAfter: "normal",
      routingSelectionAudit: selectionAudit,
    },
    { ...cooldownEvent, occurredAt: occurredAt(1), modelRouteCooldownUntil: occurredAt(0.5) },
    firstFailure,
  ];
}

function demoDegradedRouteTimeline(context: DemoRouteFixtureContext): ModelRoutingTimelineRecord[] {
  const { fixture, accountId, accountDisplayName, model, occurredAt, firstFailure } = context;
  return [
    {
      id: `event:${fixture.invocationId}:degraded`,
      kind: "event",
      occurredAt: occurredAt(),
      accountId,
      accountDisplayName,
      model,
      status: "degraded",
      action: "model_route_degraded",
      source: "call",
      reasonCode: "upstream_http_5xx",
      modelRouteStateBefore: "available",
      modelRouteStateAfter: "degraded",
      modelRoutePriorityBefore: "normal",
      modelRoutePriorityAfter: "demoted",
      modelRouteFailureCount: 1,
    },
    firstFailure,
  ];
}

function demoCoolingRouteTimeline(context: DemoRouteFixtureContext): ModelRoutingTimelineRecord[] {
  const {
    fixture,
    accountId,
    accountDisplayName,
    model,
    occurredAt,
    invokeId,
    terminalLatencyMs,
    cooldownUntil,
    selectionAudit,
    cooldownEvent,
    firstFailure,
  } = context;
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
      modelRoutePriorityBefore: "demoted",
      modelRoutePriorityAfter: "excluded",
      modelRouteFailureCount: 2,
      modelRouteCooldownUntil: cooldownUntil,
      routingSelectionAudit: selectionAudit,
    },
    cooldownEvent,
    firstFailure,
  ];
}

function demoModelRouteFixtureTimeline(
  fixture: DemoModelRouteFixture,
): ModelRoutingTimelineRecord[] {
  const context = buildDemoRouteFixtureContext(fixture);
  if (fixture.flow === "available") return demoAvailableRouteTimeline(context);
  if (fixture.flow === "recovered") return demoRecoveredRouteTimeline(context);
  if (fixture.flow === "degraded") return demoDegradedRouteTimeline(context);
  return demoCoolingRouteTimeline(context);
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

function publicModelRoutingRecord(record: ModelRoutingTimelineRecord) {
  return record;
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
      accountDisplayName: account.displayName,
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
      .slice(0, limit)
      .map(publicModelRoutingRecord),
  };
}

function demoForwardProxyNodes(): DemoProxyNode[] {
  return (demoModel.snapshot.settings.forwardProxy as { nodes: DemoProxyNode[] }).nodes;
}

function json(payload: unknown, init?: ResponseInit) {
  return HttpResponse.json(payload as JsonBodyType, init);
}
function demoAttemptPhase(
  status: string | null | undefined,
  firstTokenMs: number | null | undefined,
) {
  if (status !== "running") return "completed";
  return isFiniteNonNegativeMilliseconds(firstTokenMs) ? "responding" : "requesting";
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
function demoSummary() {
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

const DEMO_INVOCATION_ROWS = [
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
    210,
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
    "success",
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
    "success",
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
] as const;

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
  const rows = [...DEMO_INVOCATION_ROWS, ...routingRows];

  return rows
    .map((row) => buildDemoInvocationRecord(row, accounts, proxyName, attention))
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

export {
  apiPathname,
  computeDemoParallelism,
  DEMO_INVOCATION_REQUEST_BODY_SIZE,
  DEMO_INVOCATION_REQUEST_BODY_TRANSMITTED_BYTES,
  DEMO_INVOCATION_RESPONSE_BODY_SIZE,
  DEMO_INVOCATION_RESPONSE_BODY_TEXT,
  DEMO_MODEL_PERFORMANCE_MODELS,
  DEMO_MODEL_PERFORMANCE_PAIR_OVERLAPS_MS,
  DEMO_USAGE_BREAKDOWN,
  type DemoModelRoutingLiveQuery,
  type DemoModelRoutingLiveState,
  type DemoProxyNode,
  demoAccounts,
  demoAttemptPhase,
  demoAverage,
  demoDashboardActivityAccounts,
  demoForwardProxyNodes,
  demoInvocationSummary,
  demoModelPerformanceForModels,
  demoModelRouteFixtureTimeline,
  demoModelRouteTimestamp,
  demoModelRoutingLive,
  demoModelRoutingLiveTimeline,
  demoModelRoutingStates,
  demoModelRoutingTimeline,
  demoPercentile,
  demoResetModelRoutes,
  demoSummary,
  demoUsageBreakdownForModels,
  demoUsageBreakdownFromInvocations,
  formatDemoAttemptId,
  invocations,
  json,
  latestDemoRouteFixture,
  publicModelRoutingRecord,
  simulatedAccountDisplayName,
};

type DemoInvocationRow = readonly [
  number,
  number,
  string,
  string,
  string,
  string,
  string,
  number,
  number,
  number,
  number,
  number,
  number | null,
  number | null,
  string,
];

function demoInvocationFailureDetails(effectiveStatus: string, isNoCandidate: boolean) {
  const isFailure = ["http_502", "http_401", "http_429", "http_503", "client_cancelled"].includes(
    effectiveStatus,
  );
  const failureClass =
    effectiveStatus === "client_cancelled"
      ? "client_abort"
      : isFailure
        ? "service_failure"
        : "none";
  const failureKind = isNoCandidate
    ? "pool_no_available_account"
    : effectiveStatus === "http_429"
      ? "rate_limited"
      : effectiveStatus === "client_cancelled"
        ? "downstream_cancelled"
        : effectiveStatus === "http_401"
          ? "upstream_auth_rejected"
          : effectiveStatus === "http_502"
            ? "upstream_timeout"
            : null;
  const errorMessage =
    failureKind === "pool_no_available_account"
      ? "No healthy pool account is available for the requested model."
      : failureKind === "upstream_timeout"
        ? "Simulated upstream timeout after 1.8 seconds."
        : failureKind === "upstream_auth_rejected"
          ? "Simulated upstream authorization rejection."
          : failureKind === "rate_limited"
            ? "Simulated upstream rate limit."
            : failureKind === "downstream_cancelled"
              ? "Simulated client cancellation."
              : null;
  const downstreamStatusCode =
    effectiveStatus === "http_503"
      ? 503
      : effectiveStatus === "http_502"
        ? 502
        : effectiveStatus === "http_401"
          ? 401
          : effectiveStatus === "http_429"
            ? 429
            : null;
  return { isFailure, failureClass, failureKind, errorMessage, downstreamStatusCode };
}

function demoInvocationTiming(
  id: number,
  endpoint: string,
  ttfb: number | null,
  total: number | null,
) {
  return {
    tUpstreamConnectMs: ttfb == null ? null : Math.max(24, Math.round(ttfb * 0.24)),
    tUpstreamTtfbMs: ttfb,
    firstTokenMs:
      ttfb == null || !["/v1/responses", "/v1/chat/completions"].includes(endpoint)
        ? null
        : total == null
          ? ttfb + 420 + (id % 5) * 75
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
  };
}

function demoInvocationRouteFields(
  id: number,
  isNoCandidate: boolean,
  effectiveStatus: string,
  isFailure: boolean,
  failureKind: string | null,
  account: DemoAccount | undefined,
  promptCacheKey: string,
) {
  return {
    requesterIp: id % 2 === 0 ? "203.0.113.24" : "198.51.100.86",
    promptCacheKey,
    stickyKey: promptCacheKey,
    routeMode: isNoCandidate ? "pool" : account?.groupName === "standby" ? "fallback" : "pool",
    poolAttemptCount: isNoCandidate
      ? 0
      : effectiveStatus === "http_429"
        ? 2
        : effectiveStatus === "http_502"
          ? 3
          : 1,
    poolDistinctAccountCount: isNoCandidate ? 0 : effectiveStatus === "http_502" ? 2 : 1,
    poolAttemptTerminalReason: isFailure ? failureKind : "completed",
    transport: effectiveStatus === "running" ? "websocket" : "http",
  };
}

function buildDemoInvocationRecord(
  [
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
  ]: DemoInvocationRow,
  accounts: Map<number, DemoAccount>,
  proxyName: (key: string) => string,
  attention: boolean,
) {
  const routingRequest = DEMO_MODEL_ROUTE_FIXTURES.find((fixture) => fixture.invocationId === id);
  const accountId = routingRequest?.accountId ?? rowAccountId;
  const model = routingRequest?.model ?? rowModel;
  const status =
    routingRequest?.terminalStatus ??
    (attention && id === 9002 ? "http_502" : attention && id === 9010 ? "http_401" : rowStatus);
  const isNoCandidate = id === 9003;
  const effectiveStatus = isNoCandidate ? "http_503" : status;
  const account = accounts.get(accountId);
  const { isFailure, failureClass, failureKind, errorMessage, downstreamStatusCode } =
    demoInvocationFailureDetails(effectiveStatus, isNoCandidate);
  const occurredAt = routingRequest
    ? demoModelRouteTimestamp(routingRequest.minutesAgo)
    : new Date(Date.parse(demoNow()) - (id - 9001) * 8_000).toISOString();
  const routeFields = demoInvocationRouteFields(
    id,
    isNoCandidate,
    effectiveStatus,
    isFailure,
    failureKind,
    account,
    promptCacheKey,
  );
  const timing = demoInvocationTiming(id, endpoint, ttfb, total);
  return {
    id,
    invokeId: `demo-invocation-${id}`,
    occurredAt,
    createdAt: occurredAt,
    source: "proxy",
    proxyDisplayName: proxyName(proxyKey),
    upstreamAccountId: isNoCandidate ? null : accountId,
    upstreamAccountName: isNoCandidate ? null : (account?.displayName ?? null),
    upstreamAccountPlanType: isNoCandidate ? null : (account?.planType ?? null),
    endpoint,
    model,
    requestModel: model,
    responseModel: effectiveStatus === "success" ? model : null,
    status: effectiveStatus,
    livePhase: effectiveStatus === "running" ? "responding" : null,
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
    isActionable: isFailure && effectiveStatus !== "client_cancelled",
    errorMessage,
    downstreamStatusCode,
    ...routeFields,
    ...timing,
    rawMetadata: {
      request: {
        demo: true,
        routeMode: isNoCandidate ? "pool" : account?.groupName === "standby" ? "fallback" : "pool",
      },
      response: { model, requestId: `req_demo_${id}` },
    },
  };
}
