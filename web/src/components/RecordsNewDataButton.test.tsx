/** @vitest-environment jsdom */
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest'
import { RecordsNewDataButton } from './RecordsNewDataButton'

vi.mock('../i18n', () => ({
  useTranslation: () => ({
    locale: 'zh',
    t: (key: string, values?: Record<string, string | number>) => {
      const count = values?.count ?? ''
      switch (key) {
        case 'records.summary.notice.newData':
          return `有 ${count} 条新数据`
        case 'records.summary.notice.refreshAction':
          return '加载新数据'
        case 'records.summary.notice.newDataAria':
          return `有 ${count} 条新数据，点击后会并入当前快照。`
        case 'records.summary.notice.refreshAria':
          return `加载这 ${count} 条新数据并刷新当前快照。`
        case 'records.summary.notice.refreshingAria':
          return `正在加载这 ${count} 条新数据并刷新当前快照。`
        default:
          return key
      }
    },
  }),
}))

let host: HTMLDivElement | null = null
let root: Root | null = null

beforeAll(() => {
  Object.defineProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT', {
    configurable: true,
    writable: true,
    value: true,
  })
})

afterEach(() => {
  act(() => {
    root?.unmount()
  })
  host?.remove()
  host = null
  root = null
  vi.clearAllMocks()
})

function render(ui: React.ReactNode) {
  host = document.createElement('div')
  document.body.appendChild(host)
  root = createRoot(host)
  act(() => {
    root?.render(ui)
  })
}

function getButton() {
  const button = host?.querySelector('[data-testid="records-new-data-button"]')
  if (!(button instanceof HTMLButtonElement)) {
    throw new Error('missing new data button')
  }
  return button
}

function getLabel(testId: 'records-new-data-label-idle' | 'records-new-data-label-action') {
  const label = host?.querySelector(`[data-testid="${testId}"]`)
  if (!(label instanceof HTMLSpanElement)) {
    throw new Error(`missing label: ${testId}`)
  }
  return label
}

describe('RecordsNewDataButton', () => {
  it('renders idle state by default', () => {
    render(<RecordsNewDataButton count={17} onRefresh={vi.fn()} />)

    const button = getButton()
    expect(button.dataset.state).toBe('idle')
    expect(button.dataset.icon).toBe('help')
    expect(button.getAttribute('aria-label')).toBe('有 17 条新数据，点击后会并入当前快照。')
    expect(getLabel('records-new-data-label-idle').className).toContain('opacity-100')
    expect(getLabel('records-new-data-label-action').className).toContain('opacity-0')
  })

  it('supports forced interactive preview state', () => {
    render(
      <RecordsNewDataButton
        count={17}
        onRefresh={vi.fn()}
        stateOverride="interactive"
      />,
    )

    const button = getButton()
    expect(button.dataset.state).toBe('interactive')
    expect(button.className).toContain('border-primary/35')
    expect(button.getAttribute('aria-label')).toBe('加载这 17 条新数据并刷新当前快照。')
    expect(getLabel('records-new-data-label-idle').className).toContain('opacity-0')
    expect(getLabel('records-new-data-label-action').className).toContain('opacity-100')
  })

  it('shows loading state and disables clicks when loading', () => {
    const onRefresh = vi.fn()
    render(<RecordsNewDataButton count={17} onRefresh={onRefresh} isLoading />)

    const button = getButton()
    act(() => {
      button.click()
    })

    expect(onRefresh).not.toHaveBeenCalled()
    expect(button.dataset.state).toBe('loading')
    expect(button.dataset.icon).toBe('refresh')
    expect(button.disabled).toBe(true)
    expect(button.getAttribute('aria-label')).toBe('正在加载这 17 条新数据并刷新当前快照。')
  })
})
