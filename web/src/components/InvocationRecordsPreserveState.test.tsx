/** @vitest-environment jsdom */
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest'
import { InvocationRecordsSummaryCards } from './InvocationRecordsSummaryCards'
import { InvocationRecordsTable } from './InvocationRecordsTable'
import type { ApiInvocation, InvocationRecordsSummaryResponse } from '../lib/api'

vi.mock('../i18n', () => ({
  useTranslation: () => ({
    locale: 'zh',
    t: (key: string, params?: Record<string, string>) => {
      if (params?.error) return `${key}: ${params.error}`
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
})

function render(ui: React.ReactNode) {
  host = document.createElement('div')
  document.body.appendChild(host)
  root = createRoot(host)
  act(() => {
    root?.render(ui)
  })
}

function createSummary(): InvocationRecordsSummaryResponse {
  return {
    snapshotId: 42,
    newRecordsCount: 0,
    totalCount: 2,
    successCount: 2,
    failureCount: 0,
    totalCost: 1.25,
    totalTokens: 1234,
    token: {
      requestCount: 2,
      totalTokens: 1234,
      avgTokensPerRequest: 617,
      cacheInputTokens: 100,
      totalCost: 1.25,
    },
    network: {
      avgTtfbMs: 10,
      p95TtfbMs: 12,
      avgTotalMs: 20,
      p95TotalMs: 25,
    },
    exception: {
      failureCount: 0,
      serviceFailureCount: 0,
      clientFailureCount: 0,
      clientAbortCount: 0,
      actionableFailureCount: 0,
    },
  }
}

function createRecord(overrides: Partial<ApiInvocation> = {}): ApiInvocation {
  return {
    id: 1,
    invokeId: 'invoke-1',
    occurredAt: '2026-03-10T00:00:00Z',
    createdAt: '2026-03-10T00:00:00Z',
    status: 'success',
    model: 'gpt-4.1',
    source: 'proxy-a',
    ...overrides,
  }
}

describe('records stale-data rendering', () => {
  it('keeps summary metrics visible when a refresh error arrives', () => {
    render(<InvocationRecordsSummaryCards focus="token" summary={createSummary()} isLoading error="boom" />)

    const text = host?.textContent ?? ''
    expect(text).toContain('records.summary.loadError: boom')
    expect(text).toContain('records.summary.token.requests')
  })

  it('renders network summary totals in seconds while keeping ttfb in milliseconds', () => {
    render(<InvocationRecordsSummaryCards focus="network" summary={createSummary()} isLoading={false} />)

    const text = host?.textContent ?? ''
    expect(text).toContain('10 ms')
    expect(text).toContain('12 ms')
    expect(text).toContain('0.02 s')
    expect(text).toContain('0.025 s')
  })

  it('renders network record totals in seconds while keeping ttfb in milliseconds', () => {
    render(
      <InvocationRecordsTable
        focus="network"
        records={[createRecord({ endpoint: '/v1/responses', requesterIp: '127.0.0.1', tUpstreamTtfbMs: 118.2, tTotalMs: 910.4 })]}
        isLoading={false}
      />,
    )

    const text = host?.textContent ?? ''
    expect(text).toContain('118 ms')
    expect(text).toContain('0.91 s')
  })

  it('keeps table rows visible when a refresh error arrives', () => {
    render(<InvocationRecordsTable focus="token" records={[createRecord()]} isLoading error="boom" />)

    const text = host?.textContent ?? ''
    expect(text).toContain('records.table.loadError: boom')
    expect(text).toContain('gpt-4.1')
  })

  it('renders legacy success rows with a failure badge once failureClass marks them as failed', () => {
    render(
      <InvocationRecordsTable
        focus="token"
        records={[
          createRecord({
            status: 'success',
            failureClass: 'service_failure',
            errorMessage: '[upstream_response_failed] server_error',
          }),
        ]}
        isLoading={false}
      />,
    )

    const text = host?.textContent ?? ''
    expect(text).toContain('table.status.failed')
    expect(text).not.toContain('table.status.success')
  })

  it('falls back to the raw occurredAt string when a record timestamp is invalid', () => {
    render(
      <InvocationRecordsTable
        focus="token"
        records={[createRecord({ occurredAt: 'not-a-date' })]}
        isLoading={false}
      />,
    )

    const text = host?.textContent ?? ''
    expect(text).toContain('not-a-date')
  })
})
