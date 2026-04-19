import type {
  AccountTagSummary,
  BulkUpstreamAccountSyncCounts,
  BulkUpstreamAccountSyncSnapshot,
  BulkUpstreamAccountSyncRow,
  EffectiveRoutingRule,
  ForwardProxyBindingNode,
  LoginSessionStatusResponse,
  OauthMailboxStatus,
  PoolRoutingMaintenanceSettings,
  PoolRoutingSettings,
  PoolRoutingTimeoutSettings,
  TagSummary,
  UpstreamAccountDetail,
  UpstreamAccountSummary,
} from '../lib/api'
import { duplicateReasons } from './UpstreamAccountsPage.story-data'

export type StoryStore = {
  writesEnabled: boolean
  routing: PoolRoutingSettings
  accounts: UpstreamAccountSummary[]
  details: Record<number, UpstreamAccountDetail>
  groupNotes: Record<string, string>
  groupBoundProxyKeys: Record<string, string[]>
  forwardProxyNodes: ForwardProxyBindingNode[]
  nextId: number
  sessions: Record<
    string,
    LoginSessionStatusResponse & {
      displayName?: string
      groupName?: string
      isMother?: boolean
      note?: string
      groupNote?: string
      tagIds?: number[]
      mailboxSessionId?: string
      mailboxAddress?: string
      state?: string
    }
  >
  mailboxStatuses: Record<string, OauthMailboxStatus>
  nextMailboxId: number
  bulkSyncScenario: StoryBulkSyncScenario
  bulkSyncJobs: Record<string, StoryBulkSyncJob>
}

const defaultEffectiveRoutingRule: EffectiveRoutingRule = {
  guardEnabled: false,
  lookbackHours: null,
  maxConversations: null,
  allowCutOut: true,
  allowCutIn: true,
  sourceTagIds: [],
  sourceTagNames: [],
  guardRules: [],
}

const compactDefaultTags: AccountTagSummary[] = [
  {
    id: 1,
    name: 'vip',
    routingRule: defaultEffectiveRoutingRule,
  },
  {
    id: 2,
    name: 'burst-safe',
    routingRule: defaultEffectiveRoutingRule,
  },
  {
    id: 3,
    name: 'prod-apac',
    routingRule: defaultEffectiveRoutingRule,
  },
  {
    id: 4,
    name: 'sticky-pool',
    routingRule: defaultEffectiveRoutingRule,
  },
]

function buildRequestBuckets(seed: number, baseline: number, failuresEvery: number): ForwardProxyBindingNode['last24h'] {
  const start = Date.parse('2026-03-01T00:00:00.000Z')
  return Array.from({ length: 24 }, (_, index) => {
    const bucketStart = new Date(start + index * 3600_000).toISOString()
    const bucketEnd = new Date(start + (index + 1) * 3600_000).toISOString()
    return {
      bucketStart,
      bucketEnd,
      successCount: Math.max(0, Math.round(baseline + Math.sin((index + seed) / 2.3) * (baseline * 0.3))),
      failureCount: index % failuresEvery === 0 ? Math.max(0, 1 + ((seed + index) % 2)) : 0,
    }
  })
}

const directBindingKey = '__direct__'
const subscriptionSsKey =
  'ss://2022-blake3-aes-128-gcm:fixture-passphrase@fixture-ss-edge.example.invalid:443#Ivan-hinet-ss2022-01KF87EBR50MM9JKM9R9BCA9WZ'
const subscriptionVlessKey =
  'vless://11111111-2222-3333-4444-555555555555@fixture-vless-edge.example.invalid:443?encryption=none&security=tls&type=ws&host=cdn.example.invalid&path=%2Ffixture&fp=chrome&pbk=fixture-public-key&sid=fixture-subscription-node#Ivan-hinet-vless-vision-01KF874741GBN6MQYD6TNMYDVS'

const defaultForwardProxyNodes: ForwardProxyBindingNode[] = [
  {
    key: directBindingKey,
    source: 'direct',
    displayName: 'Direct',
    protocolLabel: 'DIRECT',
    penalized: false,
    selectable: true,
    last24h: buildRequestBuckets(0, 20, 8),
  },
  {
    key: 'jp-edge-01',
    source: 'manual',
    displayName: 'JP Edge 01',
    protocolLabel: 'HTTP',
    penalized: false,
    selectable: true,
    last24h: buildRequestBuckets(1, 18, 6),
  },
  {
    key: subscriptionSsKey,
    source: 'subscription',
    displayName: 'Ivan-hinet-ss2022-01KF87EBR50MM9JKM9R9BCA9WZ',
    protocolLabel: 'SS',
    penalized: false,
    selectable: true,
    last24h: buildRequestBuckets(7, 13, 5),
  },
  {
    key: subscriptionVlessKey,
    source: 'subscription',
    displayName: 'Ivan-hinet-vless-vision-01KF874741GBN6MQYD6TNMYDVS',
    protocolLabel: 'VLESS',
    penalized: true,
    selectable: true,
    last24h: buildRequestBuckets(13, 10, 4),
  },
  {
    key: 'drain-node',
    source: 'manual',
    displayName: 'Drain Node',
    protocolLabel: 'HTTP',
    penalized: true,
    selectable: false,
    last24h: buildRequestBuckets(17, 5, 3),
  },
]

const storyTagMap = {
  vip: compactDefaultTags[0],
  burstSafe: compactDefaultTags[1],
  prodApac: compactDefaultTags[2],
  stickyPool: compactDefaultTags[3],
  priority: {
    id: 20,
    name: 'priority-route',
    routingRule: defaultEffectiveRoutingRule,
  },
  analytics: {
    id: 21,
    name: 'analytics',
    routingRule: defaultEffectiveRoutingRule,
  },
  fallback: {
    id: 22,
    name: 'fallback',
    routingRule: defaultEffectiveRoutingRule,
  },
  sandbox: {
    id: 23,
    name: 'sandbox',
    routingRule: defaultEffectiveRoutingRule,
  },
  reporting: {
    id: 24,
    name: 'reporting',
    routingRule: defaultEffectiveRoutingRule,
  },
  rescue: { id: 25, name: 'rescue', routingRule: defaultEffectiveRoutingRule },
  canary: { id: 26, name: 'canary', routingRule: defaultEffectiveRoutingRule },
  overflow: {
    id: 27,
    name: 'overflow',
    routingRule: defaultEffectiveRoutingRule,
  },
  batch: { id: 28, name: 'batch', routingRule: defaultEffectiveRoutingRule },
  emea: { id: 29, name: 'emea', routingRule: defaultEffectiveRoutingRule },
} as const

type StoryTagKey = keyof typeof storyTagMap

export type StoryInitialEntry =
  | string
  | {
      pathname: string
      search?: string
      state?: unknown
    }

export type StoryBulkSyncScenario = 'success-auto-hide' | 'partial-failure' | null

export type StoryBulkSyncJob = {
  jobId: string
  snapshot: BulkUpstreamAccountSyncSnapshot
  counts: BulkUpstreamAccountSyncCounts
  error: string | null
}

export const now = '2026-03-17T12:30:00.000Z'
export const storyFutureExpiresAt = '2027-03-20T12:50:00.000Z'
export const storyFutureLoginExpiresAt = '2027-03-20T12:40:00.000Z'
const storyExistingRemoteMailboxes = new Set([
  'manual-existing@mail-tw.707079.xyz',
  'flow-oauth@mail-tw.707079.xyz',
  'pending-oauth@mail-tw.707079.xyz',
  'flow-batch@mail-tw.707079.xyz',
  'pending-batch@mail-tw.707079.xyz',
  'edited-batch@mail-tw.707079.xyz',
])
export const defaultRoutingMaintenance: PoolRoutingMaintenanceSettings = {
  primarySyncIntervalSecs: 300,
  secondarySyncIntervalSecs: 1800,
  priorityAvailableAccountCap: 100,
}

export const defaultRoutingTimeouts: PoolRoutingTimeoutSettings = {
  responsesFirstByteTimeoutSecs: 120,
  compactFirstByteTimeoutSecs: 300,
  responsesStreamTimeoutSecs: 300,
  compactStreamTimeoutSecs: 300,
}

const detailRichRoutingRule: EffectiveRoutingRule = {
  guardEnabled: true,
  lookbackHours: 24,
  maxConversations: 8,
  allowCutOut: false,
  allowCutIn: true,
  sourceTagIds: [1, 4, 20],
  sourceTagNames: ['vip', 'sticky-pool', 'priority-route'],
  guardRules: [
    {
      tagId: 1,
      tagName: 'vip',
      lookbackHours: 6,
      maxConversations: 4,
    },
    {
      tagId: 4,
      tagName: 'sticky-pool',
      lookbackHours: 24,
      maxConversations: 8,
    },
  ],
}

