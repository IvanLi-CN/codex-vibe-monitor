import { useEffect, useRef, type ReactNode } from 'react'
import { MemoryRouter, Route, Routes } from 'react-router-dom'
import { useTheme } from '../theme/context'
import type {
  AccountTagSummary,
  CreateApiKeyAccountPayload,
  CompleteOauthLoginSessionPayload,
  EffectiveRoutingRule,
  LoginSessionStatusResponse,
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
  routing: {
    apiKeyConfigured: boolean
    maskedApiKey?: string | null
  }
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
      state?: string
    }
  >
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

const defaultTags: AccountTagSummary[] = [
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

export type StoryInitialEntry =
  | string
  | {
      pathname: string
      search?: string
      state?: unknown
    }

const now = '2026-03-11T12:30:00.000Z'

function buildWindow(percent: number, durationMins: number, usedText: string, limitText: string, resetsAt: string) {
  return {
    usedPercent: percent,
    usedText,
    limitText,
    resetsAt,
    windowDurationMins: durationMins,
  }
}

function buildHistory(seed = 0) {
  return Array.from({ length: 7 }, (_, index) => ({
    capturedAt: new Date(Date.parse('2026-03-05T00:00:00.000Z') + index * 12 * 3600_000).toISOString(),
    primaryUsedPercent: Math.min(96, 18 + seed + index * 7),
    secondaryUsedPercent: Math.min(88, 8 + seed / 2 + index * 3),
    creditsBalance: (18.5 - index * 0.9).toFixed(2),
  }))
}

function clone<T>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T
}

