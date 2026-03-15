import React from 'react'
import { vi } from 'vitest'

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
