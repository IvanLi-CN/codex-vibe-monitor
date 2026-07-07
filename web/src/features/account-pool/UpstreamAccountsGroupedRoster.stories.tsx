import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, within } from 'storybook/test'
import type { ComponentProps } from 'react'
import type { AccountTagSummary, EffectiveRoutingRule, UpstreamAccountSummary } from '../../lib/api'
import { UpstreamAccountsGroupedRoster, type UpstreamAccountsGroupedRosterGroup } from './UpstreamAccountsGroupedRoster'

const defaultEffectiveRoutingRule: EffectiveRoutingRule = {
  allowCutOut: true,
  allowCutIn: true,
  sourceTagIds: [],
  sourceTagNames: [],
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
  noRefreshToken: '无 RT',
  duplicate: 'Duplicate',
  mother: 'Mother',
  hiddenTagsA11y: (count: number, names: string) => `Show ${count} hidden tags: ${names}`,
  compactSupport: () => 'Compact unsupported',
  compactSupportHint: (item: UpstreamAccountSummary) => item.compactSupport?.reason ?? null,
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
  { id: 4, name: 'priority-lane', routingRule: defaultEffectiveRoutingRule },
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
    hasCustomSettings: false,
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
      makeItem(1, {
        displayName: 'Codex Pro - Tokyo',
        currentForwardProxyDisplayName: 'JP Edge 01',
        hasRefreshToken: false,
      }),
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

const actionableStatusGroup = buildGroup(
  'status-states',
  'status-states',
  [
    makeItem(31, {
      displayName: 'Working burst lane',
      workStatus: 'working',
      activeConversationCount: 3,
      planType: 'team',
      tags: rosterTags,
    }),
    makeItem(32, {
      displayName: 'Temporary degraded lane',
      workStatus: 'degraded',
      planType: 'pro',
      lastActionReasonMessage: 'Temporary route failures are reducing fresh assignment priority.',
      lastActionReasonCode: 'upstream_http_502_temporary',
      lastActionHttpStatus: 502,
    }),
    makeItem(33, {
      displayName: 'Quota limited lane',
      workStatus: 'rate_limited',
      planType: 'free',
      lastActionReasonMessage: '7d usage window is exhausted.',
      lastActionReasonCode: 'upstream_http_429_quota_exhausted',
      lastActionHttpStatus: 429,
    }),
    makeItem(34, {
      displayName: 'Manual sync in progress',
      displayStatus: 'syncing',
      syncState: 'syncing',
      workStatus: 'idle',
      planType: 'team',
      lastActionReasonMessage: 'Primary usage snapshot refresh is still running.',
      lastActionReasonCode: 'manual_sync_started',
      lastActionHttpStatus: null,
    }),
    makeItem(35, {
      displayName: 'OAuth needs reauth',
      displayStatus: 'needs_reauth',
      healthStatus: 'needs_reauth',
      workStatus: 'unavailable',
      lastActionReasonMessage: 'Upstream session expired and requires a fresh login.',
      lastActionReasonCode: 'reauth_required',
      lastActionHttpStatus: 401,
    }),
    makeItem(36, {
      displayName: 'Data plane unavailable',
      displayStatus: 'upstream_unavailable',
      healthStatus: 'upstream_unavailable',
      workStatus: 'unavailable',
      lastActionReasonMessage: 'Regional data plane is overloaded right now.',
      lastActionReasonCode: 'upstream_server_overloaded',
      lastActionHttpStatus: 503,
    }),
    makeItem(37, {
      displayName: 'Upstream rejected',
      displayStatus: 'upstream_rejected',
      healthStatus: 'upstream_rejected',
      workStatus: 'unavailable',
      lastActionReasonMessage: 'The current token scope was rejected by the upstream gateway.',
      lastActionReasonCode: 'upstream_http_403_scope_rejected',
      lastActionHttpStatus: 403,
    }),
    makeItem(38, {
      displayName: 'Disabled fallback key',
      enabled: false,
      enableStatus: 'disabled',
      displayStatus: 'disabled',
      workStatus: 'idle',
      healthStatus: 'normal',
      syncState: 'idle',
      planType: 'pro',
      kind: 'api_key_codex',
    }),
    makeItem(39, {
      displayName: 'Other error account',
      displayStatus: 'error_other',
      healthStatus: 'error_other',
      workStatus: 'unavailable',
      lastActionReasonMessage: 'Unknown bridge exception surfaced during the latest call.',
      lastActionReasonCode: 'bridge_exception',
      lastActionHttpStatus: 500,
    }),
  ],
  {
    concurrencyLimit: 4,
    nodeShuntEnabled: false,
    boundProxyLabels: ['JP Edge 01'],
  },
)

const meta = {
  title: 'Account Pool/Components/UpstreamAccountsGroupedRoster',
  component: UpstreamAccountsGroupedRoster,
  tags: ['autodocs'],
  parameters: {
    layout: 'fullscreen',
    viewport: { defaultViewport: 'desktop1660' },
  },
  args: {
    selectedId: 1,
    selectedAccountIds: new Set<number>(),
    onSelect: () => undefined,
    onToggleSelected: () => undefined,
    canEditGroupSettings: true,
    onEditGroupSettings: () => undefined,
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
      settingsLabel: 'Edit group settings',
      upstream429Enabled: (count: number) => `429 retry × ${count}`,
      upstream429Disabled: '429 retry off',
      policyPriorityPrimary: 'Primary',
      policyPriorityFallback: 'Fallback',
      policyFastFillMissing: '+Fast',
      policyFastForceAdd: 'Fast',
      policyFastForceRemove: 'No Fast',
      policyForbidCutOut: 'No out',
      policyForbidCutIn: 'No in',
      policyForbidNewConversation: 'No new',
      policyConcurrency: (count: number) => `Conc ${count}`,
      policyRetry: (count: number) => `Retry ${count}`,
    },
  },
} satisfies Meta<typeof UpstreamAccountsGroupedRoster>

