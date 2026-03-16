/** @vitest-environment jsdom */
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'
import { OauthMailboxChip } from './OauthMailboxChip'

let host: HTMLDivElement | null = null
let root: Root | null = null

beforeAll(() => {
  Object.defineProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT', {
    configurable: true,
    writable: true,
    value: true,
  })
  class ResizeObserverMock {
    observe() {}
    unobserve() {}
    disconnect() {}
  }
  Object.defineProperty(globalThis, 'ResizeObserver', {
    configurable: true,
    writable: true,
    value: ResizeObserverMock,
  })
  Object.defineProperty(window, 'ResizeObserver', {
    configurable: true,
    writable: true,
    value: ResizeObserverMock,
  })
  if (typeof globalThis.PointerEvent === 'undefined') {
    Object.defineProperty(globalThis, 'PointerEvent', {
      configurable: true,
      writable: true,
      value: MouseEvent,
    })
  }
})

beforeEach(() => {
  vi.useFakeTimers()
})

afterEach(() => {
  act(() => {
    root?.unmount()
  })
  host?.remove()
  host = null
  root = null
  vi.useRealTimers()
})

function render(ui: React.ReactNode) {
  host = document.createElement('div')
  document.body.appendChild(host)
  root = createRoot(host)
  act(() => {
    root?.render(ui)
  })
}

function getCopyButton() {
  const button = host?.querySelector('button[aria-label="Copy mailbox"]')
  expect(button).toBeInstanceOf(HTMLButtonElement)
  return button as HTMLButtonElement
}

function getTooltip() {
  return document.body.querySelector('[role="tooltip"]') as HTMLElement | null
}

describe('OauthMailboxChip', () => {
  it('shows the copy hint on hover', () => {
    render(
      <OauthMailboxChip
        emailAddress="hover-chip@mail-tw.707079.xyz"
        emptyLabel="No mailbox yet"
        copyAriaLabel="Copy mailbox"
        copyHintLabel="Click to copy"
        copiedLabel="Copied"
        manualCopyLabel="Auto copy failed. Please copy the mailbox below manually."
        manualBadgeLabel="Manual"
        onCopy={() => undefined}
      />,
    )

    const button = getCopyButton()

    act(() => {
      button.dispatchEvent(new MouseEvent('mouseover', { bubbles: true }))
    })

    expect(getTooltip()?.textContent).toContain('Click to copy')
    expect(getTooltip()?.textContent).toContain('hover-chip@mail-tw.707079.xyz')
  })

  it('shows the copy hint after a long press and hides it when released', () => {
    render(
      <OauthMailboxChip
        emailAddress="press-chip@mail-tw.707079.xyz"
        emptyLabel="No mailbox yet"
        copyAriaLabel="Copy mailbox"
        copyHintLabel="Click to copy"
        copiedLabel="Copied"
        manualCopyLabel="Auto copy failed. Please copy the mailbox below manually."
        manualBadgeLabel="Manual"
        onCopy={() => undefined}
      />,
    )

    const button = getCopyButton()

    act(() => {
      button.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, pointerType: 'touch', button: 0 }))
      vi.advanceTimersByTime(420)
    })

    expect(getTooltip()?.textContent).toContain('Click to copy')
    expect(getTooltip()?.textContent).toContain('press-chip@mail-tw.707079.xyz')

    act(() => {
      button.dispatchEvent(new PointerEvent('pointerup', { bubbles: true, pointerType: 'touch', button: 0 }))
      vi.runOnlyPendingTimers()
    })

    expect(getTooltip()).toBeNull()
  })

  it('renders a copied success badge when the chip is in copied tone', () => {
    render(
      <OauthMailboxChip
        emailAddress="copied-chip@mail-tw.707079.xyz"
        emptyLabel="No mailbox yet"
        copyAriaLabel="Copy mailbox"
        copyHintLabel="Click to copy"
        copiedLabel="Copied"
        manualCopyLabel="Auto copy failed. Please copy the mailbox below manually."
        manualBadgeLabel="Manual"
        tone="copied"
        onCopy={() => undefined}
      />,
    )

    expect(getTooltip()?.textContent).toContain('Copied')
    expect(getCopyButton().className).toContain('border-success/55')
  })

  it('keeps the tooltip open with manual copy guidance after copy failure state', () => {
    render(
      <OauthMailboxChip
        emailAddress="manual-chip@mail-tw.707079.xyz"
        emptyLabel="No mailbox yet"
        copyAriaLabel="Copy mailbox"
        copyHintLabel="Click to copy"
        copiedLabel="Copied"
        manualCopyLabel="Auto copy failed. Please copy the mailbox below manually."
        manualBadgeLabel="Manual"
        tone="manual"
        onCopy={() => undefined}
      />,
    )

    expect(getCopyButton().className).toContain('border-warning/45')
    expect(getTooltip()?.textContent).toContain('Auto copy failed. Please copy the mailbox below manually.')
    const manualInput = document.body.querySelector('input[readonly]') as HTMLInputElement | null
    expect(manualInput?.value).toBe('manual-chip@mail-tw.707079.xyz')
  })
})
