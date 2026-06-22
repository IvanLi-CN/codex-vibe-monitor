import { useLayoutEffect, useRef, type ReactNode } from 'react'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, within } from 'storybook/test'
import { MemoryRouter, Route, Routes } from 'react-router-dom'
import { I18nProvider } from '../i18n'
import type {
  ExternalApiKeySummary,
  SettingsPayload,
  SystemStatusResponse,
  SystemTaskRunsResponse,
} from '../lib/api'
import { FullPageStorySurface, StorybookPageEnvironment, type StorybookRequestHandler } from './storybookPageHelpers'
import SystemLayout from '../pages/system/SystemLayout'
import SystemProxyPage from '../pages/system/SystemProxyPage'
import SystemSettingsPage from '../pages/system/SystemSettingsPage'
import SystemStatusPage from '../pages/system/SystemStatusPage'
import SystemTasksPage from '../pages/system/SystemTasksPage'

const STORYBOOK_SYSTEM_STATUS: SystemStatusResponse = {
  successCount: 124_882,
  nonSuccessCount: 3_194,
  archivedBodies: { count: 118_420, bytes: 8_441_053_184 },
  unarchivedBodies: { count: 246, bytes: 48_221_184 },
  databaseBytes: 618_659_840,
  otherFilesBytes: 142_344_192,
  refreshedAt: '2026-06-22T09:28:00Z',
}

const STORYBOOK_SYSTEM_TASK_ITEMS: SystemTaskRunsResponse['items'] = [
  {
    id: 41,
    taskKind: 'forward_proxy_subscription_refresh',
    triggerKind: 'interval',
    status: 'success',
    summary: 'refreshed 3 subscriptions and added 18 nodes',
    detail: 'Completed in background maintenance loop without manual intervention.',
    startedAt: '2026-06-22T09:20:00Z',
    finishedAt: '2026-06-22T09:20:02Z',
    durationMs: 2014,
  },
  {
    id: 40,
    taskKind: 'retention_archive',
    triggerKind: 'interval',
    status: 'success',
    summary: 'compressed=27 archived_invocations=860 pruned_details=860 orphan_raw_removed=4',
    detail: 'Archive maintenance rotated raw payloads and trimmed invocation details.',
    startedAt: '2026-06-22T09:00:00Z',
    finishedAt: '2026-06-22T09:00:11Z',
    durationMs: 11182,
  },
  {
    id: 39,
    taskKind: 'startup_backfill',
    triggerKind: 'startup',
    status: 'success',
    summary: 'replayed retained raw captures into usage rollups',
    detail: 'Startup backfill completed before the main scheduler resumed normal polling.',
    startedAt: '2026-06-22T08:58:00Z',
    finishedAt: '2026-06-22T08:58:12Z',
    durationMs: 12103,
  },
  {
    id: 38,
    taskKind: 'scheduler_poll',
    triggerKind: 'interval',
    status: 'failed',
    summary: 'pool poll timed out while upstream was degraded',
    detail: 'The scheduler retried after a handshake timeout and recovered on the next interval.',
    startedAt: '2026-06-22T08:40:00Z',
    finishedAt: '2026-06-22T08:40:10Z',
    durationMs: 10000,
  },
]

for (let index = 0; index < 21; index += 1) {
  const id = 37 - index
  STORYBOOK_SYSTEM_TASK_ITEMS.push({
    id,
    taskKind: index % 4 === 0 ? 'scheduler_poll' : index % 4 === 1 ? 'retention_archive' : index % 4 === 2 ? 'startup_backfill' : 'forward_proxy_subscription_refresh',
    triggerKind: index % 3 === 0 ? 'interval' : 'startup',
    status: index % 5 === 0 ? 'failed' : 'success',
    summary: `storybook task run ${id} summary`,
    detail: `Synthetic task run ${id} keeps pagination states visible in the system workspace story.`,
    startedAt: `2026-06-21T${String(23 - (index % 10)).padStart(2, '0')}:00:00Z`,
    finishedAt: `2026-06-21T${String(23 - (index % 10)).padStart(2, '0')}:00:05Z`,
    durationMs: 5000 + index * 73,
  })
}

