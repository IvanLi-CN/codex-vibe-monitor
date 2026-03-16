/** @vitest-environment jsdom */
import { renderToStaticMarkup } from 'react-dom/server'
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { MemoryRouter } from 'react-router-dom'
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'
import { I18nProvider } from '../i18n'
import type { ApiInvocation } from '../lib/api'
import type { UpstreamAccountDetail } from '../lib/api'
import {
  formatProxyWeightDelta,
  formatServiceTier,
  getFastIndicatorState,
  isPriorityServiceTier,
} from '../lib/invocation'
import { InvocationTable } from './InvocationTable'
import { getReasoningEffortTone } from './invocation-table-reasoning'

const apiMocks = vi.hoisted(() => ({
  fetchUpstreamAccountDetail: vi.fn<(accountId: number) => Promise<UpstreamAccountDetail>>(),
}))

vi.mock('../lib/api', async () => {
  const actual = await vi.importActual<typeof import('../lib/api')>('../lib/api')
  return {
    ...actual,
    fetchUpstreamAccountDetail: apiMocks.fetchUpstreamAccountDetail,
  }
})

const LONG_PROXY_NAME = 'ivan-hkl-vless-vision-01KFXRNYWYXKN4JHCF3CCV78GD'
let host: HTMLDivElement | null = null
let root: Root | null = null

beforeAll(() => {
  Object.defineProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT', {
    configurable: true,
    writable: true,
    value: true,
  })
  Object.defineProperty(window, 'matchMedia', {
    configurable: true,
    writable: true,
    value: vi.fn().mockImplementation((query: string) => ({
      matches: query === '(min-width: 1280px)',
      media: query,
      onchange: null,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
      addListener: vi.fn(),
      removeListener: vi.fn(),
      dispatchEvent: vi.fn(),
    })),
  })
})

beforeEach(() => {
  apiMocks.fetchUpstreamAccountDetail.mockReset()
  host = document.createElement('div')
  document.body.appendChild(host)
  root = createRoot(host)
})

afterEach(async () => {
  await act(async () => {
    root?.unmount()
  })
  host?.remove()
  host = null
  root = null
  document.body.innerHTML = ''
})

function renderTable(records: ApiInvocation[]) {
  return renderToStaticMarkup(
    <I18nProvider>
      <InvocationTable records={records} isLoading={false} error={null} />
    </I18nProvider>,
  )
}

async function renderInteractiveTable(records: ApiInvocation[]) {
  await act(async () => {
    root?.render(
      <MemoryRouter>
        <I18nProvider>
          <InvocationTable records={records} isLoading={false} error={null} />
        </I18nProvider>
      </MemoryRouter>,
    )
  })
}

describe('formatProxyWeightDelta', () => {
  it('formats positive deltas as up direction with absolute value', () => {
    expect(formatProxyWeightDelta(0.55)).toEqual({ direction: 'up', value: '0.55' })
  })

  it('formats negative deltas as down direction and rounds to two decimals', () => {
    expect(formatProxyWeightDelta(-0.678)).toEqual({ direction: 'down', value: '0.68' })
  })

  it('formats zero as flat direction', () => {
    expect(formatProxyWeightDelta(0)).toEqual({ direction: 'flat', value: '0.00' })
    expect(formatProxyWeightDelta(-0)).toEqual({ direction: 'flat', value: '0.00' })
    expect(formatProxyWeightDelta(-0.004)).toEqual({ direction: 'flat', value: '0.00' })
  })

  it('falls back to em dash for missing or invalid values', () => {
    expect(formatProxyWeightDelta(undefined)).toEqual({ direction: 'missing', value: '—' })
    expect(formatProxyWeightDelta(null)).toEqual({ direction: 'missing', value: '—' })
    expect(formatProxyWeightDelta(Number.NaN)).toEqual({ direction: 'missing', value: '—' })
  })
})

