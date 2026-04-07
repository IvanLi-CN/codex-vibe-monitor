import { useEffect, useRef, type ReactNode } from 'react'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, within } from 'storybook/test'
import { I18nProvider } from '../i18n'
import SettingsPage from '../pages/Settings'
import type {
  ForwardProxyNode,
  ForwardProxyNodeStats,
  ForwardProxySettings,
  PricingEntry,
  PricingSettings,
  SettingsPayload,
} from '../lib/api'

const STORYBOOK_SETTINGS_STORAGE_PREFIX = 'storybook.settings-page.mock'

const DEFAULT_PRICING_ENTRIES: PricingEntry[] = [
  {
    model: 'gpt-5.3-codex',
    inputPer1m: 8.5,
    outputPer1m: 23.5,
    cacheInputPer1m: 0.85,
    reasoningPer1m: 4.2,
    source: 'official',
  },
  {
    model: 'gpt-5.2-codex',
    inputPer1m: 6.2,
    outputPer1m: 18.6,
    cacheInputPer1m: 0.62,
    reasoningPer1m: 3.8,
    source: 'custom',
  },
  {
    model: 'gpt-5.1-codex-mini',
    inputPer1m: 1.9,
    outputPer1m: 6.4,
    cacheInputPer1m: 0.19,
    reasoningPer1m: 1.2,
    source: 'temporary',
  },
]

const DEFAULT_FORWARD_PROXY_SETTINGS: Omit<ForwardProxySettings, 'nodes'> = {
  proxyUrls: [
    'vless://11111111-1111-1111-1111-111111111111@manual.example.com:443?encryption=none&security=tls&type=ws&host=cdn.manual.example.com&path=%2Fmanual#manual-vless',
    'socks5://127.0.0.1:1080',
  ],
  subscriptionUrls: ['https://example.com/subscription.base64'],
  subscriptionUpdateIntervalSecs: 3600,
}

const MOCK_SUBSCRIPTION_NODE_TEMPLATES: Array<Pick<ForwardProxyNode, 'displayName' | 'endpointUrl'>> = [
  {
    displayName: 'edge-vless',
    endpointUrl:
      'vless://11111111-1111-1111-1111-111111111111@edge.example.com:443?encryption=none&security=tls&type=ws&host=cdn.example.com&path=%2Fvless#edge-vless',
  },
  {
    displayName: 'trojan-ws',
    endpointUrl:
      'trojan://topsecret@trojan.example.com:443?security=tls&type=ws&host=cdn.example.com&path=%2Ftrojan#trojan-ws',
  },
  {
    displayName: 'ss-main',
    endpointUrl: 'ss://YWVzLTI1Ni1nY206c3Rvcnlib29rLXBhc3M=@ss.example.com:8388#ss-main',
  },
]

type StorySettingsOverrides = {
  forwardProxy?: Partial<Omit<ForwardProxySettings, 'nodes'>>
  pricing?: Partial<PricingSettings>
}

function statsPreset(index: number): ForwardProxyNodeStats {
  const base = Math.max(1, 24 - index * 3)
  const successRate = Math.max(0.08, 0.97 - index * 0.09)
  return {
    oneMinute: { attempts: base, successRate, avgLatencyMs: 190 + index * 120 },
    fifteenMinutes: { attempts: base * 12, successRate: Math.max(0.1, successRate - 0.02), avgLatencyMs: 230 + index * 110 },
    oneHour: { attempts: base * 42, successRate: Math.max(0.12, successRate - 0.04), avgLatencyMs: 260 + index * 100 },
    oneDay: { attempts: base * 860, successRate: Math.max(0.15, successRate - 0.07), avgLatencyMs: 300 + index * 80 },
    sevenDays: { attempts: base * 5920, successRate: Math.max(0.18, successRate - 0.09), avgLatencyMs: 330 + index * 70 },
  }
}

function labelFromProxyUrl(rawUrl: string): string {
  try {
    const parsed = new URL(rawUrl)
    const defaultPort = parsed.protocol === 'https:' ? '443' : parsed.protocol === 'http:' ? '80' : ''
    const port = parsed.port || defaultPort
    return port ? `${parsed.hostname}:${port}` : parsed.hostname
  } catch {
    return rawUrl
  }
}

function buildNodesFromSettings(settings: ForwardProxySettings): ForwardProxyNode[] {
  const manualNodes: ForwardProxyNode[] = settings.proxyUrls.map((proxyUrl, index) => ({
    key: proxyUrl,
    source: 'manual',
    displayName: labelFromProxyUrl(proxyUrl),
    endpointUrl: proxyUrl,
    weight: Number((1.1 - index * 0.22).toFixed(2)),
    penalized: index >= 2,
    stats: statsPreset(index),
  }))

  const subscriptionNodes: ForwardProxyNode[] = settings.subscriptionUrls.map((_subscriptionUrl, index) => {
    const template = MOCK_SUBSCRIPTION_NODE_TEMPLATES[index % MOCK_SUBSCRIPTION_NODE_TEMPLATES.length]
    const key = `sub-${index + 1}-${template.displayName}`
    return {
      key,
      source: 'subscription',
      displayName: `${template.displayName}-${index + 1}`,
      endpointUrl: template.endpointUrl,
      weight: Number((0.65 - index * 0.12).toFixed(2)),
      penalized: false,
      stats: statsPreset(index + manualNodes.length),
    }
  })

  return [...manualNodes, ...subscriptionNodes]
}

