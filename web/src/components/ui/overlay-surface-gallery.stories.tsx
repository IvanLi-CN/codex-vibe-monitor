import { useEffect, type ReactNode } from 'react'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { Tooltip } from './tooltip'
import { InfoTooltip } from './info-tooltip'
import { InlineChartTooltipSurface } from './inline-chart-tooltip'
import { MotherSwitchUndoToast, SystemNotificationProvider } from './system-notifications'
import { I18nProvider } from '../../i18n'

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

function NoiseCard({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="rounded-[1.35rem] border border-base-300/65 bg-base-100/18 p-4 shadow-[0_18px_45px_rgba(15,23,42,0.12)] backdrop-blur-sm">
      <div className="mb-3 flex items-center justify-between gap-2">
        <h3 className="text-sm font-semibold text-base-content">{title}</h3>
        <span className="text-[11px] uppercase tracking-[0.14em] text-base-content/55">shared surface</span>
      </div>
      {children}
    </section>
  )
}

function InlineChartPreview() {
  return (
    <InlineChartTooltipSurface
      items={[
        {
          title: 'Window A',
          rows: [
            { label: 'Success', value: '24', tone: 'success' },
            { label: 'Failure', value: '2', tone: 'error' },
          ],
        },
        {
          title: 'Window B',
          rows: [
            { label: 'Success', value: '18', tone: 'success' },
            { label: 'Failure', value: '5', tone: 'error' },
          ],
        },
      ]}
      defaultIndex={0}
      ariaLabel="Overlay gallery chart"
      interactionHint="Hover a bar to inspect the shared frosted tooltip."
      className="w-full"
      chartClassName="h-28"
    >
      {({ getItemProps }) => (
        <div className="flex h-full items-end gap-4 rounded-[1rem] border border-base-300/35 bg-base-100/10 px-5 py-4">
          {[
            { height: 72, label: 'A' },
            { height: 94, label: 'B' },
          ].map((item, index) => {
            const { ref, ...itemProps } = getItemProps(index)
            return (
              <button
                key={item.label}
                ref={ref}
                type="button"
                data-testid={`overlay-bar-${index}`}
                aria-label={`Window ${item.label}`}
                className="group flex flex-1 flex-col items-center justify-end gap-2 rounded-[0.9rem] border border-transparent bg-transparent pb-1 outline-none focus-visible:border-primary/50"
                {...itemProps}
              >
                <span
                  className="w-full rounded-[0.85rem] bg-gradient-to-t from-primary/75 via-info/80 to-success/80 shadow-[inset_0_1px_0_rgba(255,255,255,0.18)] transition-transform group-hover:scale-[1.02]"
                  style={{ height: `${item.height}px` }}
                />
                <span className="text-xs font-semibold text-base-content/68">{item.label}</span>
              </button>
            )
          })}
        </div>
      )}
    </InlineChartTooltipSurface>
  )
}

