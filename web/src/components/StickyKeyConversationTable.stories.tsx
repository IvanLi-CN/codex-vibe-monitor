import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, within } from 'storybook/test'
import { I18nProvider } from '../i18n'
import type { UpstreamStickyConversationsResponse } from '../lib/api'
import { StickyKeyConversationTable } from './StickyKeyConversationTable'
import { StorybookUpstreamAccountsMock } from './UpstreamAccountsPage.story-helpers'

function buildStats(
  overrides: Partial<UpstreamStickyConversationsResponse> = {},
): UpstreamStickyConversationsResponse {
  return {
    rangeStart: '2026-03-12T04:00:00.000Z',
    rangeEnd: '2026-03-13T04:10:00.000Z',
    selectionMode: 'count',
    selectedLimit: 50,
    selectedActivityHours: null,
    implicitFilter: {
      kind: null,
      filteredCount: 0,
    },
    conversations: [
      {
        stickyKey: '019ce3a1-6787-7910-b0fd-c246d6f6a901',
        requestCount: 10,
        totalTokens: 455_170,
        totalCost: 0.3507,
        createdAt: '2026-03-13T04:01:20.000Z',
        lastActivityAt: '2026-03-13T04:03:02.000Z',
        recentInvocations: [
          {
            id: 10101,
            invokeId: 'sticky-101-001',
            occurredAt: '2026-03-13T04:03:02.000Z',
            status: 'completed',
            failureClass: 'none',
            routeMode: 'sticky',
            model: 'gpt-5.4',
            totalTokens: 198_350,
            cost: 0.1984,
            proxyDisplayName: 'Tokyo Edge',
            upstreamAccountId: 101,
            upstreamAccountName: 'Codex Pro - Tokyo',
            endpoint: '/v1/responses',
            source: 'proxy',
            inputTokens: 120_000,
            outputTokens: 62_000,
            cacheInputTokens: 16_350,
            reasoningTokens: 7_500,
            reasoningEffort: 'high',
            responseContentEncoding: 'br',
            requestedServiceTier: 'flex',
            serviceTier: 'scale',
            tReqReadMs: 12,
            tReqParseMs: 14,
            tUpstreamConnectMs: 19,
            tUpstreamTtfbMs: 260,
            tUpstreamStreamMs: 2_140,
            tRespParseMs: 18,
            tPersistMs: 9,
            tTotalMs: 2_472,
          },
        ],
        last24hRequests: [
          {
            occurredAt: '2026-03-12T10:15:00.000Z',
            status: 'success',
            isSuccess: true,
            requestTokens: 102_440,
            cumulativeTokens: 102_440,
          },
          {
            occurredAt: '2026-03-12T18:20:00.000Z',
            status: 'success',
            isSuccess: true,
            requestTokens: 154_380,
            cumulativeTokens: 256_820,
          },
          {
            occurredAt: '2026-03-13T04:03:02.000Z',
            status: 'success',
            isSuccess: true,
            requestTokens: 198_350,
            cumulativeTokens: 455_170,
          },
        ],
      },
      {
        stickyKey: '019ce39a-6cfa-7b90-8e96-6de7e6076b02',
        requestCount: 20,
        totalTokens: 1_289_447,
        totalCost: 0.7022,
        createdAt: '2026-03-13T03:51:19.000Z',
        lastActivityAt: '2026-03-12T22:54:08.000Z',
        recentInvocations: [
          {
            id: 10102,
            invokeId: 'sticky-101-002',
            occurredAt: '2026-03-12T22:54:08.000Z',
            status: 'failed',
            failureClass: 'service_failure',
            routeMode: 'sticky',
            model: 'gpt-5.4',
            totalTokens: 365_000,
            cost: 0.365,
            proxyDisplayName: 'Tokyo Edge',
            upstreamAccountId: 101,
            upstreamAccountName: 'Codex Pro - Tokyo',
            endpoint: '/v1/responses',
            source: 'proxy',
            inputTokens: 221_000,
            outputTokens: 117_000,
            cacheInputTokens: 27_000,
            reasoningTokens: 10_000,
            reasoningEffort: 'xhigh',
            errorMessage: '[pool_no_available_slot] Sticky fallback exhausted.',
            failureKind: 'pool_no_available_slot',
            isActionable: true,
            responseContentEncoding: 'br',
            requestedServiceTier: 'flex',
            serviceTier: 'scale',
            tReqReadMs: 12,
            tReqParseMs: 14,
            tUpstreamConnectMs: 19,
            tUpstreamTtfbMs: 260,
            tUpstreamStreamMs: 2_140,
            tRespParseMs: 18,
            tPersistMs: 9,
            tTotalMs: 2_472,
          },
        ],
        last24hRequests: [
          {
            occurredAt: '2026-03-12T07:52:00.000Z',
            status: 'success',
            isSuccess: true,
            requestTokens: 281_000,
            cumulativeTokens: 281_000,
          },
          {
            occurredAt: '2026-03-12T13:04:00.000Z',
            status: 'success',
            isSuccess: true,
            requestTokens: 309_447,
            cumulativeTokens: 590_447,
          },
          {
            occurredAt: '2026-03-12T22:54:08.000Z',
            status: 'failed',
            isSuccess: false,
            requestTokens: 365_000,
            cumulativeTokens: 955_447,
          },
        ],
      },
    ],
    ...overrides,
  }
}

const meta = {
  title: 'Account Pool/Components/Sticky Key Conversation Table',
  component: StickyKeyConversationTable,
  tags: ['autodocs'],
  decorators: [
    (Story) => (
      <I18nProvider>
        <StorybookUpstreamAccountsMock>
          <div className="min-h-screen bg-base-200 p-6 text-base-content">
            <Story />
          </div>
        </StorybookUpstreamAccountsMock>
      </I18nProvider>
    ),
  ],
  args: {
    accountId: 101,
    isLoading: false,
    error: null,
  },
} satisfies Meta<typeof StickyKeyConversationTable>

export default meta

type Story = StoryObj<typeof meta>

export const CountMode: Story = {
  args: {
    stats: buildStats(),
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(
      canvas.getAllByRole('button', { name: /打开全部调用记录|open full call history/i })[0],
    )
    const documentScope = within(canvasElement.ownerDocument.body)
    await expect(
      documentScope.getByText(/019ce3a1-6787-7910-b0fd-c246d6f6a901/i),
    ).toBeInTheDocument()
    await expect(documentScope.getByText(/gpt-5\.4/i)).toBeInTheDocument()
  },
}

export const ActivityWindowMode: Story = {
  args: {
    stats: buildStats({
      selectionMode: 'activityWindow',
      selectedLimit: null,
      selectedActivityHours: 3,
      implicitFilter: {
        kind: 'cappedTo50',
        filteredCount: 7,
      },
    }),
  },
}