describe('service tier helpers', () => {
  it('normalizes and formats service tiers', () => {
    expect(formatServiceTier(' Priority ')).toBe('priority')
    expect(formatServiceTier('FLEX')).toBe('flex')
  })

  it('falls back to em dash for empty or missing service tiers', () => {
    expect(formatServiceTier(undefined)).toBe('—')
    expect(formatServiceTier('   ')).toBe('—')
  })

  it('treats only priority as fast mode', () => {
    expect(isPriorityServiceTier('priority')).toBe(true)
    expect(isPriorityServiceTier(' Priority ')).toBe(true)
    expect(isPriorityServiceTier('flex')).toBe(false)
    expect(isPriorityServiceTier(undefined)).toBe(false)
  })

  it('resolves fast indicator states from requested and effective tiers', () => {
    expect(getFastIndicatorState('priority', 'priority')).toBe('effective')
    expect(getFastIndicatorState('priority', 'auto')).toBe('requested_only')
    expect(getFastIndicatorState('priority', undefined)).toBe('requested_only')
    expect(getFastIndicatorState('auto', 'priority')).toBe('effective')
    expect(getFastIndicatorState('flex', 'auto')).toBe('none')
  })
})

describe('getReasoningEffortTone', () => {
  it('maps standard effort values onto the visual ladder', () => {
    expect(getReasoningEffortTone('none')).toBe('none')
    expect(getReasoningEffortTone(' minimal ')).toBe('minimal')
    expect(getReasoningEffortTone('LOW')).toBe('low')
    expect(getReasoningEffortTone('medium')).toBe('medium')
    expect(getReasoningEffortTone('high')).toBe('high')
    expect(getReasoningEffortTone('xhigh')).toBe('xhigh')
  })

  it('treats unknown raw strings as unknown tone', () => {
    expect(getReasoningEffortTone('custom-tier')).toBe('unknown')
    expect(getReasoningEffortTone('constructor')).toBe('unknown')
    expect(getReasoningEffortTone('__proto__')).toBe('unknown')
  })
})