function normalizeGroupName(value?: string | null) {
  const trimmed = value?.trim() ?? ''
  return trimmed || null
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

function createOauthAccount(id: number, overrides?: Partial<UpstreamAccountDetail>): UpstreamAccountDetail {
  const detail: UpstreamAccountDetail = {
    id,
    kind: 'oauth_codex',
    provider: 'codex',
    displayName: 'Codex Pro - Tokyo',
    groupName: 'production',
    isMother: true,
    status: 'active',
    enabled: true,
    email: 'tokyo@example.com',
    chatgptAccountId: 'org_tokyo',
    chatgptUserId: 'user_tokyo',
    planType: 'pro',
    lastSyncedAt: now,
    lastSuccessfulSyncAt: now,
    lastActivityAt: '2026-03-11T12:12:00.000Z',
    lastRefreshedAt: now,
    tokenExpiresAt: '2026-03-12T12:30:00.000Z',
    lastError: null,
    lastErrorAt: null,
    primaryWindow: buildWindow(64, 300, '64% used', '5h rolling window', '2026-03-11T14:00:00.000Z'),
    secondaryWindow: buildWindow(22, 10080, '22% used', '7d rolling window', '2026-03-18T00:00:00.000Z'),
    credits: {
      hasCredits: true,
      unlimited: false,
      balance: '11.80',
    },
    localLimits: {
      primaryLimit: null,
      secondaryLimit: null,
      limitUnit: 'requests',
    },
    tags: defaultTags,
    effectiveRoutingRule: defaultEffectiveRoutingRule,
    note: 'Primary team account for premium traffic.',
    maskedApiKey: null,
    history: buildHistory(2),
  }
  return { ...detail, ...overrides, history: overrides?.history ?? detail.history }
}

function createApiKeyAccount(id: number, overrides?: Partial<UpstreamAccountDetail>): UpstreamAccountDetail {
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
    enabled: true,
    email: null,
    chatgptAccountId: null,
    chatgptUserId: null,
    planType: 'local',
    maskedApiKey: 'sk-live••••••c9f2',
    lastSyncedAt: now,
    lastSuccessfulSyncAt: now,
    lastActivityAt: '2026-03-11T12:24:00.000Z',
    lastRefreshedAt: null,
    tokenExpiresAt: null,
    lastError: null,
    lastErrorAt: null,
    primaryWindow: buildWindow(0, 300, `0 ${limitUnit}`, `${primaryLimit} ${limitUnit}`, '2026-03-11T14:00:00.000Z'),
    secondaryWindow: buildWindow(0, 10080, `0 ${limitUnit}`, `${secondaryLimit} ${limitUnit}`, '2026-03-18T00:00:00.000Z'),
    credits: {
      hasCredits: false,
      unlimited: false,
      balance: null,
    },
    localLimits: {
      primaryLimit,
      secondaryLimit,
      limitUnit,
    },
    tags: defaultTags,
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
  return { ...detail, ...overrides, history: overrides?.history ?? detail.history }
}

function toSummary(detail: UpstreamAccountDetail): UpstreamAccountSummary {
  return {
    id: detail.id,
    kind: detail.kind,
    provider: detail.provider,
    displayName: detail.displayName,
    groupName: detail.groupName,
    isMother: detail.isMother,
    status: detail.status,
    enabled: detail.enabled,
    email: detail.email,
    chatgptAccountId: detail.chatgptAccountId,
    planType: detail.planType,
    maskedApiKey: detail.maskedApiKey,
    lastSyncedAt: detail.lastSyncedAt,
    lastSuccessfulSyncAt: detail.lastSuccessfulSyncAt,
    lastActivityAt: detail.lastActivityAt,
    lastError: detail.lastError,
    lastErrorAt: detail.lastErrorAt,
    tokenExpiresAt: detail.tokenExpiresAt,
    primaryWindow: detail.primaryWindow,
    secondaryWindow: detail.secondaryWindow,
    credits: detail.credits,
    localLimits: detail.localLimits,
    duplicateInfo: detail.duplicateInfo,
    tags: detail.tags,
    effectiveRoutingRule: detail.effectiveRoutingRule,
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
    storyId === 'account-pool-pages-upstream-accounts--duplicate-oauth-warning' ||
    storyId === 'account-pool-pages-upstream-accounts--duplicate-oauth-detail'
  const compactStory = storyId === 'account-pool-pages-upstream-accounts--compact-long-labels'

  const oauth = createOauthAccount(101, duplicateStory
    ? {
        duplicateInfo: {
          peerAccountIds: [103],
          reasons: [...duplicateReasons],
        },
        note: 'Primary team account sharing the same upstream identity.',
      }
    : compactStory
      ? {
          displayName: 'Codex Pro - Tokyo enterprise rotation account with a deliberately long roster title',
          groupName: 'production-apac-primary-operators',
        }
    : undefined)
  const apiKey = createApiKeyAccount(102)
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
          planType: 'team',
          lastSuccessfulSyncAt: '2026-03-11T20:10:00.000Z',
          lastActivityAt: '2026-03-11T20:08:00.000Z',
          primaryWindow: buildWindow(71, 300, '71% used', '5h rolling window', '2026-03-11T22:10:00.000Z'),
          secondaryWindow: buildWindow(100, 10080, '100% used', '7d rolling window', '2026-03-18T08:00:00.000Z'),
          tags: [
            defaultTags[0],
            defaultTags[1],
            { id: 7, name: 'weekly-cap', routingRule: defaultEffectiveRoutingRule },
          ],
          note: 'Weekly window is fully exhausted while the 5h window still has room.',
        }),
        createOauthAccount(105, {
          displayName: 'Codex Pro - Osaka burst limit exhausted',
          groupName: 'production-apac-burst',
          isMother: false,
          status: 'syncing',
          planType: 'team',
          lastSuccessfulSyncAt: '2026-03-11T19:58:00.000Z',
          lastActivityAt: '2026-03-11T19:56:00.000Z',
          primaryWindow: buildWindow(100, 300, '100% used', '5h rolling window', '2026-03-11T21:42:00.000Z'),
          secondaryWindow: buildWindow(46, 10080, '46% used', '7d rolling window', '2026-03-18T08:00:00.000Z'),
          tags: [
            defaultTags[0],
            { id: 8, name: 'burst-limit', routingRule: defaultEffectiveRoutingRule },
            { id: 9, name: 'warm-spare', routingRule: defaultEffectiveRoutingRule },
          ],
          note: 'Burst traffic consumed the full 5h budget.',
        }),
        createApiKeyAccount(106, {
          displayName: 'Backup key - weekly redline',
          groupName: 'staging-overflow',
          status: 'active',
          enabled: true,
          planType: 'local',
          lastSuccessfulSyncAt: '2026-03-11T19:42:00.000Z',
          lastActivityAt: '2026-03-11T20:18:00.000Z',
          primaryWindow: buildWindow(93, 300, '112 requests', '120 requests', '2026-03-11T21:30:00.000Z'),
          secondaryWindow: buildWindow(100, 10080, '500 requests', '500 requests', '2026-03-18T08:00:00.000Z'),
          tags: [
            { id: 10, name: 'overflow', routingRule: defaultEffectiveRoutingRule },
            { id: 11, name: 'weekly-redline', routingRule: defaultEffectiveRoutingRule },
            defaultTags[1],
          ],
          note: 'Fallback key with the weekly allowance fully consumed.',
        }),
        createApiKeyAccount(107, {
          displayName: 'Emergency key - both windows saturated',
          groupName: 'rescue',
          status: 'needs_reauth',
          enabled: true,
          planType: 'local',
          lastSuccessfulSyncAt: '2026-03-11T18:55:00.000Z',
          lastActivityAt: '2026-03-11T19:14:00.000Z',
          primaryWindow: buildWindow(100, 300, '120 requests', '120 requests', '2026-03-11T20:40:00.000Z'),
          secondaryWindow: buildWindow(100, 10080, '500 requests', '500 requests', '2026-03-18T08:00:00.000Z'),
          tags: [
            { id: 12, name: 'rescue', routingRule: defaultEffectiveRoutingRule },
            { id: 13, name: 'manual-drain', routingRule: defaultEffectiveRoutingRule },
          ],
          note: 'Emergency key where both local placeholder windows are exhausted.',
        }),
      ]
    : []
  const accounts = [toSummary(oauth), ...(duplicateOauth ? [toSummary(duplicateOauth)] : []), toSummary(apiKey), ...compactExtraAccounts.map(toSummary)]
  const details = {
    [oauth.id]: oauth,
    ...(duplicateOauth ? { [duplicateOauth.id]: duplicateOauth } : {}),
    [apiKey.id]: apiKey,
    ...Object.fromEntries(compactExtraAccounts.map((account) => [account.id, account])),
  }
  return {
    writesEnabled: true,
    routing: {
      apiKeyConfigured: true,
      maskedApiKey: 'pool-live••••••c0de',
    },
    groupNotes: {
      production: 'Premium traffic group note.',
      staging: 'Staging fallback group note.',
      'production-apac-weekly': 'Weekly cap watch list.',
      'production-apac-burst': 'Burst-heavy rotation group.',
      'staging-overflow': 'Fallback keys that often ride the weekly edge.',
      rescue: 'Emergency pool for overflow and incident recovery.',
    },
    accounts,
    details,
    nextId: compactStory ? 108 : duplicateOauth ? 104 : 103,
    sessions: {},
  }
}

