import { HttpResponse, http } from "msw";
import {
  accountEvents,
  accountList,
  apiPathname,
  buildDemoInvocationWorkflowDetail,
  DEMO_INVOCATION_RESPONSE_BODY_SIZE,
  DEMO_INVOCATION_RESPONSE_BODY_TEXT,
  demoAccounts,
  demoAttemptPhase,
  demoDashboardActivityAccounts,
  demoDashboardActivitySummary,
  demoInvocationSummary,
  demoLongTermOverview,
  demoLongTermSeries,
  demoModelPerformanceForModels,
  demoModelRoutingLive,
  demoModelRoutingStates,
  demoModelRoutingTimeline,
  demoResetModelRoutes,
  demoSummary,
  filterDemoInvocations,
  forwardProxyBindingNodes,
  forwardProxyLive,
  invocations,
  json,
  parallelWork,
  poolAttempts,
  promptCacheConversations,
  publicModelRoutingRecord,
  recordsToSuggestionCounts,
  simulatedAccountDisplayName,
  systemStatus,
  systemTasks,
  timeseries,
  upstreamAccountAttempts,
} from "./handlers-support";
import { demoModel, demoNow } from "./model";

type DemoRouteResult = Response | undefined;

async function handleCoreStatsRequest(pathname: string, url: URL): Promise<DemoRouteResult> {
  if (pathname === "/api/version") return json({ backend: "0.2.0", frontend: "0.2.0" });
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
  return undefined;
}

