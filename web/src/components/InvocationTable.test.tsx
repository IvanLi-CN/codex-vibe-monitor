import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { I18nProvider } from '../i18n'
import type { ApiInvocation } from '../lib/api'
import {
  formatProxyWeightDelta,
  formatServiceTier,
  getFastIndicatorState,
  isPriorityServiceTier,
} from '../lib/invocation'
import { InvocationTable } from './InvocationTable'
import { getReasoningEffortTone } from './invocation-table-reasoning'

function renderTable(records: ApiInvocation[]) {
  return renderToStaticMarkup(
    <I18nProvider>
      <InvocationTable records={records} isLoading={false} error={null} />
    </I18nProvider>,
  )
}

function renderExpandedTable(records: ApiInvocation[], expandedId: number) {
  return renderToStaticMarkup(
    <I18nProvider>
      <InvocationTable records={records} isLoading={false} error={null} initialExpandedId={expandedId} />
    </I18nProvider>,
  )
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

  it('keeps structured-only metadata out of summary rows while retaining it inside expanded details', () => {
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

    const expandedHtml = renderExpandedTable(records, 4)
    expect(expandedHtml).toContain('data-testid="invocation-detail-level-badge"')
    expect(expandedHtml).toContain('Structured only')
    expect(expandedHtml).toContain('精简于 2026-02-01 12:34:56Z')
    expect(expandedHtml).toContain('success_over_30d')
  })

  it('keeps legacy full-detail records out of summary rows while preserving full detail in expanded details', () => {
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

    const expandedHtml = renderExpandedTable(records, 5)
    expect(expandedHtml).toContain('data-testid="invocation-detail-level-badge"')
    expect(expandedHtml).toContain('Full')
    expect(expandedHtml).not.toContain('Structured only')
    expect(expandedHtml).toContain('legacy row still renders')
  })
})
