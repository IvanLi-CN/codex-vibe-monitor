import { useEffect, type ReactNode } from 'react'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, within } from 'storybook/test'
import { UpdateAvailableBanner, type UpdateAvailableBannerProps } from './UpdateAvailableBanner'

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

function DenseRows() {
  return (
    <div className="rounded-[1.6rem] border border-primary/30 bg-primary/12 p-5">
      <div className="overflow-hidden rounded-[1.3rem] border border-base-300/60 bg-base-100/20">
        {Array.from({ length: 5 }, (_, index) => (
          <div
            key={index}
            className="grid grid-cols-[minmax(16rem,2.1fr)_1.1fr_0.8fr_0.9fr_0.8fr] items-center gap-4 border-b border-base-300/35 px-5 py-4 text-sm text-base-content/78 last:border-b-0"
          >
            <div className="min-w-0">
              <p className="truncate font-semibold text-base-content">{`ora.success.${index + 1}@mail-tw.707079.xyz`}</p>
              <div className="mt-2 flex gap-2 text-[11px]">
                <span className="rounded-full border border-success/35 bg-success/14 px-2 py-1 text-success">启用</span>
                <span className="rounded-full border border-info/35 bg-info/14 px-2 py-1 text-info">工作 1</span>
                <span className="rounded-full border border-base-300/70 bg-base-100/38 px-2 py-1">OAuth</span>
              </div>
            </div>
            <p className="font-mono text-base-content/74">{`同步 03/25 17:1${index}`}</p>
            <p className="font-semibold text-base-content/82">{`${89 - index * 7}%`}</p>
            <p className="font-mono text-base-content/74">{`重置 03/${25 + index} 18:59`}</p>
            <div className="flex items-center gap-3">
              <div className="h-2.5 flex-1 rounded-full bg-base-300/35">
                <div className="h-2.5 rounded-full bg-primary/70" style={{ width: `${68 - index * 8}%` }} />
              </div>
              <span className="font-semibold text-base-content/82">{`${41 - index * 5}%`}</span>
            </div>
          </div>
        ))}
      </div>
    </div>
  )
}

function BannerScene({
  theme,
  compact = false,
  bannerProps,
}: {
  theme: 'vibe-light' | 'vibe-dark'
  compact?: boolean
  bannerProps: UpdateAvailableBannerProps
}) {
  return (
    <ThemeRoot theme={theme}>
      <div
        className="min-h-screen px-4 py-6"
        style={{
          backgroundImage:
            'radial-gradient(circle at 16% 0%, rgba(56,189,248,0.18), transparent 34%), radial-gradient(circle at 88% 10%, rgba(45,212,191,0.16), transparent 30%)',
        }}
      >
        <div className={compact ? 'mx-auto max-w-[28rem]' : 'app-shell-boundary'}>
          <UpdateAvailableBanner {...bannerProps} />
          <div className="mt-4">
            <DenseRows />
          </div>
        </div>
      </div>
    </ThemeRoot>
  )
}

const meta = {
  title: 'Shell/Notifications/Update Available Banner',
  component: UpdateAvailableBanner,
  tags: ['autodocs'],
  args: {
    currentVersion: '1.24.4',
    availableVersion: '1.25.2',
    onReload: () => undefined,
    onDismiss: () => undefined,
    labels: {
      available: '有新版本可用：',
      refresh: '立即刷新',
      later: '稍后',
    },
  },
  parameters: {
    layout: 'fullscreen',
  },
} satisfies Meta<typeof UpdateAvailableBanner>

export default meta

type Story = StoryObj<typeof meta>

export const DenseBackdropLight: Story = {
  render: (args) => <BannerScene theme="vibe-light" bannerProps={args} />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByRole('status')).toBeInTheDocument()
    await expect(canvas.getByRole('button', { name: '立即刷新' })).toBeInTheDocument()
    await expect(canvas.getByRole('button', { name: '稍后' })).toBeInTheDocument()
  },
}

export const DenseBackdropDark: Story = {
  render: (args) => <BannerScene theme="vibe-dark" bannerProps={args} />,
}

export const CompactWidth: Story = {
  render: (args) => <BannerScene theme="vibe-light" compact bannerProps={args} />,
}
