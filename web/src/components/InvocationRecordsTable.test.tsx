/** @vitest-environment jsdom */
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest'
import { InvocationRecordsTable } from './InvocationRecordsTable'
import type { ApiInvocation, ApiPoolUpstreamRequestAttempt } from '../lib/api'

const { apiMocks } = vi.hoisted(() => ({
  apiMocks: {
    fetchInvocationPoolAttempts: vi.fn(),
    fetchInvocationRecordDetail: vi.fn(),
    fetchInvocationResponseBody: vi.fn(),
  },
}))

vi.mock('../lib/api', async () => {
  const actual = await vi.importActual<typeof import('../lib/api')>('../lib/api')
  return {
    ...actual,
    fetchInvocationPoolAttempts: apiMocks.fetchInvocationPoolAttempts,
    fetchInvocationRecordDetail: apiMocks.fetchInvocationRecordDetail,
    fetchInvocationResponseBody: apiMocks.fetchInvocationResponseBody,
  }
})

vi.mock('../i18n', () => ({
  useTranslation: () => ({
    locale: 'zh',
    t: (key: string, params?: Record<string, string | number>) => {
      if (params?.error) return `${key}: ${params.error}`
      if (params?.value) return `${key}: ${params.value}`
      return key
    },
  }),
}))

let host: HTMLDivElement | null = null
let root: Root | null = null

beforeAll(() => {
  Object.defineProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT', {
    configurable: true,
    writable: true,
    value: true,
  })
})

afterEach(() => {
  act(() => {
    root?.unmount()
  })
  host?.remove()
  host = null
  root = null
  vi.useRealTimers()
  apiMocks.fetchInvocationPoolAttempts.mockReset()
  apiMocks.fetchInvocationRecordDetail.mockReset()
  apiMocks.fetchInvocationResponseBody.mockReset()
})

function render(ui: React.ReactNode) {
  host = document.createElement('div')
  document.body.appendChild(host)
  root = createRoot(host)
  act(() => {
    root?.render(ui)
  })
}

function rerender(ui: React.ReactNode) {
  act(() => {
    root?.render(ui)
  })
}

function createRecord(overrides: Partial<ApiInvocation> = {}): ApiInvocation {
  return {
    id: 1,
    invokeId: 'invoke-1',
    occurredAt: '2026-03-10T00:00:00Z',
    createdAt: '2026-03-10T00:00:00Z',
    status: 'success',
    source: 'proxy',
    proxyDisplayName: 'jp-relay-01',
    model: 'gpt-5.4',
    endpoint: '/v1/responses',
    inputTokens: 2400,
    cacheInputTokens: 400,
    outputTokens: 320,
    reasoningTokens: 88,
    reasoningEffort: 'high',
    totalTokens: 2720,
    cost: 0.1234,
    requesterIp: '203.0.113.10',
    promptCacheKey: 'pck-test',
    requestedServiceTier: 'priority',
    serviceTier: 'priority',
    billingServiceTier: 'priority',
    responseContentEncoding: 'gzip, br',
    tReqReadMs: 12,
    tReqParseMs: 30,
    tUpstreamConnectMs: 55,
    tUpstreamTtfbMs: 142,
    tUpstreamStreamMs: 480,
    tRespParseMs: 20,
    tPersistMs: 12,
    tTotalMs: 741,
    ...overrides,
  }
}

function clickFirstToggle() {
  const button = host?.querySelector('button[aria-label="records.table.showDetails"]') as HTMLButtonElement | null
  expect(button).not.toBeNull()
  act(() => {
    button?.click()
  })
}

async function flushAsyncWork(rounds = 4) {
  await act(async () => {
    for (let index = 0; index < rounds; index += 1) {
      await Promise.resolve()
    }
    await new Promise<void>((resolve) => window.setTimeout(resolve, 0))
  })
}

async function waitFor(check: () => boolean, timeoutMs = 500) {
  const startedAt = Date.now()
  while (Date.now() - startedAt < timeoutMs) {
    await flushAsyncWork()
    if (check()) return
  }
}

