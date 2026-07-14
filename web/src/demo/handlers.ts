import { HttpResponse, http, type JsonBodyType } from "msw";
import { demoModel, demoNow } from "./model";

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

function demoAccounts(): DemoAccount[] {
  return demoModel.snapshot.accounts as DemoAccount[];
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

const EMPTY_USAGE_BREAKDOWN = {
  cacheWriteTokens: 0,
  cacheReadTokens: 0,
  outputTokens: 0,
  costs: { input: 0, cacheWrite: 0, cacheRead: 0, output: 0, reasoning: 0, unknown: 0 },
  models: [],
};

function demoUsageBreakdown() {
  return demoModel.snapshot.scene === "empty" ? EMPTY_USAGE_BREAKDOWN : DEMO_USAGE_BREAKDOWN;
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

export function demoSummary() {
  const empty = demoModel.snapshot.scene === "empty";
  const attention = demoModel.snapshot.scene === "attention";
  const totalCount = empty ? 0 : 12846;
  const failureCount = empty ? 0 : attention ? 1174 : 428;
  return {
    rangeStart: "2026-07-10T00:00:00.000Z",
    rangeEnd: demoNow(),
    totalCount,
    successCount: totalCount - failureCount,
    failureCount,
    totalCost: empty ? 0 : 582.34,
    totalTokens: empty ? 0 : 1_381_240_000,
    usageBreakdown: demoUsageBreakdown(),
    inProgressConversationCount: empty ? 0 : attention ? 9 : 4,
    token: {
      requestCount: totalCount,
      totalTokens: empty ? 0 : 1_381_240_000,
      avgTokensPerRequest: empty ? 0 : 107_521,
      cacheInputTokens: empty ? 0 : 982_000_000,
      totalCost: empty ? 0 : 582.34,
    },
    network: {
      avgTtfbMs: empty ? 0 : 214,
      p95TtfbMs: empty ? 0 : 628,
      avgTotalMs: empty ? 0 : 2801,
      p95TotalMs: empty ? 0 : 9010,
    },
    exception: {
      failureCount,
      serviceFailureCount: attention ? 924 : 284,
      clientFailureCount: attention ? 152 : 104,
      clientAbortCount: attention ? 98 : 40,
      actionableFailureCount: attention ? 1076 : 388,
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
  ] as const;

  return rows.map(
    ([
      id,
      accountId,
      ,
      proxyKey,
      endpoint,
      model,
      status,
      inputTokens,
      outputTokens,
      cacheInputTokens,
      cacheWriteTokens,
      cost,
      ttfb,
      total,
      promptCacheKey,
    ]) => {
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
      const occurredAt = new Date(Date.parse(demoNow()) - (id - 9001) * 8_000).toISOString();
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
        reasoningEffort: model === "gpt-5.6-sol" ? "high" : null,
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
  );
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
      const requestCount = 10_130 - index * 790;
      const totalTokens = 1_135_000_000 - index * 96_000_000;
      const totalCost = Math.max(18.4, 499.1 - index * 47.25);
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
        successCount:
          requestCount - failureCount * 39 - (attention && account.id === 102 ? 410 : 0),
        failureCount: failureCount * 39 + (attention && account.id === 102 ? 410 : 18),
        nonSuccessCount: failureCount * 39 + (attention && account.id === 102 ? 410 : 18),
        totalTokens,
        successTokens: Math.round(totalTokens * 0.976),
        nonSuccessTokens: Math.round(totalTokens * 0.024),
        failureTokens: Math.round(totalTokens * 0.024),
        failureCost: Number((totalCost * 0.032).toFixed(2)),
        totalCost,
        usageBreakdown: demoUsageBreakdownForModels(modelIndexes),
        cacheHitRate: Number((0.814 - index * 0.012).toFixed(3)),
        tokensPerMinute: Math.max(2_100, 37_852 - index * 3_710),
        spendRate: Number(Math.max(1.1, 15.82 - index * 1.43).toFixed(2)),
        firstByteAvgMs: 198 + index * 18,
        firstResponseByteTotalAvgMs: 198 + index * 18,
        avgTotalMs: 2_536 + index * 184,
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
        responsesStreamTimeoutSecs: 300,
        compactStreamTimeoutSecs: 420,
      },
    },
  };
}

function systemStatus() {
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
  const templates = [
    ["sync_succeeded", "success", null, null, null],
    ["usage_snapshot_updated", "success", null, null, null],
    ["forward_proxy_assigned", "success", null, null, null],
    [
      "rate_limit_recovered",
      "success",
      "upstream_http_429_rate_limit",
      "Recovered after the simulated cooldown.",
      null,
    ],
    [
      "sync_deferred",
      "deferred",
      "transport_failure",
      "Deferred until the next maintenance pass.",
      null,
    ],
    [
      "mark_unavailable",
      "failed",
      "transport_failure",
      "Simulated timeout while checking upstream health.",
      504,
    ],
    [
      "reauth_required",
      "failed",
      "reauth_required",
      "Simulated authorization renewal is required.",
      401,
    ],
    ["sync_succeeded", "success", null, null, null],
    ["routing_rule_updated", "success", null, null, null],
    ["usage_snapshot_updated", "success", null, null, null],
    ["forward_proxy_health_checked", "success", null, null, null],
    ["quota_window_reset_observed", "success", null, null, null],
    [
      "sync_deferred",
      "deferred",
      "upstream_http_429_rate_limit",
      "Deferred until the simulated upstream rate limit resets.",
      429,
    ],
    ["sync_succeeded", "success", null, null, null],
    ["routing_rule_updated", "success", null, null, null],
  ] as const;
  return templates.map(([action, result, reasonCode, reasonMessage, httpStatus], index) => {
    const account = accounts[index] ?? accounts[0];
    const proxyKey = account.boundProxyKeys?.[0] ?? null;
    return {
      id: 7100 + index,
      action,
      source: index % 2 === 0 ? "maintenance_scheduler" : "operator",
      result,
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
      reasonCode,
      reasonMessage,
      httpStatus,
      failureKind:
        reasonCode === "transport_failure"
          ? "upstream_timeout"
          : reasonCode === "reauth_required"
            ? "upstream_auth_rejected"
            : null,
      invokeId: index < 6 ? `demo-invocation-${9001 + index}` : null,
      stickyKey:
        index % 3 === 0 ? `demo-conversation-${String.fromCharCode(97 + (index % 4))}` : null,
      occurredAt: new Date(Date.parse(demoNow()) - (index + 1) * 4 * 60_000).toISOString(),
      createdAt: new Date(Date.parse(demoNow()) - (index + 1) * 4 * 60_000).toISOString(),
    };
  });
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
      upstreamAccountId: fallback,
      upstreamAccountName:
        demoAccounts().find((account) => account.id === fallback)?.displayName ?? null,
      attemptIndex: 2,
      distinctAccountIndex: 2,
      sameAccountRetryIndex: 0,
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

async function handleRequest(request: Request) {
  const url = new URL(request.url);
  const pathname = apiPathname(url.pathname);
  if (demoModel.snapshot.scene === "network-failure") return HttpResponse.error();

  if (pathname === "/api/version") return json({ backend: "demo", frontend: "demo" });
  if (pathname === "/api/stats" || pathname === "/api/stats/summary") return json(demoSummary());
  if (pathname === "/api/stats/dashboard-activity") {
    const includeAccounts = url.searchParams.get("includeAccounts") === "true";
    const includeRecent = url.searchParams.get("includeRecent") !== "false";
    if (includeAccounts && demoModel.snapshot.scene === "progressive-loading") {
      await new Promise((resolve) => setTimeout(resolve, 2_000));
    }
    const accounts = demoDashboardActivityAccounts().map((account) =>
      includeRecent ? account : { ...account, recentInvocations: [] },
    );
    return json({
      range: url.searchParams.get("range") ?? "today",
      snapshotId: 901,
      rangeStart: "2026-07-10T00:00:00Z",
      rangeEnd: demoNow(),
      rateWindow: {
        start: "2026-07-10T09:00:00Z",
        end: demoNow(),
        windowMinutes: 30,
        mode: "rolling",
      },
      summary: {
        stats: demoSummary(),
        tokensPerMinute: 46_041,
        spendRate: 19.41,
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
        responsesStreamTimeoutSecs: 300,
        compactStreamTimeoutSecs: 420,
      },
      timeoutFieldSources: {
        responsesFirstByteTimeoutSecs: "root",
        compactFirstByteTimeoutSecs: "root",
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
    let records = invocations();
    const model = url.searchParams.get("model");
    const status = url.searchParams.get("status");
    const endpoint = url.searchParams.get("endpoint");
    const upstreamAccountId = Number(url.searchParams.get("upstreamAccountId"));
    const keyword = url.searchParams.get("keyword")?.toLowerCase();
    if (model) records = records.filter((record) => record.model === model);
    if (status) records = records.filter((record) => record.status === status);
    if (endpoint) records = records.filter((record) => record.endpoint === endpoint);
    if (Number.isFinite(upstreamAccountId) && upstreamAccountId > 0)
      records = records.filter((record) => record.upstreamAccountId === upstreamAccountId);
    if (keyword)
      records = records.filter((record) => JSON.stringify(record).toLowerCase().includes(keyword));
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
  if (pathname === "/api/invocations/summary")
    return json({ snapshotId: 901, newRecordsCount: 0, ...demoSummary() });
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
  if (pathname.endsWith("/response-body")) {
    const id = Number(pathname.split("/").at(-2));
    const record = invocations().find((item) => item.id === id);
    const isFailure = record?.failureClass && record.failureClass !== "none";
    return json(
      isFailure
        ? {
            available: true,
            bodyText: JSON.stringify(
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
        responsesStreamTimeoutSecs: 300,
        compactStreamTimeoutSecs: 420,
      },
    });
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
    demoModel.record(`模拟 ${request.method} ${pathname.split("/").slice(-1)[0]}`);
    return json({ ok: true, simulated: true, updatedAt: demoNow() });
  }

  return json({ error: `Unhandled demo API route: ${pathname}` }, { status: 501 });
}

export const apiHandlers = [http.all(/\/api\/.*/, ({ request }) => handleRequest(request))];
