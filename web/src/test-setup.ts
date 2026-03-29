import React from 'react'
import { vi } from 'vitest'

class ResizeObserverMock {
  observe() {}

  unobserve() {}

  disconnect() {}
}

if (!('ResizeObserver' in globalThis)) {
  Object.defineProperty(globalThis, 'ResizeObserver', {
    configurable: true,
    writable: true,
    value: ResizeObserverMock,
  })
}

// Iconify schedules async DOM updates that outlive jsdom teardown in Vitest.
// Replace it with a stable test double so UI tests stay deterministic.
vi.mock('@iconify/react', () => {
  function Icon({
    icon,
    ...props
  }: {
    icon?: string
  } & React.HTMLAttributes<HTMLSpanElement>) {
    return React.createElement('span', {
      ...props,
      'data-icon': icon ?? '',
    })
  }

  return { Icon }
})
