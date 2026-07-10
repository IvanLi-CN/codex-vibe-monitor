import { afterAll, afterEach, beforeAll, describe, expect, it } from 'vitest'
import { setupServer } from 'msw/node'
import { apiHandlers } from './handlers'
import { demoModel } from './model'

const server = setupServer(...apiHandlers)

beforeAll(() => server.listen({ onUnhandledRequest: 'error' }))
afterEach(() => {
  demoModel.setScene('operational')
  demoModel.reset()
})
afterAll(() => server.close())

describe('demo MSW handlers', () => {
  it('serves deterministic dashboard activity in the shape used by the production normalizer', async () => {
    const response = await fetch('http://demo.invalid/api/stats/dashboard-activity?range=today')
    const payload = await response.json() as { summary: { stats: { totalCount: number } } }

    expect(response.ok).toBe(true)
    expect(payload.summary.stats.totalCount).toBe(12_846)
  })

  it('does not retain sensitive settings input', async () => {
    const response = await fetch('http://demo.invalid/api/settings/proxy', {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        enabledModels: ['gpt-5.6-sol'],
        apiKey: 'input-must-not-return',
        refreshToken: 'token-must-not-return',
      }),
    })
    const body = await response.text()

    expect(response.ok).toBe(true)
    expect(body).not.toContain('input-must-not-return')
    expect(body).not.toContain('token-must-not-return')
  })

  it('creates a deterministic external key result without retaining the submitted name', async () => {
    const response = await fetch('http://demo.invalid/api/settings/external-api-keys', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ name: 'submitted-name-must-not-persist' }),
    })
    const payload = await response.json() as { key: { name: string }; secret: string }
    const listing = await fetch('http://demo.invalid/api/settings/external-api-keys')
    const listingBody = await listing.text()

    expect(response.status).toBe(201)
    expect(payload.key.name).toBe('Demo integration 2')
    expect(payload.secret).toBe('demo-generated-key-not-valid')
    expect(listingBody).toContain('Demo integration 2')
    expect(listingBody).not.toContain('submitted-name-must-not-persist')
  })

  it('fails closed instead of returning a real network response in network-failure scene', async () => {
    demoModel.setScene('network-failure')

    await expect(fetch('http://demo.invalid/api/stats/summary')).rejects.toThrow()
  })
})
