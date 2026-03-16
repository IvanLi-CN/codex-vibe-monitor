import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import type { AccountTagSummary, EffectiveRoutingRule, UpstreamAccountSummary } from '../lib/api'
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

const labels = {
  sync: 'Sync / Call',
  lastSuccess: 'Sync',
  lastCall: 'Call',
  windows: 'Windows',
  never: 'Never',
  primary: '5h',
  primaryShort: '5h',
  secondary: '7d',
  secondaryShort: '7d',
  nextReset: 'Reset',
  oauth: 'OAuth',
  apiKey: 'API key',
  duplicate: 'Duplicate',
  mother: 'Mother',
  off: 'Off',
  status: (value: string) =>
    ({
      active: 'Active',
      syncing: 'Syncing',
      needs_reauth: 'Needs reauth',
      error: 'Error',
      disabled: 'Disabled',
    })[value] ?? value,
}

function renderTable(items: UpstreamAccountSummary[]) {
  return renderToStaticMarkup(
    <UpstreamAccountsTable
      items={items}
      selectedId={items[0]?.id ?? null}
      onSelect={() => undefined}
      emptyTitle="Empty"
      emptyDescription="Nothing here"
      labels={labels}
    />,
  )
}

describe('UpstreamAccountsTable', () => {
  it('renders the compact roster layout with a shared windows column and folded tags', () => {
    const html = renderTable([
      {
        id: 11,
        kind: 'oauth_codex',
        provider: 'codex',
        displayName: 'Codex Pro - Tokyo enterprise rotation account with a deliberately long roster title',
        groupName: 'production-apac-primary-operators',
        isMother: true,
        status: 'active',
        enabled: true,
        planType: 'team',
        lastSuccessfulSyncAt: '2026-03-16T01:55:00.000Z',
        lastActivityAt: '2026-03-16T02:05:00.000Z',
        primaryWindow: {
          usedPercent: 42,
          usedText: '42 requests',
          limitText: '120 requests',
          resetsAt: '2026-03-16T06:55:00.000Z',
          windowDurationMins: 300,
        },
        secondaryWindow: {
          usedPercent: 12,
          usedText: '12 requests',
          limitText: '500 requests',
          resetsAt: '2026-03-18T00:00:00.000Z',
          windowDurationMins: 10080,
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
    expect(html).toContain('Call')
    expect(html).toContain('Duplicate')
    expect(html).toContain('Mother')
    expect(html).toContain('team')
    expect(html).toContain('vip')
    expect(html).toContain('burst-safe')
    expect(html).toContain('prod-apac')
    expect(html).toContain('+2')
    expect(html).toContain('title="sticky-pool, rotating"')
    expect(html).toContain('5h')
    expect(html).toContain('7d')
    expect(html).not.toContain('production-apac-primary-operators')
    expect(html).not.toContain('overflow-x-auto')
    expect(html).not.toContain('min-w-[940px]')
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
        enabled: false,
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
    expect(html).toContain('Off')
    expect(html).toContain('Disabled')
    expect(html).toContain('Never')
    expect(html).toContain('truncate whitespace-nowrap')
  })
})
