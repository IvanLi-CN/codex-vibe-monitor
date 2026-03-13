import type { Meta, StoryObj } from '@storybook/react-vite'
import { I18nProvider } from '../i18n'
import type { PromptCacheConversationsResponse } from '../lib/api'
import { PromptCacheConversationTable } from './PromptCacheConversationTable'
import { STORYBOOK_COLOR_CONTRAST_TODO } from '../storybook/a11y'

const stats: PromptCacheConversationsResponse = {
  rangeStart: '2026-03-02T00:00:00.000Z',
  rangeEnd: '2026-03-03T00:00:00.000Z',
  conversations: [
    {
      promptCacheKey: 'pck-chat-20260303-01',
      requestCount: 41,
      totalTokens: 56124,
      totalCost: 1.2842,
      createdAt: '2026-02-24T03:26:11.000Z',
      lastActivityAt: '2026-03-03T12:44:10.000Z',
      last24hRequests: [
        {
          occurredAt: '2026-03-02T13:00:00.000Z',
          status: 'completed',
          isSuccess: true,
          requestTokens: 980,
          cumulativeTokens: 980,
        },
        {
          occurredAt: '2026-03-02T15:12:00.000Z',
          status: 'completed',
          isSuccess: true,
          requestTokens: 1210,
          cumulativeTokens: 2190,
        },
        {
          occurredAt: '2026-03-02T17:53:00.000Z',
          status: 'upstream_stream_error',
          isSuccess: false,
          requestTokens: 670,
          cumulativeTokens: 2860,
        },
        {
          occurredAt: '2026-03-02T20:40:00.000Z',
          status: 'completed',
          isSuccess: true,
          requestTokens: 1460,
          cumulativeTokens: 4320,
        },
        {
          occurredAt: '2026-03-03T10:44:00.000Z',
          status: 'completed',
          isSuccess: true,
          requestTokens: 1184,
          cumulativeTokens: 5504,
        },
      ],
    },
    {
      promptCacheKey: 'pck-chat-20260303-02',
      requestCount: 16,
      totalTokens: 18209,
      totalCost: 0.4628,
      createdAt: '2026-02-20T08:09:33.000Z',
      lastActivityAt: '2026-03-03T11:40:28.000Z',
      last24hRequests: [
        {
          occurredAt: '2026-03-02T14:16:00.000Z',
          status: 'completed',
          isSuccess: true,
          requestTokens: 742,
          cumulativeTokens: 742,
        },
        {
          occurredAt: '2026-03-02T14:51:00.000Z',
          status: 'invalid_api_key',
          isSuccess: false,
          requestTokens: 56,
          cumulativeTokens: 798,
        },
        {
          occurredAt: '2026-03-02T18:05:00.000Z',
          status: 'completed',
          isSuccess: true,
          requestTokens: 930,
          cumulativeTokens: 1728,
        },
        {
          occurredAt: '2026-03-03T11:40:28.000Z',
          status: 'completed',
          isSuccess: true,
          requestTokens: 804,
          cumulativeTokens: 2532,
        },
      ],
    },
  ],
}



const sharedScaleStats: PromptCacheConversationsResponse = {
  rangeStart: '2026-03-02T00:00:00.000Z',
  rangeEnd: '2026-03-03T00:00:00.000Z',
  conversations: [
    {
      promptCacheKey: 'pck-low-volume',
      requestCount: 3,
      totalTokens: 420,
      totalCost: 0.01,
      createdAt: '2026-03-02T03:00:00.000Z',
      lastActivityAt: '2026-03-02T05:00:00.000Z',
      last24hRequests: [
        {
          occurredAt: '2026-03-02T03:00:00.000Z',
          status: 'completed',
          isSuccess: true,
          requestTokens: 100,
          cumulativeTokens: 100,
        },
        {
          occurredAt: '2026-03-02T05:00:00.000Z',
          status: 'completed',
          isSuccess: true,
          requestTokens: 120,
          cumulativeTokens: 220,
        },
      ],
    },
    {
      promptCacheKey: 'pck-high-volume',
      requestCount: 8,
      totalTokens: 8600,
      totalCost: 0.21,
      createdAt: '2026-03-02T02:30:00.000Z',
      lastActivityAt: '2026-03-02T23:40:00.000Z',
      last24hRequests: [
        {
          occurredAt: '2026-03-02T02:30:00.000Z',
          status: 'completed',
          isSuccess: true,
          requestTokens: 1200,
          cumulativeTokens: 1200,
        },
        {
          occurredAt: '2026-03-02T09:10:00.000Z',
          status: 'completed',
          isSuccess: true,
          requestTokens: 1800,
          cumulativeTokens: 3000,
        },
        {
          occurredAt: '2026-03-02T18:50:00.000Z',
          status: 'upstream_stream_error',
          isSuccess: false,
          requestTokens: 900,
          cumulativeTokens: 3900,
        },
        {
          occurredAt: '2026-03-02T23:40:00.000Z',
          status: 'completed',
          isSuccess: true,
          requestTokens: 2200,
          cumulativeTokens: 6100,
        },
      ],
    },
  ],
}

const meta = {
  title: 'Monitoring/PromptCacheConversationTable',
  component: PromptCacheConversationTable,
  parameters: {
    ...STORYBOOK_COLOR_CONTRAST_TODO,
    layout: 'fullscreen',
  },
  decorators: [
    (Story) => (
      <I18nProvider>
        <div data-theme="light" className="min-h-screen bg-base-200 px-4 py-6 text-base-content sm:px-6">
          <main className="mx-auto w-full max-w-[1200px] space-y-4">
            <h2 className="text-xl font-semibold">Prompt Cache 对话统计（Storybook Mock）</h2>
            <Story />
          </main>
        </div>
      </I18nProvider>
    ),
  ],
} satisfies Meta<typeof PromptCacheConversationTable>

export default meta

type Story = StoryObj<typeof meta>

export const Populated: Story = {
  args: {
    stats,
    isLoading: false,
    error: null,
  },
}

export const Empty: Story = {
  args: {
    stats: {
      rangeStart: stats.rangeStart,
      rangeEnd: stats.rangeEnd,
      conversations: [],
    },
    isLoading: false,
    error: null,
  },
}

export const Loading: Story = {
  args: {
    stats: null,
    isLoading: true,
    error: null,
  },
}

export const ErrorState: Story = {
  args: {
    stats: null,
    isLoading: false,
    error: 'Network error',
  },
}

export const SharedScaleComparison: Story = {
  args: {
    stats: sharedScaleStats,
    isLoading: false,
    error: null,
  },
}

export const TooltipEdgeDensity: Story = {
  args: {
    stats,
    isLoading: false,
    error: null,
  },
  parameters: {
    docs: {
      description: {
        story:
          'Hover or tap the final token segment to verify the shared tooltip flips inward near the right table edge without clipping.',
      },
    },
  },
}
