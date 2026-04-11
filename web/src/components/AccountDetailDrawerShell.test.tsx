/** @vitest-environment jsdom */
import { act, StrictMode } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'
import { AccountDetailDrawerShell } from './AccountDetailDrawerShell'

let host: HTMLDivElement | null = null
let root: Root | null = null

beforeAll(() => {
  Object.defineProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT', {
    configurable: true,
    writable: true,
    value: true,
  })
})

beforeEach(() => {
  vi.useFakeTimers()
})

afterEach(() => {
  act(() => {
    root?.unmount()
  })
  host?.remove()
  root = null
  host = null
  vi.useRealTimers()
})

function renderDrawer(revision: number, open = true) {
  if (!host) {
    host = document.createElement('div')
    document.body.appendChild(host)
    root = createRoot(host)
  }

  act(() => {
    root?.render(
      <StrictMode>
        <AccountDetailDrawerShell
          open={open}
          labelledBy="drawer-shell-title"
          closeLabel="Close drawer"
          onClose={() => undefined}
          header={
            <div>
              <h2 id="drawer-shell-title">Drawer shell</h2>
              <p>revision {revision}</p>
            </div>
          }
        >
          <input data-testid="drawer-input" defaultValue={`revision-${revision}`} />
        </AccountDetailDrawerShell>
      </StrictMode>,
    )
  })
}

async function flushTimers() {
  await act(async () => {
    vi.runAllTimers()
    await Promise.resolve()
  })
}

describe('AccountDetailDrawerShell', () => {
  it('focuses the close button only when the drawer actually opens', async () => {
    renderDrawer(1, true)
    await flushTimers()

    const closeButton = Array.from(document.body.querySelectorAll('button')).find(
      (candidate) => candidate.textContent?.includes('Close drawer'),
    ) as HTMLButtonElement
    const input = document.body.querySelector('[data-testid="drawer-input"]') as HTMLInputElement

    expect(document.activeElement).toBe(closeButton)

    act(() => {
      input.focus()
    })
    expect(document.activeElement).toBe(input)

    renderDrawer(2, true)
    await flushTimers()
    expect(document.activeElement).toBe(input)

    renderDrawer(3, false)
    await flushTimers()
    expect(document.body.querySelector('[data-testid="drawer-input"]')).toBeNull()

    renderDrawer(4, true)
    await flushTimers()

    const reopenedCloseButton = Array.from(document.body.querySelectorAll('button')).find(
      (candidate) => candidate.textContent?.includes('Close drawer'),
    ) as HTMLButtonElement
    expect(document.body.querySelector('[data-testid="drawer-input"]')).not.toBeNull()
    expect(document.activeElement).toBe(reopenedCloseButton)
  })
})
