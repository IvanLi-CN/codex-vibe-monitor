import { HttpResponse, http, type JsonBodyType } from 'msw'
import { demoModel, demoNow } from './model'

function json(payload: unknown, init?: ResponseInit) {
  return HttpResponse.json(payload as JsonBodyType, init)
}

function apiPathname(pathname: string) {
  const apiIndex = pathname.indexOf('/api/')
  return apiIndex === -1 ? pathname : pathname.slice(apiIndex)
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
      model: 'gpt-5.6-sol',
      reasoningEffort: 'high',
      cacheWriteTokens: 110_000_000,
      cacheReadTokens: 480_000_000,
      outputTokens: 65_000_000,
      costs: { input: 39.5, cacheWrite: 78, cacheRead: 24, output: 143, reasoning: 22.8, unknown: 0 },
    },
    {
      model: 'gpt-5.6-sol',
      reasoningEffort: 'medium',
      cacheWriteTokens: 80_000_000,
      cacheReadTokens: 350_000_000,
      outputTokens: 50_000_000,
      costs: { input: 22, cacheWrite: 45, cacheRead: 17.5, output: 95, reasoning: 12.3, unknown: 0 },
    },
    {
      model: 'gpt-5.6-terra',
      reasoningEffort: null,
      cacheWriteTokens: 60_000_000,
      cacheReadTokens: 152_000_000,
      outputTokens: 34_240_000,
      costs: { input: 10, cacheWrite: 20, cacheRead: 5, output: 40, reasoning: 8.24, unknown: 0 },
    },
  ],
}

const EMPTY_USAGE_BREAKDOWN = {
  cacheWriteTokens: 0,
  cacheReadTokens: 0,
  outputTokens: 0,
  costs: { input: 0, cacheWrite: 0, cacheRead: 0, output: 0, reasoning: 0, unknown: 0 },
  models: [],
}

function demoUsageBreakdown() {
  return demoModel.snapshot.scene === 'empty'
    ? EMPTY_USAGE_BREAKDOWN
    : DEMO_USAGE_BREAKDOWN
}

function demoUsageBreakdownForModels(modelIndexes: number[]) {
  const models = modelIndexes.map((index) => DEMO_USAGE_BREAKDOWN.models[index]).filter((model) => model != null)
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
  )
  return {
    cacheWriteTokens: models.reduce((total, model) => total + model.cacheWriteTokens, 0),
    cacheReadTokens: models.reduce((total, model) => total + model.cacheReadTokens, 0),
    outputTokens: models.reduce((total, model) => total + model.outputTokens, 0),
    costs,
    models,
  }
}

export function demoSummary() {
  const empty = demoModel.snapshot.scene === 'empty'
  const attention = demoModel.snapshot.scene === 'attention'
  const totalCount = empty ? 0 : 12846
  const failureCount = empty ? 0 : attention ? 1174 : 428
  return {
    rangeStart: '2026-07-10T00:00:00.000Z',
    rangeEnd: demoNow(),
    totalCount,
    successCount: totalCount - failureCount,
    failureCount,
    totalCost: empty ? 0 : 582.34,
    totalTokens: empty ? 0 : 1_381_240_000,
    usageBreakdown: demoUsageBreakdown(),
    inProgressConversationCount: empty ? 0 : attention ? 9 : 4,
    token: { requestCount: totalCount, totalTokens: empty ? 0 : 1_381_240_000, avgTokensPerRequest: empty ? 0 : 107_521, cacheInputTokens: empty ? 0 : 982_000_000, totalCost: empty ? 0 : 582.34 },
    network: { avgTtfbMs: empty ? 0 : 214, p95TtfbMs: empty ? 0 : 628, avgTotalMs: empty ? 0 : 2801, p95TotalMs: empty ? 0 : 9010 },
    exception: { failureCount, serviceFailureCount: attention ? 924 : 284, clientFailureCount: attention ? 152 : 104, clientAbortCount: attention ? 98 : 40, actionableFailureCount: attention ? 1076 : 388 },
  }
}

