import type { Meta, StoryObj } from '@storybook/react-vite'
import { useState } from 'react'
import { AccountTagContextChip } from './AccountTagContextChip'

const labels = {
  selectedFromCurrentPage: 'New',
  remove: 'Unlink tag',
  deleteAndRemove: 'Delete and unlink',
  edit: 'Edit routing rule',
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
  defaultShowActionButton = false,
}: {
  currentPageCreated?: boolean
  defaultOpen?: boolean
  defaultShowActionButton?: boolean
}) {
  const [lastAction, setLastAction] = useState('None')

  return (
    <StorySurface>
      <div className="space-y-4">
        <AccountTagContextChip
          name="vip-routing"
          currentPageCreated={currentPageCreated}
          defaultOpen={defaultOpen}
          defaultShowActionButton={defaultShowActionButton}
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

function InteractionGridHarness() {
  return (
    <StorySurface>
      <div className="grid gap-4 md:grid-cols-3">
        {[
          { title: 'Default', props: {} },
          { title: 'Action Visible', props: { defaultShowActionButton: true } },
          { title: 'Menu Open', props: { defaultShowActionButton: true, defaultOpen: true } },
        ].map((item) => (
          <div key={item.title} className="rounded-[1.2rem] border border-base-300/70 bg-base-100/82 p-4">
            <div className="mb-3 text-xs font-semibold uppercase tracking-[0.14em] text-base-content/55">
              {item.title}
            </div>
            <AccountTagContextChip
              name="vip-routing"
              labels={labels}
              onRemove={() => undefined}
              onEdit={() => undefined}
              {...item.props}
            />
          </div>
        ))}
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
          '上游账号 tag 的固有交互芯片：桌面端悬浮后在标签内部右侧显示三点按钮，点击三点再打开上下文菜单；小宽度/移动端不显示这个三点图标，改为长按打开菜单。',
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

export const ActionButtonVisible: Story = {
  render: () => <ChipHarness defaultShowActionButton />,
}

export const MenuVisible: Story = {
  render: () => <ChipHarness defaultOpen defaultShowActionButton />,
}

export const InteractionGrid: Story = {
  render: () => <InteractionGridHarness />,
}
