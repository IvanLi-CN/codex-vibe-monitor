import type { BroadcastPayload } from '../lib/api'

export interface StorybookPageSseController {
  emit: (payload: BroadcastPayload) => void
  emitOpen: () => void
  reset: () => void
}

declare global {
  interface Window {
    __storybookPageSseController__?: StorybookPageSseController
  }
}

export function getStorybookPageSseController(): StorybookPageSseController | null {
  if (typeof window === 'undefined') {
    return null
  }
  return window.__storybookPageSseController__ ?? null
}