const detailRichRecentActions = [
  {
    id: 7001,
    occurredAt: '2026-03-17T12:22:00.000Z',
    action: 'route_hard_unavailable',
    source: 'call',
    reasonCode: 'upstream_http_429_quota_exhausted',
    reasonMessage: 'Weekly cap exhausted; traffic was moved to a sibling Tokyo lane.',
    httpStatus: 429,
    failureKind: 'upstream_http_429_quota_exhausted',
    invokeId: 'inv_story_pool_failover_001',
    stickyKey: '019ce3a1-6787-7910-b0fd-c246d6f6a901',
    createdAt: '2026-03-17T12:22:00.000Z',
  },
  {
    id: 7002,
    occurredAt: '2026-03-17T12:06:00.000Z',
    action: 'sync_recovery_blocked',
    source: 'sync_maintenance',
    reasonCode: 'quota_still_exhausted',
    reasonMessage: 'Maintenance sync confirmed that the 7-day quota was still exhausted.',
    httpStatus: 429,
    failureKind: 'upstream_http_429_quota_exhausted',
    invokeId: 'job_story_sync_maintenance_002',
    createdAt: '2026-03-17T12:06:00.000Z',
  },
  {
    id: 7003,
    occurredAt: '2026-03-17T11:42:00.000Z',
    action: 'sync_succeeded',
    source: 'sync_manual',
    reasonCode: 'sync_ok',
    reasonMessage: 'Manual sync refreshed the access token and compact capability snapshot.',
    httpStatus: null,
    failureKind: null,
    invokeId: 'job_story_sync_manual_003',
    createdAt: '2026-03-17T11:42:00.000Z',
  },
]

function atMinuteOffset(minutes: number) {
  return new Date(Date.parse(now) + minutes * 60_000).toISOString()
}

function buildWindow(
  percent: number,
  durationMins: number,
  usedText: string,
  limitText: string,
  resetsAt: string,
) {
  return {
    usedPercent: percent,
    usedText,
    limitText,
    resetsAt,
    windowDurationMins: durationMins,
  }
}

function buildRecentAction(
  id: number,
  occurredAt: string,
  action: string,
  source: string,
  reasonCode?: string | null,
  reasonMessage?: string | null,
  failureKind?: string | null,
  httpStatus?: number | null,
) {
  return {
    id,
    occurredAt,
    action,
    source,
    reasonCode: reasonCode ?? null,
    reasonMessage: reasonMessage ?? null,
    httpStatus: httpStatus ?? null,
    failureKind: failureKind ?? null,
    invokeId: null,
    stickyKey: null,
    createdAt: occurredAt,
  }
}

export function storyHasExistingMailboxAddress(store: StoryStore, requestedAddress: string) {
  if (storyExistingRemoteMailboxes.has(requestedAddress)) {
    return true
  }
  return Object.values(store.mailboxStatuses).some(
    (status) => status.emailAddress.trim().toLowerCase() === requestedAddress,
  )
}

function buildHistory(seed = 0) {
  return Array.from({ length: 7 }, (_, index) => ({
    capturedAt: new Date(
      Date.parse('2026-03-05T00:00:00.000Z') + index * 12 * 3600_000,
    ).toISOString(),
    primaryUsedPercent: Math.min(96, 18 + seed + index * 7),
    secondaryUsedPercent: Math.min(88, 8 + seed / 2 + index * 3),
    creditsBalance: (18.5 - index * 0.9).toFixed(2),
  }))
}

function pickStoryTags(...keys: StoryTagKey[]) {
  return keys.map((key) => storyTagMap[key])
}

function buildOauthUsage(primaryPercent: number, secondaryPercent: number) {
  return {
    primaryWindow: buildWindow(
      primaryPercent,
      300,
      `${primaryPercent}% used`,
      '5h rolling window',
      atMinuteOffset(90),
    ),
    secondaryWindow: buildWindow(
      secondaryPercent,
      10080,
      `${secondaryPercent}% used`,
      '7d rolling window',
      atMinuteOffset(3 * 24 * 60),
    ),
  }
}

function buildApiKeyUsage(
  primaryUsed: number,
  secondaryUsed: number,
  primaryLimit = 120,
  secondaryLimit = 500,
) {
  return {
    primaryWindow: buildWindow(
      Math.round((primaryUsed / primaryLimit) * 100),
      300,
      `${primaryUsed} requests`,
      `${primaryLimit} requests`,
      atMinuteOffset(90),
    ),
    secondaryWindow: buildWindow(
      Math.round((secondaryUsed / secondaryLimit) * 100),
      10080,
      `${secondaryUsed} requests`,
      `${secondaryLimit} requests`,
      atMinuteOffset(3 * 24 * 60),
    ),
    localLimits: {
      primaryLimit,
      secondaryLimit,
      limitUnit: 'requests',
    },
  }
}

