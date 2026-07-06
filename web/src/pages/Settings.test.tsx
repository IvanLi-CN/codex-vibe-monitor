/** @vitest-environment jsdom */
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { I18nProvider } from '../i18n'
import type { SettingsPayload } from '../lib/api'
import SettingsPage from './Settings'

const hookMocks = vi.hoisted(() => ({
  useSettings: vi.fn(),
}))

vi.mock('../hooks/useSettings', () => ({
  useSettings: hookMocks.useSettings,
}))

vi.mock('../components/ExternalApiKeysSettingsCard', () => ({
  ExternalApiKeysSettingsCard: () => <section data-testid="external-api-keys-settings-card" />,
}))

let host: HTMLDivElement | null = null
let root: Root | null = null

function createSettingsPayload(): SettingsPayload {
  return {
    proxy: {
      hijackEnabled: false,
      mergeUpstreamEnabled: false,
      fastModeRewriteMode: 'disabled',
      upstream429MaxRetries: 3,
      websocketEnabled: false,
      upstreamWebsocketDefaultEnabled: false,
      requestBodyLoggingEnabled: true,
      responseBodyLoggingEnabled: true,
      encryptedSessionOwnerRoutingEnabled: false,
      defaultHijackEnabled: false,
      models: ['gpt-5.5'],
      enabledModels: ['gpt-5.5'],
    },
    forwardProxy: {
      proxyUrls: ['http://proxy-order.example.com:8080'],
      subscriptionUrls: [],
      subscriptionUpdateIntervalSecs: 3600,
      nodes: [
        {
          key: 'proxy-order',
          source: 'manual',
          displayName: 'proxy-order.example.com:8080',
          endpointUrl: 'http://proxy-order.example.com:8080',
          weight: 0.75,
          penalized: false,
          stats: {
            oneMinute: { attempts: 1, successRate: 0.11, avgLatencyMs: 101 },
            fifteenMinutes: { attempts: 2, successRate: 0.22, avgLatencyMs: 202 },
            oneHour: { attempts: 3, successRate: 0.33, avgLatencyMs: 303 },
            oneDay: { attempts: 4, successRate: 0.44, avgLatencyMs: 404 },
            sevenDays: { attempts: 5, successRate: 0.55, avgLatencyMs: 505 },
          },
        },
      ],
    },
    pricing: {
      catalogVersion: 'test',
      entries: [],
    },
  }
}

function renderSettingsPage() {
  host = document.createElement('div')
  document.body.appendChild(host)
  root = createRoot(host)

  act(() => {
    root?.render(
      <I18nProvider>
        <SettingsPage />
      </I18nProvider>,
    )
  })
}

function expectTextInOrder(content: string, values: string[]) {
  let previousIndex = -1
  for (const value of values) {
    const nextIndex = content.indexOf(value, previousIndex + 1)
    expect(nextIndex, `${value} should appear after index ${previousIndex}`).toBeGreaterThan(previousIndex)
    previousIndex = nextIndex
  }
}

describe('Settings forward proxy table', () => {
  beforeEach(() => {
    hookMocks.useSettings.mockReturnValue({
      settings: createSettingsPayload(),
      isLoading: false,
      isProxySaving: false,
      isForwardProxySaving: false,
      isPricingSaving: false,
      pricingRollbackVersion: 0,
      error: null,
      refresh: vi.fn(),
      saveProxy: vi.fn(),
      saveForwardProxy: vi.fn(),
      savePricing: vi.fn(),
    })
  })

  afterEach(() => {
    act(() => {
      root?.unmount()
    })
    host?.remove()
    host = null
    root = null
    hookMocks.useSettings.mockReset()
  })

  it('renders forward proxy desktop and mobile windows from longest to shortest window', () => {
    renderSettingsPage()

    const desktopTable = host?.querySelector('[data-testid="settings-forward-proxy-desktop-table"]')
    if (!(desktopTable instanceof HTMLTableElement)) {
      throw new Error('Missing forward proxy desktop table')
    }
    const desktopHeaders = Array.from(desktopTable.querySelectorAll('thead th')).map((cell) => cell.textContent ?? '')
    expect(desktopHeaders.slice(2, 7)).toEqual(['7 天', '1 天', '1 小时', '15 分钟', '1 分钟'])

    const desktopCells = Array.from(desktopTable.querySelectorAll('tbody tr:first-child td')).map((cell) => cell.textContent ?? '')
    expect(desktopCells.slice(2, 7)).toEqual([
      '55.0%505 ms',
      '44.0%404 ms',
      '33.0%303 ms',
      '22.0%202 ms',
      '11.0%101 ms',
    ])

    const mobileWindows = host?.querySelector('[data-testid="settings-forward-proxy-mobile-windows"]')
    if (!(mobileWindows instanceof HTMLElement)) {
      throw new Error('Missing forward proxy mobile windows')
    }
    expectTextInOrder(mobileWindows.textContent ?? '', [
      '7 天',
      '55.0%',
      '505 ms',
      '1 天',
      '44.0%',
      '404 ms',
      '1 小时',
      '33.0%',
      '303 ms',
      '15 分钟',
      '22.0%',
      '202 ms',
      '1 分钟',
      '11.0%',
      '101 ms',
    ])
  })

  it('renders independent request/response body logging switches', () => {
    renderSettingsPage()

    expect(host?.textContent).toContain('记录请求 body')
    expect(host?.textContent).toContain('记录响应 body')
  })

  it('persists the encrypted owner routing toggle through proxy settings', () => {
    const saveProxy = vi.fn()
    hookMocks.useSettings.mockReturnValue({
      settings: createSettingsPayload(),
      isLoading: false,
      isProxySaving: false,
      isForwardProxySaving: false,
      isPricingSaving: false,
      pricingRollbackVersion: 0,
      error: null,
      refresh: vi.fn(),
      saveProxy,
      saveForwardProxy: vi.fn(),
      savePricing: vi.fn(),
    })

    renderSettingsPage()

    const toggle = host?.querySelector('button[aria-label="加密对话路由绑定"]')
    if (!(toggle instanceof HTMLButtonElement)) {
      throw new Error('Missing encrypted owner routing toggle')
    }
    act(() => {
      toggle.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    })

    expect(saveProxy).toHaveBeenCalledTimes(1)
    expect(saveProxy.mock.calls[0]?.[0]).toMatchObject({
      encryptedSessionOwnerRoutingEnabled: true,
    })
  })
})