function cloneSettings(payload: SettingsPayload): SettingsPayload {
  return JSON.parse(JSON.stringify(payload)) as SettingsPayload
}

function createStorySettings(overrides?: StorySettingsOverrides): SettingsPayload {
  const forwardProxyBase = {
    ...DEFAULT_FORWARD_PROXY_SETTINGS,
    ...overrides?.forwardProxy,
    proxyUrls: overrides?.forwardProxy?.proxyUrls
      ? [...overrides.forwardProxy.proxyUrls]
      : [...DEFAULT_FORWARD_PROXY_SETTINGS.proxyUrls],
    subscriptionUrls: overrides?.forwardProxy?.subscriptionUrls
      ? [...overrides.forwardProxy.subscriptionUrls]
      : [...DEFAULT_FORWARD_PROXY_SETTINGS.subscriptionUrls],
  }
  const forwardProxy: ForwardProxySettings = {
    ...forwardProxyBase,
    nodes: [],
  }
  forwardProxy.nodes = buildNodesFromSettings(forwardProxy)

  const pricing: PricingSettings = {
    catalogVersion: overrides?.pricing?.catalogVersion ?? 'storybook-2026-03-26',
    entries: overrides?.pricing?.entries ? [...overrides.pricing.entries] : DEFAULT_PRICING_ENTRIES,
  }

  return {
    forwardProxy,
    pricing,
  }
}

function loadPersistedSettings(storageKey: string, fallback: SettingsPayload): SettingsPayload {
  if (typeof window === 'undefined') return cloneSettings(fallback)
  try {
    const raw = window.sessionStorage.getItem(storageKey)
    if (!raw) return cloneSettings(fallback)
    return JSON.parse(raw) as SettingsPayload
  } catch {
    return cloneSettings(fallback)
  }
}

function persistSettings(storageKey: string, payload: SettingsPayload) {
  if (typeof window === 'undefined') return
  try {
    window.sessionStorage.setItem(storageKey, JSON.stringify(payload))
  } catch {
    // ignore session storage write failures inside Storybook mock
  }
}

