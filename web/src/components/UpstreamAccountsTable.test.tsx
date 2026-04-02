/** @vitest-environment jsdom */
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { renderToStaticMarkup } from 'react-dom/server'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type {
  AccountTagSummary,
  EffectiveRoutingRule,
  UpstreamAccountSummary,
} from '../lib/api'
import { UpstreamAccountsTable } from './UpstreamAccountsTable'

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
  { id: 1, name: 'vip', routingRule: defaultEffectiveRoutingRule },
  { id: 2, name: 'burst-safe', routingRule: defaultEffectiveRoutingRule },
  { id: 3, name: 'prod-apac', routingRule: defaultEffectiveRoutingRule },
  { id: 4, name: 'sticky-pool', routingRule: defaultEffectiveRoutingRule },
  { id: 5, name: 'rotating', routingRule: defaultEffectiveRoutingRule },
]

function actualUsage(
  requestCount: number,
  totalTokens = requestCount * 1200,
  totalCost = Number((requestCount * 0.006).toFixed(4)),
) {
  const cacheInputTokens = Math.round(totalTokens * 0.12)
  const inputTokens = Math.round(totalTokens * 0.56)
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

const labels = {
  selectPage: 'Select current page',
  selectRow: (name: string) => `Select ${name}`,
  account: 'Account',
  sync: 'Sync / Call',
  lastSuccess: 'Sync',
  lastCall: 'Call',
  latestAction: 'Latest',
  windows: 'Windows',
  never: 'Never',
  primary: '5h',
  primaryShort: '5h',
  secondary: '7d',
  secondaryShort: '7d',
  nextReset: 'Reset',
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
  hiddenTagsA11y: (count: number, names: string) =>
    `Show ${count} hidden tags: ${names}`,
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
    ({
      enabled: 'Enabled',
      disabled: 'Disabled',
    })[status] ?? status,
  healthStatus: (status: string) =>
    ({
      normal: 'Normal',
      needs_reauth: 'Needs reauth',
      upstream_unavailable: 'Upstream unavailable',
      upstream_rejected: 'Upstream rejected',
      error_other: 'Other error',
      error: 'Error',
    })[status] ?? status,
  syncState: (status: string) =>
    ({
      idle: 'Sync idle',
      syncing: 'Syncing',
    })[status] ?? status,
  action: (action?: string | null) =>
    ({
      route_hard_unavailable: 'Hard unavailable',
      route_retryable_failure: 'Temporary upstream failure',
      route_cooldown_started: 'Route cooldown',
      sync_failed: 'Sync failed',
      sync_recovery_blocked: 'Recovery blocked',
    })[action ?? ''] ??
    action ??
    null,
  compactSupport: (item: UpstreamAccountSummary) =>
    item.compactSupport?.status === 'supported'
      ? 'Compact available'
      : item.compactSupport?.status === 'unsupported'
        ? 'Compact unsupported'
        : null,
  compactSupportHint: (item: UpstreamAccountSummary) => item.compactSupport?.reason ?? null,
  actionSource: (source?: string | null) =>
    ({
      call: 'Call',
      sync_maintenance: 'Maintenance sync',
    })[source ?? ''] ??
    source ??
    null,
  actionReason: (reason?: string | null) =>
    ({
      upstream_http_402: 'Plan or billing rejected',
      upstream_http_429_quota_exhausted: 'Weekly cap exhausted',
      upstream_server_overloaded: 'Upstream is temporarily overloaded',
      quota_still_exhausted: 'Still exhausted',
      reauth_required: 'Needs reauth',
    })[reason ?? ''] ??
    reason ??
    null,
  latestActionFieldAction: 'Action',
  latestActionFieldSource: 'Source',
  latestActionFieldReason: 'Reason',
  latestActionFieldHttpStatus: 'HTTP',
  latestActionFieldOccurredAt: 'Occurred',
  latestActionFieldMessage: 'Message',
}

function renderTable(items: UpstreamAccountSummary[]) {
  return renderToStaticMarkup(
    <UpstreamAccountsTable
      items={items}
      selectedId={items[0]?.id ?? null}
      selectedAccountIds={new Set()}
      onSelect={() => undefined}
      onToggleSelected={() => undefined}
      onToggleSelectAllCurrentPage={() => undefined}
      emptyTitle="Empty"
      emptyDescription="Nothing here"
      labels={labels}
    />,
  )
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

function renderInteractiveTable(
  items: UpstreamAccountSummary[],
  onSelect = vi.fn(),
) {
  host = document.createElement('div')
  document.body.appendChild(host)
  root = createRoot(host)
  act(() => {
    root?.render(
      <UpstreamAccountsTable
        items={items}
        selectedId={items[0]?.id ?? null}
        selectedAccountIds={new Set()}
        onSelect={onSelect}
        onToggleSelected={() => undefined}
        onToggleSelectAllCurrentPage={() => undefined}
        emptyTitle="Empty"
        emptyDescription="Nothing here"
        labels={labels}
      />,
    )
  })
  return onSelect
}

describe('UpstreamAccountsTable', () => {
  it('renders a blocking loading shell when the current query is still pending', () => {
    const html = renderToStaticMarkup(
      <UpstreamAccountsTable
        items={[]}
        isLoading
        loadingTitle="Loading roster"
        loadingDescription="Waiting for the current query"
        selectedId={null}
        selectedAccountIds={new Set()}
        onSelect={() => undefined}
        onToggleSelected={() => undefined}
        onToggleSelectAllCurrentPage={() => undefined}
        emptyTitle="Empty"
        emptyDescription="Nothing here"
        labels={labels}
      />,
    )

    expect(html).toContain('Loading roster')
    expect(html).toContain('Waiting for the current query')
    expect(html).not.toContain('Nothing here')
  })

  it('keeps rendered rows in place and adds a blocking overlay while loading a new query', () => {
    const html = renderToStaticMarkup(
      <UpstreamAccountsTable
        items={[
          {
            id: 11,
            kind: 'oauth_codex',
            provider: 'codex',
            displayName: 'Existing OAuth',
            isMother: false,
            groupName: 'prod',
            status: 'active',
            displayStatus: 'active',
            enabled: true,
            tags: [],
            effectiveRoutingRule: defaultEffectiveRoutingRule,
          },
        ]}
        isLoading
        loadingTitle="Loading roster"
        loadingDescription="Waiting for the current query"
        selectedId={11}
        selectedAccountIds={new Set()}
        onSelect={() => undefined}
        onToggleSelected={() => undefined}
        onToggleSelectAllCurrentPage={() => undefined}
        emptyTitle="Empty"
        emptyDescription="Nothing here"
        labels={labels}
      />,
    )

    expect(html).toContain('Existing OAuth')
    expect(html).toContain('data-testid="upstream-accounts-table-loading-overlay"')
    expect(html).toContain('data-testid="upstream-accounts-table-loading-indicator"')
    expect(html).toContain('Loading roster')
    expect(html).toContain('Waiting for the current query')
    expect(html).toContain('aria-busy="true"')
  })

  it('renders an inline retry state instead of the empty roster when the current query fails', () => {
    const html = renderToStaticMarkup(
      <UpstreamAccountsTable
        items={[]}
        error="Query failed"
        errorTitle="Could not load this roster query"
        retryLabel="Retry"
        onRetry={() => undefined}
        selectedId={null}
        selectedAccountIds={new Set()}
        onSelect={() => undefined}
        onToggleSelected={() => undefined}
        onToggleSelectAllCurrentPage={() => undefined}
        emptyTitle="Empty"
        emptyDescription="Nothing here"
        labels={labels}
      />,
    )

    expect(html).toContain('Could not load this roster query')
    expect(html).toContain('Query failed')
    expect(html).toContain('Retry')
    expect(html).not.toContain('Nothing here')
  })

  it('renders the compact roster layout with a shared windows column and folded tags', () => {
    const html = renderTable([
      {
        id: 11,
        kind: 'oauth_codex',
        provider: 'codex',
        displayName:
          'Codex Pro - Tokyo enterprise rotation account with a deliberately long roster title',
        groupName: 'production-apac-primary-operators',
        isMother: true,
        status: 'active',
        displayStatus: 'active',
        enabled: true,
        enableStatus: 'enabled',
        workStatus: 'working',
        healthStatus: 'normal',
        syncState: 'idle',
        planType: 'team',
        lastSuccessfulSyncAt: '2026-03-16T01:55:00.000Z',
        lastActivityAt: '2026-03-16T02:05:00.000Z',
        activeConversationCount: 3,
        lastAction: 'route_hard_unavailable',
        lastActionSource: 'call',
        lastActionReasonCode: 'upstream_http_429_quota_exhausted',
        lastActionReasonMessage: 'Weekly cap exhausted for this account',
        lastActionHttpStatus: 429,
        lastActionAt: '2026-03-16T02:06:00.000Z',
        primaryWindow: {
          usedPercent: 42,
          usedText: '42 requests',
          limitText: '120 requests',
          resetsAt: '2026-03-16T06:55:00.000Z',
          windowDurationMins: 300,
          actualUsage: actualUsage(42),
        },
        secondaryWindow: {
          usedPercent: 12,
          usedText: '12 requests',
          limitText: '500 requests',
          resetsAt: '2026-03-18T00:00:00.000Z',
          windowDurationMins: 10080,
          actualUsage: actualUsage(12),
        },
        credits: null,
        localLimits: null,
        duplicateInfo: {
          peerAccountIds: [27],
          reasons: ['sharedChatgptUserId'],
        },
        tags,
        effectiveRoutingRule: defaultEffectiveRoutingRule,
      },
    ])

    expect(html).toContain('Windows')
    expect(html).toContain('Sync / Call')
    expect(html).toContain('Latest')
    expect(html).toContain('Hard unavailable')
    expect(html).toContain('Weekly cap exhausted')
    expect(html).toContain('HTTP 429')
    expect(html).toContain('Message: Weekly cap exhausted for this account')
    expect(html).toContain('font-mono tabular-nums')
    expect(html).toContain('Duplicate')
    expect(html).toContain('Mother')
    expect(html).toContain('team')
    expect(html).toContain('data-plan="team"')
    expect(html).toContain('upstream-plan-badge')
    expect(html).toContain('vip')
    expect(html).toContain('burst-safe')
    expect(html).toContain('prod-apac')
    expect(html).toContain('+2')
    expect(html).toContain('title="sticky-pool, rotating"')
    expect(html).toContain(
      'aria-label="Show 2 hidden tags: sticky-pool, rotating"',
    )
    expect(html).toContain('5H')
    expect(html).toContain('7D')
    expect(html).toContain('Req')
    expect(html).toContain('Token')
    expect(html).toContain('Cost')
    expect(html).toContain('grid-cols-[max-content,minmax(0,1fr)]')
    expect(html).not.toContain('production-apac-primary-operators')
    expect(html).toContain('overflow-x-auto')
    expect(html).toContain('md:overflow-x-visible')
    expect(html).toContain('md:min-w-0')
  })

  it('uses the actual window duration labels when a slot returns non-standard window data', () => {
    const html = renderTable([
      {
        id: 21,
        kind: 'oauth_codex',
        provider: 'codex',
        displayName: 'Unexpected window account',
        groupName: null,
        isMother: false,
        status: 'active',
        displayStatus: 'active',
        enabled: true,
        enableStatus: 'enabled',
        workStatus: 'idle',
        healthStatus: 'normal',
        syncState: 'idle',
        lastSuccessfulSyncAt: null,
        lastActivityAt: null,
        primaryWindow: {
          usedPercent: 47,
          usedText: '47%',
          limitText: '12h quota',
          resetsAt: '2026-03-31T00:08:00.000Z',
          windowDurationMins: 720,
          actualUsage: actualUsage(17),
        },
        secondaryWindow: {
          usedPercent: 0,
          usedText: '0%',
          limitText: '7d quota',
          resetsAt: '2026-04-07T00:00:00.000Z',
          windowDurationMins: 10080,
          actualUsage: actualUsage(0, 0, 0),
        },
        credits: null,
        localLimits: null,
        duplicateInfo: null,
        tags: [],
        effectiveRoutingRule: defaultEffectiveRoutingRule,
      },
    ])

    expect(html).toContain('12H')
    expect(html).toContain('title="12h quota')
  })

  it('keeps disabled and placeholder values in the compact layout', () => {
    const html = renderTable([
      {
        id: 12,
        kind: 'api_key_codex',
        provider: 'codex',
        displayName: 'Fallback API key',
        groupName: null,
        isMother: false,
        status: 'disabled',
        displayStatus: 'disabled',
        enabled: false,
        enableStatus: 'disabled',
        workStatus: 'idle',
        healthStatus: 'normal',
        syncState: 'idle',
        planType: null,
        lastSuccessfulSyncAt: null,
        lastActivityAt: null,
        primaryWindow: null,
        secondaryWindow: null,
        credits: null,
        localLimits: null,
        duplicateInfo: null,
        tags: [],
        effectiveRoutingRule: defaultEffectiveRoutingRule,
      },
    ])

    expect(html).toContain('Fallback API key')
    expect(html).toContain('Disabled')
    expect(html).not.toContain('>Idle<')
    expect(html).toContain('Never')
    expect(html).toContain('truncate whitespace-nowrap')
  })

  it('renders missing secondary windows with weak ASCII placeholders instead of 0%', () => {
    const html = renderTable([
      {
        id: 18,
        kind: 'api_key_codex',
        provider: 'codex',
        displayName: 'Missing weekly limit key',
        groupName: null,
        isMother: false,
        status: 'active',
        displayStatus: 'active',
        enabled: true,
        enableStatus: 'enabled',
        workStatus: 'idle',
        healthStatus: 'normal',
        syncState: 'idle',
        planType: 'local',
        lastSuccessfulSyncAt: '2026-03-16T01:55:00.000Z',
        lastActivityAt: '2026-03-16T02:05:00.000Z',
        primaryWindow: {
          usedPercent: 18,
          usedText: '18 requests',
          limitText: '120 requests',
          resetsAt: '2026-03-16T06:55:00.000Z',
          windowDurationMins: 300,
          actualUsage: actualUsage(18),
        },
        secondaryWindow: null,
        credits: null,
        localLimits: {
          primaryLimit: 120,
          secondaryLimit: null,
          limitUnit: 'requests',
        },
        duplicateInfo: null,
        tags: [],
        effectiveRoutingRule: defaultEffectiveRoutingRule,
      },
    ])

    expect(html).toContain('Missing weekly limit key')
    expect(html).toContain('Req')
    expect(html).toContain('>18<')
    expect((html.match(/>-</g) ?? []).length).toBe(5)
    expect(html).toContain('text-base-content/55')
    expect(html).toContain('min-w-[2ch]')
    expect(html).not.toContain('>7D<')
    expect(html).not.toContain('>0%</span>')
    expect(html).not.toContain('>—<')
  })

  it('keeps the secondary label when the weekly limit still exists but the snapshot is missing', () => {
    const html = renderTable([
      {
        id: 19,
        kind: 'api_key_codex',
        provider: 'codex',
        displayName: 'Missing weekly snapshot',
        groupName: null,
        isMother: false,
        status: 'active',
        displayStatus: 'active',
        enabled: true,
        enableStatus: 'enabled',
        workStatus: 'idle',
        healthStatus: 'normal',
        syncState: 'idle',
        planType: 'local',
        lastSuccessfulSyncAt: '2026-03-16T01:55:00.000Z',
        lastActivityAt: '2026-03-16T02:05:00.000Z',
        primaryWindow: {
          usedPercent: 18,
          usedText: '18 requests',
          limitText: '120 requests',
          resetsAt: '2026-03-16T06:55:00.000Z',
          windowDurationMins: 300,
          actualUsage: actualUsage(18),
        },
        secondaryWindow: null,
        credits: null,
        localLimits: {
          primaryLimit: 120,
          secondaryLimit: 500,
          limitUnit: 'requests',
        },
        duplicateInfo: null,
        tags: [],
        effectiveRoutingRule: defaultEffectiveRoutingRule,
      },
    ])

    expect(html).toContain('Missing weekly snapshot')
    expect(html).toContain('>7D<')
    expect((html.match(/>-</g) ?? []).length).toBe(5)
  })

  it('renders counted working badges and keeps the rate-limited exception visible', () => {
    const html = renderTable([
      {
        id: 12,
        kind: 'oauth_codex',
        provider: 'codex',
        displayName: 'Working account',
        groupName: null,
        isMother: false,
        status: 'active',
        displayStatus: 'active',
        enabled: true,
        enableStatus: 'enabled',
        workStatus: 'working',
        healthStatus: 'normal',
        syncState: 'idle',
        activeConversationCount: 3,
        planType: null,
        lastSuccessfulSyncAt: null,
        lastActivityAt: null,
        primaryWindow: null,
        secondaryWindow: null,
        credits: null,
        localLimits: null,
        duplicateInfo: null,
        tags: [],
        effectiveRoutingRule: defaultEffectiveRoutingRule,
      },
      {
        id: 13,
        kind: 'api_key_codex',
        provider: 'codex',
        displayName: 'Idle account',
        groupName: null,
        isMother: false,
        status: 'active',
        displayStatus: 'active',
        enabled: true,
        enableStatus: 'enabled',
        workStatus: 'idle',
        healthStatus: 'normal',
        syncState: 'idle',
        activeConversationCount: 0,
        planType: null,
        lastSuccessfulSyncAt: null,
        lastActivityAt: null,
        primaryWindow: null,
        secondaryWindow: null,
        credits: null,
        localLimits: null,
        duplicateInfo: null,
        tags: [],
        effectiveRoutingRule: defaultEffectiveRoutingRule,
      },
      {
        id: 13,
        kind: 'api_key_codex',
        provider: 'codex',
        displayName: 'Degraded account',
        groupName: null,
        isMother: false,
        status: 'active',
        displayStatus: 'active',
        enabled: true,
        enableStatus: 'enabled',
        workStatus: 'degraded',
        healthStatus: 'normal',
        syncState: 'idle',
        activeConversationCount: 1,
        planType: null,
        lastSuccessfulSyncAt: null,
        lastActivityAt: null,
        primaryWindow: null,
        secondaryWindow: null,
        credits: null,
        localLimits: null,
        duplicateInfo: null,
        tags: [],
        effectiveRoutingRule: defaultEffectiveRoutingRule,
      },
      {
        id: 16,
        kind: 'api_key_codex',
        provider: 'codex',
        displayName: 'Rate-limited account',
        groupName: null,
        isMother: false,
        status: 'active',
        displayStatus: 'active',
        enabled: true,
        enableStatus: 'enabled',
        workStatus: 'rate_limited',
        healthStatus: 'normal',
        syncState: 'idle',
        activeConversationCount: 5,
        planType: null,
        lastSuccessfulSyncAt: null,
        lastActivityAt: null,
        primaryWindow: null,
        secondaryWindow: null,
        credits: null,
        localLimits: null,
        duplicateInfo: null,
        tags: [],
        effectiveRoutingRule: defaultEffectiveRoutingRule,
      },
      {
        id: 14,
        kind: 'api_key_codex',
        provider: 'codex',
        displayName: 'Needs Reauth Key',
        groupName: null,
        isMother: false,
        status: 'needs_reauth',
        displayStatus: 'needs_reauth',
        enabled: true,
        enableStatus: 'enabled',
        workStatus: 'rate_limited',
        healthStatus: 'needs_reauth',
        syncState: 'idle',
        activeConversationCount: 2,
        planType: null,
        lastSuccessfulSyncAt: null,
        lastActivityAt: null,
        primaryWindow: null,
        secondaryWindow: null,
        credits: null,
        localLimits: null,
        duplicateInfo: null,
        tags: [],
        effectiveRoutingRule: defaultEffectiveRoutingRule,
      },
      {
        id: 15,
        kind: 'oauth_codex',
        provider: 'codex',
        displayName: 'Syncing OAuth',
        groupName: null,
        isMother: false,
        status: 'syncing',
        displayStatus: 'syncing',
        enabled: true,
        enableStatus: 'enabled',
        workStatus: 'rate_limited',
        healthStatus: 'normal',
        syncState: 'syncing',
        activeConversationCount: 1,
        planType: null,
        lastSuccessfulSyncAt: null,
        lastActivityAt: null,
        primaryWindow: null,
        secondaryWindow: null,
        credits: null,
        localLimits: null,
        duplicateInfo: null,
        tags: [],
        effectiveRoutingRule: defaultEffectiveRoutingRule,
      },
    ])

    expect(html).toContain('Working 3')
    expect((html.match(/>Degraded</g) ?? []).length).toBe(1)
    expect(html).toContain('>Idle<')
    expect((html.match(/>Rate limited</g) ?? []).length).toBe(1)
    expect(html).toContain('Needs reauth')
    expect(html).toContain('Syncing')
    expect((html.match(/>Idle</g) ?? []).length).toBe(1)
  })

  it('shows wrapped oauth quota exhaustion rows as rate limited instead of upstream rejected', () => {
    const html = renderTable([
      {
        id: 18,
        kind: 'oauth_codex',
        provider: 'codex',
        displayName: 'Quota exhausted OAuth routing state',
        groupName: 'production',
        isMother: false,
        status: 'error',
        displayStatus: 'active',
        enabled: true,
        enableStatus: 'enabled',
        workStatus: 'rate_limited',
        healthStatus: 'normal',
        syncState: 'idle',
        planType: 'team',
        lastError:
          'oauth_upstream_rejected_request: pool upstream responded with 429: The usage limit has been reached',
        lastErrorAt: '2026-03-25T00:31:43.000Z',
        lastAction: 'sync_recovery_blocked',
        lastActionSource: 'sync_maintenance',
        lastActionReasonCode: 'quota_still_exhausted',
        lastActionReasonMessage:
          'latest usage snapshot still shows an exhausted upstream usage limit window',
        lastActionAt: '2026-03-25T02:00:27.000Z',
        primaryWindow: {
          usedPercent: 100,
          usedText: '100% used',
          limitText: '5h rolling window',
          resetsAt: '2026-03-31T00:06:33.000Z',
          windowDurationMins: 300,
          actualUsage: actualUsage(31),
        },
        secondaryWindow: {
          usedPercent: 64,
          usedText: '64% used',
          limitText: '7d rolling window',
          resetsAt: '2026-04-01T00:06:33.000Z',
          windowDurationMins: 10080,
          actualUsage: actualUsage(119),
        },
        credits: null,
        localLimits: null,
        duplicateInfo: null,
        tags: [],
        effectiveRoutingRule: defaultEffectiveRoutingRule,
      },
    ])

    expect(html).toContain('Rate limited')
    expect(html).not.toContain('>Upstream rejected<')
    expect(html).toContain('Recovery blocked')
    expect(html).toContain('Still exhausted')
    expect(html).toContain(
      'latest usage snapshot still shows an exhausted upstream usage limit window',
    )
  })

  it('shows upstream 402 hard-unavailable rows as upstream rejected while preserving the raw context', () => {
    const html = renderTable([
      {
        id: 19,
        kind: 'oauth_codex',
        provider: 'codex',
        displayName: 'Workspace deactivated 402 routing state',
        groupName: 'production',
        isMother: false,
        status: 'error',
        displayStatus: 'upstream_rejected',
        enabled: true,
        enableStatus: 'enabled',
        workStatus: 'unavailable',
        healthStatus: 'upstream_rejected',
        syncState: 'idle',
        planType: 'team',
        lastError:
          'initial usage snapshot attempt with configured user agent failed: usage endpoint returned 402 Payment Required: {"detail":{"code":"deactivated_workspace"}}',
        lastErrorAt: '2026-03-26T08:11:47.000Z',
        lastAction: 'sync_hard_unavailable',
        lastActionSource: 'sync_maintenance',
        lastActionReasonCode: 'upstream_http_402',
        lastActionReasonMessage:
          'initial usage snapshot attempt with configured user agent failed: usage endpoint returned 402 Payment Required: {"detail":{"code":"deactivated_workspace"}}',
        lastActionHttpStatus: 402,
        lastActionAt: '2026-03-26T08:11:47.000Z',
        primaryWindow: {
          usedPercent: 38,
          usedText: '38% used',
          limitText: '5h rolling window',
          resetsAt: '2026-03-26T11:59:42.000Z',
          windowDurationMins: 300,
          actualUsage: actualUsage(15),
        },
        secondaryWindow: {
          usedPercent: 19,
          usedText: '19% used',
          limitText: '7d rolling window',
          resetsAt: '2026-04-02T00:00:00.000Z',
          windowDurationMins: 10080,
          actualUsage: actualUsage(56),
        },
        credits: null,
        localLimits: null,
        duplicateInfo: null,
        tags: [],
        effectiveRoutingRule: defaultEffectiveRoutingRule,
      },
    ])

    expect(html).toContain('Upstream rejected')
    expect(html).not.toContain('>Other error<')
    expect(html).toContain('Plan or billing rejected')
    expect(html).toContain('HTTP 402')
    expect(html).toContain('deactivated_workspace')
  })

  it('falls back to the plain working label when the active conversation count is missing', () => {
    const html = renderTable([
      {
        id: 17,
        kind: 'oauth_codex',
        provider: 'codex',
        displayName: 'Working fallback account',
        groupName: null,
        isMother: false,
        status: 'active',
        displayStatus: 'active',
        enabled: true,
        enableStatus: 'enabled',
        workStatus: 'working',
        healthStatus: 'normal',
        syncState: 'idle',
        activeConversationCount: 0,
        planType: null,
        lastSuccessfulSyncAt: null,
        lastActivityAt: null,
        primaryWindow: null,
        secondaryWindow: null,
        credits: null,
        localLimits: null,
        duplicateInfo: null,
        tags: [],
        effectiveRoutingRule: defaultEffectiveRoutingRule,
      },
    ])

    expect(html).toContain('>Working<')
    expect(html).not.toContain('Working 0')
  })

  it('keeps the folded tags trigger inside the row click target', () => {
    const onSelect = renderInteractiveTable([
      {
        id: 11,
        kind: 'oauth_codex',
        provider: 'codex',
        displayName:
          'Codex Pro - Tokyo enterprise rotation account with a deliberately long roster title',
        groupName: 'production-apac-primary-operators',
        isMother: true,
        status: 'active',
        displayStatus: 'active',
        enabled: true,
        enableStatus: 'enabled',
        workStatus: 'working',
        healthStatus: 'normal',
        syncState: 'idle',
        planType: 'team',
        lastSuccessfulSyncAt: '2026-03-16T01:55:00.000Z',
        lastActivityAt: '2026-03-16T02:05:00.000Z',
        primaryWindow: {
          usedPercent: 42,
          usedText: '42 requests',
          limitText: '120 requests',
          resetsAt: '2026-03-16T06:55:00.000Z',
          windowDurationMins: 300,
          actualUsage: actualUsage(42),
        },
        secondaryWindow: {
          usedPercent: 12,
          usedText: '12 requests',
          limitText: '500 requests',
          resetsAt: '2026-03-18T00:00:00.000Z',
          windowDurationMins: 10080,
          actualUsage: actualUsage(12),
        },
        credits: null,
        localLimits: null,
        duplicateInfo: {
          peerAccountIds: [27],
          reasons: ['sharedChatgptUserId'],
        },
        tags,
        effectiveRoutingRule: defaultEffectiveRoutingRule,
      },
    ])

    const trigger = document.body.querySelector(
      '[aria-label="Show 2 hidden tags: sticky-pool, rotating"]',
    )
    if (!(trigger instanceof HTMLElement)) {
      throw new Error('missing folded tags trigger')
    }

    act(() => {
      trigger.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    })
    expect(onSelect).toHaveBeenCalledWith(11)

    onSelect.mockClear()
    act(() => {
      trigger.dispatchEvent(
        new KeyboardEvent('keydown', { bubbles: true, key: 'Enter' }),
      )
    })
    expect(onSelect).toHaveBeenCalledWith(11)
  })

  it('attaches compact-support and folded-tag titles directly to badge elements', () => {
    renderInteractiveTable([
      {
        id: 11,
        kind: 'oauth_codex',
        provider: 'codex',
        displayName: 'Codex Pro - Tokyo enterprise rotation account with a deliberately long roster title',
        groupName: 'production-apac-primary-operators',
        isMother: true,
        status: 'active',
        displayStatus: 'active',
        enabled: true,
        enableStatus: 'enabled',
        workStatus: 'working',
        healthStatus: 'normal',
        syncState: 'idle',
        planType: 'team',
        compactSupport: {
          status: 'unsupported',
          observedAt: '2026-03-16T02:06:00.000Z',
          reason: 'Compact preview channel unavailable',
        },
        lastSuccessfulSyncAt: '2026-03-16T01:55:00.000Z',
        lastActivityAt: '2026-03-16T02:05:00.000Z',
        primaryWindow: {
          usedPercent: 42,
          usedText: '42 requests',
          limitText: '120 requests',
          resetsAt: '2026-03-16T06:55:00.000Z',
          windowDurationMins: 300,
          actualUsage: actualUsage(42),
        },
        secondaryWindow: {
          usedPercent: 12,
          usedText: '12 requests',
          limitText: '500 requests',
          resetsAt: '2026-03-18T00:00:00.000Z',
          windowDurationMins: 10080,
          actualUsage: actualUsage(12),
        },
        credits: null,
        localLimits: null,
        duplicateInfo: null,
        tags,
        effectiveRoutingRule: defaultEffectiveRoutingRule,
      },
    ])

    const badges = Array.from(
      document.body.querySelectorAll<HTMLElement>('div.inline-flex.items-center.rounded-full.border'),
    )
    const compactSupportBadge = badges.find((node) => node.textContent?.trim() === 'Compact unsupported')
    const overflowBadge = badges.find((node) => node.textContent?.trim() === '+2')

    expect(compactSupportBadge?.getAttribute('title')).toBe('Compact preview channel unavailable')
    expect(overflowBadge?.getAttribute('title')).toBe('sticky-pool, rotating')
    expect(
      document.body.querySelector('span[title="Compact preview channel unavailable"]'),
    ).toBeNull()
    expect(document.body.querySelector('span[title="sticky-pool, rotating"]')).toBeNull()
  })

  it('omits compact-supported badges from the roster chips', () => {
    const html = renderTable([
      {
        id: 12,
        kind: 'api_key_codex',
        provider: 'codex',
        displayName: 'Team key - staging',
        groupName: 'staging',
        isMother: false,
        status: 'active',
        displayStatus: 'active',
        enabled: true,
        enableStatus: 'enabled',
        workStatus: 'idle',
        healthStatus: 'normal',
        syncState: 'idle',
        compactSupport: {
          status: 'supported',
          observedAt: '2026-03-16T02:06:00.000Z',
          reason: 'Observed success for /v1/responses/compact.',
        },
        planType: 'local',
        lastSuccessfulSyncAt: '2026-03-16T01:55:00.000Z',
        lastActivityAt: '2026-03-16T02:05:00.000Z',
        primaryWindow: {
          usedPercent: 0,
          usedText: '0 requests',
          limitText: '120 requests',
          resetsAt: '2026-03-16T06:55:00.000Z',
          windowDurationMins: 300,
          actualUsage: actualUsage(0, 0, 0),
        },
        secondaryWindow: {
          usedPercent: 0,
          usedText: '0 requests',
          limitText: '500 requests',
          resetsAt: '2026-03-18T00:00:00.000Z',
          windowDurationMins: 10080,
          actualUsage: actualUsage(0, 0, 0),
        },
        credits: null,
        localLimits: null,
        duplicateInfo: null,
        tags: [],
        effectiveRoutingRule: defaultEffectiveRoutingRule,
      },
    ])

    expect(html).not.toContain('Compact available')
  })

  it('shows the token breakdown tooltip on focus', async () => {
    renderInteractiveTable([
      {
        id: 27,
        kind: 'oauth_codex',
        provider: 'codex',
        displayName: 'Tooltip account',
        groupName: null,
        isMother: false,
        status: 'active',
        displayStatus: 'active',
        enabled: true,
        enableStatus: 'enabled',
        workStatus: 'working',
        healthStatus: 'normal',
        syncState: 'idle',
        primaryWindow: {
          usedPercent: 42,
          usedText: '42 requests',
          limitText: '120 requests',
          resetsAt: '2026-03-16T06:55:00.000Z',
          windowDurationMins: 300,
          actualUsage: actualUsage(42),
        },
        secondaryWindow: null,
        credits: null,
        localLimits: null,
        duplicateInfo: null,
        tags: [],
        effectiveRoutingRule: defaultEffectiveRoutingRule,
      },
    ])

    const trigger = Array.from(document.body.querySelectorAll<HTMLElement>('[aria-label]')).find((node) =>
      node.getAttribute('aria-label')?.startsWith('Token:'),
    )
    if (!(trigger instanceof HTMLElement)) {
      throw new Error('missing token metric trigger')
    }

    act(() => {
      trigger.focus()
    })

    await expect.poll(() => document.body.textContent ?? '').toContain('Input')
    expect(document.body.textContent).toContain('Input')
    expect(document.body.textContent).toContain('Output')
    expect(document.body.textContent).toContain('Cached input')
    expect(document.body.textContent).toContain('28,224')
    expect(document.body.textContent).toContain('16,128')
    expect(document.body.textContent).toContain('6,048')
  })
})
