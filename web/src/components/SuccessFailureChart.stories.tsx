import type { Meta, StoryObj } from '@storybook/react-vite'
import { I18nProvider } from '../i18n'
import type { TimeseriesPoint } from '../lib/api'
import { SuccessFailureChart } from './SuccessFailureChart'

const FIRST_RESPONSE_BYTE_TOTAL_POINTS: TimeseriesPoint[] = [
  {
    bucketStart: '2026-03-26T10:00:00.000Z',
    bucketEnd: '2026-03-26T10:15:00.000Z',
    totalCount: 168,
    successCount: 166,
    failureCount: 2,
    totalTokens: 184_620,
    totalCost: 0.0412,
    firstByteSampleCount: 166,
    firstByteAvgMs: 81.7,
    firstByteP95Ms: 110.5,
    firstResponseByteTotalSampleCount: 166,
    firstResponseByteTotalAvgMs: 29_840,
    firstResponseByteTotalP95Ms: 31_200,
  },
  {
    bucketStart: '2026-03-26T10:15:00.000Z',
    bucketEnd: '2026-03-26T10:30:00.000Z',
    totalCount: 154,
    successCount: 151,
    failureCount: 3,
    totalTokens: 176_210,
    totalCost: 0.0384,
    firstByteSampleCount: 151,
    firstByteAvgMs: 77.1,
    firstByteP95Ms: 101.3,
    firstResponseByteTotalSampleCount: 151,
    firstResponseByteTotalAvgMs: 31_120,
    firstResponseByteTotalP95Ms: 33_400,
  },
  {
    bucketStart: '2026-03-26T10:30:00.000Z',
    bucketEnd: '2026-03-26T10:45:00.000Z',
    totalCount: 82,
    successCount: 79,
    failureCount: 3,
    totalTokens: 92_580,
    totalCost: 0.0198,
    firstByteSampleCount: 79,
    firstByteAvgMs: 74.3,
    firstByteP95Ms: 98.9,
    firstResponseByteTotalSampleCount: 79,
    firstResponseByteTotalAvgMs: 35_760,
    firstResponseByteTotalP95Ms: 38_240,
  },
  {
    bucketStart: '2026-03-26T10:45:00.000Z',
    bucketEnd: '2026-03-26T11:00:00.000Z',
    totalCount: 112,
    successCount: 109,
    failureCount: 3,
    totalTokens: 141_930,
    totalCost: 0.0316,
    firstByteSampleCount: 109,
    firstByteAvgMs: 79.2,
    firstByteP95Ms: 103.1,
    firstResponseByteTotalSampleCount: 109,
    firstResponseByteTotalAvgMs: 43_890,
    firstResponseByteTotalP95Ms: 52_340,
  },
  {
    bucketStart: '2026-03-26T11:00:00.000Z',
    bucketEnd: '2026-03-26T11:15:00.000Z',
    totalCount: 166,
    successCount: 163,
    failureCount: 3,
    totalTokens: 189_110,
    totalCost: 0.0431,
    firstByteSampleCount: 163,
    firstByteAvgMs: 83.6,
    firstByteP95Ms: 112.7,
    firstResponseByteTotalSampleCount: 163,
    firstResponseByteTotalAvgMs: 41_260,
    firstResponseByteTotalP95Ms: 47_900,
  },
  {
    bucketStart: '2026-03-26T11:15:00.000Z',
    bucketEnd: '2026-03-26T11:30:00.000Z',
    totalCount: 149,
    successCount: 145,
    failureCount: 4,
    totalTokens: 171_240,
    totalCost: 0.0395,
    firstByteSampleCount: 145,
    firstByteAvgMs: 76.4,
    firstByteP95Ms: 104.6,
    firstResponseByteTotalSampleCount: 145,
    firstResponseByteTotalAvgMs: 34_480,
    firstResponseByteTotalP95Ms: 39_120,
  },
]

const meta = {
  title: 'Stats/SuccessFailureChart',
  component: SuccessFailureChart,
  parameters: {
    layout: 'fullscreen',
  },
  decorators: [
    (Story) => (
      <I18nProvider>
        <div className="min-h-screen bg-base-200 px-6 py-6 text-base-content">
          <div className="mx-auto w-full max-w-6xl rounded-[28px] border border-base-300/70 bg-base-100/85 p-6 shadow-[0_24px_80px_rgba(15,23,42,0.24)]">
            <Story />
          </div>
        </div>
      </I18nProvider>
    ),
  ],
} satisfies Meta<typeof SuccessFailureChart>

export default meta

type Story = StoryObj<typeof meta>

export const FirstResponseByteTotalP95: Story = {
  args: {
    points: FIRST_RESPONSE_BYTE_TOTAL_POINTS,
    isLoading: false,
    bucketSeconds: 900,
    tooltipDefaultIndex: 3,
    tooltipActive: true,
  },
  parameters: {
    docs: {
      description: {
        story:
          'Stable chart evidence for the corrected stats metric. The highlighted latency line now follows `首字总耗时` semantics, so a bucket can surface `43.89 s` average with a `52.34 s` P95 while success/failure bars remain unchanged.',
      },
    },
  },
}

export const Loading: Story = {
  args: {
    points: [],
    isLoading: true,
    bucketSeconds: 900,
  },
}