function StorybookSettingsMock({
  children,
  initialSettings,
  storageKey,
}: {
  children: ReactNode
  initialSettings?: SettingsPayload
  storageKey: string
}) {
  const fallbackSettings = initialSettings ? cloneSettings(initialSettings) : createStorySettings()
  const settingsRef = useRef<SettingsPayload>(loadPersistedSettings(storageKey, fallbackSettings))
  const originalFetchRef = useRef<typeof window.fetch | null>(null)
  const mockInstalledRef = useRef(false)

  if (typeof window !== 'undefined' && !mockInstalledRef.current) {
    mockInstalledRef.current = true
    if (!Array.isArray(settingsRef.current.forwardProxy.nodes) || settingsRef.current.forwardProxy.nodes.length === 0) {
      settingsRef.current.forwardProxy = {
        ...settingsRef.current.forwardProxy,
        nodes: buildNodesFromSettings(settingsRef.current.forwardProxy),
      }
    }
    persistSettings(storageKey, settingsRef.current)

    originalFetchRef.current = window.fetch.bind(window)
    const mockedFetch: typeof window.fetch = async (input, init) => {
      const method = (init?.method || (input instanceof Request ? input.method : 'GET')).toUpperCase()
      const inputUrl = typeof input === 'string' ? input : input instanceof URL ? input.toString() : input.url
      const parsedUrl = new URL(inputUrl, window.location.origin)
      const path = parsedUrl.pathname

      const jsonResponse = (payload: unknown, status = 200) =>
        Promise.resolve(
          new Response(JSON.stringify(payload), {
            status,
            headers: { 'Content-Type': 'application/json' },
          }),
        )

      const parseBody = <T,>(fallback: T): T => {
        const raw = init?.body
        if (typeof raw !== 'string' || !raw) return fallback
        try {
          return JSON.parse(raw) as T
        } catch {
          return fallback
        }
      }

      if (path === '/api/settings' && method === 'GET') {
        return jsonResponse(cloneSettings(settingsRef.current))
      }

      if (path === '/api/settings/forward-proxy' && method === 'PUT') {
        const body = parseBody<{
          proxyUrls: string[]
          subscriptionUrls: string[]
          subscriptionUpdateIntervalSecs: number
        }>({
          proxyUrls: [],
          subscriptionUrls: [],
          subscriptionUpdateIntervalSecs: 3600,
        })

        const nextForwardProxy: ForwardProxySettings = {
          ...settingsRef.current.forwardProxy,
          proxyUrls: (body.proxyUrls || []).map((item) => item.trim()).filter(Boolean),
          subscriptionUrls: (body.subscriptionUrls || []).map((item) => item.trim()).filter(Boolean),
          subscriptionUpdateIntervalSecs: Math.max(60, Math.floor(body.subscriptionUpdateIntervalSecs || 3600)),
          nodes: [],
        }
        nextForwardProxy.nodes = buildNodesFromSettings(nextForwardProxy)
        settingsRef.current.forwardProxy = nextForwardProxy
        persistSettings(storageKey, settingsRef.current)
        return jsonResponse(nextForwardProxy)
      }

      if (path === '/api/settings/forward-proxy/validate' && method === 'POST') {
        const body = parseBody<{ kind: 'proxyUrl' | 'subscriptionUrl'; value: string }>({
          kind: 'proxyUrl',
          value: '',
        })
        const value = String(body.value || '').trim()
        if (!value) {
          return jsonResponse(
            {
              ok: false,
              message: 'empty candidate',
            },
            200,
          )
        }
        if (body.kind === 'subscriptionUrl') {
          const isHttp = value.startsWith('http://') || value.startsWith('https://')
          if (!isHttp) {
            return jsonResponse(
              {
                ok: false,
                message: 'subscription url must be http/https',
              },
              200,
            )
          }
          return jsonResponse({
            ok: true,
            message: 'subscription validation succeeded',
            normalizedValue: value,
            discoveredNodes: 3,
            latencyMs: 320,
          })
        }

        const acceptedSchemes = ['http://', 'https://', 'socks://', 'socks5://', 'socks5h://', 'vmess://', 'vless://', 'trojan://', 'ss://']
        if (!acceptedSchemes.some((prefix) => value.startsWith(prefix))) {
          return jsonResponse(
            {
              ok: false,
              message: 'unsupported proxy url scheme',
            },
            200,
          )
        }
        return jsonResponse({
          ok: true,
          message: 'proxy validation succeeded',
          normalizedValue: value,
          discoveredNodes: 1,
          latencyMs: 210,
        })
      }

      if (path === '/api/settings/pricing' && method === 'PUT') {
        const body = parseBody<{ catalogVersion: string; entries: PricingEntry[] }>({
          catalogVersion: settingsRef.current.pricing.catalogVersion,
          entries: settingsRef.current.pricing.entries,
        })
        settingsRef.current.pricing = {
          catalogVersion: String(body.catalogVersion || 'storybook'),
          entries: [...(body.entries || [])].sort((a, b) => a.model.localeCompare(b.model)),
        }
        persistSettings(storageKey, settingsRef.current)
        return jsonResponse(settingsRef.current.pricing)
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

type SettingsStoryParameters = {
  mockSettings?: SettingsPayload
}

const meta = {
  title: 'Settings/SettingsPage',
  component: SettingsPage,
  tags: ['autodocs'],
  parameters: {
    layout: 'fullscreen',
    viewport: { defaultViewport: 'desktop1660' },
  },
  decorators: [
    (Story, context) => {
      const mockSettings = (context.parameters as SettingsStoryParameters).mockSettings
      return (
        <I18nProvider>
          <StorybookSettingsMock
            initialSettings={mockSettings}
            storageKey={`${STORYBOOK_SETTINGS_STORAGE_PREFIX}.${context.id}`}
          >
            <div className="min-h-screen bg-base-200 px-6 py-6 text-base-content">
              <div className="app-shell-boundary">
                <Story />
              </div>
            </div>
          </StorybookSettingsMock>
        </I18nProvider>
      )
    },
  ],
} satisfies Meta<typeof SettingsPage>

export default meta

type Story = StoryObj<typeof meta>

export const Default: Story = {
  render: () => <SettingsPage />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByRole('heading', { name: '设置' })).toBeVisible()
    await expect(canvas.getByText('正向代理路由')).toBeVisible()
    await expect(canvas.getByText('价格配置')).toBeVisible()
  },
}

export const SubscriptionHeavy: Story = {
  parameters: {
    mockSettings: createStorySettings({
      forwardProxy: {
        proxyUrls: ['socks5://127.0.0.1:1080'],
        subscriptionUrls: [
          'https://example.com/subscription.base64',
          'https://example.com/backup-subscription.txt',
        ],
      },
    }),
  },
  render: () => <SettingsPage />,
}

export const PenalizedPool: Story = {
  parameters: {
    mockSettings: createStorySettings({
      forwardProxy: {
        proxyUrls: [
          'http://127.0.0.1:7890',
          'socks5://127.0.0.1:1080',
          'trojan://storybook-secret@trojan.example.com:443?security=tls&type=ws&host=cdn.example.com&path=%2Fedge#trojan-edge',
        ],
      },
    }),
  },
  render: () => <SettingsPage />,
}
