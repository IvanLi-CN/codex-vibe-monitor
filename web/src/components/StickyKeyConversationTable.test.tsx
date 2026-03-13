import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { I18nProvider } from '../i18n'
import type { UpstreamStickyConversationsResponse } from '../lib/api'
import { StickyKeyConversationTable } from './StickyKeyConversationTable'

function renderTable(stats: UpstreamStickyConversationsResponse) {
  return renderToStaticMarkup(
    <I18nProvider>
      <StickyKeyConversationTable stats={stats} isLoading={false} error={null} />
    </I18nProvider>,
  )
}

describe('StickyKeyConversationTable', () => {
  it('renders sticky key metrics and the shared 24h sparkline', () => {
    const stats: UpstreamStickyConversationsResponse = {
      rangeStart: '2026-03-02T00:00:00Z',
      rangeEnd: '2026-03-03T00:00:00Z',
      conversations: [
        {
          stickyKey: 'sticky-chat-001',
          requestCount: 8,
          totalTokens: 2048,
          totalCost: 0.3456,
          createdAt: '2026-03-02T01:00:00Z',
          lastActivityAt: '2026-03-02T12:00:00Z',
          last24hRequests: [
            {
              occurredAt: '2026-03-02T01:00:00Z',
              status: 'success',
              isSuccess: true,
              requestTokens: 256,
              cumulativeTokens: 256,
            },
            {
              occurredAt: '2026-03-02T10:00:00Z',
              status: 'failed',
              isSuccess: false,
              requestTokens: 128,
              cumulativeTokens: 384,
            },
          ],
        },
      ],
    }

    const html = renderTable(stats)

    expect(html).toContain('sticky-chat-001')
    expect(html).toContain('Sticky Key')
    expect(html).toContain('data-chart-kind="keyed-conversation-sparkline"')
    expect(html).toContain('aria-label="sticky-chat-001 24 小时 Token 累计图"')
  })
})
