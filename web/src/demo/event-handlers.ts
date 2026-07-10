import { sse } from 'msw'
import { subscribeToDemoRealtime } from './events'
import { demoModel } from './model'
import { demoSummary } from './handlers'

export const eventHandlers = [
  sse('/events', ({ client }) => {
    if (demoModel.snapshot.scene === 'network-failure') {
      client.error()
      return
    }
    client.send({ data: JSON.stringify({ type: 'summary', window: 'current', summary: demoSummary() }) })
    subscribeToDemoRealtime((payload) => client.send({ data: JSON.stringify(payload) }))
  }),
]
