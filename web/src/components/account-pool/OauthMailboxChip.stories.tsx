import type { Meta, StoryObj } from '@storybook/react-vite'
import type { ComponentProps, ReactNode } from 'react'
import { expect, userEvent, within } from 'storybook/test'
import { OauthMailboxChip } from './OauthMailboxChip'

function StorySurface({ children }: { children: ReactNode }) {
  return (
    <div className="min-h-screen bg-base-200 px-10 py-12">
      <div className="max-w-xl rounded-2xl border border-base-300/80 bg-base-100 p-6 shadow-sm">
        <div className="flex items-center gap-3">
          <span className="field-label shrink-0">Display Name</span>
          {children}
        </div>
      </div>
    </div>
  )
}

const meta = {
  title: 'Account Pool/Pages/Upstream Account Create/Mailbox Chip',
  component: OauthMailboxChip,
  decorators: [
    (Story) => (
      <StorySurface>
        <Story />
      </StorySurface>
    ),
  ],
  parameters: {
    layout: 'fullscreen',
  },
} satisfies Meta<typeof OauthMailboxChip>

export default meta

type Story = StoryObj<typeof meta>

const baseArgs = {
  className: 'max-w-[24rem]',
  emptyLabel: 'No mailbox yet',
  copyAriaLabel: 'Copy mailbox',
  copyHintLabel: 'Click to copy',
  copiedLabel: 'Copied',
  manualCopyLabel: 'Auto copy failed. Please copy the mailbox below manually.',
  manualBadgeLabel: 'Manual',
  onCopy: () => undefined,
} satisfies Partial<ComponentProps<typeof OauthMailboxChip>>

export const Hover: Story = {
  args: {
    ...baseArgs,
    emailAddress: 'hover-preview@mail-tw.707079.xyz',
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const copyMailboxButton = canvas.getByRole('button', { name: /copy mailbox/i })

    await userEvent.hover(copyMailboxButton)
    const tooltip = within(document.body)
    await expect(tooltip.getByText(/click to copy/i)).toBeInTheDocument()
    await expect(tooltip.getByText(/hover-preview@mail-tw\.707079\.xyz/i)).toBeInTheDocument()
  },
}

export const LongPress: Story = {
  args: {
    ...baseArgs,
    emailAddress: 'press-preview@mail-tw.707079.xyz',
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const copyMailboxButton = canvas.getByRole('button', { name: /copy mailbox/i })

    copyMailboxButton.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, pointerType: 'touch', button: 0 }))
    await new Promise((resolve) => window.setTimeout(resolve, 420))

    const tooltip = within(document.body)
    await expect(tooltip.getByText(/click to copy/i)).toBeInTheDocument()
    await expect(tooltip.getByText(/press-preview@mail-tw\.707079\.xyz/i)).toBeInTheDocument()

    copyMailboxButton.dispatchEvent(new PointerEvent('pointerup', { bubbles: true, pointerType: 'touch', button: 0 }))
  },
}

export const Copied: Story = {
  args: {
    ...baseArgs,
    emailAddress: 'copied-preview@mail-tw.707079.xyz',
    tone: 'copied',
  },
}

export const ManualCopy: Story = {
  args: {
    ...baseArgs,
    emailAddress: 'manual-copy@mail-tw.707079.xyz',
    tone: 'manual',
  },
}
