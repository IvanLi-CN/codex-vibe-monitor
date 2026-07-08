import type {
  ForwardProxyBindingNode,
  ForwardProxyHourlyBucket,
  ForwardProxyLiveStatsResponse,
  ForwardProxyNode,
  ForwardProxyNodeStats,
  ForwardProxyWeightBucket,
  ForwardProxyWindowStats,
} from '../../lib/api'

const RANGE_START = '2026-04-05T12:00:00.000Z'
const RANGE_START_EPOCH = Date.parse(RANGE_START)

function hourBucket(index: number) {
  return {
    bucketStart: new Date(RANGE_START_EPOCH + index * 3600_000).toISOString(),
    bucketEnd: new Date(RANGE_START_EPOCH + (index + 1) * 3600_000).toISOString(),
  }
}

function buildFocusedRequestBuckets(
  points: Record<number, { successCount?: number; failureCount?: number }>,
): ForwardProxyHourlyBucket[] {
  return Array.from({ length: 24 }, (_, index) => {
    const point = points[index] ?? {}
    return {
      ...hourBucket(index),
      successCount: point.successCount ?? 0,
      failureCount: point.failureCount ?? 0,
    }
  })
}

function buildWeightBuckets(base: number, dips: Record<number, number> = {}): ForwardProxyWeightBucket[] {
  return Array.from({ length: 24 }, (_, index) => {
    const lastWeight = Number((base + (dips[index] ?? 0)).toFixed(2))
    return {
      ...hourBucket(index),
      sampleCount: 1,
      minWeight: Number((lastWeight - 0.08).toFixed(2)),
      maxWeight: Number((lastWeight + 0.06).toFixed(2)),
      avgWeight: Number((lastWeight - 0.01).toFixed(2)),
      lastWeight,
    }
  })
}

function windowStats(
  attempts: number,
  successCount: number,
  avgLatencyMs?: number,
): ForwardProxyWindowStats {
  return {
    attempts,
    successRate: attempts > 0 ? Number((successCount / attempts).toFixed(4)) : undefined,
    avgLatencyMs: attempts > 0 ? avgLatencyMs : undefined,
  }
}

function nodeStats(
  oneMinute: ForwardProxyWindowStats,
  fifteenMinutes: ForwardProxyWindowStats,
  oneHour: ForwardProxyWindowStats,
  oneDay: ForwardProxyWindowStats,
  sevenDays: ForwardProxyWindowStats,
): ForwardProxyNodeStats {
  return { oneMinute, fifteenMinutes, oneHour, oneDay, sevenDays }
}

export const PARITY_DIRECT_KEY = '__direct__'
export const PARITY_JP_EDGE_KEY = 'fpn_5a7b0c1d2e3f4a10'
export const PARITY_US_EDGE_KEY = 'fpn_0c1d2e3f4a5b6c40'

export const parityBindingNodes: ForwardProxyBindingNode[] = [
  {
    key: PARITY_DIRECT_KEY,
    source: 'direct',
    displayName: 'Direct',
    protocolLabel: 'DIRECT',
    penalized: false,
    selectable: true,
    last24h: buildFocusedRequestBuckets({
      18: { successCount: 1 },
      21: { successCount: 1 },
    }),
  },
  {
    key: PARITY_JP_EDGE_KEY,
    source: 'manual',
    displayName: 'JP Edge 01',
    protocolLabel: 'HTTP',
    penalized: false,
    selectable: true,
    last24h: buildFocusedRequestBuckets({
      17: { successCount: 2 },
      18: { failureCount: 1 },
      20: { successCount: 2 },
      22: { failureCount: 1 },
    }),
  },
  {
    key: PARITY_US_EDGE_KEY,
    source: 'subscription',
    displayName: 'US Edge 03',
    protocolLabel: 'VLESS',
    penalized: true,
    selectable: true,
    last24h: buildFocusedRequestBuckets({}),
  },
]

export const parityLiveStats: ForwardProxyLiveStatsResponse = {
  rangeStart: RANGE_START,
  rangeEnd: '2026-04-06T12:00:00.000Z',
  bucketSeconds: 3600,
  nodes: [
    {
      key: PARITY_DIRECT_KEY,
      source: 'direct',
      displayName: 'Direct',
      weight: 1.02,
      penalized: false,
      stats: nodeStats(
        windowStats(0, 0),
        windowStats(0, 0),
        windowStats(0, 0),
        windowStats(2, 2, 128),
        windowStats(2, 2, 128),
      ),
      last24h: parityBindingNodes[0].last24h,
      weight24h: buildWeightBuckets(1.02, { 19: 0.03, 21: 0.04 }),
    },
    {
      key: PARITY_JP_EDGE_KEY,
      source: 'manual',
      displayName: 'JP Edge 01',
      endpointUrl: 'http://jp-edge-01.internal:8080',
      weight: 0.92,
      penalized: false,
      stats: nodeStats(
        windowStats(0, 0),
        windowStats(0, 0),
        windowStats(1, 0),
        windowStats(6, 4, 184),
        windowStats(6, 4, 184),
      ),
      last24h: parityBindingNodes[1].last24h,
      weight24h: buildWeightBuckets(0.92, { 18: -0.24, 22: -0.18 }),
    },
    {
      key: PARITY_US_EDGE_KEY,
      source: 'subscription',
      displayName: 'US Edge 03',
      endpointUrl: 'vless://example@us-edge-03.example.com:443',
      weight: -0.74,
      penalized: true,
      stats: nodeStats(
        windowStats(0, 0),
        windowStats(0, 0),
        windowStats(0, 0),
        windowStats(0, 0),
        windowStats(0, 0),
      ),
      last24h: parityBindingNodes[2].last24h,
      weight24h: buildWeightBuckets(-0.74, { 12: -0.08, 18: -0.06, 23: -0.1 }),
    },
  ],
}

export const paritySettingsNodes: ForwardProxyNode[] = parityLiveStats.nodes.map((node) => ({
  key: node.key,
  source: node.source,
  displayName: node.displayName,
  endpointUrl: node.endpointUrl,
  weight: node.weight,
  penalized: node.penalized,
  stats: node.stats,
}))

export const parityGroupDialogNote =
  'Group context still controls which bindings are shown, but the health counts now match the same real node attempts shown in Live and Settings.'
