/** @vitest-environment jsdom */
import { act, type ComponentProps } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, describe, expect, it, vi } from 'vitest'

const virtualizerMocks = vi.hoisted(() => ({
  visibleIndexes: null as number[] | null,
  sizes: [] as number[],
  lastScrollMargin: 0,
  measureCalls: 0,
}))

vi.mock('@tanstack/react-virtual', () => ({
  useWindowVirtualizer: ({
    count,
    estimateSize,
    scrollMargin = 0,
  }: {
    count: number
    estimateSize: (index: number) => number
    scrollMargin?: number
  }) => {
    const sizes = Array.from({ length: count }, (_, index) => estimateSize(index))
    virtualizerMocks.sizes = sizes
    virtualizerMocks.lastScrollMargin = scrollMargin

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
        virtualizerMocks.measureCalls += 1
      },
    }
  },
}))

import type {
  AccountTagSummary,
  EffectiveRoutingRule,
  UpstreamAccountSummary,
} from '../lib/api'
import {
  UpstreamAccountsGroupedRoster,
  type UpstreamAccountsGroupedRosterGroup,
} from './UpstreamAccountsGroupedRoster'

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

const tags: AccountTagSummary[] = [
  { id: 1, name: 'analytics', routingRule: defaultEffectiveRoutingRule },
  { id: 2, name: 'reporting', routingRule: defaultEffectiveRoutingRule },
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
  virtualizerMocks.sizes = []
  virtualizerMocks.lastScrollMargin = 0
  virtualizerMocks.measureCalls = 0
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
    const gridLayout = membersGrid?.querySelector(':scope > div') as HTMLDivElement | null

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
    const gridLayout = membersGrid?.querySelector(':scope > div') as HTMLDivElement | null

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

    const initialMeasureCalls = virtualizerMocks.measureCalls

    act(() => {
      window.dispatchEvent(new Event('resize'))
    })

    const expectedPaddingBottom = virtualizerMocks.sizes
      .slice(3)
      .reduce((sum, size) => sum + size, 0)

    expect(virtualizerMocks.lastScrollMargin).toBe(588)
    expect(virtualizerMocks.measureCalls).toBeGreaterThan(initialMeasureCalls)
    expect(spacer?.style.paddingBottom).toBe(`${expectedPaddingBottom}px`)
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

    const expectedPaddingBottom = virtualizerMocks.sizes
      .slice(4)
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
    const initialMeasureCalls = virtualizerMocks.measureCalls

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

    expect(virtualizerMocks.measureCalls).toBeGreaterThan(initialMeasureCalls)
  })
})
