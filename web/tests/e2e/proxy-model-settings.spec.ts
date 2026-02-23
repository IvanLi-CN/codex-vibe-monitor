import { expect, test, type APIRequestContext, type Page } from '@playwright/test'

const PRESET_MODELS = [
  'gpt-5.3-codex',
  'gpt-5.2-codex',
  'gpt-5.1-codex-max',
  'gpt-5.1-codex-mini',
  'gpt-5.2',
] as const

const BACKEND_BASE_URL = process.env.E2E_BACKEND_URL ?? 'http://127.0.0.1:8080'

interface ProxySettings {
  hijackEnabled: boolean
  mergeUpstreamEnabled: boolean
  defaultHijackEnabled: boolean
  models: string[]
  enabledModels: string[]
}

interface PricingEntry {
  model: string
  inputPer1m: number
  outputPer1m: number
  cacheInputPer1m?: number | null
  reasoningPer1m?: number | null
  source: string
}

interface PricingSettings {
  catalogVersion: string
  entries: PricingEntry[]
}

interface SettingsPayload {
  proxy: ProxySettings
  pricing: PricingSettings
}

async function getSettings(request: APIRequestContext): Promise<SettingsPayload> {
  const response = await request.get(`${BACKEND_BASE_URL}/api/settings`)
  expect(response.ok()).toBeTruthy()
  return (await response.json()) as SettingsPayload
}

async function putProxySettings(
  request: APIRequestContext,
  payload: Pick<ProxySettings, 'hijackEnabled' | 'mergeUpstreamEnabled' | 'enabledModels'>,
): Promise<ProxySettings> {
  const response = await request.put(`${BACKEND_BASE_URL}/api/settings/proxy`, { data: payload })
  expect(response.ok()).toBeTruthy()
  return (await response.json()) as ProxySettings
}

async function putPricingSettings(request: APIRequestContext, payload: PricingSettings): Promise<PricingSettings> {
  const response = await request.put(`${BACKEND_BASE_URL}/api/settings/pricing`, { data: payload })
  expect(response.ok()).toBeTruthy()
  return (await response.json()) as PricingSettings
}

async function openSettingsPage(page: Page): Promise<Page> {
  await page.goto('/#/settings')
  const heading = page.getByRole('heading', { name: /Settings|设置/ })
  await expect(heading).toBeVisible()
  return page
}

test.describe.serial('settings e2e', () => {
  let initialSettings: SettingsPayload

  test.beforeAll(async ({ request }) => {
    initialSettings = await getSettings(request)
  })

  test.afterAll(async ({ request }) => {
    await putProxySettings(request, {
      hijackEnabled: initialSettings.proxy.hijackEnabled,
      mergeUpstreamEnabled: initialSettings.proxy.mergeUpstreamEnabled,
      enabledModels: initialSettings.proxy.enabledModels,
    })
    await putPricingSettings(request, initialSettings.pricing)
  })

  test('renders settings page with proxy and pricing sections', async ({ page }) => {
    const settingsPage = await openSettingsPage(page)
    await expect(settingsPage.getByText(/Proxy configuration|代理配置/)).toBeVisible()
    await expect(settingsPage.getByText(/Pricing configuration|价格配置/)).toBeVisible()
  })

  test('updates enabled preset models from settings page', async ({ page, request }) => {
    await putProxySettings(request, {
      hijackEnabled: true,
      mergeUpstreamEnabled: false,
      enabledModels: [...PRESET_MODELS],
    })

    const settingsPage = await openSettingsPage(page)
    const modelRow = settingsPage.getByText('gpt-5.3-codex', { exact: true }).first()
    await modelRow.click()

    await expect
      .poll(async () => {
        const settings = await getSettings(request)
        return settings.proxy.enabledModels.includes('gpt-5.3-codex')
      })
      .toBeFalsy()
  })

  test('pricing change auto-saves and persists after reload', async ({ page, request }) => {
    const current = await getSettings(request)
    const settingsPage = await openSettingsPage(page)
    const modelField = settingsPage.locator('table tbody tr input[type="text"]').first()
    await expect(modelField).toBeVisible()
    const targetModel = (await modelField.inputValue()).trim()
    expect(targetModel).not.toHaveLength(0)

    const currentPricing = current.pricing.entries.find((entry) => entry.model === targetModel)
    expect(currentPricing).toBeTruthy()
    const nextInput = Number(((currentPricing?.inputPer1m ?? 1.75) + 0.01).toFixed(2))

    const inputField = settingsPage.locator('table tbody tr input[type="number"]').first()
    await expect(inputField).toBeVisible()
    await inputField.fill(String(nextInput))
    await inputField.blur()

    await expect
      .poll(async () => {
        const settings = await getSettings(request)
        return settings.pricing.entries.find((entry) => entry.model === targetModel)?.inputPer1m
      })
      .toBe(nextInput)

    await page.reload()
    const reloadedPage = await openSettingsPage(page)
    await expect(reloadedPage.locator('table tbody tr input[type="number"]').first()).toHaveValue(String(nextInput))
  })

  test('legacy proxy-models endpoint is removed', async ({ request }) => {
    const response = await request.get(`${BACKEND_BASE_URL}/api/settings/proxy-models`)
    expect(response.status()).toBe(404)
  })
})