describe('InvocationTable', () => {
  it('renders resolved failure rows as failed even when the raw status still says running', () => {
    const html = renderTable([
      {
        id: 24,
        invokeId: 'invocation-resolved-failure-running',
        occurredAt: '2026-03-07T03:13:50Z',
        createdAt: '2026-03-07T03:13:50Z',
        source: 'proxy',
        proxyDisplayName: 'legacy-edge',
        endpoint: '/v1/responses',
        model: 'gpt-5.4',
        status: 'running',
        failureClass: 'service_failure',
        errorMessage: '[upstream_response_failed] server_error',
        totalTokens: 1024,
        cost: 0.0021,
      },
    ])

    expect(html).toContain('失败')
    expect(html).not.toContain('运行中')
  })

  it('renders reasoning effort and reasoning-token output breakdown in the summary rows', () => {
    const records: ApiInvocation[] = [
      {
        id: 1,
        invokeId: 'invocation-reasoning-high',
        occurredAt: '2026-03-07T03:13:59Z',
        createdAt: '2026-03-07T03:13:59Z',
        source: 'proxy',
        proxyDisplayName: 'tokyo-edge-1',
        endpoint: '/v1/responses',
        model: 'gpt-5.4',
        status: 'success',
        inputTokens: 45559,
        cacheInputTokens: 43520,
        outputTokens: 83,
        reasoningTokens: 41,
        reasoningEffort: 'high',
        totalTokens: 45642,
        cost: 0.0172,
        tUpstreamTtfbMs: 149.5,
        tTotalMs: 7794.1,
      },
      {
        id: 2,
        invokeId: 'invocation-reasoning-missing',
        occurredAt: '2026-03-07T03:13:56Z',
        createdAt: '2026-03-07T03:13:56Z',
        source: 'proxy',
        proxyDisplayName: 'singapore-edge-2',
        endpoint: '/v1/chat/completions',
        model: 'gpt-5.4',
        status: 'failed',
        inputTokens: 61402,
        cacheInputTokens: 41216,
        outputTokens: 286,
        totalTokens: 61688,
        errorMessage: 'upstream timeout',
        tUpstreamTtfbMs: 186.5,
        tTotalMs: 8444.2,
      },
    ]

    const html = renderTable(records)

    expect(html).toContain('推理强度')
    expect(html).toContain('推理 Tokens')
    expect(html).toContain('high')
    expect(html).toContain('推理 41')
    expect(html).toContain('推理 —')
    expect(html).toContain('/v1/responses')
    expect(html).toContain('/v1/chat/completions')
    expect(html).toContain('data-reasoning-effort-tone="high"')
    expect(html).toContain('border-warning/45')
    expect(html).toContain('>—</span>')
  })

  it('renders account/proxy and total-latency/compression summaries', () => {
    const html = renderTable([
      {
        id: 31,
        invokeId: 'invocation-account-summary',
        occurredAt: '2026-03-07T03:13:53Z',
        createdAt: '2026-03-07T03:13:53Z',
        source: 'proxy',
        routeMode: 'pool',
        upstreamAccountId: 7,
        upstreamAccountName: 'pool-account-a',
        proxyDisplayName: 'codex-relay-01',
        responseContentEncoding: 'gzip, br',
        endpoint: '/v1/responses',
        model: 'gpt-5.4',
        status: 'success',
        totalTokens: 2048,
        cost: 0.0042,
        tUpstreamTtfbMs: 118.2,
        tTotalMs: 910.4,
      },
      {
        id: 32,
        invokeId: 'invocation-reverse-proxy-summary',
        occurredAt: '2026-03-07T03:13:52Z',
        createdAt: '2026-03-07T03:13:52Z',
        source: 'proxy',
        routeMode: 'forward_proxy',
        proxyDisplayName: 'codex-relay-02',
        endpoint: '/v1/responses',
        model: 'gpt-5.4',
        status: 'success',
        totalTokens: 1024,
        cost: 0.0021,
        tUpstreamTtfbMs: 96.5,
        tTotalMs: 804.4,
      },
    ])

    expect(html).toContain('账号')
    expect(html).toContain('代理')
    expect(html).toContain('时延')
    expect(html).toContain('首字耗时')
    expect(html).toContain('首字耗时 / HTTP 压缩')
    expect(html).toContain('pool-account-a')
    expect(html).toContain('反向代理')
    expect(html).toContain('gzip, br')
    expect(html).toContain('data-testid="invocation-account-name"')
  })

  it('colors compact endpoint paths without adding extra summary badges', () => {
    const html = renderTable([
      {
        id: 21,
        invokeId: 'invocation-compact-marker',
        occurredAt: '2026-03-07T03:13:53Z',
        createdAt: '2026-03-07T03:13:53Z',
        source: 'proxy',
        proxyDisplayName: 'codex-compact-edge',
        endpoint: '/v1/responses/compact',
        model: 'gpt-5.3-codex',
        status: 'success',
        totalTokens: 2048,
        cost: 0.0042,
      },
      {
        id: 22,
        invokeId: 'invocation-standard-marker',
        occurredAt: '2026-03-07T03:13:52Z',
        createdAt: '2026-03-07T03:13:52Z',
        source: 'proxy',
        proxyDisplayName: LONG_PROXY_NAME,
        endpoint: '/v1/responses',
        model: 'gpt-5.3-codex',
        status: 'success',
        totalTokens: 1024,
        cost: 0.0021,
      },
    ])

    expect(html).not.toContain('data-testid="invocation-compact-badge"')
    expect(html.match(/data-testid="invocation-endpoint-path"/g)?.length ?? 0).toBe(4)
    expect(html.match(/data-endpoint-kind="compact"/g)?.length ?? 0).toBe(2)
    expect(html).toContain('text-info')
    expect(html).toContain('/v1/responses/compact')
  })

  it('renders stable proxy selectors for long proxy-name truncation coverage', () => {
    const html = renderTable([
      {
        id: 23,
        invokeId: 'invocation-long-proxy-name',
        occurredAt: '2026-03-07T03:13:51Z',
        createdAt: '2026-03-07T03:13:51Z',
        source: 'proxy',
        proxyDisplayName: LONG_PROXY_NAME,
        endpoint: '/v1/responses',
        model: 'gpt-5.4',
        status: 'success',
        totalTokens: 4096,
        cost: 0.0084,
      },
    ])

    expect(html.match(/data-testid="invocation-proxy-name"/g)?.length ?? 0).toBe(2)
    expect(html.match(/data-testid="invocation-proxy-badge"/g)?.length ?? 0).toBe(1)
    expect(html).toContain(`title="${LONG_PROXY_NAME}"`)
  })

  it('renders unknown reasoning effort values as dashed neutral badges', () => {
    const html = renderTable([
      {
        id: 3,
        invokeId: 'invocation-reasoning-unknown',
        occurredAt: '2026-03-07T03:13:54Z',
        createdAt: '2026-03-07T03:13:54Z',
        source: 'proxy',
        proxyDisplayName: 'sfo-edge-3',
        endpoint: '/v1/responses',
        model: 'custom-reasoning-model',
        status: 'success',
        inputTokens: 512,
        cacheInputTokens: 128,
        outputTokens: 64,
        reasoningTokens: 12,
        reasoningEffort: 'custom-tier',
        totalTokens: 576,
        cost: 0.0012,
        tUpstreamTtfbMs: 98.4,
        tTotalMs: 404.4,
      },
    ])

    expect(html).toContain('custom-tier')
    expect(html).toContain('data-reasoning-effort-tone="unknown"')
    expect(html).toContain('border-dashed')
  })

  it('renders effective and requested-only fast indicators with distinct states', () => {
    const html = renderTable([
      {
        id: 11,
        invokeId: 'priority-priority',
        occurredAt: '2026-03-07T03:13:59Z',
        createdAt: '2026-03-07T03:13:59Z',
        source: 'proxy',
        proxyDisplayName: 'tokyo-edge-1',
        endpoint: '/v1/responses',
        model: 'gpt-5.4',
        status: 'success',
        requestedServiceTier: 'priority',
        serviceTier: 'priority',
        totalTokens: 42,
      },
      {
        id: 12,
        invokeId: 'priority-auto',
        occurredAt: '2026-03-07T03:14:00Z',
        createdAt: '2026-03-07T03:14:00Z',
        source: 'proxy',
        proxyDisplayName: 'seoul-edge-2',
        endpoint: '/v1/responses',
        model: 'gpt-5.4',
        status: 'success',
        requestedServiceTier: 'priority',
        serviceTier: 'auto',
        totalTokens: 43,
      },
      {
        id: 13,
        invokeId: 'priority-missing',
        occurredAt: '2026-03-07T03:14:01Z',
        createdAt: '2026-03-07T03:14:01Z',
        source: 'proxy',
        proxyDisplayName: 'sfo-edge-3',
        endpoint: '/v1/responses',
        model: 'gpt-5.4',
        status: 'success',
        requestedServiceTier: 'priority',
        totalTokens: 44,
      },
      {
        id: 14,
        invokeId: 'auto-priority',
        occurredAt: '2026-03-07T03:14:02Z',
        createdAt: '2026-03-07T03:14:02Z',
        source: 'proxy',
        proxyDisplayName: 'singapore-edge-4',
        endpoint: '/v1/responses',
        model: 'gpt-5.4',
        status: 'success',
        requestedServiceTier: 'auto',
        serviceTier: 'priority',
        totalTokens: 45,
      },
      {
        id: 15,
        invokeId: 'flex-none',
        occurredAt: '2026-03-07T03:14:03Z',
        createdAt: '2026-03-07T03:14:03Z',
        source: 'proxy',
        proxyDisplayName: 'nyc-edge-5',
        endpoint: '/v1/responses',
        model: 'gpt-5.4',
        status: 'success',
        requestedServiceTier: 'flex',
        totalTokens: 46,
      },
    ])

    expect(html.match(/data-fast-state="effective"/g)?.length ?? 0).toBe(4)
    expect(html.match(/data-fast-state="requested_only"/g)?.length ?? 0).toBe(4)
    expect(html).toContain('Fast 模式（Priority processing）')
    expect(html).toContain('请求想要 Fast，但实际未命中 Priority processing')
  })

  it('opens the current-page upstream account drawer when clicking a pool account name', async () => {
    apiMocks.fetchUpstreamAccountDetail.mockResolvedValue({
      id: 42,
      kind: 'oauth_codex',
      provider: 'openai',
      displayName: 'Pool Alpha',
      groupName: 'group-a',
      isMother: true,
      status: 'active',
      enabled: true,
      email: 'pool-alpha@example.com',
      chatgptAccountId: 'org_pool_alpha',
      chatgptUserId: 'user_pool_alpha',
      planType: 'team',
      maskedApiKey: null,
      lastSyncedAt: '2026-03-16T09:10:00Z',
      lastSuccessfulSyncAt: '2026-03-16T09:08:00Z',
      lastError: null,
      lastErrorAt: null,
      tokenExpiresAt: '2026-03-16T12:00:00Z',
      lastRefreshedAt: '2026-03-16T09:09:00Z',
      primaryWindow: {
        usedPercent: 22,
        usedText: '22 / 100',
        limitText: '100 requests',
        resetsAt: '2026-03-16T10:00:00Z',
        windowDurationMins: 300,
      },
      secondaryWindow: {
        usedPercent: 36,
        usedText: '36 / 100',
        limitText: '100 requests',
        resetsAt: '2026-03-17T00:00:00Z',
        windowDurationMins: 10080,
      },
      credits: null,
      localLimits: null,
      duplicateInfo: null,
      tags: [],
      effectiveRoutingRule: {
        guardEnabled: false,
        lookbackHours: null,
        maxConversations: null,
        allowCutOut: true,
        allowCutIn: true,
        sourceTagIds: [],
        sourceTagNames: [],
        guardRules: [],
      },
      note: null,
      upstreamBaseUrl: null,
      history: [],
    })

    await renderInteractiveTable([
      {
        id: 41,
        invokeId: 'pool-drawer-open',
        occurredAt: '2026-03-16T09:10:30Z',
        createdAt: '2026-03-16T09:10:30Z',
        source: 'proxy',
        routeMode: 'pool',
        upstreamAccountId: 42,
        upstreamAccountName: 'Pool Alpha',
        proxyDisplayName: 'relay-alpha',
        responseContentEncoding: 'gzip',
        endpoint: '/v1/responses',
        model: 'gpt-5.4',
        status: 'success',
        totalTokens: 512,
        tUpstreamTtfbMs: 104.4,
        tTotalMs: 702.3,
      },
    ])

    const trigger = Array.from(document.querySelectorAll('button')).find((button) => button.textContent?.includes('Pool Alpha'))
    expect(trigger).toBeTruthy()

    await act(async () => {
      trigger?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
      await Promise.resolve()
    })

    expect(apiMocks.fetchUpstreamAccountDetail).toHaveBeenCalledWith(42)
    expect(document.body.textContent).toContain('上游账号')
    expect(document.body.textContent).toContain('Pool Alpha')
    expect(document.body.textContent).toContain('去号池查看完整详情')
  })

  it('keeps structured-only metadata out of summary rows', () => {
    const records: ApiInvocation[] = [
      {
        id: 4,
        invokeId: 'invocation-detail-pruned',
        occurredAt: '2026-03-07T03:13:52Z',
        createdAt: '2026-03-07T03:13:52Z',
        source: 'proxy',
        proxyDisplayName: 'hkg-edge-4',
        endpoint: '/v1/responses',
        model: 'gpt-5.4',
        status: 'success',
        inputTokens: 1024,
        outputTokens: 64,
        totalTokens: 1088,
        cost: 0.0021,
        detailLevel: 'structured_only',
        detailPrunedAt: '2026-02-01T12:34:56Z',
        detailPruneReason: 'success_over_30d',
      },
    ]

    const summaryHtml = renderTable(records)
    expect(summaryHtml).not.toContain('data-testid="invocation-detail-level-badge"')
    expect(summaryHtml).not.toContain('Structured only')
    expect(summaryHtml).not.toContain('精简于 2026-02-01 12:34:56Z')

  })

  it('keeps legacy full-detail records out of summary rows', () => {
    const records: ApiInvocation[] = [
      {
        id: 5,
        invokeId: 'invocation-detail-full-default',
        occurredAt: '2026-03-07T03:13:50Z',
        createdAt: '2026-03-07T03:13:50Z',
        source: 'xy',
        endpoint: '/v1/chat/completions',
        model: 'gpt-4.1',
        status: 'failed',
        errorMessage: 'legacy row still renders',
      },
    ]

    const summaryHtml = renderTable(records)
    expect(summaryHtml).not.toContain('data-testid="invocation-detail-level-badge"')
    expect(summaryHtml).not.toContain('Full')
    expect(summaryHtml).not.toContain('Structured only')
    expect(summaryHtml).not.toContain('精简于')
    expect(summaryHtml).toContain('legacy row still renders')

  })

})