describe('InvocationRecordsTable', () => {
  it('treats completed rows as success in the shared records table', () => {
    render(
      <InvocationRecordsTable
        focus="token"
        isLoading={false}
        records={[
          createRecord({
            status: 'completed',
          }),
        ]}
      />,
    )

    const text = host?.textContent ?? ''
    expect(text).toContain('table.status.success')
    expect(text).not.toContain('completed')
  })

  it('renders a richer expanded panel with summary strip and structured-only notice', () => {
    render(
      <InvocationRecordsTable
        focus="token"
        isLoading={false}
        records={[
          createRecord({
            status: 'failed',
            failureClass: 'service_failure',
            failureKind: 'upstream_stream_error',
            isActionable: true,
            errorMessage: '[upstream_stream_error] upstream reset',
            detailLevel: 'structured_only',
            detailPrunedAt: '2026-03-11T08:09:10Z',
            detailPruneReason: 'success_over_30d',
          }),
        ]}
      />,
    )

    clickFirstToggle()

    expect(host?.querySelector('[data-testid="records-detail-summary-strip"]')).not.toBeNull()
    expect(host?.querySelector('[data-testid="invocation-detail-notice"]')).not.toBeNull()

    const text = host?.textContent ?? ''
    expect(text).toContain('table.details.failureClass')
    expect(text).toContain('table.details.actionable')
    expect(text).toContain('table.details.reasoningEffort')
    expect(text).toContain('table.details.poolAttemptCount')
    expect(text).toContain('table.poolAttempts.notPool')
    expect(text).toContain('success_over_30d')
  })

  it('shows billing service tier in the expanded detail panel', () => {
    render(
      <InvocationRecordsTable
        focus="token"
        isLoading={false}
        records={[
          createRecord({
            requestedServiceTier: 'priority',
            serviceTier: 'default',
            billingServiceTier: 'priority',
          }),
        ]}
      />,
    )

    clickFirstToggle()

    const text = host?.textContent ?? ''
    expect(text).toContain('table.details.requestedServiceTier')
    expect(text).toContain('table.details.serviceTier')
    expect(text).toContain('table.details.billingServiceTier')
  })

  it('renders abnormal response previews for failed records', async () => {
    apiMocks.fetchInvocationRecordDetail.mockResolvedValue({
      id: 1,
      abnormalResponseBody: {
        available: true,
        previewText: '{"error":{"message":"upstream exploded"}}',
        hasMore: true,
      },
    })

    render(
      <InvocationRecordsTable
        focus="exception"
        isLoading={false}
        records={[
          createRecord({
            status: 'failed',
            failureClass: 'service_failure',
            errorMessage: 'upstream exploded',
          }),
        ]}
      />,
    )

    clickFirstToggle()

    await waitFor(() => host?.querySelector('[data-testid="invocation-response-body-preview"]') != null)

    expect(apiMocks.fetchInvocationRecordDetail).toHaveBeenCalledWith(1)
    expect(host?.textContent ?? '').toContain('{"error":{"message":"upstream exploded"}}')
    expect(host?.textContent ?? '').toContain('table.responseBody.previewTruncated')
  })

  it('opens the full-details drawer and loads the complete abnormal response body', async () => {
    apiMocks.fetchInvocationRecordDetail.mockResolvedValue({
      id: 1,
      abnormalResponseBody: {
        available: true,
        previewText: '{"error":{"message":"preview only"}}',
        hasMore: true,
      },
    })
    apiMocks.fetchInvocationResponseBody.mockResolvedValue({
      available: true,
      bodyText: '{"error":{"message":"preview only"},"trace":"full-body"}',
    })

    render(
      <InvocationRecordsTable
        focus="exception"
        isLoading={false}
        records={[
          createRecord({
            status: 'failed',
            failureClass: 'service_failure',
            errorMessage: 'preview only',
          }),
        ]}
      />,
    )

    clickFirstToggle()
    await waitFor(() => host?.querySelector('[data-testid="invocation-response-body-preview"]') != null)

    const button = Array.from(document.body.querySelectorAll('button')).find(
      (candidate): candidate is HTMLButtonElement =>
        candidate instanceof HTMLButtonElement && candidate.textContent === 'table.responseBody.openFullDetails',
    )
    expect(button).not.toBeNull()

    act(() => {
      button?.click()
    })

    await waitFor(() => document.body.textContent?.includes('records.table.fullDetails.title') ?? false)

    expect(apiMocks.fetchInvocationResponseBody).toHaveBeenCalledWith(1)
    expect(document.body.textContent ?? '').toContain('"trace":"full-body"')
  })

  it('lazy loads pool attempts for pool-routed records', async () => {
    apiMocks.fetchInvocationPoolAttempts.mockResolvedValue([
      {
        id: 9,
        invokeId: 'invoke-pool',
        occurredAt: '2026-03-10T00:00:00Z',
        endpoint: '/v1/responses',
        attemptIndex: 1,
        distinctAccountIndex: 1,
        sameAccountRetryIndex: 1,
        status: 'success',
        httpStatus: 200,
        createdAt: '2026-03-10T00:00:01Z',
        upstreamAccountId: 42,
        upstreamAccountName: 'pool-account-42',
        firstByteLatencyMs: 180,
      },
    ])

    render(
      <InvocationRecordsTable
        focus="network"
        isLoading={false}
        records={[
          createRecord({
            id: 2,
            invokeId: 'invoke-pool',
            routeMode: 'pool',
            upstreamAccountId: 42,
            upstreamAccountName: 'pool-account-42',
            poolAttemptCount: 1,
            poolDistinctAccountCount: 1,
          }),
        ]}
      />,
    )

    clickFirstToggle()

    await waitFor(() => host?.querySelector('[data-testid="pool-attempts-list"]') != null)

    expect(apiMocks.fetchInvocationPoolAttempts).toHaveBeenCalledWith('invoke-pool')
    expect(host?.querySelector('[data-testid="pool-attempts-list"]')).not.toBeNull()
    expect(host?.textContent ?? '').toContain('pool-account-42')
  })

  it('renders the pool attempt error state when lazy loading fails', async () => {
    apiMocks.fetchInvocationPoolAttempts.mockRejectedValue(new Error('boom'))

    render(
      <InvocationRecordsTable
        focus="exception"
        isLoading={false}
        records={[
          createRecord({
            id: 3,
            invokeId: 'invoke-pool-error',
            routeMode: 'pool',
            upstreamAccountId: 7,
            poolAttemptCount: 2,
            poolDistinctAccountCount: 1,
          }),
        ]}
      />,
    )

    clickFirstToggle()

    await waitFor(() => host?.querySelector('[data-testid="pool-attempts-error"]') != null)

    expect(host?.querySelector('[data-testid="pool-attempts-error"]')).not.toBeNull()
    expect(host?.textContent ?? '').toContain('table.poolAttempts.loadError: boom')
  })

  it('refetches pool attempts when in-flight detail fields change without counter changes', async () => {
    apiMocks.fetchInvocationPoolAttempts.mockResolvedValue([])

    const initialRecord = createRecord({
      id: 31,
      invokeId: 'invoke-pool-poll',
      routeMode: 'pool',
      status: 'running',
      upstreamAccountId: 42,
      upstreamAccountName: 'pool-account-42',
      poolAttemptCount: 1,
      poolDistinctAccountCount: 1,
      upstreamRequestId: 'req-initial',
      tUpstreamTtfbMs: 120,
    })

    render(
      <InvocationRecordsTable
        focus="network"
        isLoading={false}
        records={[initialRecord]}
      />,
    )

    clickFirstToggle()
    await flushAsyncWork()
    expect(apiMocks.fetchInvocationPoolAttempts).toHaveBeenCalledTimes(1)

    rerender(
      <InvocationRecordsTable
        focus="network"
        isLoading={false}
        records={[
          createRecord({
            ...initialRecord,
            upstreamRequestId: 'req-updated',
            tUpstreamTtfbMs: 280,
          }),
        ]}
      />,
    )

    await waitFor(() => apiMocks.fetchInvocationPoolAttempts.mock.calls.length === 2)

    expect(apiMocks.fetchInvocationPoolAttempts).toHaveBeenCalledTimes(2)
    expect(apiMocks.fetchInvocationPoolAttempts).toHaveBeenNthCalledWith(2, 'invoke-pool-poll')
  })

  it('refetches pool attempts when an expanded in-flight record changes attempt counters', async () => {
    apiMocks.fetchInvocationPoolAttempts
      .mockResolvedValueOnce([])
      .mockResolvedValueOnce([
        {
          id: 10,
          invokeId: 'invoke-pool-refresh',
          occurredAt: '2026-03-10T00:00:00Z',
          endpoint: '/v1/responses',
          attemptIndex: 2,
          distinctAccountIndex: 2,
          sameAccountRetryIndex: 1,
          status: 'transport_failure',
          createdAt: '2026-03-10T00:00:02Z',
          upstreamAccountId: 84,
          upstreamAccountName: 'pool-account-84',
        },
      ])

    const initialRecord = createRecord({
      id: 4,
      invokeId: 'invoke-pool-refresh',
      routeMode: 'pool',
      status: 'running',
      upstreamAccountId: 42,
      upstreamAccountName: 'pool-account-42',
      poolAttemptCount: 1,
      poolDistinctAccountCount: 1,
    })

    render(<InvocationRecordsTable focus="network" isLoading={false} records={[initialRecord]} />)

    clickFirstToggle()
    await waitFor(() => apiMocks.fetchInvocationPoolAttempts.mock.calls.length === 1)

    rerender(
      <InvocationRecordsTable
        focus="network"
        isLoading={false}
        records={[
          createRecord({
            ...initialRecord,
            poolAttemptCount: 2,
            poolDistinctAccountCount: 2,
          }),
        ]}
      />,
    )

    await waitFor(() => apiMocks.fetchInvocationPoolAttempts.mock.calls.length === 2)

    expect(apiMocks.fetchInvocationPoolAttempts).toHaveBeenNthCalledWith(2, 'invoke-pool-refresh')
    await waitFor(() => (host?.textContent ?? '').includes('pool-account-84'))
  })

  it('renders unknown actionable state as a fallback instead of "no"', () => {
    render(
      <InvocationRecordsTable
        focus="exception"
        isLoading={false}
        records={[
          createRecord({
            id: 6,
            status: 'failed',
            failureClass: 'service_failure',
            failureKind: 'upstream_timeout',
            isActionable: undefined,
            errorMessage: 'upstream timeout',
          }),
        ]}
      />,
    )

    const text = host?.textContent ?? ''
    expect(text).toContain('records.table.exception.actionable')
    expect(text).not.toContain('records.table.exception.actionableNo')
    expect(text).toContain('—')
  })

  it('clears cancelled pool-attempt loading so re-expanding can fetch again', async () => {
    let resolveFirstRequest!: (value: ApiPoolUpstreamRequestAttempt[]) => void

    apiMocks.fetchInvocationPoolAttempts
      .mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            resolveFirstRequest = resolve
          }),
      )
      .mockResolvedValueOnce([
        {
          id: 11,
          invokeId: 'invoke-pool-cancel',
          occurredAt: '2026-03-10T00:00:00Z',
          endpoint: '/v1/responses',
          attemptIndex: 1,
          distinctAccountIndex: 1,
          sameAccountRetryIndex: 1,
          status: 'success',
          createdAt: '2026-03-10T00:00:03Z',
          upstreamAccountId: 52,
          upstreamAccountName: 'pool-account-52',
        },
      ])

    render(
      <InvocationRecordsTable
        focus="network"
        isLoading={false}
        records={[
          createRecord({
            id: 5,
            invokeId: 'invoke-pool-cancel',
            routeMode: 'pool',
            upstreamAccountId: 52,
            upstreamAccountName: 'pool-account-52',
            poolAttemptCount: 1,
            poolDistinctAccountCount: 1,
          }),
        ]}
      />,
    )

    clickFirstToggle()
    await waitFor(() => apiMocks.fetchInvocationPoolAttempts.mock.calls.length === 1)

    const collapseButton = host?.querySelector('button[aria-label="records.table.hideDetails"]') as HTMLButtonElement | null
    expect(collapseButton).not.toBeNull()
    act(() => {
      collapseButton?.click()
    })

    clickFirstToggle()
    await waitFor(() => apiMocks.fetchInvocationPoolAttempts.mock.calls.length === 2)

    resolveFirstRequest([])
    await waitFor(() => (host?.textContent ?? '').includes('pool-account-52'))
  })
})
