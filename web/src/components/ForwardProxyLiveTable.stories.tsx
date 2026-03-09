import type { Meta, StoryObj } from '@storybook/react-vite'
import { I18nProvider } from '../i18n'
import type { ForwardProxyHourlyBucket, ForwardProxyLiveStatsResponse, ForwardProxyWeightBucket } from '../lib/api'
import { ForwardProxyLiveTable } from './ForwardProxyLiveTable'

function buildRequestBuckets(seed: number): ForwardProxyHourlyBucket[] {
  const start = Date.parse('2026-03-01T00:00:00.000Z')
  return Array.from({ length: 24 }, (_, index) => {
    const bucketStart = new Date(start + index * 3600_000).toISOString()
    const bucketEnd = new Date(start + (index + 1) * 3600_000).toISOString()
    const successCount = Math.max(0, Math.round(10 + Math.sin((index + seed) / 2) * 7))
    const failureCount = Math.max(0, Math.round(2 + Math.cos((index + seed) / 3) * 3))
    return {
      bucketStart,
      bucketEnd,
      successCount,
      failureCount,
    }
  })
}

function buildWeightBuckets(seed: number, base: number): ForwardProxyWeightBucket[] {
  const start = Date.parse('2026-03-01T00:00:00.000Z')
  return Array.from({ length: 24 }, (_, index) => {
    const bucketStart = new Date(start + index * 3600_000).toISOString()
    const bucketEnd = new Date(start + (index + 1) * 3600_000).toISOString()
    const drift = Math.sin((index + seed) / 4) * 0.18 + Math.cos((index + seed) / 7) * 0.06
    const lastWeight = Number((base + drift).toFixed(3))
    const minWeight = Number((lastWeight - 0.08).toFixed(3))
    const maxWeight = Number((lastWeight + 0.07).toFixed(3))
    const avgWeight = Number(((minWeight + maxWeight + lastWeight) / 3).toFixed(3))
    return {
      bucketStart,
      bucketEnd,
      sampleCount: index % 4 === 0 ? 0 : 2 + (index % 3),
      minWeight,
      maxWeight,
      avgWeight,
      lastWeight,
    }
  })
}

const stats: ForwardProxyLiveStatsResponse = {
  rangeStart: '2026-03-01T00:00:00.000Z',
  rangeEnd: '2026-03-02T00:00:00.000Z',
  bucketSeconds: 3600,
  nodes: [
    {
      key: 'manual-sg-01',
      source: 'manual',
      displayName: 'sg-relay-edge-01',
      endpointUrl: 'socks5://203.0.113.41:1080',
      weight: 0.86,
      penalized: false,
      stats: {
        oneMinute: { attempts: 7, successRate: 0.86, avgLatencyMs: 163 },
        fifteenMinutes: { attempts: 48, successRate: 0.9, avgLatencyMs: 182 },
        oneHour: { attempts: 190, successRate: 0.91, avgLatencyMs: 195 },
        oneDay: { attempts: 3420, successRate: 0.89, avgLatencyMs: 224 },
        sevenDays: { attempts: 21142, successRate: 0.9, avgLatencyMs: 241 },
      },
      last24h: buildRequestBuckets(1),
      weight24h: buildWeightBuckets(1, 0.82),
    },
    {
      key: 'sub-tokyo-02',
      source: 'subscription',
      displayName: 'tokyo-relay-02',
      endpointUrl: 'vless://example-uuid@tokyo.example.com:443#tokyo-02',
      weight: -0.12,
      penalized: true,
      stats: {
        oneMinute: { attempts: 6, successRate: 0.67, avgLatencyMs: 420 },
        fifteenMinutes: { attempts: 39, successRate: 0.71, avgLatencyMs: 390 },
        oneHour: { attempts: 136, successRate: 0.74, avgLatencyMs: 356 },
        oneDay: { attempts: 2795, successRate: 0.78, avgLatencyMs: 310 },
        sevenDays: { attempts: 15420, successRate: 0.8, avgLatencyMs: 305 },
      },
      last24h: buildRequestBuckets(9),
      weight24h: buildWeightBuckets(9, -0.08),
    },
    {
      key: '__direct__',
      source: 'direct',
      displayName: 'Direct',
      weight: 1.03,
      penalized: false,
      stats: {
        oneMinute: { attempts: 2, successRate: 1, avgLatencyMs: 129 },
        fifteenMinutes: { attempts: 18, successRate: 1, avgLatencyMs: 136 },
        oneHour: { attempts: 88, successRate: 1, avgLatencyMs: 149 },
        oneDay: { attempts: 1510, successRate: 0.99, avgLatencyMs: 170 },
        sevenDays: { attempts: 12002, successRate: 0.99, avgLatencyMs: 182 },
      },
      last24h: buildRequestBuckets(15),
      weight24h: buildWeightBuckets(15, 1.01),
    },
  ],
}



