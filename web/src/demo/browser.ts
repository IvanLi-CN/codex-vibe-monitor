import { setupWorker } from 'msw/browser'
import { eventHandlers } from './event-handlers'
import { apiHandlers } from './handlers'

export const worker = setupWorker(...eventHandlers, ...apiHandlers)
