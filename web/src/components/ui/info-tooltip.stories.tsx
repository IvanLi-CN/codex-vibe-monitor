import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, within } from 'storybook/test'
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
  tags: ['autodocs'],
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

export const ViewportEdgePinned: Story = {
  args: {
    label: 'Help',
    content: 'Pinned tooltips use the shared anchored bubble shell and stay within the viewport even at the edge.',
  },
  render: (args) => (
    <div className="flex min-h-[70vh] items-start justify-end rounded-[1.6rem] border border-base-300/70 bg-base-100/35 px-4 py-6">
      <div className="inline-flex items-center gap-2 rounded-full border border-info/35 bg-info/10 px-3 py-2 text-xs font-semibold text-info">
        <span>边缘提示</span>
        <InfoTooltip {...args} />
      </div>
    </div>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const doc = within(canvasElement.ownerDocument.body)
    const trigger = canvas.getByRole('button', { name: /help/i })
    await userEvent.click(trigger)
    const tooltip = await doc.findByRole('tooltip')
    const rect = tooltip.getBoundingClientRect()
    await expect(trigger.getAttribute('aria-describedby')).toBe(tooltip.id)
    await expect(rect.right <= window.innerWidth - 1).toBe(true)
    await expect(rect.left >= 0).toBe(true)
  },
}
