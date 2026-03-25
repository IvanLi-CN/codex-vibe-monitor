import { useEffect, useRef, type ReactNode } from 'react'
import { MemoryRouter, Route, Routes } from 'react-router-dom'
import { useTheme } from '../theme/context'
import type {
  AccountTagSummary,
  CreateApiKeyAccountPayload,
  CompleteOauthLoginSessionPayload,
  EffectiveRoutingRule,
  LoginSessionStatusResponse,
  OauthMailboxStatus,
  PoolRoutingMaintenanceSettings,
  PoolRoutingSettings,
  PoolRoutingTimeoutSettings,
  TagSummary,
  UpdateOauthLoginSessionPayload,
  UpdatePoolRoutingSettingsPayload,
  UpdateUpstreamAccountGroupPayload,
  UpdateUpstreamAccountPayload,
  UpstreamAccountDetail,
  UpstreamAccountListResponse,
  UpstreamAccountSummary,
} from '../lib/api'
import AccountPoolLayout from '../pages/account-pool/AccountPoolLayout'
import UpstreamAccountCreatePage from '../pages/account-pool/UpstreamAccountCreate'
import UpstreamAccountsPage from '../pages/account-pool/UpstreamAccounts'
import { duplicateReasons } from './UpstreamAccountsPage.story-data'