function maskApiKey(value: string) {
  const trimmed = value.trim()
  if (!trimmed) return 'sk-empty••••'
  const suffix = trimmed.slice(-4)
  return `sk-live••••••${suffix}`
}

function buildStickyRequestPoints(
  points: Array<{ occurredAt: string; requestTokens: number; status?: string; isSuccess?: boolean }>,
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
              { occurredAt: '2026-03-12T10:15:00.000Z', requestTokens: 102_440 },
              { occurredAt: '2026-03-12T18:20:00.000Z', requestTokens: 154_380 },
              { occurredAt: '2026-03-13T04:03:02.000Z', requestTokens: 198_350 },
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
              { occurredAt: '2026-03-12T12:10:00.000Z', requestTokens: 140_000 },
              { occurredAt: '2026-03-12T20:45:00.000Z', requestTokens: 212_875 },
              { occurredAt: '2026-03-13T04:06:08.000Z', requestTokens: 276_300 },
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
              { occurredAt: '2026-03-12T09:00:00.000Z', requestTokens: 120_000 },
              { occurredAt: '2026-03-12T21:40:00.000Z', requestTokens: 131_400 },
              { occurredAt: '2026-03-13T04:00:52.000Z', requestTokens: 146_799 },
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
              { occurredAt: '2026-03-12T08:25:00.000Z', requestTokens: 330_000 },
              { occurredAt: '2026-03-12T17:15:00.000Z', requestTokens: 445_120 },
              { occurredAt: '2026-03-13T01:48:00.000Z', requestTokens: 268_624 },
              { occurredAt: '2026-03-13T04:01:05.000Z', requestTokens: 258_500 },
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
              { occurredAt: '2026-03-12T07:52:00.000Z', requestTokens: 281_000 },
              { occurredAt: '2026-03-12T13:04:00.000Z', requestTokens: 309_447 },
              { occurredAt: '2026-03-12T23:15:00.000Z', requestTokens: 334_000 },
              { occurredAt: '2026-03-13T03:54:08.000Z', requestTokens: 365_000, status: 'failed', isSuccess: false },
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
              { occurredAt: '2026-03-12T06:18:00.000Z', requestTokens: 640_000 },
              { occurredAt: '2026-03-12T11:42:00.000Z', requestTokens: 722_516 },
              { occurredAt: '2026-03-12T19:36:00.000Z', requestTokens: 841_900 },
              { occurredAt: '2026-03-13T03:56:06.000Z', requestTokens: 1_037_246 },
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
              { occurredAt: '2026-03-12T05:10:00.000Z', requestTokens: 340_000 },
              { occurredAt: '2026-03-12T15:10:00.000Z', requestTokens: 462_400 },
              { occurredAt: '2026-03-12T22:00:00.000Z', requestTokens: 299_561 },
              { occurredAt: '2026-03-13T03:53:28.000Z', requestTokens: 354_000 },
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
  if (detail.kind !== 'api_key_codex') return detail
  const primaryLimit = detail.localLimits?.primaryLimit ?? 120
  const secondaryLimit = detail.localLimits?.secondaryLimit ?? 500
  const limitUnit = detail.localLimits?.limitUnit ?? 'requests'
  return {
    ...detail,
    primaryWindow: buildWindow(0, 300, `0 ${limitUnit}`, `${primaryLimit} ${limitUnit}`, '2026-03-11T14:00:00.000Z'),
    secondaryWindow: buildWindow(0, 10080, `0 ${limitUnit}`, `${secondaryLimit} ${limitUnit}`, '2026-03-18T00:00:00.000Z'),
  }
}