const sharedScaleStats: ForwardProxyLiveStatsResponse = {
  rangeStart: '2026-03-01T00:00:00.000Z',
  rangeEnd: '2026-03-02T00:00:00.000Z',
  bucketSeconds: 3600,
  nodes: [
    {
      key: 'tiny-traffic',
      source: 'manual',
      displayName: 'tiny-traffic',
      weight: 0.4,
      penalized: false,
      stats: {
        oneMinute: { attempts: 1, successRate: 1, avgLatencyMs: 120 },
        fifteenMinutes: { attempts: 4, successRate: 1, avgLatencyMs: 130 },
        oneHour: { attempts: 8, successRate: 1, avgLatencyMs: 140 },
        oneDay: { attempts: 32, successRate: 0.97, avgLatencyMs: 150 },
        sevenDays: { attempts: 180, successRate: 0.96, avgLatencyMs: 160 },
      },
      last24h: [
        { bucketStart: '2026-03-01T00:00:00.000Z', bucketEnd: '2026-03-01T01:00:00.000Z', successCount: 2, failureCount: 0 },
        { bucketStart: '2026-03-01T01:00:00.000Z', bucketEnd: '2026-03-01T02:00:00.000Z', successCount: 1, failureCount: 0 },
        ...Array.from({ length: 22 }, (_, index) => ({
          bucketStart: new Date(Date.parse('2026-03-01T02:00:00.000Z') + index * 3600_000).toISOString(),
          bucketEnd: new Date(Date.parse('2026-03-01T03:00:00.000Z') + index * 3600_000).toISOString(),
          successCount: 0,
          failureCount: 0,
        })),
      ],
      weight24h: Array.from({ length: 24 }, (_, index) => ({
        bucketStart: new Date(Date.parse('2026-03-01T00:00:00.000Z') + index * 3600_000).toISOString(),
        bucketEnd: new Date(Date.parse('2026-03-01T01:00:00.000Z') + index * 3600_000).toISOString(),
        sampleCount: 1,
        minWeight: 0.35,
        maxWeight: 0.45,
        avgWeight: 0.4,
        lastWeight: 0.4 + (index % 3 === 0 ? 0.02 : 0),
      })),
    },
    {
      key: 'burst-traffic',
      source: 'manual',
      displayName: 'burst-traffic',
      weight: 2.4,
      penalized: false,
      stats: {
        oneMinute: { attempts: 18, successRate: 0.94, avgLatencyMs: 210 },
        fifteenMinutes: { attempts: 120, successRate: 0.93, avgLatencyMs: 220 },
        oneHour: { attempts: 480, successRate: 0.92, avgLatencyMs: 240 },
        oneDay: { attempts: 4800, successRate: 0.91, avgLatencyMs: 260 },
        sevenDays: { attempts: 32000, successRate: 0.9, avgLatencyMs: 280 },
      },
      last24h: [
        { bucketStart: '2026-03-01T00:00:00.000Z', bucketEnd: '2026-03-01T01:00:00.000Z', successCount: 22, failureCount: 1 },
        { bucketStart: '2026-03-01T01:00:00.000Z', bucketEnd: '2026-03-01T02:00:00.000Z', successCount: 18, failureCount: 2 },
        { bucketStart: '2026-03-01T02:00:00.000Z', bucketEnd: '2026-03-01T03:00:00.000Z', successCount: 30, failureCount: 4 },
        ...Array.from({ length: 21 }, (_, index) => ({
          bucketStart: new Date(Date.parse('2026-03-01T03:00:00.000Z') + index * 3600_000).toISOString(),
          bucketEnd: new Date(Date.parse('2026-03-01T04:00:00.000Z') + index * 3600_000).toISOString(),
          successCount: index % 4 === 0 ? 12 : 4,
          failureCount: index % 6 === 0 ? 2 : 0,
        })),
      ],
      weight24h: Array.from({ length: 24 }, (_, index) => ({
        bucketStart: new Date(Date.parse('2026-03-01T00:00:00.000Z') + index * 3600_000).toISOString(),
        bucketEnd: new Date(Date.parse('2026-03-01T01:00:00.000Z') + index * 3600_000).toISOString(),
        sampleCount: 3,
        minWeight: 1.8,
        maxWeight: 2.4,
        avgWeight: 2.1,
        lastWeight: 2.4 - (index % 5 === 0 ? 0.3 : 0.1),
      })),
    },
  ],
}

const meta = {
  title: 'Monitoring/ForwardProxyLiveTable',
  component: ForwardProxyLiveTable,
  parameters: {
    layout: 'fullscreen',
  },
  decorators: [
    (Story) => (
      <I18nProvider>
        <div data-theme="light" className="min-h-screen bg-base-200 px-4 py-6 text-base-content sm:px-6">
          <main className="mx-auto w-full max-w-[1200px] space-y-4">
            <h2 className="text-xl font-semibold">代理运行态</h2>
            <Story />
          </main>
        </div>
      </I18nProvider>
    ),
  ],
} satisfies Meta<typeof ForwardProxyLiveTable>

export default meta

type Story = StoryObj<typeof meta>

export const Populated: Story = {
  args: {
    stats,
    isLoading: false,
    error: null,
  },
}

export const Empty: Story = {
  args: {
    stats: {
      rangeStart: stats.rangeStart,
      rangeEnd: stats.rangeEnd,
      bucketSeconds: stats.bucketSeconds,
      nodes: [],
    },
    isLoading: false,
    error: null,
  },
}

export const SharedScaleComparison: Story = {
  args: {
    stats: sharedScaleStats,
    isLoading: false,
    error: null,
  },
}

export const TooltipEdgeDensity: Story = {
  args: {
    stats,
    isLoading: false,
    error: null,
  },
  parameters: {
    docs: {
      description: {
        story:
          'Use the rightmost request bucket and weight point to verify tooltip flip positioning against the table edge and dense hover targets.',
      },
    },
  },
}
