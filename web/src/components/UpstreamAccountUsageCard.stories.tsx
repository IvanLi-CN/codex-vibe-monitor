import type { Meta, StoryObj } from '@storybook/react-vite'
import type { UpstreamAccountHistoryPoint } from '../lib/api'
import { UpstreamAccountUsageCard } from './UpstreamAccountUsageCard'
import { STORYBOOK_COLOR_CONTRAST_TODO } from '../storybook/a11y'

const history: UpstreamAccountHistoryPoint[] = [
  { capturedAt: '2026-03-10T02:00:00.000Z', primaryUsedPercent: 24, secondaryUsedPercent: 10 },
  { capturedAt: '2026-03-10T08:00:00.000Z', primaryUsedPercent: 38, secondaryUsedPercent: 11 },
  { capturedAt: '2026-03-10T14:00:00.000Z', primaryUsedPercent: 46, secondaryUsedPercent: 13 },
  { capturedAt: '2026-03-10T20:00:00.000Z', primaryUsedPercent: 51, secondaryUsedPercent: 15 },
  { capturedAt: '2026-03-11T02:00:00.000Z', primaryUsedPercent: 57, secondaryUsedPercent: 17 },
  { capturedAt: '2026-03-11T08:00:00.000Z', primaryUsedPercent: 63, secondaryUsedPercent: 19 },
  { capturedAt: '2026-03-11T12:00:00.000Z', primaryUsedPercent: 68, secondaryUsedPercent: 20 },
]

const meta = {
  title: 'Account Pool/Components/Upstream Account Usage Card',
  component: UpstreamAccountUsageCard,
  tags: ['autodocs'],
  parameters: {
    ...STORYBOOK_COLOR_CONTRAST_TODO,
    layout: 'fullscreen',
  },
  decorators: [
    (Story) => (
      <div data-theme="light" className="min-h-screen bg-base-200 px-6 py-8 text-base-content">
        <div className="mx-auto max-w-3xl">
          <Story />
        </div>
      </div>
    ),
  ],
} satisfies Meta<typeof UpstreamAccountUsageCard>

export default meta

type Story = StoryObj<typeof meta>

export const PrimaryWindow: Story = {
  args: {
    title: '5h window',
    description: 'Primary quota window aligned with Codex 5-hour usage semantics.',
    window: {
      usedPercent: 68,
      usedText: '68% used',
      limitText: '5h rolling window',
      resetsAt: '2026-03-11T14:00:00.000Z',
      windowDurationMins: 300,
    },
    history,
    historyKey: 'primaryUsedPercent',
    emptyLabel: 'No usage samples yet',
    noteLabel: 'OAuth snapshot',
  },
}

export const PlaceholderApiKeyWindow: Story = {
  args: {
    title: '7d window',
    description: 'Secondary weekly limit for a locally managed API key account.',
    window: {
      usedPercent: 0,
      usedText: '0 requests',
      limitText: '500 requests',
      resetsAt: '2026-03-18T00:00:00.000Z',
      windowDurationMins: 10080,
    },
    history,
    historyKey: 'secondaryUsedPercent',
    emptyLabel: 'No usage samples yet',
    noteLabel: 'Local placeholder',
    accentClassName: 'text-info',
  },
}

export const EmptyHistory: Story = {
  args: {
    title: '5h window',
    description: 'New account without historical samples yet.',
    window: null,
    history: [],
    historyKey: 'primaryUsedPercent',
    emptyLabel: 'No usage samples yet',
  },
}
