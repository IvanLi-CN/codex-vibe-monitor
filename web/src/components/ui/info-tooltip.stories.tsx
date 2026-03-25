import { useEffect, type ReactNode } from 'react'
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

function ThemeRoot({ theme, children }: { theme: 'vibe-light' | 'vibe-dark'; children: ReactNode }) {
  useEffect(() => {
    const previousTheme = document.documentElement.getAttribute('data-theme')
    document.documentElement.setAttribute('data-theme', theme)
    return () => {
      if (previousTheme) {
        document.documentElement.setAttribute('data-theme', previousTheme)
      } else {
        document.documentElement.removeAttribute('data-theme')
      }
    }
  }, [theme])

  return <div data-theme={theme}>{children}</div>
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

export const SharedSurfaceNoiseCard: Story = {
  args: {
    label: 'Help',
    content:
      'This story is the quick visual check that InfoTooltip still matches the shared frosted overlay surface on a noisy background.',
  },
  render: (args) => (
    <ThemeRoot theme="vibe-dark">
      <div
        className="rounded-[1.75rem] border border-base-300/65 px-5 py-6"
        style={{
          backgroundImage:
            'radial-gradient(circle at 12% 0%, rgba(56,189,248,0.18), transparent 34%), radial-gradient(circle at 84% 12%, rgba(45,212,191,0.16), transparent 28%)',
        }}
      >
        <div className="overflow-hidden rounded-[1.2rem] border border-base-300/35 bg-base-100/12">
          {Array.from({ length: 3 }, (_, index) => (
            <div
              key={index}
              className="grid grid-cols-[1.1fr_0.8fr_1fr] items-center gap-3 border-b border-base-300/25 px-4 py-3 text-sm text-base-content/72 last:border-b-0"
            >
              <span className="truncate">{`latest batch checkpoint ${index + 1}`}</span>
              <span className="font-mono">{`03/25 17:1${index}`}</span>
              <span className="truncate">{`sync restored · ${index + 2} minutes ago`}</span>
            </div>
          ))}
        </div>
        <div className="mt-4 inline-flex items-center gap-2 rounded-full border border-info/35 bg-info/10 px-3 py-2 text-xs font-semibold text-info">
          <span>Overlay surface check</span>
          <InfoTooltip {...args} />
        </div>
      </div>
    </ThemeRoot>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const body = within(canvasElement.ownerDocument.body)
    await userEvent.click(canvas.getByRole('button', { name: /help/i }))
    await expect(body.getByRole('tooltip')).toBeInTheDocument()
  },
}
