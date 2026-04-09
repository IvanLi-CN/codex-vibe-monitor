/** @vitest-environment jsdom */
import { renderToStaticMarkup } from 'react-dom/server'
import { act, type ComponentProps } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { MemoryRouter } from 'react-router-dom'
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'
import { I18nProvider, useTranslation } from '../i18n'
import type { ApiInvocation } from '../lib/api'
import type { UpstreamAccountDetail } from '../lib/api'
import type { BroadcastPayload } from '../lib/api'
import {
  formatProxyWeightDelta,
  formatServiceTier,
  getFastIndicatorState,
  isPriorityServiceTier,
  resolveInvocationEndpointDisplay,
} from '../lib/invocation'
import { InvocationTable } from './InvocationTable'
import { getReasoningEffortTone } from './invocation-table-reasoning'
import {
  buildInvocationDetailViewModel,
  InvocationExpandedDetails,
} from './invocation-details-shared'

const apiMocks = vi.hoisted(() => ({
  fetchUpstreamAccountDetail: vi.fn<(accountId: number) => Promise<UpstreamAccountDetail>>(),
  fetchInvocationPoolAttempts: vi.fn(),
}))

const sseMocks = vi.hoisted(() => ({
  onMessage: null as null | ((payload: BroadcastPayload) => void),
}))

vi.mock('../lib/api', async () => {
  const actual = await vi.importActual<typeof import('../lib/api')>('../lib/api')
  return {
    ...actual,
    fetchUpstreamAccountDetail: apiMocks.fetchUpstreamAccountDetail,
    fetchInvocationPoolAttempts: apiMocks.fetchInvocationPoolAttempts,
  }
})

vi.mock('../lib/sse', () => ({
  subscribeToSse: (handler: (payload: BroadcastPayload) => void) => {
    sseMocks.onMessage = handler
    return () => {
      sseMocks.onMessage = null
    }
  },
}))

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
  apiMocks.fetchInvocationPoolAttempts.mockReset()
  apiMocks.fetchInvocationPoolAttempts.mockResolvedValue([])
  sseMocks.onMessage = null
  host = document.createElement('div')
  document.body.appendChild(host)
  root = createRoot(host)
})

