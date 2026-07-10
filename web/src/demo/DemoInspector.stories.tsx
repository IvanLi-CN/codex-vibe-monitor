import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, within } from 'storybook/test'
import { MemoryRouter } from 'react-router-dom'
import { DemoInspector } from './DemoInspector'
import { demoModel } from './model'
import type { DemoScene } from './runtime'

function InspectorStory({ scene }: { scene: DemoScene }) {
  demoModel.setScene(scene)

  return (
    <MemoryRouter initialEntries={[`/dashboard?demoScene=${scene}&demoTheme=light`]}>
      <div className="min-h-[44rem] bg-base-200 p-6 text-base-content">
        <p className="text-sm text-base-content/70">Demo Inspector state preview</p>
        <DemoInspector defaultOpen />
      </div>
    </MemoryRouter>
  )
}

const meta = {
  title: 'Demo/Inspector',
  component: DemoInspector,
  tags: ['autodocs'],
  parameters: {
    layout: 'fullscreen',
    docs: {
      description: {
        component: '公开 Web Demo 的受控场景、主题、reset、分享与实时事件注入入口。',
      },
    },
  },
} satisfies Meta<typeof DemoInspector>

export default meta

type Story = StoryObj<typeof meta>

export const Operational: Story = {
  render: () => <InspectorStory scene="operational" />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const controls = canvas.getByTestId('demo-inspector-controls')
    const inspector = within(controls)
    await expect(inspector.getByRole('button', { name: '正常' })).toHaveAttribute('aria-pressed', 'true')
    await userEvent.click(inspector.getByRole('button', { name: '告警' }))
    await expect(inspector.getByRole('button', { name: '告警' })).toHaveAttribute('aria-pressed', 'true')
  },
}

export const Attention: Story = {
  render: () => <InspectorStory scene="attention" />,
}

export const Empty: Story = {
  render: () => <InspectorStory scene="empty" />,
}

export const NetworkFailure: Story = {
  render: () => <InspectorStory scene="network-failure" />,
}
