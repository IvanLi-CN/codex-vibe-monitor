import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { I18nProvider } from '../i18n'
import type { ForwardProxyLiveStatsResponse } from '../lib/api'
import { ForwardProxyLiveTable } from './ForwardProxyLiveTable'

function renderTable(stats: ForwardProxyLiveStatsResponse) {
  return renderToStaticMarkup(
    <I18nProvider>
      <ForwardProxyLiveTable stats={stats} isLoading={false} error={null} />
    </I18nProvider>,
  )
}

function countOccurrences(content: string, target: string) {
  return content.split(target).length - 1
}

describe('ForwardProxyLiveTable', () => {
  it('renders unified request and weight chart surfaces with node-level summary text', () => {
    const stats: ForwardProxyLiveStatsResponse = {
      rangeStart: '2026-03-01T00:00:00Z',
      rangeEnd: '2026-03-02T00:00:00Z',
      bucketSeconds: 3600,
      nodes: [
        {
          key: 'proxy-a',
          source: 'manual',
          displayName: 'Proxy A',
          weight: 0.75,
          penalized: false,
          stats: {
            oneMinute: { attempts: 2, successRate: 1, avgLatencyMs: 120 },
            fifteenMinutes: { attempts: 12, successRate: 0.8, avgLatencyMs: 180 },
            oneHour: { attempts: 32, successRate: 0.72, avgLatencyMs: 260 },
            oneDay: { attempts: 240, successRate: 0.7, avgLatencyMs: 310 },
            sevenDays: { attempts: 1240, successRate: 0.68, avgLatencyMs: 350 },
          },
          last24h: [
            {
              bucketStart: '2026-03-01T00:00:00Z',
              bucketEnd: '2026-03-01T01:00:00Z',
              successCount: 6,
              failureCount: 2,
            },
            {
              bucketStart: '2026-03-01T01:00:00Z',
              bucketEnd: '2026-03-01T02:00:00Z',
              successCount: 4,
              failureCount: 0,
            },
          ],
          weight24h: [
            {
              bucketStart: '2026-03-01T00:00:00Z',
              bucketEnd: '2026-03-01T01:00:00Z',
              sampleCount: 2,
              minWeight: 0.45,
              maxWeight: 0.75,
              avgWeight: 0.62,
              lastWeight: 0.75,
            },
            {
              bucketStart: '2026-03-01T01:00:00Z',
              bucketEnd: '2026-03-01T02:00:00Z',
              sampleCount: 1,
              minWeight: 0.82,
              maxWeight: 0.82,
              avgWeight: 0.82,
              lastWeight: 0.82,
            },
          ],
        },
      ],
    }

    const html = renderTable(stats)

    expect(html).toContain('近 24 小时权重变化')
    expect(html).toContain('aria-label="Proxy A 近 24 小时请求量图"')
    expect(html).toContain('aria-label="Proxy A 近 24 小时权重趋势图"')
    expect(html).toContain('data-chart-kind="proxy-request-trend"')
    expect(html).toContain('data-chart-kind="proxy-weight-trend"')
    expect(html).not.toContain('<title>')
    expect(countOccurrences(html, '成功 10')).toBe(1)
    expect(countOccurrences(html, '失败 2')).toBe(1)
  })

  it('falls back to node weight trend when weight24h is missing', () => {
    const stats: ForwardProxyLiveStatsResponse = {
      rangeStart: '2026-03-01T00:00:00Z',
      rangeEnd: '2026-03-02T00:00:00Z',
      bucketSeconds: 3600,
      nodes: [
        {
          key: 'proxy-b',
          source: 'manual',
          displayName: 'Proxy B',
          weight: 1.2,
          penalized: false,
          stats: {
            oneMinute: { attempts: 0 },
            fifteenMinutes: { attempts: 0 },
            oneHour: { attempts: 0 },
            oneDay: { attempts: 0 },
            sevenDays: { attempts: 0 },
          },
          last24h: [
            {
              bucketStart: '2026-03-01T00:00:00Z',
              bucketEnd: '2026-03-01T01:00:00Z',
              successCount: 0,
              failureCount: 0,
            },
          ],
          weight24h: [],
        },
      ],
    }

    const html = renderTable(stats)

    expect(html).toContain('Proxy B')
    expect(html).toContain('aria-label="Proxy B 近 24 小时权重趋势图"')
    expect(html).toContain('近 24 小时请求量图')
  })

  it('shares request and weight trend scales across proxy rows', () => {
    const stats: ForwardProxyLiveStatsResponse = {
      rangeStart: '2026-03-01T00:00:00Z',
      rangeEnd: '2026-03-02T00:00:00Z',
      bucketSeconds: 3600,
      nodes: [
        {
          key: 'proxy-low',
          source: 'manual',
          displayName: 'Proxy Low',
          weight: 0.5,
          penalized: false,
          stats: {
            oneMinute: { attempts: 1, successRate: 1, avgLatencyMs: 100 },
            fifteenMinutes: { attempts: 1, successRate: 1, avgLatencyMs: 100 },
            oneHour: { attempts: 1, successRate: 1, avgLatencyMs: 100 },
            oneDay: { attempts: 1, successRate: 1, avgLatencyMs: 100 },
            sevenDays: { attempts: 1, successRate: 1, avgLatencyMs: 100 },
          },
          last24h: [
            {
              bucketStart: '2026-03-01T00:00:00Z',
              bucketEnd: '2026-03-01T01:00:00Z',
              successCount: 1,
              failureCount: 0,
            },
          ],
          weight24h: [
            {
              bucketStart: '2026-03-01T00:00:00Z',
              bucketEnd: '2026-03-01T01:00:00Z',
              sampleCount: 1,
              minWeight: 0.5,
              maxWeight: 0.5,
              avgWeight: 0.5,
              lastWeight: 0.5,
            },
          ],
        },
        {
          key: 'proxy-high',
          source: 'manual',
          displayName: 'Proxy High',
          weight: 2,
          penalized: false,
          stats: {
            oneMinute: { attempts: 4, successRate: 1, avgLatencyMs: 100 },
            fifteenMinutes: { attempts: 4, successRate: 1, avgLatencyMs: 100 },
            oneHour: { attempts: 4, successRate: 1, avgLatencyMs: 100 },
            oneDay: { attempts: 4, successRate: 1, avgLatencyMs: 100 },
            sevenDays: { attempts: 4, successRate: 1, avgLatencyMs: 100 },
          },
          last24h: [
            {
              bucketStart: '2026-03-01T00:00:00Z',
              bucketEnd: '2026-03-01T01:00:00Z',
              successCount: 4,
              failureCount: 0,
            },
          ],
          weight24h: [
            {
              bucketStart: '2026-03-01T00:00:00Z',
              bucketEnd: '2026-03-01T01:00:00Z',
              sampleCount: 1,
              minWeight: 2,
              maxWeight: 2,
              avgWeight: 2,
              lastWeight: 2,
            },
          ],
        },
      ],
    }

    const html = renderTable(stats)

    expect(html).toContain('Proxy Low')
    expect(html).toContain('Proxy High')
    expect(html).toContain('style="height:10px"')
    expect(html).toContain('data-chart-kind="proxy-weight-trend"')
  })

  it('keeps tiny non-zero request buckets visible on a shared scale', () => {
    const stats: ForwardProxyLiveStatsResponse = {
      rangeStart: '2026-03-01T00:00:00Z',
      rangeEnd: '2026-03-02T00:00:00Z',
      bucketSeconds: 3600,
      nodes: [
        {
          key: 'proxy-tiny',
          source: 'manual',
          displayName: 'Proxy Tiny',
          weight: 0.8,
          penalized: false,
          stats: {
            oneMinute: { attempts: 1, successRate: 1, avgLatencyMs: 100 },
            fifteenMinutes: { attempts: 1, successRate: 1, avgLatencyMs: 100 },
            oneHour: { attempts: 1, successRate: 1, avgLatencyMs: 100 },
            oneDay: { attempts: 1, successRate: 1, avgLatencyMs: 100 },
            sevenDays: { attempts: 1, successRate: 1, avgLatencyMs: 100 },
          },
          last24h: [
            {
              bucketStart: '2026-03-01T00:00:00Z',
              bucketEnd: '2026-03-01T01:00:00Z',
              successCount: 1,
              failureCount: 0,
            },
          ],
          weight24h: [
            {
              bucketStart: '2026-03-01T00:00:00Z',
              bucketEnd: '2026-03-01T01:00:00Z',
              sampleCount: 1,
              minWeight: 0.8,
              maxWeight: 0.8,
              avgWeight: 0.8,
              lastWeight: 0.8,
            },
          ],
        },
        {
          key: 'proxy-huge',
          source: 'manual',
          displayName: 'Proxy Huge',
          weight: 1.2,
          penalized: false,
          stats: {
            oneMinute: { attempts: 1000, successRate: 1, avgLatencyMs: 100 },
            fifteenMinutes: { attempts: 1000, successRate: 1, avgLatencyMs: 100 },
            oneHour: { attempts: 1000, successRate: 1, avgLatencyMs: 100 },
            oneDay: { attempts: 1000, successRate: 1, avgLatencyMs: 100 },
            sevenDays: { attempts: 1000, successRate: 1, avgLatencyMs: 100 },
          },
          last24h: [
            {
              bucketStart: '2026-03-01T00:00:00Z',
              bucketEnd: '2026-03-01T01:00:00Z',
              successCount: 1000,
              failureCount: 0,
            },
          ],
          weight24h: [
            {
              bucketStart: '2026-03-01T00:00:00Z',
              bucketEnd: '2026-03-01T01:00:00Z',
              sampleCount: 1,
              minWeight: 1.2,
              maxWeight: 1.2,
              avgWeight: 1.2,
              lastWeight: 1.2,
            },
          ],
        },
      ],
    }

    const html = renderTable(stats)

    expect(html).toContain('style="height:1px"')
  })

  it('ignores synthetic fallback weights when real history exists on the page', () => {
    const stats: ForwardProxyLiveStatsResponse = {
      rangeStart: '2026-03-01T00:00:00Z',
      rangeEnd: '2026-03-02T00:00:00Z',
      bucketSeconds: 3600,
      nodes: [
        {
          key: 'proxy-flat',
          source: 'manual',
          displayName: 'Proxy Flat',
          weight: 0.9,
          penalized: false,
          stats: {
            oneMinute: { attempts: 1, successRate: 1, avgLatencyMs: 100 },
            fifteenMinutes: { attempts: 1, successRate: 1, avgLatencyMs: 100 },
            oneHour: { attempts: 1, successRate: 1, avgLatencyMs: 100 },
            oneDay: { attempts: 1, successRate: 1, avgLatencyMs: 100 },
            sevenDays: { attempts: 1, successRate: 1, avgLatencyMs: 100 },
          },
          last24h: [
            {
              bucketStart: '2026-03-01T00:00:00Z',
              bucketEnd: '2026-03-01T01:00:00Z',
              successCount: 1,
              failureCount: 0,
            },
          ],
          weight24h: [],
        },
        {
          key: 'proxy-real',
          source: 'manual',
          displayName: 'Proxy Real',
          weight: 0.9,
          penalized: false,
          stats: {
            oneMinute: { attempts: 1, successRate: 1, avgLatencyMs: 100 },
            fifteenMinutes: { attempts: 1, successRate: 1, avgLatencyMs: 100 },
            oneHour: { attempts: 1, successRate: 1, avgLatencyMs: 100 },
            oneDay: { attempts: 1, successRate: 1, avgLatencyMs: 100 },
            sevenDays: { attempts: 1, successRate: 1, avgLatencyMs: 100 },
          },
          last24h: [
            {
              bucketStart: '2026-03-01T00:00:00Z',
              bucketEnd: '2026-03-01T01:00:00Z',
              successCount: 1,
              failureCount: 0,
            },
          ],
          weight24h: [
            {
              bucketStart: '2026-03-01T00:00:00Z',
              bucketEnd: '2026-03-01T01:00:00Z',
              sampleCount: 1,
              minWeight: -0.4,
              maxWeight: 1.4,
              avgWeight: 0.5,
              lastWeight: 1.4,
            },
          ],
        },
      ],
    }

    const html = renderTable(stats)

    expect(html).toContain('Proxy Flat')
    expect(html).toContain('Proxy Real')
    expect(html).toContain('M 108.00 3.64')
  })

  it('uses different colors for positive and negative weight regions', () => {
    const stats: ForwardProxyLiveStatsResponse = {
      rangeStart: '2026-03-01T00:00:00Z',
      rangeEnd: '2026-03-02T00:00:00Z',
      bucketSeconds: 3600,
      nodes: [
        {
          key: 'proxy-mixed',
          source: 'manual',
          displayName: 'Proxy Mixed',
          weight: 0.5,
          penalized: false,
          stats: {
            oneMinute: { attempts: 1, successRate: 1, avgLatencyMs: 100 },
            fifteenMinutes: { attempts: 1, successRate: 1, avgLatencyMs: 100 },
            oneHour: { attempts: 1, successRate: 1, avgLatencyMs: 100 },
            oneDay: { attempts: 1, successRate: 1, avgLatencyMs: 100 },
            sevenDays: { attempts: 1, successRate: 1, avgLatencyMs: 100 },
          },
          last24h: [
            {
              bucketStart: '2026-03-01T00:00:00Z',
              bucketEnd: '2026-03-01T01:00:00Z',
              successCount: 1,
              failureCount: 0,
            },
            {
              bucketStart: '2026-03-01T01:00:00Z',
              bucketEnd: '2026-03-01T02:00:00Z',
              successCount: 1,
              failureCount: 0,
            },
          ],
          weight24h: [
            {
              bucketStart: '2026-03-01T00:00:00Z',
              bucketEnd: '2026-03-01T01:00:00Z',
              sampleCount: 1,
              minWeight: -0.4,
              maxWeight: -0.1,
              avgWeight: -0.2,
              lastWeight: -0.1,
            },
            {
              bucketStart: '2026-03-01T01:00:00Z',
              bucketEnd: '2026-03-01T02:00:00Z',
              sampleCount: 1,
              minWeight: 0.2,
              maxWeight: 0.6,
              avgWeight: 0.4,
              lastWeight: 0.5,
            },
          ],
        },
      ],
    }

    const html = renderTable(stats)

    expect(html).toContain('fill="oklch(var(--color-success) / 0.18)"')
    expect(html).toContain('fill="oklch(var(--color-error) / 0.16)"')
  })
})
