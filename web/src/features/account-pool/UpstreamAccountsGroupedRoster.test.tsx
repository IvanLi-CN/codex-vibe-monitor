/** @vitest-environment jsdom */
import { act, type ComponentProps } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, describe, expect, it, vi } from 'vitest'

const virtualizerMocks = vi.hoisted(() => ({
  visibleIndexes: null as number[] | null,
  byOverscan: {} as Record<
    number,
    {
      sizes: number[]
      lastScrollMargin: number
      measureCalls: number
    }
  >,
}))

vi.mock('@tanstack/react-virtual', () => ({
  useWindowVirtualizer: ({
    count,
    estimateSize,
    overscan = 0,
    scrollMargin = 0,
  }: {
    count: number
    estimateSize: (index: number) => number
    overscan?: number
    scrollMargin?: number
  }) => {
    const sizes = Array.from({ length: count }, (_, index) => estimateSize(index))
    const metrics =
      virtualizerMocks.byOverscan[overscan] ??
      (virtualizerMocks.byOverscan[overscan] = {
        sizes: [],
        lastScrollMargin: 0,
        measureCalls: 0,
      })
    metrics.sizes = sizes
    metrics.lastScrollMargin = scrollMargin

    const indexes =
      virtualizerMocks.visibleIndexes ??
      Array.from({ length: Math.min(count, 3) }, (_, index) => index)
    const items = indexes
      .filter((index) => index >= 0 && index < count)
      .map((index) => {
        const size = sizes[index] ?? estimateSize(index)
        const start =
          scrollMargin +
          sizes.slice(0, index).reduce((sum, candidateSize) => sum + candidateSize, 0)
        const item = {
          index,
          key: index,
          start,
          size,
          end: start + size,
        }
        return item
      })
    const totalSize = sizes.reduce((sum, size) => sum + size, 0)

    return {
      getVirtualItems: () => items,
      getTotalSize: () => totalSize,
      measureElement: () => undefined,
      measure: () => {
        metrics.measureCalls += 1
      },
    }
  },
}))

import type {
  AccountTagSummary,
  EffectiveRoutingRule,
  UpstreamAccountSummary,
} from '../../lib/api'
import {
  UpstreamAccountsGroupedRoster,
  type UpstreamAccountsGroupedRosterGroup,
} from './UpstreamAccountsGroupedRoster'

const OUTER_OVERSCAN = 3

function outerVirtualizerMetrics() {
  return (
    virtualizerMocks.byOverscan[OUTER_OVERSCAN] ?? {
      sizes: [],
      lastScrollMargin: 0,
      measureCalls: 0,
    }
  )
}

const defaultEffectiveRoutingRule: EffectiveRoutingRule = {
  allowCutOut: true,
  allowCutIn: true,
  sourceTagIds: [],
  sourceTagNames: [],
}

const tags: AccountTagSummary[] = [
  { id: 1, name: 'analytics', routingRule: defaultEffectiveRoutingRule },
  { id: 2, name: 'reporting', routingRule: defaultEffectiveRoutingRule },
]

const extendedTags: AccountTagSummary[] = [
  ...tags,
  { id: 3, name: 'priority-lane', routingRule: defaultEffectiveRoutingRule },
  { id: 4, name: 'prod-apac', routingRule: defaultEffectiveRoutingRule },
]

const labels = {
  selectPage: 'Select current page',
  selectRow: (name: string) => `Select ${name}`,
  account: 'Account',
  sync: 'Sync / Call',
  lastSuccess: 'Sync',
  lastCall: 'Call',
  routingBlock: 'Routing',
  latestAction: 'Latest action',
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
  apiKey: 'API Key',
  duplicate: 'Duplicate',
  mother: 'Mother',
  hiddenTagsA11y: (count: number, names: string) => `Show ${count} hidden tags: ${names}`,
  compactSupport: () => 'Compact unsupported',
  compactSupportHint: (item: UpstreamAccountSummary) => item.compactSupport?.reason ?? null,
  workStatus: (status: string) => status,
  workStatusCount: (count: number) => `Working ${count}`,
  enableStatus: (status: string) => status,
  healthStatus: (status: string) => status,
  syncState: (status: string) => status,
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
}

