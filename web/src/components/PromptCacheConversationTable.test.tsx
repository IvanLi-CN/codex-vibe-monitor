import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { I18nProvider } from '../i18n'
import type { PromptCacheConversationsResponse } from '../lib/api'
import { PromptCacheConversationTable } from './PromptCacheConversationTable'

function renderTable(stats: PromptCacheConversationsResponse) {
  return renderToStaticMarkup(
    <I18nProvider>
      <PromptCacheConversationTable stats={stats} isLoading={false} error={null} />
    </I18nProvider>,
  )
}

describe('PromptCacheConversationTable', () => {
  it('renders conversation metrics and 24h chart segments', () => {
    const stats: PromptCacheConversationsResponse = {
      rangeStart: '2026-03-02T00:00:00Z',
      rangeEnd: '2026-03-03T00:00:00Z',
      conversations: [
        {
          promptCacheKey: 'pck-chat-001',
          requestCount: 12,
          totalTokens: 3456,
          totalCost: 1.2345,
          createdAt: '2026-02-24T11:00:00Z',
          lastActivityAt: '2026-03-02T16:00:00Z',
          last24hRequests: [
            {
              occurredAt: '2026-03-02T10:00:00Z',
              status: 'success',
              isSuccess: true,
              requestTokens: 120,
              cumulativeTokens: 120,
            },
            {
              occurredAt: '2026-03-02T12:00:00Z',
              status: 'failed',
              isSuccess: false,
              requestTokens: 80,
              cumulativeTokens: 200,
            },
          ],
        },
      ],
    }

    const html = renderTable(stats)

    expect(html).toContain('pck-chat-001')
    expect(html).toContain('Prompt Cache Key')
    expect(html).toContain('24h Token 累计')
    expect(html).toContain('sm:hidden')
    expect(html).toContain('sm:table')
    expect(html).toContain('stroke="oklch(var(--color-success) / 0.95)"')
    expect(html).toContain('stroke="oklch(var(--color-error) / 0.92)"')
  })

  it('shares the 24h token chart scale across visible conversations', () => {
    const stats: PromptCacheConversationsResponse = {
      rangeStart: '2026-03-02T00:00:00Z',
      rangeEnd: '2026-03-03T00:00:00Z',
      conversations: [
        {
          promptCacheKey: 'pck-low',
          requestCount: 1,
          totalTokens: 50,
          totalCost: 0.01,
          createdAt: '2026-03-02T01:00:00Z',
          lastActivityAt: '2026-03-02T01:00:00Z',
          last24hRequests: [
            {
              occurredAt: '2026-03-02T01:00:00Z',
              status: 'success',
              isSuccess: true,
              requestTokens: 50,
              cumulativeTokens: 50,
            },
          ],
        },
        {
          promptCacheKey: 'pck-high',
          requestCount: 1,
          totalTokens: 100,
          totalCost: 0.02,
          createdAt: '2026-03-02T02:00:00Z',
          lastActivityAt: '2026-03-02T02:00:00Z',
          last24hRequests: [
            {
              occurredAt: '2026-03-02T02:00:00Z',
              status: 'success',
              isSuccess: true,
              requestTokens: 100,
              cumulativeTokens: 100,
            },
          ],
        },
      ],
    }

    const html = renderTable(stats)

    expect(html).toContain('aria-label="pck-low"')
    expect(html).toContain('y1="24"')
  })


  it('ignores malformed timestamps when computing the shared chart scale', () => {
    const stats: PromptCacheConversationsResponse = {
      rangeStart: '2026-03-02T00:00:00Z',
      rangeEnd: '2026-03-03T00:00:00Z',
      conversations: [
        {
          promptCacheKey: 'pck-low-valid',
          requestCount: 1,
          totalTokens: 50,
          totalCost: 0.01,
          createdAt: '2026-03-02T01:00:00Z',
          lastActivityAt: '2026-03-02T01:00:00Z',
          last24hRequests: [
            {
              occurredAt: '2026-03-02T01:00:00Z',
              status: 'success',
              isSuccess: true,
              requestTokens: 50,
              cumulativeTokens: 50,
            },
          ],
        },
        {
          promptCacheKey: 'pck-bad-point',
          requestCount: 2,
          totalTokens: 100,
          totalCost: 0.02,
          createdAt: '2026-03-02T02:00:00Z',
          lastActivityAt: '2026-03-02T02:00:00Z',
          last24hRequests: [
            {
              occurredAt: 'not-a-date',
              status: 'success',
              isSuccess: true,
              requestTokens: 9999,
              cumulativeTokens: 10000,
            },
            {
              occurredAt: '2026-03-02T02:00:00Z',
              status: 'success',
              isSuccess: true,
              requestTokens: 100,
              cumulativeTokens: 100,
            },
          ],
        },
      ],
    }

    const html = renderTable(stats)

    expect(html).toContain('aria-label="pck-low-valid"')
    expect(html).toContain('y1="24"')
  })


  it('renders empty state when there are no conversations', () => {
    const stats: PromptCacheConversationsResponse = {
      rangeStart: '2026-03-02T00:00:00Z',
      rangeEnd: '2026-03-03T00:00:00Z',
      conversations: [],
    }

    const html = renderTable(stats)

    expect(html).toContain('暂无 Prompt Cache Key 对话数据。')
  })
})