function buildOperationalRosterAccounts(replicaCount = 1) {
  const baseSpecs: Array<{
    id: number
    kind: 'oauth_codex' | 'api_key_codex'
    displayName: string
    groupName?: string | null
    planType?: string | null
    tagKeys: StoryTagKey[]
  }> = [
    {
      id: 108,
      kind: 'oauth_codex',
      displayName: 'Codex Pro - Seoul',
      groupName: 'production-apac',
      planType: 'team',
      tagKeys: ['vip', 'prodApac', 'priority'],
    },
    {
      id: 109,
      kind: 'api_key_codex',
      displayName: 'Team key - analytics',
      groupName: 'analytics',
      planType: 'local',
      tagKeys: ['analytics', 'reporting'],
    },
    {
      id: 110,
      kind: 'oauth_codex',
      displayName: 'Codex Pro - Berlin',
      groupName: 'production-emea',
      planType: 'team',
      tagKeys: ['priority', 'emea'],
    },
    {
      id: 111,
      kind: 'api_key_codex',
      displayName: 'Overflow key - queue burst',
      groupName: 'overflow',
      planType: 'local',
      tagKeys: ['overflow', 'burstSafe', 'fallback'],
    },
    {
      id: 112,
      kind: 'oauth_codex',
      displayName: 'Codex Pro - Sydney',
      groupName: 'production-apac',
      planType: 'pro',
      tagKeys: ['prodApac', 'stickyPool'],
    },
    {
      id: 113,
      kind: 'api_key_codex',
      displayName: 'Sandbox key - canary',
      groupName: 'sandbox',
      planType: 'local',
      tagKeys: ['sandbox', 'canary'],
    },
    {
      id: 114,
      kind: 'oauth_codex',
      displayName: 'Codex Pro - Toronto',
      groupName: 'enterprise-ops',
      planType: 'enterprise',
      tagKeys: ['priority', 'reporting'],
    },
    {
      id: 115,
      kind: 'api_key_codex',
      displayName: 'Research key - evals',
      groupName: 'experiments',
      planType: 'local',
      tagKeys: ['analytics', 'sandbox'],
    },
    {
      id: 116,
      kind: 'oauth_codex',
      displayName: 'Codex Pro - London',
      groupName: 'production-emea',
      planType: 'team',
      tagKeys: ['vip', 'emea'],
    },
    {
      id: 117,
      kind: 'api_key_codex',
      displayName: 'Night shift key',
      groupName: 'night-ops',
      planType: 'local',
      tagKeys: ['fallback', 'overflow'],
    },
    {
      id: 118,
      kind: 'oauth_codex',
      displayName: 'Codex Pro - Mumbai',
      groupName: 'production-apac',
      planType: 'team',
      tagKeys: ['prodApac', 'burstSafe'],
    },
    {
      id: 119,
      kind: 'api_key_codex',
      displayName: 'Support key - rescue',
      groupName: 'rescue',
      planType: 'local',
      tagKeys: ['rescue', 'fallback'],
    },
    {
      id: 120,
      kind: 'oauth_codex',
      displayName: 'Codex Pro - Paris',
      groupName: 'production-emea',
      planType: 'pro',
      tagKeys: ['priority', 'emea'],
    },
    {
      id: 121,
      kind: 'api_key_codex',
      displayName: 'Batch runner key',
      groupName: 'batch-ops',
      planType: 'local',
      tagKeys: ['batch', 'reporting'],
    },
    {
      id: 122,
      kind: 'oauth_codex',
      displayName: 'Codex Pro - Sao Paulo',
      groupName: 'latam',
      planType: 'team',
      tagKeys: ['vip', 'priority'],
    },
    {
      id: 123,
      kind: 'api_key_codex',
      displayName: 'Queue key - overflow west',
      groupName: 'overflow',
      planType: 'local',
      tagKeys: ['overflow', 'burstSafe'],
    },
    {
      id: 124,
      kind: 'oauth_codex',
      displayName: 'Codex Pro - Frankfurt',
      groupName: 'production-emea',
      planType: 'team',
      tagKeys: ['stickyPool', 'emea'],
    },
    {
      id: 125,
      kind: 'api_key_codex',
      displayName: 'Staging key - eu',
      groupName: 'staging-eu',
      planType: 'local',
      tagKeys: ['canary', 'fallback'],
    },
    {
      id: 126,
      kind: 'oauth_codex',
      displayName: 'Codex Pro - Austin',
      groupName: null,
      planType: 'pro',
      tagKeys: ['vip'],
    },
    {
      id: 127,
      kind: 'api_key_codex',
      displayName: 'Migration key - ops',
      groupName: 'ops',
      planType: 'local',
      tagKeys: ['batch', 'reporting'],
    },
    {
      id: 128,
      kind: 'oauth_codex',
      displayName: 'Codex Pro - Melbourne',
      groupName: 'production-apac',
      planType: 'team',
      tagKeys: ['prodApac', 'priority'],
    },
    {
      id: 129,
      kind: 'api_key_codex',
      displayName: 'Fallback key - sandbox east',
      groupName: 'sandbox',
      planType: 'local',
      tagKeys: ['sandbox', 'fallback'],
    },
  ]

  const specs = Array.from(
    { length: Math.max(1, replicaCount) },
    (_, replicaIndex) =>
      baseSpecs.map((spec, baseIndex) => ({
        ...spec,
        id: spec.id + replicaIndex * 100,
        displayName:
          replicaIndex === 0
            ? spec.displayName
            : `${spec.displayName} · lane ${replicaIndex + 1}`,
        replicaIndex,
        baseIndex,
      })),
  ).flat()

  return specs.map((spec, index) => {
    const pattern = index % 7
    const commonOverrides: Partial<UpstreamAccountDetail> = {
      displayName: spec.displayName,
      groupName: spec.groupName ?? null,
      isMother: false,
      planType: spec.planType ?? null,
      tags: pickStoryTags(...spec.tagKeys),
      lastSyncedAt: atMinuteOffset(-(index * 17 + 8)),
      lastSuccessfulSyncAt: atMinuteOffset(-(index * 17 + 10)),
      lastActivityAt: atMinuteOffset(-(index * 13 + 4)),
      email: spec.kind === 'oauth_codex' ? `mock-${spec.id}@example.com` : null,
      chatgptAccountId:
        spec.kind === 'oauth_codex' ? `org_mock_${spec.id}` : null,
      chatgptUserId:
        spec.kind === 'oauth_codex' ? `user_mock_${spec.id}` : null,
      maskedApiKey:
        spec.kind === 'api_key_codex'
          ? `sk-live••••••${String(spec.id).padStart(4, '0').slice(-4)}`
          : null,
      note: `${spec.displayName} mock account for pagination, filtering, and bulk selection coverage.`,
    }

    const statusOverrides: Partial<UpstreamAccountDetail> =
      spec.kind === 'oauth_codex'
        ? [
            {
              status: 'active',
              displayStatus: 'active',
              enabled: true,
              enableStatus: 'enabled',
              workStatus: 'working',
              healthStatus: 'normal',
              syncState: 'idle',
              lastError: null,
              lastErrorAt: null,
              ...buildOauthUsage(58, 19),
            },
            {
              status: 'active',
              displayStatus: 'active',
              enabled: true,
              enableStatus: 'enabled',
              workStatus: 'rate_limited',
              healthStatus: 'normal',
              syncState: 'idle',
              lastError: null,
              lastErrorAt: null,
              ...buildOauthUsage(82, 44),
            },
            {
              status: 'syncing',
              displayStatus: 'syncing',
              enabled: true,
              enableStatus: 'enabled',
              workStatus: 'idle',
              healthStatus: 'normal',
              syncState: 'syncing',
              lastError: null,
              lastErrorAt: null,
              ...buildOauthUsage(36, 17),
            },
            {
              status: 'needs_reauth',
              displayStatus: 'needs_reauth',
              enabled: true,
              enableStatus: 'enabled',
              workStatus: 'idle',
              healthStatus: 'needs_reauth',
              syncState: 'idle',
              lastError: 'refresh token expired',
              lastErrorAt: atMinuteOffset(-(index * 7 + 3)),
              ...buildOauthUsage(74, 53),
            },
            {
              status: 'active',
              displayStatus: 'upstream_unavailable',
              enabled: true,
              enableStatus: 'enabled',
              workStatus: 'idle',
              healthStatus: 'upstream_unavailable',
              syncState: 'idle',
              lastError: 'upstream timeout',
              lastErrorAt: atMinuteOffset(-(index * 5 + 2)),
              ...buildOauthUsage(88, 61),
            },
            {
              status: 'active',
              displayStatus: 'active',
              enabled: true,
              enableStatus: 'enabled',
              workStatus: 'idle',
              healthStatus: 'normal',
              syncState: 'idle',
              lastError: null,
              lastErrorAt: null,
              ...buildOauthUsage(14, 6),
            },
            {
              status: 'disabled',
              displayStatus: 'disabled',
              enabled: false,
              enableStatus: 'disabled',
              workStatus: 'idle',
              healthStatus: 'normal',
              syncState: 'idle',
              lastError: null,
              lastErrorAt: null,
              ...buildOauthUsage(0, 0),
            },
          ][pattern]
        : [
            {
              status: 'active',
              displayStatus: 'active',
              enabled: true,
              enableStatus: 'enabled',
              workStatus: 'working',
              healthStatus: 'normal',
              syncState: 'idle',
              lastError: null,
              lastErrorAt: null,
              ...buildApiKeyUsage(28, 138),
            },
            {
              status: 'active',
              displayStatus: 'active',
              enabled: true,
              enableStatus: 'enabled',
              workStatus: 'rate_limited',
              healthStatus: 'normal',
              syncState: 'idle',
              lastError: null,
              lastErrorAt: null,
              ...buildApiKeyUsage(97, 402),
            },
            {
              status: 'syncing',
              displayStatus: 'syncing',
              enabled: true,
              enableStatus: 'enabled',
              workStatus: 'idle',
              healthStatus: 'normal',
              syncState: 'syncing',
              lastError: null,
              lastErrorAt: null,
              ...buildApiKeyUsage(32, 150),
            },
            {
              status: 'needs_reauth',
              displayStatus: 'needs_reauth',
              enabled: true,
              enableStatus: 'enabled',
              workStatus: 'idle',
              healthStatus: 'needs_reauth',
              syncState: 'idle',
              lastError: 'refresh token expired',
              lastErrorAt: atMinuteOffset(-(index * 7 + 3)),
              ...buildApiKeyUsage(104, 431),
            },
            {
              status: 'active',
              displayStatus: 'upstream_unavailable',
              enabled: true,
              enableStatus: 'enabled',
              workStatus: 'idle',
              healthStatus: 'upstream_unavailable',
              syncState: 'idle',
              lastError: 'upstream timeout',
              lastErrorAt: atMinuteOffset(-(index * 5 + 2)),
              ...buildApiKeyUsage(118, 500),
            },
            {
              status: 'active',
              displayStatus: 'active',
              enabled: true,
              enableStatus: 'enabled',
              workStatus: 'idle',
              healthStatus: 'normal',
              syncState: 'idle',
              lastError: null,
              lastErrorAt: null,
              ...buildApiKeyUsage(11, 52),
            },
            {
              status: 'disabled',
              displayStatus: 'disabled',
              enabled: false,
              enableStatus: 'disabled',
              workStatus: 'idle',
              healthStatus: 'normal',
              syncState: 'idle',
              lastError: null,
              lastErrorAt: null,
              ...buildApiKeyUsage(0, 0),
            },
          ][pattern]

    return spec.kind === 'oauth_codex'
      ? createOauthAccount(spec.id, { ...commonOverrides, ...statusOverrides })
      : createApiKeyAccount(spec.id, {
          ...commonOverrides,
          ...statusOverrides,
        })
  })
}

export function clone<T>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T
}

export function buildBulkSyncCounts(rows: BulkUpstreamAccountSyncRow[]): BulkUpstreamAccountSyncCounts {
  return rows.reduce<BulkUpstreamAccountSyncCounts>((counts, row) => {
    counts.total += 1
    if (row.status === 'succeeded') {
      counts.completed += 1
      counts.succeeded += 1
    } else if (row.status === 'failed') {
      counts.completed += 1
      counts.failed += 1
    } else if (row.status === 'skipped') {
      counts.completed += 1
      counts.skipped += 1
    }
    return counts
  }, {
    total: 0,
    completed: 0,
    succeeded: 0,
    failed: 0,
    skipped: 0,
  })
}

export function createBulkSyncRows(store: StoryStore, accountIds: number[]): BulkUpstreamAccountSyncRow[] {
  return accountIds.map((accountId) => {
    const account = store.details[accountId] ?? store.accounts.find((item) => item.id === accountId)
    return {
      accountId,
      displayName: account?.displayName ?? `Account ${accountId}`,
      status: 'pending',
      detail: null,
    }
  })
}

export function buildBulkSyncSnapshot(
  jobId: string,
  rows: BulkUpstreamAccountSyncRow[],
  status: BulkUpstreamAccountSyncSnapshot['status'] = 'running',
): BulkUpstreamAccountSyncSnapshot {
  return {
    jobId,
    status,
    rows: clone(rows),
  }
}