function invocations() {
  if (demoModel.snapshot.scene === 'empty') return []
  const attention = demoModel.snapshot.scene === 'attention'
  return [
    { id: 9001, invokeId: 'demo-invocation-9001', occurredAt: demoNow(), createdAt: demoNow(), source: 'proxy', proxyDisplayName: 'Tokyo demo relay', upstreamAccountId: 101, upstreamAccountName: 'alpha@demo.invalid', upstreamAccountPlanType: 'team', endpoint: '/v1/responses', model: 'gpt-5.6-sol', status: 'running', requestedServiceTier: 'priority', serviceTier: 'priority', inputTokens: 12520, outputTokens: 0, cacheInputTokens: 10880, cacheWriteTokens: 1640, reasoningTokens: 480, reasoningEffort: 'high', totalTokens: 12520, cost: 0.014, tUpstreamTtfbMs: null, tTotalMs: null },
    { id: 9002, invokeId: 'demo-invocation-9002', occurredAt: '2026-07-10T09:23:00.000Z', createdAt: '2026-07-10T09:23:00.000Z', source: 'proxy', proxyDisplayName: 'Tokyo demo relay', upstreamAccountId: 101, upstreamAccountName: 'alpha@demo.invalid', upstreamAccountPlanType: 'team', endpoint: '/v1/responses', model: 'gpt-5.6-sol', status: attention ? 'http_502' : 'success', requestedServiceTier: 'auto', serviceTier: 'auto', inputTokens: 9320, outputTokens: 882, cacheInputTokens: 7311, cacheWriteTokens: 2009, reasoningTokens: 360, reasoningEffort: 'medium', totalTokens: 10202, cost: 0.0092, tUpstreamTtfbMs: 184, tTotalMs: 1882, errorMessage: attention ? 'simulated upstream timeout' : null, failureClass: attention ? 'service_failure' : null, failureKind: attention ? 'upstream_timeout' : null },
    { id: 9003, invokeId: 'demo-invocation-9003', occurredAt: '2026-07-10T09:17:00.000Z', createdAt: '2026-07-10T09:17:00.000Z', source: 'proxy', proxyDisplayName: 'Tokyo demo relay', upstreamAccountId: 102, upstreamAccountName: 'backup-key', upstreamAccountPlanType: 'api', endpoint: '/v1/chat/completions', model: 'gpt-5.6-terra', status: 'success', requestedServiceTier: 'auto', serviceTier: 'auto', inputTokens: 4110, outputTokens: 295, cacheInputTokens: 2980, cacheWriteTokens: 1130, reasoningTokens: 0, totalTokens: 4405, cost: 0.0037, tUpstreamTtfbMs: 146, tTotalMs: 1095 },
  ]
}

