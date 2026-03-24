import type { Meta, StoryObj } from '@storybook/react-vite'
import { MemoryRouter, NavLink } from 'react-router-dom'
import { SegmentedControl, SegmentedControlItem } from './segmented-control'
import { segmentedControlItemVariants } from './segmented-control.variants'

const meta = {
  title: 'UI/SegmentedControl',
  component: SegmentedControl,
  tags: ['autodocs'],
  decorators: [
    (Story) => (
      <MemoryRouter initialEntries={['/dashboard']}>
        <div className="surface-panel max-w-3xl">
          <div className="surface-panel-body gap-6">
            <Story />
          </div>
        </div>
      </MemoryRouter>
    ),
  ],
} satisfies Meta<typeof SegmentedControl>

export default meta

type Story = StoryObj<typeof meta>

export const Overview: Story = {
  render: () => (
    <div className="flex flex-col gap-5">
      <div className="space-y-2">
        <p className="text-sm font-medium text-base-content/80">Compact metric toggle</p>
        <SegmentedControl size="compact" role="tablist" aria-label="Metric switch">
          <SegmentedControlItem active role="tab" aria-selected="true">
            次数
          </SegmentedControlItem>
          <SegmentedControlItem role="tab" aria-selected="false">
            金额
          </SegmentedControlItem>
          <SegmentedControlItem role="tab" aria-selected="false">
            Tokens
          </SegmentedControlItem>
        </SegmentedControl>
      </div>

      <div className="space-y-2">
        <p className="text-sm font-medium text-base-content/80">Default range toggle</p>
        <SegmentedControl role="tablist" aria-label="Range switch">
          <SegmentedControlItem active role="tab" aria-selected="true">
            24 小时
          </SegmentedControlItem>
          <SegmentedControlItem role="tab" aria-selected="false">
            7 日
          </SegmentedControlItem>
        </SegmentedControl>
      </div>

      <div className="space-y-2">
        <p className="text-sm font-medium text-base-content/80">Router-driven navigation</p>
        <SegmentedControl size="nav" aria-label="Primary navigation">
          <NavLink to="/dashboard" className={({ isActive }) => segmentedControlItemVariants({ size: 'nav', active: isActive })}>
            总览
          </NavLink>
          <NavLink to="/stats" className={({ isActive }) => segmentedControlItemVariants({ size: 'nav', active: isActive })}>
            统计
          </NavLink>
          <NavLink to="/settings" className={({ isActive }) => segmentedControlItemVariants({ size: 'nav', active: isActive })}>
            设置
          </NavLink>
        </SegmentedControl>
      </div>
    </div>
  ),
}
