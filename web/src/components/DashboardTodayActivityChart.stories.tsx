import type { Meta, StoryObj } from '@storybook/react-vite'
import { I18nProvider } from '../i18n'
import { DashboardTodayActivityChart } from './DashboardTodayActivityChart'

const sampleResponse = {
  rangeStart: '2026-04-08T00:00:00+08:00',
  rangeEnd: '2026-04-08T12:24:00+08:00',
  bucketSeconds: 60,
  points: Array.from({ length: 140 }, (_, index) => {
    const bucketStart = new Date('2026-04-08T00:00:00+08:00')
    bucketStart.setMinutes(bucketStart.getMinutes() + index * 5)
    const bucketEnd = new Date(bucketStart.getTime() + 60_000)
    const totalCount = index % 7 === 0 ? 0 : (index % 5) + 1
    const failureCount = totalCount > 0 && index % 6 === 0 ? 1 : 0
    const successCount = Math.max(totalCount - failureCount, 0)
    return {
      bucketStart: bucketStart.toISOString(),
      bucketEnd: bucketEnd.toISOString(),
      totalCount,
      successCount,
      failureCount,
      totalTokens: totalCount * 380,
      totalCost: Number((totalCount * 0.018).toFixed(4)),
    }
  }),
}

const meta = {
  title: 'Dashboard/DashboardTodayActivityChart',
  component: DashboardTodayActivityChart,
  tags: ['autodocs'],
  parameters: {
    layout: 'fullscreen',
  },
  decorators: [
    (Story) => (
      <I18nProvider>
        <div className="min-h-screen bg-base-200 px-6 py-6 text-base-content">
          <div className="mx-auto w-full max-w-[1560px]">
            <Story />
          </div>
        </div>
      </I18nProvider>
    ),
  ],
} satisfies Meta<typeof DashboardTodayActivityChart>

export default meta

type Story = StoryObj<typeof meta>

export const CountBars: Story = {
  args: {
    response: sampleResponse,
    loading: false,
    error: null,
    metric: 'totalCount',
  },
}

export const CostCumulative: Story = {
  args: {
    response: sampleResponse,
    loading: false,
    error: null,
    metric: 'totalCost',
  },
}

export const TokensCumulative: Story = {
  args: {
    response: sampleResponse,
    loading: false,
    error: null,
    metric: 'totalTokens',
  },
}

export const EmptyState: Story = {
  args: {
    response: {
      rangeStart: '2026-04-08T00:00:00+08:00',
      rangeEnd: '2026-04-08T00:00:00+08:00',
      bucketSeconds: 60,
      points: [],
    },
    loading: false,
    error: null,
    metric: 'totalCount',
  },
}