function filterStorybookSystemTasks(url: URL): SystemTaskRunsResponse {
  const taskKind = url.searchParams.get('taskKind')?.trim()
  const status = url.searchParams.get('status')?.trim()
  const startedAtFrom = url.searchParams.get('startedAtFrom')?.trim()
  const startedAtTo = url.searchParams.get('startedAtTo')?.trim()
  const startedAtFromMs = startedAtFrom ? Date.parse(startedAtFrom) : Number.NaN
  const startedAtToMs = startedAtTo ? Date.parse(startedAtTo) : Number.NaN
  const page = Number(url.searchParams.get('page') ?? '1')
  const pageSize = Number(url.searchParams.get('pageSize') ?? url.searchParams.get('limit') ?? '20')
  const filtered = STORYBOOK_SYSTEM_TASK_ITEMS.filter((item) => {
    const startedAtMs = Date.parse(item.startedAt)
    if (taskKind && item.taskKind !== taskKind) return false
    if (status && item.status !== status) return false
    if (startedAtFrom && Number.isFinite(startedAtFromMs) && startedAtMs < startedAtFromMs) return false
    if (startedAtTo && Number.isFinite(startedAtToMs) && startedAtMs > startedAtToMs) return false
    return true
  })
  const safePage = Math.max(1, page)
  const safePageSize = Math.min(100, Math.max(1, pageSize))
  const start = (safePage - 1) * safePageSize
  return {
    total: filtered.length,
    page: safePage,
    pageSize: safePageSize,
    items: filtered.slice(start, start + safePageSize),
  }
}

const STORYBOOK_SETTINGS: SettingsPayload = {
  proxy: {
    hijackEnabled: true,
    mergeUpstreamEnabled: true,
    fastModeRewriteMode: 'disabled',
    upstream429MaxRetries: 3,
    websocketEnabled: true,
    upstreamWebsocketDefaultEnabled: true,
    defaultHijackEnabled: false,
    models: ['gpt-5.5', 'gpt-5.5-pro', 'gpt-5.4'],
    enabledModels: ['gpt-5.5', 'gpt-5.5-pro'],
  },
  forwardProxy: {
    proxyUrls: ['http://tokyo-edge.internal:8080', 'socks5://singapore-edge.internal:1080'],
    subscriptionUrls: ['https://example.com/subscription.base64'],
    subscriptionUpdateIntervalSecs: 3600,
    nodes: [
      {
        key: 'tokyo-edge',
        source: 'manual',
        displayName: 'tokyo-edge.internal:8080',
        endpointUrl: 'http://tokyo-edge.internal:8080',
        weight: 0.92,
        penalized: false,
        stats: {
          oneMinute: { attempts: 14, successRate: 0.93, avgLatencyMs: 182 },
          fifteenMinutes: { attempts: 168, successRate: 0.94, avgLatencyMs: 190 },
          oneHour: { attempts: 672, successRate: 0.94, avgLatencyMs: 204 },
          oneDay: { attempts: 1612, successRate: 0.95, avgLatencyMs: 216 },
          sevenDays: { attempts: 9120, successRate: 0.95, avgLatencyMs: 228 },
        },
      },
      {
        key: 'singapore-edge',
        source: 'manual',
        displayName: 'singapore-edge.internal:1080',
        endpointUrl: 'socks5://singapore-edge.internal:1080',
        weight: 0.71,
        penalized: false,
        stats: {
          oneMinute: { attempts: 10, successRate: 0.88, avgLatencyMs: 236 },
          fifteenMinutes: { attempts: 134, successRate: 0.9, avgLatencyMs: 242 },
          oneHour: { attempts: 588, successRate: 0.91, avgLatencyMs: 255 },
          oneDay: { attempts: 1450, successRate: 0.91, avgLatencyMs: 269 },
          sevenDays: { attempts: 8220, successRate: 0.92, avgLatencyMs: 278 },
        },
      },
    ],
  },
  pricing: {
    catalogVersion: 'storybook-system-2026-06',
    entries: [
      {
        model: 'gpt-5.5',
        inputPer1m: 5,
        outputPer1m: 30,
        cacheInputPer1m: 0.5,
        reasoningPer1m: null,
        source: 'official',
      },
      {
        model: 'gpt-5.5-pro',
        inputPer1m: 30,
        outputPer1m: 180,
        cacheInputPer1m: null,
        reasoningPer1m: null,
        source: 'official',
      },
    ],
  },
}

