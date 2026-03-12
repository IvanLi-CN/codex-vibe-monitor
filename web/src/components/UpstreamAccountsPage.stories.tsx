import { useEffect, useRef, type ReactNode } from 'react'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { MemoryRouter, Route, Routes } from 'react-router-dom'
import { I18nProvider } from '../i18n'
import type {
  CreateApiKeyAccountPayload,
  LoginSessionStatusResponse,
  UpdateUpstreamAccountPayload,
  UpstreamAccountDetail,
  UpstreamAccountListResponse,
  UpstreamAccountSummary,
} from '../lib/api'
import AccountPoolLayout from '../pages/account-pool/AccountPoolLayout'
import UpstreamAccountsPage from '../pages/account-pool/UpstreamAccounts'

type StoryStore = {
  writesEnabled: boolean
  accounts: UpstreamAccountSummary[]
  details: Record<number, UpstreamAccountDetail>
  nextId: number
  sessions: Record<
    string,
    LoginSessionStatusResponse & {
      displayName?: string
      note?: string
      polls?: number
    }
  >
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

function createOauthAccount(id: number, overrides?: Partial<UpstreamAccountDetail>): UpstreamAccountDetail {
  const detail: UpstreamAccountDetail = {
    id,
    kind: 'oauth_codex',
    provider: 'codex',
    displayName: 'Codex Pro - Tokyo',
    status: 'active',
    enabled: true,
    email: 'tokyo@example.com',
    chatgptAccountId: 'org_tokyo',
    chatgptUserId: 'user_tokyo',
    planType: 'pro',
    lastSyncedAt: now,
    lastSuccessfulSyncAt: now,
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
    status: 'active',
    enabled: true,
    email: null,
    chatgptAccountId: null,
    chatgptUserId: null,
    planType: 'local',
    maskedApiKey: 'sk-live••••••c9f2',
    lastSyncedAt: now,
    lastSuccessfulSyncAt: now,
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
    status: detail.status,
    enabled: detail.enabled,
    email: detail.email,
    chatgptAccountId: detail.chatgptAccountId,
    planType: detail.planType,
    maskedApiKey: detail.maskedApiKey,
    lastSyncedAt: detail.lastSyncedAt,
    lastSuccessfulSyncAt: detail.lastSuccessfulSyncAt,
    lastError: detail.lastError,
    lastErrorAt: detail.lastErrorAt,
    tokenExpiresAt: detail.tokenExpiresAt,
    primaryWindow: detail.primaryWindow,
    secondaryWindow: detail.secondaryWindow,
    credits: detail.credits,
    localLimits: detail.localLimits,
  }
}

function createStore(): StoryStore {
  const oauth = createOauthAccount(101)
  const apiKey = createApiKeyAccount(102)
  return {
    writesEnabled: true,
    accounts: [toSummary(oauth), toSummary(apiKey)],
    details: {
      [oauth.id]: oauth,
      [apiKey.id]: apiKey,
    },
    nextId: 103,
    sessions: {},
  }
}

function maskApiKey(value: string) {
  const trimmed = value.trim()
  if (!trimmed) return 'sk-empty••••'
  const suffix = trimmed.slice(-4)
  return `sk-live••••••${suffix}`
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

function StorybookUpstreamAccountsMock({ children }: { children: ReactNode }) {
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
          items: store.accounts.map((item) => clone(item)),
        }
        return jsonResponse(payload)
      }

      if (path === '/api/pool/upstream-accounts/oauth/login-sessions' && method === 'POST') {
        const body = parseBody<{ displayName?: string; note?: string }>(init?.body, {})
        const loginId = `login_${Date.now()}`
        const session: StoryStore['sessions'][string] = {
          loginId,
          status: 'pending',
          authUrl: `https://auth.openai.com/authorize?mock=1&loginId=${loginId}`,
          expiresAt: '2026-03-11T12:40:00.000Z',
          accountId: null,
          error: null,
          displayName: body.displayName,
          note: body.note,
          polls: 0,
        }
        store.sessions[loginId] = session
        return jsonResponse(clone(session), 201)
      }

      const loginSessionMatch = path.match(/^\/api\/pool\/upstream-accounts\/oauth\/login-sessions\/([^/]+)$/)
      if (loginSessionMatch && method === 'GET') {
        const loginId = decodeURIComponent(loginSessionMatch[1])
        const session = store.sessions[loginId]
        if (!session) return jsonResponse({ message: 'missing mock session' }, 404)
        session.polls = (session.polls ?? 0) + 1
        if (session.status === 'pending' && session.polls >= 2) {
          if (session.accountId == null) {
            const nextId = store.nextId++
            const detail = createOauthAccount(nextId, {
              displayName: session.displayName || 'Codex Pro - New login',
              note: session.note ?? 'Freshly connected from Storybook OAuth mock.',
            })
            store.details[nextId] = detail
            store.accounts = [toSummary(detail), ...store.accounts]
            session.accountId = nextId
          }
          session.status = 'completed'
        }
        return jsonResponse(clone(session))
      }

      if (path === '/api/pool/upstream-accounts/api-keys' && method === 'POST') {
        const body = parseBody<CreateApiKeyAccountPayload>(init?.body, {
          displayName: 'New API key',
          apiKey: 'sk-storybook-key',
        })
        const nextId = store.nextId++
        const detail = createApiKeyAccount(nextId, {
          displayName: body.displayName,
          note: body.note ?? null,
          maskedApiKey: maskApiKey(body.apiKey),
          localLimits: {
            primaryLimit: body.localPrimaryLimit ?? 120,
            secondaryLimit: body.localSecondaryLimit ?? 500,
            limitUnit: body.localLimitUnit ?? 'requests',
          },
        })
        const synced = syncLocalWindows(detail)
        store.details[nextId] = synced
        store.accounts = [toSummary(synced), ...store.accounts]
        return jsonResponse(clone(synced), 201)
      }

      const reloginMatch = path.match(/^\/api\/pool\/upstream-accounts\/(\d+)\/oauth\/relogin$/)
      if (reloginMatch && method === 'POST') {
        const accountId = Number(reloginMatch[1])
        const session: StoryStore['sessions'][string] = {
          loginId: `relogin_${accountId}_${Date.now()}`,
          status: 'pending',
          authUrl: `https://auth.openai.com/authorize?mock=1&accountId=${accountId}`,
          expiresAt: '2026-03-11T12:40:00.000Z',
          accountId,
          error: null,
          polls: 0,
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

      if (detailMatch && method === 'PATCH') {
        const accountId = Number(detailMatch[1])
        const detail = store.details[accountId]
        if (!detail) return jsonResponse({ message: 'missing mock account' }, 404)
        const body = parseBody<UpdateUpstreamAccountPayload>(init?.body, {})
        const updated = syncLocalWindows({
          ...detail,
          displayName: body.displayName ?? detail.displayName,
          note: body.note ?? detail.note,
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

const meta = {
  title: 'Pages/Account Pool/Upstream Accounts',
  component: UpstreamAccountsPage,
  tags: ['autodocs'],
  parameters: {
    layout: 'fullscreen',
  },
  decorators: [
    (Story) => (
      <I18nProvider>
        <StorybookUpstreamAccountsMock>
          <div data-theme="light" className="min-h-screen bg-base-200 px-6 py-6 text-base-content">
            <MemoryRouter initialEntries={['/account-pool/upstream-accounts']}>
              <Routes>
                <Route path="/account-pool" element={<AccountPoolLayout />}>
                  <Route path="upstream-accounts" element={<Story />} />
                </Route>
              </Routes>
            </MemoryRouter>
          </div>
        </StorybookUpstreamAccountsMock>
      </I18nProvider>
    ),
  ],
} satisfies Meta<typeof UpstreamAccountsPage>

export default meta

type Story = StoryObj<typeof meta>

export const Operational: Story = {
  render: () => <UpstreamAccountsPage />,
}