type StoryStore = {
  writesEnabled: boolean
  routing: PoolRoutingSettings
  accounts: UpstreamAccountSummary[]
  details: Record<number, UpstreamAccountDetail>
  groupNotes: Record<string, string>
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

const now = '2026-03-17T12:30:00.000Z'
const storyFutureExpiresAt = '2027-03-20T12:50:00.000Z'
const storyFutureLoginExpiresAt = '2027-03-20T12:40:00.000Z'
const storyExistingRemoteMailboxes = new Set([
  'manual-existing@mail-tw.707079.xyz',
  'flow-oauth@mail-tw.707079.xyz',
  'pending-oauth@mail-tw.707079.xyz',
  'flow-batch@mail-tw.707079.xyz',
  'pending-batch@mail-tw.707079.xyz',
  'edited-batch@mail-tw.707079.xyz',
])
const defaultRoutingMaintenance: PoolRoutingMaintenanceSettings = {
  primarySyncIntervalSecs: 300,
  secondarySyncIntervalSecs: 1800,
  priorityAvailableAccountCap: 100,
}

const defaultRoutingTimeouts: PoolRoutingTimeoutSettings = {
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

function storyHasExistingMailboxAddress(store: StoryStore, requestedAddress: string) {
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

function clone<T>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T
}

function normalizeGroupName(value?: string | null) {
  const trimmed = value?.trim() ?? ''
  return trimmed || null
}

function storyEnableStatus(
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

function storyHealthStatus(
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

function storySyncState(
  item: Pick<UpstreamAccountSummary, 'syncState' | 'displayStatus' | 'status'>,
) {
  return item.status === 'syncing' || item.displayStatus === 'syncing'
    ? 'syncing'
    : 'idle'
}

function storyWorkStatus(
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
  if (healthStatus !== 'normal') return 'idle'
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

function listGroupSummaries(store: StoryStore) {
  const names = new Set<string>()
  for (const account of store.accounts) {
    const groupName = normalizeGroupName(account.groupName)
    if (groupName) names.add(groupName)
  }
  return Array.from(names)
    .sort((left, right) => left.localeCompare(right))
    .map((groupName) => ({
      groupName,
      note: store.groupNotes[groupName] ?? null,
    }))
}

function listTagSummaries(store: StoryStore): TagSummary[] {
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

function filterAccountsForQuery(store: StoryStore, url: URL) {
  const groupSearch = (url.searchParams.get('groupSearch') || '')
    .trim()
    .toLowerCase()
  const groupUngrouped = url.searchParams.get('groupUngrouped') === 'true'
  const tagIds = url.searchParams
    .getAll('tagIds')
    .map((value) => Number(value))
    .filter(Number.isFinite)
  const workStatus = (url.searchParams.get('workStatus') || '').trim()
  const enableStatus = (url.searchParams.get('enableStatus') || '').trim()
  const healthStatus = (url.searchParams.get('healthStatus') || '').trim()

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
    if (workStatus && derivedWorkStatus !== workStatus) return false
    if (enableStatus && storyEnableStatus(account) !== enableStatus)
      return false
    if (healthStatus && derivedHealthStatus !== healthStatus) return false
    if (tagIds.length === 0) return true
    const accountTagIds = new Set(account.tags.map((tag) => tag.id))
    return tagIds.every((tagId) => accountTagIds.has(tagId))
  })
}

function createOauthAccount(
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

function createApiKeyAccount(
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

function toSummary(detail: UpstreamAccountDetail): UpstreamAccountSummary {
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

function currentStoryId() {
  if (typeof window === 'undefined') return null
  const params = new URLSearchParams(window.location.search)
  return params.get('id')
}

function createStore(): StoryStore {
  const storyId = currentStoryId()
  const duplicateStory =
    storyId?.endsWith('--duplicate-oauth-warning') === true ||
    storyId?.endsWith('--duplicate-oauth-detail') === true
  const compactStory = storyId?.endsWith('--compact-long-labels') === true
  const tagFilterStory = storyId?.endsWith('--tag-filter-all-match') === true
  const availabilityBadgeStory =
    storyId?.endsWith('--availability-badges') === true
  const oauthRetryTerminalStateStory =
    storyId?.endsWith('--oauth-retry-terminal-state') === true
  const quotaExhaustedOauthStory =
    storyId?.endsWith('--quota-exhausted-oauth') === true
  const denseRosterStory =
    storyId?.endsWith('--dense-roster') === true ||
    storyId?.endsWith('--operational') === true ||
    storyId?.endsWith('--status-filters') === true ||
    storyId?.endsWith('--bulk-selection') === true

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
    compactStory
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
  const duplicateOauth = duplicateStory
    ? createOauthAccount(103, {
        displayName: 'Codex Pro - Seoul',
        email: 'seoul@example.com',
        chatgptAccountId: 'org_tokyo',
        chatgptUserId: 'user_tokyo',
        groupName: 'production',
        duplicateInfo: {
          peerAccountIds: [101],
          reasons: [...duplicateReasons],
        },
        note: 'Sibling OAuth account kept for duplicate identity review.',
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
  const operationalRosterAccounts = compactStory
    ? []
    : buildOperationalRosterAccounts(denseRosterStory ? 3 : 1)
  const storyAccounts = oauthRetryTerminalStateStory
    ? oauthRetryTerminalStateAccounts
    : quotaExhaustedOauthStory
    ? quotaExhaustedOauthAccounts
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
      maskedApiKey: 'pool-live••••••c0de',
      maintenance: clone(defaultRoutingMaintenance),
      timeouts: clone(defaultRoutingTimeouts),
    },
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
    accounts,
    details,
    nextId: Math.max(...Object.keys(details).map((value) => Number(value))) + 1,
    sessions: {},
    mailboxStatuses: {},
    nextMailboxId: 1,
  }
}

function maskApiKey(value: string) {
  const trimmed = value.trim()
  if (!trimmed) return 'sk-empty••••'
  const suffix = trimmed.slice(-4)
  return `sk-live••••••${suffix}`
}

function buildStickyRequestPoints(
  points: Array<{
    occurredAt: string
    requestTokens: number
    status?: string
    isSuccess?: boolean
  }>,
) {
  let cumulativeTokens = 0
  return points.map((point) => {
    cumulativeTokens += point.requestTokens
    return {
      occurredAt: point.occurredAt,
      status: point.status ?? 'success',
      isSuccess: point.isSuccess ?? true,
      requestTokens: point.requestTokens,
      cumulativeTokens,
    }
  })
}

function buildStickyConversations(accountId: number) {
  const stickyKeys =
    accountId === 101
      ? [
          {
            stickyKey: '019ce3a1-6787-7910-b0fd-c246d6f6a901',
            requestCount: 10,
            totalTokens: 455_170,
            totalCost: 0.3507,
            createdAt: '2026-03-13T04:01:20.000Z',
            lastActivityAt: '2026-03-13T04:03:02.000Z',
            last24hRequests: buildStickyRequestPoints([
              {
                occurredAt: '2026-03-12T10:15:00.000Z',
                requestTokens: 102_440,
              },
              {
                occurredAt: '2026-03-12T18:20:00.000Z',
                requestTokens: 154_380,
              },
              {
                occurredAt: '2026-03-13T04:03:02.000Z',
                requestTokens: 198_350,
              },
            ]),
          },
          {
            stickyKey: '019ce3a0-cf52-7740-bec5-611a0c6af442',
            requestCount: 12,
            totalTokens: 629_175,
            totalCost: 0.4101,
            createdAt: '2026-03-13T03:59:52.000Z',
            lastActivityAt: '2026-03-13T04:06:08.000Z',
            last24hRequests: buildStickyRequestPoints([
              {
                occurredAt: '2026-03-12T12:10:00.000Z',
                requestTokens: 140_000,
              },
              {
                occurredAt: '2026-03-12T20:45:00.000Z',
                requestTokens: 212_875,
              },
              {
                occurredAt: '2026-03-13T04:06:08.000Z',
                requestTokens: 276_300,
              },
            ]),
          },
          {
            stickyKey: '019ce3a0-10a2-7c40-ba26-6f3358f44c77',
            requestCount: 5,
            totalTokens: 398_199,
            totalCost: 0.7543,
            createdAt: '2026-03-13T03:57:28.000Z',
            lastActivityAt: '2026-03-13T04:00:52.000Z',
            last24hRequests: buildStickyRequestPoints([
              {
                occurredAt: '2026-03-12T09:00:00.000Z',
                requestTokens: 120_000,
              },
              {
                occurredAt: '2026-03-12T21:40:00.000Z',
                requestTokens: 131_400,
              },
              {
                occurredAt: '2026-03-13T04:00:52.000Z',
                requestTokens: 146_799,
              },
            ]),
          },
          {
            stickyKey: '019ce39e-4ab3-7452-9cc3-3c51ad9088c1',
            requestCount: 23,
            totalTokens: 1_302_244,
            totalCost: 0.7238,
            createdAt: '2026-03-13T03:55:36.000Z',
            lastActivityAt: '2026-03-13T04:01:05.000Z',
            last24hRequests: buildStickyRequestPoints([
              {
                occurredAt: '2026-03-12T08:25:00.000Z',
                requestTokens: 330_000,
              },
              {
                occurredAt: '2026-03-12T17:15:00.000Z',
                requestTokens: 445_120,
              },
              {
                occurredAt: '2026-03-13T01:48:00.000Z',
                requestTokens: 268_624,
              },
              {
                occurredAt: '2026-03-13T04:01:05.000Z',
                requestTokens: 258_500,
              },
            ]),
          },
          {
            stickyKey: '019ce39a-6cfa-7b90-8e96-6de7e6076b02',
            requestCount: 20,
            totalTokens: 1_289_447,
            totalCost: 0.7022,
            createdAt: '2026-03-13T03:51:19.000Z',
            lastActivityAt: '2026-03-13T03:54:08.000Z',
            last24hRequests: buildStickyRequestPoints([
              {
                occurredAt: '2026-03-12T07:52:00.000Z',
                requestTokens: 281_000,
              },
              {
                occurredAt: '2026-03-12T13:04:00.000Z',
                requestTokens: 309_447,
              },
              {
                occurredAt: '2026-03-12T23:15:00.000Z',
                requestTokens: 334_000,
              },
              {
                occurredAt: '2026-03-13T03:54:08.000Z',
                requestTokens: 365_000,
                status: 'failed',
                isSuccess: false,
              },
            ]),
          },
          {
            stickyKey: '019ce397-7b0c-7240-9096-0b0e2a97d57a',
            requestCount: 35,
            totalTokens: 3_241_662,
            totalCost: 1.4563,
            createdAt: '2026-03-13T03:48:11.000Z',
            lastActivityAt: '2026-03-13T03:56:06.000Z',
            last24hRequests: buildStickyRequestPoints([
              {
                occurredAt: '2026-03-12T06:18:00.000Z',
                requestTokens: 640_000,
              },
              {
                occurredAt: '2026-03-12T11:42:00.000Z',
                requestTokens: 722_516,
              },
              {
                occurredAt: '2026-03-12T19:36:00.000Z',
                requestTokens: 841_900,
              },
              {
                occurredAt: '2026-03-13T03:56:06.000Z',
                requestTokens: 1_037_246,
              },
            ]),
          },
          {
            stickyKey: '019ce395-2299-7641-a0d6-c2ac4b6d9184',
            requestCount: 23,
            totalTokens: 1_455_961,
            totalCost: 1.0577,
            createdAt: '2026-03-13T03:45:33.000Z',
            lastActivityAt: '2026-03-13T03:53:28.000Z',
            last24hRequests: buildStickyRequestPoints([
              {
                occurredAt: '2026-03-12T05:10:00.000Z',
                requestTokens: 340_000,
              },
              {
                occurredAt: '2026-03-12T15:10:00.000Z',
                requestTokens: 462_400,
              },
              {
                occurredAt: '2026-03-12T22:00:00.000Z',
                requestTokens: 299_561,
              },
              {
                occurredAt: '2026-03-13T03:53:28.000Z',
                requestTokens: 354_000,
              },
            ]),
          },
        ]
      : [
          {
            stickyKey: '019ce3f1-7aa2-74b2-a762-145ec7cfe001',
            requestCount: 8,
            totalTokens: 122_440,
            totalCost: 0.1184,
            createdAt: '2026-03-13T02:44:00.000Z',
            lastActivityAt: '2026-03-13T03:14:00.000Z',
            last24hRequests: buildStickyRequestPoints([
              { occurredAt: '2026-03-12T18:00:00.000Z', requestTokens: 28_440 },
              { occurredAt: '2026-03-13T01:00:00.000Z', requestTokens: 44_000 },
              { occurredAt: '2026-03-13T03:14:00.000Z', requestTokens: 50_000 },
            ]),
          },
          {
            stickyKey: '019ce3f1-7aa2-74b2-a762-145ec7cfe002',
            requestCount: 11,
            totalTokens: 164_920,
            totalCost: 0.1542,
            createdAt: '2026-03-13T02:21:00.000Z',
            lastActivityAt: '2026-03-13T03:09:00.000Z',
            last24hRequests: buildStickyRequestPoints([
              { occurredAt: '2026-03-12T16:45:00.000Z', requestTokens: 38_120 },
              { occurredAt: '2026-03-13T00:32:00.000Z', requestTokens: 52_400 },
              { occurredAt: '2026-03-13T03:09:00.000Z', requestTokens: 74_400 },
            ]),
          },
        ]
  return {
    rangeStart: '2026-03-12T04:00:00.000Z',
    rangeEnd: '2026-03-13T04:10:00.000Z',
    conversations: stickyKeys,
  }
}

function jsonResponse(payload: unknown, status = 200) {
  return Promise.resolve(
    new Response(JSON.stringify(payload), {
      status,
      headers: { 'Content-Type': 'application/json' },
    }),
  )
}

function wait(ms: number) {
  return new Promise((resolve) => {
    window.setTimeout(resolve, ms)
  })
}

function noContent() {
  return Promise.resolve(new Response(null, { status: 204 }))
}

function parseBody<T>(raw: BodyInit | null | undefined, fallback: T): T {
  if (typeof raw !== 'string' || !raw) return fallback
  try {
    return JSON.parse(raw) as T
  } catch {
    return fallback
  }
}

function syncLocalWindows(detail: UpstreamAccountDetail) {
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

export function StorybookUpstreamAccountsMock({
  children,
}: {
  children: ReactNode
}) {
  const storeRef = useRef<StoryStore>(createStore())
  const originalFetchRef = useRef<typeof window.fetch | null>(null)
  const installedRef = useRef(false)

  if (typeof window !== 'undefined' && !installedRef.current) {
    installedRef.current = true
    originalFetchRef.current = window.fetch.bind(window)

    const mockedFetch: typeof window.fetch = async (input, init) => {
      const method = (
        init?.method || (input instanceof Request ? input.method : 'GET')
      ).toUpperCase()
      const inputUrl =
        typeof input === 'string'
          ? input
          : input instanceof URL
            ? input.toString()
            : input.url
      const parsedUrl = new URL(inputUrl, window.location.origin)
      const path = parsedUrl.pathname
      const storyId = currentStoryId()
      const store = storeRef.current

      if (path === '/api/pool/upstream-accounts' && method === 'GET') {
        const filteredItems = filterAccountsForQuery(store, parsedUrl)
        const rawPageSize = Number(parsedUrl.searchParams.get('pageSize') || 20)
        const requestedPageSize =
          Number.isFinite(rawPageSize) && rawPageSize > 0 ? rawPageSize : 20
        const total = filteredItems.length
        const pageCount = Math.max(1, Math.ceil(total / requestedPageSize))
        const rawPage = Number(parsedUrl.searchParams.get('page') || 1)
        const requestedPage =
          Number.isFinite(rawPage) && rawPage > 0 ? rawPage : 1
        const page = Math.min(requestedPage, pageCount)
        const start = (page - 1) * requestedPageSize
        const pageItems = filteredItems
          .slice(start, start + requestedPageSize)
          .map((item) => clone(item))
        const payload: UpstreamAccountListResponse = {
          writesEnabled: store.writesEnabled,
          groups: listGroupSummaries(store),
          hasUngroupedAccounts: store.accounts.some(
            (account) => !normalizeGroupName(account.groupName),
          ),
          routing: clone(store.routing),
          items: pageItems,
          total,
          page,
          pageSize: requestedPageSize,
          metrics: {
            total,
            oauth: filteredItems.filter((item) => item.kind === 'oauth_codex')
              .length,
            apiKey: filteredItems.filter(
              (item) => item.kind === 'api_key_codex',
            ).length,
            attention: filteredItems.filter((item) => {
              const derivedHealthStatus = storyHealthStatus(item)
              const derivedSyncState = storySyncState(item)
              return (
                derivedHealthStatus !== 'normal' ||
                storyWorkStatus(item, derivedHealthStatus, derivedSyncState) ===
                  'rate_limited'
              )
            }).length,
          },
        }
        return jsonResponse(payload)
      }

      if (path === '/api/pool/tags' && method === 'GET') {
        return jsonResponse({
          writesEnabled: store.writesEnabled,
          items: listTagSummaries(store),
        })
      }

      if (path === '/api/pool/routing-settings' && method === 'PUT') {
        const body = parseBody<UpdatePoolRoutingSettingsPayload>(init?.body, {})
        const trimmed = body.apiKey?.trim()
        store.routing = {
          ...store.routing,
          ...(trimmed
            ? {
                apiKeyConfigured: true,
                maskedApiKey: maskApiKey(trimmed),
              }
            : {}),
          ...(body.maintenance
            ? {
                maintenance: {
                  primarySyncIntervalSecs:
                    body.maintenance.primarySyncIntervalSecs ??
                    store.routing.maintenance?.primarySyncIntervalSecs ??
                    defaultRoutingMaintenance.primarySyncIntervalSecs,
                  secondarySyncIntervalSecs:
                    body.maintenance.secondarySyncIntervalSecs ??
                    store.routing.maintenance?.secondarySyncIntervalSecs ??
                    defaultRoutingMaintenance.secondarySyncIntervalSecs,
                  priorityAvailableAccountCap:
                    body.maintenance.priorityAvailableAccountCap ??
                    store.routing.maintenance?.priorityAvailableAccountCap ??
                    defaultRoutingMaintenance.priorityAvailableAccountCap,
                },
              }
            : {}),
          ...(body.timeouts
            ? {
                timeouts: {
                  responsesFirstByteTimeoutSecs:
                    body.timeouts.responsesFirstByteTimeoutSecs ??
                    store.routing.timeouts?.responsesFirstByteTimeoutSecs ??
                    defaultRoutingTimeouts.responsesFirstByteTimeoutSecs,
                  compactFirstByteTimeoutSecs:
                    body.timeouts.compactFirstByteTimeoutSecs ??
                    store.routing.timeouts?.compactFirstByteTimeoutSecs ??
                    defaultRoutingTimeouts.compactFirstByteTimeoutSecs,
                  responsesStreamTimeoutSecs:
                    body.timeouts.responsesStreamTimeoutSecs ??
                    store.routing.timeouts?.responsesStreamTimeoutSecs ??
                    defaultRoutingTimeouts.responsesStreamTimeoutSecs,
                  compactStreamTimeoutSecs:
                    body.timeouts.compactStreamTimeoutSecs ??
                    store.routing.timeouts?.compactStreamTimeoutSecs ??
                    defaultRoutingTimeouts.compactStreamTimeoutSecs,
                },
              }
            : {}),
        }
        return jsonResponse(clone(store.routing))
      }

      if (
        path === '/api/pool/upstream-accounts/oauth/login-sessions' &&
        method === 'POST'
      ) {
        const body = parseBody<{
          displayName?: string
          groupName?: string
          note?: string
          groupNote?: string
          tagIds?: number[]
          isMother?: boolean
          mailboxSessionId?: string
          mailboxAddress?: string
        }>(init?.body, {})
        const loginId = `login_${Date.now()}`
        const redirectUri = `http://localhost:431${String(store.nextId).slice(-1)}/oauth/callback`
        const state = `state_${loginId}`
        const session: StoryStore['sessions'][string] = {
          loginId,
          status: 'pending',
          authUrl: `https://auth.openai.com/authorize?mock=1&loginId=${loginId}&state=${state}`,
          redirectUri,
          expiresAt: storyFutureLoginExpiresAt,
          accountId: null,
          error: null,
          displayName: body.displayName,
          groupName: body.groupName,
          isMother: body.isMother,
          note: body.note,
          groupNote: body.groupNote,
          tagIds: Array.isArray(body.tagIds) ? body.tagIds : [],
          mailboxSessionId: body.mailboxSessionId,
          mailboxAddress: body.mailboxAddress,
          state,
        }
        store.sessions[loginId] = session
        return jsonResponse(clone(session), 201)
      }

      if (
        path === '/api/pool/upstream-accounts/oauth/mailbox-sessions' &&
        method === 'POST'
      ) {
        const body = parseBody<{ emailAddress?: string }>(init?.body, {})
        const requestedAddress = body.emailAddress?.trim().toLowerCase() ?? ''
        const shouldDelayMailboxAttach =
          storyId ===
            'account-pool-pages-upstream-account-create-oauth--mailbox-attach-flow' ||
          storyId ===
            'account-pool-pages-upstream-account-create-oauth--mailbox-attach-pending' ||
          storyId ===
            'account-pool-pages-upstream-account-create-batch-oauth--mailbox-attach-flow' ||
          storyId ===
            'account-pool-pages-upstream-account-create-batch-oauth--mailbox-popover-edit' ||
          storyId ===
            'account-pool-pages-upstream-account-create-batch-oauth--mailbox-attach-pending'
        const shouldDelayMailboxGenerate =
          storyId ===
            'account-pool-pages-upstream-account-create-oauth--mailbox-generate-flow' ||
          storyId ===
            'account-pool-pages-upstream-account-create-oauth--mailbox-generate-pending' ||
          storyId ===
            'account-pool-pages-upstream-account-create-batch-oauth--mailbox-generate-flow' ||
          storyId ===
            'account-pool-pages-upstream-account-create-batch-oauth--mailbox-generate-pending'
        if (requestedAddress && shouldDelayMailboxAttach) {
          await wait(900)
        }
        if (!requestedAddress && shouldDelayMailboxGenerate) {
          await wait(900)
        }
        if (requestedAddress) {
          if (!requestedAddress.includes('@')) {
            return jsonResponse(
              {
                supported: false,
                emailAddress: requestedAddress,
                reason: 'invalid_format',
              },
              201,
            )
          }
          const isSupportedDomain = requestedAddress.endsWith(
            '@mail-tw.707079.xyz',
          )
          if (!isSupportedDomain) {
            return jsonResponse(
              {
                supported: false,
                emailAddress: requestedAddress,
                reason: 'unsupported_domain',
              },
              201,
            )
          }
          const existingRemoteMailbox = storyHasExistingMailboxAddress(
            store,
            requestedAddress,
          )
          const nextMailboxId = store.nextMailboxId++
          const sessionId = `mailbox_${nextMailboxId}`
          const expiresAt = storyFutureExpiresAt
          store.mailboxStatuses[sessionId] = {
            sessionId,
            emailAddress: requestedAddress,
            expiresAt,
            latestCode: null,
            invite: null,
            invited: false,
          }
          return jsonResponse(
            {
              supported: true,
              sessionId,
              emailAddress: requestedAddress,
              expiresAt,
              source: existingRemoteMailbox ? 'attached' : 'generated',
            },
            201,
          )
        }
        const nextMailboxId = store.nextMailboxId++
        const sessionId = `mailbox_${nextMailboxId}`
        const emailAddress = `storybook-oauth-${nextMailboxId}@mail-tw.707079.xyz`
        const expiresAt = storyFutureExpiresAt
        store.mailboxStatuses[sessionId] = {
          sessionId,
          emailAddress,
          expiresAt,
          latestCode: null,
          invite: null,
          invited: false,
        }
        return jsonResponse(
          {
            supported: true,
            sessionId,
            emailAddress,
            expiresAt,
            source: 'generated',
          },
          201,
        )
      }

      if (
        path === '/api/pool/upstream-accounts/oauth/mailbox-sessions/status' &&
        method === 'POST'
      ) {
        const body = parseBody<{ sessionIds?: string[] }>(init?.body, {})
        const sessionIds = Array.isArray(body.sessionIds) ? body.sessionIds : []
        const items = sessionIds
          .map((sessionId) => store.mailboxStatuses[sessionId])
          .filter((item): item is OauthMailboxStatus => item != null)
        return jsonResponse({ items })
      }

      const mailboxSessionMatch = path.match(
        /^\/api\/pool\/upstream-accounts\/oauth\/mailbox-sessions\/([^/]+)$/,
      )
      if (mailboxSessionMatch && method === 'DELETE') {
        const sessionId = decodeURIComponent(mailboxSessionMatch[1])
        delete store.mailboxStatuses[sessionId]
        return noContent()
      }

      const loginSessionMatch = path.match(
        /^\/api\/pool\/upstream-accounts\/oauth\/login-sessions\/([^/]+)$/,
      )
      if (loginSessionMatch && method === 'PATCH') {
        const loginId = decodeURIComponent(loginSessionMatch[1])
        const session = store.sessions[loginId]
        if (!session)
          return jsonResponse({ message: 'missing mock session' }, 404)
        const body = parseBody<UpdateOauthLoginSessionPayload>(init?.body, {})
        session.displayName = body.displayName?.trim() || undefined
        session.groupName = body.groupName?.trim() || undefined
        session.note = body.note?.trim() || undefined
        session.groupNote = body.groupNote?.trim() || undefined
        session.tagIds = Array.isArray(body.tagIds) ? body.tagIds : []
        session.isMother = body.isMother === true
        session.mailboxSessionId = body.mailboxSessionId?.trim() || undefined
        session.mailboxAddress = body.mailboxAddress?.trim() || undefined
        return jsonResponse(clone(session))
      }
      if (loginSessionMatch && method === 'GET') {
        const loginId = decodeURIComponent(loginSessionMatch[1])
        const session = store.sessions[loginId]
        if (!session)
          return jsonResponse({ message: 'missing mock session' }, 404)
        return jsonResponse(clone(session))
      }

      const completeLoginSessionMatch = path.match(
        /^\/api\/pool\/upstream-accounts\/oauth\/login-sessions\/([^/]+)\/complete$/,
      )
      if (completeLoginSessionMatch && method === 'POST') {
        const loginId = decodeURIComponent(completeLoginSessionMatch[1])
        const session = store.sessions[loginId]
        if (!session)
          return jsonResponse({ message: 'missing mock session' }, 404)
        const body = parseBody<CompleteOauthLoginSessionPayload>(init?.body, {
          callbackUrl: '',
        })
        const callbackUrl = body.callbackUrl.trim()
        if (
          !callbackUrl ||
          !session.state ||
          !callbackUrl.includes(session.state)
        ) {
          session.status = 'failed'
          session.error =
            'Mock callback URL does not contain the expected state token.'
          return jsonResponse({ message: session.error }, 400)
        }
        const nextId = session.accountId ?? store.nextId++
        const existing = store.details[nextId]
        const detail = createOauthAccount(nextId, {
          displayName:
            session.displayName ||
            existing?.displayName ||
            'Codex Pro - New login',
          groupName: session.groupName ?? existing?.groupName ?? 'default',
          isMother: session.isMother ?? existing?.isMother ?? false,
          note:
            session.note ??
            existing?.note ??
            'Freshly connected from Storybook OAuth mock.',
        })
        const normalizedGroupName = normalizeGroupName(detail.groupName)
        if (normalizedGroupName && session.groupNote?.trim()) {
          store.groupNotes[normalizedGroupName] = session.groupNote.trim()
        }
        store.details[nextId] = detail
        const summary = toSummary(detail)
        store.accounts = [
          summary,
          ...store.accounts.filter((item) => item.id !== nextId),
        ]
        session.accountId = nextId
        session.status = 'completed'
        session.authUrl = null
        session.redirectUri = null
        session.error = null
        return jsonResponse(clone(detail))
      }

      if (
        path === '/api/pool/upstream-accounts/api-keys' &&
        method === 'POST'
      ) {
        const body = parseBody<CreateApiKeyAccountPayload>(init?.body, {
          displayName: 'New API key',
          apiKey: 'sk-storybook-key',
        })
        const nextId = store.nextId++
        const detail = createApiKeyAccount(nextId, {
          displayName: body.displayName,
          groupName: body.groupName ?? 'default',
          isMother: body.isMother === true,
          note: body.note ?? null,
          upstreamBaseUrl: body.upstreamBaseUrl ?? null,
          maskedApiKey: maskApiKey(body.apiKey),
          localLimits: {
            primaryLimit: body.localPrimaryLimit ?? 120,
            secondaryLimit: body.localSecondaryLimit ?? 500,
            limitUnit: body.localLimitUnit ?? 'requests',
          },
        })
        const synced = syncLocalWindows(detail)
        const normalizedGroupName = normalizeGroupName(synced.groupName)
        if (normalizedGroupName && body.groupNote?.trim()) {
          store.groupNotes[normalizedGroupName] = body.groupNote.trim()
        }
        store.details[nextId] = synced
        store.accounts = [toSummary(synced), ...store.accounts]
        return jsonResponse(clone(synced), 201)
      }

      const reloginMatch = path.match(
        /^\/api\/pool\/upstream-accounts\/(\d+)\/oauth\/relogin$/,
      )
      if (reloginMatch && method === 'POST') {
        const accountId = Number(reloginMatch[1])
        const state = `state_relogin_${accountId}`
        const session: StoryStore['sessions'][string] = {
          loginId: `relogin_${accountId}_${Date.now()}`,
          status: 'pending',
          authUrl: `https://auth.openai.com/authorize?mock=1&accountId=${accountId}&state=${state}`,
          redirectUri: `http://localhost:432${String(accountId).slice(-1)}/oauth/callback`,
          expiresAt: storyFutureLoginExpiresAt,
          accountId,
          error: null,
          state,
        }
        store.sessions[session.loginId] = session
        return jsonResponse(clone(session), 201)
      }

      const syncMatch = path.match(
        /^\/api\/pool\/upstream-accounts\/(\d+)\/sync$/,
      )
      if (syncMatch && method === 'POST') {
        const accountId = Number(syncMatch[1])
        const detail = store.details[accountId]
        if (!detail)
          return jsonResponse({ message: 'missing mock account' }, 404)
        const updated = syncLocalWindows({
          ...detail,
          status: 'active',
          lastSyncedAt: now,
          lastSuccessfulSyncAt: now,
          lastError: null,
          lastErrorAt: null,
        })
        store.details[accountId] = updated
        store.accounts = store.accounts.map((item) =>
          item.id === accountId ? toSummary(updated) : item,
        )
        return jsonResponse(clone(updated))
      }

      const detailMatch = path.match(/^\/api\/pool\/upstream-accounts\/(\d+)$/)
      if (detailMatch && method === 'GET') {
        const accountId = Number(detailMatch[1])
        const detail = store.details[accountId]
        if (!detail)
          return jsonResponse({ message: 'missing mock account' }, 404)
        return jsonResponse(clone(detail))
      }

      const stickyMatch = path.match(
        /^\/api\/pool\/upstream-accounts\/(\d+)\/sticky-keys$/,
      )
      if (stickyMatch && method === 'GET') {
        const accountId = Number(stickyMatch[1])
        return jsonResponse(buildStickyConversations(accountId))
      }

      if (detailMatch && method === 'PATCH') {
        if (storyId?.endsWith('--completed-save-failure-feedback')) {
          return Promise.resolve(
            new Response('Storybook forced save failure.', {
              status: 500,
              headers: { 'Content-Type': 'text/plain' },
            }),
          )
        }
        const accountId = Number(detailMatch[1])
        const detail = store.details[accountId]
        if (!detail)
          return jsonResponse({ message: 'missing mock account' }, 404)
        const body = parseBody<UpdateUpstreamAccountPayload>(init?.body, {})
        const updated = syncLocalWindows({
          ...detail,
          displayName: body.displayName ?? detail.displayName,
          groupName: body.groupName ?? detail.groupName,
          isMother: body.isMother ?? detail.isMother,
          note: body.note ?? detail.note,
          upstreamBaseUrl:
            detail.kind === 'api_key_codex' &&
            Object.prototype.hasOwnProperty.call(body, 'upstreamBaseUrl')
              ? (body.upstreamBaseUrl ?? null)
              : detail.upstreamBaseUrl,
          enabled: body.enabled ?? detail.enabled,
          status:
            body.enabled === false
              ? 'disabled'
              : detail.status === 'disabled'
                ? 'active'
                : detail.status,
          maskedApiKey: body.apiKey
            ? maskApiKey(body.apiKey)
            : detail.maskedApiKey,
          localLimits:
            detail.kind === 'api_key_codex'
              ? {
                  primaryLimit:
                    body.localPrimaryLimit ??
                    detail.localLimits?.primaryLimit ??
                    120,
                  secondaryLimit:
                    body.localSecondaryLimit ??
                    detail.localLimits?.secondaryLimit ??
                    500,
                  limitUnit:
                    body.localLimitUnit ??
                    detail.localLimits?.limitUnit ??
                    'requests',
                }
              : detail.localLimits,
        })
        store.details[accountId] = updated
        store.accounts = store.accounts.map((item) =>
          item.id === accountId ? toSummary(updated) : item,
        )
        return jsonResponse(clone(updated))
      }

      const groupMatch = path.match(
        /^\/api\/pool\/upstream-accounts\/groups\/(.+)$/,
      )
      if (groupMatch && method === 'PATCH') {
        const groupName = decodeURIComponent(groupMatch[1])
        const body = parseBody<UpdateUpstreamAccountGroupPayload>(
          init?.body,
          {},
        )
        const normalized = normalizeGroupName(groupName)
        if (!normalized)
          return jsonResponse({ message: 'missing mock group' }, 404)
        const note = body.note?.trim() ?? ''
        if (note) store.groupNotes[normalized] = note
        else delete store.groupNotes[normalized]
        return jsonResponse({ groupName: normalized, note: note || null })
      }

      if (detailMatch && method === 'DELETE') {
        const accountId = Number(detailMatch[1])
        if (storyId?.endsWith('--delete-failure')) {
          return Promise.resolve(
            new Response(
              'error returned from database: (code: 5) database is locked',
              {
                status: 500,
                headers: { 'Content-Type': 'text/plain' },
              },
            ),
          )
        }
        delete store.details[accountId]
        store.accounts = store.accounts.filter((item) => item.id !== accountId)
        return noContent()
      }

      return (originalFetchRef.current as typeof window.fetch)(input, init)
    }

    window.fetch = mockedFetch
  }

  useEffect(() => {
    return () => {
      if (typeof window !== 'undefined' && originalFetchRef.current) {
        window.fetch = originalFetchRef.current
        originalFetchRef.current = null
      }
    }
  }, [])

  return <>{children}</>
}

export function AccountPoolStoryRouter({
  initialEntry,
}: {
  initialEntry: StoryInitialEntry
}) {
  const { themeMode } = useTheme()
  const isDark = themeMode === 'dark'
  return (
    <div
      className="min-h-screen bg-base-200 px-6 py-6 text-base-content"
      style={{
        backgroundImage: isDark
          ? 'radial-gradient(circle at 10% -10%, rgba(56,189,248,0.18), transparent 36%), radial-gradient(circle at 88% 0%, rgba(45,212,191,0.16), transparent 34%), linear-gradient(180deg, #081428 0%, #10213a 62%)'
          : 'radial-gradient(circle at 10% -10%, rgba(14,165,233,0.10), transparent 34%), radial-gradient(circle at 88% 0%, rgba(16,185,129,0.10), transparent 30%), linear-gradient(180deg, #f7fbff 0%, #e8f1fb 58%, #e1ecf8 100%)',
      }}
    >
      <MemoryRouter initialEntries={[initialEntry]}>
        <Routes>
          <Route path="/account-pool" element={<AccountPoolLayout />}>
            <Route
              path="upstream-accounts"
              element={<UpstreamAccountsPage />}
            />
            <Route
              path="upstream-accounts/new"
              element={<UpstreamAccountCreatePage />}
            />
          </Route>
        </Routes>
      </MemoryRouter>
    </div>
  )
}
