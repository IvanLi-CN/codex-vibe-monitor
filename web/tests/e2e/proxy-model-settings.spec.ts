import { expect, test, type APIRequestContext, type Locator, type Page } from '@playwright/test'

const PRESET_MODELS = [
  'gpt-5.3-codex',
  'gpt-5.2-codex',
  'gpt-5.1-codex-max',
  'gpt-5.1-codex-mini',
  'gpt-5.2',
] as const

const MERGE_STATUS_HEADER = 'x-proxy-model-merge-upstream'
const BACKEND_BASE_URL = process.env.E2E_BACKEND_URL ?? 'http://127.0.0.1:8080'

interface ProxyModelSettings {
  hijackEnabled: boolean
  mergeUpstreamEnabled: boolean
  defaultHijackEnabled: boolean
  models: string[]
  enabledModels: string[]
}

async function getProxyModelSettings(request: APIRequestContext): Promise<ProxyModelSettings> {
  const response = await request.get(`${BACKEND_BASE_URL}/api/settings/proxy-models`)
  expect(response.ok()).toBeTruthy()
  return (await response.json()) as ProxyModelSettings
}

async function putProxyModelSettings(
  request: APIRequestContext,
  payload: Pick<ProxyModelSettings, 'hijackEnabled' | 'mergeUpstreamEnabled' | 'enabledModels'>,
): Promise<ProxyModelSettings> {
  const response = await request.put(`${BACKEND_BASE_URL}/api/settings/proxy-models`, { data: payload })
  expect(response.ok()).toBeTruthy()
  return (await response.json()) as ProxyModelSettings
}

async function openProxySettingsModal(page: Page): Promise<Locator> {
  await page.goto('/dashboard')
  const openButton = page.getByRole('button', { name: /Proxy settings|代理设置/ })
  await expect(openButton).toBeVisible()
  await openButton.click()

  const modal = page.locator('.modal-box')
  await expect(modal.getByRole('heading', { name: /Model list hijack|模型列表劫持/ })).toBeVisible()
  return modal
}

function extractModelIds(payload: unknown): string[] {
  const obj = (payload ?? {}) as Record<string, unknown>
  const data = Array.isArray(obj.data) ? obj.data : []
  return data
    .map((entry) => ((entry ?? {}) as Record<string, unknown>).id)
    .filter((id): id is string => typeof id === 'string')
}

test.describe.serial('proxy model settings e2e', () => {
  let initialSettings: ProxyModelSettings

  test.beforeAll(async ({ request }) => {
    initialSettings = await getProxyModelSettings(request)
  })

  test.afterAll(async ({ request }) => {
    await putProxyModelSettings(request, {
      hijackEnabled: initialSettings.hijackEnabled,
      mergeUpstreamEnabled: initialSettings.mergeUpstreamEnabled,
      enabledModels: initialSettings.enabledModels,
    })
  })

  test('renders proxy model settings modal with expected preset models', async ({ page, request }) => {
    await putProxyModelSettings(request, {
      hijackEnabled: false,
      mergeUpstreamEnabled: false,
      enabledModels: [...PRESET_MODELS],
    })

    const modal = await openProxySettingsModal(page)
    for (const model of PRESET_MODELS) {
      await expect(modal.getByText(model, { exact: true })).toBeVisible()
    }
    await expect(modal.getByText(/已启用：\s*5\s*\/\s*5|Enabled:\s*5\s*\/\s*5/)).toBeVisible()
  })

  test('updates enabled preset models from UI and persists after reload', async ({ page, request }) => {
    await putProxyModelSettings(request, {
      hijackEnabled: true,
      mergeUpstreamEnabled: false,
      enabledModels: [...PRESET_MODELS],
    })

    const modal = await openProxySettingsModal(page)
    await modal.getByText('gpt-5.3-codex', { exact: true }).click()

    await expect
      .poll(async () => {
        const settings = await getProxyModelSettings(request)
        return settings.enabledModels.includes('gpt-5.3-codex')
      })
      .toBeFalsy()

    await page.reload()
    const reloadedModal = await openProxySettingsModal(page)
    await expect(reloadedModal.getByText(/已启用：\s*4\s*\/\s*5|Enabled:\s*4\s*\/\s*5/)).toBeVisible()
  })

  test('returns only enabled presets when hijack is enabled and merge is disabled', async ({ request }) => {
    const enabledModels = PRESET_MODELS.filter((model) => model !== 'gpt-5.3-codex')
    await putProxyModelSettings(request, {
      hijackEnabled: true,
      mergeUpstreamEnabled: false,
      enabledModels,
    })

    const response = await request.get(`${BACKEND_BASE_URL}/v1/models`)
    expect(response.ok()).toBeTruthy()

    const payload = await response.json()
    const ids = extractModelIds(payload)
    expect(new Set(ids)).toEqual(new Set(enabledModels))
    expect(ids).toHaveLength(enabledModels.length)
    expect(response.headers()[MERGE_STATUS_HEADER]).toBeUndefined()
  })

  test('merge-upstream mode keeps enabled presets and returns merge status header', async ({ request }) => {
    const enabledModels = ['gpt-5.3-codex', 'gpt-5.1-codex-mini']
    await putProxyModelSettings(request, {
      hijackEnabled: true,
      mergeUpstreamEnabled: true,
      enabledModels,
    })

    const response = await request.get(`${BACKEND_BASE_URL}/v1/models`)
    expect(response.ok()).toBeTruthy()

    const payload = await response.json()
    const ids = extractModelIds(payload)
    for (const modelId of enabledModels) {
      expect(ids).toContain(modelId)
    }

    const mergeStatus = response.headers()[MERGE_STATUS_HEADER]
    expect(mergeStatus === 'success' || mergeStatus === 'failed').toBeTruthy()
  })

  test('inserts no extra models when all presets are disabled', async ({ page, request }) => {
    await putProxyModelSettings(request, {
      hijackEnabled: true,
      mergeUpstreamEnabled: false,
      enabledModels: [],
    })

    const response = await request.get(`${BACKEND_BASE_URL}/v1/models`)
    expect(response.ok()).toBeTruthy()

    const payload = await response.json()
    const ids = extractModelIds(payload)
    expect(ids).toHaveLength(0)

    const modal = await openProxySettingsModal(page)
    await expect(modal.getByText(/已启用：\s*0\s*\/\s*5|Enabled:\s*0\s*\/\s*5/)).toBeVisible()
  })
})