function demoDashboardActivityAccounts() {
  if (demoModel.snapshot.scene === 'empty') return []
  const attention = demoModel.snapshot.scene === 'attention'
  const recent = invocations()
  return [
    {
      accountKey: 'upstream:101',
      upstreamAccountId: 101,
      displayName: 'alpha@demo.invalid',
      groupName: 'production',
      planType: 'team',
      enabled: true,
      displayStatus: 'active',
      enableStatus: 'enabled',
      workStatus: 'working',
      healthStatus: 'normal',
      syncState: 'idle',
      requestCount: 10_130,
      successCount: attention ? 9_430 : 9_830,
      failureCount: attention ? 700 : 300,
      nonSuccessCount: attention ? 700 : 300,
      totalTokens: 1_135_000_000,
      successTokens: 1_109_000_000,
      nonSuccessTokens: 26_000_000,
      failureTokens: 26_000_000,
      failureCost: attention ? 28.7 : 11.2,
      totalCost: 499.1,
      usageBreakdown: demoUsageBreakdownForModels([0, 1]),
      cacheHitRate: 0.814,
      tokensPerMinute: 37_852,
      spendRate: 15.82,
      firstByteAvgMs: 198,
      firstResponseByteTotalAvgMs: 198,
      avgTotalMs: 2_536,
      inProgressInvocationCount: attention ? 7 : 3,
      inProgressPhaseCounts: { queued: 1, requesting: 1, responding: attention ? 5 : 1 },
      retryInvocationCount: attention ? 2 : 0,
      effectiveRoutingRule: {
        allowCutOut: true,
        allowCutIn: true,
        priorityTier: 'normal',
        fastModeRewriteMode: 'keep_original',
        imageToolRewriteMode: 'keep_original',
        concurrencyLimit: 4,
        upstream429RetryEnabled: true,
        upstream429MaxRetries: 2,
        availableModels: ['gpt-5.6-sol'],
        availableModelsDefined: true,
        systemDeniedModels: [],
        sourceTagIds: [],
        sourceTagNames: [],
      },
      recentInvocations: recent.filter((record) => record.upstreamAccountId === 101),
    },
    {
      accountKey: 'upstream:102',
      upstreamAccountId: 102,
      displayName: 'backup-key',
      groupName: 'standby',
      planType: 'api',
      enabled: true,
      displayStatus: attention ? 'upstream_unavailable' : 'active',
      enableStatus: 'enabled',
      workStatus: attention ? 'unavailable' : 'idle',
      healthStatus: attention ? 'upstream_unavailable' : 'normal',
      syncState: 'idle',
      lastError: attention ? 'Simulated upstream timeout.' : null,
      requestCount: 2_716,
      successCount: attention ? 2_242 : 2_588,
      failureCount: attention ? 474 : 128,
      nonSuccessCount: attention ? 474 : 128,
      totalTokens: 246_240_000,
      successTokens: 239_000_000,
      nonSuccessTokens: 7_240_000,
      failureTokens: 7_240_000,
      failureCost: attention ? 12.4 : 3.8,
      totalCost: 83.24,
      usageBreakdown: demoUsageBreakdownForModels([2]),
      cacheHitRate: 0.717,
      tokensPerMinute: 8_189,
      spendRate: 3.59,
      firstByteAvgMs: 342,
      firstResponseByteTotalAvgMs: 342,
      avgTotalMs: 3_981,
      inProgressInvocationCount: attention ? 2 : 1,
      inProgressPhaseCounts: { queued: 0, requesting: 1, responding: attention ? 1 : 0 },
      retryInvocationCount: attention ? 1 : 0,
      effectiveRoutingRule: {
        allowCutOut: true,
        allowCutIn: false,
        priorityTier: 'fallback',
        fastModeRewriteMode: 'keep_original',
        imageToolRewriteMode: 'keep_original',
        concurrencyLimit: 2,
        upstream429RetryEnabled: true,
        upstream429MaxRetries: 1,
        availableModels: ['gpt-5.6-terra'],
        availableModelsDefined: true,
        systemDeniedModels: [],
        sourceTagIds: [],
        sourceTagNames: [],
      },
      recentInvocations: recent.filter((record) => record.upstreamAccountId === 102),
    },
  ]
}

function timeseries() {
  const empty = demoModel.snapshot.scene === 'empty'
  const start = Date.parse('2026-07-10T00:00:00.000Z')
  return {
    rangeStart: new Date(start).toISOString(),
    rangeEnd: demoNow(),
    bucketSeconds: 3600,
    effectiveBucket: '1h',
    availableBuckets: ['1m', '15m', '1h', '1d'],
    points: empty ? [] : Array.from({ length: 10 }, (_, index) => ({
      bucketStart: new Date(start + index * 3_600_000).toISOString(),
      bucketEnd: new Date(start + (index + 1) * 3_600_000).toISOString(),
      totalCount: 920 + index * 61,
      successCount: 886 + index * 57,
      failureCount: 34 + (index % 3),
      totalTokens: 104_000_000 + index * 4_200_000,
      totalCost: 42.1 + index * 1.2,
      avgLatencyMs: 210 + index * 4,
    })),
  }
}