const STORYBOOK_EXTERNAL_API_KEYS: ExternalApiKeySummary[] = [
  {
    id: 11,
    name: 'Partner sync',
    status: 'active',
    prefix: 'cvm_ext_sys',
    lastUsedAt: '2026-06-22T08:22:00Z',
    createdAt: '2026-06-21T10:00:00Z',
    updatedAt: '2026-06-22T08:22:00Z',
  },
]

function clone<T>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T
}

function buildSystemWorkspaceRequestHandler(): StorybookRequestHandler {
  return async ({ url, init }) => {
    const method = (init?.method ?? 'GET').toUpperCase()
    const jsonResponse = (payload: unknown, status = 200) =>
      new Response(JSON.stringify(payload), {
        status,
        headers: { 'Content-Type': 'application/json' },
      })

    if (url.pathname === '/api/system/status' && method === 'GET') {
      return jsonResponse(clone(STORYBOOK_SYSTEM_STATUS))
    }

    if (url.pathname === '/api/system/tasks' && method === 'GET') {
      return jsonResponse(clone(filterStorybookSystemTasks(url)))
    }

    if (url.pathname === '/api/settings' && method === 'GET') {
      return jsonResponse(clone(STORYBOOK_SETTINGS))
    }

    if (url.pathname === '/api/settings/external-api-keys' && method === 'GET') {
      return jsonResponse({ items: clone(STORYBOOK_EXTERNAL_API_KEYS) })
    }

    return undefined
  }
}

function StorybookSystemWorkspaceRoutes() {
  return (
    <Routes>
      <Route path="/system" element={<SystemLayout />}>
        <Route path="status" element={<SystemStatusPage />} />
        <Route path="tasks" element={<SystemTasksPage />} />
        <Route path="settings" element={<SystemSettingsPage />} />
        <Route path="proxy" element={<SystemProxyPage />} />
      </Route>
    </Routes>
  )
}

function StorybookSystemWorkspaceMock({
  children,
}: {
  children: ReactNode
}) {
  const originalFetchRef = useRef<typeof window.fetch | null>(null)

  useLayoutEffect(() => {
    originalFetchRef.current = window.fetch.bind(window)
    return () => {
      if (originalFetchRef.current) {
        window.fetch = originalFetchRef.current
      }
    }
  }, [])

  return <>{children}</>
}

const meta = {
  title: 'System/SystemWorkspace',
  tags: ['autodocs'],
  parameters: {
    layout: 'fullscreen',
    viewport: { defaultViewport: 'desktop1660' },
  },
  decorators: [
    (Story) => (
      <I18nProvider>
        <StorybookSystemWorkspaceMock>
          <StorybookPageEnvironment onRequest={buildSystemWorkspaceRequestHandler()}>
            <FullPageStorySurface>
              <Story />
            </FullPageStorySurface>
          </StorybookPageEnvironment>
        </StorybookSystemWorkspaceMock>
      </I18nProvider>
    ),
  ],
} satisfies Meta

export default meta

type Story = StoryObj<typeof meta>

function renderWorkspace(initialEntry: string) {
  return (
    <MemoryRouter initialEntries={[initialEntry]}>
      <StorybookSystemWorkspaceRoutes />
    </MemoryRouter>
  )
}

export const Status: Story = {
  render: () => renderWorkspace('/system/status'),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByRole('heading', { name: '系统状态' })).toBeVisible()
    await expect(canvas.getByTestId('system-status-grid')).toBeVisible()
    await expect(canvas.getByRole('link', { name: '状态' })).toHaveAttribute('aria-current', 'page')
  },
}

export const Tasks: Story = {
  render: () => renderWorkspace('/system/tasks'),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByRole('heading', { name: '后台任务' })).toBeVisible()
    await expect(canvas.getByTestId('system-tasks-list')).toBeVisible()
    await expect(canvas.getByText(/forward_proxy_subscription_refresh/)).toBeVisible()
  },
}

export const Settings: Story = {
  render: () => renderWorkspace('/system/settings'),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByRole('heading', { name: '系统设置' })).toBeVisible()
    await expect(canvas.getByText('价格配置')).toBeVisible()
    await expect(canvas.getByText('External API Keys')).toBeVisible()
  },
}

export const Proxy: Story = {
  render: () => renderWorkspace('/system/proxy'),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByRole('heading', { name: '代理' })).toBeVisible()
    await expect(canvas.getByText('正向代理路由')).toBeVisible()
    await expect(canvas.getByTestId('settings-forward-proxy-desktop-table')).toBeVisible()
  },
}