export default meta

type Story = StoryObj<typeof meta>
type GroupedRosterStoryArgs = NonNullable<Story['args']>
type GroupedRosterProps = ComponentProps<typeof UpstreamAccountsGroupedRoster>

function renderWithinWidth(args: GroupedRosterStoryArgs, widthPx: number) {
  const resolvedArgs = { ...meta.args, ...args } as GroupedRosterProps
  return (
    <div className="mx-auto w-full py-6" style={{ maxWidth: `${widthPx}px` }}>
      <UpstreamAccountsGroupedRoster {...resolvedArgs} />
    </div>
  )
}

function expectActionableStatusBadges(canvasElement: HTMLElement) {
  const canvas = within(canvasElement)

  return Promise.all([
    expect(canvas.getByText('Working 3')).toBeInTheDocument(),
    expect(canvas.getByText('Degraded')).toBeInTheDocument(),
    expect(canvas.getByText('Rate limited')).toBeInTheDocument(),
    expect(canvas.getByText('Syncing')).toBeInTheDocument(),
    expect(canvas.getByText('Needs reauth')).toBeInTheDocument(),
    expect(canvas.getByText('Upstream unavailable')).toBeInTheDocument(),
    expect(canvas.getByText('Upstream rejected')).toBeInTheDocument(),
    expect(canvas.getByText('Other error')).toBeInTheDocument(),
    expect(canvas.getByText('Disabled')).toBeInTheDocument(),
    expect(canvas.getByText('vip')).toBeInTheDocument(),
    expect(canvas.getByText('burst-safe')).toBeInTheDocument(),
    expect(canvas.getByText('prod-apac')).toBeInTheDocument(),
    expect(canvas.getByText('+1')).toBeInTheDocument(),
  ]).then(() => {
    expect(canvas.queryByText('Enabled')).toBeNull()
    expect(canvas.queryByText('Idle')).toBeNull()
    expect(canvas.queryByText('Normal')).toBeNull()
    expect(canvas.queryByText('Sync idle')).toBeNull()
    expect(canvas.queryByText('priority-lane')).toBeNull()
  })
}

function expectGridBadgeParity(canvasElement: HTMLElement) {
  const canvas = within(canvasElement)

  return Promise.all([
    expect(canvas.getByText('Mother')).toBeInTheDocument(),
    expect(canvas.getByText('Duplicate')).toBeInTheDocument(),
    expect(canvas.getByText('Compact unsupported')).toBeInTheDocument(),
    expect(canvas.getByText('Tokyo subscription edge with a deliberately long display name')).toBeInTheDocument(),
    expect(canvas.getByText('OAuth')).toBeInTheDocument(),
    expect(canvas.getByText('team')).toBeInTheDocument(),
    expect(canvas.getByText('vip')).toBeInTheDocument(),
    expect(canvas.getByText('burst-safe')).toBeInTheDocument(),
    expect(canvas.getByText('prod-apac')).toBeInTheDocument(),
    expect(canvas.getByText('+1')).toBeInTheDocument(),
  ]).then(() => {
    const badgeRow = canvasElement.querySelector<HTMLElement>(
      '[data-testid="upstream-accounts-group-grid-card-badges"]',
    )
    expect(badgeRow).not.toBeNull()
    expect(badgeRow?.className).toContain('flex-wrap')
    expect(canvasElement.querySelectorAll('[data-testid="upstream-accounts-group-grid-card-badges"]')).toHaveLength(4)
  })
}