function parallelWork() {
  const points = demoModel.snapshot.scene === 'empty' ? [] : Array.from({ length: 12 }, (_, index) => ({
    bucketStart: new Date(Date.parse('2026-07-10T00:00:00Z') + index * 3_600_000).toISOString(),
    bucketEnd: new Date(Date.parse('2026-07-10T00:00:00Z') + (index + 1) * 3_600_000).toISOString(),
    parallelCount: 2 + (index % 5),
  }))
  const current = { rangeStart: '2026-07-10T00:00:00Z', rangeEnd: demoNow(), bucketSeconds: 3600, completeBucketCount: points.length, activeBucketCount: points.length, minCount: points.length ? 2 : null, maxCount: points.length ? 6 : null, avgCount: points.length ? 4 : null, effectiveTimeZone: 'Asia/Shanghai', timeZoneFallback: false, points, conversations: [] }
  return { current, minute7d: current, hour30d: current, dayAll: current }
}

function promptCacheConversations() {
  if (demoModel.snapshot.scene === 'empty') {
    return { rangeStart: '2026-07-10T09:00:00Z', rangeEnd: demoNow(), selectionMode: 'count', selectedLimit: 50, selectedActivityHours: null, selectedActivityMinutes: null, implicitFilter: { kind: null, filteredCount: 0 }, totalMatched: 0, hasMore: false, nextCursor: null, conversations: [] }
  }
  const recent = invocations().slice(0, 2)
  return {
    rangeStart: '2026-07-10T09:00:00Z', rangeEnd: demoNow(), selectionMode: 'count', selectedLimit: 50, selectedActivityHours: null, selectedActivityMinutes: null, implicitFilter: { kind: null, filteredCount: 0 }, totalMatched: 1, hasMore: false, nextCursor: null,
    conversations: [{ promptCacheKey: 'demo-conversation-a', hasEncryptedSessionOwner: false, encryptedOwnerAccountId: null, encryptedOwnerAccountName: null, encryptedOwnerGroupName: null, requestCount: recent.length, totalTokens: 22722, totalCost: 0.0232, createdAt: '2026-07-10T09:15:00Z', lastActivityAt: demoNow(), upstreamAccounts: [], recentInvocations: recent, last24hRequests: [] }],
  }
}

function forwardProxyLive() {
  if (demoModel.snapshot.scene === 'empty') {
    return { rangeStart: '2026-07-10T00:00:00Z', rangeEnd: demoNow(), bucketSeconds: 3600, nodes: [] }
  }
  const node = (demoModel.snapshot.settings.forwardProxy as { nodes: Array<Record<string, unknown>> }).nodes[0]
  return {
    rangeStart: '2026-07-10T00:00:00Z',
    rangeEnd: demoNow(),
    bucketSeconds: 3600,
    nodes: [{
      ...node,
      last24h: [{ bucketStart: '2026-07-10T08:00:00Z', bucketEnd: '2026-07-10T09:00:00Z', successCount: demoModel.snapshot.scene === 'attention' ? 12 : 17, failureCount: demoModel.snapshot.scene === 'attention' ? 6 : 1 }],
      weight24h: [{ bucketStart: '2026-07-10T08:00:00Z', bucketEnd: '2026-07-10T09:00:00Z', sampleCount: 18, minWeight: 0.76, maxWeight: 0.94, avgWeight: 0.88, lastWeight: 0.92 }],
    }],
  }
}

function accountList() {
  const items = demoModel.snapshot.scene === 'empty' ? [] : demoModel.snapshot.accounts
  return {
    items,
    total: items.length,
    page: 1,
    pageSize: 50,
    groups: [
      { groupName: 'production', note: 'Simulated primary pool', accountCount: 1, boundProxyKeys: ['demo-tokyo'] },
      { groupName: 'standby', note: 'Simulated recovery capacity', accountCount: 1, boundProxyKeys: [] },
    ],
    forwardProxyNodes: (demoModel.snapshot.settings.forwardProxy as { nodes: unknown[] }).nodes,
    writesEnabled: true,
    availableModels: ['gpt-5.6-sol', 'gpt-5.6-terra', 'gpt-5.4-mini'],
    metrics: { total: items.length, oauth: items.filter((item) => item.kind === 'oauth_codex').length, apiKey: items.filter((item) => item.kind === 'api_key').length, attention: demoModel.snapshot.scene === 'attention' ? 1 : 0 },
  }
}