function OverlayGallery({ theme }: { theme: 'vibe-light' | 'vibe-dark' }) {
  const previewNotification = {
    id: 'storybook-mother-switch-preview',
    kind: 'motherSwitchUndo' as const,
    groupKey: 'tokyo-core',
    payload: {
      groupKey: 'tokyo-core',
      groupName: 'Tokyo Core',
      previousMotherAccountId: 11,
      previousMotherDisplayName: 'Codex Pro - Tokyo',
      newMotherAccountId: 18,
      newMotherDisplayName: 'Codex Team - Tokyo',
      hadNoMotherBefore: false,
    },
    onUndo: async () => undefined,
    error: null,
  }

  return (
    <I18nProvider>
      <ThemeRoot theme={theme}>
        <SystemNotificationProvider>
          <div
            className="min-h-screen px-5 py-6"
            style={{
              backgroundImage:
                'radial-gradient(circle at 14% 0%, rgba(56,189,248,0.18), transparent 36%), radial-gradient(circle at 86% 8%, rgba(45,212,191,0.16), transparent 30%)',
            }}
          >
            <div className="mx-auto max-w-6xl space-y-4">
              <div className="flex justify-center px-3">
                <MotherSwitchUndoToast
                  notification={previewNotification}
                  onDismiss={() => undefined}
                  onUndoSettled={() => undefined}
                  theme={theme}
                />
              </div>
              <div className="grid gap-4 lg:grid-cols-[1.15fr_1fr]">
                <NoiseCard title="Tooltip + InfoTooltip">
                  <div className="rounded-[1.1rem] border border-base-300/35 bg-base-100/12 px-4 py-4">
                    <div className="flex flex-wrap items-center gap-3 text-sm text-base-content/80">
                      <Tooltip open content="This tooltip now uses the same shared frosted surface tokens as the rest of the overlay family.">
                        <button type="button" className="rounded-full border border-primary/35 bg-primary/12 px-3 py-2 font-medium text-primary">
                          Shared tooltip
                        </button>
                      </Tooltip>
                      <div className="inline-flex items-center gap-2 rounded-full border border-warning/35 bg-warning/10 px-3 py-2 text-xs font-semibold text-warning">
                        <span>17 条新数据</span>
                        <InfoTooltip
                          label="Explain shared overlay surface"
                          content="Pinned info tooltips should feel like the same frosted family instead of a separate semi-transparent bubble."
                        />
                      </div>
                    </div>
                  </div>
                </NoiseCard>
                <NoiseCard title="Inline chart tooltip">
                  <InlineChartPreview />
                </NoiseCard>
              </div>
              <NoiseCard title="Dense background reference">
                <div className="overflow-hidden rounded-[1.1rem] border border-base-300/35 bg-base-100/8">
                  {Array.from({ length: 4 }, (_, index) => (
                    <div
                      key={index}
                      className="grid grid-cols-[1.4fr_0.9fr_0.8fr_1fr] items-center gap-3 border-b border-base-300/25 px-4 py-3 text-sm text-base-content/72 last:border-b-0"
                    >
                      <span className="truncate font-medium">{`ora.success.${index + 3}@mail-tw.707079.xyz`}</span>
                      <span className="font-mono">{`同步 03/25 17:1${index}`}</span>
                      <span>{`${84 - index * 9}%`}</span>
                      <span className="truncate">{`最近动作 路由恢复成功 · ${index + 1} 分钟前`}</span>
                    </div>
                  ))}
                </div>
              </NoiseCard>
            </div>
          </div>
        </SystemNotificationProvider>
      </ThemeRoot>
    </I18nProvider>
  )
}

const meta = {
  title: 'UI/Overlay Surface Gallery',
  component: OverlayGallery,
  tags: ['autodocs'],
  args: {
    theme: 'vibe-light',
  },
  parameters: {
    layout: 'fullscreen',
  },
} satisfies Meta<typeof OverlayGallery>

export default meta

type Story = StoryObj<typeof meta>

async function openOverlaySurfaceGallery({ canvasElement }: { canvasElement: HTMLElement }) {
    const canvas = within(canvasElement)
    const body = within(canvasElement.ownerDocument.body)
    await userEvent.click(canvas.getByRole('button', { name: /explain shared overlay surface/i }))
    await userEvent.hover(canvas.getByRole('button', { name: 'Window A' }))
    await waitFor(async () => {
      await expect(body.getAllByRole('tooltip').length).toBeGreaterThan(1)
      await expect(body.getByText(/Tokyo Core/i)).toBeInTheDocument()
    })
}

export const LightTheme: Story = {
  args: {
    theme: 'vibe-light',
  },
  render: (args) => <OverlayGallery {...args} />,
  play: openOverlaySurfaceGallery,
}

export const DarkTheme: Story = {
  args: {
    theme: 'vibe-dark',
  },
  render: (args) => <OverlayGallery {...args} />,
  play: openOverlaySurfaceGallery,
}
