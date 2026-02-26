import { useEffect, useMemo, useState } from 'react'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { I18nProvider } from '../i18n'
import type { StatsResponse } from '../lib/api'
import { TodayStatsOverview } from './TodayStatsOverview'

const sampleStats: StatsResponse = {
  totalCount: 2184,
  successCount: 2149,
  failureCount: 35,
  totalCost: 12.47,
  totalTokens: 842190,
}

const meta = {
  title: 'Dashboard/TodayStatsOverview',
  component: TodayStatsOverview,
  parameters: {
    layout: 'fullscreen',
  },
  decorators: [
    (Story) => (
      <I18nProvider>
        <div data-theme="light" className="min-h-screen bg-base-200 px-6 py-6 text-base-content">
          <div className="mx-auto w-full max-w-4xl">
            <Story />
          </div>
        </div>
      </I18nProvider>
    ),
  ],
} satisfies Meta<typeof TodayStatsOverview>

export default meta

type Story = StoryObj<typeof meta>

export const Populated: Story = {
  args: {
    stats: sampleStats,
    loading: false,
    error: null,
  },
}

export const Loading: Story = {
  args: {
    stats: null,
    loading: true,
    error: null,
  },
}

export const Empty: Story = {
  args: {
    stats: null,
    loading: false,
    error: null,
  },
}

export const LoadError: Story = {
  args: {
    stats: null,
    loading: false,
    error: 'Request failed: 500 unable to open database file',
  },
}

function buildAnimatedStats(step: number): StatsResponse {
  const totalCount = sampleStats.totalCount + step * 17
  const failureCount = 18 + (step % 5) * 3
  const successCount = Math.max(totalCount - failureCount, 0)
  const totalTokens = sampleStats.totalTokens + step * 5630 + (step % 3) * 830
  const totalCost = Number((sampleStats.totalCost + step * 0.11 + (step % 4) * 0.03).toFixed(2))

  return {
    totalCount,
    successCount,
    failureCount,
    totalCost,
    totalTokens,
  }
}

function LiveTickerPreview() {
  const [ready, setReady] = useState(false)
  const [step, setStep] = useState(0)

  useEffect(() => {
    const warmup = window.setTimeout(() => {
      setReady(true)
    }, 900)

    return () => {
      window.clearTimeout(warmup)
    }
  }, [])

  useEffect(() => {
    if (!ready) return undefined
    const timer = window.setInterval(() => {
      setStep((value) => value + 1)
    }, 1400)

    return () => {
      window.clearInterval(timer)
    }
  }, [ready])

  const stats = useMemo(() => buildAnimatedStats(step), [step])

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between rounded-xl border border-primary/25 bg-primary/10 px-4 py-2 text-sm text-base-content/75">
        <span>Live demo auto-updates every 1.4s</span>
        <span className="font-semibold text-primary">Tick #{step}</span>
      </div>
      <TodayStatsOverview stats={ready ? stats : null} loading={!ready} error={null} />
    </div>
  )
}

export const LiveTicker: Story = {
  args: {
    stats: null,
    loading: true,
    error: null,
  },
  render: () => <LiveTickerPreview />,
}