function systemStatus() {
  return { liveInvocationsCount: 128_076, successCount: 124_882, nonSuccessCount: 3_194, completedArchiveBatchesCount: 384, archivedBodies: { count: 118_420, bytes: 8_441_053_184 }, rawBodies: { count: 1_482, bytes: 84_221_184 }, requestRawBodies: { count: 812, bytes: 76_221_184 }, responseRawBodies: { count: 670, bytes: 8_000_000 }, databaseBytes: 618_659_840, otherFilesBytes: 142_344_192, refreshedAt: demoNow() }
}

async function handleRequest(request: Request) {
  const url = new URL(request.url)
  const pathname = apiPathname(url.pathname)
  if (demoModel.snapshot.scene === 'network-failure') return HttpResponse.error()

  if (pathname === '/api/version') return json({ backend: 'demo', frontend: 'demo' })
  if (pathname === '/api/stats' || pathname === '/api/stats/summary') return json(demoSummary())
  if (pathname === '/api/stats/dashboard-activity') {
    const includeAccounts = url.searchParams.get('includeAccounts') === 'true'
    return json({
      range: url.searchParams.get('range') ?? 'today',
      snapshotId: 901,
      rangeStart: '2026-07-10T00:00:00Z',
      rangeEnd: demoNow(),
      rateWindow: {
        start: '2026-07-10T09:00:00Z',
        end: demoNow(),
        windowMinutes: 30,
        mode: 'rolling',
      },
      summary: {
        stats: demoSummary(),
        tokensPerMinute: 46_041,
        spendRate: 19.41,
      },
      accounts: includeAccounts ? demoDashboardActivityAccounts() : undefined,
    })
  }
  if (pathname === '/api/stats/timeseries') return json(timeseries())
  if (pathname === '/api/stats/parallel-work') return json(parallelWork(), { headers: { ETag: 'demo-parallel-work' } })
  if (pathname === '/api/stats/errors') return json({ rangeStart: '2026-07-10T00:00:00Z', rangeEnd: demoNow(), items: demoModel.snapshot.scene === 'empty' ? [] : [{ reason: 'upstream_timeout', count: 24 }, { reason: 'rate_limited', count: 11 }] })
  if (pathname === '/api/stats/failures/summary') return json({ rangeStart: '2026-07-10T00:00:00Z', rangeEnd: demoNow(), totalFailures: 35, serviceFailureCount: 24, clientFailureCount: 7, clientAbortCount: 4, actionableFailureCount: 31, actionableFailureRate: 0.88 })
  if (pathname === '/api/stats/forward-proxy') return json(forwardProxyLive())
  if (pathname === '/api/stats/forward-proxy/timeseries') return json({ rangeStart: '2026-07-10T00:00:00Z', rangeEnd: demoNow(), nodes: [], points: [] })
  if (pathname === '/api/stats/prompt-cache-conversations') return json(promptCacheConversations())
  if (pathname.startsWith('/api/stats/prompt-cache-conversation-bindings/')) return json({ bindingKind: 'none', upstreamAccountId: null, groupName: null })
  if (pathname === '/api/quota/latest') return json({ capturedAt: demoNow(), accounts: [] })

  if (pathname === '/api/invocations') return json({ snapshotId: 901, total: invocations().length, page: 1, pageSize: 50, records: invocations() })
  if (pathname === '/api/invocations/summary') return json({ snapshotId: 901, newRecordsCount: 0, ...demoSummary() })
  if (pathname === '/api/invocations/new-count') return json({ snapshotId: 901, newRecordsCount: 0 })
  if (pathname === '/api/invocations/suggestions') return json({ model: { items: [{ value: 'gpt-5.6-sol', count: 12 }], hasMore: false }, proxy: { items: [], hasMore: false }, endpoint: { items: [], hasMore: false }, failureKind: { items: [], hasMore: false }, promptCacheKey: { items: [], hasMore: false }, requesterIp: { items: [], hasMore: false } })
  if (pathname.endsWith('/detail')) return json({ record: invocations()[0], attempts: [] })
  if (pathname.endsWith('/response-body')) return json({ content: '{"demo":true}', encoding: 'identity', truncated: false })
  if (pathname.endsWith('/pool-attempts')) return json({ attempts: [] })

  if (pathname === '/api/settings' && request.method === 'GET') return json(demoModel.snapshot.settings)
  if (pathname === '/api/settings/external-api-keys' && request.method === 'GET') return json({ items: demoModel.snapshot.externalApiKeys })
  if (pathname === '/api/settings/external-api-keys' && request.method === 'POST') return json(demoModel.createExternalApiKey(), { status: 201 })
  if (pathname === '/api/system/status') return json(systemStatus())
  if (pathname === '/api/system/tasks') return json({ total: 2, page: 1, pageSize: 20, items: [{ id: 1, taskKind: 'demo_refresh', triggerKind: 'manual', status: 'success', summary: 'simulated refresh completed', detail: 'Demo-only task record.', startedAt: '2026-07-10T09:00:00Z', finishedAt: '2026-07-10T09:00:01Z', durationMs: 1000 }] })

  if (pathname === '/api/pool/upstream-accounts' && request.method === 'GET') return json(accountList())
  if (pathname === '/api/pool/upstream-account-events') return json({ total: 1, page: 1, pageSize: 20, items: [{ id: 1, action: 'sync', result: 'success', accountId: 101, occurredAt: demoNow(), detail: 'Simulated account sync.' }] })
  if (pathname === '/api/pool/upstream-accounts/window-usage') return json({ items: [] })
  if (pathname === '/api/pool/forward-proxy-binding-nodes') return json((demoModel.snapshot.settings.forwardProxy as { nodes: unknown[] }).nodes)
  if (pathname === '/api/pool/tags' && request.method === 'GET') return json({ writesEnabled: true, items: [{ id: 1, name: 'primary', accountCount: 1, groupCount: 1, updatedAt: demoNow() }, { id: 2, name: 'fallback', accountCount: 1, groupCount: 1, updatedAt: demoNow() }] })
  if (pathname === '/api/pool/routing-settings') return json({ defaultHijackEnabled: false, maintenance: { primarySyncIntervalSecs: 300, secondarySyncIntervalSecs: 1800, priorityAvailableAccountCap: 100 } })
  if (pathname.includes('/sticky-keys')) return json({ totalMatched: 0, conversations: [], hasMore: false, nextCursor: null })
  if (/^\/api\/pool\/upstream-accounts\/\d+$/.test(pathname) && request.method === 'GET') return json(demoModel.snapshot.accounts[0])

  if (request.method !== 'GET' && request.method !== 'HEAD') {
    let body: unknown = null
    try { body = await request.clone().json() } catch { /* no JSON body */ }
    if (pathname === '/api/settings' || pathname.startsWith('/api/settings/')) return json(demoModel.updateSettings(pathname, body))
    if (pathname === '/api/pool/upstream-accounts') return json(demoModel.createAccount(), { status: 201 })
    demoModel.record(`模拟 ${request.method} ${pathname.split('/').slice(-1)[0]}`)
    return json({ ok: true, simulated: true, updatedAt: demoNow() })
  }

  return json({ error: `Unhandled demo API route: ${pathname}` }, { status: 501 })
}

export const apiHandlers = [
  http.all(/\/api\/.*/, ({ request }) => handleRequest(request)),
]