async function expectGridColumnCount(canvasElement: HTMLElement, expectedCount: number) {
  const grid = canvasElement.querySelector<HTMLElement>('[data-testid="upstream-accounts-group-grid-row"]')
  expect(grid).not.toBeNull()
  const cards = Array.from(
    canvasElement.querySelectorAll<HTMLElement>('[data-testid="upstream-accounts-group-grid-card"]'),
  )
  expect(cards.length).toBeGreaterThan(0)
  const distinctColumns = new Set(cards.map((card) => Math.round(card.getBoundingClientRect().left)))
  expect(distinctColumns.size).toBe(expectedCount)
}

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
    groups: [
      buildGroup(
        'grid-badge-parity',
        'grid-badge-parity',
        [
          makeItem(81, {
            displayName: 'Mother compact duplicate lane',
            isMother: true,
            duplicateInfo: {
              peerAccountIds: [181],
              reasons: ['sharedChatgptAccountId'],
            },
            compactSupport: {
              status: 'unsupported',
              reason: 'No available channel for model gpt-5.5',
            },
            currentForwardProxyDisplayName:
              'Tokyo subscription edge with a deliberately long display name',
            planType: 'team',
            tags: rosterTags,
          }),
          ...baseGroups[0].items,
        ],
        {
          concurrencyLimit: 6,
          boundProxyLabels: ['Tokyo subscription edge with a deliberately long display name'],
        },
      ),
    ],
    memberLayout: 'grid',
    selectionMode: 'none',
    onToggleSelected: undefined,
    onToggleSelectAllVisible: undefined,
  },
  play: async ({ canvasElement }) => {
    await expectGridBadgeParity(canvasElement)
  },
}

export const ActionableStatusGridCards: Story = {
  args: {
    groups: [actionableStatusGroup],
    memberLayout: 'grid',
    selectionMode: 'none',
    onToggleSelected: undefined,
    onToggleSelectAllVisible: undefined,
  },
  play: async ({ canvasElement }) => {
    await expectActionableStatusBadges(canvasElement)
  },
}

export const ActionableStatusGridCardsThreeColumns: Story = {
  args: ActionableStatusGridCards.args,
  render: (args) => renderWithinWidth(args, 1440),
  play: async ({ canvasElement }) => {
    await expectActionableStatusBadges(canvasElement)
    await expectGridColumnCount(canvasElement, 3)
  },
}

export const ActionableStatusGridCardsTwoColumns: Story = {
  args: ActionableStatusGridCards.args,
  render: (args) => renderWithinWidth(args, 1220),
  play: async ({ canvasElement }) => {
    await expectActionableStatusBadges(canvasElement)
    await expectGridColumnCount(canvasElement, 2)
  },
}

export const ActionableStatusGridCardsOneColumn: Story = {
  args: ActionableStatusGridCards.args,
  render: (args) => renderWithinWidth(args, 980),
  play: async ({ canvasElement }) => {
    await expectActionableStatusBadges(canvasElement)
    await expectGridColumnCount(canvasElement, 1)
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
    groups: Array.from({ length: 36 }, (_, groupIndex) => {
      const groupId = `virtual-group-${groupIndex + 1}`
      return buildGroup(
        groupId,
        `virtual-group-${groupIndex + 1}`,
        Array.from({ length: 4 + (groupIndex % 5) }, (_, itemIndex) =>
          makeItem(groupIndex * 20 + itemIndex + 100, {
            displayName: `Group ${groupIndex + 1} Account ${itemIndex + 1}`,
            groupName: groupId,
            planType:
              itemIndex % 4 === 0
                ? 'team'
                : itemIndex % 3 === 0
                  ? 'free'
                  : 'pro',
            currentForwardProxyDisplayName:
              itemIndex % 3 === 0 ? 'JP Edge 01' : 'SG Transit 03',
            currentForwardProxyState:
              itemIndex % 6 === 0
                ? 'pending'
                : itemIndex % 7 === 0
                  ? 'unconfigured'
                  : 'assigned',
            currentForwardProxyKey:
              itemIndex % 7 === 0
                ? null
                : itemIndex % 3 === 0
                  ? 'jp-edge-01'
                  : 'sg-transit-03',
          }),
        ),
        {
          concurrencyLimit: 2 + (groupIndex % 5),
          nodeShuntEnabled: groupIndex % 4 === 0,
          hasCustomSettings: groupIndex % 3 === 0,
          boundProxyLabels:
            groupIndex % 2 === 0 ? ['JP Edge 01', 'SG Transit 03'] : ['DE Transit 02'],
        },
      )
    }),
  },
}

export const VirtualizedLargeGridRoster: Story = {
  args: {
    ...VirtualizedLargeRoster.args,
    memberLayout: 'grid',
    selectionMode: 'none',
    onToggleSelected: undefined,
    onToggleSelectAllVisible: undefined,
  },
  render: (args) => renderWithinWidth(args, 1440),
}
