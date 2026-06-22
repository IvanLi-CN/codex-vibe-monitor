/** @vitest-environment jsdom */
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { I18nProvider } from '../../i18n'
import SystemTasksPage from './SystemTasksPage'

const apiMocks = vi.hoisted(() => ({
  fetchSystemTaskRuns: vi.fn(),
}))

vi.mock('../../lib/api', async () => {
  const actual = await vi.importActual<typeof import('../../lib/api')>('../../lib/api')
  return {
    ...actual,
    fetchSystemTaskRuns: apiMocks.fetchSystemTaskRuns,
  }
})

let host: HTMLDivElement | null = null
let root: Root | null = null

function buildResponse(page: number, pageSize: number) {
  return {
    total: 25,
    page,
    pageSize,
    items: [
      {
        id: page,
        taskKind: `task-${page}`,
        triggerKind: 'interval',
        status: 'success',
        summary: `summary-${page}`,
        detail: `detail-${page}`,
        startedAt: '2026-06-22T09:00:00Z',
        finishedAt: '2026-06-22T09:00:01Z',
        durationMs: 1000,
      },
    ],
  }
}

function expectedIsoForLocalValue(value: string, upperBound = false) {
  const parsed = new Date(value)
  if (upperBound) {
    parsed.setSeconds(59, 999)
  }
  return parsed.toISOString()
}

function renderPage() {
  host = document.createElement('div')
  document.body.appendChild(host)
  root = createRoot(host)

  act(() => {
    root?.render(
      <I18nProvider>
        <SystemTasksPage />
      </I18nProvider>,
    )
  })
}

async function flushEffects() {
  await act(async () => {
    await Promise.resolve()
  })
}

describe('SystemTasksPage', () => {
  beforeEach(() => {
    apiMocks.fetchSystemTaskRuns.mockImplementation(({ page = 1, pageSize = 20 } = {}) =>
      Promise.resolve(buildResponse(page, pageSize)),
    )
  })

  afterEach(() => {
    act(() => {
      root?.unmount()
    })
    host?.remove()
    host = null
    root = null
    apiMocks.fetchSystemTaskRuns.mockReset()
  })

  it('loads the first page and advances pagination', async () => {
    renderPage()
    await flushEffects()

    expect(apiMocks.fetchSystemTaskRuns).toHaveBeenNthCalledWith(1, {
      taskKind: undefined,
      status: undefined,
      startedAtFrom: undefined,
      startedAtTo: undefined,
      page: 1,
      pageSize: 20,
    })
    expect(host?.textContent ?? '').toContain('第 1 / 2 页')

    const nextButton = Array.from(host?.querySelectorAll('button') ?? []).find((node) =>
      node.textContent?.includes('下一页'),
    )
    expect(nextButton).toBeTruthy()

    await act(async () => {
      nextButton?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    })
    await flushEffects()

    expect(apiMocks.fetchSystemTaskRuns).toHaveBeenNthCalledWith(2, {
      taskKind: undefined,
      status: undefined,
      startedAtFrom: undefined,
      startedAtTo: undefined,
      page: 2,
      pageSize: 20,
    })
    expect(host?.textContent ?? '').toContain('task-2')
  })

  it('resets to page one after changing filters', async () => {
    renderPage()
    await flushEffects()

    const nextButton = Array.from(host?.querySelectorAll('button') ?? []).find((node) =>
      node.textContent?.includes('下一页'),
    )
    expect(nextButton).toBeTruthy()

    await act(async () => {
      nextButton?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    })
    await flushEffects()

    const input = host?.querySelector('input[placeholder="按任务类型筛选"]') as HTMLInputElement | null
    expect(input).not.toBeNull()

    await act(async () => {
      if (input) {
        const valueSetter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value')?.set
        valueSetter?.call(input, 'scheduler')
        input.dispatchEvent(new Event('change', { bubbles: true }))
        input.dispatchEvent(new Event('input', { bubbles: true }))
      }
    })
    await flushEffects()

    expect(apiMocks.fetchSystemTaskRuns).toHaveBeenLastCalledWith({
      taskKind: 'scheduler',
      status: undefined,
      startedAtFrom: undefined,
      startedAtTo: undefined,
      page: 1,
      pageSize: 20,
    })
  })

  it('passes the started-at range filters to the API', async () => {
    renderPage()
    await flushEffects()

    const inputs = Array.from(host?.querySelectorAll('input[type="datetime-local"]') ?? []) as HTMLInputElement[]
    expect(inputs).toHaveLength(2)

    await act(async () => {
      const valueSetter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value')?.set
      valueSetter?.call(inputs[0], '2026-06-22T09:00')
      inputs[0].dispatchEvent(new Event('change', { bubbles: true }))
      inputs[0].dispatchEvent(new Event('input', { bubbles: true }))
    })
    await flushEffects()

    expect(apiMocks.fetchSystemTaskRuns).toHaveBeenLastCalledWith({
      taskKind: undefined,
      status: undefined,
      startedAtFrom: expectedIsoForLocalValue('2026-06-22T09:00'),
      startedAtTo: undefined,
      page: 1,
      pageSize: 20,
    })

    await act(async () => {
      const valueSetter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value')?.set
      valueSetter?.call(inputs[1], '2026-06-22T10:30')
      inputs[1].dispatchEvent(new Event('change', { bubbles: true }))
      inputs[1].dispatchEvent(new Event('input', { bubbles: true }))
    })
    await flushEffects()

    expect(apiMocks.fetchSystemTaskRuns).toHaveBeenLastCalledWith({
      taskKind: undefined,
      status: undefined,
      startedAtFrom: expectedIsoForLocalValue('2026-06-22T09:00'),
      startedAtTo: expectedIsoForLocalValue('2026-06-22T10:30', true),
      page: 1,
      pageSize: 20,
    })
  })
})