const groupLabels = {
  count: (count: number) => `${count} accounts`,
  concurrency: (value: number) => `Concurrency ${value}`,
  exclusiveNode: 'Exclusive node',
  selectVisible: 'Select visible accounts',
  infoTitle: 'Group info',
  noteLabel: 'Note',
  noteEmpty: 'No note',
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
}

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
  return {
    id,
    kind: 'api_key_codex',
    provider: 'codex',
    displayName: `Account ${id}`,
    groupName: 'analytics',
    isMother: false,
    status: 'active',
    displayStatus: 'active',
    enabled: true,
    enableStatus: 'enabled',
    workStatus: 'working',
    healthStatus: 'normal',
    syncState: 'idle',
    email: `account-${id}@example.com`,
    chatgptAccountId: `org_${id}`,
    planType: 'pro',
    lastSyncedAt: '2026-03-11T12:10:00.000Z',
    lastSuccessfulSyncAt: '2026-03-11T12:10:00.000Z',
    lastActivityAt: '2026-03-11T12:11:00.000Z',
    activeConversationCount: 0,
    lastAction: 'route_hard_unavailable',
    lastActionSource: 'call',
    lastActionReasonCode: 'upstream_http_429_quota_exhausted',
    lastActionReasonMessage: 'Weekly cap exhausted',
    lastActionHttpStatus: 429,
    lastActionAt: '2026-03-11T12:11:00.000Z',
    currentForwardProxyKey: null,
    currentForwardProxyDisplayName: null,
    currentForwardProxyState: 'unconfigured',
    primaryWindow: {
      usedPercent: 42,
      usedText: 'rolling 5h',
      limitText: '5h rolling window',
      resetsAt: '2026-03-11T14:00:00.000Z',
      windowDurationMins: 300,
      actualUsage: usage(18, 120000, 1.26),
    },
    secondaryWindow: {
      usedPercent: 21,
      usedText: 'rolling 7d',
      limitText: '7d rolling window',
      resetsAt: '2026-03-18T00:00:00.000Z',
      windowDurationMins: 10080,
      actualUsage: usage(65, 640000, 4.82),
    },
    credits: null,
    localLimits: {
      primaryLimit: null,
      secondaryLimit: null,
      limitUnit: 'requests',
    },
    compactSupport: null,
    duplicateInfo: null,
    tags,
    effectiveRoutingRule: defaultEffectiveRoutingRule,
    ...overrides,
  }
}

function makeGroup(
  id: string,
  items: UpstreamAccountSummary[],
  overrides: Partial<UpstreamAccountsGroupedRosterGroup> = {},
): UpstreamAccountsGroupedRosterGroup {
  return {
    id,
    groupName: id,
    displayName: id,
    items,
    note: 'This note should not render in grouped list mode.',
    boundProxyLabels: [],
    concurrencyLimit: 2,
    nodeShuntEnabled: false,
    hasCustomSettings: false,
    planCounts: [{ key: 'api', label: 'API', count: items.length }],
    ...overrides,
  }
}

let host: HTMLDivElement | null = null
let root: Root | null = null

afterEach(() => {
  virtualizerMocks.visibleIndexes = null
  virtualizerMocks.byOverscan = {}
  act(() => {
    root?.unmount()
  })
  host?.remove()
  host = null
  root = null
})

function createRosterProps(
  groups: UpstreamAccountsGroupedRosterGroup[],
  overrides: Partial<ComponentProps<typeof UpstreamAccountsGroupedRoster>> = {},
) {
  return {
    groups,
    selectedId: null,
    selectedAccountIds: new Set<number>(),
    onSelect: () => undefined,
    onToggleSelected: () => undefined,
    onToggleSelectAllVisible: () => undefined,
    emptyTitle: 'Empty',
    emptyDescription: 'Nothing here',
    labels,
    groupLabels,
    ...overrides,
  } satisfies ComponentProps<typeof UpstreamAccountsGroupedRoster>
}

async function flushAsync() {
  await act(async () => {
    await Promise.resolve()
    await Promise.resolve()
  })
}

function renderRoster(
  groups: UpstreamAccountsGroupedRosterGroup[],
  overrides: Partial<ComponentProps<typeof UpstreamAccountsGroupedRoster>> = {},
) {
  host = document.createElement('div')
  document.body.appendChild(host)
  root = createRoot(host)
  act(() => {
    root?.render(<UpstreamAccountsGroupedRoster {...createRosterProps(groups, overrides)} />)
  })
}

