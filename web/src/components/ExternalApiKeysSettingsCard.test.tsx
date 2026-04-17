/** @vitest-environment jsdom */
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest'
import { I18nProvider } from '../i18n'
import { ExternalApiKeysSettingsCard } from './ExternalApiKeysSettingsCard'

const apiMocks = vi.hoisted(() => ({
  fetchExternalApiKeys: vi.fn(),
  createExternalApiKey: vi.fn(),
  rotateExternalApiKey: vi.fn(),
  disableExternalApiKey: vi.fn(),
}))

const {
  fetchExternalApiKeys,
  createExternalApiKey,
  rotateExternalApiKey,
  disableExternalApiKey,
} = apiMocks

vi.mock('../lib/api', () => ({
  fetchExternalApiKeys: apiMocks.fetchExternalApiKeys,
  createExternalApiKey: apiMocks.createExternalApiKey,
  rotateExternalApiKey: apiMocks.rotateExternalApiKey,
  disableExternalApiKey: apiMocks.disableExternalApiKey,
}))

let host: HTMLDivElement | null = null
let root: Root | null = null

beforeAll(() => {
  Object.defineProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT', {
    configurable: true,
    writable: true,
    value: true,
  })
})

afterEach(() => {
  vi.clearAllMocks()
  act(() => {
    root?.unmount()
  })
  host?.remove()
  host = null
  root = null
  document.body.innerHTML = ''
})

function renderCard() {
  host = document.createElement('div')
  document.body.appendChild(host)
  root = createRoot(host)
  act(() => {
    root?.render(
      <I18nProvider>
        <ExternalApiKeysSettingsCard />
      </I18nProvider>,
    )
  })
}

async function flush() {
  await act(async () => {
    await Promise.resolve()
  })
}

function textContent() {
  return document.body.textContent ?? ''
}

describe('ExternalApiKeysSettingsCard', () => {
  it('renders list and supports create, rotate, and disable flows', async () => {
    fetchExternalApiKeys.mockResolvedValue({
      items: [
        {
          id: 1,
          name: 'Vendor A upstream sync',
          status: 'active',
          prefix: 'cvm_ext_ven',
          lastUsedAt: '2026-04-16T09:30:00Z',
          createdAt: '2026-04-15T08:00:00Z',
          updatedAt: '2026-04-16T09:30:00Z',
        },
        {
          id: 2,
          name: 'Vendor B repair',
          status: 'disabled',
          prefix: 'cvm_ext_rep',
          createdAt: '2026-04-10T12:00:00Z',
          updatedAt: '2026-04-12T18:45:00Z',
        },
      ],
    })
    createExternalApiKey.mockResolvedValue({
      key: {
        id: 3,
        name: 'Vendor C realtime fill',
        status: 'active',
        prefix: 'cvm_ext_sto',
        createdAt: '2026-04-17T00:00:00Z',
        updatedAt: '2026-04-17T00:00:00Z',
      },
      secret: 'cvm_ext_story_000003',
    })
    rotateExternalApiKey.mockResolvedValue({
      key: {
        id: 4,
        name: 'Vendor A upstream sync',
        status: 'active',
        prefix: 'cvm_ext_rot',
        createdAt: '2026-04-17T01:00:00Z',
        updatedAt: '2026-04-17T01:00:00Z',
      },
      secret: 'cvm_ext_story_000004',
    })
    disableExternalApiKey.mockResolvedValue({
      key: {
        id: 4,
        name: 'Vendor A upstream sync',
        status: 'disabled',
        prefix: 'cvm_ext_rot',
        createdAt: '2026-04-17T01:00:00Z',
        updatedAt: '2026-04-17T02:00:00Z',
      },
    })

    renderCard()
    await flush()

    expect(textContent()).toContain('Vendor A upstream sync')
    expect(textContent()).toContain('Vendor B repair')

    const createButton = [...document.querySelectorAll('button')].find(
      (button) => button.textContent?.includes('创建 Key'),
    ) as HTMLButtonElement | undefined
    expect(createButton).toBeDefined()
    act(() => {
      createButton?.click()
    })

    const dialog = document.querySelector('[role="dialog"]') as HTMLElement | null
    expect(dialog).not.toBeNull()
    const nameInput = dialog?.querySelector('#external-api-key-name') as HTMLInputElement | null
    expect(nameInput).not.toBeNull()
    const valueSetter = Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      'value',
    )?.set
    act(() => {
      valueSetter?.call(nameInput, 'Vendor C realtime fill')
      nameInput!.dispatchEvent(new Event('input', { bubbles: true }))
      nameInput!.dispatchEvent(new Event('change', { bubbles: true }))
    })
    await flush()
    const confirmCreateButton = [...dialog!.querySelectorAll('button')].find((button) =>
      button.textContent?.includes('创建 Key'),
    ) as HTMLButtonElement | undefined
    act(() => {
      confirmCreateButton?.click()
    })
    await flush()

    expect(createExternalApiKey).toHaveBeenCalledWith({
      name: 'Vendor C realtime fill',
    })
    expect(textContent()).toContain('cvm_ext_story_000003')

    const rotateButtons = [...document.querySelectorAll('button')].filter((button) =>
      button.textContent?.includes('轮换'),
    ) as HTMLButtonElement[]
    act(() => {
      rotateButtons[1]?.click()
    })
    const rotateConfirmButton = [...document.querySelectorAll('button')].find((button) =>
      button.textContent?.includes('立即轮换'),
    ) as HTMLButtonElement | undefined
    act(() => {
      rotateConfirmButton?.click()
    })
    await flush()

    expect(rotateExternalApiKey).toHaveBeenCalledWith(1)
    expect(textContent()).toContain('cvm_ext_story_000004')

    const disableButtons = [...document.querySelectorAll('button')].filter((button) =>
      button.textContent?.includes('停用'),
    ) as HTMLButtonElement[]
    act(() => {
      disableButtons[disableButtons.length - 1]?.click()
    })
    const disableConfirmButton = [...document.querySelectorAll('button')].find((button) =>
      button.textContent?.includes('立即停用'),
    ) as HTMLButtonElement | undefined
    act(() => {
      disableConfirmButton?.click()
    })
    await flush()

    expect(disableExternalApiKey).toHaveBeenCalledWith(4)
    expect(textContent()).toContain('已停用')
  })
})
