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
  it('renders weight trend column and keeps only node-level success/failure summary text', () => {
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
    expect(html).toContain('aria-label="weight trend"')
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
    expect(html).toContain('aria-label="weight trend"')
    expect(html).toContain('近 24 小时请求量（成功/失败）')
  })
})
