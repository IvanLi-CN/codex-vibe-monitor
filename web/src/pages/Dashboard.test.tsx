/** @vitest-environment jsdom */
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { MemoryRouter } from 'react-router-dom'
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest'
import type { DashboardWorkingConversationCardModel } from '../lib/dashboardWorkingConversations'
import DashboardPage from './Dashboard'

const hookMocks = vi.hoisted(() => ({
  useDashboardWorkingConversations: vi.fn(),
  useSummary: vi.fn(),
}))

vi.mock('../hooks/useDashboardWorkingConversations', () => ({
  useDashboardWorkingConversations: hookMocks.useDashboardWorkingConversations,
}))

vi.mock('../hooks/useStats', () => ({
  useSummary: hookMocks.useSummary,
}))

vi.mock('../components/TodayStatsOverview', () => ({
  TodayStatsOverview: () => <div data-testid="today-stats-overview" />,
}))

vi.mock('../components/UsageCalendar', () => ({
  UsageCalendar: ({
    metric,
    showSurface,
    showMetricToggle,
  }: {
    metric?: string
    showSurface?: boolean
    showMetricToggle?: boolean
  }) => (
    <div data-testid="usage-calendar">
      {`metric:${metric ?? 'unset'};surface:${String(showSurface)};toggle:${String(showMetricToggle)}`}
    </div>
  ),
}))

vi.mock('../components/StatsCards', () => ({
  StatsCards: ({
    stats,
    loading,
    error,
  }: {
    stats: { totalCount?: number } | null
    loading: boolean
    error?: string | null
  }) => (
    <div data-testid="stats-cards">
      {loading ? 'loading' : error ? `error:${error}` : `total:${stats?.totalCount ?? 0}`}
    </div>
  ),
}))

vi.mock('../components/Last24hTenMinuteHeatmap', () => ({
  Last24hTenMinuteHeatmap: ({
    metric,
    showHeader,
  }: {
    metric?: string
    showHeader?: boolean
  }) => (
    <div data-testid="heatmap-24h">
      {`metric:${metric ?? 'unset'};header:${String(showHeader)}`}
    </div>
  ),
}))

vi.mock('../components/WeeklyHourlyHeatmap', () => ({
  WeeklyHourlyHeatmap: ({
    metric,
    showHeader,
    showSurface,
  }: {
    metric?: string
    showHeader?: boolean
    showSurface?: boolean
  }) => (
    <div data-testid="heatmap-7d">
      {`metric:${metric ?? 'unset'};header:${String(showHeader)};surface:${String(showSurface)}`}
    </div>
  ),
}))

vi.mock('../components/DashboardWorkingConversationsSection', () => ({
  DashboardWorkingConversationsSection: ({
    cards,
    onOpenUpstreamAccount,
    onOpenInvocation,
  }: {
    cards: DashboardWorkingConversationCardModel[]
    onOpenUpstreamAccount?: (accountId: number, accountLabel: string) => void
    onOpenInvocation?: (selection: {
      slotKind: 'current' | 'previous'
      conversationSequenceId: string
      promptCacheKey: string
      invocation: { record: { invokeId: string } }
    }) => void
  }) => (
    <div data-testid="dashboard-working-conversations-section">
      {cards.map((card) => card.conversationSequenceId).join(',')}
      {cards[0] ? (
        <>
          <button
            type="button"
            data-testid="dashboard-open-invocation"
            onClick={() =>
              onOpenInvocation?.({
                slotKind: 'current',
                conversationSequenceId: cards[0].conversationSequenceId,
                promptCacheKey: cards[0].promptCacheKey,
                invocation: cards[0].currentInvocation,
              })
            }
          >
            open invocation
          </button>
          <button
            type="button"
            data-testid="dashboard-open-account"
            onClick={() => onOpenUpstreamAccount?.(77, 'section-account@example.com')}
          >
            open account
          </button>
        </>
      ) : null}
    </div>
  ),
}))

vi.mock('../components/DashboardInvocationDetailDrawer', () => ({
  DashboardInvocationDetailDrawer: ({
    open,
    selection,
    onClose,
    onOpenUpstreamAccount,
  }: {
    open: boolean
    selection: { invocation: { record: { invokeId: string } } } | null
    onClose: () => void
    onOpenUpstreamAccount?: (accountId: number, accountLabel: string) => void
  }) =>
    open ? (
      <div data-testid="dashboard-invocation-detail-drawer-mock">
        <span data-testid="dashboard-invocation-drawer-selection">
          {selection?.invocation.record.invokeId ?? 'none'}
        </span>
        <button type="button" data-testid="dashboard-invocation-drawer-close" onClick={onClose}>
          close invocation drawer
        </button>
        <button
          type="button"
          data-testid="dashboard-invocation-drawer-open-account"
          onClick={() => onOpenUpstreamAccount?.(88, 'drawer-account@example.com')}
        >
          open account from invocation drawer
        </button>
      </div>
    ) : null,
}))