describe('UpstreamAccountsGroupedRoster', () => {
  it('uses page-level rendering without internal roster scrolling', () => {
    renderRoster([makeGroup('analytics', [makeItem(1), makeItem(2)])])

    const roster = host?.querySelector('[data-testid="upstream-accounts-grouped-roster"]') as HTMLElement | null
    const members = host?.querySelector('[data-testid="upstream-accounts-group-members"]') as HTMLElement | null

    expect(roster).toBeTruthy()
    expect(roster?.className).not.toContain('overflow-auto')
    expect(members).toBeTruthy()
    expect(members?.className).not.toContain('overflow-auto')
    expect(members?.style.minHeight).toBe('')
    expect(members?.style.height).toBe('')
  })

  it('renders a group settings action and keeps group notes out of the summary', () => {
    const onEditGroupSettings = vi.fn()
    renderRoster(
      [
        makeGroup('analytics', [makeItem(1)], {
          hasCustomSettings: true,
          boundProxyLabels: ['JP Edge 01'],
        }),
      ],
      {
        canEditGroupSettings: true,
        onEditGroupSettings,
      },
    )

    const settingsButton = host?.querySelector('button[aria-label="Edit group settings"]') as HTMLButtonElement | null
    expect(settingsButton).toBeTruthy()

    act(() => {
      settingsButton?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    })

    expect(onEditGroupSettings).toHaveBeenCalledTimes(1)
    expect(host?.textContent).not.toContain('This note should not render in grouped list mode.')
  })

  it('renders active policy badges for groups and accounts', () => {
    renderRoster([
      makeGroup(
        'analytics',
        [
          makeItem(1, {
            effectiveRoutingRule: {
              ...defaultEffectiveRoutingRule,
              allowCutIn: false,
              fastModeRewriteMode: 'force_add',
              upstream429RetryEnabled: true,
              upstream429MaxRetries: 3,
            },
          }),
        ],
        {
          routingRule: {
            priorityTier: "no_new",
            allowCutOut: false,
            allowCutIn: true,
            fastModeRewriteMode: 'force_add',
            concurrencyLimit: 4,
            upstream429RetryEnabled: true,
            upstream429MaxRetries: 3,
          },
        },
      ),
    ])

    expect(host?.textContent).toContain('No new')
    expect(host?.textContent).toContain('Fast')
    expect(host?.textContent).toContain('No out')
    expect(host?.textContent).toContain('No in')
    expect(host?.textContent).toContain('Conc 4')
    expect(host?.textContent).toContain('Retry 3')
  })

  it('reports visible member ids for rendered groups', async () => {
    const onVisibleAccountIdsChange = vi.fn()

    renderRoster([makeGroup('analytics', [makeItem(1), makeItem(2)])], {
      onVisibleAccountIdsChange,
    })
    await flushAsync()

    expect(onVisibleAccountIdsChange).toHaveBeenLastCalledWith([1, 2])
  })

  it('reports only visible group-card accounts and clears them on unmount', async () => {
    virtualizerMocks.visibleIndexes = [1]
    const onVisibleAccountIdsChange = vi.fn()

    renderRoster(
      [
        makeGroup('group-a', [makeItem(1), makeItem(2)]),
        makeGroup('group-b', [
          makeItem(3, { groupName: 'group-b' }),
          makeItem(4, { groupName: 'group-b' }),
        ]),
      ],
      {
        onVisibleAccountIdsChange,
        memberLayout: 'grid',
        selectionMode: 'none',
        onToggleSelected: undefined,
        onToggleSelectAllVisible: undefined,
      },
    )
    await flushAsync()

    expect(onVisibleAccountIdsChange).toHaveBeenLastCalledWith([3, 4])

    act(() => {
      root?.unmount()
    })
    await flushAsync()

    expect(onVisibleAccountIdsChange).toHaveBeenLastCalledWith([])
  })

  it('hides the group settings action for read-only and ungrouped summaries', () => {
    renderRoster(
      [
        makeGroup('analytics', [makeItem(1)]),
        makeGroup('__ungrouped__', [makeItem(2, { groupName: null })], {
          groupName: null,
          displayName: 'Ungrouped',
        }),
      ],
      {
        canEditGroupSettings: false,
        onEditGroupSettings: vi.fn(),
      },
    )

    const settingsButtons = host?.querySelectorAll(
      'button[aria-label="Edit group settings"]',
    ) ?? []

    expect(settingsButtons).toHaveLength(0)
  })

  it('renders actionable-only status badges in grid cards and hides neutral statuses', async () => {
    renderRoster(
      [
        makeGroup('status-states', [
          makeItem(11, {
            displayName: 'Working burst lane',
            workStatus: 'working',
            activeConversationCount: 3,
            tags: extendedTags,
          }),
          makeItem(12, {
            displayName: 'Temporary degraded lane',
            workStatus: 'degraded',
          }),
          makeItem(13, {
            displayName: 'Quota limited lane',
            workStatus: 'rate_limited',
          }),
          makeItem(14, {
            displayName: 'Manual sync in progress',
            displayStatus: 'syncing',
            syncState: 'syncing',
            workStatus: 'idle',
          }),
          makeItem(15, {
            displayName: 'OAuth needs reauth',
            displayStatus: 'needs_reauth',
            healthStatus: 'needs_reauth',
            workStatus: 'unavailable',
          }),
          makeItem(16, {
            displayName: 'Data plane unavailable',
            displayStatus: 'upstream_unavailable',
            healthStatus: 'upstream_unavailable',
            workStatus: 'unavailable',
          }),
          makeItem(17, {
            displayName: 'Upstream rejected',
            displayStatus: 'upstream_rejected',
            healthStatus: 'upstream_rejected',
            workStatus: 'unavailable',
          }),
          makeItem(18, {
            displayName: 'Disabled fallback key',
            enabled: false,
            enableStatus: 'disabled',
            displayStatus: 'disabled',
            workStatus: 'idle',
            healthStatus: 'normal',
            syncState: 'idle',
            kind: 'api_key_codex',
          }),
          makeItem(19, {
            displayName: 'Other error account',
            displayStatus: 'error_other',
            healthStatus: 'error_other',
            workStatus: 'unavailable',
          }),
        ]),
      ],
      {
        memberLayout: 'grid',
        selectionMode: 'none',
        onToggleSelected: undefined,
        onToggleSelectAllVisible: undefined,
      },
    )
    await flushAsync()

    const content = host?.textContent ?? ''
    expect(content).toContain('Working 3')
    expect(content).toContain('analytics')
    expect(content).toContain('reporting')
    expect(content).toContain('+1')
    expect(content).toContain('degraded')
    expect(content).toContain('rate_limited')
    expect(content).toContain('syncing')
    expect(content).toContain('needs_reauth')
    expect(content).toContain('upstream_unavailable')
    expect(content).toContain('upstream_rejected')
    expect(content).toContain('error_other')
    expect(content).toContain('disabled')
    expect(content).not.toContain('Enabled')
    expect(content).not.toContain('Idle')
    expect(content).not.toContain('Normal')
    expect(content).not.toContain('Sync idle')
    expect(content).not.toContain('prod-apac')
  })

  it('renders list-parity identity, support, proxy, and overflow badges in one-line grid rows', async () => {
    renderRoster(
      [
        makeGroup('badge-parity', [
          makeItem(31, {
            displayName: 'Mother compact duplicate lane',
            kind: 'oauth_codex',
            isMother: true,
            duplicateInfo: {
              peerAccountIds: [41],
              reasons: ['sharedChatgptAccountId'],
            },
            compactSupport: {
              status: 'unsupported',
              reason: 'No available channel for model gpt-5.5',
            },
            currentForwardProxyState: 'assigned',
            currentForwardProxyDisplayName:
              'Very long proxy display name for Tokyo subscription edge',
            tags: extendedTags,
          }),
        ]),
      ],
      {
        memberLayout: 'grid',
        selectionMode: 'none',
        onToggleSelected: undefined,
        onToggleSelectAllVisible: undefined,
      },
    )
    await flushAsync()

    const card = host?.querySelector(
      '[data-testid="upstream-accounts-group-grid-card"]',
    ) as HTMLElement | null
    const badgeRow = card?.querySelector(
      '[data-testid="upstream-accounts-group-grid-card-badges"]',
    ) as HTMLElement | null

    expect(card?.textContent).toContain('Mother')
    expect(card?.textContent).toContain('Duplicate')
    expect(card?.textContent).toContain('Compact unsupported')
    expect(card?.textContent).toContain('Very long proxy display name')
    expect(card?.textContent).toContain('OAuth')
    expect(card?.textContent).toContain('pro')
    expect(card?.textContent).toContain('analytics')
    expect(card?.textContent).toContain('reporting')
    expect(card?.textContent).toContain('priority-lane')
    expect(card?.textContent).toContain('+1')
    expect(badgeRow).toBeTruthy()
    expect(badgeRow?.className).toContain('flex-wrap')
    expect(card?.querySelectorAll('[data-testid="upstream-accounts-group-grid-card-badges"]')).toHaveLength(1)
  })

  it('prioritizes disabled, syncing, and health badges ahead of work-state badges in grid cards', async () => {
    renderRoster(
      [
        makeGroup('precedence', [
          makeItem(21, {
            displayName: 'Syncing beats rate limit',
            displayStatus: 'syncing',
            syncState: 'syncing',
            workStatus: 'rate_limited',
          }),
          makeItem(22, {
            displayName: 'Health beats working',
            displayStatus: 'upstream_unavailable',
            healthStatus: 'upstream_unavailable',
            workStatus: 'working',
            activeConversationCount: 2,
          }),
          makeItem(23, {
            displayName: 'Disabled precedence lane',
            enabled: false,
            enableStatus: 'disabled',
            displayStatus: 'disabled',
            workStatus: 'degraded',
          }),
        ]),
      ],
      {
        memberLayout: 'grid',
        selectionMode: 'none',
        onToggleSelected: undefined,
        onToggleSelectAllVisible: undefined,
      },
    )
    await flushAsync()

    const cards = Array.from(
      host?.querySelectorAll('[data-testid="upstream-accounts-group-grid-card"]') ?? [],
    ) as HTMLDivElement[]
    expect(cards).toHaveLength(3)

    expect(cards[0]?.textContent).toContain('syncing')
    expect(cards[0]?.textContent).not.toContain('rate_limited')

    expect(cards[1]?.textContent).toContain('upstream_unavailable')
    expect(cards[1]?.textContent).not.toContain('Working 2')

    expect(cards[2]?.textContent).toContain('disabled')
    expect(cards[2]?.textContent).not.toContain('degraded')
  })

  it('virtualizes large rosters by group card instead of member rows', () => {
    const groups = Array.from({ length: 12 }, (_, groupIndex) =>
      makeGroup(
        `group-${groupIndex + 1}`,
        Array.from({ length: 6 }, (_, itemIndex) =>
          makeItem(groupIndex * 10 + itemIndex + 1, {
            groupName: `group-${groupIndex + 1}`,
            displayName: `Group ${groupIndex + 1} Account ${itemIndex + 1}`,
          }),
        ),
      ),
    )

    renderRoster(groups, {
      memberLayout: 'grid',
      selectionMode: 'none',
      onToggleSelected: undefined,
      onToggleSelectAllVisible: undefined,
    })

    const renderedGroupCards = host?.querySelectorAll('[data-testid="upstream-accounts-group-card"]') ?? []
    const gridCards = host?.querySelectorAll('[data-testid="upstream-accounts-group-grid-card"]') ?? []

    expect(renderedGroupCards.length).toBeGreaterThan(0)
    expect(renderedGroupCards.length).toBeLessThan(groups.length)
    expect(gridCards.length).toBeGreaterThan(0)
    expect(gridCards.length).toBe(18)
  })

  it('uses the rendered roster width instead of window.innerWidth for grouped grid columns', () => {
    const groups = [
      makeGroup('analytics', [
        makeItem(1),
        makeItem(2, { displayName: 'Account 2' }),
        makeItem(3, { displayName: 'Account 3' }),
      ]),
    ]

    renderRoster(groups, {
      memberLayout: 'grid',
      selectionMode: 'none',
      onToggleSelected: undefined,
      onToggleSelectAllVisible: undefined,
    })

    const roster = host?.querySelector(
      '[data-testid="upstream-accounts-grouped-roster"]',
    ) as HTMLDivElement | null
    const spacer = host?.querySelector(
      '[data-testid="upstream-accounts-grouped-roster-spacer"]',
    ) as HTMLDivElement | null
    const membersGrid = host?.querySelector(
      '[data-testid="upstream-accounts-group-members-grid"]',
    ) as HTMLDivElement | null
    const gridLayout = membersGrid?.querySelector(
      '[data-testid="upstream-accounts-group-grid-row"]',
    ) as HTMLDivElement | null

    expect(roster).toBeTruthy()
    expect(spacer).toBeTruthy()
    expect(membersGrid).toBeTruthy()
    expect(gridLayout).toBeTruthy()

    Object.defineProperty(window, 'innerWidth', {
      configurable: true,
      value: 1600,
    })
    Object.defineProperty(roster!, 'getBoundingClientRect', {
      configurable: true,
      value: () =>
        ({
          top: 160,
          left: 0,
          right: 920,
          bottom: 760,
          width: 920,
          height: 600,
          x: 0,
          y: 160,
          toJSON: () => ({}),
        }) satisfies DOMRect,
    })
    Object.defineProperty(spacer!, 'getBoundingClientRect', {
      configurable: true,
      value: () =>
        ({
          top: 160,
          left: 0,
          right: 920,
          bottom: 760,
          width: 920,
          height: 600,
          x: 0,
          y: 160,
          toJSON: () => ({}),
        }) satisfies DOMRect,
    })
    Object.defineProperty(membersGrid!, 'getBoundingClientRect', {
      configurable: true,
      value: () =>
        ({
          top: 210,
          left: 0,
          right: 920,
          bottom: 560,
          width: 920,
          height: 350,
          x: 0,
          y: 210,
          toJSON: () => ({}),
        }) satisfies DOMRect,
    })

    act(() => {
      window.dispatchEvent(new Event('resize'))
    })

    expect(gridLayout?.style.gridTemplateColumns).toBe('repeat(1, minmax(0, 1fr))')
  })

  it('uses the viewport xl breakpoint when estimating grouped grid columns before member widths are measured', () => {
    const groups = [
      makeGroup('analytics', [
        makeItem(1),
        makeItem(2, { displayName: 'Account 2' }),
        makeItem(3, { displayName: 'Account 3' }),
      ]),
    ]

    renderRoster(groups, {
      memberLayout: 'grid',
      selectionMode: 'none',
      onToggleSelected: undefined,
      onToggleSelectAllVisible: undefined,
    })

    const roster = host?.querySelector(
      '[data-testid="upstream-accounts-grouped-roster"]',
    ) as HTMLDivElement | null
    const spacer = host?.querySelector(
      '[data-testid="upstream-accounts-grouped-roster-spacer"]',
    ) as HTMLDivElement | null
    const membersGrid = host?.querySelector(
      '[data-testid="upstream-accounts-group-members-grid"]',
    ) as HTMLDivElement | null
    const gridLayout = membersGrid?.querySelector(
      '[data-testid="upstream-accounts-group-grid-row"]',
    ) as HTMLDivElement | null

    expect(roster).toBeTruthy()
    expect(spacer).toBeTruthy()
    expect(membersGrid).toBeTruthy()
    expect(gridLayout).toBeTruthy()

    Object.defineProperty(window, 'innerWidth', {
      configurable: true,
      value: 1600,
    })
    Object.defineProperty(roster!, 'getBoundingClientRect', {
      configurable: true,
      value: () =>
        ({
          top: 160,
          left: 0,
          right: 1180,
          bottom: 760,
          width: 1180,
          height: 600,
          x: 0,
          y: 160,
          toJSON: () => ({}),
        }) satisfies DOMRect,
    })
    Object.defineProperty(spacer!, 'getBoundingClientRect', {
      configurable: true,
      value: () =>
        ({
          top: 160,
          left: 0,
          right: 1180,
          bottom: 760,
          width: 1180,
          height: 600,
          x: 0,
          y: 160,
          toJSON: () => ({}),
        }) satisfies DOMRect,
    })
    Object.defineProperty(membersGrid!, 'getBoundingClientRect', {
      configurable: true,
      value: () =>
        ({
          top: 210,
          left: 0,
          right: 1180,
          bottom: 560,
          width: 0,
          height: 350,
          x: 0,
          y: 210,
          toJSON: () => ({}),
        }) satisfies DOMRect,
    })

    act(() => {
      window.dispatchEvent(new Event('resize'))
    })

    expect(gridLayout?.style.gridTemplateColumns).toBe('repeat(1, minmax(0, 1fr))')
  })

  it('restores the three-column desktop grouped grid once the member pane is wide enough', () => {
    const groups = [
      makeGroup('analytics', [
        makeItem(1),
        makeItem(2, { displayName: 'Account 2' }),
        makeItem(3, { displayName: 'Account 3' }),
      ]),
    ]

    renderRoster(groups, {
      memberLayout: 'grid',
      selectionMode: 'none',
      onToggleSelected: undefined,
      onToggleSelectAllVisible: undefined,
    })

    const roster = host?.querySelector(
      '[data-testid="upstream-accounts-grouped-roster"]',
    ) as HTMLDivElement | null
    const spacer = host?.querySelector(
      '[data-testid="upstream-accounts-grouped-roster-spacer"]',
    ) as HTMLDivElement | null
    const membersGrid = host?.querySelector(
      '[data-testid="upstream-accounts-group-members-grid"]',
    ) as HTMLDivElement | null
    const gridLayout = membersGrid?.querySelector(
      '[data-testid="upstream-accounts-group-grid-row"]',
    ) as HTMLDivElement | null

    expect(roster).toBeTruthy()
    expect(spacer).toBeTruthy()
    expect(membersGrid).toBeTruthy()
    expect(gridLayout).toBeTruthy()

    Object.defineProperty(window, 'innerWidth', {
      configurable: true,
      value: 1660,
    })
    Object.defineProperty(roster!, 'getBoundingClientRect', {
      configurable: true,
      value: () =>
        ({
          top: 160,
          left: 0,
          right: 1440,
          bottom: 760,
          width: 1440,
          height: 600,
          x: 0,
          y: 160,
          toJSON: () => ({}),
        }) satisfies DOMRect,
    })
    Object.defineProperty(spacer!, 'getBoundingClientRect', {
      configurable: true,
      value: () =>
        ({
          top: 160,
          left: 0,
          right: 1440,
          bottom: 760,
          width: 1440,
          height: 600,
          x: 0,
          y: 160,
          toJSON: () => ({}),
        }) satisfies DOMRect,
    })
    Object.defineProperty(membersGrid!, 'getBoundingClientRect', {
      configurable: true,
      value: () =>
        ({
          top: 210,
          left: 0,
          right: 1048,
          bottom: 560,
          width: 1048,
          height: 350,
          x: 0,
          y: 210,
          toJSON: () => ({}),
        }) satisfies DOMRect,
    })

    act(() => {
      window.dispatchEvent(new Event('resize'))
    })

    expect(gridLayout?.style.gridTemplateColumns).toBe('repeat(3, minmax(0, 1fr))')
  })

  it('keeps the bottom spacer sized to the remaining virtualized groups below the viewport', () => {
    virtualizerMocks.visibleIndexes = [1, 2]
    const groups = Array.from({ length: 6 }, (_, index) =>
      makeGroup(`group-${index + 1}`, [
        makeItem(index + 1, {
          groupName: `group-${index + 1}`,
          displayName: `Group ${index + 1} Account`,
        }),
      ]),
    )

    renderRoster(groups)

    const roster = host?.querySelector(
      '[data-testid="upstream-accounts-grouped-roster"]',
    ) as HTMLDivElement | null
    const spacer = host?.querySelector(
      '[data-testid="upstream-accounts-grouped-roster-spacer"]',
    ) as HTMLDivElement | null
    expect(roster).toBeTruthy()
    expect(spacer).toBeTruthy()

    Object.defineProperty(window, 'scrollY', {
      configurable: true,
      value: 300,
    })
    Object.defineProperty(window, 'innerWidth', {
      configurable: true,
      value: 1440,
    })
    Object.defineProperty(roster!, 'getBoundingClientRect', {
      configurable: true,
      value: () =>
        ({
          top: 240,
          left: 0,
          right: 1200,
          bottom: 900,
          width: 1200,
          height: 660,
          x: 0,
          y: 240,
          toJSON: () => ({}),
        }) satisfies DOMRect,
    })
    Object.defineProperty(spacer!, 'getBoundingClientRect', {
      configurable: true,
      value: () =>
        ({
          top: 288,
          left: 0,
          right: 1200,
          bottom: 900,
          width: 1200,
          height: 612,
          x: 0,
          y: 288,
          toJSON: () => ({}),
        }) satisfies DOMRect,
    })

    const initialMeasureCalls = outerVirtualizerMetrics().measureCalls

    act(() => {
      window.dispatchEvent(new Event('resize'))
    })

    const expectedPaddingBottom = outerVirtualizerMetrics().sizes
      .slice(3)
      .reduce((sum, size) => sum + size, 0)

    expect(outerVirtualizerMetrics().lastScrollMargin).toBe(588)
    expect(outerVirtualizerMetrics().measureCalls).toBeGreaterThan(initialMeasureCalls)
    expect(spacer?.style.paddingBottom).toBe(`${expectedPaddingBottom}px`)
  })

  it('keeps the virtualized card gap on the last rendered non-terminal group', () => {
    virtualizerMocks.visibleIndexes = [1, 2]
    const groups = Array.from({ length: 6 }, (_, index) =>
      makeGroup(`group-${index + 1}`, [
        makeItem(index + 1, {
          groupName: `group-${index + 1}`,
          displayName: `Group ${index + 1} Account`,
        }),
      ]),
    )

    renderRoster(groups)

    const renderedGroupCards = host?.querySelectorAll(
      '[data-testid="upstream-accounts-group-card"]',
    ) ?? []
    const lastRenderedCard = renderedGroupCards[renderedGroupCards.length - 1] as HTMLElement | undefined

    expect(renderedGroupCards).toHaveLength(2)
    expect(lastRenderedCard?.dataset.index).toBe('2')
    expect(lastRenderedCard?.className).toContain('pb-4')
  })

  it('includes inter-card gaps in the fallback spacer height when the virtualizer has not returned items yet', () => {
    virtualizerMocks.visibleIndexes = []
    Object.defineProperty(window, 'scrollY', {
      configurable: true,
      value: 0,
    })
    const groups = Array.from({ length: 6 }, (_, index) =>
      makeGroup(`group-${index + 1}`, [
        makeItem(index + 1, {
          groupName: `group-${index + 1}`,
          displayName: `Group ${index + 1} Account`,
        }),
      ]),
    )

    renderRoster(groups)

    const spacer = host?.querySelector(
      '[data-testid="upstream-accounts-grouped-roster-spacer"]',
    ) as HTMLDivElement | null

    expect(spacer).toBeTruthy()

    const expectedPaddingBottom = outerVirtualizerMetrics().sizes
      .slice(3)
      .reduce((sum, size) => sum + size, 0)

    expect(spacer?.style.paddingBottom).toBe(`${expectedPaddingBottom}px`)
  })

  it('re-measures cached group heights when the layout mode changes', () => {
    const groups = Array.from({ length: 4 }, (_, index) =>
      makeGroup(`group-${index + 1}`, [
        makeItem(index + 1, {
          groupName: `group-${index + 1}`,
        }),
      ]),
    )

    renderRoster(groups, { memberLayout: 'list' })
    const initialMeasureCalls = outerVirtualizerMetrics().measureCalls

    act(() => {
      root?.render(
        <UpstreamAccountsGroupedRoster
          {...createRosterProps(groups, {
            memberLayout: 'grid',
            selectionMode: 'none',
            onToggleSelected: undefined,
            onToggleSelectAllVisible: undefined,
          })}
        />,
      )
    })

    expect(outerVirtualizerMetrics().measureCalls).toBeGreaterThan(initialMeasureCalls)
  })

  it('recomputes the window scroll margin when grouped toolbar chrome toggles', () => {
    const groups = Array.from({ length: 3 }, (_, index) =>
      makeGroup(`group-${index + 1}`, [
        makeItem(index + 1, {
          groupName: `group-${index + 1}`,
        }),
      ]),
    )

    renderRoster(groups, { memberLayout: 'list' })

    const roster = host?.querySelector(
      '[data-testid="upstream-accounts-grouped-roster"]',
    ) as HTMLDivElement | null
    const spacer = host?.querySelector(
      '[data-testid="upstream-accounts-grouped-roster-spacer"]',
    ) as HTMLDivElement | null

    expect(roster).toBeTruthy()
    expect(spacer).toBeTruthy()

    let rosterTop = 220
    let spacerTop = 220
    Object.defineProperty(window, 'scrollY', {
      configurable: true,
      value: 40,
    })
    Object.defineProperty(roster!, 'getBoundingClientRect', {
      configurable: true,
      value: () =>
        ({
          top: rosterTop,
          left: 0,
          right: 1100,
          bottom: rosterTop + 640,
          width: 1100,
          height: 640,
          x: 0,
          y: rosterTop,
          toJSON: () => ({}),
        }) satisfies DOMRect,
    })
    Object.defineProperty(spacer!, 'getBoundingClientRect', {
      configurable: true,
      value: () =>
        ({
          top: spacerTop,
          left: 0,
          right: 1100,
          bottom: spacerTop + 600,
          width: 1100,
          height: 600,
          x: 0,
          y: spacerTop,
          toJSON: () => ({}),
        }) satisfies DOMRect,
    })

    act(() => {
      window.dispatchEvent(new Event('resize'))
    })

    expect(outerVirtualizerMetrics().lastScrollMargin).toBe(260)

    rosterTop = 172
    spacerTop = 172

    act(() => {
      root?.render(
        <UpstreamAccountsGroupedRoster
          {...createRosterProps(groups, {
            memberLayout: 'grid',
            selectionMode: 'none',
            onToggleSelected: undefined,
            onToggleSelectAllVisible: undefined,
          })}
        />,
      )
    })

    expect(outerVirtualizerMetrics().lastScrollMargin).toBe(212)
  })
})
