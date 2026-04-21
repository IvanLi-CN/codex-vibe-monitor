import type { Meta, StoryObj } from '@storybook/react-vite'
import type { AccountTagSummary, EffectiveRoutingRule, UpstreamAccountSummary } from '../lib/api'
import { UpstreamAccountsGroupedRoster, type UpstreamAccountsGroupedRosterGroup } from './UpstreamAccountsGroupedRoster'

const defaultEffectiveRoutingRule: EffectiveRoutingRule = {
  guardEnabled: false,
  lookbackHours: null,
  maxConversations: null,
  allowCutOut: true,
  allowCutIn: true,
  sourceTagIds: [],
  sourceTagNames: [],
  guardRules: [],
}

const labels = {
  selectPage: 'Select current page',
  selectRow: (name: string) => `Select ${name}`,
  account: 'Account',
  sync: 'Sync / Call',
  lastSuccess: 'Sync',
  lastCall: 'Call',
  routingBlock: 'Blocked',
  latestAction: 'Latest',
  windows: 'Windows',
  never: 'Never',
  primary: '5h',
  primaryShort: '5h',
  secondary: '7d',
  secondaryShort: '7d',
  nextReset: 'Reset',
  nextResetCompact: 'Reset',
  requestsMetric: 'Req',
  tokensMetric: 'Token',
  costMetric: 'Cost',
  inputTokensMetric: 'Input',
  outputTokensMetric: 'Output',
  cacheInputTokensMetric: 'Cached input',
  unknown: 'Unknown',
  unavailable: 'Unavailable',
  oauth: 'OAuth',
  apiKey: 'API key',
  duplicate: 'Duplicate',
  mother: 'Mother',
  hiddenTagsA11y: (count: number, names: string) => `Show ${count} hidden tags: ${names}`,
  workStatus: (status: string) =>
    ({
      working: 'Working',
      degraded: 'Degraded',
      idle: 'Idle',
      rate_limited: 'Rate limited',
      unavailable: 'Unavailable',
    })[status] ?? status,
  workStatusCount: (count: number) => `Working ${count}`,
  enableStatus: (status: string) =>
    ({ enabled: 'Enabled', disabled: 'Disabled' })[status] ?? status,
  healthStatus: (status: string) =>
    ({
      normal: 'Normal',
      needs_reauth: 'Needs reauth',
      upstream_unavailable: 'Upstream unavailable',
      upstream_rejected: 'Upstream rejected',
      error_other: 'Other error',
      error: 'Error',
    })[status] ?? status,
  syncState: (status: string) => ({ idle: 'Sync idle', syncing: 'Syncing' })[status] ?? status,
  action: (action?: string | null) => action ?? null,
  actionSource: (source?: string | null) => source ?? null,
  actionReason: (reason?: string | null) => reason ?? null,
  latestActionFieldAction: 'Action',
  latestActionFieldSource: 'Source',
  latestActionFieldReason: 'Reason',
  latestActionFieldHttpStatus: 'HTTP',
  latestActionFieldOccurredAt: 'Occurred',
  latestActionFieldMessage: 'Message',
  forwardProxyPending: 'Pending',
  forwardProxyUnconfigured: 'Unconfigured proxy',
}

const rosterTags: AccountTagSummary[] = [
  { id: 1, name: 'vip', routingRule: defaultEffectiveRoutingRule },
  { id: 2, name: 'burst-safe', routingRule: defaultEffectiveRoutingRule },
  { id: 3, name: 'prod-apac', routingRule: defaultEffectiveRoutingRule },
]

function usage(requestCount: number, totalTokens: number, totalCost: number) {
  const cacheInputTokens = Math.round(totalTokens * 0.1)
  const inputTokens = Math.round(totalTokens * 0.55)
  const outputTokens = totalTokens - inputTokens - cacheInputTokens
  return {
    requestCount,
    totalTokens,
    totalCost,
    inputTokens,
    outputTokens,
    cacheInputTokens,
  }
}