vi.mock('./account-pool/UpstreamAccounts', () => ({
  SharedUpstreamAccountDetailDrawer: ({
    open,
    accountId,
    onClose,
  }: {
    open: boolean
    accountId: number | null
    onClose: () => void
  }) =>
    open ? (
      <div data-testid="shared-upstream-account-detail-drawer-mock">
        <span data-testid="shared-upstream-account-drawer-account-id">{accountId}</span>
        <button type="button" data-testid="shared-upstream-account-drawer-close" onClick={onClose}>
          close account drawer
        </button>
      </div>
    ) : null,
}))

vi.mock('../theme', () => ({
  useTheme: () => ({ themeMode: 'light' }),
}))

vi.mock('../i18n', () => ({
  useTranslation: () => ({
    locale: 'zh',
    t: (key: string) => {
      const map: Record<string, string> = {
        'dashboard.activityOverview.title': '活动总览',
        'dashboard.activityOverview.range24h': '24 小时',
        'dashboard.activityOverview.range7d': '7 日',
        'dashboard.activityOverview.rangeUsage': '历史',
        'dashboard.activityOverview.rangeToggleAria': '时间范围切换',
        'dashboard.today.title': '今日统计信息',
        'dashboard.section.workingConversationsTitle': '当前工作中的对话',
        'heatmap.metricsToggleAria': '指标切换',
        'metric.totalCount': '次数',
        'metric.totalCost': '金额',
        'metric.totalTokens': 'Tokens',
      }
      return map[key] ?? key
    },
  }),
}))

let host: HTMLDivElement | null = null
let root: Root | null = null

beforeAll(() => {
  Object.defineProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT', {
    configurable: true,
    writable: true,
    value: true,
  })
})

afterEach(() => {
  act(() => {
    root?.unmount()
  })
  host?.remove()
  host = null
  root = null
  vi.clearAllMocks()
})

function render(ui: React.ReactNode) {
  host = document.createElement('div')
  document.body.appendChild(host)
  root = createRoot(host)
  act(() => {
    root?.render(<MemoryRouter>{ui}</MemoryRouter>)
  })
}

function installSummaryMocks() {
  hookMocks.useSummary.mockImplementation((window: string) => {
    if (window === 'today') {
      return { summary: { totalCount: 12 }, isLoading: false, error: null }
    }
    if (window === '1d') {
      return { summary: { totalCount: 100 }, isLoading: false, error: null }
    }
    if (window === '7d') {
      return { summary: { totalCount: 700 }, isLoading: false, error: null }
    }
    return { summary: null, isLoading: false, error: null }
  })
}

function createWorkingConversationCard(): DashboardWorkingConversationCardModel {
  return {
    promptCacheKey: 'pck-drawer-switch',
    normalizedPromptCacheKey: 'pck-drawer-switch',
    conversationSequenceId: 'WC-ABCD12',
    currentInvocation: {
      preview: {
        id: 101,
        invokeId: 'invoke-dashboard-current',
        occurredAt: '2026-04-06T10:20:00Z',
        status: 'completed',
        failureClass: null,
        routeMode: 'forward_proxy',
        model: 'gpt-5.4',
        totalTokens: 120,
        cost: 0.01,
        proxyDisplayName: 'tokyo-edge-01',
        upstreamAccountId: 77,
        upstreamAccountName: 'section-account@example.com',
        endpoint: '/v1/responses',
      },
      record: {
        id: 101,
        invokeId: 'invoke-dashboard-current',
        occurredAt: '2026-04-06T10:20:00Z',
        createdAt: '2026-04-06T10:20:00Z',
        status: 'completed',
        source: 'proxy',
        routeMode: 'forward_proxy',
        model: 'gpt-5.4',
        totalTokens: 120,
      },
      displayStatus: 'success',
      occurredAtEpoch: Date.parse('2026-04-06T10:20:00Z'),
      isInFlight: false,
      isTerminal: true,
      tone: 'success',
    },
    previousInvocation: null,
    hasPreviousPlaceholder: true,
    createdAtEpoch: Date.parse('2026-04-06T10:20:00Z'),
    sortAnchorEpoch: Date.parse('2026-04-06T10:20:00Z'),
    lastTerminalAtEpoch: Date.parse('2026-04-06T10:20:00Z'),
    lastInFlightAtEpoch: null,
    tone: 'success',
    requestCount: 1,
    totalTokens: 120,
    totalCost: 0.01,
  }
}

