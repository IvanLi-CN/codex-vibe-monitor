/** @vitest-environment jsdom */
import * as React from 'react'
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeAll, describe, expect, it } from 'vitest'
import {
  Dialog,
  DialogContent,
  DialogTitle,
} from './dialog'
import { OverlayHostProvider } from './overlay-host'
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from './popover'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from './select'
import { Tooltip } from './tooltip'

class MockResizeObserver {
  observe() {}
  unobserve() {}
  disconnect() {}
}

let root: Root | null = null
let host: HTMLDivElement | null = null
let overlayHost: HTMLDivElement | null = null
let explicitRoot: HTMLDivElement | null = null

beforeAll(() => {
  Object.defineProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT', {
    configurable: true,
    writable: true,
    value: true,
  })
  Object.defineProperty(window, 'ResizeObserver', {
    configurable: true,
    writable: true,
    value: MockResizeObserver,
  })
  Object.defineProperty(globalThis, 'ResizeObserver', {
    configurable: true,
    writable: true,
    value: MockResizeObserver,
  })
  Object.defineProperty(HTMLElement.prototype, 'scrollIntoView', {
    configurable: true,
    writable: true,
    value: () => undefined,
  })
})

afterEach(() => {
  act(() => {
    root?.unmount()
  })
  host?.remove()
  overlayHost?.remove()
  explicitRoot?.remove()
  root = null
  host = null
  overlayHost = null
  explicitRoot = null
})

function render(ui: React.ReactNode) {
  host = document.createElement('div')
  document.body.appendChild(host)
  root = createRoot(host)
  act(() => {
    root?.render(ui)
  })
}

function setupOverlayRoots() {
  overlayHost = document.createElement('div')
  overlayHost.setAttribute('data-testid', 'overlay-host')
  explicitRoot = document.createElement('div')
  explicitRoot.setAttribute('data-testid', 'explicit-root')
  document.body.appendChild(overlayHost)
  document.body.appendChild(explicitRoot)
}

function findElementByText(text: string) {
  return Array.from(document.body.querySelectorAll<HTMLElement>('*')).find(
    (node) => node.textContent?.trim() === text,
  ) ?? null
}

describe('overlay host inheritance', () => {
  it('falls back to document.body when no overlay host is provided', () => {
    render(
      <Popover open>
        <PopoverTrigger asChild>
          <button type="button">Open</button>
        </PopoverTrigger>
        <PopoverContent>Body fallback popover</PopoverContent>
      </Popover>,
    )

    const content = findElementByText('Body fallback popover')
    expect(content).not.toBeNull()
    expect(host?.contains(content)).toBe(false)
    expect(document.body.contains(content)).toBe(true)
  })

  it('inherits the nearest overlay host for popover, select, dialog, and tooltip content', () => {
    setupOverlayRoots()
    if (!overlayHost) throw new Error('missing overlay host')

    render(
      <OverlayHostProvider value={overlayHost}>
        <>
          <Popover open>
            <PopoverTrigger asChild>
              <button type="button">Open popover</button>
            </PopoverTrigger>
            <PopoverContent>Popover inside host</PopoverContent>
          </Popover>

          <Select open value="one" onValueChange={() => undefined}>
            <SelectTrigger aria-label="Host select">
              <SelectValue placeholder="Pick one" />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="one">Select inside host</SelectItem>
            </SelectContent>
          </Select>

          <Dialog open>
            <DialogContent>
              <DialogTitle>Dialog in host</DialogTitle>
              <p>Dialog body inside host</p>
            </DialogContent>
          </Dialog>

          <Tooltip open content="Tooltip inside host">
            <span>Tooltip target</span>
          </Tooltip>
        </>
      </OverlayHostProvider>,
    )

    expect(overlayHost.contains(findElementByText('Popover inside host'))).toBe(true)
    expect(overlayHost.contains(findElementByText('Select inside host'))).toBe(true)
    expect(overlayHost.contains(findElementByText('Dialog body inside host'))).toBe(true)
    expect(overlayHost.contains(findElementByText('Tooltip inside host'))).toBe(true)
  })

  it('lets an explicit container override the inherited overlay host', () => {
    setupOverlayRoots()
    if (!overlayHost || !explicitRoot) throw new Error('missing overlay roots')

    render(
      <OverlayHostProvider value={overlayHost}>
        <Popover open>
          <PopoverTrigger asChild>
            <button type="button">Open</button>
          </PopoverTrigger>
          <PopoverContent container={explicitRoot}>Popover in explicit root</PopoverContent>
        </Popover>
      </OverlayHostProvider>,
    )

    const content = findElementByText('Popover in explicit root')
    expect(content).not.toBeNull()
    expect(explicitRoot.contains(content)).toBe(true)
    expect(overlayHost.contains(content)).toBe(false)
  })

  it('keeps nested popovers out of overflow-hidden dialog content while staying inside the dialog host', () => {
    setupOverlayRoots()
    if (!overlayHost) throw new Error('missing overlay host')

    render(
      <OverlayHostProvider value={overlayHost}>
        <Dialog open>
          <DialogContent className="overflow-hidden">
            <DialogTitle>Dialog with nested popover</DialogTitle>
            <Popover open>
              <PopoverTrigger asChild>
                <button type="button">Open nested popover</button>
              </PopoverTrigger>
              <PopoverContent>Nested popover inside dialog</PopoverContent>
            </Popover>
          </DialogContent>
        </Dialog>
      </OverlayHostProvider>,
    )

    const dialog = document.body.querySelector('[role="dialog"]') as HTMLElement | null
    const popover = findElementByText('Nested popover inside dialog')

    expect(dialog).not.toBeNull()
    expect(popover).not.toBeNull()
    expect(overlayHost.contains(popover)).toBe(true)
    expect(dialog?.contains(popover)).toBe(false)
  })
})