function makeItem(id: number, overrides: Partial<UpstreamAccountSummary> = {}): UpstreamAccountSummary {
  const baseTime = `2026-03-11T12:${String((id % 45) + 10).padStart(2, '0')}:00.000Z`
  return {
    id,
    kind: id % 4 === 0 ? 'api_key_codex' : 'oauth_codex',
    provider: 'codex',
    displayName: `Account ${id}`,
    groupName: 'production-apac',
    isMother: id % 9 === 0,
    status: 'active',
    displayStatus: 'active',
    enabled: true,
    enableStatus: 'enabled',
    workStatus: id % 6 === 0 ? 'rate_limited' : id % 5 === 0 ? 'degraded' : 'working',
    healthStatus: 'normal',
    syncState: 'idle',
    email: `account-${id}@example.com`,
    chatgptAccountId: `org_${id}`,
    planType: id % 5 === 0 ? 'free' : id % 3 === 0 ? 'team' : 'pro',
    lastSyncedAt: baseTime,
    lastSuccessfulSyncAt: baseTime,
    lastActivityAt: baseTime,
    activeConversationCount: id % 4,
    lastAction: 'route_hard_unavailable',
    lastActionSource: 'call',
    lastActionReasonCode: 'upstream_http_429_quota_exhausted',
    lastActionReasonMessage: 'Weekly cap exhausted for this account',
    lastActionHttpStatus: 429,
    lastActionAt: baseTime,
    currentForwardProxyKey: 'jp-edge-01',
    currentForwardProxyDisplayName: 'JP Edge 01',
    currentForwardProxyState: 'assigned',
    primaryWindow: {
      usedPercent: (id * 13) % 100,
      usedText: 'rolling 5h',
      limitText: '5h rolling window',
      resetsAt: '2026-03-11T14:00:00.000Z',
      windowDurationMins: 300,
      actualUsage: usage(12 + (id % 14), 32000 + id * 1200, Number((0.16 + id * 0.013).toFixed(4))),
    },
    secondaryWindow: {
      usedPercent: (id * 7) % 100,
      usedText: 'rolling 7d',
      limitText: '7d rolling window',
      resetsAt: '2026-03-18T00:00:00.000Z',
      windowDurationMins: 10080,
      actualUsage: usage(50 + (id % 30), 180000 + id * 4000, Number((1.2 + id * 0.03).toFixed(4))),
    },
    credits: null,
    localLimits: {
      primaryLimit: null,
      secondaryLimit: null,
      limitUnit: 'requests',
    },
    compactSupport: id % 8 === 0 ? { status: 'unsupported', reason: 'Compact channel unavailable' } : null,
    duplicateInfo: id % 11 === 0 ? { peerAccountIds: [id + 100], reasons: ['sharedChatgptAccountId'] } : null,
    tags: rosterTags.slice(0, (id % rosterTags.length) + 1),
    effectiveRoutingRule: defaultEffectiveRoutingRule,
    ...overrides,
  }
}

function buildGroup(
  id: string,
  displayName: string,
  items: UpstreamAccountSummary[],
  overrides: Partial<UpstreamAccountsGroupedRosterGroup> = {},
): UpstreamAccountsGroupedRosterGroup {
  const counts = new Map<string, number>()
  for (const item of items) {
    const plan = item.planType?.trim().toLowerCase()
    if (!plan) continue
    counts.set(plan, (counts.get(plan) ?? 0) + 1)
  }
  const apiCount = items.filter((item) => item.kind === 'api_key_codex').length
  const orderedPlans = ['free', 'pro', 'team', 'enterprise'].filter((plan) => counts.has(plan))
  return {
    id,
    groupName: id === '__ungrouped__' ? null : id,
    displayName,
    items,
    note: `${displayName} group note`,
    boundProxyLabels: [],
    concurrencyLimit: 3,
    nodeShuntEnabled: false,
    planCounts: [
      ...orderedPlans.map((plan) => ({
        key: plan,
        label: ({ free: 'Free', pro: 'Plus', team: 'Team', enterprise: 'Enterprise' })[plan] ?? plan,
        count: counts.get(plan) ?? 0,
      })),
      ...(apiCount > 0
        ? [
            {
              key: 'api',
              label: 'API',
              count: apiCount,
            },
          ]
        : []),
    ],
    ...overrides,
  }
}