export function buildBulkSyncSnapshotEvent(job: StoryBulkSyncJob) {
  return {
    snapshot: clone(job.snapshot),
    counts: clone(job.counts),
  }
}

export function buildBulkSyncRowEvent(
  row: BulkUpstreamAccountSyncRow,
  counts: BulkUpstreamAccountSyncCounts,
) {
  return {
    row: clone(row),
    counts: clone(counts),
  }
}

export function updateStoryBulkSyncJob(
  job: StoryBulkSyncJob,
  rows: BulkUpstreamAccountSyncRow[],
  status: BulkUpstreamAccountSyncSnapshot['status'] = 'running',
  error: string | null = null,
) {
  job.snapshot = buildBulkSyncSnapshot(job.jobId, rows, status)
  job.counts = buildBulkSyncCounts(job.snapshot.rows)
  job.error = error
}

export function normalizeGroupName(value?: string | null) {
  const trimmed = value?.trim() ?? ''
  return trimmed || null
}

export function storyEnableStatus(
  item: Pick<
    UpstreamAccountSummary,
    'enableStatus' | 'enabled' | 'displayStatus'
  >,
) {
  if (typeof item.enableStatus === 'string' && item.enableStatus)
    return item.enableStatus
  return item.enabled === false || item.displayStatus === 'disabled'
    ? 'disabled'
    : 'enabled'
}

export function storyHealthStatus(
  item: Pick<
    UpstreamAccountSummary,
    'healthStatus' | 'displayStatus' | 'status'
  >,
) {
  const legacyStatus = item.displayStatus ?? item.status ?? 'error_other'
  if (
    legacyStatus === 'needs_reauth' ||
    legacyStatus === 'upstream_unavailable' ||
    legacyStatus === 'upstream_rejected' ||
    legacyStatus === 'error_other'
  ) {
    return legacyStatus
  }
  if (legacyStatus === 'error') return 'error_other'
  return 'normal'
}

export function storySyncState(
  item: Pick<UpstreamAccountSummary, 'syncState' | 'displayStatus' | 'status'>,
) {
  return item.status === 'syncing' || item.displayStatus === 'syncing'
    ? 'syncing'
    : 'idle'
}

export function storyWorkStatus(
  item: Pick<
    UpstreamAccountSummary,
    | 'workStatus'
    | 'enableStatus'
    | 'enabled'
    | 'displayStatus'
    | 'status'
    | 'healthStatus'
    | 'syncState'
  >,
  healthStatus: string,
  syncState: string,
) {
  if (storyEnableStatus(item) === 'disabled') return 'idle'
  if (syncState === 'syncing') return 'idle'
  if (item.workStatus === 'degraded') return 'degraded'
  if (item.workStatus === 'rate_limited') return 'rate_limited'
  if (healthStatus !== 'normal') return 'unavailable'
  return typeof item.workStatus === 'string' && item.workStatus
    ? item.workStatus
    : 'idle'
}

function storyDisplayStatus(
  item: Pick<
    UpstreamAccountSummary,
    | 'displayStatus'
    | 'healthStatus'
    | 'syncState'
    | 'enableStatus'
    | 'enabled'
    | 'status'
  >,
) {
  if (typeof item.displayStatus === 'string' && item.displayStatus)
    return item.displayStatus
  if (storyEnableStatus(item) === 'disabled') return 'disabled'
  if (storySyncState(item) === 'syncing') return 'syncing'
  const healthStatus = storyHealthStatus(item)
  if (healthStatus !== 'normal') return healthStatus
  return 'active'
}

function withDerivedStatusFields<T extends UpstreamAccountDetail>(
  detail: T,
): T {
  const enableStatus = storyEnableStatus(detail)
  const healthStatus = storyHealthStatus(detail)
  const syncState = storySyncState(detail)
  const workStatus = storyWorkStatus(detail, healthStatus, syncState)
  return {
    ...detail,
    enableStatus,
    workStatus,
    healthStatus,
    syncState,
    displayStatus: storyDisplayStatus(detail),
  }
}

export function listGroupSummaries(store: StoryStore) {
  const names = new Set<string>()
  for (const account of store.accounts) {
    const groupName = normalizeGroupName(account.groupName)
    if (groupName) names.add(groupName)
  }
  for (const groupName of Object.keys(store.groupNotes)) {
    const normalized = normalizeGroupName(groupName)
    if (normalized) names.add(normalized)
  }
  for (const groupName of Object.keys(store.groupBoundProxyKeys)) {
    const normalized = normalizeGroupName(groupName)
    if (normalized) names.add(normalized)
  }
  return Array.from(names)
    .sort((left, right) => left.localeCompare(right))
    .map((groupName) => ({
      groupName,
      note: store.groupNotes[groupName] ?? null,
      boundProxyKeys: [...(store.groupBoundProxyKeys[groupName] ?? [])],
    }))
}

export function listTagSummaries(store: StoryStore): TagSummary[] {
  const summaries = new Map<number, TagSummary>()
  const accountIdsByTag = new Map<number, Set<number>>()
  const groupNamesByTag = new Map<number, Set<string>>()

  for (const account of store.accounts) {
    const groupName = normalizeGroupName(account.groupName)
    for (const tag of account.tags) {
      if (!summaries.has(tag.id)) {
        summaries.set(tag.id, {
          id: tag.id,
          name: tag.name,
          routingRule: clone(tag.routingRule),
          accountCount: 0,
          groupCount: 0,
          updatedAt: now,
        })
      }
      const accountIds = accountIdsByTag.get(tag.id) ?? new Set<number>()
      accountIds.add(account.id)
      accountIdsByTag.set(tag.id, accountIds)
      const groupNames = groupNamesByTag.get(tag.id) ?? new Set<string>()
      if (groupName) {
        groupNames.add(groupName)
      }
      groupNamesByTag.set(tag.id, groupNames)
    }
  }

  return Array.from(summaries.values())
    .map((tag) => ({
      ...tag,
      accountCount: accountIdsByTag.get(tag.id)?.size ?? 0,
      groupCount: groupNamesByTag.get(tag.id)?.size ?? 0,
    }))
    .sort((left, right) => left.name.localeCompare(right.name))
}

export function filterAccountsForQuery(store: StoryStore, url: URL) {
  const groupSearch = (url.searchParams.get('groupSearch') || '')
    .trim()
    .toLowerCase()
  const groupUngrouped = url.searchParams.get('groupUngrouped') === 'true'
  const tagIds = url.searchParams
    .getAll('tagIds')
    .map((value) => Number(value))
    .filter(Number.isFinite)
  const workStatuses = url.searchParams
    .getAll('workStatus')
    .map((value) => value.trim())
    .filter((value) => value.length > 0)
  const enableStatuses = url.searchParams
    .getAll('enableStatus')
    .map((value) => value.trim())
    .filter((value) => value.length > 0)
  const healthStatuses = url.searchParams
    .getAll('healthStatus')
    .map((value) => value.trim())
    .filter((value) => value.length > 0)

  return store.accounts.filter((account) => {
    const normalizedGroup =
      normalizeGroupName(account.groupName)?.toLowerCase() ?? ''
    const derivedHealthStatus = storyHealthStatus(account)
    const derivedSyncState = storySyncState(account)
    const derivedWorkStatus = storyWorkStatus(
      account,
      derivedHealthStatus,
      derivedSyncState,
    )
    const matchesGroup = groupUngrouped
      ? !normalizeGroupName(account.groupName)
      : groupSearch
        ? normalizedGroup.includes(groupSearch)
        : true
    if (!matchesGroup) return false
    if (workStatuses.length > 0 && !workStatuses.includes(derivedWorkStatus)) return false
    if (enableStatuses.length > 0 && !enableStatuses.includes(storyEnableStatus(account)))
      return false
    if (healthStatuses.length > 0 && !healthStatuses.includes(derivedHealthStatus)) return false
    if (tagIds.length === 0) return true
    const accountTagIds = new Set(account.tags.map((tag) => tag.id))
    return tagIds.every((tagId) => accountTagIds.has(tagId))
  })
}