function handleForwardProxyRequest(pathname: string): DemoRouteResult {
  if (pathname === "/api/stats/forward-proxy") return json(forwardProxyLive());
  if (pathname !== "/api/stats/forward-proxy/timeseries") return undefined;
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

function promptCacheBindingEvents(promptCacheKey: string) {
  return [
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
          selectedAccountId: 102,
          selectedAccountName: simulatedAccountDisplayName(102),
          eligibleCandidateCount: 1,
          winnerReasonCode: "onlyEligibleCandidate",
          comparedAccountId: null,
          comparedAccountName: null,
          excludedCandidates: [
            {
              accountId: 115,
              accountName: simulatedAccountDisplayName(115),
              reasonCode: "modelNotAllowed",
            },
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
          selectedAccountId: 102,
          selectedAccountName: simulatedAccountDisplayName(102),
          eligibleCandidateCount: 1,
          winnerReasonCode: "onlyEligibleCandidate",
          comparedAccountId: null,
          comparedAccountName: null,
          excludedCandidates: [
            {
              accountId: 115,
              accountName: simulatedAccountDisplayName(115),
              reasonCode: "modelNotAllowed",
            },
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
}

function handleInvocationListRequest(pathname: string, url: URL): DemoRouteResult {
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
  if (pathname !== "/api/invocations/suggestions") return undefined;
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

function handleInvocationDetailRequest(pathname: string): DemoRouteResult {
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
  if (!pathname.endsWith("/request-body")) return undefined;
  const id = Number(pathname.split("/").at(-2));
  const record = invocations().find((item) => item.id === id);
  if (!record) return json({ error: `Demo invocation ${id} not found.` }, { status: 404 });
  if (id === 9002) return json({ available: false, unavailableReason: "missing_body" });
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

function handleInvocationAttemptResponseRequest(pathname: string): DemoRouteResult {
  const match = pathname.match(/^\/api\/invocations\/(\d+)\/attempts\/([^/]+)\/response-body$/);
  if (!match) return undefined;
  const id = Number(match[1]);
  const attemptId = decodeURIComponent(match[2] ?? "");
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
    routing: { forwardedChunkCount: 12 },
    bodySize: DEMO_INVOCATION_RESPONSE_BODY_SIZE,
    bodyTruncated: false,
    detailLevel: "full",
    captureSource: "attempt_raw_file",
    availableAtAttemptLevel: true,
  });
}

function handleInvocationResponseRequest(pathname: string): DemoRouteResult {
  if (!pathname.endsWith("/response-body")) return undefined;
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
      routing: { forwardedChunkCount: 12 },
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

function handleInvocationAttemptsRequest(
  pathname: string,
  url: URL,
  request: Request,
): DemoRouteResult {
  if (pathname.endsWith("/pool-attempts"))
    return json(poolAttempts(decodeURIComponent(pathname.split("/").at(-2) ?? "")));
  const match = pathname.match(
    /^\/api\/pool\/upstream-accounts\/(\d+)\/call-attempts(?:\/locate)?$/,
  );
  if (!match || request.method !== "GET") return undefined;
  const accountId = Number(match[1]);
  const response = upstreamAccountAttempts(accountId, url.searchParams);
  const requestedAttemptId = url.searchParams.get("attemptId")?.trim();
  if (
    pathname.endsWith("/locate") &&
    requestedAttemptId &&
    !response.items.some((attempt) => attempt.attemptId === requestedAttemptId)
  ) {
    return json({ message: "upstream account attempt was not found" }, { status: 404 });
  }
  return json(response);
}

function handlePromptCacheRequest(pathname: string, url: URL): DemoRouteResult {
  if (pathname === "/api/stats/prompt-cache-conversations") return json(promptCacheConversations());
  if (pathname.startsWith("/api/stats/prompt-cache-conversation-binding-events/")) {
    const promptCacheKey = decodeURIComponent(pathname.split("/").at(-1) ?? "");
    const items = promptCacheBindingEvents(promptCacheKey);
    const infoType = url.searchParams.get("infoType");
    const filtered = infoType ? items.filter((item) => item.infoTypes.includes(infoType)) : items;
    return json({ items: filtered, total: filtered.length, page: 1, pageSize: 20 });
  }
  if (!pathname.startsWith("/api/stats/prompt-cache-conversation-bindings/")) return undefined;
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

function handleSettingsAndSystemRequest(
  pathname: string,
  url: URL,
  request: Request,
): DemoRouteResult {
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
        : { key: { ...key, updatedAt: demoNow() }, secret: "cvm-synthetic-rotated-key-not-valid" },
    );
  }
  if (pathname === "/api/system/status") return json(systemStatus());
  if (pathname !== "/api/system/tasks") return undefined;
  let items = systemTasks();
  const taskKind = url.searchParams.get("taskKind");
  const status = url.searchParams.get("status");
  if (taskKind) items = items.filter((item) => item.taskKind.includes(taskKind));
  if (status) items = items.filter((item) => item.status === status);
  const pageSize = Number(url.searchParams.get("pageSize") ?? url.searchParams.get("limit") ?? 20);
  const page = Number(url.searchParams.get("page") ?? 1);
  return json({
    total: items.length,
    page,
    pageSize,
    items: items.slice((page - 1) * pageSize, page * pageSize),
  });
}

function handlePoolRequest(pathname: string, url: URL, request: Request): DemoRouteResult {
  if (pathname === "/api/pool/upstream-accounts" && request.method === "GET") {
    return json(accountList(url.searchParams.get("kind")));
  }
  if (pathname === "/api/pool/upstream-accounts/window-usage") {
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
  }
  if (pathname === "/api/pool/forward-proxy-binding-nodes") {
    return json(forwardProxyBindingNodes());
  }
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
  if (pathname.includes("/sticky-keys")) {
    return json({
      rangeStart: "2026-07-10T00:00:00Z",
      rangeEnd: demoNow(),
      selectionMode: "count",
      selectedLimit: 50,
      selectedActivityHours: null,
      selectedActivityMinutes: null,
      implicitFilter: { kind: null, filteredCount: 0 },
      totalMatched: 3,
      conversations: promptCacheConversations().conversations.slice(0, 3),
      hasMore: false,
      nextCursor: null,
    });
  }
  return undefined;
}

export async function handleDemoRequest(request: Request) {
  const url = new URL(request.url);
  const pathname = apiPathname(url.pathname);
  if (demoModel.snapshot.scene === "network-failure") return HttpResponse.error();

  const coreStatsResponse = await handleCoreStatsRequest(pathname, url);
  if (coreStatsResponse) return coreStatsResponse;
  const forwardProxyResponse = handleForwardProxyRequest(pathname);
  if (forwardProxyResponse) return forwardProxyResponse;
  const invocationListResponse = handleInvocationListRequest(pathname, url);
  if (invocationListResponse) return invocationListResponse;
  const invocationDetailResponse = handleInvocationDetailRequest(pathname);
  if (invocationDetailResponse) return invocationDetailResponse;
  const invocationAttemptResponse = handleInvocationAttemptResponseRequest(pathname);
  if (invocationAttemptResponse) return invocationAttemptResponse;
  const invocationResponse = handleInvocationResponseRequest(pathname);
  if (invocationResponse) return invocationResponse;
  const invocationAttemptsResponse = handleInvocationAttemptsRequest(pathname, url, request);
  if (invocationAttemptsResponse) return invocationAttemptsResponse;
  const promptCacheResponse = handlePromptCacheRequest(pathname, url);
  if (promptCacheResponse) return promptCacheResponse;
  const settingsResponse = handleSettingsAndSystemRequest(pathname, url, request);
  if (settingsResponse) return settingsResponse;
  const poolResponse = handlePoolRequest(pathname, url, request);
  if (poolResponse) return poolResponse;

  if (pathname === "/api/pool/upstream-accounts/api-keys/migration/preflight") {
    const legacyApiKeyCount = demoAccounts().filter(
      (account) =>
        account.kind === "api_key_codex" &&
        ((typeof account.groupName === "string" && account.groupName.trim().length > 0) ||
          account.isMother === true),
    ).length;
    return json({
      confirmationHash: "demo-migration-confirmation-hash",
      apiKeyCount: legacyApiKeyCount,
      portableFields: [
        "account-level routing policy",
        "bound proxy keys",
        "local quota limits",
        "note",
      ],
      blockedStrategies: [],
      canMigrate: true,
    });
  }
  if (pathname === "/api/pool/upstream-accounts/api-keys/migration/confirm") {
    const migratedCount = demoModel.migrateLegacyApiKeyAccounts();
    return json({
      migratedCount,
      confirmationHash: "demo-migration-confirmation-hash",
      auditAction: "api_key_transit_proxy_binding_migrated",
    });
  }
  if (pathname === "/api/pool/upstream-account-events") {
    let items = accountEvents();
    const kind = url.searchParams.get("kind");
    const account = url.searchParams.get("account")?.toLowerCase();
    const group = url.searchParams.get("group")?.toLowerCase();
    const proxyKey = url.searchParams.get("proxyKey");
    const result = url.searchParams.get("result");
    if (account)
      items = items.filter((item) => item.accountDisplayName?.toLowerCase().includes(account));
    if (group) items = items.filter((item) => item.accountGroupName?.toLowerCase().includes(group));
    if (proxyKey) items = items.filter((item) => item.forwardProxyKey === proxyKey);
    if (result) items = items.filter((item) => item.result === result);
    if (kind) {
      const accountIds = new Set(
        demoAccounts()
          .filter((account) => account.kind === kind)
          .map((account) => account.id),
      );
      items = items.filter((item) => accountIds.has(item.upstreamAccountId));
    }
    const pageSize = Number(url.searchParams.get("pageSize") ?? 20);
    const page = Number(url.searchParams.get("page") ?? 1);
    return json({
      total: items.length,
      page,
      pageSize,
      items: items.slice((page - 1) * pageSize, page * pageSize),
    });
  }
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
      priorityHandoffAdmissionEnabled: true,
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

    const items = demoModelRoutingTimeline(accountId, model).map(publicModelRoutingRecord);
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
export { demoAttemptPhase, demoSummary };
