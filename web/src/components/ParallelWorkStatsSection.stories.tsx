import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, within } from 'storybook/test'
import { I18nProvider } from '../i18n'
import type { ParallelWorkStatsResponse, ParallelWorkWindowResponse } from '../lib/api'
import { ParallelWorkStatsSection } from './ParallelWorkStatsSection'

function buildWindow(
  overrides: Partial<ParallelWorkWindowResponse> & {
    rangeStart: string
    rangeEnd: string
    bucketSeconds: number
    completeBucketCount: number
    activeBucketCount: number
    points: ParallelWorkWindowResponse['points']
  },
): ParallelWorkWindowResponse {
  return {
    rangeStart: overrides.rangeStart,
    rangeEnd: overrides.rangeEnd,
    bucketSeconds: overrides.bucketSeconds,
    completeBucketCount: overrides.completeBucketCount,
    activeBucketCount: overrides.activeBucketCount,
    minCount: overrides.minCount ?? 0,
    maxCount: overrides.maxCount ?? 0,
    avgCount: overrides.avgCount ?? 0,
    points: overrides.points,
  }
}

const populatedStats: ParallelWorkStatsResponse = {
  minute7d: buildWindow({
    rangeStart: '2026-03-01T00:00:00Z',
    rangeEnd: '2026-03-08T00:00:00Z',
    bucketSeconds: 60,
    completeBucketCount: 10_080,
    activeBucketCount: 4_132,
    minCount: 0,
    maxCount: 18,
    avgCount: 4.67,
    points: Array.from({ length: 16 }, (_, index) => ({
      bucketStart: new Date(Date.UTC(2026, 2, 7, 10, index)).toISOString(),
      bucketEnd: new Date(Date.UTC(2026, 2, 7, 10, index + 1)).toISOString(),
      parallelCount: [1, 3, 2, 5, 7, 10, 8, 9, 12, 11, 15, 14, 13, 9, 6, 4][index] ?? 0,
    })),
  }),
  hour30d: buildWindow({
    rangeStart: '2026-02-06T00:00:00Z',
    rangeEnd: '2026-03-08T00:00:00Z',
    bucketSeconds: 3600,
    completeBucketCount: 720,
    activeBucketCount: 321,
    minCount: 0,
    maxCount: 9,
    avgCount: 2.13,
    points: Array.from({ length: 12 }, (_, index) => ({
      bucketStart: new Date(Date.UTC(2026, 2, 7, index)).toISOString(),
      bucketEnd: new Date(Date.UTC(2026, 2, 7, index + 1)).toISOString(),
      parallelCount: [0, 1, 2, 2, 3, 4, 6, 5, 4, 3, 2, 1][index] ?? 0,
    })),
  }),
  dayAll: buildWindow({
    rangeStart: '2026-01-01T00:00:00Z',
    rangeEnd: '2026-03-08T00:00:00Z',
    bucketSeconds: 86_400,
    completeBucketCount: 67,
    activeBucketCount: 54,
    minCount: 0,
    maxCount: 6,
    avgCount: 2.04,
    points: Array.from({ length: 10 }, (_, index) => ({
      bucketStart: new Date(Date.UTC(2026, 1, 27 + index)).toISOString(),
      bucketEnd: new Date(Date.UTC(2026, 1, 28 + index)).toISOString(),
      parallelCount: [1, 2, 3, 5, 4, 4, 6, 5, 3, 2][index] ?? 0,
    })),
  }),
}

const emptyDayAllStats: ParallelWorkStatsResponse = {
  ...populatedStats,
  dayAll: buildWindow({
    rangeStart: '2026-03-08T00:00:00Z',
    rangeEnd: '2026-03-08T00:00:00Z',
    bucketSeconds: 86_400,
    completeBucketCount: 0,
    activeBucketCount: 0,
    minCount: null,
    maxCount: null,
    avgCount: null,
    points: [],
  }),
}

const meta = {
  title: 'Stats/ParallelWorkStatsSection',
  component: ParallelWorkStatsSection,
  tags: ['autodocs'],
  parameters: {
    layout: 'fullscreen',
  },
  decorators: [
    (Story) => (
      <I18nProvider>
        <div className="min-h-screen bg-base-200 px-6 py-6 text-base-content">
          <div className="mx-auto w-full max-w-7xl">
            <Story />
          </div>
        </div>
      </I18nProvider>
    ),
  ],
} satisfies Meta<typeof ParallelWorkStatsSection>

export default meta

type Story = StoryObj<typeof meta>

export const Populated: Story = {
  args: {
    stats: populatedStats,
    isLoading: false,
    error: null,
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByTestId('parallel-work-card-minute7d')).toBeInTheDocument()
    await expect(canvas.queryByTestId('parallel-work-card-hour30d')).toBeNull()
    await expect(canvas.queryByTestId('parallel-work-card-dayAll')).toBeNull()
  },
}

export const Hour30dSelected: Story = {
  args: {
    stats: populatedStats,
    isLoading: false,
    error: null,
    defaultWindowKey: 'hour30d',
  },
}

export const DayAllEmpty: Story = {
  args: {
    stats: emptyDayAllStats,
    isLoading: false,
    error: null,
    defaultWindowKey: 'dayAll',
  },
}

export const Loading: Story = {
  args: {
    stats: null,
    isLoading: true,
    error: null,
    defaultWindowKey: 'hour30d',
  },
}

export const LoadError: Story = {
  args: {
    stats: null,
    isLoading: false,
    error: 'Request failed: 400 unsupported timeZone for historical parallel-work rollups',
  },
}

export const Gallery: Story = {
  args: {
    stats: populatedStats,
    isLoading: false,
    error: null,
  },
  render: () => (
    <div className="space-y-6">
      <ParallelWorkStatsSection stats={populatedStats} isLoading={false} error={null} />
      <ParallelWorkStatsSection
        stats={populatedStats}
        isLoading={false}
        error={null}
        defaultWindowKey="hour30d"
      />
      <ParallelWorkStatsSection
        stats={emptyDayAllStats}
        isLoading={false}
        error={null}
        defaultWindowKey="dayAll"
      />
      <ParallelWorkStatsSection stats={null} isLoading={true} error={null} />
      <ParallelWorkStatsSection
        stats={null}
        isLoading={false}
        error="Request failed: 500 unable to load parallel-work stats"
      />
    </div>
  ),
}
