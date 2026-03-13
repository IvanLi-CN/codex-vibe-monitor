import type { Meta, StoryObj } from '@storybook/react-vite'
import { InfoTooltip } from './info-tooltip'

function StorySurface({ children }: { children: React.ReactNode }) {
  return (
    <div className="min-h-screen bg-base-200 px-6 py-6 text-base-content">
      <div className="mx-auto w-full max-w-xl">{children}</div>
    </div>
  )
}

const meta = {
  title: 'UI/InfoTooltip',
  component: InfoTooltip,
  decorators: [
    (Story) => (
      <StorySurface>
        <Story />
      </StorySurface>
    ),
  ],
} satisfies Meta<typeof InfoTooltip>

export default meta

type Story = StoryObj<typeof meta>

export const Inline: Story = {
  args: {
    label: 'Help',
    content:
      'Current list is based on the latest search snapshot. Paging/sorting/focus will not auto-include new records. Click Search to refresh.',
  },
  render: (args) => (
    <div className="inline-flex items-center gap-2 rounded-full border border-base-300/70 bg-base-100/45 px-3 py-2 text-sm text-base-content/80">
      <span>17 new records</span>
      <InfoTooltip {...args} />
    </div>
  ),
}

export const MatchTextColor: Story = {
  args: {
    label: 'Help',
    content:
      'The help icon uses currentColor so it matches the text color of the surrounding notice.',
  },
  render: (args) => (
    <div className="inline-flex items-center gap-2 rounded-full border border-warning/35 bg-warning/10 px-3 py-2 text-xs font-semibold text-warning">
      <span>有 17 条新数据</span>
      <InfoTooltip {...args} />
    </div>
  ),
}

