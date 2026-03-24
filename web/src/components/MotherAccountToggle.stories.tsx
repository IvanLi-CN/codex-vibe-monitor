import { useState, type ReactNode } from 'react'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, within } from 'storybook/test'
import { MotherAccountBadge, MotherAccountToggle } from './MotherAccountToggle'

const noop = () => undefined

function StorySurface({ children }: { children: ReactNode }) {
  return (
    <div className="min-h-screen bg-base-200 px-6 py-8 text-base-content">
      <div className="mx-auto flex max-w-6xl flex-col gap-6 rounded-[1.75rem] border border-base-300/70 bg-base-100/50 p-6 shadow-sm">
        {children}
      </div>
    </div>
  )
}

function ThemePanel({
  theme,
  title,
  children,
}: {
  theme: 'vibe-light' | 'vibe-dark'
  title: string
  children: ReactNode
}) {
  return (
    <section
      data-theme={theme}
      className="rounded-[1.5rem] border border-base-300/70 bg-base-200/82 p-5 text-base-content shadow-[0_18px_40px_rgba(15,23,42,0.14)]"
    >
      <div className="mb-4 text-xs font-semibold uppercase tracking-[0.16em] text-base-content/55">{title}</div>
      {children}
    </section>
  )
}

function OverviewGallery() {
  return (
    <StorySurface>
      <div className="grid gap-5 xl:grid-cols-2">
        {[
          { theme: 'vibe-light' as const, title: 'Light Theme' },
          { theme: 'vibe-dark' as const, title: 'Dark Theme' },
        ].map((panel) => (
          <ThemePanel key={panel.theme} theme={panel.theme} title={panel.title}>
            <div className="space-y-5">
              <div className="flex flex-wrap items-center gap-3">
                <MotherAccountBadge label="母号" />
                <div className="inline-flex rounded-full border border-base-300/75 bg-base-100/72 px-3 py-1.5 text-xs text-base-content/68">
                  与普通状态标签并排时也能单独识别
                </div>
              </div>
              <div className="grid gap-4">
                <div className="rounded-[1.25rem] border border-base-300/70 bg-base-100/78 p-4">
                  <div className="mb-3 text-xs font-semibold uppercase tracking-[0.14em] text-base-content/52">
                    Checked Toggle
                  </div>
                  <MotherAccountToggle
                    checked
                    label="设为母号"
                    description="每个分组只能保留一个母号。开启后会自动把同组旧母号的皇冠切走。"
                    onToggle={() => undefined}
                  />
                </div>
                <div className="rounded-[1.25rem] border border-base-300/70 bg-base-100/78 p-4">
                  <div className="mb-3 text-xs font-semibold uppercase tracking-[0.14em] text-base-content/52">
                    Batch Row Icon
                  </div>
                  <div className="flex items-center gap-3">
                    <MotherAccountToggle
                      checked
                      iconOnly
                      label="母号"
                      ariaLabel="切换母号"
                      onToggle={() => undefined}
                    />
                    <MotherAccountToggle
                      checked={false}
                      iconOnly
                      label="母号"
                      ariaLabel="切换母号"
                      onToggle={() => undefined}
                    />
                  </div>
                </div>
              </div>
            </div>
          </ThemePanel>
        ))}
      </div>
    </StorySurface>
  )
}

function ToggleHarness({ iconOnly = false }: { iconOnly?: boolean }) {
  const [checked, setChecked] = useState(false)

  return (
    <StorySurface>
      <div className="max-w-lg rounded-[1.4rem] border border-base-300/70 bg-base-100/78 p-5">
        <div className="mb-4 flex items-center gap-3">
          <MotherAccountBadge label="母号" />
          <span className="text-sm text-base-content/72">点击切换后应立即更新高对比状态。</span>
        </div>
        <div className="space-y-4">
          <MotherAccountToggle
            checked={checked}
            iconOnly={iconOnly}
            label="设为母号"
            description="每个分组只能保留一个母号。开启后会自动把同组旧母号的皇冠切走。"
            ariaLabel={iconOnly ? '切换母号' : undefined}
            onToggle={() => setChecked((current) => !current)}
          />
          <div className="rounded-xl border border-base-300/70 bg-base-200/75 px-4 py-3 text-sm text-base-content/70">
            当前状态:
            <span data-testid="mother-toggle-state" className="ml-2 font-semibold text-base-content">
              {checked ? 'mother' : 'not-mother'}
            </span>
          </div>
        </div>
      </div>
    </StorySurface>
  )
}

const meta = {
  title: 'Account Pool/Components/Mother Account Toggle',
  component: MotherAccountToggle,
  subcomponents: {
    MotherAccountBadge,
  },
  tags: ['autodocs'],
  parameters: {
    layout: 'fullscreen',
    docs: {
      description: {
        component:
          '母号标签与切换卡的专用高对比展示。保留 amber 语义，但不再依赖低对比的 `warning-content` 文本色。',
      },
    },
  },
  args: {
    checked: false,
    disabled: false,
    iconOnly: false,
    label: '设为母号',
    description: '每个分组只能保留一个母号。开启后会自动把同组旧母号的皇冠切走。',
    onToggle: noop,
    ariaLabel: '切换母号',
  },
} satisfies Meta<typeof MotherAccountToggle>

export default meta

type Story = StoryObj<typeof meta>

export const Overview: Story = {
  render: () => <OverviewGallery />,
}

export const InteractiveToggle: Story = {
  render: () => <ToggleHarness />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const toggle = canvas.getByRole('button', { name: /设为母号/i })
    const state = canvas.getByTestId('mother-toggle-state')

    await expect(toggle).toHaveAttribute('aria-pressed', 'false')
    await expect(state).toHaveTextContent('not-mother')

    await userEvent.click(toggle)
    await expect(toggle).toHaveAttribute('aria-pressed', 'true')
    await expect(state).toHaveTextContent('mother')

    await userEvent.click(toggle)
    await expect(toggle).toHaveAttribute('aria-pressed', 'false')
    await expect(state).toHaveTextContent('not-mother')
  },
}

export const BatchRowIconToggle: Story = {
  render: () => <ToggleHarness iconOnly />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const toggle = canvas.getByRole('button', { name: /切换母号/i })
    const state = canvas.getByTestId('mother-toggle-state')

    await expect(toggle).toHaveAttribute('aria-pressed', 'false')
    await userEvent.click(toggle)
    await expect(toggle).toHaveAttribute('aria-pressed', 'true')
    await expect(state).toHaveTextContent('mother')
  },
}
