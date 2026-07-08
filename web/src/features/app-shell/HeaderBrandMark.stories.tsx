import type { ReactNode } from 'react'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, within } from 'storybook/test'
import { HeaderBrandMark, type HeaderBrandMarkState } from './HeaderBrandMark'

const stateEntries: Array<{
  state: HeaderBrandMarkState
  title: string
  description: string
}> = [
  {
    state: 'idle',
    title: 'Quiet',
    description: 'SSE 已连接，但最近没有新的数据更新，logo mark 保持稳定待机态。',
  },
  {
    state: 'active',
    title: 'Active Updates',
    description: '最近一段时间持续有数据流入时，进入连续呼吸态，不会被每条事件重置回首帧。',
  },
  {
    state: 'reconnecting',
    title: 'Reconnecting',
    description: '连接恢复期间切换成旋转虚线环，保留 transport feedback，但不再伪装成活跃更新。',
  },
  {
    state: 'disabled',
    title: 'Auto Reconnect Disabled',
    description: '自动重连关闭后，mark 降亮并停在警示态，避免继续暗示“数据还在流动”。',
  },
]

function StorySurface({ children }: { children: ReactNode }) {
  return (
    <div className="min-h-screen bg-base-200 px-6 py-8 text-base-content">
      <div
        data-testid="header-brand-mark-story-surface"
        className="mx-auto flex max-w-6xl flex-col gap-6 rounded-[1.75rem] border border-base-300/70 bg-base-100/50 p-6 shadow-sm"
      >
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
      className="overflow-hidden rounded-[1.5rem] border border-base-300/70 bg-base-200 text-base-content shadow-[0_18px_40px_rgba(15,23,42,0.14)]"
    >
      <div className="border-b border-base-300/60 px-5 py-4 text-xs font-semibold uppercase tracking-[0.16em] text-base-content/55">
        {title}
      </div>
      <div className="p-5">{children}</div>
    </section>
  )
}

function HeaderStateCard({
  state,
  title,
  description,
}: {
  state: HeaderBrandMarkState
  title: string
  description: string
}) {
  return (
    <article
      className="rounded-[1.25rem] border border-base-300/70 bg-base-100/88 p-4 shadow-[0_12px_30px_rgba(15,23,42,0.08)]"
      data-testid={`header-brand-mark-card-${state}`}
    >
      <div className="rounded-[1.05rem] border border-base-300/70 bg-base-100/92 px-4 py-3">
        <div className="flex items-center gap-3">
          <HeaderBrandMark
            alt="Codex Vibe Monitor product icon"
            state={state}
            data-testid={`header-brand-mark-${state}`}
          />
          <div className="min-w-0">
            <p className="truncate text-lg font-semibold tracking-tight text-base-content">Codex Vibe Monitor</p>
            <p className="mt-1 text-sm text-base-content/72">{title}</p>
          </div>
        </div>
      </div>
      <p className="mt-3 text-sm leading-6 text-base-content/72">{description}</p>
    </article>
  )
}

function StateGalleryScene() {
  return (
    <StorySurface>
      <div className="space-y-3">
        <h1 className="text-xl font-semibold tracking-tight text-base-content">Header Logo Mark State Gallery</h1>
        <p className="max-w-3xl text-sm leading-6 text-base-content/72">
          页头 `logo mark` 现在按连接状态与最近活动窗口驱动。活跃态保持连续呼吸，不再被每条 SSE
          message 重置回动画起点。
        </p>
      </div>
      <div className="grid gap-5 xl:grid-cols-2">
        {[
          { theme: 'vibe-light' as const, title: 'Light Theme' },
          { theme: 'vibe-dark' as const, title: 'Dark Theme' },
        ].map((panel) => (
          <ThemePanel key={panel.theme} theme={panel.theme} title={panel.title}>
            <div className="grid gap-4 md:grid-cols-2">
              {stateEntries.map((entry) => (
                <HeaderStateCard
                  key={`${panel.theme}-${entry.state}`}
                  state={entry.state}
                  title={entry.title}
                  description={entry.description}
                />
              ))}
            </div>
          </ThemePanel>
        ))}
      </div>
    </StorySurface>
  )
}

function SingleStatePreview({ state }: { state: HeaderBrandMarkState }) {
  const entry = stateEntries.find((candidate) => candidate.state === state) ?? stateEntries[0]

  return (
    <StorySurface>
      <ThemePanel theme="vibe-light" title="Light Theme">
        <HeaderStateCard state={entry.state} title={entry.title} description={entry.description} />
      </ThemePanel>
    </StorySurface>
  )
}

const meta = {
  title: 'Shell/Header/Header Brand Mark',
  component: HeaderBrandMark,
  tags: ['autodocs'],
  parameters: {
    layout: 'fullscreen',
    docs: {
      description: {
        component:
          'Sticky app header 的产品图标状态组件。活跃态使用连续呼吸动画表达最近持续更新；重连与禁用状态切换到独立 transport feedback。',
      },
    },
  },
  args: {
    alt: 'Codex Vibe Monitor product icon',
    state: 'idle',
  },
  render: (args) => <SingleStatePreview state={args.state ?? 'idle'} />,
} satisfies Meta<typeof HeaderBrandMark>

export default meta

type Story = StoryObj<typeof meta>

export const StateGallery: Story = {
  render: () => <StateGalleryScene />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByTestId('header-brand-mark-card-active')).toBeVisible()
    await expect(canvas.getByTestId('header-brand-mark-card-reconnecting')).toBeVisible()
    await expect(canvas.getByTestId('header-brand-mark-card-disabled')).toBeVisible()
  },
}

export const Quiet: Story = {
  args: {
    state: 'idle',
  },
}

export const ActiveUpdates: Story = {
  args: {
    state: 'active',
  },
  parameters: {
    docs: {
      description: {
        story: '最近一段时间持续有数据更新时，呼吸动效会连续保持，直到活跃窗口超时后才平滑退回静止态。',
      },
    },
  },
}

export const Reconnecting: Story = {
  args: {
    state: 'reconnecting',
  },
}

export const AutoReconnectDisabled: Story = {
  args: {
    state: 'disabled',
  },
}
