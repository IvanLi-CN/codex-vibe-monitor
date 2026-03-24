/** @vitest-environment jsdom */
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeAll, describe, expect, it } from 'vitest'
import { MemoryRouter, NavLink } from 'react-router-dom'
import {
  SegmentedControl,
  SegmentedControlItem,
  segmentedControlItemVariants,
} from './segmented-control'

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
})

function render(ui: React.ReactNode) {
  host = document.createElement('div')
  document.body.appendChild(host)
  root = createRoot(host)
  act(() => {
    root?.render(ui)
  })
}

describe('SegmentedControl', () => {
  it('applies shared group and active item styling', () => {
    render(
      <SegmentedControl size="compact" role="tablist" aria-label="Metric switch">
        <SegmentedControlItem active role="tab" aria-selected="true">
          次数
        </SegmentedControlItem>
        <SegmentedControlItem role="tab" aria-selected="false">
          金额
        </SegmentedControlItem>
      </SegmentedControl>,
    )

    const group = host?.querySelector('[role="tablist"]')
    const activeButton = host?.querySelector('button[aria-selected="true"]')
    const inactiveButton = host?.querySelector('button[aria-selected="false"]')

    expect(group?.className).toContain('segmented-control')
    expect(activeButton?.className).toContain('segmented-control-item')
    expect(activeButton?.className).toContain('segmented-control-item--active')
    expect(activeButton?.getAttribute('data-active')).toBe('true')
    expect(inactiveButton?.getAttribute('data-active')).toBe('false')
  })

  it('supports router-driven links through the exported class helper', () => {
    render(
      <MemoryRouter initialEntries={['/dashboard']}>
        <SegmentedControl size="nav" aria-label="Primary navigation">
          <NavLink to="/dashboard" className={({ isActive }) => segmentedControlItemVariants({ size: 'nav', active: isActive })}>
            总览
          </NavLink>
          <NavLink to="/settings" className={({ isActive }) => segmentedControlItemVariants({ size: 'nav', active: isActive })}>
            设置
          </NavLink>
        </SegmentedControl>
      </MemoryRouter>,
    )

    const dashboardLink = host?.querySelector('a[href="/dashboard"]')
    const settingsLink = host?.querySelector('a[href="/settings"]')

    expect(dashboardLink?.className).toContain('segmented-control-item')
    expect(dashboardLink?.className).toContain('segmented-control-item--active')
    expect(settingsLink?.className).toContain('segmented-control-item')
    expect(settingsLink?.className).not.toContain('segmented-control-item--active')
  })
})