describe('DashboardPage', () => {
  it('keeps usage activity inside the shared overview card instead of as a standalone top card', () => {
    installSummaryMocks()
    hookMocks.useDashboardWorkingConversations.mockReturnValue({
      cards: [createWorkingConversationCard()],
      isLoading: false,
      error: null,
    })

    render(<DashboardPage />)

    expect(host?.textContent).toContain('活动总览')
    expect(host?.querySelectorAll('[data-testid="dashboard-activity-overview"]')).toHaveLength(1)
    expect(host?.querySelector('[data-testid="stats-cards"]')?.textContent).toBe('total:100')
    expect(host?.querySelector('[data-testid="dashboard-activity-range-1d"]')?.getAttribute('data-active')).toBe('true')
    expect(host?.querySelector('[data-testid="usage-calendar"]')).toBeNull()
    expect(host?.querySelector('[data-testid="heatmap-24h"]')?.textContent).toContain('metric:totalCount')
    expect(host?.querySelector('[data-testid="dashboard-working-conversations-section"]')?.textContent).toContain(
      'WC-ABCD12',
    )
    expect(host?.textContent).not.toContain('最近 20 条实况')

    const rangeButtons = host?.querySelectorAll('button[role="tab"]')
    const usageButton = Array.from(rangeButtons ?? []).find(
      (button) => button.textContent === '历史',
    )
    if (!(usageButton instanceof HTMLButtonElement)) {
      throw new Error('missing usage range button')
    }

    act(() => {
      usageButton.click()
    })

    expect(host?.querySelector('[data-testid="stats-cards"]')).toBeNull()
    expect(host?.querySelector('[data-testid="usage-calendar"]')?.textContent).toBe(
      'metric:totalCount;surface:false;toggle:false',
    )

    const range7dButton = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find(
      (button) => button.textContent === '7 日',
    )
    if (!(range7dButton instanceof HTMLButtonElement)) {
      throw new Error('missing 7d range button')
    }

    act(() => {
      range7dButton.click()
    })

    expect(host?.querySelector('[data-testid="stats-cards"]')?.textContent).toBe('total:700')
    expect(host?.querySelector('[data-testid="dashboard-activity-range-7d"]')?.getAttribute('data-active')).toBe('true')
    expect(host?.querySelector('[data-testid="dashboard-activity-range-usage"]')?.getAttribute('data-active')).toBe(
      'false',
    )
    expect(host?.querySelector('[data-testid="usage-calendar"]')?.textContent).toBe(
      'metric:totalCount;surface:false;toggle:false',
    )
    expect(host?.querySelector('[data-testid="heatmap-7d"]')?.textContent).toBe(
      'metric:totalCount;header:false;surface:false',
    )

  })

  it('switches between the invocation drawer and the shared account drawer from dashboard interactions', () => {
    installSummaryMocks()
    hookMocks.useDashboardWorkingConversations.mockReturnValue({
      cards: [createWorkingConversationCard()],
      isLoading: false,
      error: null,
    })

    render(<DashboardPage />)

    const openInvocationButton = host?.querySelector(
      '[data-testid="dashboard-open-invocation"]',
    )
    if (!(openInvocationButton instanceof HTMLButtonElement)) {
      throw new Error('missing invocation trigger')
    }

    act(() => {
      openInvocationButton.click()
    })

    expect(
      host?.querySelector('[data-testid="dashboard-invocation-detail-drawer-mock"]'),
    ).not.toBeNull()
    expect(
      host?.querySelector('[data-testid="dashboard-invocation-drawer-selection"]')?.textContent,
    ).toBe('invoke-dashboard-current')
    expect(
      host?.querySelector('[data-testid="shared-upstream-account-detail-drawer-mock"]'),
    ).toBeNull()

    const openAccountFromSectionButton = host?.querySelector(
      '[data-testid="dashboard-open-account"]',
    )
    if (!(openAccountFromSectionButton instanceof HTMLButtonElement)) {
      throw new Error('missing section account trigger')
    }

    act(() => {
      openAccountFromSectionButton.click()
    })

    expect(
      host?.querySelector('[data-testid="dashboard-invocation-detail-drawer-mock"]'),
    ).toBeNull()
    expect(
      host?.querySelector('[data-testid="shared-upstream-account-drawer-account-id"]')?.textContent,
    ).toBe('77')

    act(() => {
      openInvocationButton.click()
    })

    expect(
      host?.querySelector('[data-testid="shared-upstream-account-detail-drawer-mock"]'),
    ).toBeNull()
    expect(
      host?.querySelector('[data-testid="dashboard-invocation-detail-drawer-mock"]'),
    ).not.toBeNull()

    const openAccountFromInvocationDrawerButton = host?.querySelector(
      '[data-testid="dashboard-invocation-drawer-open-account"]',
    )
    if (!(openAccountFromInvocationDrawerButton instanceof HTMLButtonElement)) {
      throw new Error('missing invocation drawer account trigger')
    }

    act(() => {
      openAccountFromInvocationDrawerButton.click()
    })

    expect(
      host?.querySelector('[data-testid="dashboard-invocation-detail-drawer-mock"]'),
    ).toBeNull()
    expect(
      host?.querySelector('[data-testid="shared-upstream-account-drawer-account-id"]')?.textContent,
    ).toBe('88')
  })
})
