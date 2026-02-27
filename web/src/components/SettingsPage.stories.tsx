import { useEffect, useRef } from 'react'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { I18nProvider } from '../i18n'
import SettingsPage from '../pages/Settings'
import type { ForwardProxyNode, ForwardProxyNodeStats, ForwardProxySettings, PricingEntry, ProxySettings, SettingsPayload } from '../lib/api'

const PRESET_MODELS = ['gpt-5.3-codex', 'gpt-5.2-codex', 'gpt-5.1-codex-max', 'gpt-5.1-codex-mini', 'gpt-5.2']

const DEFAULT_PROXY_SETTINGS: ProxySettings = {
  hijackEnabled: true,
  mergeUpstreamEnabled: true,
  defaultHijackEnabled: false,
  models: PRESET_MODELS,
  enabledModels: ['gpt-5.3-codex', 'gpt-5.2-codex', 'gpt-5.1-codex-mini'],
}

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
    penalized: index >= 3,
    stats: statsPreset(index),
  }))

  const subscriptionNodes: ForwardProxyNode[] = settings.subscriptionUrls.map((subscriptionUrl, index) => {
    const virtualProxyUrl = `${subscriptionUrl}#node-${index + 1}`
    return {
      key: virtualProxyUrl,
      source: 'subscription',
      displayName: `sub-${index + 1}`,
      endpointUrl: virtualProxyUrl,
      weight: Number((0.65 - index * 0.12).toFixed(2)),
      penalized: false,
      stats: statsPreset(index + manualNodes.length),
    }
  })

  const nodes: ForwardProxyNode[] = [...manualNodes, ...subscriptionNodes]
  if (settings.insertDirect || nodes.length === 0) {
    nodes.push({
      key: '__direct__',
      source: 'direct',
      displayName: 'Direct',
      weight: 0.88,
      penalized: false,
      stats: statsPreset(nodes.length),
    })
  }

  return nodes
}

function cloneSettings(payload: SettingsPayload): SettingsPayload {
  return JSON.parse(JSON.stringify(payload)) as SettingsPayload
}

function StorybookSettingsMock({ children }: { children: React.ReactNode }) {
  const settingsRef = useRef<SettingsPayload>({
    proxy: DEFAULT_PROXY_SETTINGS,
    forwardProxy: {
      proxyUrls: [
        'http://127.0.0.1:7890',
        'socks5://127.0.0.1:1080',
      ],
      subscriptionUrls: ['https://example.com/subscription.base64'],
      subscriptionUpdateIntervalSecs: 3600,
      insertDirect: true,
      nodes: [],
    },
    pricing: {
      catalogVersion: 'storybook-2026-02-27',
      entries: DEFAULT_PRICING_ENTRIES,
    },
  })
  const originalFetchRef = useRef<typeof window.fetch | null>(null)
  const mockInstalledRef = useRef(false)

  if (typeof window !== 'undefined' && !mockInstalledRef.current) {
    mockInstalledRef.current = true
    const initial = settingsRef.current.forwardProxy
    settingsRef.current.forwardProxy = {
      ...initial,
      nodes: buildNodesFromSettings(initial),
    }

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

      if (path === '/api/settings/proxy' && method === 'PUT') {
        const body = parseBody<{ hijackEnabled: boolean; mergeUpstreamEnabled: boolean; enabledModels: string[] }>({
          hijackEnabled: false,
          mergeUpstreamEnabled: false,
          enabledModels: [],
        })
        const normalizedEnabledModels = settingsRef.current.proxy.models.filter((model) =>
          (body.enabledModels || []).includes(model),
        )
        settingsRef.current.proxy = {
          ...settingsRef.current.proxy,
          hijackEnabled: Boolean(body.hijackEnabled),
          mergeUpstreamEnabled: Boolean(body.hijackEnabled && body.mergeUpstreamEnabled),
          enabledModels: normalizedEnabledModels,
        }
        return jsonResponse(settingsRef.current.proxy)
      }

      if (path === '/api/settings/forward-proxy' && method === 'PUT') {
        const body = parseBody<{
          proxyUrls: string[]
          subscriptionUrls: string[]
          subscriptionUpdateIntervalSecs: number
          insertDirect: boolean
        }>({
          proxyUrls: [],
          subscriptionUrls: [],
          subscriptionUpdateIntervalSecs: 3600,
          insertDirect: true,
        })

        const nextForwardProxy: ForwardProxySettings = {
          ...settingsRef.current.forwardProxy,
          proxyUrls: (body.proxyUrls || []).map((item) => item.trim()).filter(Boolean),
          subscriptionUrls: (body.subscriptionUrls || []).map((item) => item.trim()).filter(Boolean),
          subscriptionUpdateIntervalSecs: Math.max(60, Math.floor(body.subscriptionUpdateIntervalSecs || 3600)),
          insertDirect: body.insertDirect !== false,
          nodes: [],
        }
        nextForwardProxy.nodes = buildNodesFromSettings(nextForwardProxy)
        settingsRef.current.forwardProxy = nextForwardProxy
        return jsonResponse(nextForwardProxy)
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

const meta = {
  title: 'Settings/SettingsPage',
  component: SettingsPage,
  parameters: {
    layout: 'fullscreen',
  },
  decorators: [
    (Story) => (
      <I18nProvider>
        <StorybookSettingsMock>
          <div data-theme="light" className="min-h-screen bg-base-200 px-6 py-6 text-base-content">
            <Story />
          </div>
        </StorybookSettingsMock>
      </I18nProvider>
    ),
  ],
} satisfies Meta<typeof SettingsPage>

export default meta

type Story = StoryObj<typeof meta>

export const Default: Story = {
  render: () => <SettingsPage />,
}

export const PenalizedPool: Story = {
  render: () => {
    return <SettingsPage />
  },
  play: async () => {
    // Keep this story variant listed for quick manual checks in Storybook.
  },
}