export function createOauthAccount(
  id: number,
  overrides?: Partial<UpstreamAccountDetail>,
): UpstreamAccountDetail {
  const detail: UpstreamAccountDetail = {
    id,
    kind: 'oauth_codex',
    provider: 'codex',
    displayName: 'Codex Pro - Tokyo',
    groupName: 'production',
    isMother: true,
    status: 'active',
    displayStatus: 'active',
    enabled: true,
    enableStatus: 'enabled',
    workStatus: 'working',
    healthStatus: 'normal',
    syncState: 'idle',
    email: 'tokyo@example.com',
    chatgptAccountId: 'org_tokyo',
    chatgptUserId: 'user_tokyo',
    planType: 'pro',
    lastSyncedAt: now,
    lastSuccessfulSyncAt: now,
    lastActivityAt: '2026-03-11T12:12:00.000Z',
    activeConversationCount: 3,
    lastRefreshedAt: now,
    tokenExpiresAt: '2026-03-12T12:30:00.000Z',
    lastError: null,
    lastErrorAt: null,
    primaryWindow: buildWindow(
      64,
      300,
      '64% used',
      '5h rolling window',
      '2026-03-11T14:00:00.000Z',
    ),
    secondaryWindow: buildWindow(
      22,
      10080,
      '22% used',
      '7d rolling window',
      '2026-03-18T00:00:00.000Z',
    ),
    credits: {
      hasCredits: true,
      unlimited: false,
      balance: '11.80',
    },
    compactSupport: {
      status: 'unsupported',
      observedAt: '2026-03-17T12:18:00.000Z',
      reason:
        'No available channel for model gpt-5.4-openai-compact under group default.',
    },
    localLimits: {
      primaryLimit: null,
      secondaryLimit: null,
      limitUnit: 'requests',
    },
    tags: [],
    effectiveRoutingRule: defaultEffectiveRoutingRule,
    note: 'Primary team account for premium traffic.',
    maskedApiKey: null,
    history: buildHistory(2),
  }
  return withDerivedStatusFields({
    ...detail,
    ...overrides,
    history: overrides?.history ?? detail.history,
    recentActions: overrides?.recentActions ?? detail.recentActions,
  })
}

export function createApiKeyAccount(
  id: number,
  overrides?: Partial<UpstreamAccountDetail>,
): UpstreamAccountDetail {
  const primaryLimit = overrides?.localLimits?.primaryLimit ?? 120
  const secondaryLimit = overrides?.localLimits?.secondaryLimit ?? 500
  const limitUnit = overrides?.localLimits?.limitUnit ?? 'requests'
  const detail: UpstreamAccountDetail = {
    id,
    kind: 'api_key_codex',
    provider: 'codex',
    displayName: 'Team key - staging',
    groupName: 'staging',
    isMother: false,
    status: 'active',
    displayStatus: 'active',
    enabled: true,
    enableStatus: 'enabled',
    workStatus: 'rate_limited',
    healthStatus: 'normal',
    syncState: 'idle',
    email: null,
    chatgptAccountId: null,
    chatgptUserId: null,
    planType: 'local',
    maskedApiKey: 'sk-live••••••c9f2',
    lastSyncedAt: now,
    lastSuccessfulSyncAt: now,
    lastActivityAt: '2026-03-11T12:24:00.000Z',
    activeConversationCount: 0,
    lastRefreshedAt: null,
    tokenExpiresAt: null,
    lastError: null,
    lastErrorAt: null,
    primaryWindow: buildWindow(
      0,
      300,
      `0 ${limitUnit}`,
      `${primaryLimit} ${limitUnit}`,
      '2026-03-11T14:00:00.000Z',
    ),
    secondaryWindow: buildWindow(
      0,
      10080,
      `0 ${limitUnit}`,
      `${secondaryLimit} ${limitUnit}`,
      '2026-03-18T00:00:00.000Z',
    ),
    credits: {
      hasCredits: false,
      unlimited: false,
      balance: null,
    },
    compactSupport: {
      status: 'supported',
      observedAt: '2026-03-17T12:06:00.000Z',
      reason: 'Observed success for /v1/responses/compact.',
    },
    localLimits: {
      primaryLimit,
      secondaryLimit,
      limitUnit,
    },
    tags: [],
    effectiveRoutingRule: defaultEffectiveRoutingRule,
    upstreamBaseUrl: 'https://proxy.example.com/gateway',
    note: 'Fallback API key before router metrics land.',
    history: buildHistory(0).map((point) => ({
      ...point,
      primaryUsedPercent: 0,
      secondaryUsedPercent: 0,
      creditsBalance: null,
    })),
  }
  return withDerivedStatusFields({
    ...detail,
    ...overrides,
    history: overrides?.history ?? detail.history,
    recentActions: overrides?.recentActions ?? detail.recentActions,
  })
}

export function toSummary(detail: UpstreamAccountDetail): UpstreamAccountSummary {
  const normalized = withDerivedStatusFields(detail)
  return {
    id: normalized.id,
    kind: normalized.kind,
    provider: normalized.provider,
    displayName: normalized.displayName,
    groupName: normalized.groupName,
    isMother: normalized.isMother,
    status: normalized.status,
    displayStatus: normalized.displayStatus,
    enabled: normalized.enabled,
    enableStatus: normalized.enableStatus,
    workStatus: normalized.workStatus,
    healthStatus: normalized.healthStatus,
    syncState: normalized.syncState,
    email: normalized.email,
    chatgptAccountId: normalized.chatgptAccountId,
    planType: normalized.planType,
    maskedApiKey: normalized.maskedApiKey,
    lastSyncedAt: normalized.lastSyncedAt,
    lastSuccessfulSyncAt: normalized.lastSuccessfulSyncAt,
    lastActivityAt: normalized.lastActivityAt,
    activeConversationCount: normalized.activeConversationCount ?? 0,
    lastError: normalized.lastError,
    lastErrorAt: normalized.lastErrorAt,
    lastAction: normalized.lastAction,
    lastActionSource: normalized.lastActionSource,
    lastActionReasonCode: normalized.lastActionReasonCode,
    lastActionReasonMessage: normalized.lastActionReasonMessage,
    lastActionHttpStatus: normalized.lastActionHttpStatus,
    lastActionInvokeId: normalized.lastActionInvokeId,
    lastActionAt: normalized.lastActionAt,
    tokenExpiresAt: normalized.tokenExpiresAt,
    primaryWindow: normalized.primaryWindow,
    secondaryWindow: normalized.secondaryWindow,
    credits: normalized.credits,
    localLimits: normalized.localLimits,
    compactSupport: normalized.compactSupport,
    duplicateInfo: normalized.duplicateInfo,
    tags: normalized.tags,
    effectiveRoutingRule: normalized.effectiveRoutingRule,
  }
}

export function currentStoryId() {
  if (typeof window === 'undefined') return null
  const params = new URLSearchParams(window.location.search)
  return params.get('id')
}

