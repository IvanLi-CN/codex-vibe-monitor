import type { Meta, StoryObj } from '@storybook/react-vite'
import { useState } from 'react'
import { AccountTagContextChip } from './AccountTagContextChip'

const labels = {
  selectedFromCurrentPage: 'New',
  remove: 'Unlink tag',
  deleteAndRemove: 'Delete and unlink',
  edit: 'Edit routing rule',
  hoverHint: 'Hover or long-press to open the tag actions.',
}

function StorySurface({ children }: { children: React.ReactNode }) {
  return (
    <div className="min-h-screen bg-base-200 px-6 py-8 text-base-content">
      <div className="mx-auto flex max-w-3xl flex-wrap gap-4 rounded-[1.6rem] border border-dashed border-base-300/70 bg-base-100/45 p-6">
        {children}
      </div>
    </div>
  )
}

function ChipHarness({
  currentPageCreated = false,
  defaultOpen = false,
}: {
  currentPageCreated?: boolean
  defaultOpen?: boolean
}) {
  const [lastAction, setLastAction] = useState('None')

  return (
    <StorySurface>
      <div className="space-y-4">
        <AccountTagContextChip
          name="vip-routing"
          currentPageCreated={currentPageCreated}
          defaultOpen={defaultOpen}
          labels={labels}
          onRemove={() => setLastAction(currentPageCreated ? 'delete-and-unlink' : 'unlink')}
          onEdit={() => setLastAction('edit')}
        />
        <div className="rounded-xl border border-base-300/70 bg-base-100/80 px-4 py-3 text-sm text-base-content/70">
          Last action: <span className="font-mono text-base-content">{lastAction}</span>
        </div>
      </div>
    </StorySurface>
  )
}

const meta = {
  title: 'Account Pool/Components/Account Tag Context Chip',
  component: AccountTagContextChip,
  tags: ['autodocs'],
  parameters: {
    layout: 'fullscreen',
    docs: {
      description: {
        component:
          '上游账号 tag 的固有交互芯片：桌面端悬浮可见菜单，移动端长按可见菜单；菜单内支持反关联/删除和编辑。',
      },
    },
  },
  args: {
    name: 'vip-routing',
    currentPageCreated: false,
    labels,
    onRemove: () => undefined,
    onEdit: () => undefined,
  },
} satisfies Meta<typeof AccountTagContextChip>

export default meta

type Story = StoryObj<typeof meta>

export const Default: Story = {
  render: () => <ChipHarness />,
}

export const CurrentPageCreated: Story = {
  render: () => <ChipHarness currentPageCreated />,
}

export const MenuVisible: Story = {
  render: () => <ChipHarness defaultOpen />,
}