afterEach(async () => {
  vi.useRealTimers()
  await act(async () => {
    root?.unmount()
  })
  sseMocks.onMessage = null
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

async function renderInteractiveTable(
  records: ApiInvocation[],
  props: Partial<ComponentProps<typeof InvocationTable>> = {},
) {
  await act(async () => {
    root?.render(
      <MemoryRouter>
        <I18nProvider>
          <InvocationTable records={records} isLoading={false} error={null} {...props} />
        </I18nProvider>
      </MemoryRouter>,
    )
  })
}

function InvocationDetailProbe({ record }: { record: ApiInvocation }) {
  const { t, locale } = useTranslation()
  const localeTag = locale === 'zh' ? 'zh-CN' : 'en-US'
  const detailView = buildInvocationDetailViewModel({
    record,
    normalizedStatus: (record.status ?? 'unknown').toLowerCase(),
    t,
    locale,
    localeTag,
    nowMs: Date.now(),
    numberFormatter: new Intl.NumberFormat(localeTag),
    currencyFormatter: new Intl.NumberFormat(localeTag, {
      style: 'currency',
      currency: 'USD',
      minimumFractionDigits: 4,
      maximumFractionDigits: 4,
    }),
    renderAccountValue: (accountLabel) => <span>{accountLabel}</span>,
  })

  return (
    <InvocationExpandedDetails
      record={record}
      detailId="invocation-detail-probe"
      detailPairs={detailView.detailPairs}
      timingPairs={detailView.timingPairs}
      errorMessage={detailView.errorMessage}
      detailNotice={detailView.detailNotice}
      size="default"
      poolAttemptsState={{ attemptsByInvokeId: {}, loadingByInvokeId: {}, errorByInvokeId: {} }}
      t={t}
    />
  )
}

async function waitForCondition(
  predicate: () => boolean,
  options?: { attempts?: number; delayMs?: number },
) {
  const attempts = options?.attempts ?? 25
  const delayMs = options?.delayMs ?? 0

  for (let index = 0; index < attempts; index += 1) {
    if (predicate()) return
    await act(async () => {
      await new Promise((resolve) => window.setTimeout(resolve, delayMs))
    })
  }

  throw new Error('Condition was not met before timeout')
}

function createDeferredPromise<T>() {
  let resolve!: (value: T | PromiseLike<T>) => void
  let reject!: (reason?: unknown) => void
  const promise = new Promise<T>((innerResolve, innerReject) => {
    resolve = innerResolve
    reject = innerReject
  })
  return { promise, resolve, reject }
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

  it('resolves fast indicator states from requested and billing tiers', () => {
    expect(getFastIndicatorState('priority', 'priority', 'priority')).toBe('effective')
    expect(getFastIndicatorState('priority', 'default', 'priority')).toBe('effective')
    expect(getFastIndicatorState('priority', 'auto')).toBe('requested_only')
    expect(getFastIndicatorState('priority', undefined)).toBe('requested_only')
    expect(getFastIndicatorState('auto', 'priority')).toBe('none')
    expect(getFastIndicatorState('flex', 'auto')).toBe('none')
  })
})

describe('resolveInvocationEndpointDisplay', () => {
  it('maps the recognized invocation endpoints onto badge metadata', () => {
    expect(resolveInvocationEndpointDisplay(' /v1/responses ')).toEqual({
      kind: 'responses',
      endpointValue: '/v1/responses',
      badgeVariant: 'default',
      labelKey: 'table.endpoint.responsesBadge',
    })
    expect(resolveInvocationEndpointDisplay('/v1/chat/completions')).toEqual({
      kind: 'chat',
      endpointValue: '/v1/chat/completions',
      badgeVariant: 'secondary',
      labelKey: 'table.endpoint.chatBadge',
    })
    expect(resolveInvocationEndpointDisplay('/v1/responses/compact')).toEqual({
      kind: 'compact',
      endpointValue: '/v1/responses/compact',
      badgeVariant: 'info',
      labelKey: 'table.endpoint.compactBadge',
    })
  })

  it('keeps unknown or missing endpoints on the raw fallback path', () => {
    expect(resolveInvocationEndpointDisplay('/v1/responses/experimental')).toEqual({
      kind: 'raw',
      endpointValue: '/v1/responses/experimental',
      badgeVariant: null,
      labelKey: null,
    })
    expect(resolveInvocationEndpointDisplay(undefined)).toEqual({
      kind: 'raw',
      endpointValue: '—',
      badgeVariant: null,
      labelKey: null,
    })
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

  it('preserves nonstandard terminal statuses in the badge label', () => {
    const html = renderTable([
      {
        id: 25,
        invokeId: 'invocation-http-502',
        occurredAt: '2026-03-07T03:13:49Z',
        createdAt: '2026-03-07T03:13:49Z',
        source: 'proxy',
        proxyDisplayName: 'legacy-edge',
        endpoint: '/v1/responses',
        model: 'gpt-5.4',
        status: 'http_502',
        failureClass: 'service_failure',
        totalTokens: 1024,
        cost: 0.0021,
      },
    ])

    expect(html).toContain('HTTP 502')
    expect(html).not.toContain('>失败<')
  })

  it('falls back to downstream-facing diagnostics in collapsed summaries when canonical upstream text is empty', () => {
    const html = renderTable([
      {
        id: 26,
        invokeId: 'invocation-downstream-summary',
        occurredAt: '2026-03-07T03:13:48Z',
        createdAt: '2026-03-07T03:13:48Z',
        source: 'proxy',
        proxyDisplayName: 'legacy-edge',
        endpoint: '/v1/responses',
        model: 'gpt-5.4',
        status: 'failed',
        failureClass: 'client_abort',
        failureKind: 'downstream_closed',
        downstreamStatusCode: 200,
        downstreamErrorMessage:
          '[downstream_closed] downstream closed while streaming upstream response',
      },
    ])

    expect(html).toContain('[downstream_closed] downstream closed while streaming upstream response')
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
        tReqReadMs: 3200,
        tReqParseMs: 20,
        tUpstreamConnectMs: 456.7,
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
    expect(html).toContain('7.79 s')
    expect(html).toContain('3.83 s')
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
        tReqReadMs: 31,
        tReqParseMs: 1,
        tUpstreamConnectMs: 9330,
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
        tReqReadMs: 120,
        tReqParseMs: 18,
        tUpstreamConnectMs: 512,
        tUpstreamTtfbMs: 96.5,
        tTotalMs: 804.4,
      },
    ])

    expect(html).toContain('账号')
    expect(html).toContain('代理')
    expect(html).toContain('用时')
    expect(html).toContain('首字总耗时')
    expect(html).toContain('首字总耗时 / HTTP 压缩')
    expect(html).toContain('9.48 s')
    expect(html).toContain('pool-account-a')
    expect(html).toContain('反向代理')
    expect(html).toContain('gzip, br')
    expect(html).toContain('data-testid="invocation-account-name"')
  })

  it('shows a neutral pool-routing label before the upstream account identity is known', () => {
    const html = renderTable([
      {
        id: 34,
        invokeId: 'invocation-pool-routing-pending',
        occurredAt: '2026-03-07T03:13:50Z',
        createdAt: '2026-03-07T03:13:50Z',
        source: 'proxy',
        routeMode: 'pool',
        proxyDisplayName: 'codex-relay-03',
        endpoint: '/v1/responses',
        model: 'gpt-5.4',
        status: 'running',
      },
      {
        id: 35,
        invokeId: 'invocation-pool-routing-id-only',
        occurredAt: '2026-03-07T03:13:49Z',
        createdAt: '2026-03-07T03:13:49Z',
        source: 'proxy',
        routeMode: 'pool',
        upstreamAccountId: 19,
        proxyDisplayName: 'codex-relay-04',
        endpoint: '/v1/responses',
        model: 'gpt-5.4',
        status: 'running',
      },
      {
        id: 36,
        invokeId: 'invocation-forward-proxy-fallback',
        occurredAt: '2026-03-07T03:13:48Z',
        createdAt: '2026-03-07T03:13:48Z',
        source: 'proxy',
        routeMode: 'forward_proxy',
        proxyDisplayName: 'codex-relay-05',
        endpoint: '/v1/responses',
        model: 'gpt-5.4',
        status: 'running',
      },
      {
        id: 37,
        invokeId: 'invocation-pool-account-unavailable',
        occurredAt: '2026-03-07T03:13:47Z',
        createdAt: '2026-03-07T03:13:47Z',
        source: 'proxy',
        routeMode: 'pool',
        proxyDisplayName: 'codex-relay-06',
        endpoint: '/v1/responses',
        model: 'gpt-5.4',
        status: 'success',
      },
    ])

    expect(html).toContain('号池路由中')
    expect(html).toContain('账号 #19')
    expect(html).toContain('号池账号未知')
    expect(html).toContain('反向代理')
  })

  it('uses the resolved display status when deciding whether a pool label is still pending', () => {
    const html = renderTable([
      {
        id: 38,
        invokeId: 'invocation-pool-resolved-failure',
        occurredAt: '2026-03-07T03:13:46Z',
        createdAt: '2026-03-07T03:13:46Z',
        source: 'proxy',
        routeMode: 'pool',
        proxyDisplayName: 'codex-relay-07',
        endpoint: '/v1/responses',
        model: 'gpt-5.4',
        status: 'running',
        failureClass: 'service_failure',
        errorMessage: '[upstream_response_failed] server_error',
      },
    ])

    expect(html).toContain('失败')
    expect(html).toContain('号池账号未知')
    expect(html).not.toContain('号池路由中')
  })

  it('shows proxyDisplayName in both summary and expanded details when present', async () => {
    await renderInteractiveTable([
      {
        id: 33,
        invokeId: 'invocation-proxy-detail-visible',
        occurredAt: '2026-03-07T03:13:51Z',
        createdAt: '2026-03-07T03:13:51Z',
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
    ])

    const beforeExpandMatches = document.body.textContent?.match(/codex-relay-01/g) ?? []
    expect(beforeExpandMatches.length).toBeGreaterThanOrEqual(1)

    const toggle = document.querySelector(
      '[data-testid="invocation-table-scroll"] button[aria-expanded="false"]',
    ) as HTMLButtonElement | null
    expect(toggle).toBeTruthy()

    await act(async () => {
      toggle?.click()
      await Promise.resolve()
      await Promise.resolve()
    })

    expect(document.body.textContent).toContain('请求详情')
    const afterExpandMatches = document.body.textContent?.match(/codex-relay-01/g) ?? []
    expect(afterExpandMatches.length).toBeGreaterThanOrEqual(2)
  })

  it('keeps latency summary fields out of request details while preserving stage timings', async () => {
    const record: ApiInvocation = {
      id: 34,
      invokeId: 'invocation-detail-deduped-latency',
      occurredAt: '2026-03-24T06:48:52Z',
      createdAt: '2026-03-24T06:48:52Z',
      source: 'proxy',
      routeMode: 'pool',
      upstreamAccountId: 7,
      upstreamAccountName: 'pool-account-a',
      proxyDisplayName: 'storybook-proxy',
      responseContentEncoding: 'gzip',
      endpoint: '/v1/responses',
      model: 'gpt-5.4',
      status: 'success',
      totalTokens: 2236,
      cost: 0.0046,
      tUpstreamConnectMs: 26,
      tUpstreamTtfbMs: 148.2,
      tUpstreamStreamMs: 613,
      tTotalMs: 805,
    }

    const html = renderToStaticMarkup(
      <I18nProvider>
        <InvocationDetailProbe record={record} />
      </I18nProvider>,
    )

    expect(html).toContain('请求详情')
    expect(html).toContain('HTTP 压缩算法')
    expect(html).not.toContain('>用时<')
    expect(html).not.toContain('>首字耗时<')
    expect(html).toContain('阶段耗时')
    expect(html).toContain('上游首字节')
    expect(html).toContain('总耗时')
  })

  it('shows requested, response, and billing service tiers in request details', () => {
    const record: ApiInvocation = {
      id: 35,
      invokeId: 'invocation-detail-billing-service-tier',
      occurredAt: '2026-03-24T06:49:52Z',
      createdAt: '2026-03-24T06:49:52Z',
      source: 'proxy',
      routeMode: 'pool',
      upstreamAccountId: 17,
      upstreamAccountName: 'API Keys Pool',
      proxyDisplayName: 'api-keys-gateway',
      endpoint: '/v1/responses',
      model: 'gpt-5.4',
      status: 'success',
      requestedServiceTier: 'priority',
      serviceTier: 'default',
      billingServiceTier: 'priority',
      totalTokens: 4096,
      cost: 0.1024,
    }

    const html = renderToStaticMarkup(
      <I18nProvider>
        <InvocationDetailProbe record={record} />
      </I18nProvider>,
    )

    expect(html).toContain('Requested service tier')
    expect(html).toContain('Service tier')
    expect(html).toContain('Billing service tier')
  })

  it('lazy-loads pool attempts inside expanded details', async () => {
    apiMocks.fetchInvocationPoolAttempts.mockResolvedValue([
      {
        id: 1,
        invokeId: 'invocation-pool-attempts-visible',
        occurredAt: '2026-03-07T03:13:51Z',
        endpoint: '/v1/responses',
        upstreamAccountId: 7,
        upstreamAccountName: 'pool-account-a',
        attemptIndex: 1,
        distinctAccountIndex: 1,
        sameAccountRetryIndex: 1,
        startedAt: '2026-03-07T03:13:51Z',
        finishedAt: '2026-03-07T03:13:52Z',
        status: 'success',
        httpStatus: 200,
        connectLatencyMs: 42.3,
        firstByteLatencyMs: 15.2,
        streamLatencyMs: 188.4,
        upstreamRequestId: 'req_pool_123',
        createdAt: '2026-03-07T03:13:52Z',
      },
    ])

    await renderInteractiveTable([
      {
        id: 40,
        invokeId: 'invocation-pool-attempts-visible',
        occurredAt: '2026-03-07T03:13:51Z',
        createdAt: '2026-03-07T03:13:51Z',
        source: 'proxy',
        routeMode: 'pool',
        upstreamAccountId: 7,
        upstreamAccountName: 'pool-account-a',
        proxyDisplayName: 'codex-relay-01',
        endpoint: '/v1/responses',
        model: 'gpt-5.4',
        status: 'success',
        poolAttemptCount: 1,
        poolDistinctAccountCount: 1,
        totalTokens: 2048,
        cost: 0.0042,
      },
    ])

    const toggle = Array.from(document.querySelectorAll('button')).find(
      (button) => button.getAttribute('aria-expanded') === 'false',
    )
    expect(toggle).toBeTruthy()

    await act(async () => {
      toggle?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
      await Promise.resolve()
    })

    expect(apiMocks.fetchInvocationPoolAttempts).toHaveBeenCalledWith(
      'invocation-pool-attempts-visible',
    )

    await waitForCondition(
      () => document.querySelector('[data-testid="pool-attempt-item"]') !== null,
    )

    expect(document.body.textContent).toContain('号池尝试明细')
    expect(document.body.textContent).toContain('pool-account-a')
    expect(document.body.textContent).toContain('成功')
  })

  it('replaces an empty cached pool-attempt detail with live SSE attempts', async () => {
    apiMocks.fetchInvocationPoolAttempts.mockResolvedValue([])

    await renderInteractiveTable([
      {
        id: 42,
        invokeId: 'invocation-pool-attempts-live-sse',
        occurredAt: '2026-03-07T03:13:51Z',
        createdAt: '2026-03-07T03:13:51Z',
        source: 'proxy',
        routeMode: 'pool',
        upstreamAccountId: 7,
        upstreamAccountName: 'pool-account-a',
        endpoint: '/v1/responses',
        model: 'gpt-5.4',
        status: 'running',
        poolAttemptCount: 0,
      },
    ])

    const toggle = Array.from(document.querySelectorAll('button')).find(
      (button) => button.getAttribute('aria-expanded') === 'false',
    )
    expect(toggle).toBeTruthy()

    await act(async () => {
      toggle?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
      await Promise.resolve()
    })

    expect(apiMocks.fetchInvocationPoolAttempts).toHaveBeenCalledWith(
      'invocation-pool-attempts-live-sse',
    )
    expect(sseMocks.onMessage).toBeTruthy()

    await act(async () => {
      sseMocks.onMessage?.({
        type: 'pool_attempts',
        invokeId: 'invocation-pool-attempts-live-sse',
        attempts: [
          {
            id: 9,
            invokeId: 'invocation-pool-attempts-live-sse',
            occurredAt: '2026-03-07T03:13:51Z',
            endpoint: '/v1/responses',
            upstreamAccountId: 7,
            upstreamAccountName: 'pool-account-a',
            attemptIndex: 1,
            distinctAccountIndex: 1,
            sameAccountRetryIndex: 1,
            startedAt: '2026-03-07T03:13:51Z',
            finishedAt: null,
            status: 'pending',
            phase: 'sending_request',
            httpStatus: null,
            connectLatencyMs: null,
            firstByteLatencyMs: null,
            streamLatencyMs: null,
            upstreamRequestId: null,
            createdAt: '2026-03-07T03:13:51Z',
          },
        ],
      })
      await Promise.resolve()
    })

    await waitForCondition(() => document.body.textContent?.includes('进行中') === true)

    expect(document.body.textContent).toContain('pool-account-a')
    expect(document.body.textContent).toContain('进行中')
    expect(document.body.textContent).toContain('发送请求中')
  })

  it('shows newer pending attempt phases without letting older pending snapshots roll them back', async () => {
    apiMocks.fetchInvocationPoolAttempts.mockResolvedValue([])

    await renderInteractiveTable([
      {
        id: 420,
        invokeId: 'invocation-pool-attempts-phase-ordering',
        occurredAt: '2026-03-07T03:13:51Z',
        createdAt: '2026-03-07T03:13:51Z',
        source: 'proxy',
        routeMode: 'pool',
        upstreamAccountId: 7,
        upstreamAccountName: 'pool-account-a',
        endpoint: '/v1/responses',
        model: 'gpt-5.4',
        status: 'running',
        poolAttemptCount: 1,
      },
    ])

    const toggle = Array.from(document.querySelectorAll('button')).find(
      (button) => button.getAttribute('aria-expanded') === 'false',
    )
    expect(toggle).toBeTruthy()

    await act(async () => {
      toggle?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
      await Promise.resolve()
    })

    await act(async () => {
      sseMocks.onMessage?.({
        type: 'pool_attempts',
        invokeId: 'invocation-pool-attempts-phase-ordering',
        attempts: [
          {
            id: 90,
            invokeId: 'invocation-pool-attempts-phase-ordering',
            occurredAt: '2026-03-07T03:13:51Z',
            endpoint: '/v1/responses',
            upstreamAccountId: 7,
            upstreamAccountName: 'pool-account-a',
            attemptIndex: 1,
            distinctAccountIndex: 1,
            sameAccountRetryIndex: 1,
            startedAt: '2026-03-07T03:13:51Z',
            finishedAt: null,
            status: 'pending',
            phase: 'streaming_response',
            httpStatus: null,
            connectLatencyMs: 26,
            firstByteLatencyMs: 148,
            streamLatencyMs: null,
            upstreamRequestId: null,
            createdAt: '2026-03-07T03:13:51Z',
          },
        ],
      })
      await Promise.resolve()
    })

    await waitForCondition(() => document.body.textContent?.includes('接收中') === true)

    await act(async () => {
      sseMocks.onMessage?.({
        type: 'pool_attempts',
        invokeId: 'invocation-pool-attempts-phase-ordering',
        attempts: [
          {
            id: 90,
            invokeId: 'invocation-pool-attempts-phase-ordering',
            occurredAt: '2026-03-07T03:13:51Z',
            endpoint: '/v1/responses',
            upstreamAccountId: 7,
            upstreamAccountName: 'pool-account-a',
            attemptIndex: 1,
            distinctAccountIndex: 1,
            sameAccountRetryIndex: 1,
            startedAt: '2026-03-07T03:13:51Z',
            finishedAt: null,
            status: 'pending',
            phase: 'sending_request',
            httpStatus: null,
            connectLatencyMs: null,
            firstByteLatencyMs: null,
            streamLatencyMs: null,
            upstreamRequestId: null,
            createdAt: '2026-03-07T03:13:51Z',
          },
        ],
      })
      await Promise.resolve()
    })

    expect(document.body.textContent).toContain('接收中')
    expect(document.body.textContent).not.toContain('发送请求中')
  })

  it('keeps newer SSE pool attempts when an older fetch resolves later', async () => {
    const deferred = createDeferredPromise<
      Array<{
        id: number
        invokeId: string
        occurredAt: string
        endpoint: string
        upstreamAccountId: number
        upstreamAccountName: string
        attemptIndex: number
        distinctAccountIndex: number
        sameAccountRetryIndex: number
        startedAt: string
        finishedAt: string | null
        status: string
        httpStatus: number | null
        connectLatencyMs: number | null
        firstByteLatencyMs: number | null
        streamLatencyMs: number | null
        upstreamRequestId: string | null
        createdAt: string
      }>
    >()
    apiMocks.fetchInvocationPoolAttempts.mockReturnValue(deferred.promise)

    await renderInteractiveTable([
      {
        id: 43,
        invokeId: 'invocation-pool-attempts-stale-fetch',
        occurredAt: '2026-03-07T03:13:51Z',
        createdAt: '2026-03-07T03:13:51Z',
        source: 'proxy',
        routeMode: 'pool',
        upstreamAccountId: 7,
        upstreamAccountName: 'pool-account-a',
        endpoint: '/v1/responses',
        model: 'gpt-5.4',
        status: 'running',
        poolAttemptCount: 1,
      },
    ])

    const toggle = Array.from(document.querySelectorAll('button')).find(
      (button) => button.getAttribute('aria-expanded') === 'false',
    )
    expect(toggle).toBeTruthy()

    await act(async () => {
      toggle?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
      await Promise.resolve()
    })

    expect(sseMocks.onMessage).toBeTruthy()

    await act(async () => {
      sseMocks.onMessage?.({
        type: 'pool_attempts',
        invokeId: 'invocation-pool-attempts-stale-fetch',
        attempts: [
          {
            id: 10,
            invokeId: 'invocation-pool-attempts-stale-fetch',
            occurredAt: '2026-03-07T03:13:51Z',
            endpoint: '/v1/responses',
            upstreamAccountId: 7,
            upstreamAccountName: 'pool-account-a',
            attemptIndex: 1,
            distinctAccountIndex: 1,
            sameAccountRetryIndex: 1,
            startedAt: '2026-03-07T03:13:51Z',
            finishedAt: '2026-03-07T03:13:52Z',
            status: 'success',
            httpStatus: 200,
            connectLatencyMs: 30,
            firstByteLatencyMs: 12,
            streamLatencyMs: 80,
            upstreamRequestId: 'req_live_newer',
            createdAt: '2026-03-07T03:13:52Z',
          },
        ],
      })
      await Promise.resolve()
    })

    await waitForCondition(() => document.body.textContent?.includes('req_live_newer') === true)

    await act(async () => {
      deferred.resolve([
        {
          id: 10,
          invokeId: 'invocation-pool-attempts-stale-fetch',
          occurredAt: '2026-03-07T03:13:51Z',
          endpoint: '/v1/responses',
          upstreamAccountId: 7,
          upstreamAccountName: 'pool-account-a',
          attemptIndex: 1,
          distinctAccountIndex: 1,
          sameAccountRetryIndex: 1,
          startedAt: '2026-03-07T03:13:51Z',
          finishedAt: null,
          status: 'pending',
          httpStatus: null,
          connectLatencyMs: null,
          firstByteLatencyMs: null,
          streamLatencyMs: null,
          upstreamRequestId: null,
          createdAt: '2026-03-07T03:13:51Z',
        },
      ])
      await Promise.resolve()
    })

    expect(document.body.textContent).toContain('成功')
    expect(document.body.textContent).toContain('req_live_newer')
    expect(document.body.textContent).not.toContain('进行中')
  })

  it('accepts a newer fetch result after an earlier pending SSE snapshot', async () => {
    const deferred = createDeferredPromise<
      Array<{
        id: number
        invokeId: string
        occurredAt: string
        endpoint: string
        upstreamAccountId: number
        upstreamAccountName: string
        attemptIndex: number
        distinctAccountIndex: number
        sameAccountRetryIndex: number
        startedAt: string
        finishedAt: string | null
        status: string
        httpStatus: number | null
        connectLatencyMs: number | null
        firstByteLatencyMs: number | null
        streamLatencyMs: number | null
        upstreamRequestId: string | null
        createdAt: string
      }>
    >()
    apiMocks.fetchInvocationPoolAttempts.mockReturnValue(deferred.promise)

    await renderInteractiveTable([
      {
        id: 44,
        invokeId: 'invocation-pool-attempts-newer-fetch',
        occurredAt: '2026-03-07T03:13:51Z',
        createdAt: '2026-03-07T03:13:51Z',
        source: 'proxy',
        routeMode: 'pool',
        upstreamAccountId: 7,
        upstreamAccountName: 'pool-account-a',
        endpoint: '/v1/responses',
        model: 'gpt-5.4',
        status: 'running',
        poolAttemptCount: 1,
      },
    ])

    const toggle = Array.from(document.querySelectorAll('button')).find(
      (button) => button.getAttribute('aria-expanded') === 'false',
    )
    expect(toggle).toBeTruthy()

    await act(async () => {
      toggle?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
      await Promise.resolve()
    })

    await act(async () => {
      sseMocks.onMessage?.({
        type: 'pool_attempts',
        invokeId: 'invocation-pool-attempts-newer-fetch',
        attempts: [
          {
            id: 13,
            invokeId: 'invocation-pool-attempts-newer-fetch',
            occurredAt: '2026-03-07T03:13:51Z',
            endpoint: '/v1/responses',
            upstreamAccountId: 7,
            upstreamAccountName: 'pool-account-a',
            attemptIndex: 1,
            distinctAccountIndex: 1,
            sameAccountRetryIndex: 1,
            startedAt: '2026-03-07T03:13:51Z',
            finishedAt: null,
            status: 'pending',
            httpStatus: null,
            connectLatencyMs: null,
            firstByteLatencyMs: null,
            streamLatencyMs: null,
            upstreamRequestId: null,
            createdAt: '2026-03-07T03:13:51Z',
          },
        ],
      })
      await Promise.resolve()
    })

    await waitForCondition(() => document.body.textContent?.includes('进行中') === true)

    await act(async () => {
      deferred.resolve([
        {
          id: 13,
          invokeId: 'invocation-pool-attempts-newer-fetch',
          occurredAt: '2026-03-07T03:13:51Z',
          endpoint: '/v1/responses',
          upstreamAccountId: 7,
          upstreamAccountName: 'pool-account-a',
          attemptIndex: 1,
          distinctAccountIndex: 1,
          sameAccountRetryIndex: 1,
          startedAt: '2026-03-07T03:13:51Z',
          finishedAt: '2026-03-07T03:13:52Z',
          status: 'success',
          httpStatus: 200,
          connectLatencyMs: 31,
          firstByteLatencyMs: 10,
          streamLatencyMs: 55,
          upstreamRequestId: 'req_fetch_newer',
          createdAt: '2026-03-07T03:13:52Z',
        },
      ])
      await Promise.resolve()
    })

    await waitForCondition(() => document.body.textContent?.includes('req_fetch_newer') === true)
    expect(document.body.textContent).toContain('成功')
    expect(document.body.textContent).not.toContain('进行中')
  })

  it('refetches expanded pool attempts when the invocation turns terminal with pending cached attempts', async () => {
    apiMocks.fetchInvocationPoolAttempts
      .mockResolvedValueOnce([
        {
          id: 14,
          invokeId: 'invocation-pool-attempts-terminal-refresh',
          occurredAt: '2026-03-07T03:13:51Z',
          endpoint: '/v1/responses',
          upstreamAccountId: 7,
          upstreamAccountName: 'pool-account-a',
          attemptIndex: 1,
          distinctAccountIndex: 1,
          sameAccountRetryIndex: 1,
          startedAt: '2026-03-07T03:13:51Z',
          finishedAt: null,
          status: 'pending',
          httpStatus: null,
          connectLatencyMs: null,
          firstByteLatencyMs: null,
          streamLatencyMs: null,
          upstreamRequestId: null,
          createdAt: '2026-03-07T03:13:51Z',
        },
      ])
      .mockResolvedValueOnce([
        {
          id: 14,
          invokeId: 'invocation-pool-attempts-terminal-refresh',
          occurredAt: '2026-03-07T03:13:51Z',
          endpoint: '/v1/responses',
          upstreamAccountId: 7,
          upstreamAccountName: 'pool-account-a',
          attemptIndex: 1,
          distinctAccountIndex: 1,
          sameAccountRetryIndex: 1,
          startedAt: '2026-03-07T03:13:51Z',
          finishedAt: '2026-03-07T03:13:52Z',
          status: 'success',
          httpStatus: 200,
          connectLatencyMs: 28,
          firstByteLatencyMs: 9,
          streamLatencyMs: 61,
          upstreamRequestId: 'req_terminal_refresh',
          createdAt: '2026-03-07T03:13:52Z',
        },
      ])

    await renderInteractiveTable([
      {
        id: 45,
        invokeId: 'invocation-pool-attempts-terminal-refresh',
        occurredAt: '2026-03-07T03:13:51Z',
        createdAt: '2026-03-07T03:13:51Z',
        source: 'proxy',
        routeMode: 'pool',
        upstreamAccountId: 7,
        upstreamAccountName: 'pool-account-a',
        endpoint: '/v1/responses',
        model: 'gpt-5.4',
        status: 'running',
        poolAttemptCount: 1,
      },
    ])

    const toggle = Array.from(document.querySelectorAll('button')).find(
      (button) => button.getAttribute('aria-expanded') === 'false',
    )
    expect(toggle).toBeTruthy()

    await act(async () => {
      toggle?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
      await Promise.resolve()
    })

    await waitForCondition(() => document.body.textContent?.includes('进行中') === true)
    expect(apiMocks.fetchInvocationPoolAttempts).toHaveBeenNthCalledWith(
      1,
      'invocation-pool-attempts-terminal-refresh',
    )

    await renderInteractiveTable([
      {
        id: 45,
        invokeId: 'invocation-pool-attempts-terminal-refresh',
        occurredAt: '2026-03-07T03:13:51Z',
        createdAt: '2026-03-07T03:13:51Z',
        source: 'proxy',
        routeMode: 'pool',
        upstreamAccountId: 7,
        upstreamAccountName: 'pool-account-a',
        endpoint: '/v1/responses',
        model: 'gpt-5.4',
        status: 'success',
        poolAttemptCount: 1,
      },
    ])

    await waitForCondition(
      () => apiMocks.fetchInvocationPoolAttempts.mock.calls.length >= 2,
    )
    expect(apiMocks.fetchInvocationPoolAttempts).toHaveBeenNthCalledWith(
      2,
      'invocation-pool-attempts-terminal-refresh',
    )
    await waitForCondition(() => document.body.textContent?.includes('req_terminal_refresh') === true)
    expect(document.body.textContent).toContain('成功')
    expect(document.body.textContent).not.toContain('进行中')
  })

  it('ignores an older pending pool-attempt SSE snapshot after a newer terminal snapshot', async () => {
    apiMocks.fetchInvocationPoolAttempts.mockResolvedValue([])

    await renderInteractiveTable([
      {
        id: 46,
        invokeId: 'invocation-pool-attempts-sse-ordering',
        occurredAt: '2026-03-07T03:13:51Z',
        createdAt: '2026-03-07T03:13:51Z',
        source: 'proxy',
        routeMode: 'pool',
        upstreamAccountId: 7,
        upstreamAccountName: 'pool-account-a',
        endpoint: '/v1/responses',
        model: 'gpt-5.4',
        status: 'running',
        poolAttemptCount: 1,
      },
    ])

    const toggle = Array.from(document.querySelectorAll('button')).find(
      (button) => button.getAttribute('aria-expanded') === 'false',
    )
    expect(toggle).toBeTruthy()

    await act(async () => {
      toggle?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
      await Promise.resolve()
    })

    await act(async () => {
      sseMocks.onMessage?.({
        type: 'pool_attempts',
        invokeId: 'invocation-pool-attempts-sse-ordering',
        attempts: [
          {
            id: 15,
            invokeId: 'invocation-pool-attempts-sse-ordering',
            occurredAt: '2026-03-07T03:13:51Z',
            endpoint: '/v1/responses',
            upstreamAccountId: 7,
            upstreamAccountName: 'pool-account-a',
            attemptIndex: 1,
            distinctAccountIndex: 1,
            sameAccountRetryIndex: 1,
            startedAt: '2026-03-07T03:13:51Z',
            finishedAt: '2026-03-07T03:13:52Z',
            status: 'success',
            httpStatus: 200,
            connectLatencyMs: 28,
            firstByteLatencyMs: 9,
            streamLatencyMs: 61,
            upstreamRequestId: 'req_sse_terminal',
            createdAt: '2026-03-07T03:13:52Z',
          },
        ],
      })
      await Promise.resolve()
    })

    await waitForCondition(() => document.body.textContent?.includes('req_sse_terminal') === true)

    await act(async () => {
      sseMocks.onMessage?.({
        type: 'pool_attempts',
        invokeId: 'invocation-pool-attempts-sse-ordering',
        attempts: [
          {
            id: 15,
            invokeId: 'invocation-pool-attempts-sse-ordering',
            occurredAt: '2026-03-07T03:13:51Z',
            endpoint: '/v1/responses',
            upstreamAccountId: 7,
            upstreamAccountName: 'pool-account-a',
            attemptIndex: 1,
            distinctAccountIndex: 1,
            sameAccountRetryIndex: 1,
            startedAt: '2026-03-07T03:13:51Z',
            finishedAt: null,
            status: 'pending',
            httpStatus: null,
            connectLatencyMs: null,
            firstByteLatencyMs: null,
            streamLatencyMs: null,
            upstreamRequestId: null,
            createdAt: '2026-03-07T03:13:51Z',
          },
        ],
      })
      await Promise.resolve()
    })

    expect(document.body.textContent).toContain('成功')
    expect(document.body.textContent).toContain('req_sse_terminal')
    expect(document.body.textContent).not.toContain('进行中')
  })

  it('keeps pool-attempt SSE payloads fresh for invocations that are not expanded', async () => {
    apiMocks.fetchInvocationPoolAttempts
      .mockResolvedValueOnce([
        {
          id: 11,
          invokeId: 'invocation-expanded-a',
          occurredAt: '2026-03-07T03:13:51Z',
          endpoint: '/v1/responses',
          upstreamAccountId: 7,
          upstreamAccountName: 'pool-account-a',
          attemptIndex: 1,
          distinctAccountIndex: 1,
          sameAccountRetryIndex: 1,
          startedAt: '2026-03-07T03:13:51Z',
          finishedAt: null,
          status: 'pending',
          httpStatus: null,
          connectLatencyMs: null,
          firstByteLatencyMs: null,
          streamLatencyMs: null,
          upstreamRequestId: null,
          createdAt: '2026-03-07T03:13:51Z',
        },
      ])
      .mockResolvedValueOnce([])

    await renderInteractiveTable([
      {
        id: 50,
        invokeId: 'invocation-expanded-a',
        occurredAt: '2026-03-07T03:13:51Z',
        createdAt: '2026-03-07T03:13:51Z',
        source: 'proxy',
        routeMode: 'pool',
        upstreamAccountId: 7,
        upstreamAccountName: 'pool-account-a',
        endpoint: '/v1/responses',
        model: 'gpt-5.4',
        status: 'running',
        poolAttemptCount: 1,
      },
      {
        id: 51,
        invokeId: 'invocation-collapsed-b',
        occurredAt: '2026-03-07T03:13:52Z',
        createdAt: '2026-03-07T03:13:52Z',
        source: 'proxy',
        routeMode: 'pool',
        upstreamAccountId: 8,
        upstreamAccountName: 'pool-account-b',
        endpoint: '/v1/responses',
        model: 'gpt-5.4',
        status: 'running',
        poolAttemptCount: 1,
      },
    ])

    const toggles = Array.from(document.querySelectorAll('button')).filter(
      (button) => button.getAttribute('aria-expanded') === 'false',
    )
    expect(toggles.length).toBeGreaterThanOrEqual(2)

    await act(async () => {
      toggles[0]?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
      await Promise.resolve()
    })

    await waitForCondition(() => document.body.textContent?.includes('pool-account-a') === true)

    await act(async () => {
      sseMocks.onMessage?.({
        type: 'pool_attempts',
        invokeId: 'invocation-collapsed-b',
        attempts: [
          {
            id: 12,
            invokeId: 'invocation-collapsed-b',
            occurredAt: '2026-03-07T03:13:52Z',
            endpoint: '/v1/responses',
            upstreamAccountId: 8,
            upstreamAccountName: 'pool-account-b',
            attemptIndex: 1,
            distinctAccountIndex: 1,
            sameAccountRetryIndex: 1,
            startedAt: '2026-03-07T03:13:52Z',
            finishedAt: '2026-03-07T03:13:53Z',
            status: 'success',
            httpStatus: 200,
            connectLatencyMs: 25,
            firstByteLatencyMs: 11,
            streamLatencyMs: 40,
            upstreamRequestId: 'req_ignored',
            createdAt: '2026-03-07T03:13:53Z',
          },
        ],
      })
      await Promise.resolve()
    })

    await act(async () => {
      toggles[1]?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
      await Promise.resolve()
    })

    expect(apiMocks.fetchInvocationPoolAttempts).toHaveBeenCalledTimes(1)
    await waitForCondition(() => document.body.textContent?.includes('req_ignored') === true)
    expect(document.body.textContent).toContain('req_ignored')
    expect(document.body.textContent).not.toContain('未找到号池尝试记录')
  })

  it('does not replace live pool attempts with a stale REST load error', async () => {
    let rejectFetch: ((error: Error) => void) | null = null
    apiMocks.fetchInvocationPoolAttempts.mockImplementationOnce(
      () =>
        new Promise((_, reject) => {
          rejectFetch = (error: Error) => reject(error)
        }),
    )

    await renderInteractiveTable([
      {
        id: 60,
        invokeId: 'invocation-fetch-error-after-sse',
        occurredAt: '2026-03-07T03:13:51Z',
        createdAt: '2026-03-07T03:13:51Z',
        source: 'proxy',
        routeMode: 'pool',
        upstreamAccountId: 7,
        upstreamAccountName: 'pool-account-a',
        endpoint: '/v1/responses',
        model: 'gpt-5.4',
        status: 'running',
        poolAttemptCount: 1,
      },
    ])

    const toggle = Array.from(document.querySelectorAll('button')).find(
      (button) => button.getAttribute('aria-expanded') === 'false',
    )
    expect(toggle).toBeTruthy()

    await act(async () => {
      toggle?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
      await Promise.resolve()
    })

    await act(async () => {
      sseMocks.onMessage?.({
        type: 'pool_attempts',
        invokeId: 'invocation-fetch-error-after-sse',
        attempts: [
          {
            id: 21,
            invokeId: 'invocation-fetch-error-after-sse',
            occurredAt: '2026-03-07T03:13:51Z',
            endpoint: '/v1/responses',
            upstreamAccountId: 7,
            upstreamAccountName: 'pool-account-a',
            attemptIndex: 1,
            distinctAccountIndex: 1,
            sameAccountRetryIndex: 1,
            startedAt: '2026-03-07T03:13:51Z',
            finishedAt: null,
            status: 'pending',
            phase: 'waiting_first_byte',
            httpStatus: null,
            connectLatencyMs: 18,
            firstByteLatencyMs: null,
            streamLatencyMs: null,
            upstreamRequestId: null,
            createdAt: '2026-03-07T03:13:51Z',
          },
        ],
      })
      await Promise.resolve()
    })

    await waitForCondition(() => document.body.textContent?.includes('等待首字节') === true)

    await act(async () => {
      rejectFetch?.(new Error('network down'))
      await Promise.resolve()
      await Promise.resolve()
    })

    expect(document.body.textContent).toContain('等待首字节')
    expect(document.querySelector('[data-testid="pool-attempts-error"]')).toBeNull()
  })

  it('re-fetches running pool attempts when the parent invocation summary changes', async () => {
    apiMocks.fetchInvocationPoolAttempts
      .mockResolvedValueOnce([
        {
          id: 31,
          invokeId: 'invocation-refetch-running-attempts',
          occurredAt: '2026-03-07T03:13:51Z',
          endpoint: '/v1/responses',
          upstreamAccountId: 7,
          upstreamAccountName: 'pool-account-a',
          attemptIndex: 1,
          distinctAccountIndex: 1,
          sameAccountRetryIndex: 1,
          startedAt: '2026-03-07T03:13:51Z',
          finishedAt: null,
          status: 'pending',
          phase: 'connecting',
          httpStatus: null,
          connectLatencyMs: null,
          firstByteLatencyMs: null,
          streamLatencyMs: null,
          upstreamRequestId: null,
          createdAt: '2026-03-07T03:13:51Z',
        },
      ])
      .mockResolvedValueOnce([
        {
          id: 31,
          invokeId: 'invocation-refetch-running-attempts',
          occurredAt: '2026-03-07T03:13:51Z',
          endpoint: '/v1/responses',
          upstreamAccountId: 7,
          upstreamAccountName: 'pool-account-a',
          attemptIndex: 1,
          distinctAccountIndex: 1,
          sameAccountRetryIndex: 1,
          startedAt: '2026-03-07T03:13:51Z',
          finishedAt: null,
          status: 'pending',
          phase: 'waiting_first_byte',
          httpStatus: null,
          connectLatencyMs: 22,
          firstByteLatencyMs: null,
          streamLatencyMs: null,
          upstreamRequestId: null,
          createdAt: '2026-03-07T03:13:52Z',
        },
      ])

    await renderInteractiveTable([
      {
        id: 70,
        invokeId: 'invocation-refetch-running-attempts',
        occurredAt: '2026-03-07T03:13:51Z',
        createdAt: '2026-03-07T03:13:51Z',
        source: 'proxy',
        routeMode: 'pool',
        upstreamAccountId: 7,
        upstreamAccountName: 'pool-account-a',
        endpoint: '/v1/responses',
        model: 'gpt-5.4',
        status: 'running',
        poolAttemptCount: 1,
      },
    ])

    const toggle = Array.from(document.querySelectorAll('button')).find(
      (button) => button.getAttribute('aria-expanded') === 'false',
    )
    expect(toggle).toBeTruthy()

    await act(async () => {
      toggle?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
      await Promise.resolve()
    })

    expect(apiMocks.fetchInvocationPoolAttempts).toHaveBeenNthCalledWith(
      1,
      'invocation-refetch-running-attempts',
    )
    await waitForCondition(() => document.body.textContent?.includes('连接中') === true)

    await renderInteractiveTable([
      {
        id: 70,
        invokeId: 'invocation-refetch-running-attempts',
        occurredAt: '2026-03-07T03:13:51Z',
        createdAt: '2026-03-07T03:13:51Z',
        source: 'proxy',
        routeMode: 'pool',
        upstreamAccountId: 7,
        upstreamAccountName: 'pool-account-a',
        endpoint: '/v1/responses',
        model: 'gpt-5.4',
        status: 'running',
        poolAttemptCount: 1,
        tUpstreamConnectMs: 22,
      },
    ])

    await waitForCondition(() => apiMocks.fetchInvocationPoolAttempts.mock.calls.length >= 2)
    expect(apiMocks.fetchInvocationPoolAttempts).toHaveBeenNthCalledWith(
      2,
      'invocation-refetch-running-attempts',
    )
    await waitForCondition(() => document.body.textContent?.includes('等待首字节') === true)
  })

  it('shows a clear non-pool empty state without fetching attempts', async () => {
    await renderInteractiveTable([
      {
        id: 41,
        invokeId: 'invocation-forward-proxy-detail',
        occurredAt: '2026-03-07T03:13:51Z',
        createdAt: '2026-03-07T03:13:51Z',
        source: 'proxy',
        routeMode: 'forward_proxy',
        proxyDisplayName: 'codex-relay-02',
        endpoint: '/v1/responses',
        model: 'gpt-5.4',
        status: 'success',
      },
    ])

    const toggle = Array.from(document.querySelectorAll('button')).find(
      (button) => button.getAttribute('aria-expanded') === 'false',
    )
    expect(toggle).toBeTruthy()

    await act(async () => {
      toggle?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
      await Promise.resolve()
    })

    expect(apiMocks.fetchInvocationPoolAttempts).not.toHaveBeenCalled()
    expect(document.body.textContent).toContain('这条请求没有使用号池路由')
  })

  it('renders recognized endpoints as badges and keeps unknown endpoints on the raw-path fallback', () => {
    const html = renderTable([
      {
        id: 21,
        invokeId: 'invocation-responses-badge',
        occurredAt: '2026-03-07T03:13:53Z',
        createdAt: '2026-03-07T03:13:53Z',
        source: 'proxy',
        proxyDisplayName: 'codex-responses-edge',
        endpoint: '/v1/responses',
        model: 'gpt-5.3-codex',
        status: 'success',
        totalTokens: 2048,
        cost: 0.0042,
      },
      {
        id: 22,
        invokeId: 'invocation-chat-badge',
        occurredAt: '2026-03-07T03:13:52Z',
        createdAt: '2026-03-07T03:13:52Z',
        source: 'proxy',
        proxyDisplayName: LONG_PROXY_NAME,
        endpoint: '/v1/chat/completions',
        model: 'gpt-5.3-codex',
        status: 'success',
        totalTokens: 1024,
        cost: 0.0021,
      },
      {
        id: 23,
        invokeId: 'invocation-compact-badge',
        occurredAt: '2026-03-07T03:13:51Z',
        createdAt: '2026-03-07T03:13:51Z',
        source: 'proxy',
        proxyDisplayName: 'codex-compact-edge',
        endpoint: '/v1/responses/compact',
        model: 'gpt-5.3-codex',
        status: 'success',
        totalTokens: 512,
        cost: 0.0011,
      },
      {
        id: 24,
        invokeId: 'invocation-raw-endpoint',
        occurredAt: '2026-03-07T03:13:50Z',
        createdAt: '2026-03-07T03:13:50Z',
        source: 'proxy',
        proxyDisplayName: 'codex-raw-edge',
        endpoint: '/v1/responses/' + 'very-long-segment-'.repeat(4),
        model: 'gpt-5.3-codex',
        status: 'success',
        totalTokens: 256,
        cost: 0.0006,
      },
    ])

    expect(html.match(/data-testid="invocation-endpoint-badge"/g)?.length ?? 0).toBe(6)
    expect(html.match(/data-testid="invocation-endpoint-path"/g)?.length ?? 0).toBe(2)
    expect(html.match(/data-endpoint-kind="responses"/g)?.length ?? 0).toBe(2)
    expect(html.match(/data-endpoint-kind="chat"/g)?.length ?? 0).toBe(2)
    expect(html.match(/data-endpoint-kind="compact"/g)?.length ?? 0).toBe(2)
    expect(html.match(/data-endpoint-kind="raw"/g)?.length ?? 0).toBe(2)
    expect(html).toContain('Responses')
    expect(html).toContain('Chat')
    expect(html).toContain('远程压缩')
    expect(html).toContain('/v1/responses/compact')
    expect(html).toContain('/v1/responses/very-long-segment-')
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
        billingServiceTier: 'priority',
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
        serviceTier: 'default',
        billingServiceTier: 'priority',
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
        billingServiceTier: 'priority',
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

    expect(html.match(/data-fast-state="effective"/g)?.length ?? 0).toBe(6)
    expect(html.match(/data-fast-state="requested_only"/g)?.length ?? 0).toBe(2)
    expect(html).toContain('Fast 模式（Priority processing）')
    expect(html).toContain('请求想要 Fast，但实际未命中 Priority processing')
  })

  it('renders expanded timing details with seconds for non-ttfb durations', async () => {
    await renderInteractiveTable([
      {
        id: 91,
        invokeId: 'invocation-expanded-timings',
        occurredAt: '2026-03-16T09:10:30Z',
        createdAt: '2026-03-16T09:10:30Z',
        source: 'proxy',
        proxyDisplayName: 'relay-expanded',
        endpoint: '/v1/responses',
        model: 'gpt-5.4',
        status: 'success',
        totalTokens: 512,
        tReqReadMs: 3450,
        tReqParseMs: 20,
        tUpstreamConnectMs: 456.7,
        tUpstreamTtfbMs: 88.8,
        tUpstreamStreamMs: 12000,
        tRespParseMs: 90,
        tPersistMs: 8,
        tTotalMs: 12345,
      },
    ])

    const trigger = Array.from(document.querySelectorAll('button')).find((button) => {
      const label = button.getAttribute('aria-label')
      return label === '展开详情' || label === 'Show details'
    })
    expect(trigger).toBeTruthy()

    await act(async () => {
      trigger?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
      await Promise.resolve()
    })

    const text = document.body.textContent ?? ''
    expect(text).toContain('首字总耗时')
    expect(text).toContain('4.02 s')
    expect(text).toContain('3.45 s')
    expect(text).toContain('0.02 s')
    expect(text).toContain('0.457 s')
    expect(text).toContain('88.8 ms')
    expect(text).toContain('12 s')
    expect(text).toContain('12.35 s')
  })

  it('shows a non-zero first-response-byte total when upstream first-byte stage is 0 ms', async () => {
    await renderInteractiveTable([
      {
        id: 92,
        invokeId: 'invocation-zero-upstream-first-byte',
        occurredAt: '2026-03-23T12:53:05Z',
        createdAt: '2026-03-23T12:53:05Z',
        source: 'proxy',
        proxyDisplayName: 'relay-zero-ttfb',
        endpoint: '/v1/responses',
        model: 'gpt-5.4',
        status: 'success',
        tReqReadMs: 31,
        tReqParseMs: 1,
        tUpstreamConnectMs: 9330,
        tUpstreamTtfbMs: 0,
        tUpstreamStreamMs: 10080,
        tRespParseMs: 1,
        tPersistMs: 1,
        tTotalMs: 19460,
      },
    ])

    expect(document.body.textContent).toContain('首字总 9.36 s')

    const trigger = Array.from(document.querySelectorAll('button')).find((button) => {
      const label = button.getAttribute('aria-label')
      return label === '展开详情' || label === 'Show details'
    })
    expect(trigger).toBeTruthy()

    await act(async () => {
      trigger?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
      await Promise.resolve()
    })

    const text = document.body.textContent ?? ''
    expect(text).toContain('首字总耗时')
    expect(text).toContain('9.36 s')
    expect(text).toContain('上游首字节')
    expect(text).toContain('0.0 ms')
  })

  it('ticks running elapsed time on the client while leaving first-byte data empty', async () => {
    vi.useFakeTimers()
    vi.setSystemTime(new Date('2026-03-16T09:10:35Z'))

    await renderInteractiveTable([
      {
        id: -91,
        invokeId: 'invocation-running-elapsed',
        occurredAt: '2026-03-16T09:10:30Z',
        createdAt: '2026-03-16T09:10:30Z',
        source: 'proxy',
        proxyDisplayName: 'relay-running',
        endpoint: '/v1/responses',
        model: 'gpt-5.4',
        status: 'running',
      },
    ])

    expect(document.body.textContent).toContain('用时')
    expect(document.body.textContent).toContain('5 s')
    expect(document.body.textContent).toContain('— · —')

    await act(async () => {
      vi.advanceTimersByTime(1000)
      await Promise.resolve()
    })

    expect(document.body.textContent).toContain('6 s')
    vi.useRealTimers()
  })

  it('forwards pool account clicks to the shared upstream account controller', async () => {
    const onOpenUpstreamAccount = vi.fn()

    await renderInteractiveTable(
      [
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
      ],
      { onOpenUpstreamAccount },
    )

    const trigger = Array.from(document.querySelectorAll('button')).find((button) =>
      button.textContent?.includes('Pool Alpha'),
    )
    expect(trigger).toBeTruthy()

    await act(async () => {
      trigger?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
      await Promise.resolve()
    })

    expect(onOpenUpstreamAccount).toHaveBeenCalledWith(42, 'Pool Alpha')
  })

  it('does not render a local account detail drawer after clicking a pool account name', async () => {
    await renderInteractiveTable([
      {
        id: 41,
        invokeId: 'pool-no-local-drawer',
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
      },
    ])

    const trigger = Array.from(document.querySelectorAll('button')).find((button) =>
      button.textContent?.includes('Pool Alpha'),
    )
    expect(trigger).toBeTruthy()

    await act(async () => {
      trigger?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
      await Promise.resolve()
    })

    expect(document.body.querySelector('[role="dialog"]')).toBeNull()
    expect(document.body.textContent).not.toContain('去号池查看完整详情')
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