export function createStore(): StoryStore {
  const storyId = currentStoryId()
  const duplicateStory =
    storyId?.endsWith('--duplicate-oauth-warning') === true ||
    storyId?.endsWith('--duplicate-oauth-detail') === true
  const mixedPlanCoexistenceStory =
    storyId?.endsWith('--mixed-plan-coexistence') === true
  const teamSharedOrgCoexistenceStory =
    storyId?.endsWith('--team-shared-org-coexistence') === true
  const compactStory = storyId?.endsWith('--compact-long-labels') === true
  const tagFilterStory = storyId?.endsWith('--tag-filter-all-match') === true
  const availabilityBadgeStory =
    storyId?.endsWith('--availability-badges') === true
  const oauthRetryTerminalStateStory =
    storyId?.endsWith('--oauth-retry-terminal-state') === true
  const quotaExhaustedOauthStory =
    storyId?.endsWith('--quota-exhausted-oauth') === true
  const upstreamRejected402Story =
    storyId?.endsWith('--upstream-rejected-402') === true
  const degradedWorkStatusStory =
    storyId?.endsWith('--degraded-work-status-filter') === true
  const unavailableWorkStatusStory =
    storyId?.endsWith('--unavailable-work-status-filter') === true
  const missingWindowPlaceholdersStory =
    storyId?.endsWith('--missing-window-placeholders') === true
  const bulkSyncSuccessStory =
    storyId?.endsWith('--bulk-sync-success-auto-hide') === true
  const bulkSyncFailureStory =
    storyId?.endsWith('--bulk-sync-failure-dismiss') === true
  const denseRosterStory =
    storyId?.endsWith('--dense-roster') === true ||
    storyId?.endsWith('--operational') === true ||
    storyId?.endsWith('--status-filters') === true ||
    storyId?.endsWith('--bulk-selection') === true ||
    storyId?.endsWith('--slow-page-switch') === true

  const baseOauthStoryOverrides: Partial<UpstreamAccountDetail> = {
    tags: pickStoryTags('vip', 'stickyPool', 'priority'),
    effectiveRoutingRule: detailRichRoutingRule,
    lastAction: 'route_hard_unavailable',
    lastActionSource: 'call',
    lastActionReasonCode: 'upstream_http_429_quota_exhausted',
    lastActionReasonMessage: 'Weekly cap exhausted; traffic was moved to a sibling Tokyo lane.',
    lastActionHttpStatus: 429,
    lastActionInvokeId: 'inv_story_pool_failover_001',
    lastActionAt: '2026-03-17T12:22:00.000Z',
    recentActions: detailRichRecentActions,
  }

  const oauth = createOauthAccount(101, {
    ...baseOauthStoryOverrides,
    ...(duplicateStory
      ? {
          duplicateInfo: {
            peerAccountIds: [103],
            reasons: [...duplicateReasons],
          },
          note: 'Primary team account sharing the same upstream identity.',
        }
      : mixedPlanCoexistenceStory
        ? {
            displayName: 'Fixture Billing Team',
            email: 'team@billing-fixture.example.invalid',
            chatgptAccountId: 'fixture_shared_billing_org',
            chatgptUserId: 'fixture_shared_billing_user',
            planType: 'team',
            duplicateInfo: null,
            note: 'Synthetic team-billed OAuth fixture sharing the same upstream identity intentionally.',
          }
      : teamSharedOrgCoexistenceStory
        ? {
            displayName: 'Fixture Team Mother',
            email: 'mother@team-fixture.example.invalid',
            groupName: 'fixture-team',
            isMother: true,
            chatgptAccountId: 'fixture_shared_team_org',
            chatgptUserId: 'fixture_team_owner',
            planType: 'team',
            duplicateInfo: null,
            note: 'Synthetic mother account for the shared team org fixture.',
          }
      : compactStory
        ? {
            displayName:
              'Codex Pro - Tokyo enterprise rotation account with a deliberately long roster title',
            groupName: 'production-apac-primary-operators',
            tags: [
              compactDefaultTags[0],
              compactDefaultTags[1],
              compactDefaultTags[2],
              compactDefaultTags[3],
            ],
          }
        : tagFilterStory
          ? {
              tags: [
                compactDefaultTags[0],
              compactDefaultTags[1],
              compactDefaultTags[2],
            ],
          }
        : undefined),
  })
  const apiKey = createApiKeyAccount(
    102,
    missingWindowPlaceholdersStory
      ? {
          displayName: 'Team key - missing weekly limit',
          primaryWindow: buildWindow(
            18,
            300,
            '18 requests',
            '120 requests',
            '2026-03-11T13:00:00.000Z',
          ),
          secondaryWindow: null,
          localLimits: {
            primaryLimit: 120,
            secondaryLimit: null,
            limitUnit: 'requests',
          },
          history: buildHistory(0).map((point) => ({
            ...point,
            primaryUsedPercent: 18,
            secondaryUsedPercent: null,
            creditsBalance: null,
          })),
          note: 'Secondary quota window is intentionally missing in this story.',
        }
      : compactStory
        ? {
            enabled: false,
            enableStatus: 'disabled',
            workStatus: 'idle',
            healthStatus: 'normal',
            syncState: 'idle',
            status: 'disabled',
            displayStatus: 'disabled',
            lastError: null,
            lastErrorAt: null,
            tags: [
              compactDefaultTags[0],
              compactDefaultTags[1],
              compactDefaultTags[2],
              compactDefaultTags[3],
            ],
          }
        : tagFilterStory
          ? {
              tags: [compactDefaultTags[0], compactDefaultTags[3]],
            }
          : undefined,
  )
  const duplicateOauth =
    duplicateStory || mixedPlanCoexistenceStory || teamSharedOrgCoexistenceStory
      ? createOauthAccount(103, {
          displayName: mixedPlanCoexistenceStory
            ? 'Fixture Billing Free'
            : teamSharedOrgCoexistenceStory
              ? 'Fixture Team Member'
            : 'Codex Pro - Seoul',
          email: mixedPlanCoexistenceStory
            ? 'free@billing-fixture.example.invalid'
            : teamSharedOrgCoexistenceStory
              ? 'member@team-fixture.example.invalid'
            : 'seoul@example.com',
          chatgptAccountId: mixedPlanCoexistenceStory
            ? 'fixture_shared_billing_org'
            : teamSharedOrgCoexistenceStory
              ? 'fixture_shared_team_org'
            : 'org_tokyo',
          chatgptUserId: mixedPlanCoexistenceStory
            ? 'fixture_shared_billing_user'
            : teamSharedOrgCoexistenceStory
              ? 'fixture_team_member'
            : 'user_tokyo',
          groupName: teamSharedOrgCoexistenceStory ? 'fixture-team' : 'production',
          planType: mixedPlanCoexistenceStory
            ? 'free'
            : teamSharedOrgCoexistenceStory
              ? 'team'
              : 'pro',
          isMother: false,
          duplicateInfo: duplicateStory
            ? {
                peerAccountIds: [101],
                reasons: [...duplicateReasons],
              }
            : null,
          note: mixedPlanCoexistenceStory
            ? 'Synthetic personal-billed OAuth fixture sharing the same upstream identity intentionally.'
            : teamSharedOrgCoexistenceStory
              ? 'Synthetic sibling team member sharing the same upstream org intentionally.'
            : 'Sibling OAuth account kept for duplicate identity review.',
        })
      : null
  const compactExtraAccounts = compactStory
    ? [
        createOauthAccount(104, {
          displayName: 'Codex Pro - Singapore weekly ceiling watch',
          groupName: 'production-apac-weekly',
          isMother: false,
          status: 'active',
          displayStatus: 'active',
          enableStatus: 'enabled',
          workStatus: 'working',
          healthStatus: 'normal',
          syncState: 'idle',
          planType: 'team',
          lastSuccessfulSyncAt: '2026-03-11T20:10:00.000Z',
          lastActivityAt: '2026-03-11T20:08:00.000Z',
          primaryWindow: buildWindow(
            71,
            300,
            '71% used',
            '5h rolling window',
            '2026-03-11T22:10:00.000Z',
          ),
          secondaryWindow: buildWindow(
            100,
            10080,
            '100% used',
            '7d rolling window',
            '2026-03-18T08:00:00.000Z',
          ),
          tags: [
            compactDefaultTags[0],
            compactDefaultTags[1],
            {
              id: 7,
              name: 'weekly-cap',
              routingRule: defaultEffectiveRoutingRule,
            },
          ],
          note: 'Weekly window is fully exhausted while the 5h window still has room.',
        }),
        createOauthAccount(105, {
          displayName: 'Codex Pro - Osaka burst limit exhausted',
          groupName: 'production-apac-burst',
          isMother: false,
          status: 'syncing',
          displayStatus: 'syncing',
          enableStatus: 'enabled',
          workStatus: 'rate_limited',
          healthStatus: 'normal',
          syncState: 'syncing',
          planType: 'team',
          lastSuccessfulSyncAt: '2026-03-11T19:58:00.000Z',
          lastActivityAt: '2026-03-11T19:56:00.000Z',
          primaryWindow: buildWindow(
            100,
            300,
            '100% used',
            '5h rolling window',
            '2026-03-11T21:42:00.000Z',
          ),
          secondaryWindow: buildWindow(
            46,
            10080,
            '46% used',
            '7d rolling window',
            '2026-03-18T08:00:00.000Z',
          ),
          tags: [
            compactDefaultTags[0],
            {
              id: 8,
              name: 'burst-limit',
              routingRule: defaultEffectiveRoutingRule,
            },
            {
              id: 9,
              name: 'warm-spare',
              routingRule: defaultEffectiveRoutingRule,
            },
          ],
          note: 'Burst traffic consumed the full 5h budget.',
        }),
        createApiKeyAccount(106, {
          displayName: 'Backup key - weekly redline',
          groupName: 'staging-overflow',
          status: 'active',
          displayStatus: 'upstream_unavailable',
          enabled: true,
          enableStatus: 'enabled',
          workStatus: 'rate_limited',
          healthStatus: 'upstream_unavailable',
          syncState: 'idle',
          planType: 'local',
          lastSuccessfulSyncAt: '2026-03-11T19:42:00.000Z',
          lastActivityAt: '2026-03-11T20:18:00.000Z',
          primaryWindow: buildWindow(
            93,
            300,
            '112 requests',
            '120 requests',
            '2026-03-11T21:30:00.000Z',
          ),
          secondaryWindow: buildWindow(
            100,
            10080,
            '500 requests',
            '500 requests',
            '2026-03-18T08:00:00.000Z',
          ),
          tags: [
            {
              id: 10,
              name: 'overflow',
              routingRule: defaultEffectiveRoutingRule,
            },
            {
              id: 11,
              name: 'weekly-redline',
              routingRule: defaultEffectiveRoutingRule,
            },
            compactDefaultTags[1],
          ],
          note: 'Fallback key with the weekly allowance fully consumed.',
        }),
        createApiKeyAccount(107, {
          displayName: 'Emergency key - both windows saturated',
          groupName: 'rescue',
          status: 'needs_reauth',
          enabled: true,
          displayStatus: 'needs_reauth',
          enableStatus: 'enabled',
          workStatus: 'rate_limited',
          healthStatus: 'needs_reauth',
          syncState: 'idle',
          planType: 'local',
          lastSuccessfulSyncAt: '2026-03-11T18:55:00.000Z',
          lastActivityAt: '2026-03-11T19:14:00.000Z',
          primaryWindow: buildWindow(
            100,
            300,
            '120 requests',
            '120 requests',
            '2026-03-11T20:40:00.000Z',
          ),
          secondaryWindow: buildWindow(
            100,
            10080,
            '500 requests',
            '500 requests',
            '2026-03-18T08:00:00.000Z',
          ),
          tags: [
            {
              id: 12,
              name: 'rescue',
              routingRule: defaultEffectiveRoutingRule,
            },
            {
              id: 13,
              name: 'manual-drain',
              routingRule: defaultEffectiveRoutingRule,
            },
          ],
          note: 'Emergency key where both local placeholder windows are exhausted.',
        }),
      ]
    : []
  const availabilityBadgeAccounts = availabilityBadgeStory
    ? [
        createOauthAccount(201, {
          displayName: 'Availability working badge',
          groupName: 'production',
          isMother: false,
          workStatus: 'working',
          activeConversationCount: 3,
          tags: pickStoryTags('vip', 'priority'),
        }),
        createApiKeyAccount(202, {
          displayName: 'Availability idle badge',
          groupName: 'staging',
          workStatus: 'idle',
          activeConversationCount: 0,
          tags: pickStoryTags('fallback'),
        }),
        createApiKeyAccount(203, {
          displayName: 'Availability rate limited visible',
          groupName: 'overflow',
          workStatus: 'rate_limited',
          activeConversationCount: 6,
          tags: pickStoryTags('overflow'),
        }),
        createOauthAccount(204, {
          displayName: 'Availability unavailable hidden',
          groupName: 'rescue',
          isMother: false,
          status: 'active',
          displayStatus: 'upstream_unavailable',
          enableStatus: 'enabled',
          workStatus: 'working',
          healthStatus: 'upstream_unavailable',
          syncState: 'idle',
          activeConversationCount: 2,
          tags: pickStoryTags('rescue'),
        }),
      ]
    : []
  const quotaExhaustedOauthAccounts = quotaExhaustedOauthStory
    ? [
        createOauthAccount(301, {
          displayName: 'Quota exhausted OAuth routing state',
          groupName: 'production',
          isMother: false,
          status: 'error',
          displayStatus: 'active',
          enableStatus: 'enabled',
          workStatus: 'rate_limited',
          healthStatus: 'normal',
          syncState: 'idle',
          planType: 'team',
          email: 'tokyo@example.com',
          chatgptAccountId: 'org_tokyo',
          chatgptUserId: 'user_tokyo',
          lastSuccessfulSyncAt: '2026-03-24T19:52:00.000Z',
          lastActivityAt: '2026-03-25T00:31:43.000Z',
          lastError:
            'oauth_upstream_rejected_request: pool upstream responded with 429: The usage limit has been reached',
          lastErrorAt: '2026-03-25T00:31:43.000Z',
          lastAction: 'sync_recovery_blocked',
          lastActionSource: 'sync_maintenance',
          lastActionReasonCode: 'quota_still_exhausted',
          lastActionReasonMessage:
            'latest usage snapshot still shows an exhausted upstream usage limit window',
          lastActionAt: '2026-03-25T02:00:27.000Z',
          lastActionHttpStatus: null,
          primaryWindow: buildWindow(
            100,
            300,
            '100% used',
            '5h rolling window',
            '2026-03-31T00:06:33.000Z',
          ),
          secondaryWindow: buildWindow(
            64,
            10080,
            '64% used',
            '7d rolling window',
            '2026-04-01T00:06:33.000Z',
          ),
          tags: pickStoryTags('vip', 'prodApac', 'priority'),
          note: 'Quota exhausted OAuth account should stay visible as rate limited, not upstream rejected.',
          recentActions: [
            buildRecentAction(
              9101,
              '2026-03-25T02:00:27.000Z',
              'sync_recovery_blocked',
              'sync_maintenance',
              'quota_still_exhausted',
              'latest usage snapshot still shows an exhausted upstream usage limit window',
              'upstream_http_429_quota_exhausted',
              null,
            ),
            buildRecentAction(
              9100,
              '2026-03-25T00:31:43.000Z',
              'route_hard_unavailable',
              'call',
              'upstream_http_429_quota_exhausted',
              'oauth_upstream_rejected_request: pool upstream responded with 429: The usage limit has been reached',
              'upstream_http_429_quota_exhausted',
              429,
            ),
          ],
        }),
      ]
    : []
  const oauthRetryTerminalStateAccounts = oauthRetryTerminalStateStory
    ? [
        createOauthAccount(401, {
          displayName: 'Retry refresh failure settled as needs reauth',
          groupName: 'production',
          isMother: false,
          status: 'needs_reauth',
          displayStatus: 'needs_reauth',
          enableStatus: 'enabled',
          workStatus: 'idle',
          healthStatus: 'needs_reauth',
          syncState: 'idle',
          planType: 'team',
          email: 'retry-needs-reauth@example.com',
          chatgptAccountId: 'org_retry_terminal',
          chatgptUserId: 'user_retry_terminal',
          lastSuccessfulSyncAt: '2026-03-25T01:40:00.000Z',
          lastActivityAt: '2026-03-25T02:01:00.000Z',
          lastError:
            'upstream usage snapshot request returned 403 Forbidden: Authentication token has been invalidated, please sign in again',
          lastErrorAt: '2026-03-25T02:04:00.000Z',
          lastAction: 'sync_failed',
          lastActionSource: 'sync_maintenance',
          lastActionReasonCode: 'reauth_required',
          lastActionReasonMessage:
            'upstream usage snapshot request returned 403 Forbidden: Authentication token has been invalidated, please sign in again',
          lastActionAt: '2026-03-25T02:04:00.000Z',
          lastActionHttpStatus: 403,
          primaryWindow: buildWindow(
            52,
            300,
            '52% used',
            '5h rolling window',
            '2026-03-25T06:40:00.000Z',
          ),
          secondaryWindow: buildWindow(
            40,
            10080,
            '40% used',
            '7d rolling window',
            '2026-04-01T00:00:00.000Z',
          ),
          tags: pickStoryTags('vip', 'canary'),
          note: 'Retry-after-refresh failure should settle into a terminal state instead of showing syncing forever.',
          recentActions: [
            buildRecentAction(
              9201,
              '2026-03-25T02:04:00.000Z',
              'sync_failed',
              'sync_maintenance',
              'reauth_required',
              'upstream usage snapshot request returned 403 Forbidden: Authentication token has been invalidated, please sign in again',
              'upstream_http_auth',
              403,
            ),
            buildRecentAction(
              9200,
              '2026-03-25T01:40:00.000Z',
              'sync_succeeded',
              'sync_manual',
              'sync_ok',
              'Manual sync refreshed the access token and capability snapshot.',
              null,
              null,
            ),
          ],
        }),
      ]
    : []
  const upstreamRejected402Accounts = upstreamRejected402Story
    ? [
        createOauthAccount(501, {
          displayName: 'Workspace deactivated 402 routing state',
          groupName: 'production',
          isMother: false,
          status: 'error',
          displayStatus: 'upstream_rejected',
          enableStatus: 'enabled',
          workStatus: 'unavailable',
          healthStatus: 'upstream_rejected',
          syncState: 'idle',
          planType: 'team',
          email: 'workspace-blocked@example.com',
          chatgptAccountId: 'org_workspace_blocked',
          chatgptUserId: 'user_workspace_blocked',
          lastSuccessfulSyncAt: '2026-03-26T07:59:42.000Z',
          lastActivityAt: '2026-03-26T08:11:47.000Z',
          lastError:
            'initial usage snapshot attempt with configured user agent failed: usage endpoint returned 402 Payment Required: {"detail":{"code":"deactivated_workspace"}}',
          lastErrorAt: '2026-03-26T08:11:47.000Z',
          lastAction: 'sync_hard_unavailable',
          lastActionSource: 'sync_maintenance',
          lastActionReasonCode: 'upstream_http_402',
          lastActionReasonMessage:
            'initial usage snapshot attempt with configured user agent failed: usage endpoint returned 402 Payment Required: {"detail":{"code":"deactivated_workspace"}}',
          lastActionAt: '2026-03-26T08:11:47.000Z',
          lastActionHttpStatus: 402,
          lastActionInvokeId: 'inv_story_workspace_402',
          primaryWindow: buildWindow(
            38,
            300,
            '38% used',
            '5h rolling window',
            '2026-03-26T11:59:42.000Z',
          ),
          secondaryWindow: buildWindow(
            19,
            10080,
            '19% used',
            '7d rolling window',
            '2026-04-02T00:00:00.000Z',
          ),
          tags: pickStoryTags('vip', 'prodApac'),
          note: 'A stale quota event exists in history, but the current 402 workspace deactivation must still render as upstream rejected + unavailable.',
          recentActions: [
            buildRecentAction(
              9301,
              '2026-03-26T08:11:47.000Z',
              'sync_hard_unavailable',
              'sync_maintenance',
              'upstream_http_402',
              'initial usage snapshot attempt with configured user agent failed: usage endpoint returned 402 Payment Required: {"detail":{"code":"deactivated_workspace"}}',
              'upstream_http_402',
              402,
            ),
            buildRecentAction(
              9300,
              '2026-03-26T08:01:12.000Z',
              'route_hard_unavailable',
              'call',
              'upstream_http_429_quota_exhausted',
              'Weekly cap exhausted on the previous routing attempt before maintenance retried the account.',
              'upstream_http_429_quota_exhausted',
              429,
            ),
            buildRecentAction(
              9299,
              '2026-03-26T07:59:42.000Z',
              'sync_succeeded',
              'sync_manual',
              'sync_ok',
              'Manual sync refreshed the account before the workspace was deactivated.',
              null,
              null,
            ),
          ],
        }),
      ]
    : []
  const unavailableWorkStatusAccounts = unavailableWorkStatusStory
    ? [
        createOauthAccount(601, {
          displayName: 'Needs reauth unavailable work status',
          groupName: 'rescue',
          isMother: false,
          status: 'needs_reauth',
          displayStatus: 'needs_reauth',
          enableStatus: 'enabled',
          workStatus: 'unavailable',
          healthStatus: 'needs_reauth',
          syncState: 'idle',
          lastError: 'refresh token expired',
          lastErrorAt: '2026-03-27T08:11:47.000Z',
          tags: pickStoryTags('rescue'),
        }),
        createOauthAccount(602, {
          displayName: 'Upstream unavailable work status',
          groupName: 'rescue',
          isMother: false,
          status: 'active',
          displayStatus: 'upstream_unavailable',
          enableStatus: 'enabled',
          workStatus: 'unavailable',
          healthStatus: 'upstream_unavailable',
          syncState: 'idle',
          lastError: 'gateway temporarily unavailable',
          lastErrorAt: '2026-03-27T08:21:47.000Z',
          tags: pickStoryTags('rescue'),
        }),
        createOauthAccount(603, {
          displayName: 'Upstream rejected unavailable work status',
          groupName: 'rescue',
          isMother: false,
          status: 'error',
          displayStatus: 'upstream_rejected',
          enableStatus: 'enabled',
          workStatus: 'unavailable',
          healthStatus: 'upstream_rejected',
          syncState: 'idle',
          lastError: 'oauth bridge upstream rejected request: 403 forbidden',
          lastErrorAt: '2026-03-27T08:31:47.000Z',
          tags: pickStoryTags('rescue'),
        }),
        createApiKeyAccount(604, {
          displayName: 'Rate limited filter control',
          groupName: 'overflow',
          workStatus: 'rate_limited',
          healthStatus: 'normal',
          syncState: 'idle',
          tags: pickStoryTags('overflow'),
        }),
      ]
    : []
  const degradedWorkStatusAccounts = degradedWorkStatusStory
    ? [
        createOauthAccount(611, {
          displayName: 'Plain 429 degraded work status',
          groupName: 'production',
          isMother: false,
          status: 'active',
          displayStatus: 'active',
          enableStatus: 'enabled',
          workStatus: 'degraded',
          healthStatus: 'normal',
          syncState: 'idle',
          lastError: 'pool upstream responded with 429: too many requests',
          lastErrorAt: '2026-03-30T08:31:47.000Z',
          lastAction: 'route_retryable_failure',
          lastActionSource: 'call',
          lastActionReasonCode: 'upstream_http_429_rate_limit',
          lastActionReasonMessage: 'pool upstream responded with 429: too many requests',
          lastActionHttpStatus: 429,
          lastActionAt: '2026-03-30T08:31:47.000Z',
          tags: pickStoryTags('vip', 'priority'),
        }),
        createOauthAccount(612, {
          displayName: '5xx degraded work status',
          groupName: 'production',
          isMother: false,
          status: 'active',
          displayStatus: 'active',
          enableStatus: 'enabled',
          workStatus: 'degraded',
          healthStatus: 'normal',
          syncState: 'idle',
          lastError: 'pool upstream responded with 503 service unavailable',
          lastErrorAt: '2026-03-30T08:32:47.000Z',
          lastAction: 'route_retryable_failure',
          lastActionSource: 'call',
          lastActionReasonCode: 'upstream_http_5xx',
          lastActionReasonMessage: 'pool upstream responded with 503 service unavailable',
          lastActionHttpStatus: 503,
          lastActionAt: '2026-03-30T08:32:47.000Z',
          tags: pickStoryTags('prodApac'),
        }),
        createApiKeyAccount(613, {
          displayName: 'Healthy filter control',
          groupName: 'staging',
          workStatus: 'working',
          healthStatus: 'normal',
          syncState: 'idle',
          activeConversationCount: 2,
          tags: pickStoryTags('fallback'),
        }),
      ]
    : []
  const operationalRosterAccounts = compactStory
    ? []
    : buildOperationalRosterAccounts(denseRosterStory ? 3 : 1)
  const storyAccounts = oauthRetryTerminalStateStory
    ? oauthRetryTerminalStateAccounts
    : quotaExhaustedOauthStory
    ? quotaExhaustedOauthAccounts
    : upstreamRejected402Story
    ? upstreamRejected402Accounts
    : degradedWorkStatusStory
    ? degradedWorkStatusAccounts
    : unavailableWorkStatusStory
    ? unavailableWorkStatusAccounts
    : availabilityBadgeStory
      ? availabilityBadgeAccounts
      : [
          oauth,
          ...(duplicateOauth ? [duplicateOauth] : []),
          apiKey,
          ...compactExtraAccounts,
          ...operationalRosterAccounts,
        ]
  const accounts = storyAccounts.map(toSummary)
  const details = Object.fromEntries(
    storyAccounts.map((account) => [account.id, account]),
  )
  return {
    writesEnabled: true,
    routing: {
      writesEnabled: true,
      apiKeyConfigured: true,
      maskedApiKey: 'fixture-pool••••••demo',
      maintenance: clone(defaultRoutingMaintenance),
      timeouts: clone(defaultRoutingTimeouts),
    },
    forwardProxyNodes: clone(defaultForwardProxyNodes),
    groupNotes: {
      production: 'Premium traffic group note.',
      staging: 'Staging fallback group note.',
      'production-apac-weekly': 'Weekly cap watch list.',
      'production-apac-burst': 'Burst-heavy rotation group.',
      'production-apac':
        'APAC production roster for regional failover and premium traffic.',
      'production-emea':
        'EMEA production roster with mixed OAuth and API key coverage.',
      analytics: 'Analytics workloads with lower latency sensitivity.',
      overflow:
        'Overflow keys reserved for burst absorption and emergency routing.',
      sandbox: 'Sandbox and canary accounts used for smoke traffic.',
      'enterprise-ops':
        'Enterprise workspace accounts for higher-tier traffic.',
      experiments:
        'Evaluation and research traffic that can tolerate instability.',
      'night-ops': 'Night shift routing accounts for off-hours coverage.',
      'batch-ops': 'Batch processing keys used by scheduled jobs.',
      latam: 'LATAM fallback coverage for regional traffic.',
      'staging-eu': 'European staging accounts and shared API keys.',
      ops: 'Internal operational accounts for migration and support tooling.',
      'staging-overflow': 'Fallback keys that often ride the weekly edge.',
      rescue: 'Emergency pool for overflow and incident recovery.',
    },
    groupBoundProxyKeys: {
      production: [directBindingKey, subscriptionVlessKey],
      staging: ['drain-node'],
      overflow: ['missing-node-legacy'],
    },
    accounts,
    details,
    nextId: Math.max(...Object.keys(details).map((value) => Number(value))) + 1,
    sessions: {},
    mailboxStatuses: {},
    nextMailboxId: 1,
    bulkSyncScenario: bulkSyncSuccessStory
      ? 'success-auto-hide'
      : bulkSyncFailureStory
        ? 'partial-failure'
        : null,
    bulkSyncJobs: {},
  }
}

export function maskApiKey(value: string) {
  const trimmed = value.trim()
  if (!trimmed) return 'sk-empty••••'
  const suffix = trimmed.slice(-4)
  return `sk-live••••••${suffix}`
}



export function syncLocalWindows(detail: UpstreamAccountDetail) {
  if (detail.kind !== 'api_key_codex') return withDerivedStatusFields(detail)
  const primaryLimit = detail.localLimits?.primaryLimit ?? 120
  const secondaryLimit = detail.localLimits?.secondaryLimit ?? 500
  const limitUnit = detail.localLimits?.limitUnit ?? 'requests'
  return withDerivedStatusFields({
    ...detail,
    primaryWindow: buildWindow(
      0,
      300,
      `0 ${limitUnit}`,
      `${primaryLimit} ${limitUnit}`,
      '2026-03-11T14:00:00.000Z',
    ),
    secondaryWindow: buildWindow(
      0,
      10080,
      `0 ${limitUnit}`,
      `${secondaryLimit} ${limitUnit}`,
      '2026-03-18T00:00:00.000Z',
    ),
  })
}