const baseGroups = [
  buildGroup(
    'production-apac',
    'production-apac',
    [
      makeItem(1, { displayName: 'Codex Pro - Tokyo', currentForwardProxyDisplayName: 'JP Edge 01' }),
      makeItem(2, { displayName: 'Codex Team - Osaka', planType: 'team', currentForwardProxyState: 'pending', currentForwardProxyKey: null, currentForwardProxyDisplayName: null }),
      makeItem(3, { displayName: 'Fallback Key - Seoul', kind: 'api_key_codex', currentForwardProxyState: 'unconfigured', currentForwardProxyKey: null, currentForwardProxyDisplayName: null }),
    ],
    {
      concurrencyLimit: 6,
      nodeShuntEnabled: true,
      note: 'APAC production roster for regional failover and premium traffic.',
      boundProxyLabels: ['JP Edge 01', 'SG Transit 03'],
    },
  ),
  buildGroup(
    'production-emea',
    'production-emea',
    [
      makeItem(4, { displayName: 'EMEA Pro - Paris', groupName: 'production-emea', currentForwardProxyDisplayName: 'DE Transit 02' }),
      makeItem(5, { displayName: 'EMEA Free - Berlin', groupName: 'production-emea', planType: 'free', currentForwardProxyDisplayName: 'DE Transit 02' }),
    ],
    { concurrencyLimit: 2, note: 'EMEA production roster with mixed OAuth and API key coverage.', boundProxyLabels: ['DE Transit 02'] },
  ),
]

const meta = {
  title: 'Account Pool/Components/UpstreamAccountsGroupedRoster',
  component: UpstreamAccountsGroupedRoster,
  parameters: {
    layout: 'fullscreen',
  },
  args: {
    selectedId: 1,
    selectedAccountIds: new Set<number>(),
    onSelect: () => undefined,
    onToggleSelected: () => undefined,
    memberLayout: 'list',
    selectionMode: 'multi',
    emptyTitle: 'No upstream account yet',
    emptyDescription: 'Create an OAuth or API key account to start building the pool.',
    labels,
    groupLabels: {
      count: (count: number) => `${count} accounts`,
      concurrency: (value: number) => `Concurrency ${value}`,
      exclusiveNode: 'Exclusive node',
      selectVisible: 'Select visible accounts',
      infoTitle: 'Group info',
      noteLabel: 'Note',
      noteEmpty: 'No group note',
      proxiesLabel: 'Forward proxies',
      proxiesEmpty: 'No bound proxy',
    },
  },
} satisfies Meta<typeof UpstreamAccountsGroupedRoster>

export default meta

type Story = StoryObj<typeof meta>

export const Overview: Story = {
  args: {
    groups: baseGroups,
  },
}

export const ProxyBadgeStates: Story = {
  args: {
    groups: [baseGroups[0]],
  },
}

export const GridCards: Story = {
  args: {
    groups: baseGroups,
    memberLayout: 'grid',
    selectionMode: 'none',
    onToggleSelected: undefined,
    onToggleSelectAllVisible: undefined,
  },
}

export const UngroupedBucket: Story = {
  args: {
    groups: [
      buildGroup(
        '__ungrouped__',
        'Ungrouped',
        [
          makeItem(21, { groupName: null, displayName: 'Ungrouped OAuth 01' }),
          makeItem(22, { groupName: null, displayName: 'Ungrouped API Key 02', kind: 'api_key_codex', currentForwardProxyState: 'unconfigured', currentForwardProxyKey: null, currentForwardProxyDisplayName: null }),
        ],
        { concurrencyLimit: 0 },
      ),
    ],
  },
}

export const VirtualizedLargeRoster: Story = {
  args: {
    groups: [
      buildGroup(
        'production-apac',
        'production-apac',
        Array.from({ length: 90 }, (_, index) =>
          makeItem(100 + index, {
            displayName: `APAC Account ${index + 1}`,
            groupName: 'production-apac',
            currentForwardProxyDisplayName: index % 4 === 0 ? 'JP Edge 01' : 'SG Transit 03',
            currentForwardProxyState:
              index % 9 === 0 ? 'pending' : index % 11 === 0 ? 'unconfigured' : 'assigned',
            currentForwardProxyKey:
              index % 11 === 0 ? null : index % 4 === 0 ? 'jp-edge-01' : 'sg-transit-03',
          }),
        ),
        { concurrencyLimit: 12, nodeShuntEnabled: true },
      ),
      buildGroup(
        'overflow',
        'overflow',
        Array.from({ length: 64 }, (_, index) =>
          makeItem(300 + index, {
            displayName: `Overflow Account ${index + 1}`,
            groupName: 'overflow',
            planType: index % 2 === 0 ? 'team' : 'pro',
            currentForwardProxyDisplayName: 'DE Transit 02',
          }),
        ),
        { concurrencyLimit: 8 },
      ),
    ],
  },
}
