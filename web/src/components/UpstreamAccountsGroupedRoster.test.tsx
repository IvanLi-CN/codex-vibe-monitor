/** @vitest-environment jsdom */
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, describe, expect, it, vi } from 'vitest'

vi.mock('@tanstack/react-virtual', () => ({
  useVirtualizer: ({ count, estimateSize, gap = 0 }: { count: number; estimateSize: () => number; gap?: number }) => {
    const size = estimateSize()
    return {
      getVirtualItems: () =>
        Array.from({ length: count }, (_, index) => ({
          index,
          key: index,
          start: index * (size + gap),
          size,
        })),
      getTotalSize: () => count * size + Math.max(0, count - 1) * gap,
      measureElement: () => undefined,
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

function makeGroup(items: UpstreamAccountSummary[]): UpstreamAccountsGroupedRosterGroup {
  return {
    id: 'analytics',
    groupName: 'analytics',
    displayName: 'analytics',
    items,
    note: 'This note should not render in grouped list mode.',
    boundProxyLabels: [],
    concurrencyLimit: 2,
    nodeShuntEnabled: false,
    planCounts: [{ key: 'api', label: 'API', count: items.length }],
  }
}

let host: HTMLDivElement | null = null
let root: Root | null = null

afterEach(() => {
  act(() => {
    root?.unmount()
  })
  host?.remove()
  host = null
  root = null
})

function renderRoster(groups: UpstreamAccountsGroupedRosterGroup[]) {
  host = document.createElement('div')
  document.body.appendChild(host)
  root = createRoot(host)
  act(() => {
    root?.render(
      <UpstreamAccountsGroupedRoster
        groups={groups}
        selectedId={null}
        selectedAccountIds={new Set<number>()}
        onSelect={() => undefined}
        onToggleSelected={() => undefined}
        onToggleSelectAllVisible={() => undefined}
        emptyTitle="Empty"
        emptyDescription="Nothing here"
        labels={labels}
        groupLabels={groupLabels}
      />,
    )
  })
}

describe('UpstreamAccountsGroupedRoster', () => {
  it('keeps grouped list minimum height aligned with the flattened row density', () => {
    renderRoster([makeGroup([makeItem(1), makeItem(2)])])

    const members = host?.querySelector('[data-testid="upstream-accounts-group-members"]') as HTMLElement | null
    expect(members).toBeTruthy()
    expect(members?.style.minHeight).toBe('216px')
  })

  it('shrinks single-account groups to a single compact row and omits the group note', () => {
    renderRoster([makeGroup([makeItem(1)])])

    const members = host?.querySelector('[data-testid="upstream-accounts-group-members"]') as HTMLElement | null
    expect(members).toBeTruthy()
    expect(members?.style.minHeight).toBe('104px')
    expect(host?.textContent).not.toContain('This note should not render in grouped list mode.')
  })
})