export function StorybookUpstreamAccountsMock({ children }: { children: ReactNode }) {
  const storeRef = useRef<StoryStore>(createStore())
  const originalFetchRef = useRef<typeof window.fetch | null>(null)
  const installedRef = useRef(false)

  if (typeof window !== 'undefined' && !installedRef.current) {
    installedRef.current = true
    originalFetchRef.current = window.fetch.bind(window)

    const mockedFetch: typeof window.fetch = async (input, init) => {
      const method = (init?.method || (input instanceof Request ? input.method : 'GET')).toUpperCase()
      const inputUrl = typeof input === 'string' ? input : input instanceof URL ? input.toString() : input.url
      const parsedUrl = new URL(inputUrl, window.location.origin)
      const path = parsedUrl.pathname
      const store = storeRef.current

      if (path === '/api/pool/upstream-accounts' && method === 'GET') {
        const payload: UpstreamAccountListResponse = {
          writesEnabled: store.writesEnabled,
          groups: listGroupSummaries(store),
          routing: clone(store.routing),
          items: store.accounts.map((item) => clone(item)),
        }
        return jsonResponse(payload)
      }

      if (path === '/api/pool/routing-settings' && method === 'PUT') {
        const body = parseBody<{ apiKey?: string }>(init?.body, {})
        const trimmed = body.apiKey?.trim() ?? ''
        store.routing = {
          apiKeyConfigured: trimmed.length > 0,
          maskedApiKey: trimmed ? maskApiKey(trimmed) : null,
        }
        return jsonResponse(clone(store.routing))
      }

      if (path === '/api/pool/upstream-accounts/oauth/login-sessions' && method === 'POST') {
        const body = parseBody<{ displayName?: string; groupName?: string; note?: string; groupNote?: string; isMother?: boolean }>(init?.body, {})
        const loginId = `login_${Date.now()}`
        const redirectUri = `http://localhost:431${String(store.nextId).slice(-1)}/oauth/callback`
        const state = `state_${loginId}`
        const session: StoryStore['sessions'][string] = {
          loginId,
          status: 'pending',
          authUrl: `https://auth.openai.com/authorize?mock=1&loginId=${loginId}&state=${state}`,
          redirectUri,
          expiresAt: '2026-03-11T12:40:00.000Z',
          accountId: null,
          error: null,
          displayName: body.displayName,
          groupName: body.groupName,
          isMother: body.isMother,
          note: body.note,
          groupNote: body.groupNote,
          state,
        }
        store.sessions[loginId] = session
        return jsonResponse(clone(session), 201)
      }

      const loginSessionMatch = path.match(/^\/api\/pool\/upstream-accounts\/oauth\/login-sessions\/([^/]+)$/)
      if (loginSessionMatch && method === 'GET') {
        const loginId = decodeURIComponent(loginSessionMatch[1])
        const session = store.sessions[loginId]
        if (!session) return jsonResponse({ message: 'missing mock session' }, 404)
        return jsonResponse(clone(session))
      }

      const completeLoginSessionMatch = path.match(/^\/api\/pool\/upstream-accounts\/oauth\/login-sessions\/([^/]+)\/complete$/)
      if (completeLoginSessionMatch && method === 'POST') {
        const loginId = decodeURIComponent(completeLoginSessionMatch[1])
        const session = store.sessions[loginId]
        if (!session) return jsonResponse({ message: 'missing mock session' }, 404)
        const body = parseBody<CompleteOauthLoginSessionPayload>(init?.body, { callbackUrl: '' })
        const callbackUrl = body.callbackUrl.trim()
        if (!callbackUrl || !session.state || !callbackUrl.includes(session.state)) {
          session.status = 'failed'
          session.error = 'Mock callback URL does not contain the expected state token.'
          return jsonResponse({ message: session.error }, 400)
        }
        const nextId = session.accountId ?? store.nextId++
        const existing = store.details[nextId]
        const detail = createOauthAccount(nextId, {
          displayName: session.displayName || existing?.displayName || 'Codex Pro - New login',
          groupName: session.groupName ?? existing?.groupName ?? 'default',
          isMother: session.isMother ?? existing?.isMother ?? false,
          note: session.note ?? existing?.note ?? 'Freshly connected from Storybook OAuth mock.',
        })
        const normalizedGroupName = normalizeGroupName(detail.groupName)
        if (normalizedGroupName && session.groupNote?.trim()) {
          store.groupNotes[normalizedGroupName] = session.groupNote.trim()
        }
        store.details[nextId] = detail
        const summary = toSummary(detail)
        store.accounts = [summary, ...store.accounts.filter((item) => item.id !== nextId)]
        session.accountId = nextId
        session.status = 'completed'
        session.authUrl = null
        session.redirectUri = null
        session.error = null
        return jsonResponse(clone(detail))
      }

      if (path === '/api/pool/upstream-accounts/api-keys' && method === 'POST') {
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

      const reloginMatch = path.match(/^\/api\/pool\/upstream-accounts\/(\d+)\/oauth\/relogin$/)
      if (reloginMatch && method === 'POST') {
        const accountId = Number(reloginMatch[1])
        const state = `state_relogin_${accountId}`
        const session: StoryStore['sessions'][string] = {
          loginId: `relogin_${accountId}_${Date.now()}`,
          status: 'pending',
          authUrl: `https://auth.openai.com/authorize?mock=1&accountId=${accountId}&state=${state}`,
          redirectUri: `http://localhost:432${String(accountId).slice(-1)}/oauth/callback`,
          expiresAt: '2026-03-11T12:40:00.000Z',
          accountId,
          error: null,
          state,
        }
        store.sessions[session.loginId] = session
        return jsonResponse(clone(session), 201)
      }

      const syncMatch = path.match(/^\/api\/pool\/upstream-accounts\/(\d+)\/sync$/)
      if (syncMatch && method === 'POST') {
        const accountId = Number(syncMatch[1])
        const detail = store.details[accountId]
        if (!detail) return jsonResponse({ message: 'missing mock account' }, 404)
        const updated = syncLocalWindows({
          ...detail,
          status: 'active',
          lastSyncedAt: now,
          lastSuccessfulSyncAt: now,
          lastError: null,
          lastErrorAt: null,
        })
        store.details[accountId] = updated
        store.accounts = store.accounts.map((item) => (item.id === accountId ? toSummary(updated) : item))
        return jsonResponse(clone(updated))
      }

      const detailMatch = path.match(/^\/api\/pool\/upstream-accounts\/(\d+)$/)
      if (detailMatch && method === 'GET') {
        const accountId = Number(detailMatch[1])
        const detail = store.details[accountId]
        if (!detail) return jsonResponse({ message: 'missing mock account' }, 404)
        return jsonResponse(clone(detail))
      }

      const stickyMatch = path.match(/^\/api\/pool\/upstream-accounts\/(\d+)\/sticky-keys$/)
      if (stickyMatch && method === 'GET') {
        const accountId = Number(stickyMatch[1])
        return jsonResponse(buildStickyConversations(accountId))
      }

      if (detailMatch && method === 'PATCH') {
        const accountId = Number(detailMatch[1])
        const detail = store.details[accountId]
        if (!detail) return jsonResponse({ message: 'missing mock account' }, 404)
        const body = parseBody<UpdateUpstreamAccountPayload>(init?.body, {})
        const updated = syncLocalWindows({
          ...detail,
          displayName: body.displayName ?? detail.displayName,
          groupName: body.groupName ?? detail.groupName,
          isMother: body.isMother ?? detail.isMother,
          note: body.note ?? detail.note,
          upstreamBaseUrl:
            detail.kind === 'api_key_codex' && Object.prototype.hasOwnProperty.call(body, 'upstreamBaseUrl')
              ? body.upstreamBaseUrl ?? null
              : detail.upstreamBaseUrl,
          enabled: body.enabled ?? detail.enabled,
          status: body.enabled === false ? 'disabled' : detail.status === 'disabled' ? 'active' : detail.status,
          maskedApiKey: body.apiKey ? maskApiKey(body.apiKey) : detail.maskedApiKey,
          localLimits:
            detail.kind === 'api_key_codex'
              ? {
                  primaryLimit: body.localPrimaryLimit ?? detail.localLimits?.primaryLimit ?? 120,
                  secondaryLimit: body.localSecondaryLimit ?? detail.localLimits?.secondaryLimit ?? 500,
                  limitUnit: body.localLimitUnit ?? detail.localLimits?.limitUnit ?? 'requests',
                }
              : detail.localLimits,
        })
        store.details[accountId] = updated
        store.accounts = store.accounts.map((item) => (item.id === accountId ? toSummary(updated) : item))
        return jsonResponse(clone(updated))
      }

      const groupMatch = path.match(/^\/api\/pool\/upstream-accounts\/groups\/(.+)$/)
      if (groupMatch && method === 'PATCH') {
        const groupName = decodeURIComponent(groupMatch[1])
        const body = parseBody<UpdateUpstreamAccountGroupPayload>(init?.body, {})
        const normalized = normalizeGroupName(groupName)
        if (!normalized) return jsonResponse({ message: 'missing mock group' }, 404)
        const note = body.note?.trim() ?? ''
        if (note) store.groupNotes[normalized] = note
        else delete store.groupNotes[normalized]
        return jsonResponse({ groupName: normalized, note: note || null })
      }

      if (detailMatch && method === 'DELETE') {
        const accountId = Number(detailMatch[1])
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

export function AccountPoolStoryRouter({ initialEntry }: { initialEntry: StoryInitialEntry }) {
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
            <Route path="upstream-accounts" element={<UpstreamAccountsPage />} />
            <Route path="upstream-accounts/new" element={<UpstreamAccountCreatePage />} />
          </Route>
        </Routes>
      </MemoryRouter>
    </div>
  )
}
